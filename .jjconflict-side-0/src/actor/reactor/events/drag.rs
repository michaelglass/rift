use tracing::{trace, warn};

use crate::actor::app::WindowId;
use crate::actor::drag::{DragKind, DragScene, DragSceneTarget, DropIntent, NativeOp};
use crate::actor::reactor::LayoutEvent;
use crate::actor::reactor::events::EventOutcome;
use crate::actor::reactor::managers::{DragManager, LayoutManager};
use crate::model::RiftState;
use crate::model::drag::Session;
use crate::model::window_store::Placed;
use crate::sys::screen::SpaceId;

pub fn build_drag_scene(
    state: &RiftState,
    layout: &LayoutManager,
    source: crate::actor::app::WindowId,
    space: SpaceId,
) -> DragScene {
    DragScene {
        action_override: layout.layout_engine.drop_action_override(space),
        source_neighbors: layout.layout_engine.source_move_neighbors(space, source),
        targets: layout
            .layout_engine
            .drop_scene_windows(space, source)
            .into_iter()
            .filter(|window| *window != source)
            .filter_map(|window| {
                Some(DragSceneTarget {
                    window,
                    space,
                    frame: state.windows.window(window)?.frame_monotonic(),
                })
            })
            .collect(),
    }
}

pub fn handle_cancel(state: &mut RiftState, layout: &mut LayoutManager) -> EventOutcome {
    let Some(ungripped) = state.let_go() else {
        return EventOutcome::no_change();
    };
    let window = ungripped.window;
    let placed = match ungripped.placed {
        Placed::ByWorkspace => EventOutcome::no_change(),
        Placed::Unplaced => place_on_its_space(state, layout, window),
    };
    let Some(cancelled) = ungripped.session else {
        return placed;
    };
    let mut outcome = cancelled_outcome(window, cancelled);
    outcome.absorb(placed);
    outcome
}

/// A let-go without a drop of a window a workspace transfer left while it was held: it joins
/// the workspace shown on the Space it is on.
fn place_on_its_space(
    state: &mut RiftState,
    layout: &mut LayoutManager,
    window: WindowId,
) -> EventOutcome {
    let Some(space) = state
        .windows
        .server_id(window)
        .and_then(|server_id| state.windows.window_server_space(server_id))
    else {
        return EventOutcome::no_change();
    };
    let Some(workspace) = layout.layout_engine.workspaces().active_workspace(space) else {
        return EventOutcome::no_change();
    };
    if !layout.layout_engine.workspaces_mut().assign_window_to_workspace(
        &mut state.windows,
        space,
        window,
        workspace,
    ) {
        warn!(?window, ?workspace, "failed to assign a let-go window");
        return EventOutcome::no_change();
    }
    EventOutcome::layout_changed(false).with_layout_event(LayoutEvent::WindowAdded(space, window))
}

fn cancelled_outcome(window: WindowId, cancelled: Session) -> EventOutcome {
    let source = cancelled.source;
    match (cancelled.kind, source.tiled) {
        (DragKind::Modifier { .. }, true) => EventOutcome::layout_changed(false),
        (DragKind::Modifier { .. }, false) if source.last_frame != source.origin_frame => {
            EventOutcome::no_change().with_placement(window, source.origin_frame)
        }
        (DragKind::Modifier { .. }, false)
        | (DragKind::Native(NativeOp::Move | NativeOp::Resize), true | false) => {
            EventOutcome::no_change()
        }
    }
}

/// What a drop does, decided from the drag and the release; `handle_mouse_up` carries it out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DropPlan {
    /// The window stays where it was in its display's layout.
    Stays,
    /// Onto the previewed tile on the display it was dragged on.
    Within(DropIntent),
    /// Onto another display: its previewed tile there if there is one, else the end of the
    /// destination's active workspace.
    Across {
        space: SpaceId,
        onto: Option<DropIntent>,
    },
    /// Off every Space, or not Rift's to place: it only leaves the display it was on.
    Leaves,
}

/// The drop of `commit` released over `final_space`; `live` says whether a window is still
/// there to drop onto. A window no workspace places (a transfer left it while it was held) has
/// no slot to stay in: it joins the drop display's shown workspace, as a window dropped from
/// another display does.
pub fn plan_drop(
    commit: &Session,
    placed: Placed,
    final_space: Option<SpaceId>,
    admitted: bool,
    live: impl Fn(WindowId) -> bool,
) -> DropPlan {
    // The previewed drop, if it is on the display the window ends up on.
    let onto = commit
        .target()
        .map(|target| target.intent)
        .filter(|intent| final_space == Some(intent.space) && live(intent.window));
    let same_space = match placed {
        Placed::ByWorkspace => commit.source.origin_space == final_space,
        Placed::Unplaced => false,
    };
    match (same_space, final_space, onto) {
        (true, _, Some(intent)) => DropPlan::Within(intent),
        (true, _, None) => DropPlan::Stays,
        (false, Some(space), onto) if admitted => DropPlan::Across { space, onto },
        (false, _, _) => DropPlan::Leaves,
    }
}

/// The release dropped `window`'s drag (`commit`; the core decided the release ended it) over
/// `final_space`.
pub fn handle_mouse_up(
    state: &mut RiftState,
    layout: &mut LayoutManager,
    drag: &mut DragManager,
    window: WindowId,
    commit: Session,
    placed: Placed,
    final_space: Option<SpaceId>,
) -> anyhow::Result<EventOutcome> {
    let mut outcome = EventOutcome::layout_changed(false);
    let plan = plan_drop(
        &commit,
        placed,
        final_space,
        state.windows.is_admitted(window),
        |target| state.windows.contains_window(target),
    );
    trace!(?window, ?plan, "drop");
    // Whether the window leaves a slot in a layout (an unplaced one left its layout already).
    let left = match placed {
        Placed::ByWorkspace => commit.source.origin_space.is_some(),
        Placed::Unplaced => false,
    };
    let needs_layout = match plan {
        DropPlan::Stays => commit.source.tiled,
        DropPlan::Within(intent) => {
            let dropped =
                layout.layout_engine.apply_window_drop(crate::layout_engine::WindowDropRequest {
                    source: window,
                    target: intent.window,
                    space: intent.space,
                    action: intent.action,
                });
            outcome.drop_haptic = dropped && drag.haptics_enabled;
            commit.source.tiled || dropped
        }
        DropPlan::Across { space, onto } => {
            if let Some(server_id) = state.windows.server_id(window) {
                state.windows.observe_native_space(server_id, space, true);
            }
            let workspace = layout.layout_engine.workspaces().active_workspace(space);
            let floating = layout.layout_engine.is_window_floating(window);
            let dropped = if floating {
                // A floating window stays floating there, at the frame it was released at
                // (its position on the source display is forgotten).
                workspace.is_some_and(|workspace| {
                    layout.layout_engine.relocate_window_with_drop(
                        &mut state.windows,
                        &crate::layout_engine::WorkspaceDropRequest {
                            window,
                            workspace,
                            target: None,
                            frame: Some(commit.source.last_frame),
                        },
                    )
                })
            } else {
                // Onto the previewed tile of the destination's active workspace, in one step:
                // the layout the preview was computed against is the one the window joins.
                onto.zip(workspace).is_some_and(|(intent, workspace)| {
                    layout.layout_engine.relocate_window_with_drop(
                        &mut state.windows,
                        &crate::layout_engine::WorkspaceDropRequest {
                            window,
                            workspace,
                            target: Some((intent.window, intent.action)),
                            frame: None,
                        },
                    )
                })
            };
            // The haptic marks a drop onto a tile.
            outcome.drop_haptic = dropped && !floating && drag.haptics_enabled;
            if !dropped {
                // Assigning to the destination takes the window out of the old workspace's tree
                // itself; a removal would run after it and clear the new assignment (#702's
                // WindowAdded then needs that assignment).
                if left && workspace.is_none() {
                    outcome = outcome.with_layout_event(LayoutEvent::WindowRemoved(window));
                }
                if let Some(workspace) = workspace
                    && !layout.layout_engine.workspaces_mut().assign_window_to_workspace(
                        &mut state.windows,
                        space,
                        window,
                        workspace,
                    )
                {
                    warn!(?window, ?workspace, "failed to assign dragged window");
                }
                outcome = outcome.with_layout_event(LayoutEvent::WindowAdded(space, window));
            }
            true
        }
        DropPlan::Leaves => {
            if left {
                outcome = outcome.with_layout_event(LayoutEvent::WindowRemoved(window));
            }
            true
        }
    };

    if let Some(space) = final_space
        && layout.layout_engine.is_window_floating(window)
    {
        state.windows.clear_floating_display_geometry(window);
        if commit.source.origin_space != final_space {
            layout.layout_engine.remove_floating_position(window);
        }
        if let Some(workspace) = layout
            .layout_engine
            .workspaces()
            .workspace_for_window(&state.windows, space, window)
            .or_else(|| layout.layout_engine.workspaces().active_workspace(space))
        {
            layout.layout_engine.store_floating_position(
                workspace,
                window,
                commit.source.last_frame,
            );
        }
    }

    Ok(outcome.with_arrange_passes(u8::from(needs_layout)))
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    use super::*;
    use crate::actor::drag::{DragSource, DropTarget, DropZone};
    use crate::layout_engine::{Direction, WindowDropAction};
    use crate::model::drag::TargetState;

    fn space(n: u64) -> SpaceId { SpaceId::new(n) }
    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }

    /// A native drag that started on space 1, with `target` previewed on `on`.
    fn commit(on: Option<u64>) -> Session {
        let frame = CGRect::new(CGPoint::ZERO, CGSize::new(100., 100.));
        let mut session = Session::native(
            DragSource {
                origin_frame: frame,
                last_frame: frame,
                origin_space: Some(space(1)),
                current_space: Some(space(1)),
                tiled: true,
            },
            DragScene::default(),
        );
        if let Some(on) = on {
            let intent = DropIntent {
                window: w(2),
                space: space(on),
                frame,
                zone: DropZone::West,
                action: WindowDropAction::Insert(Direction::Left),
            };
            session.target = TargetState::Validated(DropTarget { intent, preview_area: frame });
        }
        session
    }

    #[test]
    fn a_drop_on_its_own_display_takes_the_previewed_tile_or_stays() {
        let live = |_| true;
        assert!(matches!(
            plan_drop(&commit(Some(1)), Placed::ByWorkspace, Some(space(1)), true, live),
            DropPlan::Within(intent) if intent.window == w(2)
        ));
        assert_eq!(
            plan_drop(&commit(None), Placed::ByWorkspace, Some(space(1)), true, live),
            DropPlan::Stays
        );
        // The previewed window went away: nothing to drop onto.
        assert_eq!(
            plan_drop(
                &commit(Some(1)),
                Placed::ByWorkspace,
                Some(space(1)),
                true,
                |_| false
            ),
            DropPlan::Stays
        );
    }

    #[test]
    fn a_drop_on_another_display_goes_onto_its_previewed_tile_there_or_joins_it() {
        let live = |_| true;
        assert!(matches!(
            plan_drop(&commit(Some(2)), Placed::ByWorkspace, Some(space(2)), true, live),
            DropPlan::Across { space: to, onto: Some(intent) } if to == space(2) && intent.window == w(2)
        ));
        assert_eq!(
            plan_drop(&commit(None), Placed::ByWorkspace, Some(space(2)), true, live),
            DropPlan::Across { space: space(2), onto: None }
        );
        // A preview on a display the window did not end up on is no drop.
        assert_eq!(
            plan_drop(&commit(Some(1)), Placed::ByWorkspace, Some(space(2)), true, live),
            DropPlan::Across { space: space(2), onto: None }
        );
    }

    #[test]
    fn a_drop_off_every_space_or_of_an_unadmitted_window_only_leaves() {
        let live = |_| true;
        assert_eq!(
            plan_drop(&commit(Some(1)), Placed::ByWorkspace, None, true, live),
            DropPlan::Leaves
        );
        assert_eq!(
            plan_drop(&commit(None), Placed::ByWorkspace, Some(space(2)), false, live),
            DropPlan::Leaves
        );
    }
}
