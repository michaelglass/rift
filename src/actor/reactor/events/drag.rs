use tracing::{trace, warn};

use crate::actor::app::WindowId;
use crate::actor::drag::{DragKind, DragScene, DragSceneTarget, NativeOp};
use crate::actor::reactor::LayoutEvent;
use crate::actor::reactor::events::EventOutcome;
use crate::actor::reactor::managers::{DragManager, LayoutManager};
use crate::model::RiftState;
use crate::model::drag::Session;
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

#[derive(Debug, Clone)]
pub struct MouseUpPayload {
    /// The drag the release dropped, with its session (the core decided the release ended it).
    pub dropped: Option<(WindowId, Session)>,
    pub final_space: Option<SpaceId>,
}

pub fn handle_cancel(state: &mut RiftState) -> EventOutcome {
    let Some((window, cancelled)) = state
        .let_go()
        .and_then(|ungripped| Some((ungripped.window, ungripped.session?)))
    else {
        return EventOutcome::no_change();
    };
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

pub fn handle_mouse_up(
    state: &mut RiftState,
    layout: &mut LayoutManager,
    drag: &mut DragManager,
    payload: MouseUpPayload,
) -> anyhow::Result<EventOutcome> {
    let mut outcome = EventOutcome::layout_changed(false);
    let Some((window, commit)) = payload.dropped else {
        return Ok(outcome);
    };
    let mut needs_layout = commit.source.tiled;
    // The previewed drop, if it is on the display the window ends up on.
    let target = commit.target().filter(|target| {
        payload.final_space == Some(target.intent.space)
            && state.windows.contains_window(target.intent.window)
    });

    if commit.source.origin_space == payload.final_space {
        if let Some(target) = target {
            trace!(source=?window, target=?target.intent.window, action=?target.intent.action, "performing window drop");
            if layout.layout_engine.apply_window_drop(crate::layout_engine::WindowDropRequest {
                source: window,
                target: target.intent.window,
                space: target.intent.space,
                action: target.intent.action,
            }) {
                needs_layout = true;
                outcome.drop_haptic = drag.haptics_enabled;
            }
        }
    } else {
        needs_layout = true;
        if let Some(space) = payload.final_space
            && state.windows.is_admitted(window)
        {
            if let Some(server_id) = state.windows.server_id(window) {
                state.windows.observe_native_space(server_id, space, true);
            }
            // Onto the previewed tile of the destination's active workspace, in one step: the
            // layout the preview was computed against is the one the window joins.
            let dropped = target.is_some_and(|target| {
                layout.layout_engine.workspaces().active_workspace(space).is_some_and(
                    |workspace| {
                        trace!(source=?window, target=?target.intent.window, action=?target.intent.action, "performing cross-display window drop");
                        layout.layout_engine.relocate_window_with_drop(
                            &mut state.windows,
                            &crate::layout_engine::WorkspaceDropRequest {
                                window,
                                workspace,
                                target: Some((target.intent.window, target.intent.action)),
                                frame: None,
                            },
                        )
                    },
                )
            });
            if dropped {
                outcome.drop_haptic = drag.haptics_enabled;
            } else {
                if commit.source.origin_space.is_some() {
                    outcome = outcome.with_layout_event(LayoutEvent::WindowRemoved(window));
                }
                if let Some(workspace) = layout.layout_engine.workspaces().active_workspace(space)
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
        } else if commit.source.origin_space.is_some() {
            outcome = outcome.with_layout_event(LayoutEvent::WindowRemoved(window));
        }
    }

    if let Some(space) = payload.final_space
        && layout.layout_engine.is_window_floating(window)
    {
        state.windows.clear_floating_display_geometry(window);
        if commit.source.origin_space != payload.final_space {
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
