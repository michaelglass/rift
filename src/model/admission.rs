//! Whether rift manages a window, as a pure function of typed facts.
//!
//! Each fact is read off the window once ([`crate::model::reactor::WindowState::admission_facts`]),
//! and [`admission`] is the only place the policy lives.

/// The manageability heuristic's verdict on the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Manageability {
    Manageable,
    Unmanageable,
}

/// What an app rule says about managing the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rule {
    NoRule,
    Manage,
    Ignore,
}

impl From<Option<bool>> for Rule {
    fn from(manage_override: Option<bool>) -> Rule {
        match manage_override {
            None => Rule::NoRule,
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
    NativeTransient,
    Minimized,
    AxTransient,
    RuleIgnores,
    Unmanageable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Admitted,
    Vetoed(Veto),
}

/// A window is admitted unless the window server marks it transient, it is minimized, its roles
/// mark it transient (a Manage rule outranks only the AXDialog marker), a rule ignores it, or,
/// with no rule, the heuristic finds it unmanageable.
pub(crate) fn admission(facts: AdmissionFacts) -> Admission {
    let veto = match (facts.native, facts.minimize, facts.ax, facts.rule) {
        (NativeKind::Transient, ..) => Some(Veto::NativeTransient),
        (_, Minimize::Minimized, ..) => Some(Veto::Minimized),
        (_, _, AxKind::Transient, _) => Some(Veto::AxTransient),
        (_, _, AxKind::DialogWindow, Rule::NoRule | Rule::Ignore) => Some(Veto::AxTransient),
        (_, _, _, Rule::Ignore) => Some(Veto::RuleIgnores),
        (_, _, _, Rule::Manage) => None,
        (_, _, _, Rule::NoRule) => match facts.manageability {
            Manageability::Manageable => None,
            Manageability::Unmanageable => Some(Veto::Unmanageable),
        },
    };
    veto.map_or(Admission::Admitted, Admission::Vetoed)
}
