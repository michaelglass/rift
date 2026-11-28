//! The scrolling layouts' cameras (`rift_core::camera::Cameras`), with their Roc twin under
//! `roc-core` (`RocCameras`, `Cameras.roc`). Flipped (policy (a)): under `roc-core` the Roc core
//! decides (its answers are the system's) and the Rust machine is its shadow, read through
//! `Deref` (the queries) and held to it: every answer on every step, the whole machine on every
//! step in tests and debug builds and every [`VIEW_EVERY`]th in a release build. A difference is
//! reported (`roc-divergence:`, a failure in tests) and counted ([`cameras_roc_reseeds`]), and
//! the shadow re-seeded from the Roc core's view.
//!
//! A camera loaded or restored reaches the Roc core as a `Seeded` event (the Rust value both
//! sides then hold); those are counted apart ([`cameras_roc_seeds`]), so a disagreement can never
//! hide among restores.

use std::ops::Deref;
#[cfg(feature = "roc-core")]
use std::sync::atomic::{AtomicU64, Ordering};

use rift_core::camera::{Answer, Cameras, CamerasEvent};

use super::ColumnId;
use crate::actor::app::WindowId;
use crate::layout_engine::LayoutId;

/// The cameras as the scrolling system holds them.
pub(super) type RiftCameras = Cameras<LayoutId, ColumnId, WindowId>;

#[cfg(feature = "roc-core")]
static RESEEDS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "roc-core")]
static SEEDS: AtomicU64 = AtomicU64::new(0);

/// How often a `roc-core` release build compares the whole machines after a step.
#[cfg(feature = "roc-core")]
const VIEW_EVERY: u64 = 32;

/// How many times the cameras' Rust shadow differed from the deciding Roc core and was
/// re-seeded from it, process-wide.
#[cfg(feature = "roc-core")]
pub(crate) fn cameras_roc_reseeds() -> u64 { RESEEDS.load(Ordering::Relaxed) }

/// How many cameras reached the Roc core loaded or restored (`Seeded`), process-wide: never a
/// disagreement.
#[cfg(feature = "roc-core")]
pub(crate) fn cameras_roc_seeds() -> u64 { SEEDS.load(Ordering::Relaxed) }

#[cfg(feature = "roc-core")]
fn layout_word(layout: LayoutId) -> u64 { slotmap::Key::data(&layout).as_ffi() }

#[cfg(feature = "roc-core")]
fn layout_of_word(word: u64) -> LayoutId { slotmap::KeyData::from_ffi(word).into() }

#[cfg(feature = "roc-core")]
fn column_word(column: ColumnId) -> u64 { column.0 }

#[cfg(feature = "roc-core")]
fn window_word(window: WindowId) -> u64 { crate::roc_path::Word::word(window) }

/// A window back from the Roc core: every window word it holds came from rift.
#[cfg(feature = "roc-core")]
fn window_of_word(word: u64) -> WindowId {
    crate::roc_path::window_of_word(word).expect("a window word rift gave the Roc core")
}

/// The cameras, deciding in Roc under `roc-core`.
#[derive(Default)]
pub(super) struct TwinnedCameras {
    rust: RiftCameras,
    /// Boxed: the scrolling system sits inline in the layout-system enum, which the twin's box,
    /// host table and step count would otherwise grow by some 70 bytes.
    #[cfg(feature = "roc-core")]
    twin: Box<Twin>,
}

#[cfg(feature = "roc-core")]
#[derive(Default)]
struct Twin {
    roc: rift_core_roc::RocCameras,
    steps: u64,
}

impl Deref for TwinnedCameras {
    type Target = RiftCameras;

    fn deref(&self) -> &RiftCameras { &self.rust }
}

impl std::fmt::Debug for TwinnedCameras {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.rust.fmt(f) }
}

/// A copy is a restore: the Roc core seeded with the copied cameras.
impl Clone for TwinnedCameras {
    fn clone(&self) -> Self { Self::seeded(self.rust.clone()) }
}

impl TwinnedCameras {
    /// Cameras loaded or copied: the Roc core seeded with each (`Seeded`, counted as seeds).
    pub(super) fn seeded(rust: RiftCameras) -> Self {
        let mut seeded = TwinnedCameras::default();
        let cameras: Vec<_> = rust.cameras().map(|(layout, c)| (layout, c.clone())).collect();
        for (layout, camera) in cameras {
            seeded.step(CamerasEvent::Seeded { layout, camera });
        }
        seeded
    }

    /// As [`Cameras::step`]: the Roc core's answer under `roc-core`, the shadow held to it.
    pub(super) fn step(
        &mut self,
        event: CamerasEvent<LayoutId, ColumnId, WindowId>,
    ) -> Answer<ColumnId> {
        #[cfg(feature = "roc-core")]
        let plain = {
            if matches!(event, CamerasEvent::Seeded { .. }) {
                SEEDS.fetch_add(1, Ordering::Relaxed);
            }
            event.clone().map(&layout_word, &column_word, &window_word)
        };
        let answer = self.rust.step(event);
        #[cfg(feature = "roc-core")]
        let decided = {
            use crate::roc_path::diverged;
            let roc = self.twin.roc.step(plain.clone()).map(&ColumnId);
            let answer_differs = roc != answer;
            if answer_differs {
                diverged("cameras step", &plain, &answer, &roc);
            }
            self.twin.steps += 1;
            if answer_differs
                || cfg!(any(test, debug_assertions))
                || self.twin.steps.is_multiple_of(VIEW_EVERY)
            {
                let rust = self.rust.clone().map(&layout_word, &column_word, &window_word);
                let view = self.twin.roc.view();
                if rust != view {
                    diverged("cameras", &plain, &rust, &view);
                }
                if answer_differs || rust != view {
                    self.reseed(view);
                }
            }
            // The Roc core decides.
            roc
        };
        #[cfg(not(feature = "roc-core"))]
        let decided = answer;
        decided
    }

    /// The Rust shadow replaced by the Roc core's cameras, and counted.
    #[cfg(feature = "roc-core")]
    fn reseed(&mut self, view: Cameras<u64, u64, u64>) {
        self.rust = view.map(&layout_of_word, &ColumnId, &window_of_word);
        let reseeds = RESEEDS.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::warn!(
            reseeds,
            "roc-divergence: the cameras' Rust shadow was re-seeded from the Roc core"
        );
    }
}

#[cfg(all(test, feature = "roc-core"))]
mod tests {
    use rift_core::camera::{Camera, CameraEvent, Showing, Viewport};

    use super::*;

    /// Re-seeding gives the shadow exactly the Roc core's cameras, each id back from its word,
    /// and is counted as a re-seed; seeding counts as a seed, never as a re-seed.
    #[test]
    fn a_cameras_reseed_makes_the_shadow_the_roc_cores_and_seeds_count_apart() {
        let mut layouts = slotmap::SlotMap::<LayoutId, ()>::with_key();
        let layout = layouts.insert(());
        let window = WindowId::new(-4, 1);
        let (seeds, reseeds) = (cameras_roc_seeds(), cameras_roc_reseeds());
        let camera = Camera::saved(
            120.0,
            None,
            Showing::Fullscreen {
                window,
                return_to: rift_core::camera::Bookmark {
                    column: ColumnId(3),
                    relative_offset: 5.0,
                },
            },
            None,
        );
        let mut cameras = TwinnedCameras::seeded(RiftCameras::from_parts([(layout, camera)]));
        assert!(cameras_roc_seeds() > seeds, "a loaded camera is a seed");
        assert_eq!(cameras_roc_reseeds(), reseeds, "and no re-seed");
        // The shadow loses what the Roc core holds.
        cameras.rust.step(CamerasEvent::Camera(layout, CameraEvent::Finished));
        cameras.rust = RiftCameras::default();
        let view = cameras.twin.roc.view();
        cameras.reseed(view.clone());
        assert_eq!(
            cameras.rust.clone().map(&layout_word, &column_word, &window_word),
            view
        );
        let held = cameras.get(layout).expect("reads follow the Roc core");
        assert_eq!(*held.viewport(), Viewport::saved(120.0));
        assert!(matches!(held.showing(), Showing::Fullscreen { window: w, .. } if *w == window));
        assert!(cameras_roc_reseeds() > reseeds);
    }
}
