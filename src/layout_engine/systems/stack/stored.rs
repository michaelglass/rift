//! The stack system's saved form (upstream's, byte for byte) and its load: the current form and
//! the legacy forest (a stack layout saved by the old tree), parsed into the layouts' stacks; the
//! states only a saved file holds are normalized here, at the boundary (one `stack-load:` warn
//! each).

use serde::{Deserialize, Serialize};
use slotmap::{Key, SlotMap};

use crate::actor::app::WindowId;
use crate::common::config::{StackDefaultOrientation, default_stack_orientation};
use crate::layout_engine::systems::group_stored::{LayoutRoot, focus_target};
use crate::layout_engine::{LayoutId, LayoutKind};
use crate::model::tree::{NodeId, Tree};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(super) struct Member {
    pub(super) id: NodeId,
    pub(super) window: WindowId,
    #[serde(default = "unit_weight")]
    pub(super) weight: f64,
    #[serde(default)]
    pub(super) fullscreen: bool,
    #[serde(default)]
    pub(super) within_gaps: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(super) struct Stack {
    pub(super) id: NodeId,
    pub(super) members: Vec<Member>,
    pub(super) active: Option<NodeId>,
    pub(super) root_selected: bool,
    pub(super) kind: LayoutKind,
    pub(super) fullscreen: bool,
    pub(super) within_gaps: bool,
}

fn unit_weight() -> f64 { 1.0 }

impl Stack {
    /// A saved stack as an operation leaves one (one `stack-load:` warn per state fixed; upstream's
    /// load keeps them all, an upstream candidate): a stack layout kind (`stack_kind`, as the
    /// legacy load maps one), and a selection naming a member while there are members (the first
    /// when it names none). Two more states are only warned here, as the core's types cannot
    /// hold them: both fullscreen flags on a stack or a member (a `Fill` is one of three, and
    /// fullscreen wins, as upstream's projection reads them), and an empty stack with a selection
    /// (an empty stack holds none).
    pub(super) fn normalize(&mut self, layout: LayoutId) {
        let kind = stack_kind(self.kind);
        if kind != self.kind {
            tracing::warn!(?layout, saved = ?self.kind, ?kind, "stack-load: normalized the stack's layout kind");
            self.kind = kind;
        }
        if self.fullscreen && self.within_gaps {
            tracing::warn!(
                ?layout,
                "stack-load: normalized a stack with both fullscreen flags to fullscreen"
            );
        }
        for member in self.members.iter().filter(|m| m.fullscreen && m.within_gaps) {
            tracing::warn!(
                ?layout,
                member = ?member.id,
                "stack-load: normalized a member with both fullscreen flags to fullscreen"
            );
        }
        let names_a_member = self.members.iter().any(|m| Some(m.id) == self.active);
        match self.members.first() {
            Some(first) if !names_a_member => {
                tracing::warn!(
                    ?layout,
                    active = ?self.active,
                    "stack-load: normalized a selection naming no member to the first member"
                );
                self.active = Some(first.id);
            }
            None if self.active.is_some() || self.root_selected => {
                tracing::warn!(
                    ?layout,
                    active = ?self.active,
                    root_selected = self.root_selected,
                    "stack-load: normalized an empty stack's selection away"
                );
            }
            _ => {}
        }
    }
}

/// A stack's layout kind as a stack layout's (a tiled kind as the stack of its orientation).
pub(super) fn stack_kind(kind: LayoutKind) -> LayoutKind {
    match kind {
        LayoutKind::Horizontal | LayoutKind::HorizontalStack => LayoutKind::HorizontalStack,
        _ => LayoutKind::VerticalStack,
    }
}

#[derive(Deserialize)]
struct LegacyStackForest {
    #[serde(
        deserialize_with = "crate::layout_engine::systems::group_stored::deserialize_tile_tree"
    )]
    tree: Tree,
    layout_roots: Vec<crate::layout_engine::systems::SavedSlot<LayoutRoot>>,
}

impl LegacyStackForest {
    fn migrate(self) -> Result<(SlotMap<LayoutId, Stack>, u32), ron::Error> {
        let map = self.tree.map;
        let mut next_node = 1;
        let slots: Vec<_> = self
            .layout_roots
            .into_iter()
            .map(|slot| {
                let value = slot.value.map(|root| {
                    let root = root.0;
                    let selected = map.current_selection(root);
                    let active = focus_target(&map, selected)
                        .or_else(|| focus_target(&map, root))
                        .map(|(node, _)| node);
                    let members: Vec<_> = root
                        .traverse_preorder(&map)
                        .filter_map(|id| {
                            next_node =
                                next_node.max((id.data().as_ffi() as u32).saturating_add(1));
                            let window = map.window(id)?;
                            let info = &map[id].layout;
                            Some(Member {
                                id,
                                window,
                                weight: f64::from(info.size),
                                fullscreen: info.is_fullscreen,
                                within_gaps: info.is_fullscreen_within_gaps,
                            })
                        })
                        .collect();
                    let info = &map[root].layout;
                    Stack {
                        id: root,
                        active,
                        members,
                        root_selected: selected == root,
                        kind: stack_kind(info.kind),
                        fullscreen: info.is_fullscreen,
                        within_gaps: info.is_fullscreen_within_gaps,
                    }
                });
                crate::layout_engine::systems::SavedSlot { value, version: slot.version }
            })
            .collect();
        let layouts = ron::from_str(&ron::to_string(&slots)?).map_err(|error| error.code)?;
        Ok((layouts, next_node))
    }
}

/// A stack system as rift saves it (upstream's derived form; the insertion point and the
/// windows left out are not saved), or the legacy forest.
#[derive(Deserialize)]
pub(super) struct StoredStackSystem {
    #[serde(default)]
    layouts: SlotMap<LayoutId, Stack>,
    #[serde(default = "first_node")]
    next_node: u32,
    #[serde(default = "default_stack_orientation")]
    default_orientation: StackDefaultOrientation,
    #[serde(default, deserialize_with = "crate::layout_engine::systems::present")]
    inner: Option<LegacyStackForest>,
}

fn first_node() -> u32 { 1 }

/// A saved system, parsed: the legacy forest read into the current form, and on each stack the
/// states only a saved file holds normalized (one `stack-load:` warn each). Refused when the
/// legacy forest does not read.
pub(super) struct Loaded {
    pub(super) layouts: SlotMap<LayoutId, Stack>,
    pub(super) next_node: u32,
    pub(super) default_orientation: StackDefaultOrientation,
}

impl TryFrom<StoredStackSystem> for Loaded {
    type Error = String;

    fn try_from(stored: StoredStackSystem) -> Result<Self, String> {
        let (mut layouts, next_node) = match stored.inner {
            Some(inner) => inner.migrate().map_err(|error| error.to_string())?,
            None => (stored.layouts, stored.next_node),
        };
        for (layout, stack) in &mut layouts {
            stack.normalize(layout);
        }
        Ok(Self {
            layouts,
            next_node,
            default_orientation: stored.default_orientation,
        })
    }
}
