use objc2_core_foundation::CGSize;
use serde::{Deserialize, Serialize};

use super::{LayoutId, LayoutSystem};
use crate::common::collections::HashMap;

/// A workspace's layout state as a saved file holds it: its active layout and the display size
/// it was laid out for, and the layouts an older multi-size file kept for other sizes
/// (`obsolete`). Only this type knows them: [`SavedLayoutState::into_live`] drops their trees,
/// once, as the file becomes live state.
#[derive(Debug, Default)]
pub(crate) struct SavedLayoutState {
    layout: Option<LayoutId>,
    active_size: Size,
    obsolete: Vec<LayoutId>,
}

impl<'de> Deserialize<'de> for SavedLayoutState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Snapshot {
            #[serde(default)]
            layout: Option<LayoutId>,
            active_size: Size,
            #[serde(default)]
            configurations: HashMap<Size, LayoutId>,
            #[serde(default, rename = "last_saved")]
            legacy: Option<LayoutId>,
            #[serde(default)]
            obsolete: Vec<LayoutId>,
        }
        let saved = Snapshot::deserialize(deserializer)?;
        if saved.layout.is_none()
            && !saved.configurations.is_empty()
            && !saved.configurations.contains_key(&saved.active_size)
        {
            return Err(serde::de::Error::custom(
                "no configuration for its active display size",
            ));
        }
        let layout = saved
            .layout
            .or_else(|| saved.configurations.get(&saved.active_size).copied())
            .or(saved.legacy);
        let mut obsolete: Vec<_> = saved
            .configurations
            .into_values()
            .chain(saved.legacy)
            .chain(saved.obsolete)
            .filter(|id| Some(*id) != layout)
            .collect();
        obsolete.sort_unstable();
        obsolete.dedup();
        Ok(Self {
            layout,
            active_size: saved.active_size,
            obsolete,
        })
    }
}

impl SavedLayoutState {
    /// Every layout the file holds for the workspace: the active one, then the obsolete ones.
    pub(crate) fn trees(&self) -> impl Iterator<Item = LayoutId> + '_ {
        self.layout.into_iter().chain(self.obsolete.iter().copied())
    }

    /// An error if a layout the file names is not in `tree`.
    pub(crate) fn validate(&self, tree: &impl LayoutSystem) -> Result<(), String> {
        for layout in self.trees() {
            if !tree.contains_layout(layout) {
                return Err(format!("references missing layout {layout:?}"));
            }
        }
        Ok(())
    }

    /// The live state: the active layout kept, the obsolete layouts' trees dropped from `tree`.
    pub(crate) fn into_live(self, tree: &mut impl LayoutSystem) -> WorkspaceLayoutState {
        for old in self.obsolete {
            tree.remove_layout(old);
        }
        match self.layout {
            Some(layout) => WorkspaceLayoutState::Laid { layout, size: self.active_size },
            None => WorkspaceLayoutState::Unlaid,
        }
    }
}

/// One logical arrangement follows its workspace between viewports: none yet, or its layout and
/// the display size it is laid out for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorkspaceLayoutState {
    #[default]
    Unlaid,
    Laid {
        layout: LayoutId,
        size: Size,
    },
}

/// Saved as files have always held it: the layout, if any, and the size (zero when none).
impl Serialize for WorkspaceLayoutState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let (layout, size) = match *self {
            WorkspaceLayoutState::Unlaid => (None, Size::default()),
            WorkspaceLayoutState::Laid { layout, size } => (Some(layout), size),
        };
        let mut saved = serializer.serialize_struct("WorkspaceLayoutState", 2)?;
        saved.serialize_field("layout", &layout)?;
        saved.serialize_field("active_size", &size)?;
        saved.end()
    }
}

#[derive(
    Serialize, Deserialize, Default, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Debug,
)]
pub(crate) struct Size {
    width: i32,
    height: i32,
}

impl From<CGSize> for Size {
    fn from(value: CGSize) -> Self {
        Self {
            width: value.width.round() as i32,
            height: value.height.round() as i32,
        }
    }
}

impl WorkspaceLayoutState {
    pub(crate) fn active(&self) -> Option<LayoutId> {
        match *self {
            WorkspaceLayoutState::Unlaid => None,
            WorkspaceLayoutState::Laid { layout, .. } => Some(layout),
        }
    }

    pub(crate) fn active_size(&self) -> Option<CGSize> {
        match *self {
            WorkspaceLayoutState::Unlaid => None,
            WorkspaceLayoutState::Laid { size, .. } => {
                Some(CGSize::new(size.width.into(), size.height.into()))
            }
        }
    }

    /// Laid out for `size`: its layout, created in `tree` if it had none.
    pub(crate) fn ensure_active(&mut self, size: CGSize, tree: &mut impl LayoutSystem) {
        let layout = self.active().unwrap_or_else(|| tree.create_layout());
        *self = WorkspaceLayoutState::Laid { layout, size: size.into() };
    }

    /// `layout` the active one (at a nominal size, if there was none).
    pub(crate) fn replace(&mut self, layout: LayoutId) {
        let size = match *self {
            WorkspaceLayoutState::Unlaid => Size::from(CGSize::new(1000.0, 1000.0)),
            WorkspaceLayoutState::Laid { size, .. } => size,
        };
        *self = WorkspaceLayoutState::Laid { layout, size };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_engine::systems::TraditionalLayoutSystem;

    /// Laid out once, a workspace keeps its layout across sizes and its size across a new
    /// layout; with none yet, a new layout takes a nominal size.
    #[test]
    fn a_layout_state_keeps_its_size_across_a_new_layout_and_its_layout_across_sizes() {
        let mut tree = TraditionalLayoutSystem::default();
        let mut state = WorkspaceLayoutState::Unlaid;
        assert_eq!((state.active(), state.active_size()), (None, None));
        state.ensure_active(CGSize::new(1200.0, 800.0), &mut tree);
        let layout = state.active().unwrap();
        state.ensure_active(CGSize::new(1600.0, 1000.0), &mut tree);
        assert_eq!(state.active(), Some(layout));
        assert_eq!(state.active_size(), Some(CGSize::new(1600.0, 1000.0)));
        let other = tree.create_layout();
        state.replace(other);
        assert_eq!(state.active(), Some(other));
        assert_eq!(state.active_size(), Some(CGSize::new(1600.0, 1000.0)));
        let mut fresh = WorkspaceLayoutState::Unlaid;
        fresh.replace(other);
        assert_eq!(fresh.active_size(), Some(CGSize::new(1000.0, 1000.0)));
    }
}
