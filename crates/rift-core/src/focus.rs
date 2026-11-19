//! Decisions about focus and raising, as pure functions of what the platform observed. The
//! platform asks and acts (making a window key, warping, refocusing); these decide, asking for
//! a fact only when the decision needs it.

/// A window or an app leaving the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal<W, P> {
    AppClosed(P),
    WindowRemoved(W),
}

/// What a removal does to focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalFocus<W, S> {
    /// It did not take the focused window.
    Unaffected,
    /// macOS already moved focus to this sibling of the removed window (a native tab taking
    /// over): leave it there; raising a fallback would steal focus from it.
    KeepNative(W),
    /// Focus is gone from this Space: refocus there.
    Refocus(S),
}

/// What `removal` does to focus, given the focused window (the layout's, or the active
/// workspace's last focused when AX suspension already cleared it). The removed window's Space
/// is asked only when it held focus; macOS's key window only for a window removal; the
/// successor's identity and workspace only for a sibling macOS focused.
pub fn removal_focus<W: Copy + PartialEq, S: Copy, P: Copy + PartialEq>(
    removal: Removal<W, P>,
    focused: Option<W>,
    app: impl Fn(W) -> P,
    removed_space: impl FnOnce(W) -> Option<S>,
    native_focus: impl FnOnce(S) -> Option<W>,
    successor: impl FnOnce(W) -> W,
    in_inactive_workspace: impl FnOnce(S, W) -> bool,
) -> RemovalFocus<W, S> {
    let removed = match removal {
        Removal::AppClosed(pid) => focused.filter(|&window| app(window) == pid),
        Removal::WindowRemoved(window) => focused.filter(|&focused| focused == window),
    };
    let Some(window) = removed else {
        return RemovalFocus::Unaffected;
    };
    let Some(space) = removed_space(window) else {
        return RemovalFocus::Unaffected;
    };
    match removal {
        Removal::AppClosed(_) => RemovalFocus::Refocus(space),
        Removal::WindowRemoved(_) => {
            let sibling = native_focus(space)
                .filter(|&native| app(native) == app(window) && native != window)
                .map(successor)
                .filter(|&successor| {
                    successor != window && !in_inactive_workspace(space, successor)
                });
            match sibling {
                Some(successor) => RemovalFocus::KeepNative(successor),
                None => RemovalFocus::Refocus(space),
            }
        }
    }
}

/// The window to focus in a Space's active workspace: the preferred one, else the last
/// focused, else the first of the workspace's windows, whichever is visible there first. The
/// history is asked only without a visible preferred window, the workspace only after that.
pub fn focus_candidate<W: Copy>(
    preferred: Option<W>,
    last_focused: impl FnOnce() -> Option<W>,
    in_workspace: impl FnOnce() -> Vec<W>,
    visible: impl Fn(W) -> bool,
) -> Option<W> {
    preferred
        .filter(|&window| visible(window))
        .or_else(|| last_focused().filter(|&window| visible(window)))
        .or_else(|| in_workspace().into_iter().find(|&window| visible(window)))
}

/// Whether focus is hidden: `window`, the main or the focused window, is in a workspace that is
/// not its Space's active one (asked only then). Only hidden focus needs a replacement: a
/// membership refresh also sees unfocused windows on inactive workspaces.
pub fn hides_focus<W: Copy + PartialEq>(
    window: W,
    main: Option<W>,
    focused: Option<W>,
    in_inactive_workspace: impl FnOnce() -> bool,
) -> bool {
    (main == Some(window) || focused == Some(window)) && in_inactive_workspace()
}

/// What the platform knows of an activated window, asked in this order and no further than an
/// activation follow needs.
pub trait ActivationFacts {
    type Space: Copy;
    type Workspace: Copy + PartialEq;
    /// An input mode that refuses to follow activations.
    type Mode;
    /// A window-server id.
    type Server;

    /// A standard window (not a panel, sheet, ...).
    fn standard(&self) -> bool;
    /// Temporarily unavailable (hidden, minimized, on its way somewhere).
    fn unavailable(&self) -> bool;
    fn space(&self) -> Option<Self::Space>;
    fn workspace(&self, space: Self::Space) -> Option<Self::Workspace>;
    fn active_workspace(&self, space: Self::Space) -> Option<Self::Workspace>;
    /// The app has a window macOS reported that AX has not admitted yet.
    fn pending_window(&self) -> bool;
    fn mode_refusal(&self) -> Option<Self::Mode>;
    /// The active Space is a fullscreen one.
    fn active_space_fullscreen(&self) -> bool;
    /// The window the activation came from, if it is not manageable.
    fn unmanageable_source(&self) -> Option<Self::Server>;
    /// Whether the app is blacklisted for auto-focus; `None` without a bundle id.
    fn blacklisted(&self) -> Option<bool>;
}

/// Why an activation does not follow its window to its workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stay<M, V> {
    NotStandard,
    Unavailable,
    NoSpace,
    NoWorkspace,
    /// Its workspace is the active one (or its Space has none).
    AlreadyActive,
    /// A newly appearing native window can leave focus on the app's old window until AX
    /// admission completes.
    PendingWindow,
    Mode(M),
    FullscreenActive,
    Unmanageable(V),
    NoBundleId,
    Blacklisted,
}

/// Whether an activation of a window follows it to its workspace, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow<S, K, M, V> {
    To(S, K),
    Stay(Stay<M, V>),
}

/// Follows an activated window to its workspace when that is not its Space's active one and
/// nothing says not to.
pub fn activation_follow<F: ActivationFacts>(
    facts: &F,
) -> Follow<F::Space, F::Workspace, F::Mode, F::Server> {
    let stay = |why| Follow::Stay(why);
    if !facts.standard() {
        return stay(Stay::NotStandard);
    }
    if facts.unavailable() {
        return stay(Stay::Unavailable);
    }
    let Some(space) = facts.space() else {
        return stay(Stay::NoSpace);
    };
    let Some(workspace) = facts.workspace(space) else {
        return stay(Stay::NoWorkspace);
    };
    if facts.active_workspace(space).is_none_or(|active| active == workspace) {
        return stay(Stay::AlreadyActive);
    }
    if facts.pending_window() {
        return stay(Stay::PendingWindow);
    }
    if let Some(mode) = facts.mode_refusal() {
        return stay(Stay::Mode(mode));
    }
    if facts.active_space_fullscreen() {
        return stay(Stay::FullscreenActive);
    }
    if let Some(server) = facts.unmanageable_source() {
        return stay(Stay::Unmanageable(server));
    }
    match facts.blacklisted() {
        None => stay(Stay::NoBundleId),
        Some(true) => stay(Stay::Blacklisted),
        Some(false) => Follow::To(space, workspace),
    }
}

/// The display to activate after a command moved the focused window: its assigned Space, if
/// that changed, is active and is on a known display. macOS moves its active display with the
/// key window only later; until then the next command would act on the old display.
pub fn display_to_follow<S: Copy + PartialEq>(
    focused_space: Option<S>,
    previous: Option<S>,
    active: impl Fn(S) -> bool,
    on_known_display: impl Fn(S) -> bool,
) -> Option<S> {
    focused_space
        .filter(|&space| Some(space) != previous && active(space))
        .filter(|&space| on_known_display(space))
}

/// What to do about the window under the cursor when the platform does not track it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UntrackedFocus<P, V> {
    /// Nothing there, it is tracked, or nothing is known of it.
    Nothing,
    /// A system surface (desktop, menu bar, Dock: a nonzero layer), never made key merely
    /// because the pointer crossed it.
    SystemSurface { layer: i32, window: V },
    /// An ordinary application window outside rift's model: make it key.
    MakeKey(P, V),
}

/// Whether to make the untracked window under the cursor key: only an application window
/// (layer 0). Its tracking is asked only if there is one, what is known of it (its layer and
/// app) only if untracked.
pub fn untracked_focus<P, V: Copy>(
    under_cursor: Option<V>,
    tracked: impl FnOnce(V) -> bool,
    known: impl FnOnce(V) -> Option<(i32, P)>,
) -> UntrackedFocus<P, V> {
    let Some(window) = under_cursor.filter(|&window| !tracked(window)) else {
        return UntrackedFocus::Nothing;
    };
    match known(window) {
        None => UntrackedFocus::Nothing,
        Some((0, app)) => UntrackedFocus::MakeKey(app, window),
        Some((layer, _)) => UntrackedFocus::SystemSurface { layer, window },
    }
}

/// Whether to focus a Space's desktop: it is active and its active workspace is empty (asked
/// only then).
pub fn focuses_desktop(active: bool, workspace_empty: impl FnOnce() -> bool) -> bool {
    active && workspace_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window is (app, index).
    type W = (u32, u32);

    #[test]
    fn focus_goes_to_the_preferred_then_the_last_focused_then_the_first_visible_window() {
        let visible = |window: u32| window % 2 == 0;
        let none = || None;
        let nothing = Vec::new;
        assert_eq!(
            focus_candidate(Some(2), || panic!("not asked"), || panic!("not asked"), visible),
            Some(2)
        );
        assert_eq!(
            focus_candidate(Some(1), || Some(4), || panic!("not asked"), visible),
            Some(4)
        );
        assert_eq!(
            focus_candidate(Some(1), || Some(3), || vec![5, 6, 8], visible),
            Some(6)
        );
        assert_eq!(focus_candidate(None, none, || vec![1, 3], visible), None);
        assert_eq!(focus_candidate(None, none, nothing, visible), None);
    }

    #[test]
    fn hidden_focus_is_the_main_or_focused_window_in_an_inactive_workspace() {
        assert!(hides_focus(1, Some(1), None, || true));
        assert!(hides_focus(1, None, Some(1), || true));
        assert!(!hides_focus(1, Some(1), Some(1), || false));
        assert!(!hides_focus(1, Some(2), Some(3), || panic!("not asked")));
    }

    /// Facts as data; a fact left `None` panics if asked.
    #[derive(Clone, Copy)]
    struct Facts {
        standard: bool,
        unavailable: bool,
        space: Option<Option<u64>>,
        workspace: Option<Option<u32>>,
        active: Option<Option<u32>>,
        pending: Option<bool>,
        mode: Option<Option<&'static str>>,
        fullscreen: Option<bool>,
        unmanageable: Option<Option<u32>>,
        blacklisted: Option<Option<bool>>,
    }

    impl ActivationFacts for Facts {
        type Mode = &'static str;
        type Server = u32;
        type Space = u64;
        type Workspace = u32;

        fn standard(&self) -> bool { self.standard }

        fn unavailable(&self) -> bool { self.unavailable }

        fn space(&self) -> Option<u64> { self.space.expect("space asked") }

        fn workspace(&self, _: u64) -> Option<u32> { self.workspace.expect("workspace asked") }

        fn active_workspace(&self, _: u64) -> Option<u32> { self.active.expect("active asked") }

        fn pending_window(&self) -> bool { self.pending.expect("pending asked") }

        fn mode_refusal(&self) -> Option<&'static str> { self.mode.expect("mode asked") }

        fn active_space_fullscreen(&self) -> bool { self.fullscreen.expect("fullscreen asked") }

        fn unmanageable_source(&self) -> Option<u32> { self.unmanageable.expect("source asked") }

        fn blacklisted(&self) -> Option<bool> { self.blacklisted.expect("blacklist asked") }
    }

    #[test]
    fn an_activation_follows_its_window_to_another_workspace_unless_something_says_not_to() {
        let follows = Facts {
            standard: true,
            unavailable: false,
            space: Some(Some(1)),
            workspace: Some(Some(7)),
            active: Some(Some(3)),
            pending: Some(false),
            mode: Some(None),
            fullscreen: Some(false),
            unmanageable: Some(None),
            blacklisted: Some(Some(false)),
        };
        assert_eq!(activation_follow(&follows), Follow::To(1, 7));
        let unasked = |facts: Facts| Facts {
            space: None,
            workspace: None,
            active: None,
            pending: None,
            mode: None,
            fullscreen: None,
            unmanageable: None,
            blacklisted: None,
            ..facts
        };
        // Each reason stops it, and nothing after it is asked.
        assert_eq!(
            activation_follow(&unasked(Facts { standard: false, ..follows })),
            Follow::Stay(Stay::NotStandard)
        );
        assert_eq!(
            activation_follow(&unasked(Facts { unavailable: true, ..follows })),
            Follow::Stay(Stay::Unavailable)
        );
        let cases: [(Facts, Stay<&str, u32>); 9] = [
            (
                Facts {
                    space: Some(None),
                    workspace: None,
                    active: None,
                    pending: None,
                    mode: None,
                    fullscreen: None,
                    unmanageable: None,
                    blacklisted: None,
                    ..follows
                },
                Stay::NoSpace,
            ),
            (
                Facts {
                    workspace: Some(None),
                    active: None,
                    pending: None,
                    mode: None,
                    fullscreen: None,
                    unmanageable: None,
                    blacklisted: None,
                    ..follows
                },
                Stay::NoWorkspace,
            ),
            (
                Facts {
                    active: Some(Some(7)),
                    pending: None,
                    mode: None,
                    fullscreen: None,
                    unmanageable: None,
                    blacklisted: None,
                    ..follows
                },
                Stay::AlreadyActive,
            ),
            (
                Facts {
                    active: Some(None),
                    pending: None,
                    mode: None,
                    fullscreen: None,
                    unmanageable: None,
                    blacklisted: None,
                    ..follows
                },
                Stay::AlreadyActive,
            ),
            (
                Facts {
                    pending: Some(true),
                    mode: None,
                    fullscreen: None,
                    unmanageable: None,
                    blacklisted: None,
                    ..follows
                },
                Stay::PendingWindow,
            ),
            (
                Facts {
                    mode: Some(Some("drag")),
                    fullscreen: None,
                    unmanageable: None,
                    blacklisted: None,
                    ..follows
                },
                Stay::Mode("drag"),
            ),
            (
                Facts {
                    fullscreen: Some(true),
                    unmanageable: None,
                    blacklisted: None,
                    ..follows
                },
                Stay::FullscreenActive,
            ),
            (
                Facts {
                    unmanageable: Some(Some(42)),
                    blacklisted: None,
                    ..follows
                },
                Stay::Unmanageable(42),
            ),
            (
                Facts {
                    blacklisted: Some(None),
                    ..follows
                },
                Stay::NoBundleId,
            ),
        ];
        for (facts, why) in cases {
            assert_eq!(activation_follow(&facts), Follow::Stay(why));
        }
        assert_eq!(
            activation_follow(&Facts {
                blacklisted: Some(Some(true)),
                ..follows
            }),
            Follow::Stay(Stay::Blacklisted)
        );
    }

    #[test]
    fn the_display_follows_a_focused_window_to_another_active_known_display() {
        let active = |space: u64| space != 3;
        let known = |space: u64| space != 4;
        assert_eq!(display_to_follow(Some(2), Some(1), active, known), Some(2));
        assert_eq!(display_to_follow(Some(2), None, active, known), Some(2));
        assert_eq!(display_to_follow(Some(1), Some(1), active, known), None);
        assert_eq!(display_to_follow(Some(3), Some(1), active, known), None);
        assert_eq!(display_to_follow(Some(4), Some(1), active, known), None);
        assert_eq!(display_to_follow(None, Some(1), active, known), None);
    }

    #[test]
    fn only_an_untracked_application_window_under_the_cursor_is_made_key() {
        use UntrackedFocus::{MakeKey, Nothing, SystemSurface};
        assert_eq!(
            untracked_focus(Some(5_u32), |_| false, |_| Some((0, 7_i32))),
            MakeKey(7, 5)
        );
        assert_eq!(
            untracked_focus(Some(5_u32), |_| false, |_| Some((25, 7_i32))),
            SystemSurface { layer: 25, window: 5 }
        );
        assert_eq!(
            untracked_focus(Some(5_u32), |_| false, |_| None::<(i32, i32)>),
            Nothing
        );
        assert_eq!(
            untracked_focus(
                Some(5_u32),
                |_| true,
                |_| -> Option<(i32, i32)> { panic!("not asked") }
            ),
            Nothing
        );
        assert_eq!(
            untracked_focus(
                None::<u32>,
                |_| panic!("not asked"),
                |_| -> Option<(i32, i32)> { panic!("not asked") }
            ),
            Nothing
        );
    }

    #[test]
    fn the_desktop_takes_focus_on_an_active_space_with_an_empty_workspace() {
        assert!(focuses_desktop(true, || true));
        assert!(!focuses_desktop(true, || false));
        assert!(!focuses_desktop(false, || panic!("not asked")));
    }

    #[test]
    fn a_removal_refocuses_only_when_it_takes_focus_and_keeps_a_sibling_macos_focused() {
        let app = |window: W| window.0;
        let (focused, sibling, other_app) = ((1, 1), (1, 2), (2, 1));
        let space = 9_u64;
        let decide = |removal, native: Option<W>, inactive: bool| {
            removal_focus(
                removal,
                Some(focused),
                app,
                |_| Some(space),
                |_| native,
                |window| window,
                |_, _| inactive,
            )
        };
        use Removal::{AppClosed, WindowRemoved};
        use RemovalFocus::{KeepNative, Refocus, Unaffected};
        // Not the focused window, not its app: nothing to do, nothing asked.
        assert_eq!(
            removal_focus(
                WindowRemoved(sibling),
                Some(focused),
                app,
                |_| -> Option<u64> { panic!("not asked") },
                |_| panic!("not asked"),
                |w| w,
                |_, _| panic!("not asked"),
            ),
            Unaffected
        );
        assert_eq!(decide(AppClosed(2), None, false), Unaffected);
        assert_eq!(
            removal_focus(
                WindowRemoved(focused),
                None,
                app,
                |_| Some(space),
                |_| None,
                |w| w,
                |_, _| false
            ),
            Unaffected
        );
        // The focused window or its app goes: refocus its Space.
        assert_eq!(decide(WindowRemoved(focused), None, false), Refocus(space));
        assert_eq!(decide(AppClosed(1), Some(sibling), false), Refocus(space));
        // macOS moved focus to a sibling (a native tab): keep it, unless it is in a workspace
        // that is not active, or another app's, or the removed window itself.
        assert_eq!(
            decide(WindowRemoved(focused), Some(sibling), false),
            KeepNative(sibling)
        );
        assert_eq!(
            decide(WindowRemoved(focused), Some(sibling), true),
            Refocus(space)
        );
        assert_eq!(
            decide(WindowRemoved(focused), Some(other_app), false),
            Refocus(space)
        );
        assert_eq!(
            decide(WindowRemoved(focused), Some(focused), false),
            Refocus(space)
        );
        // The sibling as the platform tracks it.
        let tracked = (1, 7);
        assert_eq!(
            removal_focus(
                WindowRemoved(focused),
                Some(focused),
                app,
                |_| Some(space),
                |_| Some(sibling),
                |_| tracked,
                |_, _| false
            ),
            KeepNative(tracked)
        );
        // No Space to refocus on: nothing to do.
        assert_eq!(
            removal_focus(
                WindowRemoved(focused),
                Some(focused),
                app,
                |_| None::<u64>,
                |_| None,
                |w| w,
                |_, _| false
            ),
            Unaffected
        );
    }
}
