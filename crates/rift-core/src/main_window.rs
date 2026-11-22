//! Which window is the main one: the frontmost app's, as the window server reports focus or,
//! before its first report, as AX activation and main-window events say (a cold start). The
//! reactor's `MainWindowTracker` steps this machine with what it observes; a step answers the
//! window to focus, if the event makes one known.

use std::collections::BTreeMap;

/// Who started an activation: Rift itself (its echo is not news), or anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Initiator {
    Rift,
    Other,
}

/// Whether an app is frontmost by its own AX activation events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Frontmost,
    Background,
}

/// What the window server has said about focus. Before its first answer, AX activation and
/// main-window events stand in for it; after it, they are metadata only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NativeFocus<P, W> {
    #[default]
    NotYetReported,
    /// The window it last reported focused (and its app), while that window exists.
    Reported(Option<(P, W)>),
}

/// One app as the machine knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct App<W> {
    pub standing: Standing,
    /// Who started the app's latest activation.
    pub last_activation: Initiator,
    pub main_window: Option<W>,
}

/// What the reactor observes that the machine reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainWindowEvent<P, W> {
    Launched {
        pid: P,
        standing: Standing,
        main_window: Option<W>,
    },
    /// The app's thread is gone.
    Terminated(P),
    Activated(P, Initiator),
    Deactivated(P),
    GloballyActivated(P),
    GloballyDeactivated(P),
    MainWindowChanged(P, Option<W>, Initiator),
    /// The window server reports `window` (of `pid`) focused.
    NativeFocused {
        pid: P,
        window: W,
    },
    /// `window` (of `pid`) is retired (no replacement) or has a new identity: the main window and
    /// the native focus follow it (upstream #702's `replace_window`).
    Replaced {
        pid: P,
        window: W,
        replacement: Option<W>,
    },
}

/// Every app's standing and main window, the globally frontmost app and the native focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainWindows<P, W> {
    apps: BTreeMap<P, App<W>>,
    global_frontmost: Option<P>,
    native_focus: NativeFocus<P, W>,
}

impl<P, W> Default for MainWindows<P, W> {
    fn default() -> Self {
        MainWindows {
            apps: BTreeMap::new(),
            global_frontmost: None,
            native_focus: NativeFocus::NotYetReported,
        }
    }
}

impl<P: Copy + Ord, W: Copy + PartialEq> MainWindows<P, W> {
    /// Applies `event`; the window to focus, if the event makes one known. Only a cold start
    /// answers one (before the window server's first focus report): an AX event of the
    /// globally frontmost app that Rift did not start names its main window.
    pub fn step(&mut self, event: MainWindowEvent<P, W>) -> Option<W> {
        let (pid, initiator) = match event {
            MainWindowEvent::Launched { pid, standing, main_window } => {
                self.apps.insert(pid, App {
                    standing,
                    last_activation: Initiator::Other,
                    main_window,
                });
                (pid, Initiator::Other)
            }
            MainWindowEvent::Terminated(pid) => {
                self.apps.remove(&pid);
                if let NativeFocus::Reported(Some((owner, _))) = self.native_focus
                    && owner == pid
                {
                    self.native_focus = NativeFocus::Reported(None);
                }
                return None;
            }
            MainWindowEvent::Activated(pid, initiator) => {
                let app = self.apps.get_mut(&pid)?;
                app.standing = Standing::Frontmost;
                app.last_activation = initiator;
                (pid, initiator)
            }
            MainWindowEvent::Deactivated(pid) => {
                self.apps.get_mut(&pid)?.standing = Standing::Background;
                return None;
            }
            MainWindowEvent::GloballyActivated(pid) => {
                self.global_frontmost = Some(pid);
                let app = self.apps.get_mut(&pid)?;
                app.standing = Standing::Frontmost;
                (pid, app.last_activation)
            }
            MainWindowEvent::GloballyDeactivated(pid) => {
                if self.global_frontmost == Some(pid) {
                    self.global_frontmost = None;
                }
                if let Some(app) = self.apps.get_mut(&pid) {
                    app.standing = Standing::Background;
                }
                return None;
            }
            MainWindowEvent::MainWindowChanged(pid, window, initiator) => {
                self.apps.get_mut(&pid)?.main_window = window;
                (pid, initiator)
            }
            MainWindowEvent::NativeFocused { pid, window } => {
                self.native_focus = NativeFocus::Reported(Some((pid, window)));
                return None;
            }
            MainWindowEvent::Replaced { pid, window, replacement } => {
                if self.native_focus == NativeFocus::Reported(Some((pid, window))) {
                    self.native_focus =
                        NativeFocus::Reported(replacement.map(|replacement| (pid, replacement)));
                }
                if let Some(app) = self.apps.get_mut(&pid)
                    && app.main_window == Some(window)
                {
                    app.main_window = replacement;
                }
                return None;
            }
        };
        // Once the window server has reported focus, AX activation and main-window events are
        // metadata and the cold-start fallback only: answering from them could replay the
        // previous native focus while the new one is still being resolved.
        match self.native_focus {
            NativeFocus::Reported(_) => None,
            NativeFocus::NotYetReported => (Some(pid) == self.global_frontmost
                && initiator == Initiator::Other)
                .then(|| self.main_window())
                .flatten(),
        }
    }

    /// The main window: the window server's focus if it belongs to the globally frontmost app,
    /// else that app's main window while it is frontmost.
    pub fn main_window(&self) -> Option<W> {
        let pid = self.global_frontmost?;
        if let NativeFocus::Reported(Some((owner, window))) = self.native_focus
            && owner == pid
        {
            return Some(window);
        }
        match self.apps.get(&pid) {
            Some(&App {
                standing: Standing::Frontmost,
                main_window: Some(window),
                ..
            }) => Some(window),
            _ => None,
        }
    }

    /// The window the window server last reported focused, while it exists.
    pub fn native_focus(&self) -> Option<W> {
        match self.native_focus {
            NativeFocus::NotYetReported | NativeFocus::Reported(None) => None,
            NativeFocus::Reported(Some((_, window))) => Some(window),
        }
    }

    pub fn is_globally_frontmost(&self, pid: P) -> bool { self.global_frontmost == Some(pid) }

    pub fn app_main_window(&self, pid: P) -> Option<W> { self.apps.get(&pid)?.main_window }

    /// Every app and its state, in app order.
    pub fn apps(&self) -> impl Iterator<Item = (P, App<W>)> + '_ {
        self.apps.iter().map(|(&pid, &app)| (pid, app))
    }

    pub fn global_frontmost(&self) -> Option<P> { self.global_frontmost }

    pub fn native_focus_state(&self) -> NativeFocus<P, W> { self.native_focus }
}

#[cfg(test)]
mod tests {
    use super::*;

    type M = MainWindows<u32, u64>;
    use MainWindowEvent as E;

    fn launched(m: &mut M, pid: u32, main: u64) -> Option<u64> {
        m.step(E::Launched {
            pid,
            standing: Standing::Frontmost,
            main_window: Some(main),
        })
    }

    #[test]
    fn a_cold_start_answers_the_frontmost_apps_main_window_unless_rift_started_it() {
        let mut m = M::default();
        assert_eq!(launched(&mut m, 1, 10), None, "not globally frontmost yet");
        assert_eq!(m.step(E::GloballyActivated(1)), Some(10));
        assert_eq!(m.step(E::MainWindowChanged(1, Some(11), Initiator::Rift)), None);
        assert_eq!(m.main_window(), Some(11));
        assert_eq!(
            m.step(E::MainWindowChanged(1, Some(12), Initiator::Other)),
            Some(12)
        );
        // An app's latest activation by Rift keeps a later global activation quiet.
        m.step(E::Activated(1, Initiator::Rift));
        assert_eq!(m.step(E::GloballyActivated(1)), None);
    }

    #[test]
    fn after_the_window_servers_first_report_ax_events_answer_nothing() {
        let mut m = M::default();
        launched(&mut m, 1, 10);
        m.step(E::GloballyActivated(1));
        m.step(E::NativeFocused { pid: 1, window: 13 });
        assert_eq!(m.step(E::MainWindowChanged(1, Some(12), Initiator::Other)), None);
        assert_eq!(
            m.main_window(),
            Some(13),
            "native focus outranks the AX main window"
        );
        // Native focus on another app's window is not the frontmost app's main window.
        m.step(E::NativeFocused { pid: 2, window: 20 });
        assert_eq!(m.main_window(), Some(12));
    }

    #[test]
    fn a_retired_or_rekeyed_window_takes_the_main_window_and_native_focus_with_it() {
        let mut m = M::default();
        launched(&mut m, 1, 10);
        m.step(E::GloballyActivated(1));
        m.step(E::NativeFocused { pid: 1, window: 10 });
        m.step(E::Replaced {
            pid: 1,
            window: 10,
            replacement: Some(14),
        });
        assert_eq!((m.native_focus(), m.app_main_window(1)), (Some(14), Some(14)));
        m.step(E::Replaced {
            pid: 1,
            window: 14,
            replacement: None,
        });
        assert_eq!((m.native_focus(), m.app_main_window(1)), (None, None));
        assert_eq!(
            m.native_focus_state(),
            NativeFocus::Reported(None),
            "still reported"
        );
        // A window that is neither is left alone.
        m.step(E::MainWindowChanged(1, Some(15), Initiator::Other));
        m.step(E::Replaced {
            pid: 1,
            window: 99,
            replacement: None,
        });
        assert_eq!(m.app_main_window(1), Some(15));
    }

    #[test]
    fn a_terminated_app_leaves_with_its_native_focus() {
        let mut m = M::default();
        launched(&mut m, 1, 10);
        m.step(E::NativeFocused { pid: 1, window: 10 });
        m.step(E::Terminated(1));
        assert_eq!(m.native_focus(), None);
        assert_eq!(m.apps().count(), 0);
        assert_eq!(m.step(E::Activated(1, Initiator::Other)), None, "unknown app");
    }

    #[test]
    fn globally_deactivating_another_app_keeps_the_frontmost_one() {
        let mut m = M::default();
        launched(&mut m, 1, 10);
        m.step(E::GloballyActivated(1));
        m.step(E::GloballyDeactivated(2));
        assert!(m.is_globally_frontmost(1));
        m.step(E::GloballyDeactivated(1));
        assert_eq!(m.global_frontmost(), None);
        assert_eq!(m.main_window(), None);
    }
}
