//! Where a display's departure moved windows from (engine slice E3b), so the display's return
//! can put each back at the same workspace position instead of the active one.
//!
//! A record holds the display the window was moved off, its workspace's position there, and the
//! workspace the departure gave it. It applies only while the window is still in that workspace
//! and only on the display it came from: placed anywhere else since, the user (or another change)
//! decided, and the record no longer applies. It was the engine's `evicted`.

use std::collections::BTreeMap;

use crate::floats::OfApp;

/// What a departure left of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Eviction<D, Ws> {
    /// The display it was moved off.
    pub display: D,
    /// Its workspace's position on that display.
    pub ordinal: u64,
    /// The workspace the departure gave it.
    pub landed: Ws,
}

/// What the layout engine decides or observes that the machine reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EvictionEvent<W, P, D, Ws> {
    /// A departure moved `window` off `from`, out of the workspace at `ordinal` there, into
    /// `landed`, on the display `landed_on` (none if that Space shows on no known display). A
    /// window that stayed on its display was not evicted, and leaves no record.
    Evicted {
        window: W,
        from: D,
        ordinal: u64,
        landed: Ws,
        landed_on: Option<D>,
    },
    /// `window`'s record goes: it returned, was placed elsewhere, or is gone.
    Forgotten(W),
    /// `from` has a new identity, `to`: its record follows it, replacing `to`'s.
    Replaced { from: W, to: W },
    /// `app` quit: its windows' records go.
    AppQuit(P),
}

/// Every evicted window's record.
#[derive(Debug, Clone, PartialEq)]
pub struct Evictions<W, D, Ws> {
    records: BTreeMap<W, Eviction<D, Ws>>,
}

impl<W, D, Ws> Default for Evictions<W, D, Ws> {
    fn default() -> Self { Evictions { records: BTreeMap::new() } }
}

impl<W, D, Ws> Evictions<W, D, Ws>
where
    W: Ord + Copy + OfApp,
    W::App: PartialEq,
    D: PartialEq + Clone,
    Ws: PartialEq + Copy,
{
    /// A machine holding `records`.
    pub fn from_parts(records: impl IntoIterator<Item = (W, Eviction<D, Ws>)>) -> Self {
        Evictions {
            records: records.into_iter().collect(),
        }
    }

    pub fn step(&mut self, event: EvictionEvent<W, W::App, D, Ws>) {
        match event {
            EvictionEvent::Evicted {
                window,
                from,
                ordinal,
                landed,
                landed_on,
            } => {
                if landed_on.as_ref() != Some(&from) {
                    self.records.insert(window, Eviction { display: from, ordinal, landed });
                }
            }
            EvictionEvent::Forgotten(window) => {
                self.records.remove(&window);
            }
            EvictionEvent::Replaced { from, to } => {
                if from != to {
                    self.records.remove(&to);
                    if let Some(record) = self.records.remove(&from) {
                        self.records.insert(to, record);
                    }
                }
            }
            EvictionEvent::AppQuit(app) => self.records.retain(|window, _| window.app() != app),
        }
    }

    /// The workspace position `window` was evicted from, if its record still applies: the window
    /// is still in the workspace the departure gave it (`assigned`), and the display now showing
    /// the Space it is reconciled to (`showing`) is the one it was moved off.
    pub fn ordinal(&self, window: W, assigned: Option<Ws>, showing: Option<&D>) -> Option<u64> {
        let record = self.records.get(&window)?;
        (assigned == Some(record.landed) && showing == Some(&record.display))
            .then_some(record.ordinal)
    }

    /// The windows whose record no longer applies, given where each is now (`assigned`): closed,
    /// rekeyed or placed somewhere else since.
    pub fn stale(&self, assigned: impl Fn(W) -> Option<Ws>) -> Vec<W> {
        (self.records.iter())
            .filter(|(window, record)| assigned(**window) != Some(record.landed))
            .map(|(window, _)| *window)
            .collect()
    }

    /// The windows a departure put into `workspace`.
    pub fn landed_in(&self, workspace: Ws) -> Vec<W> {
        (self.records.iter())
            .filter(|(_, record)| record.landed == workspace)
            .map(|(window, _)| *window)
            .collect()
    }

    /// `window`'s record, applying or not.
    pub fn get(&self, window: W) -> Option<&Eviction<D, Ws>> { self.records.get(&window) }

    /// Every record, in window order.
    pub fn records(&self) -> impl Iterator<Item = (W, &Eviction<D, Ws>)> + '_ {
        self.records.iter().map(|(window, record)| (*window, record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plain::WindowId;

    type Machine = Evictions<WindowId, &'static str, u32>;

    fn evict(
        m: &mut Machine,
        window: WindowId,
        ordinal: u64,
        landed: u32,
        on: Option<&'static str>,
    ) {
        m.step(EvictionEvent::Evicted {
            window,
            from: "a",
            ordinal,
            landed,
            landed_on: on,
        });
    }

    #[test]
    fn a_window_moved_off_its_display_is_recorded_and_one_that_stayed_is_not() {
        let mut m = Machine::default();
        let (w, v) = (WindowId { pid: 1, idx: 1 }, WindowId { pid: 1, idx: 2 });
        evict(&mut m, w, 2, 7, Some("b"));
        evict(&mut m, v, 1, 7, Some("a"));
        assert_eq!(
            m.get(w),
            Some(&Eviction {
                display: "a",
                ordinal: 2,
                landed: 7
            })
        );
        assert_eq!(m.get(v), None);
        // Landed on a Space with no known display, it was moved off.
        evict(&mut m, v, 1, 7, None);
        assert!(m.get(v).is_some());
    }

    #[test]
    fn a_record_applies_only_in_its_landing_and_on_its_display() {
        let mut m = Machine::default();
        let w = WindowId { pid: 1, idx: 1 };
        evict(&mut m, w, 2, 7, Some("b"));
        assert_eq!(m.ordinal(w, Some(7), Some(&"a")), Some(2));
        assert_eq!(m.ordinal(w, Some(8), Some(&"a")), None);
        assert_eq!(m.ordinal(w, None, Some(&"a")), None);
        assert_eq!(m.ordinal(w, Some(7), Some(&"b")), None);
        assert_eq!(m.ordinal(w, Some(7), None), None);
        assert_eq!(m.stale(|_| Some(7)), []);
        assert_eq!(m.stale(|_| Some(8)), [w]);
        assert_eq!(m.stale(|_| None), [w]);
        assert_eq!((m.landed_in(7), m.landed_in(8)), (vec![w], vec![]));
    }

    #[test]
    fn a_record_follows_its_windows_identity_and_goes_with_it_or_its_app() {
        let mut m = Machine::default();
        let (w, v, x) = (
            WindowId { pid: 1, idx: 1 },
            WindowId { pid: 1, idx: 2 },
            WindowId { pid: 2, idx: 1 },
        );
        evict(&mut m, w, 2, 7, None);
        evict(&mut m, v, 3, 8, None);
        m.step(EvictionEvent::Replaced { from: w, to: v });
        assert_eq!((m.get(w), m.get(v).map(|r| r.ordinal)), (None, Some(2)));
        m.step(EvictionEvent::Replaced { from: v, to: v });
        assert!(m.get(v).is_some());
        // A replaced identity with no record takes the target's away.
        m.step(EvictionEvent::Replaced { from: w, to: v });
        assert_eq!(m.get(v), None);
        evict(&mut m, v, 1, 7, None);
        evict(&mut m, x, 1, 7, None);
        m.step(EvictionEvent::AppQuit(1));
        assert_eq!((m.get(v), m.get(x).is_some()), (None, true));
        m.step(EvictionEvent::Forgotten(x));
        assert_eq!(m.records().count(), 0);
    }
}
