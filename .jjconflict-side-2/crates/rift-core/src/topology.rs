//! Decisions about Spaces and the windows on them, as pure functions of what the platform
//! observed. The platform asks macOS and acts; these decide. Each takes the platform's own
//! types (a Space, a window, a window-server id, a display) and asks for a fact only when the
//! decision needs it, so the platform's queries stay lazy.
//!
//! [`plain`] holds each decision as a question on plain data, which the Roc core is asked
//! through its typed entries (`roc/platform/Boundary.roc`).

/// Where an app inventory finds a window, by what the platform already knows of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// It has a native fullscreen record, settled or pending.
    RecoveringFullscreen,
    /// The platform tracks no window with its window-server id.
    Untracked,
    /// The platform tracks it and has no fullscreen record for it.
    Tracked,
}

/// The Space an app inventory takes a window to be on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveredSpace<S> {
    /// Live on exactly one Space, a fullscreen one or a display's: also evidence of a
    /// fullscreen entry or exit.
    Observed(S),
    /// Live on exactly one other Space.
    Live(S),
    /// Not live on exactly one Space: what the platform resolves for it, if anything.
    Fallback(Option<S>),
}

impl<S: Copy> DiscoveredSpace<S> {
    pub fn native(self) -> Option<S> {
        match self {
            DiscoveredSpace::Observed(space) | DiscoveredSpace::Live(space) => Some(space),
            DiscoveredSpace::Fallback(space) => space,
        }
    }

    pub fn observation(self) -> Option<S> {
        match self {
            DiscoveredSpace::Observed(space) => Some(space),
            DiscoveredSpace::Live(_) | DiscoveredSpace::Fallback(_) => None,
        }
    }
}

/// A tracked window resolves as usual. An untracked or recovering one is where it is live, if
/// that is exactly one Space: several memberships are transitional, even if one is a user
/// Space, and contradictory evidence must not complete a fullscreen exit. Failing that, an
/// untracked window falls back to its recorded Space and a recovering one to none.
pub fn discovered_space<S: Copy>(
    standing: Standing,
    live: impl FnOnce() -> Vec<S>,
    observable: impl Fn(S) -> bool,
    recorded: impl FnOnce() -> Option<S>,
    resolved: impl FnOnce() -> Option<S>,
) -> DiscoveredSpace<S> {
    let one_live = || match live().as_slice() {
        &[space] if observable(space) => Some(DiscoveredSpace::Observed(space)),
        &[space] => Some(DiscoveredSpace::Live(space)),
        _ => None,
    };
    match standing {
        Standing::Tracked => DiscoveredSpace::Fallback(resolved()),
        Standing::Untracked => one_live().unwrap_or_else(|| DiscoveredSpace::Fallback(recorded())),
        Standing::RecoveringFullscreen => one_live().unwrap_or(DiscoveredSpace::Fallback(None)),
    }
}

/// What an app inventory may do with a window it reports, for its discovery Space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryStanding {
    /// It has a native fullscreen record: its fullscreen Space is not one to admit it on.
    FullscreenRecorded,
    /// Only an assignment holds it: that supplies ownership but cannot admit an AX window.
    Placeholder,
    /// The platform holds its AX window and no fullscreen record.
    Admissible,
}

/// A reported window's authoritative Space (native, else assigned) and the Space discovery may
/// admit it on: only an admissible window's, preferring its assignment, then its native Space,
/// then where its frame is.
pub fn discovery_spaces<S: Copy>(
    standing: DiscoveryStanding,
    native: Option<S>,
    assigned: Option<S>,
    by_frame: impl FnOnce() -> Option<S>,
) -> (Option<S>, Option<S>) {
    let authoritative = native.or(assigned);
    match standing {
        DiscoveryStanding::FullscreenRecorded | DiscoveryStanding::Placeholder => {
            (authoritative, None)
        }
        DiscoveryStanding::Admissible => {
            (authoritative, assigned.or(authoritative).or_else(by_frame))
        }
    }
}

/// Whether native Mission Control is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionControl {
    Open,
    Closed,
}

/// The displays and Spaces of a snapshot, as admission reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct Displays<D, S> {
    /// Each screen's display, in screen order; `None` for a display with no id.
    pub screens: Vec<Option<D>>,
    /// Each display's Spaces, in the platform's order.
    pub spaces: Vec<(D, Vec<S>)>,
}

/// A snapshot being admitted: its displays and what changed since the last sample.
#[derive(Debug, Clone, PartialEq)]
pub struct Incoming<D, S> {
    pub displays: Displays<D, S>,
    /// Spaces macOS renumbered, old to new.
    pub remaps: Vec<(S, S)>,
    pub display_set_changed: bool,
    pub force_refresh: bool,
    /// Whether every window's Space membership was observed.
    pub membership_complete: bool,
}

/// Why a space snapshot waits instead of being committed. It is kept, with its deltas, for
/// the next admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// Native Mission Control is open.
    MissionControl,
    /// Spaces went missing and membership is incomplete: retry exhaustion is not evidence of
    /// ownership, so the logical model stays until membership is observable.
    MembershipIncomplete,
}

/// Whether a space snapshot is committed now, and what it invalidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission<S> {
    Hold(Hold),
    Commit {
        /// Spaces a display had that it no longer has (a remapped one is not lost), in the
        /// order of the current snapshot's displays and Spaces.
        invalidated: Vec<S>,
        /// Of those, the ones gone from every display while the same single display stayed:
        /// the native inventory also knows the owners of unvisited desktops.
        lost_local_desktops: Vec<S>,
    },
}

/// Admits `incoming` over the `current` displays. Only a display change or a forced refresh
/// invalidates Spaces; a current-Space switch alone does not invalidate ownership.
pub fn admit_space_snapshot<D: Copy + PartialEq, S: Copy + PartialEq>(
    current: &Displays<D, S>,
    incoming: &Incoming<D, S>,
    mission_control: MissionControl,
) -> Admission<S> {
    match mission_control {
        MissionControl::Open => return Admission::Hold(Hold::MissionControl),
        MissionControl::Closed => {}
    }
    let sole_display = match (current.screens.as_slice(), incoming.displays.screens.as_slice()) {
        ([previous], [now]) if previous == now => *now,
        _ => None,
    };
    let now_on = |display: D| {
        incoming
            .displays
            .spaces
            .iter()
            .find(|(id, _)| *id == display)
            .map(|(_, spaces)| spaces)
    };
    let mut invalidated = Vec::new();
    let mut lost_local_desktops = Vec::new();
    if incoming.display_set_changed || incoming.force_refresh {
        for (display, spaces) in &current.spaces {
            for &space in spaces {
                if now_on(*display).is_some_and(|ids| ids.contains(&space))
                    || incoming.remaps.iter().any(|(old, _)| *old == space)
                {
                    continue;
                }
                invalidated.push(space);
                if sole_display == Some(*display)
                    && !incoming.displays.spaces.iter().any(|(_, ids)| ids.contains(&space))
                {
                    lost_local_desktops.push(space);
                }
            }
        }
    }
    if !incoming.membership_complete && !invalidated.is_empty() {
        return Admission::Hold(Hold::MembershipIncomplete);
    }
    Admission::Commit {
        invalidated,
        lost_local_desktops,
    }
}

/// The windows whose app rules an activation re-applies, by app in the order their first
/// window is seen: the tracked windows live on a Space that just became active whose admission
/// can still be reconciled.
pub fn app_rule_candidates<V, S: PartialEq, W: Copy, P: Copy + PartialEq>(
    window_spaces: impl IntoIterator<Item = (V, S)>,
    activated: &[S],
    tracked: impl Fn(V) -> Option<W>,
    reconcilable: impl Fn(W) -> bool,
    app: impl Fn(W) -> P,
) -> Vec<(P, Vec<W>)> {
    let mut by_app: Vec<(P, Vec<W>)> = Vec::new();
    for window in window_spaces
        .into_iter()
        .filter(|(_, space)| activated.contains(space))
        .filter_map(|(server, _)| tracked(server))
        .filter(|&window| reconcilable(window))
    {
        match by_app.iter_mut().find(|(pid, _)| *pid == app(window)) {
            Some((_, windows)) => windows.push(window),
            None => by_app.push((app(window), vec![window])),
        }
    }
    by_app
}

/// Each decision as a question on plain data. Its arguments are plain values; every fact it
/// asks for is a [`Fact`] (or, asked about a key, [`Answers`]): what the platform answered, or
/// that the platform's run of the decision never asked it. Each question names its facts in
/// `FACTS`, in the order the wire carries them; an answer that read a fact the question does not
/// carry is [`Unasked`]. Spaces, window-server ids and displays are one word each.
pub mod plain {
    use super::{
        Admission, DiscoveredSpace, DiscoveryStanding, Displays, Incoming, MissionControl, Standing,
    };
    use crate::plain::{Answers, Fact, Reading, Unasked, WindowId};

    /// [`super::discovered_space`]: the window's Spaces, whether a Space is fullscreen or a
    /// display's, its recorded Space and what the platform resolves for it.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Discovered {
        pub standing: Standing,
        pub live: Fact<Vec<u64>>,
        pub observable: Answers<u64, bool>,
        pub recorded: Fact<Option<u64>>,
        pub resolved: Fact<Option<u64>>,
    }

    impl Discovered {
        pub const FACTS: &[&str] = &["live", "observable", "recorded", "resolved"];

        pub fn answer(&self) -> Result<DiscoveredSpace<u64>, Unasked> {
            let reading = Reading::new();
            let answer = super::discovered_space(
                self.standing,
                || reading.fact(&self.live, 0, Vec::new()),
                |space| reading.about(&self.observable, &space, 1, false),
                || reading.fact(&self.recorded, 2, None),
                || reading.fact(&self.resolved, 3, None),
            );
            reading.answer(answer)
        }
    }

    /// [`super::discovery_spaces`]: where the window's frame is.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Discovery {
        pub standing: DiscoveryStanding,
        pub native: Option<u64>,
        pub assigned: Option<u64>,
        pub by_frame: Fact<Option<u64>>,
    }

    impl Discovery {
        pub const FACTS: &[&str] = &["by_frame"];

        pub fn answer(&self) -> Result<(Option<u64>, Option<u64>), Unasked> {
            let reading = Reading::new();
            let answer = super::discovery_spaces(self.standing, self.native, self.assigned, || {
                reading.fact(&self.by_frame, 0, None)
            });
            reading.answer(answer)
        }
    }

    /// [`super::admit_space_snapshot`]: it asks for no fact.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Admit {
        pub mission_control: MissionControl,
        pub current: Displays<u64, u64>,
        pub incoming: Incoming<u64, u64>,
    }

    impl Admit {
        pub const FACTS: &[&str] = &[];

        pub fn answer(&self) -> Admission<u64> {
            super::admit_space_snapshot(&self.current, &self.incoming, self.mission_control)
        }
    }

    /// [`super::app_rule_candidates`]: the window a window-server id is tracked as (if any),
    /// and whether a window's admission can still be reconciled; a window's app is its pid.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Candidates {
        pub window_spaces: Vec<(u64, u64)>,
        pub activated: Vec<u64>,
        pub tracked: Answers<u64, Option<WindowId>>,
        pub reconcilable: Answers<WindowId, bool>,
    }

    impl Candidates {
        pub const FACTS: &[&str] = &["tracked", "reconcilable"];

        pub fn answer(&self) -> Result<Vec<(i32, Vec<WindowId>)>, Unasked> {
            let reading = Reading::new();
            let answer = super::app_rule_candidates(
                self.window_spaces.iter().copied(),
                &self.activated,
                |server| reading.about(&self.tracked, &server, 0, None),
                |window| reading.about(&self.reconcilable, &window, 1, false),
                |window| window.pid,
            );
            reading.answer(answer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plain::WindowId;

    #[test]
    fn a_discovered_windows_native_space_is_its_one_live_space_or_a_fallback_by_its_standing() {
        let (display, full, other, recorded) = (1_u64, 9, 5, 7);
        let observable = |space| space == display || space == full;
        let decide = |standing, live: Vec<u64>| {
            discovered_space(
                standing,
                || live,
                observable,
                || Some(recorded),
                || panic!("only a tracked window is resolved"),
            )
        };
        use DiscoveredSpace::{Fallback, Live, Observed};
        use Standing::{RecoveringFullscreen, Tracked, Untracked};
        // A tracked window resolves as usual; its live Spaces are not asked.
        assert_eq!(
            discovered_space(
                Tracked,
                || panic!("a tracked window's live Spaces are not asked"),
                observable,
                || panic!("nor its recorded Space"),
                || Some(other),
            ),
            Fallback(Some(other))
        );
        for standing in [Untracked, RecoveringFullscreen] {
            // One live Space that is fullscreen or a display's is also an observation.
            assert_eq!(decide(standing, vec![display]), Observed(display));
            assert_eq!(decide(standing, vec![full]), Observed(full));
            assert_eq!(decide(standing, vec![other]), Live(other));
        }
        // Several memberships are transitional: an untracked window falls back to its recorded
        // Space, a recovering one to none (contradictory evidence is no completed exit).
        assert_eq!(decide(Untracked, vec![display, full]), Fallback(Some(recorded)));
        assert_eq!(decide(Untracked, vec![]), Fallback(Some(recorded)));
        let recovering = |live: Vec<u64>| {
            discovered_space(
                RecoveringFullscreen,
                || live,
                observable,
                || panic!("a recovering window's recorded Space is not asked"),
                || panic!("nor is it resolved"),
            )
        };
        assert_eq!(recovering(vec![display, full]), Fallback(None));
        assert_eq!(recovering(vec![]), Fallback(None));
        // What the inventory uses: the native Space, and the observation if there is one.
        assert_eq!(
            (Observed(full).native(), Observed(full).observation()),
            (Some(full), Some(full))
        );
        assert_eq!(
            (Live(other).native(), Live(other).observation()),
            (Some(other), None)
        );
        assert_eq!(
            (
                Fallback::<u64>(None).native(),
                Fallback::<u64>(None).observation()
            ),
            (None, None)
        );
    }

    #[test]
    fn only_an_admissible_window_gets_a_discovery_space_and_it_prefers_the_assignment() {
        let (native, assigned, framed) = (1_u64, 2, 3);
        use DiscoveryStanding::{Admissible, FullscreenRecorded, Placeholder};
        let no_frame = || panic!("the frame is asked only when nothing else answers");
        // Authoritative is the native Space, else the assignment, whatever the standing.
        for standing in [FullscreenRecorded, Placeholder] {
            assert_eq!(
                discovery_spaces(standing, Some(native), Some(assigned), no_frame),
                (Some(native), None)
            );
            assert_eq!(
                discovery_spaces(standing, None, Some(assigned), no_frame),
                (Some(assigned), None)
            );
        }
        // Discovery prefers the assignment, then the native Space, then the frame.
        assert_eq!(
            discovery_spaces(Admissible, Some(native), Some(assigned), no_frame),
            (Some(native), Some(assigned))
        );
        assert_eq!(
            discovery_spaces(Admissible, Some(native), None, no_frame),
            (Some(native), Some(native))
        );
        assert_eq!(
            discovery_spaces(Admissible, None, None, || Some(framed)),
            (None, Some(framed))
        );
        assert_eq!(
            discovery_spaces::<u64>(Admissible, None, None, || None),
            (None, None)
        );
    }

    #[test]
    fn a_space_snapshot_is_held_or_committed_with_the_spaces_it_invalidates() {
        let displays =
            |screens: Vec<Option<&'static str>>, spaces: &[(&'static str, &[u64])]| Displays {
                screens,
                spaces: spaces.iter().map(|(display, ids)| (*display, ids.to_vec())).collect(),
            };
        let incoming = |displays| Incoming {
            displays,
            remaps: vec![],
            display_set_changed: false,
            force_refresh: false,
            membership_complete: true,
        };
        let main = Some("main");
        let current = displays(vec![main], &[("main", &[1, 2, 3])]);
        let commit = |invalidated: &[u64], lost: &[u64]| Admission::Commit {
            invalidated: invalidated.to_vec(),
            lost_local_desktops: lost.to_vec(),
        };
        let admit = |incoming: &Incoming<&str, u64>| {
            admit_space_snapshot(&current, incoming, MissionControl::Closed)
        };

        // Without a display change or a forced refresh, nothing is invalidated.
        let quiet = incoming(displays(vec![main], &[("main", &[1])]));
        assert_eq!(admit(&quiet), commit(&[], &[]));
        // Mission Control holds any snapshot.
        assert_eq!(
            admit_space_snapshot(&current, &quiet, MissionControl::Open),
            Admission::Hold(Hold::MissionControl)
        );

        // Desktop 2 is gone from the sole display (3 was remapped, so it is not): invalidated,
        // and lost, since no display has it.
        let mut closed = incoming(displays(vec![main], &[("main", &[1])]));
        closed.display_set_changed = true;
        closed.remaps = vec![(3, 9)];
        assert_eq!(admit(&closed), commit(&[2], &[2]));
        closed.force_refresh = true;
        closed.display_set_changed = false;
        assert_eq!(admit(&closed), commit(&[2], &[2]));
        // Not while membership is incomplete: retry exhaustion is no evidence of ownership.
        closed.membership_complete = false;
        assert_eq!(admit(&closed), Admission::Hold(Hold::MembershipIncomplete));

        // On another display now: invalidated here, but not lost.
        let mut moved = incoming(displays(vec![main], &[("main", &[1, 3]), ("other", &[2])]));
        moved.display_set_changed = true;
        assert_eq!(admit(&moved), commit(&[2], &[]));

        // A display that is not the sole one, before and after, loses no local desktops; nor
        // does a display with no id.
        for screen in [Some("another"), None] {
            let mut renamed = incoming(displays(vec![screen], &[("main", &[1])]));
            renamed.display_set_changed = true;
            assert_eq!(admit(&renamed), commit(&[2, 3], &[]));
        }
    }

    #[test]
    fn an_activated_space_reapplies_app_rules_to_its_admissible_windows_by_app() {
        let (activated, other) = (1_u64, 2);
        let window = |pid, idx| WindowId { pid, idx };
        let (a1, a2, b1, settled, elsewhere) = (
            window(10, 1),
            window(10, 2),
            window(20, 1),
            window(20, 5),
            window(30, 1),
        );
        let server = |window: WindowId| window.pid as u64 * 100 + u64::from(window.idx);
        let windows = [a1, b1, a2, settled, elsewhere];
        let on_spaces: Vec<(u64, u64)> = windows
            .iter()
            .map(|&window| {
                (
                    server(window),
                    if window == elsewhere {
                        other
                    } else {
                        activated
                    },
                )
            })
            .chain([(4, activated)])
            .collect();
        let tracked = |id: u64| windows.iter().copied().find(|&w| server(w) == id);
        let candidates = |activated: &[u64]| {
            app_rule_candidates(
                on_spaces.iter().copied(),
                activated,
                tracked,
                |window| window != settled,
                |window| window.pid,
            )
        };
        // Untracked, not admissible, or on a Space that did not activate: no candidate. Apps
        // come in the order their first window is seen.
        assert_eq!(candidates(&[activated]), vec![
            (10, vec![a1, a2]),
            (20, vec![b1])
        ]);
        assert!(candidates(&[]).is_empty());
    }
}
