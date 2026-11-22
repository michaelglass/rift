//! Whether rift manages a window, as a pure function of typed facts.
//!
//! The window keeps these facts and [`crate::model::reactor::WindowState::admission_facts`]
//! reads them off it; [`admission`] is the only place the policy lives.

use crate::sys::app::{WindowInfo, is_ibkr_desktop_dialog};

/// The manageability heuristic's verdict on the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Manageability {
    Manageable,
    Unmanageable,
}

/// What an app rule says about managing the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Rule {
    /// No rule decides, so the heuristic does.
    #[default]
    Unset,
    Manage,
    Ignore,
}

impl From<Option<bool>> for Rule {
    fn from(manage_override: Option<bool>) -> Rule {
        match manage_override {
            None => Rule::Unset,
            Some(true) => Rule::Manage,
            Some(false) => Rule::Ignore,
        }
    }
}

/// What the accessibility roles say of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AxKind {
    Ordinary,
    /// An AXWindow whose only transient marker is its AXDialog subrole: some apps use it for a
    /// real main window, so a Manage rule admits it.
    DialogWindow,
    /// Any other transient (a non-window role, a sheet, a system dialog).
    Transient,
}

impl AxKind {
    /// Reads the window's AX role and subrole.
    pub(crate) fn of(info: &WindowInfo) -> AxKind {
        match (info.ax_role.as_deref(), info.ax_subrole.as_deref()) {
            (Some(role), _) if role != "AXWindow" => AxKind::Transient,
            (Some(_), Some("AXDialog")) if is_ibkr_desktop_dialog(info) => AxKind::Ordinary,
            (Some(_), Some("AXDialog")) => AxKind::DialogWindow,
            (_, Some("AXDialog" | "AXSystemDialog" | "AXSheet")) => AxKind::Transient,
            _ => AxKind::Ordinary,
        }
    }
}

/// Whether the window server marks the window transient for its app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeKind {
    Ordinary,
    Transient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Minimize {
    Shown,
    Minimized,
}

/// Everything admission reads of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdmissionFacts {
    pub manageability: Manageability,
    pub rule: Rule,
    pub ax: AxKind,
    pub native: NativeKind,
    pub minimize: Minimize,
}

/// Why a window is not admitted: the first that applies, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Veto {
    Minimized,
    RuleIgnores,
    NativeTransient,
    AxTransient,
    Unmanageable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Admitted,
    Vetoed(Veto),
}

impl Admission {
    /// Kept out for a reason that can pass (anything but a rule ignoring it): the window keeps
    /// its ownership and only leaves layout until then.
    pub(crate) fn is_suspended(self) -> bool {
        matches!(self, Admission::Vetoed(veto) if veto != Veto::RuleIgnores)
    }
}

/// A window is admitted unless it is minimized, a rule ignores it, the window server marks it
/// transient, its roles mark it transient (a Manage rule outranks only the AXDialog marker), or,
/// with no rule, the heuristic finds it unmanageable.
pub(crate) fn admission(facts: AdmissionFacts) -> Admission {
    let veto = match (facts.minimize, facts.rule, facts.native, facts.ax) {
        (Minimize::Minimized, ..) => Some(Veto::Minimized),
        (_, Rule::Ignore, ..) => Some(Veto::RuleIgnores),
        (_, _, NativeKind::Transient, _) => Some(Veto::NativeTransient),
        (_, _, _, AxKind::Transient) => Some(Veto::AxTransient),
        (_, Rule::Unset, _, AxKind::DialogWindow) => Some(Veto::AxTransient),
        (_, Rule::Manage, ..) => None,
        (_, Rule::Unset, ..) => match facts.manageability {
            Manageability::Manageable => None,
            Manageability::Unmanageable => Some(Veto::Unmanageable),
        },
    };
    veto.map_or(Admission::Admitted, Admission::Vetoed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy() {
        use Admission::*;
        use AxKind::{DialogWindow, Ordinary as Ax, Transient as AxT};
        use Manageability::{Manageable as M, Unmanageable as U};
        use Minimize::{Minimized as Min, Shown};
        use NativeKind::{Ordinary as Nat, Transient as NatT};
        use Rule::{Ignore, Manage, Unset};
        use Veto::*;
        #[rustfmt::skip]
        let table = [
            // manageability, rule, ax, native, minimize => admission
            ((M, Unset, Ax, Nat, Shown), Admitted),
            ((U, Unset, Ax, Nat, Shown), Vetoed(Unmanageable)),
            ((U, Manage, Ax, Nat, Shown), Admitted),
            ((M, Ignore, Ax, Nat, Shown), Vetoed(RuleIgnores)),
            ((M, Unset, DialogWindow, Nat, Shown), Vetoed(AxTransient)),
            ((M, Manage, DialogWindow, Nat, Shown), Admitted),
            ((M, Manage, AxT, Nat, Shown), Vetoed(AxTransient)),
            ((M, Manage, Ax, NatT, Shown), Vetoed(NativeTransient)),
            ((M, Ignore, AxT, NatT, Shown), Vetoed(RuleIgnores)),
            ((M, Ignore, Ax, Nat, Min), Vetoed(Minimized)),
            ((M, Manage, Ax, Nat, Min), Vetoed(Minimized)),
        ];
        for ((manageability, rule, ax, native, minimize), expected) in table {
            let facts = AdmissionFacts {
                manageability,
                rule,
                ax,
                native,
                minimize,
            };
            assert_eq!(admission(facts), expected, "{facts:?}");
        }
    }
}
