//! What the layout passes know of each window's size: its sizing, the size it is locked at and
//! its limits (`WindowConstraints`), kept for as long as the window lives (upstream #707: only
//! its retirement drops them; leaving a layout, as a move between Spaces does, does not), and
//! each app's last reported minimum, which a `respect_min_size` window that reports none yet
//! takes (some apps, WhatsApp among them, report none for a new window until it is first used).
//!
//! The layout engine steps this machine with what it observes and reads its records; `revision`
//! moves on every change, so a layout pass can tell its constraints from the last pass's.

use std::collections::BTreeMap;

use crate::layout::{Size, Sizing, WindowConstraints};

/// A window's size as an observation reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Observed {
    pub resizable: bool,
    /// The window's size now: what it is locked at.
    pub width: f64,
    pub height: f64,
    /// Its reported minimum, if it reports a non-zero one.
    pub min: Option<Size>,
    /// Its reported maximum (0: no limit).
    pub max_width: f64,
    pub max_height: f64,
}

/// What the layout engine observes that the machine reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConstraintsEvent<W, A> {
    /// `window`, of `app`, is observed: its record is what it reports, its minimum (under
    /// `respect_min_size`) its app's last one when it reports none.
    Observed {
        window: W,
        app: A,
        respect_min_size: bool,
        observed: Observed,
    },
    /// The user or a rule left `window` this wide: a recorded window is locked at it.
    WidthSet { window: W, width: f64 },
    /// A scrolling column keeps `window` this wide: it is recorded at it, from `observed` (without
    /// `respect_min_size`, which scrolling ignores) if it has no record yet.
    WidthPreserved {
        window: W,
        observed: Observed,
        width: f64,
    },
    /// `window` is gone: its record goes with it.
    Retired(W),
    /// `from` has a new identity, `to`: its record follows it.
    Replaced { from: W, to: W },
    /// `app` quit: its last minimum is forgotten (its windows retire one by one).
    AppQuit(A),
}

/// Every window's constraints, each app's last reported minimum, and how often they changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Constraints<W, A> {
    windows: BTreeMap<W, WindowConstraints>,
    app_minimums: BTreeMap<A, Size>,
    revision: u64,
}

impl<W, A> Default for Constraints<W, A> {
    fn default() -> Self {
        Constraints {
            windows: BTreeMap::new(),
            app_minimums: BTreeMap::new(),
            revision: 0,
        }
    }
}

impl<W: Ord + Copy, A: Ord + Copy> Constraints<W, A> {
    /// A machine holding `windows`' records and `app_minimums`, at `revision`.
    pub fn from_parts(
        windows: impl IntoIterator<Item = (W, WindowConstraints)>,
        app_minimums: impl IntoIterator<Item = (A, Size)>,
        revision: u64,
    ) -> Self {
        Constraints {
            windows: windows.into_iter().collect(),
            app_minimums: app_minimums.into_iter().collect(),
            revision,
        }
    }

    /// One event; whether the records or the app minimums changed (`revision` moved).
    pub fn step(&mut self, event: ConstraintsEvent<W, A>) -> bool {
        let changed = match event {
            ConstraintsEvent::Observed {
                window,
                app,
                respect_min_size,
                observed,
            } => {
                let min = match (respect_min_size, observed.min) {
                    (true, Some(min)) => {
                        self.app_minimums.insert(app, min);
                        Some(min)
                    }
                    (true, None) => self.app_minimums.get(&app).copied(),
                    (false, min) => min,
                };
                let record = record(observed, respect_min_size, min, observed.width);
                self.windows.insert(window, record) != Some(record)
            }
            ConstraintsEvent::WidthSet { window, width } => match self.windows.get_mut(&window) {
                Some(record) if record.locked_width != width => {
                    record.locked_width = width;
                    true
                }
                Some(_) | None => false,
            },
            ConstraintsEvent::WidthPreserved { window, observed, width } => {
                let mut preserved = self
                    .windows
                    .get(&window)
                    .copied()
                    .unwrap_or_else(|| record(observed, false, observed.min, 0.0));
                preserved.locked_width = width;
                self.windows.insert(window, preserved) != Some(preserved)
            }
            ConstraintsEvent::Retired(window) => self.windows.remove(&window).is_some(),
            ConstraintsEvent::Replaced { from, to } => match self.windows.remove(&from) {
                Some(record) => {
                    self.windows.insert(to, record);
                    true
                }
                None => false,
            },
            // Not a window's record: a layout pass reads none of it.
            ConstraintsEvent::AppQuit(app) => {
                self.app_minimums.remove(&app);
                false
            }
        };
        self.revision += u64::from(changed);
        changed
    }

    /// `window`'s constraints, if it has a record.
    pub fn get(&self, window: W) -> Option<WindowConstraints> { self.windows.get(&window).copied() }

    /// Every window's constraints, in window order.
    pub fn windows(&self) -> impl Iterator<Item = (W, WindowConstraints)> + '_ {
        self.windows.iter().map(|(&window, &record)| (window, record))
    }

    /// Each app's last reported minimum, in app order.
    pub fn app_minimums(&self) -> impl Iterator<Item = (A, Size)> + '_ {
        self.app_minimums.iter().map(|(&app, &min)| (app, min))
    }

    /// How many changes the records have had: equal revisions, equal records.
    pub fn revision(&self) -> u64 { self.revision }
}

/// A window's record from what is observed of it, locked at `locked_width`.
fn record(
    observed: Observed,
    respect_min_size: bool,
    min: Option<Size>,
    locked_width: f64,
) -> WindowConstraints {
    WindowConstraints {
        sizing: Sizing::new(observed.resizable, respect_min_size, min),
        locked_width,
        locked_height: observed.height,
        max_width: observed.max_width,
        max_height: observed.max_height,
    }
    .normalized()
}

#[cfg(test)]
mod tests {
    use super::*;

    type Machine = Constraints<u32, i32>;

    fn observed(min: Option<Size>) -> Observed {
        Observed {
            resizable: true,
            width: 600.0,
            height: 400.0,
            min,
            max_width: 0.0,
            max_height: 0.0,
        }
    }

    fn observe(m: &mut Machine, window: u32, app: i32, respect: bool, min: Option<Size>) -> bool {
        m.step(ConstraintsEvent::Observed {
            window,
            app,
            respect_min_size: respect,
            observed: observed(min),
        })
    }

    #[test]
    fn an_observed_window_is_recorded_at_its_size_and_limits() {
        let mut m = Machine::default();
        assert!(observe(&mut m, 1, 7, false, Size::reported(200.0, 100.0)));
        let record = m.get(1).unwrap();
        assert_eq!((record.locked_width, record.locked_height), (600.0, 400.0));
        assert_eq!(record.sizing, Sizing::Resizable {
            advisory_min: Size::reported(200.0, 100.0).unwrap()
        });
        assert_eq!(m.revision(), 1);
        // The same observation again changes nothing.
        assert!(!observe(&mut m, 1, 7, false, Size::reported(200.0, 100.0)));
        assert_eq!(m.revision(), 1);
    }

    #[test]
    fn a_rule_window_reporting_no_minimum_takes_its_apps_last_one() {
        let mut m = Machine::default();
        let min = Size::reported(300.0, 200.0);
        observe(&mut m, 1, 7, true, min);
        observe(&mut m, 2, 7, true, None);
        assert_eq!(m.get(2).unwrap().sizing, Sizing::ResizableAtLeastMin {
            min: min.unwrap()
        });
        // Without respect_min_size the app's minimum is not taken.
        observe(&mut m, 3, 7, false, None);
        assert_eq!(m.get(3).unwrap().sizing, Sizing::Resizable {
            advisory_min: Size::ZERO
        });
        // Another app's window has no minimum to take.
        observe(&mut m, 4, 8, true, None);
        assert_eq!(m.get(4).unwrap().sizing, Sizing::ResizableAtLeastMin {
            min: Size::ZERO
        });
        // An app that quit leaves no minimum behind.
        assert!(!m.step(ConstraintsEvent::AppQuit(7)));
        observe(&mut m, 5, 7, true, None);
        assert_eq!(m.get(5).unwrap().sizing, Sizing::ResizableAtLeastMin {
            min: Size::ZERO
        });
    }

    #[test]
    fn a_width_set_locks_only_a_recorded_window() {
        let mut m = Machine::default();
        assert!(!m.step(ConstraintsEvent::WidthSet { window: 1, width: 500.0 }));
        assert_eq!(m.get(1), None);
        observe(&mut m, 1, 7, false, None);
        assert!(m.step(ConstraintsEvent::WidthSet { window: 1, width: 500.0 }));
        assert_eq!(m.get(1).unwrap().locked_width, 500.0);
        assert!(!m.step(ConstraintsEvent::WidthSet { window: 1, width: 500.0 }));
        assert_eq!(m.revision(), 2);
    }

    #[test]
    fn a_preserved_width_records_a_window_without_respect_min_size() {
        let mut m = Machine::default();
        let preserve = |m: &mut Machine, width| {
            m.step(ConstraintsEvent::WidthPreserved {
                window: 1,
                observed: observed(Size::reported(200.0, 100.0)),
                width,
            })
        };
        assert!(preserve(&mut m, 450.0));
        let record = m.get(1).unwrap();
        assert_eq!((record.locked_width, record.locked_height), (450.0, 400.0));
        assert!(matches!(record.sizing, Sizing::Resizable { .. }));
        // A recorded window keeps its record; only the width moves.
        observe(&mut m, 2, 7, true, Size::reported(300.0, 200.0));
        let before = m.get(2).unwrap();
        m.step(ConstraintsEvent::WidthPreserved {
            window: 2,
            observed: observed(None),
            width: 320.0,
        });
        assert_eq!(m.get(2).unwrap(), WindowConstraints {
            locked_width: 320.0,
            ..before
        });
        assert!(!preserve(&mut m, 450.0));
    }

    #[test]
    fn a_record_lives_as_long_as_its_window_and_follows_its_identity() {
        let mut m = Machine::default();
        observe(&mut m, 1, 7, false, None);
        let record = m.get(1).unwrap();
        assert!(m.step(ConstraintsEvent::Replaced { from: 1, to: 9 }));
        assert_eq!((m.get(1), m.get(9)), (None, Some(record)));
        assert!(!m.step(ConstraintsEvent::Replaced { from: 1, to: 10 }));
        assert!(m.step(ConstraintsEvent::Retired(9)));
        assert_eq!(m.get(9), None);
        assert!(!m.step(ConstraintsEvent::Retired(9)));
        assert_eq!(m.revision(), 3);
    }

    #[test]
    fn reported_limits_are_normalized() {
        let mut m = Machine::default();
        m.step(ConstraintsEvent::Observed {
            window: 1,
            app: 7,
            respect_min_size: true,
            observed: Observed {
                resizable: false,
                width: f64::NAN,
                height: -4.0,
                min: Size::reported(300.0, 0.0),
                max_width: 100.0,
                max_height: f64::INFINITY,
            },
        });
        let record = m.get(1).unwrap();
        assert_eq!(record.locked_width, 0.0);
        assert_eq!(record.locked_height, 0.0);
        // A maximum below the minimum is raised to it; an infinite one is no limit.
        assert_eq!((record.max_width, record.max_height), (300.0, 0.0));
        assert_eq!(record.sizing, Sizing::Fixed {
            min: Size::reported(300.0, 0.0).unwrap()
        });
    }
}
