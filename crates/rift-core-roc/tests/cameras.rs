//! The scrolling layouts' cameras, kept between events: the Roc twin (`RocCameras`) against the
//! Rust machine (`rift_core::camera::Cameras`), on random runs of events. Runs reuse a few
//! layouts, columns and windows; every event carries a random strip (`Spans`, some columns
//! laid out) and arrangement (`Facts`, with a selection and fullscreen windows), and the clock
//! moves forward, repeats a moment, and goes back. Gestures, presentations, edits, selections,
//! fullscreens and snaps interleave. Every step must answer the same and leave the same cameras.

use std::collections::BTreeSet;

use rift_core::camera::{
    Alignment, Camera, CameraEvent, CameraSettings, Cameras, CamerasEvent, Ended, FactColumn,
    Facts, HeldAt, Motion, MotionHistory, Navigation, Presented, Snap, Span, Spans, Spring, Time,
    Viewport,
};
use rift_core_roc::{AllocationCount, RocCameras};

/// xorshift64*: deterministic, no dependency.
struct Rng {
    state: u64,
    clock: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Rng {
            state: seed,
            clock: 1_000_000_000,
        }
    }

    fn next(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 { self.next() % n }

    fn chance(&mut self, percent: u64) -> bool { self.below(100) < percent }

    /// A moment: mostly later than the last, sometimes the same one, sometimes earlier.
    fn now(&mut self) -> Time {
        match self.below(10) {
            0 => {}
            1 | 2 => {
                let back = self.below(40_000_000);
                return Time(self.clock.saturating_sub(back));
            }
            _ => self.clock += self.below(30_000_000),
        }
        Time(self.clock)
    }

    /// A moment inside `(started, sampled)` now and then, else as [`Rng::now`].
    fn now_in(&mut self, inside: Option<(Time, Time)>) -> Time {
        match inside {
            Some((started, sampled)) if self.chance(50) => {
                Time(started.0 + 1 + self.below(sampled.0 - started.0))
            }
            _ => self.now(),
        }
    }

    /// A length in half pixels: plain, never negative zero.
    fn length(&mut self, max: u64) -> f64 { self.below(max * 2) as f64 / 2.0 }

    fn layout(&mut self) -> u64 { 1 + self.below(3) }

    fn window(&mut self) -> u64 { 1 + self.below(8) }

    fn settings(&mut self) -> CameraSettings {
        CameraSettings {
            alignment: [Alignment::Left, Alignment::Center, Alignment::Right]
                [self.below(3) as usize],
            navigation: if self.chance(50) {
                Navigation::Niri
            } else {
                Navigation::Anchored
            },
        }
    }

    /// An arrangement: up to five columns of the column ids 1..=6, each with a window or two.
    fn facts(&mut self) -> Facts<u64, u64> {
        let n = self.below(6) as usize;
        let mut ids: Vec<u64> = (1..=6).collect();
        let mut columns = Vec::new();
        let mut windows = Vec::new();
        for _ in 0..n {
            let id = ids.remove(self.below(ids.len() as u64) as usize);
            let mut column = vec![self.window()];
            if self.chance(30) {
                column.push(self.window());
            }
            windows.extend(column.iter().copied());
            columns.push(FactColumn { id, windows: column });
        }
        let active_column = if n == 0 {
            0
        } else {
            self.below(n as u64 + 1) as usize
        };
        let selected = if self.chance(85) && !windows.is_empty() {
            Some(windows[self.below(windows.len() as u64) as usize])
        } else {
            None
        };
        let fullscreen: BTreeSet<u64> =
            windows.iter().copied().filter(|_| self.chance(15)).collect();
        Facts {
            columns,
            active_column,
            selected,
            fullscreen,
        }
    }

    /// A strip laid out from `facts`: most columns laid out (a dormant one is not), sometimes a
    /// stray column the arrangement does not hold, edges and anchors among the resting points.
    fn spans(&mut self, facts: &Facts<u64, u64>) -> Spans<u64> {
        let view = 600.0 + self.length(800);
        let mut x = self.length(40);
        let mut columns = Vec::new();
        for c in &facts.columns {
            if self.chance(85) {
                let width = 200.0 + self.length(900);
                columns.push(Span { id: c.id, world_x: x, width });
                x += width + self.length(30);
            }
        }
        if self.chance(10) {
            columns.push(Span {
                id: 9,
                world_x: x,
                width: 300.0,
            });
        }
        let first = columns.first().map_or(0.0, |c| c.world_x);
        let last = columns.last().map_or(0.0, |c| c.world_x + c.width - view);
        let low = first;
        let high = if last > low { last } else { low };
        let mut snaps = Vec::new();
        for c in &columns {
            for offset in [c.world_x, c.world_x + c.width - view] {
                if (low..=high).contains(&offset) || self.chance(10) {
                    snaps.push(Snap { offset, column: c.id });
                }
            }
        }
        // Only a tie between the active column and another, or only a dormant column's point: the
        // nearest point is then the one that decides.
        let active = facts.columns.get(facts.active_column).map(|c| c.id);
        let laid_active = columns.iter().find(|c| Some(c.id) == active).copied();
        if self.chance(15)
            && let Some(active) = laid_active
            && let Some(other) = columns.iter().find(|c| c.id != active.id)
        {
            let offset = active.world_x;
            let tie = [Snap { offset, column: other.id }, Snap {
                offset,
                column: active.id,
            }];
            let snaps = if self.chance(50) {
                tie.to_vec()
            } else {
                vec![tie[1], tie[0]]
            };
            return Spans {
                columns,
                bounds: (low, high),
                view,
                snaps,
            };
        }
        if self.chance(10)
            && let Some(dormant) =
                facts.columns.iter().find(|c| !columns.iter().any(|s| s.id == c.id))
        {
            let snaps = vec![Snap {
                offset: self.length(2000),
                column: dormant.id,
            }];
            return Spans {
                columns,
                bounds: (low, high),
                view,
                snaps,
            };
        }
        // Ties: another column resting at a point already listed.
        if self.chance(30) && !snaps.is_empty() && columns.len() > 1 {
            let tied = snaps[self.below(snaps.len() as u64) as usize].offset;
            let column = columns[self.below(columns.len() as u64) as usize].id;
            snaps.push(Snap { offset: tied, column });
        }
        // A resting point of a column the arrangement holds but the strip does not lay out.
        if self.chance(20)
            && let Some(dormant) =
                facts.columns.iter().find(|c| !columns.iter().any(|s| s.id == c.id))
        {
            let offset = self.length(2000);
            snaps.push(Snap { offset, column: dormant.id });
        }
        Spans {
            columns,
            bounds: (low, high),
            view,
            snaps,
        }
    }

    fn maybe_spans(&mut self, facts: &Facts<u64, u64>) -> Option<Spans<u64>> {
        if self.chance(85) {
            Some(self.spans(facts))
        } else {
            None
        }
    }

    fn spring(&mut self) -> Spring {
        let started = self.now();
        let mut spring = Spring::new(
            self.length(2000),
            self.length(2000),
            self.length(4000) - 2000.0,
            started,
        );
        if self.chance(50) {
            let later = self.now();
            spring.sample(later, 1.0 + self.below(2) as f64);
        }
        spring
    }

    fn history(&mut self) -> MotionHistory {
        let mut time = self.clock.saturating_sub(100_000_000);
        // A long history is dense (a fast trackpad), so it is not aged out at once.
        let (n, spacing) = if self.chance(15) {
            // 63 or 64: the history at its cap of 64, or one short of it.
            (63 + self.below(2), 2_000_000)
        } else {
            (self.below(6), 30_000_000)
        };
        let samples: Vec<_> = (0..n)
            .map(|_| {
                time += self.below(spacing);
                (Time(time), self.length(200) - 100.0)
            })
            .collect();
        MotionHistory::from_parts(samples, self.length(100) - 50.0)
    }

    /// One camera's event, aimed now and then at what `camera` holds (a presented spring on
    /// its way to its target, a presented moment at it).
    fn camera_event(&mut self, camera: Option<&Camera<u64, u64>>) -> CameraEvent<u64, u64> {
        let target = camera.map(|c| c.viewport().target());
        // A moment inside a sampled spring's flight (after its start, before its last sample):
        // the stale `now` a spring must never be sampled or retargeted back to.
        let inside = camera.and_then(|c| match c.viewport() {
            Viewport::Resting(_, Motion::Animation(s)) if s.sampled > s.started => {
                Some((s.started, s.sampled))
            }
            _ => None,
        });
        let facts = self.facts();
        let spans = self.maybe_spans(&facts);
        let settings = self.settings();
        match self.below(30) {
            0..=2 => CameraEvent::Prepared {
                before: self.maybe_spans(&facts),
                spans: self.spans(&facts),
                facts,
                settings,
                now: self.now_in(inside),
            },
            3..=4 => CameraEvent::Rebuilt {
                spans,
                hold: if self.chance(60) {
                    Some(HeldAt {
                        window: if self.chance(70) {
                            Some(self.window())
                        } else {
                            None
                        },
                        column: 1 + self.below(6),
                        x: self.length(2000),
                    })
                } else {
                    None
                },
                facts,
                now: self.now_in(inside),
            },
            5 => CameraEvent::Revealed { spans, facts, settings },
            6 => CameraEvent::Centred { spans, facts },
            7 => CameraEvent::Inserting { spans, facts },
            8 => CameraEvent::WindowRemoved {
                active_removed: self.chance(60),
                column_removed: self.chance(60),
                spans,
                facts,
                settings,
            },
            9 => CameraEvent::Selected {
                column_changed: self.chance(50),
                spans,
                facts,
                settings,
            },
            10 => CameraEvent::Followed { spans, facts, settings },
            11 => CameraEvent::TransientForgotten,
            12 => CameraEvent::ViewForgotten,
            13 => CameraEvent::Replaced {
                from: self.window(),
                to: self.window(),
            },
            14 => CameraEvent::FullscreenLeft {
                window: self.window(),
                spans,
                facts,
            },
            15 => CameraEvent::FullscreenEntered {
                window: self.window(),
                spans,
                before: facts,
                after: self.facts(),
            },
            16 => CameraEvent::Finished,
            17 => {
                let aimed = target.filter(|_| self.chance(60));
                let presented = match self.below(3) {
                    0 => Presented::Gesture(self.length(2000)),
                    1 => Presented::Still(aimed.unwrap_or_else(|| self.length(2000))),
                    _ => {
                        let mut spring = self.spring();
                        if let Some(target) = aimed {
                            spring =
                                Spring::new(spring.from, target, spring.velocity, spring.started);
                            if self.chance(50) {
                                let later = self.now();
                                spring.sample(later, 1.0);
                            }
                        }
                        Presented::Moving(spring)
                    }
                };
                let ended = if self.chance(50) {
                    Ended::No
                } else {
                    Ended::Yes
                };
                CameraEvent::Presented {
                    presented,
                    motion: self.history(),
                    ended,
                }
            }
            18 => CameraEvent::ScrollBy {
                delta: self.length(6) - 3.0,
                spans,
                facts,
                gap: self.length(20),
            },
            19 | 20 => CameraEvent::GestureBegan {
                now: self.now_in(inside),
                spans,
                facts,
            },
            21 | 22 => CameraEvent::GestureMoved {
                delta: self.length(400) - 200.0,
                at: self.now(),
                spans,
            },
            23 | 26..=29 => CameraEvent::GestureEnded {
                at: self.now(),
                animate: self.chance(60),
                started: self.now(),
                spans,
                facts,
                settings,
            },
            24 => CameraEvent::GestureCancelled { spans },
            _ => CameraEvent::Snapped { spans, facts, settings },
        }
    }

    fn bookmark(&mut self) -> Option<rift_core::camera::Bookmark<u64>> {
        self.chance(50).then(|| rift_core::camera::Bookmark {
            column: 1 + self.below(6),
            relative_offset: self.length(400) - 200.0,
        })
    }

    /// A camera as a file or a restore might hold it: any viewport, history, views and return.
    fn seed(&mut self) -> Camera<u64, u64> {
        use rift_core::camera::{Rest, Showing};
        let rest = if self.chance(70) {
            Rest::At
        } else {
            Rest::Centred
        };
        let viewport = match self.below(5) {
            0 => Viewport::Uninitialized,
            1 => Viewport::Gesture(self.length(2000)),
            2 => Viewport::Resting(rest, Motion::Static(self.length(2000))),
            3 => Viewport::Resting(rest, Motion::Pending {
                from: self.length(2000),
                target: self.length(2000),
            }),
            _ => Viewport::Resting(rest, Motion::Animation(self.spring())),
        };
        let showing = match self.bookmark() {
            Some(return_to) if self.chance(50) => Showing::Fullscreen {
                window: self.window(),
                return_to,
            },
            _ => Showing::Strip,
        };
        let (transient, restored) = (self.bookmark(), self.bookmark());
        Camera::from_parts(viewport, self.history(), transient, showing, restored)
    }

    fn event(&mut self, cameras: &Cameras<u64, u64, u64>) -> CamerasEvent<u64, u64, u64> {
        match self.below(40) {
            3 => CamerasEvent::Seeded {
                layout: self.layout(),
                camera: self.seed(),
            },
            0 => CamerasEvent::Created(self.layout()),
            1 => {
                let facts = self.facts();
                CamerasEvent::Cloned {
                    from: self.layout(),
                    to: self.layout(),
                    spans: self.maybe_spans(&facts),
                }
            }
            2 => CamerasEvent::Removed(self.layout()),
            _ => {
                let layout = self.layout();
                CamerasEvent::Camera(layout, self.camera_event(cameras.get(layout)))
            }
        }
    }
}

#[test]
fn the_roc_cameras_answer_and_keep_state_as_the_rust_ones_do() {
    let runs: u64 = std::env::var("RIFT_CAMERAS_RUNS").map_or(2_000, |n| n.parse().unwrap());
    let mut rng = Rng::new(0xCA_3E7A_0000_0E3C);
    let before = AllocationCount::now();
    let (mut steps, mut answered, mut springing) = (0, 0, 0);
    for run in 0..runs {
        let mut rust = Cameras::<u64, u64, u64>::default();
        let mut roc = RocCameras::default();
        for layout in 1..=3 {
            rust.step(CamerasEvent::Created(layout));
            roc.step(CamerasEvent::Created(layout));
        }
        let mut history = Vec::new();
        for step in 0..60 {
            let event = rng.event(&rust);
            history.push(event.clone());
            let expected = rust.step(event.clone());
            let got = roc.step(event);
            assert_eq!(
                got, expected,
                "run {run} step {step}: the answer after {history:#?}"
            );
            assert_eq!(
                roc.view(),
                rust,
                "run {run} step {step}: the cameras after {history:#?}"
            );
            steps += 1;
            answered += u64::from(expected != rift_core::camera::Answer::Nothing);
            springing += rust
                .cameras()
                .filter(|(_, c)| {
                    matches!(
                        c.viewport(),
                        rift_core::camera::Viewport::Resting(
                            _,
                            rift_core::camera::Motion::Animation(_)
                        )
                    )
                })
                .count() as u64;
        }
    }
    assert_eq!(
        AllocationCount::now().live(),
        before.live(),
        "Roc allocations left live"
    );
    assert!(answered > 0 && springing > 0, "no answer or no spring exercised");
    eprintln!(
        "cameras: {steps} steps identical ({answered} answered, {springing} springs held), state compared after each"
    );
}

/// A step updates the table in place: with 200 layouts' cameras, no step allocates more than a
/// sliver of the table.
#[test]
fn the_roc_cameras_step_without_copying_the_table() {
    let mut rng = Rng::new(0xCA_3E7A_0000_0E3D);
    let mut rust = Cameras::<u64, u64, u64>::default();
    let mut roc = RocCameras::default();
    for layout in 0..200 {
        rust.step(CamerasEvent::Created(layout));
        roc.step(CamerasEvent::Created(layout));
    }
    let bound = 8192;
    for step in 0..2000 {
        let layout = rng.below(200);
        // Keep the table full: camera events only, small arrangements.
        let event = loop {
            let event = rng.camera_event(rust.get(layout));
            if !matches!(
                event,
                CameraEvent::FullscreenEntered { .. } | CameraEvent::Prepared { .. }
            ) {
                break event;
            }
        };
        let event = CamerasEvent::Camera(layout, event);
        let expected = rust.step(event.clone());
        rift_core_roc::take_largest_allocation();
        let got = roc.step(event.clone());
        let largest = rift_core_roc::take_largest_allocation();
        assert_eq!(got, expected);
        assert!(
            largest < bound,
            "step {step}: {event:?} allocated {largest} bytes (bound {bound})"
        );
    }
    assert_eq!(roc.view(), rust);
}

/// A gesture whose presented history is at its cap of 64 samples: the next delta drops the
/// oldest, in both cores (a sequence random runs reach too rarely).
#[test]
fn the_roc_cameras_cap_a_full_motion_history_as_the_rust_ones_do() {
    let mut rng = Rng::new(0xCA_3E7A_0000_0E3E);
    let facts = Facts {
        columns: (1..=4).map(|id| FactColumn { id, windows: vec![id] }).collect(),
        active_column: 1,
        selected: Some(2),
        fullscreen: BTreeSet::new(),
    };
    let spans = Spans {
        columns: (1..=4)
            .map(|id| Span {
                id,
                world_x: (id - 1) as f64 * 500.0,
                width: 480.0,
            })
            .collect(),
        bounds: (0.0, 1000.0),
        view: 1000.0,
        snaps: vec![Snap { offset: 0.0, column: 1 }, Snap {
            offset: 1000.0,
            column: 4,
        }],
    };
    let settings = CameraSettings {
        alignment: Alignment::Center,
        navigation: Navigation::Niri,
    };
    for samples in [63, 64] {
        let mut rust = Cameras::<u64, u64, u64>::default();
        let mut roc = RocCameras::default();
        let full = MotionHistory::from_parts(
            (0..samples).map(|i| (Time(1_000_000_000 + i * 1_000_000), 10.0)),
            0.0,
        );
        let events = [
            CamerasEvent::Created(1),
            CamerasEvent::Camera(1, CameraEvent::Prepared {
                before: None,
                spans: spans.clone(),
                facts: facts.clone(),
                settings,
                now: Time(900_000_000),
            }),
            CamerasEvent::Camera(1, CameraEvent::GestureBegan {
                now: Time(950_000_000),
                spans: Some(spans.clone()),
                facts: facts.clone(),
            }),
            CamerasEvent::Camera(1, CameraEvent::Presented {
                presented: Presented::Gesture(300.0),
                motion: full,
                ended: Ended::No,
            }),
            CamerasEvent::Camera(1, CameraEvent::GestureMoved {
                delta: 25.0,
                at: Time(1_100_000_000),
                spans: Some(spans.clone()),
            }),
            CamerasEvent::Camera(1, CameraEvent::GestureEnded {
                at: Time(1_110_000_000),
                animate: rng.chance(50),
                started: Time(1_110_000_000),
                spans: Some(spans.clone()),
                facts: facts.clone(),
                settings,
            }),
        ];
        for (step, event) in events.into_iter().enumerate() {
            assert_eq!(
                roc.step(event.clone()),
                rust.step(event),
                "{samples} samples, step {step}"
            );
            assert_eq!(roc.view(), rust, "{samples} samples, step {step}");
        }
        let held = rust.get(1).unwrap().motion().samples().count();
        assert!(held <= 64, "{samples} samples: {held} held");
    }
}
