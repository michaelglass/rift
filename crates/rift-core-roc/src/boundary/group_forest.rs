//! The group forest's layout systems (`rift_core::layout::group_forest`: the traditional and
//! master-stack systems) as the platform's boundary types and back, and the boxes holding them
//! in Roc. Unsafe operations on Roc memory as in the rest of [`super`].

use std::mem::ManuallyDrop;

use rift_core::layout::group_forest::{
    Arrangement, Beyond, Body, Container, DropAction, Edit, Fill, InsertionPoint, Kind,
    MasterSettings, MasterSide, Members, Memory, NewSibling, NewWindowPlacement, Node, Place,
    RegionInsertion, ResizeAlong,
};

use super::stack_tree::default_orientation;
use super::{
    from_insertion, from_orientation, from_side, from_tree_answer, insertion, list, rect,
    traditional_fill, tree_direction, tree_focus, tree_orientation, tree_query, tree_side,
};
use crate::abi;
use crate::bsp_tree::{Answer, LayoutKey, NodeKey, Saved};
use crate::group_forest::{
    ForestEvent, ForestView, MasterStackEvent, MasterStackView, TraditionalEvent, TraditionalView,
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

fn node(node: &Node<NodeKey, u64>, host: &abi::RocHost) -> abi::BoundaryGroupTreeNode {
    abi::BoundaryGroupTreeNode {
        place: match node.place {
            Place::Root => union!(
                BoundaryGroupPlace,
                BoundaryGroupPlacePayload.root,
                abi::BoundaryGroupPlaceTag::Root
            ),
            Place::Child { parent, size } => union!(
                BoundaryGroupPlace,
                BoundaryGroupPlacePayload
                    .child(abi::BoundaryGroupChildPlace { parent: parent.0, size }),
                abi::BoundaryGroupPlaceTag::Child
            ),
        },
        fill: traditional_fill(node.fill),
        body: match &node.body {
            Body::Window { window, stamp } => union!(
                BoundaryGroupBody,
                BoundaryGroupBodyPayload
                    .window(abi::BoundaryGroupLeaf { window: *window, stamp: *stamp }),
                abi::BoundaryGroupBodyTag::Window
            ),
            Body::Container(held) => union!(
                BoundaryGroupBody,
                BoundaryGroupBodyPayload.container(container(held, host)),
                abi::BoundaryGroupBodyTag::Container
            ),
        },
    }
}

fn from_node(node: &abi::BoundaryGroupTreeNode) -> Node<NodeKey, u64> {
    let place = match node.place.tag {
        abi::BoundaryGroupPlaceTag::Root => Place::Root,
        abi::BoundaryGroupPlaceTag::Child => {
            // SAFETY: the tag says the payload is the child's place.
            let child = unsafe { node.place.borrow_payload_child_unchecked() };
            Place::Child {
                parent: NodeKey(child.parent),
                size: child.size,
            }
        }
    };
    let body = match node.body.tag {
        abi::BoundaryGroupBodyTag::Window => {
            // SAFETY: the tag says the payload is the leaf.
            let leaf = unsafe { node.body.borrow_payload_window_unchecked() };
            Body::Window {
                window: leaf.window,
                stamp: leaf.stamp,
            }
        }
        // SAFETY: the tag says the payload is the container.
        abi::BoundaryGroupBodyTag::Container => Body::Container(from_container(unsafe {
            node.body.borrow_payload_container_unchecked()
        })),
    };
    Node {
        place,
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
            BoundaryGroupEditPayload.moved(abi::BoundaryStackMove {
                direction: tree_direction(direction),
                beyond: match beyond {
                    Beyond::AnotherDisplay => abi::BoundaryStackBeyond::AnotherDisplay,
                    Beyond::Nothing => abi::BoundaryStackBeyond::Nothing,
                },
            }),
            Tag::Moved
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
