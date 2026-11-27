//! Which windows float, and what the layout engine remembers of them (engine slice E2): the
//! floating windows, a floating window's fullscreen, the floating windows active on each Space,
//! the last floating window focused, each workspace's floating frames, and the engine's focused
//! window. The layout engine steps this machine with what it decides and observes and reads it
//! back; it was the engine's `FloatingManager`, `FloatingPositionStore` and `focused_window`.

use std::collections::{BTreeMap, BTreeSet};

/// A window's app, for what an app quitting takes with it.
pub trait OfApp {
    type App;
    fn app(&self) -> Self::App;
}

impl OfApp for crate::plain::WindowId {
    type App = i32;

    fn app(&self) -> i32 { self.pid }
}

/// A window as a word (the Roc core's form): its pid in the high half, so its app is that half,
/// sign-extended as a pid word is.
impl OfApp for u64 {
    type App = u64;

    fn app(&self) -> u64 { (*self >> 32) as u32 as i32 as i64 as u64 }
}

/// A floating window shown fullscreen: over the whole screen, or within the gaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullscreenKind {
    Full,
    WithinGaps,
}

/// What the layout engine decides or observes that the machine reads.
#[derive(Debug, Clone, PartialEq)]
pub enum FloatsEvent<W, P, S, Ws, F> {
    /// `window` floats.
    Floated(W),
    /// `window` no longer floats: its fullscreen, its active entries and the floating focus on it
    /// go with it.
    Unfloated(W),
    /// A floating window's fullscreen, or none.
    Fullscreen(W, Option<FullscreenKind>),
    /// `window` is an active floating window on `space`.
    Activated { space: S, window: W },
    /// `window` is no longer active on `space`.
    Deactivated { space: S, window: W },
    /// `window` is active on no Space.
    DeactivatedEverywhere(W),
    /// `space`'s active floating windows are `windows`' floating ones.
    ActiveRebuilt { space: S, windows: Vec<W> },
    /// `from` is now `to`: its active floating windows go with it.
    SpaceRemapped { from: S, to: S },
    /// The last floating window focused, or none.
    FloatingFocus(Option<W>),
    /// The engine's focused window, or none.
    Focused(Option<W>),
    /// `from` has a new identity, `to`: floating, fullscreen, active entries, the floating focus
    /// and saved frames follow it, replacing whatever `to` had.
    Replaced { from: W, to: W },
    /// `app` quit: its windows float no longer, and their fullscreen, active entries, the floating
    /// focus on them and their saved frames go.
    AppQuit(P),
    /// The floating windows leave the machine (a restore takes them as candidates).
    FloatingTaken,
    /// `window`'s floating frame in `workspace`; `if_absent`, only if it has none there.
    FrameStored {
        workspace: Ws,
        window: W,
        frame: F,
        if_absent: bool,
    },
    /// `window`'s floating frames, in every workspace.
    FramesForgotten(W),
    /// `window`'s floating frame in `workspace`.
    FrameForgotten { workspace: Ws, window: W },
    /// Every floating frame in `workspace`.
    WorkspaceForgotten(Ws),
}

/// The floating state, all of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Floats<W, S, Ws, F> {
    floating: BTreeSet<W>,
    fullscreen: BTreeMap<W, FullscreenKind>,
    active: BTreeMap<S, BTreeSet<W>>,
    floating_focus: Option<W>,
    focused: Option<W>,
    frames: BTreeMap<(Ws, W), F>,
}

impl<W, S, Ws, F> Default for Floats<W, S, Ws, F> {
    fn default() -> Self {
        Floats {
            floating: BTreeSet::new(),
            fullscreen: BTreeMap::new(),
            active: BTreeMap::new(),
            floating_focus: None,
            focused: None,
            frames: BTreeMap::new(),
        }
    }
}

impl<W, S, Ws, F> Floats<W, S, Ws, F>
where
    W: Ord + Copy + OfApp,
    W::App: PartialEq,
    S: Ord + Copy,
    Ws: Ord + Copy,
    F: Copy,
{
    /// A machine holding these parts.
    pub fn from_parts(
        floating: impl IntoIterator<Item = W>,
        fullscreen: impl IntoIterator<Item = (W, FullscreenKind)>,
        active: impl IntoIterator<Item = (S, W)>,
        floating_focus: Option<W>,
        focused: Option<W>,
        frames: impl IntoIterator<Item = (Ws, W, F)>,
    ) -> Self {
        let mut spaces: BTreeMap<S, BTreeSet<W>> = BTreeMap::new();
        for (space, window) in active {
            spaces.entry(space).or_default().insert(window);
        }
        Floats {
            floating: floating.into_iter().collect(),
            fullscreen: fullscreen.into_iter().collect(),
            active: spaces,
            floating_focus,
            focused,
            frames: frames.into_iter().map(|(ws, w, f)| ((ws, w), f)).collect(),
        }
    }

    pub fn step(&mut self, event: FloatsEvent<W, W::App, S, Ws, F>) {
        match event {
            FloatsEvent::Floated(window) => {
                self.floating.insert(window);
            }
            FloatsEvent::Unfloated(window) => {
                self.floating.remove(&window);
                self.fullscreen.remove(&window);
                self.deactivate_everywhere(window);
                if self.floating_focus == Some(window) {
                    self.floating_focus = None;
                }
            }
            FloatsEvent::Fullscreen(window, Some(kind)) => {
                self.fullscreen.insert(window, kind);
            }
            FloatsEvent::Fullscreen(window, None) => {
                self.fullscreen.remove(&window);
            }
            FloatsEvent::Activated { space, window } => {
                self.active.entry(space).or_default().insert(window);
            }
            FloatsEvent::Deactivated { space, window } => {
                if let Some(windows) = self.active.get_mut(&space) {
                    windows.remove(&window);
                }
            }
            FloatsEvent::DeactivatedEverywhere(window) => self.deactivate_everywhere(window),
            FloatsEvent::ActiveRebuilt { space, windows } => {
                let floating = &self.floating;
                let active: BTreeSet<W> =
                    windows.into_iter().filter(|w| floating.contains(w)).collect();
                self.active.insert(space, active);
            }
            FloatsEvent::SpaceRemapped { from, to } => {
                if from != to {
                    let mut merged = self.active.remove(&to).unwrap_or_default();
                    if let Some(old) = self.active.remove(&from) {
                        merged.extend(old);
                    }
                    if !merged.is_empty() {
                        self.active.insert(to, merged);
                    }
                }
            }
            FloatsEvent::FloatingFocus(window) => self.floating_focus = window,
            FloatsEvent::Focused(window) => self.focused = window,
            FloatsEvent::Replaced { from, to } => {
                if from == to {
                    return;
                }
                let was_floating = self.floating.remove(&from);
                self.floating.remove(&to);
                if was_floating {
                    self.floating.insert(to);
                }
                let fullscreen = self.fullscreen.remove(&from);
                self.fullscreen.remove(&to);
                if let Some(kind) = fullscreen {
                    self.fullscreen.insert(to, kind);
                }
                let spaces: Vec<S> = self
                    .active
                    .iter()
                    .filter_map(|(space, windows)| windows.contains(&from).then_some(*space))
                    .collect();
                self.deactivate_everywhere(from);
                self.deactivate_everywhere(to);
                for space in spaces {
                    self.active.entry(space).or_default().insert(to);
                }
                if self.floating_focus == Some(from) {
                    self.floating_focus = Some(to);
                }
                let moved: Vec<(Ws, F)> = self
                    .frames
                    .iter()
                    .filter_map(|(&(ws, w), &f)| (w == from).then_some((ws, f)))
                    .collect();
                self.frames.retain(|&(_, w), _| w != from && w != to);
                for (ws, f) in moved {
                    self.frames.insert((ws, to), f);
                }
            }
            FloatsEvent::AppQuit(app) => {
                self.floating.retain(|w| w.app() != app);
                self.fullscreen.retain(|w, _| w.app() != app);
                for windows in self.active.values_mut() {
                    windows.retain(|w| w.app() != app);
                }
                if self.floating_focus.is_some_and(|w| w.app() == app) {
                    self.floating_focus = None;
                }
                self.frames.retain(|(_, w), _| w.app() != app);
            }
            FloatsEvent::FloatingTaken => self.floating.clear(),
            FloatsEvent::FrameStored {
                workspace,
                window,
                frame,
                if_absent,
            } => {
                if if_absent {
                    self.frames.entry((workspace, window)).or_insert(frame);
                } else {
                    self.frames.insert((workspace, window), frame);
                }
            }
            FloatsEvent::FramesForgotten(window) => self.frames.retain(|&(_, w), _| w != window),
            FloatsEvent::FrameForgotten { workspace, window } => {
                self.frames.remove(&(workspace, window));
            }
            FloatsEvent::WorkspaceForgotten(workspace) => {
                self.frames.retain(|&(ws, _), _| ws != workspace)
            }
        }
    }

    fn deactivate_everywhere(&mut self, window: W) {
        for windows in self.active.values_mut() {
            windows.remove(&window);
        }
    }

    pub fn is_floating(&self, window: W) -> bool { self.floating.contains(&window) }

    /// The floating windows, in window order.
    pub fn floating(&self) -> impl Iterator<Item = W> + '_ { self.floating.iter().copied() }

    pub fn fullscreen_kind(&self, window: W) -> Option<FullscreenKind> {
        self.fullscreen.get(&window).copied()
    }

    /// Every floating fullscreen, in window order.
    pub fn fullscreens(&self) -> impl Iterator<Item = (W, FullscreenKind)> + '_ {
        self.fullscreen.iter().map(|(&w, &k)| (w, k))
    }

    /// `space`'s active floating windows, in window order.
    pub fn active_windows(&self, space: S) -> impl Iterator<Item = W> + '_ {
        self.active.get(&space).into_iter().flatten().copied()
    }

    /// Every active floating window, by Space then window.
    pub fn active(&self) -> impl Iterator<Item = (S, W)> + '_ {
        self.active
            .iter()
            .flat_map(|(&s, windows)| windows.iter().map(move |&w| (s, w)))
    }

    pub fn is_active(&self, space: S, window: W) -> bool {
        self.active.get(&space).is_some_and(|windows| windows.contains(&window))
    }

    pub fn floating_focus(&self) -> Option<W> { self.floating_focus }

    pub fn focused(&self) -> Option<W> { self.focused }

    pub fn frame(&self, workspace: Ws, window: W) -> Option<F> {
        self.frames.get(&(workspace, window)).copied()
    }

    /// `workspace`'s floating frames, in window order.
    pub fn workspace_frames(&self, workspace: Ws) -> Vec<(W, F)> {
        self.frames
            .iter()
            .filter_map(|(&(ws, w), &f)| (ws == workspace).then_some((w, f)))
            .collect()
    }

    /// Every floating frame, by workspace then window.
    pub fn frames(&self) -> impl Iterator<Item = (Ws, W, F)> + '_ {
        self.frames.iter().map(|(&(ws, w), &f)| (ws, w, f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plain::{Rect, WindowId};

    type Machine = Floats<WindowId, u64, u64, Rect>;

    fn w(pid: i32, idx: u32) -> WindowId { WindowId { pid, idx } }

    fn frame(x: f64) -> Rect { Rect { x, y: 0.0, w: 100.0, h: 100.0 } }

    #[test]
    fn unfloating_takes_the_fullscreen_active_entries_and_floating_focus_with_it() {
        let mut m = Machine::default();
        let a = w(1, 1);
        m.step(FloatsEvent::Floated(a));
        m.step(FloatsEvent::Fullscreen(a, Some(FullscreenKind::Full)));
        m.step(FloatsEvent::Activated { space: 9, window: a });
        m.step(FloatsEvent::FloatingFocus(Some(a)));
        m.step(FloatsEvent::FrameStored {
            workspace: 3,
            window: a,
            frame: frame(1.0),
            if_absent: false,
        });
        m.step(FloatsEvent::Unfloated(a));
        assert!(!m.is_floating(a));
        assert_eq!(m.fullscreen_kind(a), None);
        assert!(!m.is_active(9, a));
        assert_eq!(m.floating_focus(), None);
        // Its frame stays: a frame belongs to the workspace, not to floating.
        assert_eq!(m.frame(3, a), Some(frame(1.0)));
    }

    #[test]
    fn a_rebuild_keeps_only_floating_windows_and_a_remap_merges_spaces() {
        let mut m = Machine::default();
        let (a, b, c) = (w(1, 1), w(1, 2), w(2, 1));
        m.step(FloatsEvent::Floated(a));
        m.step(FloatsEvent::Floated(c));
        m.step(FloatsEvent::ActiveRebuilt { space: 9, windows: vec![a, b] });
        assert_eq!(m.active_windows(9).collect::<Vec<_>>(), [a]);
        m.step(FloatsEvent::Activated { space: 8, window: c });
        m.step(FloatsEvent::SpaceRemapped { from: 8, to: 9 });
        assert_eq!(m.active_windows(9).collect::<Vec<_>>(), [a, c]);
        assert_eq!(m.active_windows(8).count(), 0);
        m.step(FloatsEvent::Deactivated { space: 9, window: a });
        assert_eq!(m.active_windows(9).collect::<Vec<_>>(), [c]);
    }

    #[test]
    fn a_new_identity_replaces_what_the_target_had() {
        let mut m = Machine::default();
        let (restored, live) = (w(1, 1), w(2, 2));
        m.step(FloatsEvent::Floated(live));
        m.step(FloatsEvent::Fullscreen(live, Some(FullscreenKind::Full)));
        m.step(FloatsEvent::Activated { space: 3, window: live });
        m.step(FloatsEvent::FrameStored {
            workspace: 1,
            window: live,
            frame: frame(5.0),
            if_absent: false,
        });
        m.step(FloatsEvent::FrameStored {
            workspace: 1,
            window: restored,
            frame: frame(7.0),
            if_absent: false,
        });
        m.step(FloatsEvent::Replaced { from: restored, to: live });
        assert!(!m.is_floating(live));
        assert_eq!(m.fullscreen_kind(live), None);
        assert!(!m.is_active(3, live));
        assert_eq!(m.frame(1, live), Some(frame(7.0)));
        assert_eq!(m.frame(1, restored), None);
        // A floating window's identity carries it all.
        let next = w(2, 3);
        m.step(FloatsEvent::Floated(live));
        m.step(FloatsEvent::FloatingFocus(Some(live)));
        m.step(FloatsEvent::Activated { space: 3, window: live });
        m.step(FloatsEvent::Replaced { from: live, to: next });
        assert!(m.is_floating(next) && m.is_active(3, next));
        assert_eq!(m.floating_focus(), Some(next));
    }

    #[test]
    fn an_app_quitting_takes_its_windows_floating_state_and_frames() {
        let mut m = Machine::default();
        let (a, b) = (w(1, 1), w(2, 1));
        for x in [a, b] {
            m.step(FloatsEvent::Floated(x));
            m.step(FloatsEvent::Fullscreen(x, Some(FullscreenKind::WithinGaps)));
            m.step(FloatsEvent::Activated { space: 4, window: x });
            m.step(FloatsEvent::FrameStored {
                workspace: 1,
                window: x,
                frame: frame(0.0),
                if_absent: false,
            });
        }
        m.step(FloatsEvent::FloatingFocus(Some(a)));
        m.step(FloatsEvent::AppQuit(1));
        assert!(!m.is_floating(a) && m.is_floating(b));
        assert_eq!(m.fullscreen_kind(a), None);
        assert_eq!(m.active_windows(4).collect::<Vec<_>>(), [b]);
        assert_eq!(m.floating_focus(), None);
        assert_eq!(m.frame(1, a), None);
        assert_eq!(m.frame(1, b), Some(frame(0.0)));
    }

    #[test]
    fn frames_are_stored_kept_if_present_and_forgotten() {
        let mut m = Machine::default();
        let a = w(1, 1);
        m.step(FloatsEvent::FrameStored {
            workspace: 1,
            window: a,
            frame: frame(1.0),
            if_absent: true,
        });
        m.step(FloatsEvent::FrameStored {
            workspace: 1,
            window: a,
            frame: frame(2.0),
            if_absent: true,
        });
        assert_eq!(m.frame(1, a), Some(frame(1.0)));
        m.step(FloatsEvent::FrameStored {
            workspace: 1,
            window: a,
            frame: frame(2.0),
            if_absent: false,
        });
        m.step(FloatsEvent::FrameStored {
            workspace: 2,
            window: a,
            frame: frame(3.0),
            if_absent: false,
        });
        assert_eq!(m.workspace_frames(1), [(a, frame(2.0))]);
        m.step(FloatsEvent::FrameForgotten { workspace: 1, window: a });
        assert_eq!(m.frame(1, a), None);
        m.step(FloatsEvent::FrameStored {
            workspace: 1,
            window: a,
            frame: frame(2.0),
            if_absent: false,
        });
        m.step(FloatsEvent::WorkspaceForgotten(1));
        assert_eq!((m.frame(1, a), m.frame(2, a)), (None, Some(frame(3.0))));
        m.step(FloatsEvent::FramesForgotten(a));
        assert_eq!(m.frames().count(), 0);
    }

    #[test]
    fn the_floating_windows_are_taken_and_focus_is_recorded() {
        let mut m = Machine::default();
        let a = w(1, 1);
        m.step(FloatsEvent::Floated(a));
        m.step(FloatsEvent::Fullscreen(a, Some(FullscreenKind::Full)));
        m.step(FloatsEvent::FloatingTaken);
        assert_eq!(m.floating().count(), 0);
        // Taking the floating windows leaves the rest.
        assert_eq!(m.fullscreen_kind(a), Some(FullscreenKind::Full));
        m.step(FloatsEvent::Focused(Some(a)));
        assert_eq!(m.focused(), Some(a));
        m.step(FloatsEvent::Focused(None));
        assert_eq!(m.focused(), None);
    }
}
