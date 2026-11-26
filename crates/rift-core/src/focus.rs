//! Decisions about focus and raising, as pure functions of what the platform observed. The
//! platform asks and acts (making a window key, warping, refocusing); these decide, asking for
//! a fact only when the decision needs it.

use crate::plain::Rect;

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

/// The window under the pointer, as the platform tracks it: its frame and, if it has one, its
/// window-server id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hovered<V> {
    pub frame: Rect,
    pub server: Option<V>,
}

/// What the platform knows of the window under the pointer, asked in this order and no further
/// than a mouse raise needs.
pub trait RaiseFacts {
    type Space: Copy;
    /// A window-server id.
    type Server: Copy + PartialEq;
    /// A window's level and sub-level.
    type Levels: Copy + PartialEq;

    /// Claimed by an owner other than the layout.
    fn claimed(&self) -> bool;
    /// Admitted to the layout.
    fn admitted(&self) -> bool;
    fn floating(&self) -> bool;
    fn active(&self, space: Self::Space) -> bool;
    fn in_active_workspace(&self, space: Self::Space) -> bool;
    /// Another tracked floating window lies entirely within the window's frame.
    fn could_occlude(&self) -> bool;
    /// The Space's stacking order, front first.
    fn order(&self, space: Self::Space) -> Vec<Self::Server>;
    /// The frame of the tracked floating window with this id, if it is one.
    fn floater(&self, server: Self::Server) -> Option<Rect>;
    /// A window's levels, if known.
    fn levels(&self, server: Self::Server) -> Option<Self::Levels>;
}

/// Why the window under the pointer is not raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unraised {
    /// The platform does not track it.
    Unknown,
    /// Claimed, or neither admitted nor floating.
    OutsideAdmission,
    /// No Space was resolved for it.
    NoSpace,
    InactiveSpace,
    /// Not in its Space's active workspace.
    OutsideActiveWorkspace,
    /// Raising it would bury a floating window above it.
    BuriesFloater,
}

/// Whether focus-follows-mouse raises the window under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseRaise {
    Raise,
    Skip(Unraised),
}

/// Whether to raise the window under the pointer: a tracked window that is unclaimed and
/// admitted or floating, on an active Space (`space`) in its active workspace, unless raising
/// it would bury a floating window above it. The stacking order and levels are asked only for
/// a window with a window-server id that could cover a floating window at all.
pub fn mouse_raise<F: RaiseFacts>(
    window: Option<Hovered<F::Server>>,
    space: Option<F::Space>,
    facts: &F,
) -> MouseRaise {
    let skip = MouseRaise::Skip;
    let Some(window) = window else {
        return skip(Unraised::Unknown);
    };
    if facts.claimed() || (!facts.admitted() && !facts.floating()) {
        return skip(Unraised::OutsideAdmission);
    }
    let Some(space) = space else {
        return skip(Unraised::NoSpace);
    };
    if !facts.active(space) {
        return skip(Unraised::InactiveSpace);
    }
    if !facts.in_active_workspace(space) {
        return skip(Unraised::OutsideActiveWorkspace);
    }
    let Some(candidate) = window.server else {
        return MouseRaise::Raise;
    };
    // Native stacking order matters only if another tracked floating window could be
    // completely covered by this raise.
    if !facts.could_occlude() {
        return MouseRaise::Raise;
    }
    let order = facts.order(space);
    if raise_buries_a_floater(
        candidate,
        window.frame,
        &order,
        |above| facts.floater(above),
        |server| facts.levels(server),
    ) {
        skip(Unraised::BuriesFloater)
    } else {
        MouseRaise::Raise
    }
}

/// Whether `outer` covers `inner` entirely (edges included).
pub fn contains(outer: Rect, inner: Rect) -> bool {
    outer.x <= inner.x
        && outer.y <= inner.y
        && outer.x + outer.w >= inner.x + inner.w
        && outer.y + outer.h >= inner.y + inner.h
}

/// Whether raising `candidate` (at `frame`) would bury a floating window it covers entirely that
/// is above it at the same level. `order` is the space's stacking order, front first;
/// `floating(id)` the frame of the tracked floating window with that id; `levels(id)` its
/// window level and sub-level, if known (asked only for windows that matter: the candidate's
/// once, at the first floater it covers, then each such floater's).
pub fn raise_buries_a_floater<V: Copy + PartialEq, L: PartialEq + Copy>(
    candidate: V,
    frame: Rect,
    order: &[V],
    floating: impl Fn(V) -> Option<Rect>,
    levels: impl Fn(V) -> Option<L>,
) -> bool {
    let mut candidate_levels = None;
    order.iter().take_while(|&&above| above != candidate).any(|&above| {
        floating(above).is_some_and(|above_frame| contains(frame, above_frame))
            && candidate_levels
                .get_or_insert_with(|| levels(candidate))
                .zip(levels(above))
                .is_some_and(|(candidate, above)| candidate == above)
    })
}

/// Each decision as a question on plain data. Its arguments are plain values; every fact it
/// asks for is a [`Fact`] (or, asked about a key, [`Answers`]): what the platform answered, or
/// that the platform's run of the decision never asked it. Each question names its facts in
/// `FACTS`, in the order the wire carries them; an answer that read a fact the question does not
/// carry is [`Unasked`]. Spaces, workspaces, window-server ids, modes and apps are one word
/// each; a window that has an app is a [`WindowId`] (its app is its pid).
pub mod plain {
    use super::{
        ActivationFacts, Follow, Hovered, MouseRaise, RaiseFacts, Removal, RemovalFocus,
        UntrackedFocus,
    };
    use crate::modes::Axis;
    use crate::plain::{Answers, Fact, Reading, Rect, Unasked, WindowId};

    /// [`super::removal_focus`]: the removed window's Space, macOS's key window, the successor
    /// each native window is tracked as, and whether a successor is in a workspace that is not
    /// active.
    #[derive(Debug, Clone, PartialEq)]
    pub struct RemovalQuestion {
        pub removal: Removal<WindowId, i32>,
        pub focused: Option<WindowId>,
        pub removed_space: Fact<Option<u64>>,
        pub native_focus: Fact<Option<WindowId>>,
        pub successors: Answers<WindowId, WindowId>,
        pub inactive: Answers<WindowId, bool>,
    }

    impl RemovalQuestion {
        pub const FACTS: &[&str] = &["removed_space", "native_focus", "successor", "inactive"];

        pub fn answer(&self) -> Result<RemovalFocus<WindowId, u64>, Unasked> {
            let reading = Reading::new();
            let answer = super::removal_focus(
                self.removal,
                self.focused,
                |window| window.pid,
                |_| reading.fact(&self.removed_space, 0, None),
                |_| reading.fact(&self.native_focus, 1, None),
                |native| reading.about(&self.successors, &native, 2, native),
                |_, successor| reading.about(&self.inactive, &successor, 3, false),
            );
            reading.answer(answer)
        }
    }

    /// [`super::focus_candidate`].
    #[derive(Debug, Clone, PartialEq)]
    pub struct Candidate {
        pub preferred: Option<u64>,
        pub last_focused: Fact<Option<u64>>,
        pub in_workspace: Fact<Vec<u64>>,
        pub visible: Answers<u64, bool>,
    }

    impl Candidate {
        pub const FACTS: &[&str] = &["last_focused", "in_workspace", "visible"];

        pub fn answer(&self) -> Result<Option<u64>, Unasked> {
            let reading = Reading::new();
            let answer = super::focus_candidate(
                self.preferred,
                || reading.fact(&self.last_focused, 0, None),
                || reading.fact(&self.in_workspace, 1, Vec::new()),
                |window| reading.about(&self.visible, &window, 2, false),
            );
            reading.answer(answer)
        }
    }

    /// [`super::hides_focus`].
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Hides {
        pub window: u64,
        pub main: Option<u64>,
        pub focused: Option<u64>,
        pub in_inactive_workspace: Fact<bool>,
    }

    impl Hides {
        pub const FACTS: &[&str] = &["in_inactive_workspace"];

        pub fn answer(&self) -> Result<bool, Unasked> {
            let reading = Reading::new();
            let answer = super::hides_focus(self.window, self.main, self.focused, || {
                reading.fact(&self.in_inactive_workspace, 0, false)
            });
            reading.answer(answer)
        }
    }

    /// [`super::activation_follow`]'s facts.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Activation {
        pub standard: Fact<bool>,
        pub unavailable: Fact<bool>,
        pub space: Fact<Option<u64>>,
        pub workspace: Fact<Option<u64>>,
        pub active_workspace: Fact<Option<u64>>,
        pub pending_window: Fact<bool>,
        /// The mode refusing the follow: the refusing axis, a refusing pointer's window as its
        /// word.
        pub mode_refusal: Fact<Option<Axis<u64>>>,
        pub active_space_fullscreen: Fact<bool>,
        pub unmanageable_source: Fact<Option<u64>>,
        pub blacklisted: Fact<Option<bool>>,
    }

    /// An [`Activation`] being read.
    struct Reads<'a> {
        facts: &'a Activation,
        reading: Reading,
    }

    impl ActivationFacts for Reads<'_> {
        type Mode = Axis<u64>;
        type Server = u64;
        type Space = u64;
        type Workspace = u64;

        fn standard(&self) -> bool { self.reading.fact(&self.facts.standard, 0, false) }

        fn unavailable(&self) -> bool { self.reading.fact(&self.facts.unavailable, 1, false) }

        fn space(&self) -> Option<u64> { self.reading.fact(&self.facts.space, 2, None) }

        fn workspace(&self, _: u64) -> Option<u64> {
            self.reading.fact(&self.facts.workspace, 3, None)
        }

        fn active_workspace(&self, _: u64) -> Option<u64> {
            self.reading.fact(&self.facts.active_workspace, 4, None)
        }

        fn pending_window(&self) -> bool { self.reading.fact(&self.facts.pending_window, 5, false) }

        fn mode_refusal(&self) -> Option<Axis<u64>> {
            self.reading.fact(&self.facts.mode_refusal, 6, None)
        }

        fn active_space_fullscreen(&self) -> bool {
            self.reading.fact(&self.facts.active_space_fullscreen, 7, false)
        }

        fn unmanageable_source(&self) -> Option<u64> {
            self.reading.fact(&self.facts.unmanageable_source, 8, None)
        }

        fn blacklisted(&self) -> Option<bool> {
            self.reading.fact(&self.facts.blacklisted, 9, None)
        }
    }

    impl Activation {
        pub const FACTS: &[&str] = &[
            "standard",
            "unavailable",
            "space",
            "workspace",
            "active_workspace",
            "pending_window",
            "mode_refusal",
            "active_space_fullscreen",
            "unmanageable_source",
            "blacklisted",
        ];

        pub fn answer(&self) -> Result<Follow<u64, u64, Axis<u64>, u64>, Unasked> {
            let reads = Reads {
                facts: self,
                reading: Reading::new(),
            };
            let answer = super::activation_follow(&reads);
            reads.reading.answer(answer)
        }
    }

    /// [`super::display_to_follow`].
    #[derive(Debug, Clone, PartialEq)]
    pub struct DisplayFollow {
        pub focused_space: Option<u64>,
        pub previous: Option<u64>,
        pub active: Answers<u64, bool>,
        pub on_known_display: Answers<u64, bool>,
    }

    impl DisplayFollow {
        pub const FACTS: &[&str] = &["active", "on_known_display"];

        pub fn answer(&self) -> Result<Option<u64>, Unasked> {
            let reading = Reading::new();
            let answer = super::display_to_follow(
                self.focused_space,
                self.previous,
                |space| reading.about(&self.active, &space, 0, false),
                |space| reading.about(&self.on_known_display, &space, 1, false),
            );
            reading.answer(answer)
        }
    }

    /// [`super::untracked_focus`]: whether the window is tracked, what is known of it (its
    /// layer and app).
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Untracked {
        pub under_cursor: Option<u64>,
        pub tracked: Fact<bool>,
        pub known: Fact<Option<(i32, u64)>>,
    }

    impl Untracked {
        pub const FACTS: &[&str] = &["tracked", "known"];

        pub fn answer(&self) -> Result<UntrackedFocus<u64, u64>, Unasked> {
            let reading = Reading::new();
            let answer = super::untracked_focus(
                self.under_cursor,
                |_| reading.fact(&self.tracked, 0, false),
                |_| reading.fact(&self.known, 1, None),
            );
            reading.answer(answer)
        }
    }

    /// [`super::mouse_raise`]: the window (its frame and window-server id, if tracked) and its
    /// Space are given; the rest is asked about the window, its Space, and the windows above it
    /// (each a window-server id; levels are a level and a sub-level).
    #[derive(Debug, Clone, PartialEq)]
    pub struct Raise {
        pub window: Option<Hovered<u64>>,
        pub space: Option<u64>,
        pub claimed: Fact<bool>,
        pub admitted: Fact<bool>,
        pub floating: Fact<bool>,
        pub active: Fact<bool>,
        pub in_active_workspace: Fact<bool>,
        pub could_occlude: Fact<bool>,
        pub order: Fact<Vec<u64>>,
        pub floaters: Answers<u64, Option<Rect>>,
        pub levels: Answers<u64, Option<(i64, i32)>>,
    }

    /// A [`Raise`] being read.
    struct RaiseReads<'a> {
        facts: &'a Raise,
        reading: Reading,
    }

    impl RaiseFacts for RaiseReads<'_> {
        type Levels = (i64, i32);
        type Server = u64;
        type Space = u64;

        fn claimed(&self) -> bool { self.reading.fact(&self.facts.claimed, 0, false) }

        fn admitted(&self) -> bool { self.reading.fact(&self.facts.admitted, 1, false) }

        fn floating(&self) -> bool { self.reading.fact(&self.facts.floating, 2, false) }

        fn active(&self, _: u64) -> bool { self.reading.fact(&self.facts.active, 3, false) }

        fn in_active_workspace(&self, _: u64) -> bool {
            self.reading.fact(&self.facts.in_active_workspace, 4, false)
        }

        fn could_occlude(&self) -> bool { self.reading.fact(&self.facts.could_occlude, 5, false) }

        fn order(&self, _: u64) -> Vec<u64> { self.reading.fact(&self.facts.order, 6, Vec::new()) }

        fn floater(&self, server: u64) -> Option<Rect> {
            self.reading.about(&self.facts.floaters, &server, 7, None)
        }

        fn levels(&self, server: u64) -> Option<(i64, i32)> {
            self.reading.about(&self.facts.levels, &server, 8, None)
        }
    }

    impl Raise {
        pub const FACTS: &[&str] = &[
            "claimed",
            "admitted",
            "floating",
            "active",
            "in_active_workspace",
            "could_occlude",
            "order",
            "floater",
            "levels",
        ];

        pub fn answer(&self) -> Result<MouseRaise, Unasked> {
            let reads = RaiseReads {
                facts: self,
                reading: Reading::new(),
            };
            let answer = super::mouse_raise(self.window, self.space, &reads);
            reads.reading.answer(answer)
        }
    }

    /// [`super::focuses_desktop`].
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Desktop {
        pub active: bool,
        pub workspace_empty: Fact<bool>,
    }

    impl Desktop {
        pub const FACTS: &[&str] = &["workspace_empty"];

        pub fn answer(&self) -> Result<bool, Unasked> {
            let reading = Reading::new();
            let answer = super::focuses_desktop(self.active, || {
                reading.fact(&self.workspace_empty, 0, false)
            });
            reading.answer(answer)
        }
    }
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
    fn a_raise_buries_a_floater_only_above_it_covered_entirely_at_the_same_level() {
        let rect = |x, y, w, h| Rect { x, y, w, h };
        let frame = rect(0., 0., 500., 500.);
        let inside = rect(10., 10., 100., 100.);
        let floating = |id| (id == 2).then_some(inside);
        let same = |_| Some(0);
        // Floater 2 above the candidate 5, inside it, same level: buried.
        assert!(raise_buries_a_floater(5, frame, &[2, 5], floating, same));
        // Below the candidate, it stays above nothing.
        assert!(!raise_buries_a_floater(5, frame, &[5, 2], floating, same));
        // At another level it stays visible.
        assert!(!raise_buries_a_floater(5, frame, &[2, 5], floating, |w| Some(w)));
        // Unknown levels are not the same level.
        assert!(!raise_buries_a_floater(5, frame, &[2, 5], floating, |_| None::<
            i32,
        >));
        // Not entirely covered.
        let partly = |id| (id == 2).then_some(rect(450., 0., 100., 100.));
        assert!(!raise_buries_a_floater(5, frame, &[2, 5], partly, same));
        // Edges included.
        assert!(contains(frame, frame));
    }

    /// Facts for a mouse raise as data; a fact left `None` panics if asked.
    #[derive(Clone, Copy, Default)]
    struct Pointed {
        claimed: Option<bool>,
        admitted: Option<bool>,
        floating: Option<bool>,
        active: Option<bool>,
        in_workspace: Option<bool>,
        occludes: Option<bool>,
    }

    impl RaiseFacts for Pointed {
        type Levels = i32;
        type Server = u32;
        type Space = u32;

        fn claimed(&self) -> bool { self.claimed.expect("claimed asked") }

        fn admitted(&self) -> bool { self.admitted.expect("admitted asked") }

        fn floating(&self) -> bool { self.floating.expect("floating asked") }

        fn active(&self, _: u32) -> bool { self.active.expect("active asked") }

        fn in_active_workspace(&self, _: u32) -> bool { self.in_workspace.expect("asked") }

        fn could_occlude(&self) -> bool { self.occludes.expect("could_occlude asked") }

        fn order(&self, _: u32) -> Vec<u32> { vec![2, 5] }

        fn floater(&self, id: u32) -> Option<Rect> {
            (id == 2).then_some(Rect { x: 10., y: 10., w: 10., h: 10. })
        }

        fn levels(&self, _: u32) -> Option<i32> { Some(0) }
    }

    #[test]
    fn the_mouse_raises_an_admitted_or_floating_window_in_the_active_workspace_unless_it_buries_a_floater()
     {
        let window = Some(Hovered {
            frame: Rect { x: 0., y: 0., w: 100., h: 100. },
            server: Some(5),
        });
        let all = Pointed {
            claimed: Some(false),
            admitted: Some(true),
            floating: None,
            active: Some(true),
            in_workspace: Some(true),
            occludes: Some(false),
        };
        let skip = MouseRaise::Skip;
        assert_eq!(
            mouse_raise(None, Some(1), &Pointed::default()),
            skip(Unraised::Unknown)
        );
        let claimed = Pointed {
            claimed: Some(true),
            ..Pointed::default()
        };
        assert_eq!(
            mouse_raise(window, Some(1), &claimed),
            skip(Unraised::OutsideAdmission)
        );
        let tiled_out = Pointed {
            admitted: Some(false),
            floating: Some(false),
            ..all
        };
        assert_eq!(
            mouse_raise(window, Some(1), &tiled_out),
            skip(Unraised::OutsideAdmission)
        );
        let floating = Pointed {
            admitted: Some(false),
            floating: Some(true),
            ..all
        };
        assert_eq!(mouse_raise(window, Some(1), &floating), MouseRaise::Raise);
        assert_eq!(mouse_raise(window, None, &all), skip(Unraised::NoSpace));
        let inactive = Pointed {
            active: Some(false),
            in_workspace: None,
            ..all
        };
        assert_eq!(
            mouse_raise(window, Some(1), &inactive),
            skip(Unraised::InactiveSpace)
        );
        let outside = Pointed {
            in_workspace: Some(false),
            occludes: None,
            ..all
        };
        assert_eq!(
            mouse_raise(window, Some(1), &outside),
            skip(Unraised::OutsideActiveWorkspace)
        );
        let native_less = window.map(|window| Hovered { server: None, ..window });
        assert_eq!(
            mouse_raise(native_less, Some(1), &Pointed { occludes: None, ..all }),
            MouseRaise::Raise
        );
        assert_eq!(mouse_raise(window, Some(1), &all), MouseRaise::Raise);
        let occludes = Pointed { occludes: Some(true), ..all };
        assert_eq!(
            mouse_raise(window, Some(1), &occludes),
            skip(Unraised::BuriesFloater)
        );
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
