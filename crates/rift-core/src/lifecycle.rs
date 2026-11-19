//! Decisions about a window's life (admission, app rules, placement), as pure functions of
//! what the platform observed. The platform asks and acts; these decide, asking for a fact
//! only when the decision needs it.

/// Why app rules run over some windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleRun {
    /// Windows are being admitted: only those whose admission can still be reconciled, and
    /// the layout hears of a window only if a rule changed something for it.
    Admission,
    /// The rules are re-applied (a title changed): every window, and the layout hears of each.
    Reapply,
}

/// The windows app rules run over, by Space in Space order: each window's Space is its
/// assignment's, else its best guess; a window with neither, or on an inactive Space, is left.
pub fn rule_targets<W: Copy, S: Copy + Ord>(
    windows: &[W],
    run: RuleRun,
    reconcilable: impl Fn(W) -> bool,
    space: impl Fn(W) -> Option<S>,
    active: impl Fn(S) -> bool,
) -> Vec<(S, Vec<W>)> {
    let mut by_space: std::collections::BTreeMap<S, Vec<W>> = Default::default();
    for &window in windows {
        let considered = match run {
            RuleRun::Reapply => true,
            RuleRun::Admission => reconcilable(window),
        };
        if !considered {
            continue;
        }
        if let Some(space) = space(window) {
            by_space.entry(space).or_default().push(window);
        }
    }
    by_space.into_iter().filter(|(space, _)| active(*space)).collect()
}

/// A window before and after its app rules ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleOutcome<K> {
    pub workspace_before: Option<K>,
    pub workspace: K,
    pub floating_before: bool,
    pub floating: bool,
    /// A rule had told rift to leave it alone.
    pub was_ignored: bool,
    /// It is temporarily unavailable (hidden, minimized, on its way somewhere).
    pub unavailable: bool,
}

/// Whether the layout hears of a window after its app rules ran: always on a re-apply;
/// otherwise when the rules moved it, floated or tiled it, or it had been ignored or was away.
pub fn rule_refreshes_layout<K: PartialEq>(run: RuleRun, outcome: &RuleOutcome<K>) -> bool {
    match run {
        RuleRun::Reapply => true,
        RuleRun::Admission => {
            outcome.unavailable
                || outcome.workspace_before.as_ref() != Some(&outcome.workspace)
                || outcome.floating_before != outcome.floating
                || outcome.was_ignored
        }
    }
}

/// How workspaces map onto displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMode {
    /// Each display's desktops have their own workspaces.
    PerDisplay,
    /// One set of workspaces across displays.
    Global,
}

/// A window that just opened, as placement sees it; a standard one with what the app rules
/// read of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewWindow<T> {
    /// The platform has no state for it.
    Untracked,
    /// Not a standard window (a panel, a sheet, ...).
    NotStandard,
    Standard(T),
}

/// The Space a new window moves to, opened on `opened_on`: in global mode, the pointer's
/// display's, if that is another active one, unless an app rule names the window's workspace
/// (launchers can activate an app on another display). The pointer is asked only in global
/// mode, the rules only for a standard window that would move.
pub fn pointer_display_target<S: Copy + PartialEq, T>(
    mode: WorkspaceMode,
    opened_on: S,
    pointer_space: impl FnOnce() -> Option<S>,
    active: impl Fn(S) -> bool,
    window: NewWindow<T>,
    rule_names_workspace: impl FnOnce(T) -> bool,
) -> Option<S> {
    match mode {
        WorkspaceMode::PerDisplay => None,
        WorkspaceMode::Global => {
            let target = pointer_space().filter(|&space| space != opened_on && active(space))?;
            match window {
                NewWindow::Untracked | NewWindow::NotStandard => None,
                NewWindow::Standard(window) => (!rule_names_workspace(window)).then_some(target),
            }
        }
    }
}

/// Where the evidence that a window is transient (a popover, a sheet, a panel) came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransientOrigin {
    /// The window server's own tags: final.
    Native,
    /// The accessibility heuristic, which a rule can override.
    Heuristic,
}

/// Whether `owner`'s window is a native transient, from its transient entry (none for a window
/// with no window-server id or no entry): native evidence for the same app, unless the window
/// is exempt (asked only then).
pub fn is_native_transient<P: PartialEq>(
    entry: Option<(TransientOrigin, P)>,
    owner: P,
    exempt: impl FnOnce() -> bool,
) -> bool {
    matches!(entry, Some((TransientOrigin::Native, pid)) if pid == owner) && !exempt()
}

/// A window lifecycle event, as the transient entries hear of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Departure<Id, P> {
    /// An AX window was destroyed, with its window-server id if it had one: its heuristic
    /// entry goes; native evidence stays until the window server says so.
    Destroyed(Option<Id>),
    /// The window server closed the window.
    Closed(Id),
    /// The app is gone, and every entry of its windows.
    AppGone(P),
}

/// Whether the transient entry for window `id`, of `owner` and from `origin`, survives
/// `departure`.
pub fn keeps_transient<Id: PartialEq, P: PartialEq>(
    departure: Departure<Id, P>,
    id: Id,
    origin: TransientOrigin,
    owner: P,
) -> bool {
    match departure {
        Departure::Destroyed(gone) => origin == TransientOrigin::Native || gone != Some(id),
        Departure::Closed(gone) => gone != id,
        Departure::AppGone(pid) => pid != owner,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_rules_run_over_the_windows_on_active_spaces_by_space() {
        // Window n is on Space n % 3 (Space 0: none); Space 2 is inactive; even windows are
        // reconcilable.
        let windows = [1_u32, 2, 3, 4, 5, 6, 7];
        let space = |window: u32| (window % 3 != 0).then_some(window % 3);
        let active = |space: u32| space != 2;
        let reconcilable = |window: u32| window % 2 == 0;
        assert_eq!(
            rule_targets(&windows, RuleRun::Reapply, reconcilable, space, active),
            vec![(1, vec![1, 4, 7])]
        );
        assert_eq!(
            rule_targets(&windows, RuleRun::Admission, reconcilable, space, active),
            vec![(1, vec![4])]
        );
        let all = |_| true;
        assert_eq!(
            rule_targets(&windows, RuleRun::Reapply, reconcilable, space, all),
            vec![(1, vec![1, 4, 7]), (2, vec![2, 5])]
        );
        // An unconsidered window's Space is not asked.
        let asked = std::cell::Cell::new(0);
        let counted = |window: u32| {
            asked.set(asked.get() + 1);
            space(window)
        };
        rule_targets(&windows, RuleRun::Admission, reconcilable, counted, active);
        assert_eq!(asked.get(), 3);
    }

    #[test]
    fn a_new_window_moves_to_the_pointers_other_active_display_in_global_mode() {
        use NewWindow::{NotStandard, Untracked};
        let standard = NewWindow::Standard(());
        use WorkspaceMode::{Global, PerDisplay};
        let (opened_on, pointer, inactive) = (1_u64, 2, 3);
        let active = |space| space != inactive;
        let no_rule = |()| false;
        let place = |pointer_space: Option<u64>, window: NewWindow<()>| {
            pointer_display_target(Global, opened_on, || pointer_space, active, window, no_rule)
        };
        assert_eq!(place(Some(pointer), standard), Some(pointer));
        // Already there, an inactive or unknown display, or not a standard window: it stays.
        assert_eq!(place(Some(opened_on), standard), None);
        assert_eq!(place(Some(inactive), standard), None);
        assert_eq!(place(None, standard), None);
        assert_eq!(place(Some(pointer), NotStandard), None);
        assert_eq!(place(Some(pointer), Untracked), None);
        // An app rule naming its workspace keeps it where the rule puts it.
        assert_eq!(
            pointer_display_target(
                Global,
                opened_on,
                || Some(pointer),
                active,
                standard,
                |()| true
            ),
            None
        );
        // Per display: the pointer is not asked.
        assert_eq!(
            pointer_display_target(
                PerDisplay,
                opened_on,
                || panic!("the pointer is not asked"),
                active,
                standard,
                |()| panic!("nor the rules"),
            ),
            None
        );
        // The rules are asked only for a standard window that would move.
        assert_eq!(
            pointer_display_target(
                Global,
                opened_on,
                || Some(opened_on),
                active,
                standard,
                |()| panic!("the rules are not asked")
            ),
            None
        );
    }

    #[test]
    fn a_window_is_a_native_transient_on_native_evidence_for_its_own_app_unless_exempt() {
        use TransientOrigin::{Heuristic, Native};
        let not_exempt = || false;
        assert!(is_native_transient(Some((Native, 7)), 7, not_exempt));
        assert!(!is_native_transient(Some((Native, 8)), 7, || panic!(
            "not asked"
        )));
        assert!(!is_native_transient(Some((Heuristic, 7)), 7, || panic!(
            "not asked"
        )));
        assert!(!is_native_transient(None, 7, || panic!("not asked")));
        assert!(!is_native_transient(Some((Native, 7)), 7, || true));
    }

    #[test]
    fn a_transient_entry_goes_with_its_window_or_its_app() {
        use Departure::{AppGone, Closed, Destroyed};
        use TransientOrigin::{Heuristic, Native};
        // An AX destruction drops the heuristic entry for that id only.
        assert!(!keeps_transient(Destroyed(Some(1)), 1, Heuristic, 7));
        assert!(keeps_transient(Destroyed(Some(1)), 1, Native, 7));
        assert!(keeps_transient(Destroyed(Some(1)), 2, Heuristic, 7));
        assert!(keeps_transient(Destroyed::<u32, i32>(None), 1, Heuristic, 7));
        // A window-server close drops its entry, native or not.
        assert!(!keeps_transient(Closed::<u32, i32>(1), 1, Native, 7));
        assert!(keeps_transient(Closed::<u32, i32>(1), 2, Native, 7));
        // An app's end drops every entry of its windows.
        assert!(!keeps_transient(AppGone::<u32, i32>(7), 1, Native, 7));
        assert!(keeps_transient(AppGone::<u32, i32>(8), 1, Native, 7));
    }

    #[test]
    fn the_layout_hears_of_a_window_whose_rules_changed_something_or_on_a_reapply() {
        let still = RuleOutcome {
            workspace_before: Some(1),
            workspace: 1,
            floating_before: false,
            floating: false,
            was_ignored: false,
            unavailable: false,
        };
        assert!(!rule_refreshes_layout(RuleRun::Admission, &still));
        assert!(rule_refreshes_layout(RuleRun::Reapply, &still));
        for changed in [
            RuleOutcome { workspace: 2, ..still },
            RuleOutcome {
                workspace_before: None,
                ..still
            },
            RuleOutcome { floating: true, ..still },
            RuleOutcome { floating_before: true, ..still },
            RuleOutcome { was_ignored: true, ..still },
            RuleOutcome { unavailable: true, ..still },
        ] {
            assert!(
                rule_refreshes_layout(RuleRun::Admission, &changed),
                "{changed:?}"
            );
        }
    }
}
