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

#[cfg(test)]
mod tests {
    use super::*;

    /// A window is (app, index).
    type W = (u32, u32);

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
