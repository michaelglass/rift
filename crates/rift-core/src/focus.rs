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
