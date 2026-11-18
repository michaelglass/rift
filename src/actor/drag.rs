//! Single-owner drag interaction primitives.
//!
//! The event tap only publishes the latest pointer value. Geometry and drag
//! semantics are consumed by the drag actor/reactor bridge, never in the tap.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use objc2_core_foundation::{CGPoint, CGRect};

use crate::actor::app::WindowId;
use crate::common::config::{DragDropSettings, MouseAction, MouseDropAction};
use crate::layout_engine::{Direction, WindowDropAction};
use crate::model::drag::TargetState;
pub use crate::model::drag::{
    DragKind, DragScene, DragSceneTarget, DragSource, DropIntent, DropTarget, DropZone,
    MouseButton, Session,
};
use crate::sys::geometry::SameAs;
use crate::sys::screen::SpaceId;

const HYSTERESIS_POINTS: f64 = 8.0;

#[derive(Debug, Clone, Copy)]
pub struct DragMotion {
    pub point: CGPoint,
}

/// Latest-value transport with at most one outstanding actor wake.
#[derive(Debug, Default)]
struct DragMotionState {
    latest: Mutex<Option<DragMotion>>,
    wake_queued: AtomicBool,
}

#[derive(Clone, Debug, Default)]
pub struct DragMotionPublisher(Arc<DragMotionState>);

impl DragMotionPublisher {
    /// Publish a sample. Returns true exactly when the caller must enqueue a wake.
    pub fn publish(&self, motion: DragMotion) -> bool {
        *self.0.latest.lock().expect("drag motion mutex poisoned") = Some(motion);
        !self.0.wake_queued.swap(true, Ordering::AcqRel)
    }

    /// Consume the newest sample and re-arm publication atomically with respect
    /// to the value mutex, so a concurrent publication cannot be stranded.
    pub fn take_latest(&self) -> Option<DragMotion> {
        let mut latest = self.0.latest.lock().expect("drag motion mutex poisoned");
        let motion = latest.take();
        self.0.wake_queued.store(false, Ordering::Release);
        motion
    }

    pub fn wake_queued(&self) -> bool { self.0.wake_queued.load(Ordering::Acquire) }
}

impl Session {
    fn new(source: DragSource, pointer: CGPoint, scene: DragScene, kind: DragKind) -> Self {
        Self {
            source,
            pointer,
            scene,
            target: TargetState::None,
            unavailable: Vec::new(),
            kind,
        }
    }

    /// The user started moving or resizing the window themselves.
    pub fn native(source: DragSource, scene: DragScene) -> Self {
        let pointer = CGPoint::new(
            source.last_frame.origin.x + source.last_frame.size.width / 2.0,
            source.last_frame.origin.y + source.last_frame.size.height / 2.0,
        );
        Self::new(source, pointer, scene, DragKind::Native)
    }

    /// Whether a native drag is resizing the window (its size changed), not moving it.
    pub fn resizing(&self) -> bool {
        matches!(self.kind, DragKind::Native)
            && !self.source.origin_frame.size.same_as(self.source.last_frame.size)
    }

    /// Whether Rift moves the window (a modifier drag).
    pub fn is_modifier(&self) -> bool { matches!(self.kind, DragKind::Modifier { .. }) }

    /// A modifier press on a window; only a move action drags it.
    pub fn modifier(
        source: DragSource,
        point: CGPoint,
        action: MouseAction,
        scene: DragScene,
    ) -> Option<Self> {
        match action {
            MouseAction::Move => Some(Self::new(source, point, scene, DragKind::Modifier {
                anchor: point,
            })),
            MouseAction::None => None,
        }
    }

    pub fn target(&self) -> Option<DropTarget> { self.target.validated() }

    pub fn intent(&self) -> Option<DropIntent> { self.target.intent() }

    pub fn preview_target(&self, window: WindowId) -> Option<DropTarget> {
        if let Some(target) = self.target.validated() {
            return (self.source.tiled || target.intent.action == WindowDropAction::Stack)
                .then_some(target);
        }
        if !self.source.tiled
            || !matches!(self.target, TargetState::None)
            || !contains(self.source.origin_frame, self.pointer, 0.0)
        {
            return None;
        }
        Some(DropTarget {
            intent: DropIntent {
                window,
                space: self.source.current_space?,
                frame: self.source.origin_frame,
                zone: DropZone::Center,
                action: WindowDropAction::Swap,
            },
            preview_area: self.source.origin_frame,
        })
    }

    /// The frame a modifier move puts the window at; the user moves a native drag themselves.
    pub fn interactive_frame(&self) -> Option<CGRect> {
        self.is_modifier().then_some(self.source.last_frame)
    }

    fn invalidate_drop(&mut self) {
        self.target = TargetState::None;
        self.unavailable.clear();
    }

    /// Update reactor-resolved space membership and invalidate destination state.
    pub fn update_current_space(&mut self, current_space: Option<SpaceId>) -> bool {
        if self.source.current_space == current_space {
            return false;
        }
        self.source.current_space = current_space;
        self.invalidate_drop();
        true
    }

    pub fn replace_scene(
        &mut self,
        window: WindowId,
        settings: &DragDropSettings,
        scene: DragScene,
    ) {
        let previous = self.target.intent();
        self.scene = scene;
        self.invalidate_drop();
        if self.source.tiled
            && !self.resizing()
            && self.source.origin_space == self.source.current_space
        {
            let next = hit_test_available(
                &self.scene,
                self.pointer,
                settings.drop_zone_fraction,
                settings.drop_action,
                previous,
                &self.unavailable,
                Some((window, self.source)),
                None,
            );
            self.target = next.map_or(TargetState::None, TargetState::Candidate);
        }
    }

    pub fn set_preview(
        &mut self,
        window: WindowId,
        settings: &DragDropSettings,
        intent: DropIntent,
        preview_area: Option<CGRect>,
    ) -> bool {
        if !self
            .target
            .intent()
            .is_some_and(|current| current.window == intent.window && current.zone == intent.zone)
        {
            return false;
        }
        if intent.window == window && !matches!(intent.action, WindowDropAction::Move(_)) {
            return false;
        }
        if let Some(preview_area) = preview_area {
            self.target = TargetState::Validated(DropTarget { intent, preview_area });
            return false;
        }
        self.unavailable.push((intent.window, intent.zone, intent.action));
        let next = hit_test_available(
            &self.scene,
            self.pointer,
            settings.drop_zone_fraction,
            settings.drop_action,
            None,
            &self.unavailable,
            Some((window, self.source)),
            None,
        );
        self.target = next.map_or(TargetState::None, TargetState::Candidate);
        !matches!(self.target, TargetState::None)
    }

    /// Follows a native drag's frame. A modifier drag's frame is Rift's to set, and its echoes
    /// never get here.
    pub fn update_native(&mut self, frame: CGRect, current_space: Option<SpaceId>) {
        self.source.last_frame = frame;
        if self.source.current_space != current_space {
            self.source.current_space = current_space;
            self.invalidate_drop();
        }
    }

    /// Where a modifier move puts the window when the pointer is at `point`.
    /// Where a modifier move puts the window when the pointer is at `point`; a native drag's
    /// frame is the user's.
    pub fn modifier_frame_at(&self, point: CGPoint) -> Option<CGRect> {
        let DragKind::Modifier { anchor } = self.kind else {
            return None;
        };
        Some(CGRect::new(
            CGPoint::new(
                self.source.origin_frame.origin.x + (point.x - anchor.x),
                self.source.origin_frame.origin.y + (point.y - anchor.y),
            ),
            self.source.origin_frame.size,
        ))
    }

    pub fn motion(
        &mut self,
        window: WindowId,
        settings: &DragDropSettings,
        motion: DragMotion,
    ) -> bool {
        let home_preview_changed = contains(self.source.origin_frame, self.pointer, 0.0)
            != contains(self.source.origin_frame, motion.point, 0.0);
        self.pointer = motion.point;
        if let Some(frame) = self.modifier_frame_at(motion.point) {
            self.source.last_frame = frame;
        }
        let next = if self.resizing()
            || !self.source.tiled
            || self.source.origin_space != self.source.current_space
        {
            None
        } else {
            hit_test_available(
                &self.scene,
                motion.point,
                settings.drop_zone_fraction,
                settings.drop_action,
                self.target.intent(),
                &self.unavailable,
                Some((window, self.source)),
                None,
            )
        };
        let changed = next != self.target.intent();
        if changed {
            self.target = next.map_or(TargetState::None, TargetState::Candidate);
        }
        changed || home_preview_changed
    }

    /// Another window left Rift's control; it is no longer somewhere to drop.
    pub fn forget_target(&mut self, window: WindowId) {
        self.scene.targets.retain(|target| target.window != window);
        if self.target.intent().is_some_and(|intent| intent.window == window) {
            self.target = TargetState::None;
        }
    }
}

impl PartialEq for DropTarget {
    fn eq(&self, other: &Self) -> bool {
        self.intent.window == other.intent.window
            && self.intent.space == other.intent.space
            && self.intent.zone == other.intent.zone
    }
}

impl PartialEq for DropIntent {
    fn eq(&self, other: &Self) -> bool {
        self.window == other.window && self.space == other.space && self.action == other.action
    }
}

fn contains(rect: CGRect, point: CGPoint, margin: f64) -> bool {
    point.x >= rect.origin.x - margin
        && point.x <= rect.origin.x + rect.size.width + margin
        && point.y >= rect.origin.y - margin
        && point.y <= rect.origin.y + rect.size.height + margin
}

pub fn classify_zone(frame: CGRect, point: CGPoint, fraction: f64) -> Option<DropZone> {
    if frame.size.width <= 0.0 || frame.size.height <= 0.0 || !contains(frame, point, 0.0) {
        return None;
    }
    let left = (point.x - frame.origin.x) / frame.size.width;
    let right = 1.0 - left;
    let top = (point.y - frame.origin.y) / frame.size.height;
    let bottom = 1.0 - top;
    if left >= fraction && right >= fraction && top >= fraction && bottom >= fraction {
        return Some(DropZone::Center);
    }
    let mut nearest = (left, DropZone::West);
    for candidate in [
        (right, DropZone::East),
        (top, DropZone::North),
        (bottom, DropZone::South),
    ] {
        if candidate.0 < nearest.0 {
            nearest = candidate;
        }
    }
    Some(nearest.1)
}

fn zone_frame(frame: CGRect, zone: DropZone, fraction: f64) -> CGRect {
    let (x, y, width, height) = match zone {
        DropZone::Center => (fraction, fraction, 1.0 - 2.0 * fraction, 1.0 - 2.0 * fraction),
        DropZone::West => (0.0, 0.0, fraction, 1.0),
        DropZone::East => (1.0 - fraction, 0.0, fraction, 1.0),
        DropZone::North => (0.0, 0.0, 1.0, fraction),
        DropZone::South => (0.0, 1.0 - fraction, 1.0, fraction),
    };
    CGRect::new(
        CGPoint::new(
            frame.origin.x + frame.size.width * x,
            frame.origin.y + frame.size.height * y,
        ),
        objc2_core_foundation::CGSize::new(frame.size.width * width, frame.size.height * height),
    )
}

pub fn resolve_action(zone: DropZone, center: MouseDropAction) -> WindowDropAction {
    match zone {
        DropZone::Center => center.into(),
        DropZone::West => WindowDropAction::Insert(Direction::Left),
        DropZone::East => WindowDropAction::Insert(Direction::Right),
        DropZone::North => WindowDropAction::Insert(Direction::Up),
        DropZone::South => WindowDropAction::Insert(Direction::Down),
    }
}

/// Hit-test a cached immutable scene without allocating or sorting.
pub fn hit_test(
    scene: &DragScene,
    point: CGPoint,
    fraction: f64,
    center: MouseDropAction,
    previous: Option<DropIntent>,
) -> Option<DropIntent> {
    hit_test_available(scene, point, fraction, center, previous, &[], None, None)
}

/// Projected scenes include the source for presentation; it is not a relative drop target.
pub fn hit_test_projected(
    scene: &DragScene,
    source: WindowId,
    point: CGPoint,
) -> Option<DropIntent> {
    if scene
        .targets
        .iter()
        .find(|target| target.window == source)
        .and_then(|target| classify_zone(target.frame, point, 0.25))
        == Some(DropZone::Center)
    {
        return None;
    }
    hit_test_available(
        scene,
        point,
        0.25,
        MouseDropAction::Stack,
        None,
        &[],
        None,
        Some(source),
    )
}

fn hit_test_available(
    scene: &DragScene,
    point: CGPoint,
    fraction: f64,
    center: MouseDropAction,
    previous: Option<DropIntent>,
    unavailable: &[(WindowId, DropZone, WindowDropAction)],
    source: Option<(WindowId, DragSource)>,
    excluded: Option<WindowId>,
) -> Option<DropIntent> {
    if !source.is_some_and(|(_, source)| contains(source.origin_frame, point, 0.0))
        && let Some(previous) = previous
        && let Some(target) = scene
            .targets
            .iter()
            .find(|target| target.window == previous.window && Some(target.window) != excluded)
    {
        let retained = zone_frame(target.frame, previous.zone, fraction);
        if contains(retained, point, HYSTERESIS_POINTS) {
            let intent = DropIntent {
                frame: target.frame,
                action: previous.action,
                ..previous
            };
            if !unavailable.contains(&(intent.window, intent.zone, intent.action)) {
                return Some(intent);
            }
        }
    }
    let intent_for = |target: &DragSceneTarget, point: CGPoint| {
        let zone = classify_zone(target.frame, point, fraction)?;
        Some(DropIntent {
            window: target.window,
            space: target.space,
            frame: target.frame,
            zone,
            action: scene.action_override.unwrap_or_else(|| resolve_action(zone, center)),
        })
    };

    for target in scene.targets.iter().filter(|target| Some(target.window) != excluded) {
        if contains(target.frame, point, 0.0)
            && let Some(intent) = intent_for(target, point)
            && !unavailable.contains(&(intent.window, intent.zone, intent.action))
        {
            return Some(intent);
        }
    }

    if let Some((window, source)) = source
        && let Some(space) = source.current_space
        && let Some(zone) = classify_zone(source.origin_frame, point, fraction)
    {
        if let (None, WindowDropAction::Insert(direction)) =
            (scene.action_override, resolve_action(zone, center))
            && !scene.source_neighbors[match direction {
                Direction::Left => 0,
                Direction::Right => 1,
                Direction::Up => 2,
                Direction::Down => 3,
            }]
            && !unavailable.contains(&(window, zone, WindowDropAction::Move(direction)))
        {
            return Some(DropIntent {
                window,
                space,
                frame: source.origin_frame,
                zone,
                action: WindowDropAction::Move(direction),
            });
        }
        return None;
    }

    let nearest = scene.targets.iter().enumerate().filter(|(_, target)| {
        Some(target.window) != excluded
            && target.frame.size.width > 0.0
            && target.frame.size.height > 0.0
    });
    if unavailable.is_empty() {
        let (_, target) = nearest.min_by(|(a_order, a), (b_order, b)| {
            distance_to_rect_squared(a.frame, point)
                .total_cmp(&distance_to_rect_squared(b.frame, point))
                .then_with(|| a_order.cmp(b_order))
        })?;
        let clamped = CGPoint::new(
            point.x.clamp(
                target.frame.origin.x,
                target.frame.origin.x + target.frame.size.width,
            ),
            point.y.clamp(
                target.frame.origin.y,
                target.frame.origin.y + target.frame.size.height,
            ),
        );
        return intent_for(target, clamped);
    }
    nearest
        .filter_map(|(order, target)| {
            let clamped = CGPoint::new(
                point.x.clamp(
                    target.frame.origin.x,
                    target.frame.origin.x + target.frame.size.width,
                ),
                point.y.clamp(
                    target.frame.origin.y,
                    target.frame.origin.y + target.frame.size.height,
                ),
            );
            let intent = intent_for(target, clamped)?;
            (!unavailable.contains(&(intent.window, intent.zone, intent.action))).then_some((
                distance_to_rect_squared(target.frame, point),
                order,
                intent,
            ))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        .map(|(_, _, intent)| intent)
}

fn distance_to_rect_squared(frame: CGRect, point: CGPoint) -> f64 {
    let dx = (frame.origin.x - point.x)
        .max(0.0)
        .max(point.x - (frame.origin.x + frame.size.width));
    let dy = (frame.origin.y - point.y)
        .max(0.0)
        .max(point.y - (frame.origin.y + frame.size.height));
    dx * dx + dy * dy
}

pub fn preview_frame(target: DropTarget) -> CGRect {
    let mut frame = target.preview_area;
    frame.origin.x += 4.0;
    frame.origin.y += 4.0;
    frame.size.width = (frame.size.width - 8.0).max(0.0);
    frame.size.height = (frame.size.height - 8.0).max(0.0);
    frame
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::CGSize;

    use super::*;
    use crate::model::window_store::{WindowStore, WindowVisibility};

    fn frame(x: f64, y: f64, width: f64, height: f64) -> CGRect {
        CGRect::new(CGPoint::new(x, y), CGSize::new(width, height))
    }
    fn point(x: f64, y: f64) -> CGPoint { CGPoint::new(x, y) }
    fn rect() -> CGRect { frame(0.0, 0.0, 200.0, 100.0) }
    fn w(idx: u32) -> WindowId { WindowId::new(1, idx) }
    fn space() -> SpaceId { SpaceId::new(1) }
    fn target(idx: u32, frame: CGRect) -> DragSceneTarget {
        DragSceneTarget {
            window: w(idx),
            space: space(),
            frame,
        }
    }
    fn source(tiled: bool) -> DragSource {
        DragSource {
            origin_frame: rect(),
            last_frame: rect(),
            origin_space: Some(space()),
            current_space: Some(space()),
            tiled,
        }
    }
    fn scene(frame: CGRect) -> DragScene {
        DragScene {
            action_override: None,
            source_neighbors: [false; 4],
            targets: vec![target(2, frame)],
        }
    }
    fn scene_with(targets: Vec<DragSceneTarget>) -> DragScene {
        DragScene {
            action_override: None,
            source_neighbors: [false; 4],
            targets,
        }
    }
    fn settings() -> DragDropSettings { DragDropSettings::default() }
    fn tracked(windows: &[WindowId]) -> WindowStore {
        let mut store = WindowStore::default();
        for window in windows {
            store.set_visibility(*window, WindowVisibility::Visible);
        }
        store
    }
    fn motion(drag: &mut Session, x: f64, y: f64) -> bool {
        drag.motion(w(1), &settings(), DragMotion { point: point(x, y) })
    }
    fn native(tiled: bool, last_frame: CGRect, scene: DragScene) -> Session {
        let mut source = source(tiled);
        source.last_frame = last_frame;
        Session::native(source, scene)
    }
    fn modifier(tiled: bool, scene: DragScene) -> Session {
        Session::modifier(source(tiled), point(100.0, 50.0), MouseAction::Move, scene).unwrap()
    }

    #[test]
    fn classifies_target_local_center_and_edges() {
        for (coords, zone) in [
            ((100., 50.), DropZone::Center),
            ((1., 50.), DropZone::West),
            ((199., 50.), DropZone::East),
            ((100., 1.), DropZone::North),
            ((100., 99.), DropZone::South),
            ((0., 0.), DropZone::West),
            ((200., 0.), DropZone::East),
            ((0., 100.), DropZone::West),
            ((200., 100.), DropZone::East),
        ] {
            assert_eq!(
                classify_zone(rect(), point(coords.0, coords.1), 0.25),
                Some(zone)
            );
        }
        assert_eq!(
            resolve_action(DropZone::South, MouseDropAction::Swap),
            WindowDropAction::Insert(Direction::Down),
        );
        assert_eq!(classify_zone(rect(), point(-1.0, 50.0), 0.25), None);
    }

    #[test]
    fn target_zones_keep_their_local_frames() {
        let target = frame(10.0, 20.0, 200.0, 100.0);
        for (zone, expected) in [
            (DropZone::Center, frame(60.0, 45.0, 100.0, 50.0)),
            (DropZone::West, frame(10.0, 20.0, 50.0, 100.0)),
            (DropZone::East, frame(160.0, 20.0, 50.0, 100.0)),
            (DropZone::North, frame(10.0, 20.0, 200.0, 25.0)),
            (DropZone::South, frame(10.0, 95.0, 200.0, 25.0)),
        ] {
            assert_eq!(zone_frame(target, zone, 0.25), expected);
        }
    }

    #[test]
    fn preview_inset_clamps_small_tiles() {
        let intent = hit_test(
            &scene(rect()),
            point(100.0, 50.0),
            0.25,
            MouseDropAction::Swap,
            None,
        )
        .unwrap();
        for (area, expected) in [
            (rect(), frame(4.0, 4.0, 192.0, 92.0)),
            (frame(10.0, 20.0, 6.0, 6.0), frame(14.0, 24.0, 0.0, 0.0)),
        ] {
            assert_eq!(
                preview_frame(DropTarget { intent, preview_area: area }),
                expected
            );
        }
    }

    #[test]
    fn publisher_coalesces_to_newest_motion() {
        let publisher = DragMotionPublisher::default();
        for x in 0..1_000 {
            assert_eq!(
                publisher.publish(DragMotion {
                    point: CGPoint::new(x as f64, 0.0),
                }),
                x == 0
            );
        }
        assert!(publisher.wake_queued());
        assert_eq!(publisher.take_latest().unwrap().point.x, 999.0);
        assert!(!publisher.wake_queued());
        assert!(publisher.publish(DragMotion {
            point: CGPoint::new(1_000.0, 0.0),
        }));
    }

    #[test]
    fn native_tiled_move_targets_pointer_zone() {
        let target_frame = frame(200.0, 0.0, 100.0, 100.0);
        let moved = frame(10.0, 0.0, 200.0, 100.0);
        let mut actor = native(true, moved, scene(target_frame));
        assert!(motion(&mut actor, 201.0, 50.0));
        let intent = actor.intent().unwrap();
        assert_eq!(intent.action, WindowDropAction::Insert(Direction::Left));
        actor.set_preview(w(1), &settings(), intent, Some(target_frame));
        let preview = preview_frame(actor.target().unwrap());
        assert_eq!(preview.origin, CGPoint::new(204.0, 4.0));
        assert_eq!(preview.size, CGSize::new(92.0, 92.0));
        assert_eq!(actor.target().unwrap().intent.window, w(2));
    }

    #[test]
    fn original_tile_edges_offer_keyboard_equivalent_moves() {
        // Diagonal, so no window lies straight past any edge of the source's tile.
        let scene = scene(frame(220.0, 120.0, 200.0, 100.0));
        for ((x, y), direction) in [
            ((1.0, 50.0), Direction::Left),
            ((199.0, 50.0), Direction::Right),
            ((100.0, 1.0), Direction::Up),
            ((100.0, 99.0), Direction::Down),
        ] {
            let mut actor = native(true, rect(), scene.clone());
            motion(&mut actor, x, y);
            let intent = actor.intent().unwrap();
            assert_eq!(intent.window, w(1));
            assert_eq!(intent.action, WindowDropAction::Move(direction));
            actor.set_preview(w(1), &settings(), intent, Some(rect()));
            assert_eq!(
                actor.target().unwrap().intent.action,
                WindowDropAction::Move(direction)
            );
        }
    }

    #[test]
    fn floating_moves_do_not_preview_the_starting_position() {
        let mut native_drag = native(false, rect(), DragScene::default());
        motion(&mut native_drag, 100.0, 50.0);
        assert!(native_drag.preview_target(w(1)).is_none());

        let mut modifier_drag = modifier(false, DragScene::default());
        assert!(modifier_drag.preview_target(w(1)).is_none());
        motion(&mut modifier_drag, 110.0, 50.0);
        assert!(modifier_drag.preview_target(w(1)).is_none());
    }

    #[test]
    fn source_tile_edge_is_preview_only_until_pointer_enters_neighbor() {
        let mut drag_scene = scene(frame(200.0, 0.0, 200.0, 100.0));
        drag_scene.source_neighbors[1] = true;
        let mut actor = native(true, rect(), drag_scene.clone());
        // Home is preview-only; mouse-up has no validated operation to commit.
        motion(&mut actor, 199.0, 50.0);
        assert!(actor.intent().is_none());
        assert!(actor.preview_target(w(1)).is_some());
        assert!(actor.target().is_none());
        assert!(actor.target().is_none());

        let mut actor = native(true, rect(), drag_scene);
        motion(&mut actor, 201.0, 50.0);
        assert_eq!(
            actor.intent().unwrap().action,
            WindowDropAction::Insert(Direction::Left)
        );
        let intent = actor.intent().unwrap();
        actor.set_preview(w(1), &settings(), intent, Some(rect()));
        assert!(!motion(&mut actor, 225.0, 50.0));
        assert_eq!(
            actor.intent().unwrap().action,
            WindowDropAction::Insert(Direction::Left)
        );
        assert!(motion(&mut actor, 199.0, 50.0));
        assert!(actor.intent().is_none());
        assert_eq!(actor.preview_target(w(1)).unwrap().preview_area, rect());
        assert!(actor.target().is_none());
    }

    #[test]
    fn unavailable_source_tile_edge_does_not_retry_the_same_move() {
        for ((x, y), direction) in [
            ((1.0, 50.0), Direction::Left),
            ((199.0, 50.0), Direction::Right),
            ((100.0, 1.0), Direction::Up),
            ((100.0, 99.0), Direction::Down),
        ] {
            let mut actor = native(true, rect(), scene_with(vec![]));
            motion(&mut actor, x, y);
            let intent = actor.intent().unwrap();
            assert_eq!(intent.window, w(1));
            assert_eq!(intent.action, WindowDropAction::Move(direction));

            // A rejected preview must terminate the reactor's resolution loop.
            assert!(
                !actor.set_preview(w(1), &settings(), intent, None),
                "retries rejected {direction:?} move"
            );
            assert!(actor.intent().is_none());
            assert!(actor.target().is_none());
        }
    }

    #[test]
    fn unavailable_preview_does_not_expose_a_target() {
        let mut actor = native(true, rect(), scene(rect()));
        motion(&mut actor, 1.0, 50.0);
        let intent = actor.intent().unwrap();
        actor.set_preview(w(1), &settings(), intent, None);
        assert!(actor.target().is_none());
        assert!(actor.target().is_none());
    }

    #[test]
    fn unavailable_nearest_target_falls_back_to_the_next_target() {
        let nearest = w(2);
        let fallback = w(3);
        let scene = scene_with(vec![
            target(2, frame(220.0, 0.0, 200.0, 100.0)),
            target(3, frame(440.0, 0.0, 200.0, 100.0)),
        ]);
        let mut actor = native(true, rect(), scene);
        motion(&mut actor, 230.0, 50.0);
        let intent = actor.intent().unwrap();
        assert_eq!(intent.window, nearest);
        assert!(actor.set_preview(w(1), &settings(), intent, None));
        assert_eq!(actor.intent().unwrap().window, fallback);
    }

    #[test]
    fn equidistant_outside_targets_keep_scene_order() {
        let scene = scene_with(vec![
            target(2, frame(0.0, 0.0, 100.0, 100.0)),
            target(3, frame(200.0, 0.0, 100.0, 100.0)),
        ]);
        let point = point(150.0, 50.0);
        assert_eq!(
            hit_test(&scene, point, 0.25, MouseDropAction::Swap, None).unwrap().window,
            w(2)
        );
    }

    #[test]
    fn overlapping_targets_prefer_scene_order() {
        let visible = w(2);
        let scene = scene_with(vec![target(2, rect()), target(3, rect())]);
        let intent =
            hit_test(&scene, point(100.0, 50.0), 0.25, MouseDropAction::Swap, None).unwrap();
        assert_eq!(intent.window, visible);
    }

    #[test]
    fn replacing_scene_invalidates_a_validated_preview() {
        let mut actor = native(true, rect(), scene(frame(200.0, 0.0, 200.0, 100.0)));
        motion(&mut actor, 201.0, 50.0);
        let intent = actor.intent().unwrap();
        actor.set_preview(w(1), &settings(), intent, Some(rect()));
        assert!(actor.target().is_some());

        let moved = frame(300.0, 0.0, 200.0, 100.0);
        actor.replace_scene(w(1), &settings(), scene(moved));
        assert!(actor.target().is_none());
        let moved_intent = actor.intent().unwrap();
        assert_eq!(moved_intent.frame, moved);
        actor.set_preview(w(1), &settings(), moved_intent, Some(moved));
        assert!(actor.target().is_some());
    }

    #[test]
    fn floating_move_never_builds_a_drop_target() {
        let moved = frame(10.0, 0.0, 200.0, 100.0);
        let mut actor = native(false, moved, scene(rect()));
        motion(&mut actor, 100.0, 50.0);
        assert!(actor.target().is_none());
    }

    #[test]
    fn modifier_space_change_invalidates_origin_drop_state() {
        let destination = SpaceId::new(2);
        let mut actor = modifier(true, scene(rect()));
        motion(&mut actor, 100.0, 50.0);
        let intent = actor.intent().unwrap();
        actor.set_preview(w(1), &settings(), intent, Some(rect()));
        assert!(actor.target().is_some());

        assert!(actor.update_current_space(Some(destination)));
        assert_eq!(actor.source.current_space, Some(destination));
        assert!(actor.intent().is_none());
        assert!(actor.target().is_none());
        motion(&mut actor, 100.0, 50.0);
        assert!(actor.intent().is_none());
        assert!(actor.target().is_none());
    }

    #[test]
    fn stack_scene_resolves_every_zone_to_swap() {
        let mut scene = scene(rect());
        scene.action_override = Some(WindowDropAction::Swap);
        for point in [point(1.0, 50.0), point(100.0, 50.0)] {
            let intent = hit_test(&scene, point, 0.25, MouseDropAction::Stack, None).unwrap();
            assert_eq!(intent.action, WindowDropAction::Swap);
        }
    }

    #[test]
    fn sessions_know_whether_the_user_or_rift_moves_the_window() {
        let moved = frame(20.0, 20.0, 200.0, 100.0);
        let moved = native(false, moved, DragScene::default());
        assert!(moved.kind == DragKind::Native && !moved.resizing());
        let resized = frame(0.0, 0.0, 300.0, 100.0);
        assert!(native(false, resized, DragScene::default()).resizing());
        assert!(modifier(false, DragScene::default()).is_modifier());
        let none = MouseAction::None;
        assert!(
            Session::modifier(source(false), point(1.0, 1.0), none, DragScene::default()).is_none()
        );
    }

    /// The core with `windows` tracked: the grip's rules are the core's, the slot the store's.
    fn hand(windows: &[WindowId]) -> crate::model::RiftState {
        crate::model::RiftState {
            windows: tracked(windows),
            core: Default::default(),
        }
    }

    #[test]
    fn only_the_owning_button_finishes_a_modifier_drag() {
        let mut state = hand(&[w(1)]);
        state.press_and_drag(w(1), MouseButton::Left, modifier(true, DragScene::default()));
        assert!(state.release(rift_core::Release::Button(rift_core::Button::Right)).is_none());
        assert!(state.windows.drag().is_some());
        assert!(state.release(rift_core::Release::Button(rift_core::Button::Left)).is_some());
        assert!(state.windows.drag().is_none());
    }

    #[test]
    fn starting_a_drag_ends_the_last_one() {
        let mut state = hand(&[w(1), w(2)]);
        state.press_and_drag(
            w(1),
            MouseButton::Left,
            native(true, rect(), DragScene::default()),
        );
        state.press_and_drag(
            w(2),
            MouseButton::Left,
            native(true, rect(), DragScene::default()),
        );
        assert_eq!(state.windows.drag().map(|(window, _)| window), Some(w(2)));
        assert!(state.windows.drag_on(w(1)).is_none());
    }
}
