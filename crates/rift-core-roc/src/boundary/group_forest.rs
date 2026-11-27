//! The group forest's layout systems (`rift_core::layout::group_forest`: the traditional and
//! master-stack systems) as the platform's boundary types and back, and the boxes holding them
//! in Roc. Unsafe operations on Roc memory as in the rest of [`super`].

use std::mem::ManuallyDrop;

use rift_core::layout::WindowConstraints;
use rift_core::layout::floating::{self, Floating};
use rift_core::layout::group_forest::{
    Arrangement, Bars, Beyond, Body, Branch, Child, Container, ContainerView, Drawn, DropAction,
    Edit, Fill, Geometry, GroupBar, Held, InsertionPoint, Kind, Mark, MasterSettings, MasterSide,
    Members, Memory, NewSibling, NewWindowPlacement, Node, Pass, Place, RegionInsertion,
    ResizeAlong, Role, Row,
};
use rift_core::plain::{Rect, WindowId};

use super::stack_tree::default_orientation;
use super::{
    frames, from_insertion, from_orientation, from_rect, from_side, from_tree_answer, from_window,
    gaps, insertion, list, rect, stack_line, traditional_fill, tree_direction, tree_focus,
    tree_orientation, tree_query, tree_side, window_constraints,
};
use crate::abi;
use crate::bsp_tree::{Answer, LayoutKey, NodeKey, Saved};
use crate::group_forest::{
    FloatingAnswer, FloatingEvent, FloatingView, ForestEvent, ForestView, MasterStackEvent,
    MasterStackView, Projected, TraditionalEvent, TraditionalView,
};

// ---------------------------------------------------------------------------------------------
// Settings.

fn new_sibling(sibling: NewSibling) -> abi::BoundaryGroupNewSibling {
    match sibling {
        NewSibling::AverageWeight => abi::BoundaryGroupNewSibling::AverageWeight,
        NewSibling::HalvesSelection => abi::BoundaryGroupNewSibling::HalvesSelection,
    }
}

fn from_new_sibling(sibling: abi::BoundaryGroupNewSibling) -> NewSibling {
    match sibling {
        abi::BoundaryGroupNewSibling::AverageWeight => NewSibling::AverageWeight,
        abi::BoundaryGroupNewSibling::HalvesSelection => NewSibling::HalvesSelection,
    }
}

fn traditional_settings(
    point: InsertionPoint,
    sibling: NewSibling,
) -> abi::BoundaryTraditionalSettings {
    abi::BoundaryTraditionalSettings {
        insertion: insertion(point),
        new_sibling: new_sibling(sibling),
    }
}

fn arrangement(arrangement: Arrangement) -> abi::BoundaryMasterArrangement {
    use abi::BoundaryMasterArrangementTag as Tag;
    match arrangement {
        Arrangement::Derived => union!(
            BoundaryMasterArrangement,
            BoundaryMasterArrangementPayload.derived,
            Tag::Derived
        ),
        Arrangement::Set(orientation) => union!(
            BoundaryMasterArrangement,
            BoundaryMasterArrangementPayload.fixed(tree_orientation(orientation)),
            Tag::Fixed
        ),
    }
}

fn from_arrangement(arrangement: abi::BoundaryMasterArrangement) -> Arrangement {
    match arrangement.tag {
        abi::BoundaryMasterArrangementTag::Derived => Arrangement::Derived,
        // SAFETY: the tag says the payload is the orientation.
        abi::BoundaryMasterArrangementTag::Fixed => Arrangement::Set(from_orientation(unsafe {
            *arrangement.borrow_payload_fixed_unchecked()
        })),
    }
}

fn master_settings(settings: MasterSettings) -> abi::BoundaryMasterSettings {
    abi::BoundaryMasterSettings {
        insertion: match settings.insertion {
            RegionInsertion::NextToSelection => abi::BoundaryMasterInsertion::NextToSelection,
            RegionInsertion::EndOfTree => abi::BoundaryMasterInsertion::EndOfTree,
            RegionInsertion::Front => abi::BoundaryMasterInsertion::Front,
        },
        ratio: settings.ratio,
        count: settings.count as u64,
        side: match settings.side {
            MasterSide::Left => abi::BoundaryMasterSide::Left,
            MasterSide::Right => abi::BoundaryMasterSide::Right,
            MasterSide::Top => abi::BoundaryMasterSide::Top,
            MasterSide::Bottom => abi::BoundaryMasterSide::Bottom,
        },
        placement: match settings.placement {
            NewWindowPlacement::Master => abi::BoundaryMasterPlacement::Master,
            NewWindowPlacement::Stack => abi::BoundaryMasterPlacement::Stack,
            NewWindowPlacement::Focused => abi::BoundaryMasterPlacement::Focused,
        },
        master: arrangement(settings.master),
        stack: arrangement(settings.stack),
    }
}

fn from_master_settings(settings: abi::BoundaryMasterSettings) -> MasterSettings {
    MasterSettings {
        insertion: match settings.insertion {
            abi::BoundaryMasterInsertion::NextToSelection => RegionInsertion::NextToSelection,
            abi::BoundaryMasterInsertion::EndOfTree => RegionInsertion::EndOfTree,
            abi::BoundaryMasterInsertion::Front => RegionInsertion::Front,
        },
        ratio: settings.ratio,
        count: settings.count as usize,
        side: match settings.side {
            abi::BoundaryMasterSide::Left => MasterSide::Left,
            abi::BoundaryMasterSide::Right => MasterSide::Right,
            abi::BoundaryMasterSide::Top => MasterSide::Top,
            abi::BoundaryMasterSide::Bottom => MasterSide::Bottom,
        },
        placement: match settings.placement {
            abi::BoundaryMasterPlacement::Master => NewWindowPlacement::Master,
            abi::BoundaryMasterPlacement::Stack => NewWindowPlacement::Stack,
            abi::BoundaryMasterPlacement::Focused => NewWindowPlacement::Focused,
        },
        master: from_arrangement(settings.master),
        stack: from_arrangement(settings.stack),
    }
}

// ---------------------------------------------------------------------------------------------
// The forest's nodes.

fn kind(kind: Kind) -> abi::BoundaryGroupKind {
    use abi::BoundaryGroupKindTag as Tag;
    match kind {
        Kind::Split(orientation) => union!(
            BoundaryGroupKind,
            BoundaryGroupKindPayload.split(tree_orientation(orientation)),
            Tag::Split
        ),
        Kind::Stack(orientation) => union!(
            BoundaryGroupKind,
            BoundaryGroupKindPayload.stack(tree_orientation(orientation)),
            Tag::Stack
        ),
    }
}

fn from_kind(kind: abi::BoundaryGroupKind) -> Kind {
    match kind.tag {
        // SAFETY (each arm): the tag says the payload is the orientation.
        abi::BoundaryGroupKindTag::Split => Kind::Split(from_orientation(unsafe {
            *kind.borrow_payload_split_unchecked()
        })),
        abi::BoundaryGroupKindTag::Stack => Kind::Stack(from_orientation(unsafe {
            *kind.borrow_payload_stack_unchecked()
        })),
    }
}

fn from_fill(fill: abi::BoundaryFill) -> Fill {
    match fill {
        abi::BoundaryFill::Tiled => Fill::Tiled,
        abi::BoundaryFill::Screen => Fill::Screen,
        abi::BoundaryFill::WithinGaps => Fill::WithinGaps,
    }
}

fn container(container: &Container<NodeKey>, host: &abi::RocHost) -> abi::BoundaryGroupContainer {
    abi::BoundaryGroupContainer {
        kind: kind(container.kind),
        members: match &container.members {
            Members::Children(children) => {
                let children: Vec<u64> = children.iter().map(|child| child.0).collect();
                union!(
                    BoundaryGroupMembers,
                    BoundaryGroupMembersPayload.children(list(&children, host)),
                    abi::BoundaryGroupMembersTag::Children
                )
            }
            Members::Regions {
                order: [first, second],
                primary,
            } => union!(
                BoundaryGroupMembers,
                BoundaryGroupMembersPayload.regions(abi::BoundaryGroupRegions {
                    first: first.0,
                    second: second.0,
                    primary: tree_side(*primary),
                }),
                abi::BoundaryGroupMembersTag::Regions
            ),
        },
        memory: match container.memory {
            Memory::Unset => union!(
                BoundaryGroupMemory,
                BoundaryGroupMemoryPayload.unset,
                abi::BoundaryGroupMemoryTag::Unset
            ),
            Memory::Toward(child) => union!(
                BoundaryGroupMemory,
                BoundaryGroupMemoryPayload.toward(child.0),
                abi::BoundaryGroupMemoryTag::Toward
            ),
            Memory::Stops(child) => union!(
                BoundaryGroupMemory,
                BoundaryGroupMemoryPayload.stops(child.0),
                abi::BoundaryGroupMemoryTag::Stops
            ),
        },
    }
}

fn from_container(container: &abi::BoundaryGroupContainer) -> Container<NodeKey> {
    let members = match container.members.tag {
        abi::BoundaryGroupMembersTag::Children => {
            // SAFETY: the tag says the payload is the children; the list is only read.
            let children = unsafe { container.members.borrow_payload_children_unchecked() };
            Members::Children(children.as_slice().iter().map(|&child| NodeKey(child)).collect())
        }
        abi::BoundaryGroupMembersTag::Regions => {
            // SAFETY: the tag says the payload is the regions.
            let regions = unsafe { container.members.borrow_payload_regions_unchecked() };
            Members::Regions {
                order: [NodeKey(regions.first), NodeKey(regions.second)],
                primary: from_side(regions.primary),
            }
        }
    };
    let memory = match container.memory.tag {
        abi::BoundaryGroupMemoryTag::Unset => Memory::Unset,
        // SAFETY (each arm): the tag says the payload is the child's key.
        abi::BoundaryGroupMemoryTag::Toward => Memory::Toward(NodeKey(unsafe {
            *container.memory.borrow_payload_toward_unchecked()
        })),
        abi::BoundaryGroupMemoryTag::Stops => Memory::Stops(NodeKey(unsafe {
            *container.memory.borrow_payload_stops_unchecked()
        })),
    };
    Container {
        kind: from_kind(container.kind),
        members,
        memory,
    }
}

fn child_place(at: Child<NodeKey>) -> abi::BoundaryGroupChildPlace {
    abi::BoundaryGroupChildPlace {
        parent: at.parent.0,
        size: at.size,
    }
}

fn from_child_place(at: &abi::BoundaryGroupChildPlace) -> Child<NodeKey> {
    Child {
        parent: NodeKey(at.parent),
        size: at.size,
    }
}

fn node(node: &Node<NodeKey, u64>, host: &abi::RocHost) -> abi::BoundaryGroupTreeNode {
    abi::BoundaryGroupTreeNode {
        fill: traditional_fill(node.fill),
        body: match &node.body {
            Body::Window { at, window, stamp } => union!(
                BoundaryGroupBody,
                BoundaryGroupBodyPayload.window(abi::BoundaryGroupLeaf {
                    at: child_place(*at),
                    window: *window,
                    stamp: *stamp,
                }),
                abi::BoundaryGroupBodyTag::Window
            ),
            Body::Container { place, container: held } => union!(
                BoundaryGroupBody,
                BoundaryGroupBodyPayload.container(abi::BoundaryGroupContainerNode {
                    place: match *place {
                        Place::Root => union!(
                            BoundaryGroupPlace,
                            BoundaryGroupPlacePayload.root,
                            abi::BoundaryGroupPlaceTag::Root
                        ),
                        Place::Child(at) => union!(
                            BoundaryGroupPlace,
                            BoundaryGroupPlacePayload.child(child_place(at)),
                            abi::BoundaryGroupPlaceTag::Child
                        ),
                    },
                    container: container(held, host),
                }),
                abi::BoundaryGroupBodyTag::Container
            ),
        },
    }
}

fn from_node(node: &abi::BoundaryGroupTreeNode) -> Node<NodeKey, u64> {
    let body = match node.body.tag {
        abi::BoundaryGroupBodyTag::Window => {
            // SAFETY: the tag says the payload is the leaf.
            let leaf = unsafe { node.body.borrow_payload_window_unchecked() };
            Body::Window {
                at: from_child_place(&leaf.at),
                window: leaf.window,
                stamp: leaf.stamp,
            }
        }
        abi::BoundaryGroupBodyTag::Container => {
            // SAFETY: the tag says the payload is the container node.
            let held = unsafe { node.body.borrow_payload_container_unchecked() };
            let place = match held.place.tag {
                abi::BoundaryGroupPlaceTag::Root => Place::Root,
                // SAFETY: the tag says the payload is the child's place.
                abi::BoundaryGroupPlaceTag::Child => Place::Child(from_child_place(unsafe {
                    held.place.borrow_payload_child_unchecked()
                })),
            };
            Body::Container {
                place,
                container: from_container(&held.container),
            }
        }
    };
    Node {
        fill: from_fill(node.fill),
        body,
    }
}

// ---------------------------------------------------------------------------------------------
// Seeds and views.

/// A forest as the platform holds it, inside either system's seed.
fn forest_seed(view: &ForestView, host: &abi::RocHost) -> abi::BoundaryGroupForestSeed {
    let layouts: Vec<abi::BoundaryBspLayoutSlot> = view
        .layouts
        .iter()
        .map(|(held, version)| match held {
            Some(root) => union!(
                BoundaryBspLayoutSlot,
                BoundaryBspLayoutSlotPayload.occupied(abi::BoundaryBspLayoutHeld {
                    version: *version,
                    root: root.0,
                }),
                abi::BoundaryBspLayoutSlotTag::Occupied
            ),
            None => union!(
                BoundaryBspLayoutSlot,
                BoundaryBspLayoutSlotPayload.vacant(*version),
                abi::BoundaryBspLayoutSlotTag::Vacant
            ),
        })
        .collect();
    let nodes: Vec<abi::BoundaryGroupNodeSlot> = view
        .nodes
        .iter()
        .map(|(held, version)| match held {
            Some(held) => union!(
                BoundaryGroupNodeSlot,
                BoundaryGroupNodeSlotPayload.occupied(abi::BoundaryGroupNodeHeld {
                    version: *version,
                    node: node(held, host),
                }),
                abi::BoundaryGroupNodeSlotTag::Occupied
            ),
            None => union!(
                BoundaryGroupNodeSlot,
                BoundaryGroupNodeSlotPayload.vacant(*version),
                abi::BoundaryGroupNodeSlotTag::Vacant
            ),
        })
        .collect();
    abi::BoundaryGroupForestSeed {
        layouts: list(&layouts, host),
        layout_free: list(&view.layout_free, host),
        nodes: list(&nodes, host),
        node_free: list(&view.node_free, host),
        left_out: list(&view.left_out, host),
        next_stamp: view.next_stamp,
    }
}

/// The forest a seed holds, read in full (the caller releases the seed).
fn from_forest_seed(seed: &abi::BoundaryGroupForestSeed) -> ForestView {
    let layouts: Vec<Saved<NodeKey>> = seed
        .layouts
        .as_slice()
        .iter()
        .map(|slot| match slot.tag {
            abi::BoundaryBspLayoutSlotTag::Occupied => {
                // SAFETY: the tag says the payload is the held layout.
                let held = unsafe { slot.borrow_payload_occupied_unchecked() };
                (Some(NodeKey(held.root)), held.version)
            }
            // SAFETY: the tag says the payload is the vacant version.
            abi::BoundaryBspLayoutSlotTag::Vacant => {
                (None, unsafe { *slot.borrow_payload_vacant_unchecked() })
            }
        })
        .collect();
    let nodes = seed
        .nodes
        .as_slice()
        .iter()
        .map(|slot| match slot.tag {
            abi::BoundaryGroupNodeSlotTag::Occupied => {
                // SAFETY: the tag says the payload is the held node.
                let held = unsafe { slot.borrow_payload_occupied_unchecked() };
                (Some(from_node(&held.node)), held.version)
            }
            // SAFETY: the tag says the payload is the vacant version.
            abi::BoundaryGroupNodeSlotTag::Vacant => {
                (None, unsafe { *slot.borrow_payload_vacant_unchecked() })
            }
        })
        .collect();
    ForestView {
        layouts,
        layout_free: seed.layout_free.as_slice().to_vec(),
        nodes,
        node_free: seed.node_free.as_slice().to_vec(),
        left_out: seed.left_out.as_slice().to_vec(),
        next_stamp: seed.next_stamp,
    }
}

/// A traditional system as the platform holds it, for `roc_traditional_tree_seed`.
fn traditional_seed(view: &TraditionalView, host: &abi::RocHost) -> abi::BoundaryTraditionalSeed {
    abi::BoundaryTraditionalSeed {
        forest: forest_seed(&view.forest, host),
        settings: traditional_settings(view.insertion, view.new_sibling),
    }
}

fn from_traditional_seed(seed: &abi::BoundaryTraditionalSeed) -> TraditionalView {
    TraditionalView {
        forest: from_forest_seed(&seed.forest),
        insertion: from_insertion(seed.settings.insertion),
        new_sibling: from_new_sibling(seed.settings.new_sibling),
    }
}

/// A master-stack system as the platform holds it, for `roc_master_stack_seed`.
fn master_stack_seed(view: &MasterStackView, host: &abi::RocHost) -> abi::BoundaryMasterStackSeed {
    abi::BoundaryMasterStackSeed {
        forest: forest_seed(&view.forest, host),
        settings: master_settings(view.settings),
    }
}

fn from_master_stack_seed(seed: &abi::BoundaryMasterStackSeed) -> MasterStackView {
    MasterStackView {
        forest: from_forest_seed(&seed.forest),
        settings: from_master_settings(seed.settings),
    }
}

// ---------------------------------------------------------------------------------------------
// Events.

fn drop_action(action: DropAction) -> abi::BoundaryGroupDropAction {
    use abi::BoundaryGroupDropActionTag as Tag;
    match action {
        DropAction::Swap => {
            union!(
                BoundaryGroupDropAction,
                BoundaryGroupDropActionPayload.swap,
                Tag::Swap
            )
        }
        DropAction::Stack => {
            union!(
                BoundaryGroupDropAction,
                BoundaryGroupDropActionPayload.stack,
                Tag::Stack
            )
        }
        DropAction::Insert(direction) => union!(
            BoundaryGroupDropAction,
            BoundaryGroupDropActionPayload.insert(tree_direction(direction)),
            Tag::Insert
        ),
    }
}

fn edit(edit: Edit<u64>) -> abi::BoundaryGroupEdit {
    use abi::BoundaryGroupEditTag as Tag;
    match edit {
        Edit::Move { direction, beyond } => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.r#move(abi::BoundaryStackMove {
                direction: tree_direction(direction),
                beyond: match beyond {
                    Beyond::AnotherDisplay => abi::BoundaryStackBeyond::AnotherDisplay,
                    Beyond::Nothing => abi::BoundaryStackBeyond::Nothing,
                },
            }),
            Tag::Move
        ),
        Edit::Split(asked) => {
            union!(
                BoundaryGroupEdit,
                BoundaryGroupEditPayload.split(kind(asked)),
                Tag::Split
            )
        }
        Edit::Fullscreen { within_gaps } => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.fullscreen(if within_gaps {
                abi::BoundaryBspFullscreen::WithinGaps
            } else {
                abi::BoundaryBspFullscreen::Screen
            }),
            Tag::Fullscreen
        ),
        Edit::Swap(a, b) => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.swap(abi::BoundaryBspPair { a, b }),
            Tag::Swap
        ),
        Edit::ReorderTab { window, target } => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.reorder_tab(abi::BoundaryStackTab { window, target }),
            Tag::ReorderTab
        ),
        Edit::Drop { source, target, action } => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.drop(abi::BoundaryGroupDrop {
                source,
                target,
                action: drop_action(action),
            }),
            Tag::Drop
        ),
        Edit::Join(direction) => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.join(tree_direction(direction)),
            Tag::Join
        ),
        Edit::ConsumeOrExpel(direction) => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.consume_or_expel(tree_direction(direction)),
            Tag::ConsumeOrExpel
        ),
        Edit::Stack(orientation) => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.stack(default_orientation(orientation)),
            Tag::Stack
        ),
        Edit::Unstack(orientation) => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.unstack(default_orientation(orientation)),
            Tag::Unstack
        ),
        Edit::Unjoin => union!(BoundaryGroupEdit, BoundaryGroupEditPayload.unjoin, Tag::Unjoin),
        Edit::Resize { amount, along } => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.resize(abi::BoundaryBspResize {
                amount,
                along: match along {
                    ResizeAlong::Horizontal => abi::BoundaryBspResizeAlong::Horizontal,
                    ResizeAlong::Vertical => abi::BoundaryBspResizeAlong::Vertical,
                    ResizeAlong::Smart => abi::BoundaryBspResizeAlong::Smart,
                },
            }),
            Tag::Resize
        ),
        Edit::Rebalance => {
            union!(
                BoundaryGroupEdit,
                BoundaryGroupEditPayload.rebalance,
                Tag::Rebalance
            )
        }
        Edit::ToggleOrientation => union!(
            BoundaryGroupEdit,
            BoundaryGroupEditPayload.toggle_orientation,
            Tag::ToggleOrientation
        ),
    }
}

fn edit_in(layout: LayoutKey, asked: Edit<u64>) -> abi::BoundaryGroupEditIn {
    abi::BoundaryGroupEditIn {
        layout: layout.0,
        edit: edit(asked),
    }
}

fn move_query(
    layout: LayoutKey,
    window: u64,
    direction: rift_core::layout::group_forest::Direction,
) -> abi::BoundaryBspMoveQuery {
    abi::BoundaryBspMoveQuery {
        layout: layout.0,
        window,
        direction: tree_direction(direction),
    }
}

fn forest_event(event: ForestEvent, host: &abi::RocHost) -> abi::BoundaryGroupForestEvent {
    use abi::BoundaryGroupForestEventTag as Tag;
    use rift_core::layout::group_forest::{Participation, Resize};
    let window_in =
        |layout: LayoutKey, window| abi::BoundaryBspWindowIn { layout: layout.0, window };
    match event {
        ForestEvent::CloneLayout(layout) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.clone_layout(layout.0),
            Tag::CloneLayout
        ),
        ForestEvent::RemoveLayout(layout) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.remove_layout(layout.0),
            Tag::RemoveLayout
        ),
        ForestEvent::Participation(window, participation) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.participation(abi::BoundaryBspParticipation {
                window,
                participation: match participation {
                    Participation::TakesPart => abi::BoundaryBspTakesPart::TakesPart,
                    Participation::LeftOut => abi::BoundaryBspTakesPart::LeftOut,
                },
            }),
            Tag::Participation
        ),
        ForestEvent::ReplaceWindow { from, to } => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload
                .replace_window(abi::BoundaryBspReplacement { from, to }),
            Tag::ReplaceWindow
        ),
        ForestEvent::ResizeWindow(layout, window, Resize { old, new, screen, tiling }) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.resize_window(abi::BoundaryBspNativeResize {
                layout: layout.0,
                window,
                old: rect(old),
                new: rect(new),
                screen: rect(screen),
                tiling: rect(tiling),
            }),
            Tag::ResizeWindow
        ),
        ForestEvent::ContainsLayout(layout) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.contains_layout(layout.0),
            Tag::ContainsLayout
        ),
        ForestEvent::WindowParticipates(window) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.window_participates(window),
            Tag::WindowParticipates
        ),
        ForestEvent::SelectedWindow(layout) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.selected_window(layout.0),
            Tag::SelectedWindow
        ),
        ForestEvent::SelectionContainsWindow(layout, window) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.selection_contains_window(window_in(layout, window)),
            Tag::SelectionContainsWindow
        ),
        ForestEvent::WindowInDirection(layout, direction) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.window_in_direction(abi::BoundaryBspLayoutDirection {
                layout: layout.0,
                direction: tree_direction(direction),
            }),
            Tag::WindowInDirection
        ),
        ForestEvent::ContainsWindow(layout, window) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.contains_window(window_in(layout, window)),
            Tag::ContainsWindow
        ),
        ForestEvent::HasAnyFullscreenNode(layout) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.has_any_fullscreen_node(layout.0),
            Tag::HasAnyFullscreenNode
        ),
        ForestEvent::HasWindowsForApp(layout, windows) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.has_windows_for_app(abi::BoundaryBspAppIn {
                layout: layout.0,
                windows: list(&windows, host),
            }),
            Tag::HasWindowsForApp
        ),
        ForestEvent::ParentOfSelectionIsStacked(layout) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.parent_of_selection_is_stacked(layout.0),
            Tag::ParentOfSelectionIsStacked
        ),
        ForestEvent::Windows(layout, query) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.windows(abi::BoundaryBspWindowsQuery {
                layout: layout.0,
                query: tree_query(query),
            }),
            Tag::Windows
        ),
        ForestEvent::WindowSlot(layout, window) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.window_slot(window_in(layout, window)),
            Tag::WindowSlot
        ),
        ForestEvent::Focus(layout, focus) => union!(
            BoundaryGroupForestEvent,
            BoundaryGroupForestEventPayload.focus(abi::BoundaryBspFocusIn {
                layout: layout.0,
                focus: tree_focus(focus),
            }),
            Tag::Focus
        ),
    }
}

fn traditional_event(
    event: TraditionalEvent,
    host: &abi::RocHost,
) -> abi::BoundaryTraditionalEvent {
    use abi::BoundaryTraditionalEventTag as Tag;
    match event {
        TraditionalEvent::Forest(event) => union!(
            BoundaryTraditionalEvent,
            BoundaryTraditionalEventPayload.forest(forest_event(event, host)),
            Tag::Forest
        ),
        TraditionalEvent::CreateLayout => union!(
            BoundaryTraditionalEvent,
            BoundaryTraditionalEventPayload.create_layout,
            Tag::CreateLayout
        ),
        TraditionalEvent::Settings(point, sibling) => union!(
            BoundaryTraditionalEvent,
            BoundaryTraditionalEventPayload.settings(traditional_settings(point, sibling)),
            Tag::Settings
        ),
        TraditionalEvent::AddWindow(layout, window, toward) => union!(
            BoundaryTraditionalEvent,
            BoundaryTraditionalEventPayload.add_window(abi::BoundaryTraditionalAdd {
                layout: layout.0,
                window,
                toward: match toward {
                    None => union!(
                        BoundaryTraditionalToward,
                        BoundaryTraditionalTowardPayload.after_selection,
                        abi::BoundaryTraditionalTowardTag::AfterSelection
                    ),
                    Some(direction) => union!(
                        BoundaryTraditionalToward,
                        BoundaryTraditionalTowardPayload.toward(tree_direction(direction)),
                        abi::BoundaryTraditionalTowardTag::Toward
                    ),
                },
            }),
            Tag::AddWindow
        ),
        TraditionalEvent::RemoveWindow(window) => union!(
            BoundaryTraditionalEvent,
            BoundaryTraditionalEventPayload.remove_window(window),
            Tag::RemoveWindow
        ),
        TraditionalEvent::RemoveAndRebalance(window) => union!(
            BoundaryTraditionalEvent,
            BoundaryTraditionalEventPayload.remove_and_rebalance(window),
            Tag::RemoveAndRebalance
        ),
        TraditionalEvent::CloseApp(windows) => union!(
            BoundaryTraditionalEvent,
            BoundaryTraditionalEventPayload.close_app(list(&windows, host)),
            Tag::CloseApp
        ),
        TraditionalEvent::CanMoveWindow(layout, window, direction) => union!(
            BoundaryTraditionalEvent,
            BoundaryTraditionalEventPayload.can_move_window(move_query(layout, window, direction)),
            Tag::CanMoveWindow
        ),
        TraditionalEvent::Edit(layout, asked) => union!(
            BoundaryTraditionalEvent,
            BoundaryTraditionalEventPayload.edit(edit_in(layout, asked)),
            Tag::Edit
        ),
    }
}

fn master_stack_event(
    event: MasterStackEvent,
    host: &abi::RocHost,
) -> abi::BoundaryMasterStackEvent {
    use abi::BoundaryMasterStackEventTag as Tag;
    match event {
        MasterStackEvent::Forest(event) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.forest(forest_event(event, host)),
            Tag::Forest
        ),
        MasterStackEvent::CreateLayout => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.create_layout,
            Tag::CreateLayout
        ),
        MasterStackEvent::Settings(settings) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.settings(master_settings(settings)),
            Tag::Settings
        ),
        MasterStackEvent::AddWindow(layout, window) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload
                .add_window(abi::BoundaryBspWindowIn { layout: layout.0, window }),
            Tag::AddWindow
        ),
        MasterStackEvent::RemoveWindow(window) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.remove_window(window),
            Tag::RemoveWindow
        ),
        MasterStackEvent::CloseApp(windows) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.close_app(list(&windows, host)),
            Tag::CloseApp
        ),
        MasterStackEvent::AdjustRatio(delta) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.adjust_ratio(delta),
            Tag::AdjustRatio
        ),
        MasterStackEvent::AdjustCount(delta) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.adjust_count(delta),
            Tag::AdjustCount
        ),
        MasterStackEvent::Promote(layout) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.promote(layout.0),
            Tag::Promote
        ),
        MasterStackEvent::Rotate(layout, backwards) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload
                .rotate(abi::BoundaryMasterRotate { layout: layout.0, backwards }),
            Tag::Rotate
        ),
        MasterStackEvent::SwapMasterStack(layout) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.swap_master_stack(layout.0),
            Tag::SwapMasterStack
        ),
        MasterStackEvent::CanMoveWindow(layout, window, direction) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.can_move_window(move_query(layout, window, direction)),
            Tag::CanMoveWindow
        ),
        MasterStackEvent::Edit(layout, asked) => union!(
            BoundaryMasterStackEvent,
            BoundaryMasterStackEventPayload.edit(edit_in(layout, asked)),
            Tag::Edit
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// Layout questions (`Forest::shape` and its geometry, `project`, `container_view`, `rows`).

/// A layout pass as the platform asks it: the layout, the pass, the constraints the caller knows.
fn pass_question(
    layout: LayoutKey,
    pass: Pass,
    known: &[(u64, WindowConstraints)],
    host: &abi::RocHost,
) -> abi::BoundaryStackFramesQuestion {
    let known: Vec<abi::BoundaryBspKnown> = known
        .iter()
        .map(|&(window, held)| abi::BoundaryBspKnown {
            window,
            constraints: window_constraints(held),
        })
        .collect();
    abi::BoundaryStackFramesQuestion {
        layout: layout.0,
        screen: rect(pass.screen),
        gaps: gaps(pass.gaps),
        stack_offset: pass.stack_offset,
        stack_line: stack_line(pass.stack_line),
        known: list(&known, host),
    }
}

/// A layout's frames.
pub(crate) fn frames_question(
    layout: LayoutKey,
    pass: Pass,
    known: &[(u64, WindowConstraints)],
    host: &abi::RocHost,
) -> abi::BoundaryGroupLayoutQuestion {
    use abi::BoundaryGroupLayoutQuestionTag as Tag;
    union!(
        BoundaryGroupLayoutQuestion,
        BoundaryGroupLayoutQuestionPayload.frames(pass_question(layout, pass, known, host)),
        Tag::Frames
    )
}

/// A layout pass with the group bars `bars` asks for.
pub(crate) fn project_question(
    layout: LayoutKey,
    pass: Pass,
    known: &[(u64, WindowConstraints)],
    bars: Bars,
    host: &abi::RocHost,
) -> abi::BoundaryGroupLayoutQuestion {
    use abi::BoundaryGroupLayoutQuestionTag as Tag;
    union!(
        BoundaryGroupLayoutQuestion,
        BoundaryGroupLayoutQuestionPayload.project(abi::BoundaryGroupProjectQuestion {
            pass: pass_question(layout, pass, known, host),
            bars: match bars {
                Bars::Every => abi::BoundaryGroupBars::Every,
                Bars::SelectionPath => abi::BoundaryGroupBars::SelectionPath,
            },
        }),
        Tag::Project
    )
}

/// A layout's container view.
pub(crate) fn container_question(layout: LayoutKey) -> abi::BoundaryGroupLayoutQuestion {
    use abi::BoundaryGroupLayoutQuestionTag as Tag;
    union!(
        BoundaryGroupLayoutQuestion,
        BoundaryGroupLayoutQuestionPayload.container(layout.0),
        Tag::Container
    )
}

/// A layout's drawing.
pub(crate) fn draw_question(layout: LayoutKey) -> abi::BoundaryGroupLayoutQuestion {
    use abi::BoundaryGroupLayoutQuestionTag as Tag;
    union!(
        BoundaryGroupLayoutQuestion,
        BoundaryGroupLayoutQuestionPayload.draw(layout.0),
        Tag::Draw
    )
}

/// The frames a frames question answers (Roc answers each question with its own kind).
pub(crate) fn read_frames(
    answer: abi::BoundaryGroupLayoutAnswer,
    host: &abi::RocHost,
) -> Vec<(WindowId, Rect)> {
    assert_eq!(
        answer.tag,
        abi::BoundaryGroupLayoutAnswerTag::Frames,
        "a frames question's answer"
    );
    // SAFETY: the tag says the payload is the frames, which `frames` reads and releases.
    frames(*unsafe { answer.borrow_payload_frames_unchecked() }, host)
}

/// The pass a project question answers.
pub(crate) fn read_projected(
    answer: abi::BoundaryGroupLayoutAnswer,
    host: &abi::RocHost,
) -> Projected {
    assert_eq!(
        answer.tag,
        abi::BoundaryGroupLayoutAnswerTag::Projected,
        "a pass's answer"
    );
    // SAFETY: the tag says the payload is the pass, read in full here; the answer is then
    // released (its bars' windows with it: a list's own release is shallow).
    let projected = unsafe { answer.borrow_payload_projected_unchecked() };
    let read = Projected {
        frames: projected
            .frames
            .as_slice()
            .iter()
            .map(|frame| (from_window(frame.window), from_rect(frame.frame)))
            .collect(),
        bars: projected
            .bars
            .as_slice()
            .iter()
            .map(|bar| GroupBar {
                stack: NodeKey(bar.stack),
                orientation: from_orientation(bar.orientation),
                frame: from_rect(bar.frame),
                windows: bar.windows.as_slice().to_vec(),
                selected: usize::try_from(bar.selected).expect("a bar's index fits a usize"),
            })
            .collect(),
    };
    unsafe { answer.decref(host) };
    read
}

fn from_role(role: abi::BoundaryGroupRole) -> Role {
    match role {
        abi::BoundaryGroupRole::Plain => Role::Plain,
        abi::BoundaryGroupRole::Master => Role::Master,
        abi::BoundaryGroupRole::Stack => Role::Stack,
    }
}

fn from_view(view: &abi::BoundaryGroupContainerView) -> ContainerView<NodeKey, u64> {
    ContainerView {
        node: NodeKey(view.node),
        selected: view.selected,
        weight: match view.weight.tag {
            abi::BoundaryGroupWeightTag::AtRoot => None,
            // SAFETY: the tag says the payload is the weight.
            abi::BoundaryGroupWeightTag::Weight => {
                Some(unsafe { *view.weight.borrow_payload_weight_unchecked() })
            }
        },
        fill: from_fill(view.fill),
        role: from_role(view.role),
        held: match view.held.tag {
            // SAFETY (each arm): the tag says which payload it is.
            abi::BoundaryGroupViewHeldTag::Window => {
                Held::Window(unsafe { *view.held.borrow_payload_window_unchecked() })
            }
            abi::BoundaryGroupViewHeldTag::Container => {
                let container = unsafe { view.held.borrow_payload_container_unchecked() };
                Held::Container {
                    kind: from_kind(container.kind),
                    children: container.children.as_slice().iter().map(from_view).collect(),
                }
            }
        },
    }
}

/// The container view a container question answers.
pub(crate) fn read_container(
    answer: abi::BoundaryGroupLayoutAnswer,
    host: &abi::RocHost,
) -> ContainerView<NodeKey, u64> {
    assert_eq!(
        answer.tag,
        abi::BoundaryGroupLayoutAnswerTag::Container,
        "a container view"
    );
    let view = from_view(unsafe { answer.borrow_payload_container_unchecked() });
    // SAFETY: the answer is Roc's, owned here and read in full above.
    unsafe { answer.decref(host) };
    view
}

/// The rows a draw question answers.
pub(crate) fn read_rows(
    answer: abi::BoundaryGroupLayoutAnswer,
    host: &abi::RocHost,
) -> Vec<Row<NodeKey, u64>> {
    assert_eq!(answer.tag, abi::BoundaryGroupLayoutAnswerTag::Drawn, "a drawing");
    // SAFETY: the tag says the payload is the rows, read in full here, then released.
    let rows = rows_of(unsafe { answer.borrow_payload_drawn_unchecked() });
    unsafe { answer.decref(host) };
    rows
}

/// A drawing's rows, read.
fn rows_of(rows: &abi::RocListWith<abi::BoundaryGroupRow, true>) -> Vec<Row<NodeKey, u64>> {
    rows.as_slice()
        .iter()
        .map(|row| Row {
            branches: row
                .branches
                .as_slice()
                .iter()
                .map(|glyph| match glyph {
                    abi::BoundaryGroupBranch::Blank => Branch::Blank,
                    abi::BoundaryGroupBranch::Last => Branch::Last,
                    abi::BoundaryGroupBranch::Through => Branch::Through,
                    abi::BoundaryGroupBranch::Tee => Branch::Tee,
                })
                .collect(),
            mark: match row.mark {
                abi::BoundaryGroupMark::Root => Mark::Root,
                abi::BoundaryGroupMark::Chosen => Mark::Chosen,
                abi::BoundaryGroupMark::NotChosen => Mark::NotChosen,
            },
            node: NodeKey(row.node),
            held: match row.held.tag {
                // SAFETY (each arm): the tag says which payload it is.
                abi::BoundaryGroupDrawnTag::Window => {
                    Drawn::Window(unsafe { *row.held.borrow_payload_window_unchecked() })
                }
                abi::BoundaryGroupDrawnTag::Container => {
                    let container = unsafe { row.held.borrow_payload_container_unchecked() };
                    Drawn::Container {
                        kind: from_kind(container.kind),
                        total: container.total,
                    }
                }
            },
            size: row.size,
            role: from_role(row.role),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The boxes.

/// The traditional system's box, which only Roc can read or free; only the
/// `roc_traditional_tree_*` entries make and take one. Dropping it releases the system.
pub(crate) struct TraditionalBox(abi::RocBox);

impl TraditionalBox {
    pub(crate) fn new(point: InsertionPoint, sibling: NewSibling) -> TraditionalBox {
        TraditionalBox(unsafe {
            abi::roc_traditional_tree_init(traditional_settings(point, sibling))
        })
    }

    /// The system `view` holds, which the caller has parsed (`TraditionalView::traditional`).
    pub(crate) fn seeded(view: &TraditionalView, host: &abi::RocHost) -> TraditionalBox {
        // SAFETY: the seed's lists are allocated through the host and consumed by Roc.
        TraditionalBox(unsafe { abi::roc_traditional_tree_seed(traditional_seed(view, host)) })
    }

    /// The raw box, handed to Roc (see `LifecycleBox::into_raw`).
    fn into_raw(self) -> abi::RocBox {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }

    /// One operation: the system after it, and what it answered.
    pub(crate) fn step(
        self,
        asked: TraditionalEvent,
        host: &abi::RocHost,
    ) -> (TraditionalBox, Answer) {
        // SAFETY: the box is this system's, given to the step; its successor comes back. The
        // answer is read, then released.
        let stepped = unsafe {
            abi::roc_traditional_tree_step(self.into_raw(), traditional_event(asked, host))
        };
        let answer = from_tree_answer(stepped.answer);
        unsafe { stepped.answer.decref(host) };
        (TraditionalBox(stepped.model), answer)
    }

    /// A layout question, its answer read by `read` (which releases it).
    pub(crate) fn layout<T>(
        self,
        question: abi::BoundaryGroupLayoutQuestion,
        host: &abi::RocHost,
        read: impl FnOnce(abi::BoundaryGroupLayoutAnswer, &abi::RocHost) -> T,
    ) -> (TraditionalBox, T) {
        // SAFETY: as in `step`; the question's lists are allocated through the host and consumed
        // by Roc; the answer is Roc's, handed to `read`.
        let laid = unsafe { abi::roc_traditional_tree_layout(self.into_raw(), question) };
        (TraditionalBox(laid.model), read(laid.answer, host))
    }

    /// The forest's tables and free lists, the windows left out, the settings.
    pub(crate) fn view(self, host: &abi::RocHost) -> (TraditionalBox, TraditionalView) {
        // SAFETY: as in `step`; the seed is Roc's answer, owned here, read in full, released.
        let viewed = unsafe { abi::roc_traditional_tree_view(self.into_raw()) };
        let view = from_traditional_seed(&viewed.view);
        unsafe { viewed.view.decref(host) };
        (TraditionalBox(viewed.model), view)
    }
}

impl Drop for TraditionalBox {
    fn drop(&mut self) {
        // SAFETY: the box is this system's and is never used again.
        unsafe { abi::roc_traditional_tree_release(self.0) }
    }
}

/// The master-stack system's box, which only Roc can read or free; only the
/// `roc_master_stack_*` entries make and take one. Dropping it releases the system.
pub(crate) struct MasterStackBox(abi::RocBox);

impl MasterStackBox {
    pub(crate) fn new(settings: MasterSettings) -> MasterStackBox {
        MasterStackBox(unsafe { abi::roc_master_stack_init(master_settings(settings)) })
    }

    /// The system `view` holds, which the caller has parsed (`MasterStackView::master_stack`).
    pub(crate) fn seeded(view: &MasterStackView, host: &abi::RocHost) -> MasterStackBox {
        // SAFETY: the seed's lists are allocated through the host and consumed by Roc.
        MasterStackBox(unsafe { abi::roc_master_stack_seed(master_stack_seed(view, host)) })
    }

    /// The raw box, handed to Roc (see `LifecycleBox::into_raw`).
    fn into_raw(self) -> abi::RocBox {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }

    /// One operation: the system after it, and what it answered.
    pub(crate) fn step(
        self,
        asked: MasterStackEvent,
        host: &abi::RocHost,
    ) -> (MasterStackBox, Answer) {
        // SAFETY: as in `TraditionalBox::step`.
        let stepped =
            unsafe { abi::roc_master_stack_step(self.into_raw(), master_stack_event(asked, host)) };
        let answer = from_tree_answer(stepped.answer);
        unsafe { stepped.answer.decref(host) };
        (MasterStackBox(stepped.model), answer)
    }

    /// A layout question, its answer read by `read` (which releases it).
    pub(crate) fn layout<T>(
        self,
        question: abi::BoundaryGroupLayoutQuestion,
        host: &abi::RocHost,
        read: impl FnOnce(abi::BoundaryGroupLayoutAnswer, &abi::RocHost) -> T,
    ) -> (MasterStackBox, T) {
        // SAFETY: as in `TraditionalBox::layout`.
        let laid = unsafe { abi::roc_master_stack_layout(self.into_raw(), question) };
        (MasterStackBox(laid.model), read(laid.answer, host))
    }

    /// The forest's tables and free lists, the windows left out, the settings.
    pub(crate) fn view(self, host: &abi::RocHost) -> (MasterStackBox, MasterStackView) {
        // SAFETY: as in `TraditionalBox::view`.
        let viewed = unsafe { abi::roc_master_stack_view(self.into_raw()) };
        let view = from_master_stack_seed(&viewed.view);
        unsafe { viewed.view.decref(host) };
        (MasterStackBox(viewed.model), view)
    }
}

impl Drop for MasterStackBox {
    fn drop(&mut self) {
        // SAFETY: the box is this system's and is never used again.
        unsafe { abi::roc_master_stack_release(self.0) }
    }
}

// ---------------------------------------------------------------------------------------------
// The floating system (`FloatingSystem`).

fn floating_geometry(geometry: Geometry) -> abi::BoundaryFloatingGeometry {
    use abi::BoundaryFloatingGeometryTag as Tag;
    match geometry {
        Geometry::Kept(frame) => union!(
            BoundaryFloatingGeometry,
            BoundaryFloatingGeometryPayload.kept(rect(frame)),
            Tag::Kept
        ),
        Geometry::Aligned { frame, restore } => union!(
            BoundaryFloatingGeometry,
            BoundaryFloatingGeometryPayload.aligned(abi::BoundaryFloatingAligned {
                frame: rect(frame),
                restore: rect(restore),
            }),
            Tag::Aligned
        ),
        Geometry::Restorable(restore) => union!(
            BoundaryFloatingGeometry,
            BoundaryFloatingGeometryPayload.restorable(rect(restore)),
            Tag::Restorable
        ),
    }
}

fn from_floating_geometry(geometry: &abi::BoundaryFloatingGeometry) -> Geometry {
    use abi::BoundaryFloatingGeometryTag as Tag;
    match geometry.tag {
        // SAFETY (each arm): the tag says which payload it is.
        Tag::Kept => {
            Geometry::Kept(from_rect(unsafe { *geometry.borrow_payload_kept_unchecked() }))
        }
        Tag::Aligned => {
            let aligned = unsafe { geometry.borrow_payload_aligned_unchecked() };
            Geometry::Aligned {
                frame: from_rect(aligned.frame),
                restore: from_rect(aligned.restore),
            }
        }
        Tag::Restorable => Geometry::Restorable(from_rect(unsafe {
            *geometry.borrow_payload_restorable_unchecked()
        })),
    }
}

fn floating_seed(view: &FloatingView, host: &abi::RocHost) -> abi::BoundaryFloatingSeed {
    let geometry: Vec<abi::BoundaryFloatingFrames> = view
        .geometry
        .iter()
        .map(|&(window, geometry)| abi::BoundaryFloatingFrames {
            window,
            geometry: floating_geometry(geometry),
        })
        .collect();
    abi::BoundaryFloatingSeed {
        forest: forest_seed(&view.forest, host),
        geometry: list(&geometry, host),
    }
}

fn from_floating_seed(seed: &abi::BoundaryFloatingSeed) -> FloatingView {
    FloatingView {
        forest: from_forest_seed(&seed.forest),
        geometry: seed
            .geometry
            .as_slice()
            .iter()
            .map(|framed| (framed.window, from_floating_geometry(&framed.geometry)))
            .collect(),
    }
}

fn floating_screen(floating: Floating) -> abi::BoundaryFloatingScreen {
    abi::BoundaryFloatingScreen {
        screen: rect(floating.screen),
        spread: floating.spread,
    }
}

fn known_list(
    known: &[(u64, WindowConstraints)],
    host: &abi::RocHost,
) -> abi::RocListWith<abi::BoundaryBspKnown, false> {
    let known: Vec<abi::BoundaryBspKnown> = known
        .iter()
        .map(|&(window, held)| abi::BoundaryBspKnown {
            window,
            constraints: window_constraints(held),
        })
        .collect();
    list(&known, host)
}

fn floating_event(event: FloatingEvent, host: &abi::RocHost) -> abi::BoundaryFloatingEvent {
    use abi::BoundaryFloatingEventTag as Tag;
    use rift_core::layout::group_forest::Participation;
    let window_in =
        |layout: LayoutKey, window| abi::BoundaryBspWindowIn { layout: layout.0, window };
    match event {
        FloatingEvent::CreateLayout => {
            union!(
                BoundaryFloatingEvent,
                BoundaryFloatingEventPayload.create_layout,
                Tag::CreateLayout
            )
        }
        FloatingEvent::CloneLayout(layout) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.clone_layout(layout.0),
            Tag::CloneLayout
        ),
        FloatingEvent::RemoveLayout(layout) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.remove_layout(layout.0),
            Tag::RemoveLayout
        ),
        FloatingEvent::Participation(window, participation) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.participation(abi::BoundaryBspParticipation {
                window,
                participation: match participation {
                    Participation::TakesPart => abi::BoundaryBspTakesPart::TakesPart,
                    Participation::LeftOut => abi::BoundaryBspTakesPart::LeftOut,
                },
            }),
            Tag::Participation
        ),
        FloatingEvent::AddWindow(layout, window, point) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.add_window(abi::BoundaryFloatingAdd {
                layout: layout.0,
                window,
                insertion: insertion(point),
            }),
            Tag::AddWindow
        ),
        FloatingEvent::RemoveWindow(window) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.remove_window(window),
            Tag::RemoveWindow
        ),
        FloatingEvent::ReplaceWindow { from, to } => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.replace_window(abi::BoundaryBspReplacement { from, to }),
            Tag::ReplaceWindow
        ),
        FloatingEvent::CloseApp(windows) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.close_app(list(&windows, host)),
            Tag::CloseApp
        ),
        FloatingEvent::StoreFrame(window, frame) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload
                .store_frame(abi::BoundaryFloatingStore { window, frame: rect(frame) }),
            Tag::StoreFrame
        ),
        FloatingEvent::InitializeFrames { layout, floating, known, live } => {
            let live: Vec<abi::BoundaryFloatingLive> = live
                .iter()
                .map(|&(window, frame)| abi::BoundaryFloatingLive { window, frame: rect(frame) })
                .collect();
            union!(
                BoundaryFloatingEvent,
                BoundaryFloatingEventPayload.initialize_frames(abi::BoundaryFloatingInitialize {
                    layout: layout.0,
                    floating: floating_screen(floating),
                    known: known_list(&known, host),
                    live: list(&live, host),
                }),
                Tag::InitializeFrames
            )
        }
        FloatingEvent::Focus(layout, focus) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.focus(abi::BoundaryBspFocusIn {
                layout: layout.0,
                focus: tree_focus(focus),
            }),
            Tag::Focus
        ),
        FloatingEvent::Edit(layout, asked) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.edit(edit_in(layout, asked)),
            Tag::Edit
        ),
        FloatingEvent::ContainsLayout(layout) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.contains_layout(layout.0),
            Tag::ContainsLayout
        ),
        FloatingEvent::WindowParticipates(window) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.window_participates(window),
            Tag::WindowParticipates
        ),
        FloatingEvent::SelectedWindow(layout) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.selected_window(layout.0),
            Tag::SelectedWindow
        ),
        FloatingEvent::SelectionContainsWindow(layout, window) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.selection_contains_window(window_in(layout, window)),
            Tag::SelectionContainsWindow
        ),
        FloatingEvent::ContainsWindow(layout, window) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.contains_window(window_in(layout, window)),
            Tag::ContainsWindow
        ),
        FloatingEvent::HasAnyFullscreenNode(layout) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.has_any_fullscreen_node(layout.0),
            Tag::HasAnyFullscreenNode
        ),
        FloatingEvent::HasWindowsForApp(layout, windows) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.has_windows_for_app(abi::BoundaryBspAppIn {
                layout: layout.0,
                windows: list(&windows, host),
            }),
            Tag::HasWindowsForApp
        ),
        FloatingEvent::ParentOfSelectionIsStacked(layout) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.parent_of_selection_is_stacked(layout.0),
            Tag::ParentOfSelectionIsStacked
        ),
        FloatingEvent::Windows(layout, query) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.windows(abi::BoundaryBspWindowsQuery {
                layout: layout.0,
                query: tree_query(query),
            }),
            Tag::Windows
        ),
        FloatingEvent::WindowSlot(layout, window) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.window_slot(window_in(layout, window)),
            Tag::WindowSlot
        ),
        FloatingEvent::WindowInDirection(layout, direction) => union!(
            BoundaryFloatingEvent,
            BoundaryFloatingEventPayload.window_in_direction(abi::BoundaryBspLayoutDirection {
                layout: layout.0,
                direction: tree_direction(direction),
            }),
            Tag::WindowInDirection
        ),
        FloatingEvent::Cycle(layout) => {
            union!(
                BoundaryFloatingEvent,
                BoundaryFloatingEventPayload.cycle(layout.0),
                Tag::Cycle
            )
        }
        FloatingEvent::FrameOf(window) => {
            union!(
                BoundaryFloatingEvent,
                BoundaryFloatingEventPayload.frame_of(window),
                Tag::FrameOf
            )
        }
        FloatingEvent::Floats(window) => {
            union!(
                BoundaryFloatingEvent,
                BoundaryFloatingEventPayload.floats(window),
                Tag::Floats
            )
        }
    }
}

fn from_floating_answer(answer: &abi::BoundaryFloatingAnswer) -> FloatingAnswer {
    use abi::BoundaryFloatingAnswerTag as Tag;
    match answer.tag {
        // SAFETY (each arm): the tag says which payload it is; read, not released (the caller
        // releases the answer).
        Tag::Tree => FloatingAnswer::Tree(from_tree_answer(unsafe {
            std::ptr::read(answer.borrow_payload_tree_unchecked())
        })),
        Tag::Frame => {
            let frame = unsafe { answer.borrow_payload_frame_unchecked() };
            FloatingAnswer::Frame(match frame.tag {
                abi::BoundaryFloatingFrameTag::Framed => {
                    Some(from_rect(unsafe { *frame.borrow_payload_framed_unchecked() }))
                }
                abi::BoundaryFloatingFrameTag::Unframed => None,
            })
        }
    }
}

/// A floating layout's shape.
pub(crate) fn shape_question(
    layout: LayoutKey,
    floating: Floating,
    gaps_given: rift_core::layout::Gaps,
    known: &[(u64, WindowConstraints)],
    host: &abi::RocHost,
) -> abi::BoundaryFloatingLayoutQuestion {
    use abi::BoundaryFloatingLayoutQuestionTag as Tag;
    union!(
        BoundaryFloatingLayoutQuestion,
        BoundaryFloatingLayoutQuestionPayload.shape(abi::BoundaryFloatingShapeQuestion {
            layout: layout.0,
            floating: floating_screen(floating),
            gaps: gaps(gaps_given),
            known: known_list(known, host),
        }),
        Tag::Shape
    )
}

/// A floating layout's group bars.
pub(crate) fn bars_question(
    layout: LayoutKey,
    floating: Floating,
    selection_path_only: bool,
) -> abi::BoundaryFloatingLayoutQuestion {
    use abi::BoundaryFloatingLayoutQuestionTag as Tag;
    union!(
        BoundaryFloatingLayoutQuestion,
        BoundaryFloatingLayoutQuestionPayload.bars(abi::BoundaryFloatingBarsQuestion {
            layout: layout.0,
            floating: floating_screen(floating),
            selection_path_only,
        }),
        Tag::Bars
    )
}

/// A floating layout's container view.
pub(crate) fn floating_container_question(
    layout: LayoutKey,
) -> abi::BoundaryFloatingLayoutQuestion {
    use abi::BoundaryFloatingLayoutQuestionTag as Tag;
    union!(
        BoundaryFloatingLayoutQuestion,
        BoundaryFloatingLayoutQuestionPayload.container(layout.0),
        Tag::Container
    )
}

/// A floating layout's drawing.
pub(crate) fn floating_draw_question(layout: LayoutKey) -> abi::BoundaryFloatingLayoutQuestion {
    use abi::BoundaryFloatingLayoutQuestionTag as Tag;
    union!(
        BoundaryFloatingLayoutQuestion,
        BoundaryFloatingLayoutQuestionPayload.draw(layout.0),
        Tag::Draw
    )
}

fn from_size(size: abi::BoundarySize) -> rift_core::layout::Size {
    rift_core::layout::Size::reported(size.width, size.height)
        .unwrap_or(rift_core::layout::Size::ZERO)
}

fn from_window_constraints(constraints: &abi::BoundaryWindowConstraints) -> WindowConstraints {
    use abi::BoundarySizingTag as Tag;
    use rift_core::layout::Sizing;
    let sizing = &constraints.sizing;
    WindowConstraints {
        // SAFETY (each arm): the tag says which payload it is.
        sizing: match sizing.tag {
            Tag::Fixed => Sizing::Fixed {
                min: from_size(unsafe { *sizing.borrow_payload_fixed_unchecked() }),
            },
            Tag::Resizable => Sizing::Resizable {
                advisory_min: from_size(unsafe { *sizing.borrow_payload_resizable_unchecked() }),
            },
            Tag::ResizableAtLeastMin => Sizing::ResizableAtLeastMin {
                min: from_size(unsafe {
                    *sizing.borrow_payload_resizable_at_least_min_unchecked()
                }),
            },
        },
        locked_width: constraints.locked_width,
        locked_height: constraints.locked_height,
        max_width: constraints.max_width,
        max_height: constraints.max_height,
    }
}

fn from_gaps(given: abi::BoundaryGaps) -> rift_core::layout::Gaps {
    rift_core::layout::Gaps {
        top: given.top,
        left: given.left,
        bottom: given.bottom,
        right: given.right,
        horizontal: given.horizontal,
        vertical: given.vertical,
    }
}

/// The shape a shape question answers, windows as the geometry's ids.
pub(crate) fn read_shape(
    answer: abi::BoundaryFloatingLayoutAnswer,
    host: &abi::RocHost,
) -> floating::Layout<WindowId> {
    use abi::{BoundaryFloatingPlaceTag as PlaceTag, BoundaryFloatingSlotTag as SlotTag};
    assert_eq!(
        answer.tag,
        abi::BoundaryFloatingLayoutAnswerTag::Shape,
        "a shape"
    );
    // SAFETY: the tag says the payload is the shape, read in full here, then released.
    let shape = unsafe { answer.borrow_payload_shape_unchecked() };
    let read = floating::Layout {
        floating: Floating {
            screen: from_rect(shape.screen),
            spread: shape.spread,
        },
        gaps: from_gaps(shape.gaps),
        windows: shape
            .windows
            .as_slice()
            .iter()
            .map(|slot| match slot.tag {
                SlotTag::LeftOut => floating::Slot::LeftOut,
                SlotTag::Placed => {
                    let placed = unsafe { slot.borrow_payload_placed_unchecked() };
                    let place = &placed.place;
                    floating::Slot::In {
                        window: from_window(placed.window),
                        place: match place.tag {
                            PlaceTag::Screen => floating::Place::Screen,
                            PlaceTag::WithinGaps => floating::Place::WithinGaps,
                            PlaceTag::Kept => floating::Place::Kept(from_rect(unsafe {
                                *place.borrow_payload_kept_unchecked()
                            })),
                            PlaceTag::First => {
                                floating::Place::First(from_window_constraints(unsafe {
                                    place.borrow_payload_first_unchecked()
                                }))
                            }
                        },
                    }
                }
            })
            .collect(),
    };
    unsafe { answer.decref(host) };
    read
}

/// The group bars a bars question answers.
pub(crate) fn read_bars(
    answer: abi::BoundaryFloatingLayoutAnswer,
    host: &abi::RocHost,
) -> Vec<GroupBar<NodeKey, u64>> {
    assert_eq!(
        answer.tag,
        abi::BoundaryFloatingLayoutAnswerTag::Bars,
        "group bars"
    );
    // SAFETY: the tag says the payload is the bars, read in full here, then released.
    let bars = unsafe { answer.borrow_payload_bars_unchecked() }
        .as_slice()
        .iter()
        .map(|bar| GroupBar {
            stack: NodeKey(bar.stack),
            orientation: from_orientation(bar.orientation),
            frame: from_rect(bar.frame),
            windows: bar.windows.as_slice().to_vec(),
            selected: usize::try_from(bar.selected).expect("a bar's index fits a usize"),
        })
        .collect();
    unsafe { answer.decref(host) };
    bars
}

/// The container view a container question answers.
pub(crate) fn read_floating_container(
    answer: abi::BoundaryFloatingLayoutAnswer,
    host: &abi::RocHost,
) -> ContainerView<NodeKey, u64> {
    assert_eq!(
        answer.tag,
        abi::BoundaryFloatingLayoutAnswerTag::Container,
        "a container view"
    );
    let view = from_view(unsafe { answer.borrow_payload_container_unchecked() });
    // SAFETY: the answer is Roc's, owned here and read in full above.
    unsafe { answer.decref(host) };
    view
}

/// The rows a draw question answers.
pub(crate) fn read_floating_rows(
    answer: abi::BoundaryFloatingLayoutAnswer,
    host: &abi::RocHost,
) -> Vec<Row<NodeKey, u64>> {
    assert_eq!(
        answer.tag,
        abi::BoundaryFloatingLayoutAnswerTag::Drawn,
        "a drawing"
    );
    // SAFETY: the tag says the payload is the rows; they are read through the forest's own
    // reader by viewing them as a forest answer's (the same list type).
    let rows = rows_of(unsafe { answer.borrow_payload_drawn_unchecked() });
    unsafe { answer.decref(host) };
    rows
}

/// The floating system's box, which only Roc can read or free; only the
/// `roc_floating_tree_*` entries make and take one. Dropping it releases the system.
pub(crate) struct FloatingBox(abi::RocBox);

impl FloatingBox {
    pub(crate) fn new() -> FloatingBox { FloatingBox(unsafe { abi::roc_floating_tree_init() }) }

    /// The system `view` holds, which the caller has parsed (`FloatingView::floating`).
    pub(crate) fn seeded(view: &FloatingView, host: &abi::RocHost) -> FloatingBox {
        // SAFETY: the seed's lists are allocated through the host and consumed by Roc.
        FloatingBox(unsafe { abi::roc_floating_tree_seed(floating_seed(view, host)) })
    }

    fn into_raw(self) -> abi::RocBox {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }

    /// One operation or query: the system after it, and what it answered.
    pub(crate) fn step(
        self,
        asked: FloatingEvent,
        host: &abi::RocHost,
    ) -> (FloatingBox, FloatingAnswer) {
        // SAFETY: as in `TraditionalBox::step`.
        let stepped =
            unsafe { abi::roc_floating_tree_step(self.into_raw(), floating_event(asked, host)) };
        let answer = from_floating_answer(&stepped.answer);
        unsafe { stepped.answer.decref(host) };
        (FloatingBox(stepped.model), answer)
    }

    /// A layout question, its answer read by `read` (which releases it).
    pub(crate) fn layout<T>(
        self,
        question: abi::BoundaryFloatingLayoutQuestion,
        host: &abi::RocHost,
        read: impl FnOnce(abi::BoundaryFloatingLayoutAnswer, &abi::RocHost) -> T,
    ) -> (FloatingBox, T) {
        // SAFETY: as in `TraditionalBox::layout`.
        let laid = unsafe { abi::roc_floating_tree_layout(self.into_raw(), question) };
        (FloatingBox(laid.model), read(laid.answer, host))
    }

    /// The forest's tables and free lists, the windows left out, the frames kept.
    pub(crate) fn view(self, host: &abi::RocHost) -> (FloatingBox, FloatingView) {
        // SAFETY: as in `TraditionalBox::view`.
        let viewed = unsafe { abi::roc_floating_tree_view(self.into_raw()) };
        let view = from_floating_seed(&viewed.view);
        unsafe { viewed.view.decref(host) };
        (FloatingBox(viewed.model), view)
    }
}

impl Drop for FloatingBox {
    fn drop(&mut self) {
        // SAFETY: the box is this system's and is never used again.
        unsafe { abi::roc_floating_tree_release(self.0) }
    }
}
