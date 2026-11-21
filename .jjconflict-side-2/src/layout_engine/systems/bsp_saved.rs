//! A BSP layout system's saved form from the core's trees (version 2), written by rift: the
//! core has no serde, so this is the boundary where its trees become words on disk and words
//! on disk become its trees again, checked. Layouts keep rift's ids, which the rest of the saved
//! state refers to. (Rift's NodeId form, version 1, is still read: see `SavedBsp`.)

use rift_core::layout::bsp_tree::{
    Content, Direction, Fill, Layout, Layouts, Leaf, Memory, Node, Orientation, Preselect, Ratio,
    Side, Split, Tabs,
};
use serde::{Deserialize, Serialize};

use crate::actor::app::WindowId;
use crate::layout_engine::LayoutId;

/// The saved trees: the version, then each layout under its id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedTrees {
    version: Version,
    layouts: Vec<(LayoutId, SavedNode)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Version {
    #[serde(rename = "2")]
    Two,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum SavedNode {
    Split {
        orientation: SavedOrientation,
        ratio: f32,
        memory: SavedMemory,
        first: Box<SavedNode>,
        second: Box<SavedNode>,
    },
    Leaf {
        content: SavedContent,
        preselect: Option<SavedDirection>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SavedOrientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SavedDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SavedMemory {
    Stops,
    First,
    Second,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SavedFill {
    Tiled,
    Screen,
    WithinGaps,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum SavedContent {
    Empty,
    Window {
        window: WindowId,
        fill: SavedFill,
    },
    Stack {
        before: Vec<WindowId>,
        top: WindowId,
        after: Vec<WindowId>,
        fill: SavedFill,
    },
}

/// A saved tree the core cannot hold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SavedTreeError {
    /// A split ratio outside 0.05..=0.95, or not a number.
    Ratio(f32),
}

impl std::fmt::Display for SavedTreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a saved BSP layout the core cannot hold: {self:?}")
    }
}

impl SavedTrees {
    /// The core's trees as saved.
    pub(crate) fn of(layouts: &Layouts<LayoutId, WindowId>) -> Self {
        SavedTrees {
            version: Version::Two,
            layouts: layouts.iter().map(|(id, tree)| (*id, saved_node(&tree.root))).collect(),
        }
    }

    /// The core's trees from the saved ones (the windows left out are not saved: none are).
    pub(crate) fn into_layouts(self) -> Result<Layouts<LayoutId, WindowId>, SavedTreeError> {
        let Version::Two = self.version;
        let mut core = Layouts::default();
        for (id, node) in self.layouts {
            let mut failed = None;
            core.adopt(id, |tags| match core_node(node, tags) {
                Ok(root) => Layout { root },
                Err(problem) => {
                    failed = Some(problem);
                    Layout::new(tags)
                }
            });
            if let Some(problem) = failed {
                return Err(problem);
            }
        }
        Ok(core)
    }
}

fn saved_node(node: &Node<WindowId>) -> SavedNode {
    match node {
        Node::Split(split) => SavedNode::Split {
            orientation: match split.orientation {
                Orientation::Horizontal => SavedOrientation::Horizontal,
                Orientation::Vertical => SavedOrientation::Vertical,
            },
            ratio: split.ratio.get(),
            memory: match split.memory {
                Memory::Stops => SavedMemory::Stops,
                Memory::Toward(Side::First) => SavedMemory::First,
                Memory::Toward(Side::Second) => SavedMemory::Second,
            },
            first: Box::new(saved_node(&split.first)),
            second: Box::new(saved_node(&split.second)),
        },
        Node::Leaf(leaf) => SavedNode::Leaf {
            content: match &leaf.content {
                Content::Empty => SavedContent::Empty,
                Content::Window { window, fill } => SavedContent::Window {
                    window: *window,
                    fill: saved_fill(*fill),
                },
                Content::Stack { tabs, fill } => SavedContent::Stack {
                    before: tabs.before.clone(),
                    top: tabs.top,
                    after: tabs.after.clone(),
                    fill: saved_fill(*fill),
                },
            },
            preselect: match leaf.preselect {
                Preselect::Unset => None,
                Preselect::Toward(direction) => Some(match direction {
                    Direction::Left => SavedDirection::Left,
                    Direction::Right => SavedDirection::Right,
                    Direction::Up => SavedDirection::Up,
                    Direction::Down => SavedDirection::Down,
                }),
            },
        },
    }
}

fn saved_fill(fill: Fill) -> SavedFill {
    match fill {
        Fill::Tiled => SavedFill::Tiled,
        Fill::Screen => SavedFill::Screen,
        Fill::WithinGaps => SavedFill::WithinGaps,
    }
}

fn core_fill(fill: SavedFill) -> Fill {
    match fill {
        SavedFill::Tiled => Fill::Tiled,
        SavedFill::Screen => Fill::Screen,
        SavedFill::WithinGaps => Fill::WithinGaps,
    }
}

fn core_node(
    node: SavedNode,
    tags: &mut rift_core::layout::bsp_tree::Tags,
) -> Result<Node<WindowId>, SavedTreeError> {
    Ok(match node {
        SavedNode::Split {
            orientation,
            ratio,
            memory,
            first,
            second,
        } => Node::Split(Box::new(Split {
            tag: tags.mint(),
            orientation: match orientation {
                SavedOrientation::Horizontal => Orientation::Horizontal,
                SavedOrientation::Vertical => Orientation::Vertical,
            },
            ratio: Ratio::clamped(ratio)
                .filter(|kept| kept.get() == ratio)
                .ok_or(SavedTreeError::Ratio(ratio))?,
            memory: match memory {
                SavedMemory::Stops => Memory::Stops,
                SavedMemory::First => Memory::Toward(Side::First),
                SavedMemory::Second => Memory::Toward(Side::Second),
            },
            first: core_node(*first, tags)?,
            second: core_node(*second, tags)?,
        })),
        SavedNode::Leaf { content, preselect } => Node::Leaf(Leaf {
            tag: tags.mint(),
            content: match content {
                SavedContent::Empty => Content::Empty,
                SavedContent::Window { window, fill } => {
                    Content::Window { window, fill: core_fill(fill) }
                }
                SavedContent::Stack { before, top, after, fill } => Content::Stack {
                    tabs: Tabs { before, top, after },
                    fill: core_fill(fill),
                },
            },
            preselect: match preselect {
                None => Preselect::Unset,
                Some(direction) => Preselect::Toward(match direction {
                    SavedDirection::Left => Direction::Left,
                    SavedDirection::Right => Direction::Right,
                    SavedDirection::Up => Direction::Up,
                    SavedDirection::Down => Direction::Down,
                }),
            },
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_engine::systems::{BspLayoutSystem, LayoutSystem};

    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    #[test]
    fn the_cores_trees_survive_the_saved_form_and_a_bad_ratio_is_refused() {
        let mut system = BspLayoutSystem::default();
        let layout = system.create_layout();
        for i in 1..=5 {
            system.add_window_after_selection(layout, w(i));
        }
        assert!(system.apply_window_drop(
            layout,
            w(5),
            w(4),
            crate::layout_engine::WindowDropAction::Stack
        ));
        assert_eq!(system.stack_members(layout, w(4)).len(), 2);
        let other = system.create_layout();
        system.add_window_after_selection(other, w(9));
        let core = system.core_layouts().expect("the system's trees parse");
        let text = ron::to_string(&SavedTrees::of(&core)).unwrap();
        let read: SavedTrees = ron::from_str(&text).unwrap();
        let back = read.into_layouts().expect("the saved trees load");
        for (id, tree) in core.iter() {
            assert!(
                back.layout(*id).is_some_and(|held| held.same_as(tree)),
                "layout {id:?}"
            );
        }
        assert_eq!(back.iter().count(), 2);
        let mut bad = SavedTrees::of(&core);
        if let Some((_, SavedNode::Split { ratio, .. })) = bad.layouts.first_mut() {
            *ratio = 1.5;
        }
        assert_eq!(bad.into_layouts().err(), Some(SavedTreeError::Ratio(1.5)));
    }
}
