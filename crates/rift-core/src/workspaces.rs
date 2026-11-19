//! Decisions about virtual workspaces (which one a command reaches, where stepping goes), as
//! pure functions of the catalog the platform keeps. Generic over the platform's workspace id
//! `K` and Space `S`.

use crate::lifecycle::WorkspaceMode;

/// Where a workspace command is given: the Space it acts from, and the Spaces visible now (none
/// known: only the acting Space).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context<'a, S> {
    pub space: S,
    pub visible: &'a [S],
}

/// Whether a workspace on `on` is reachable from `context`: per display, only the acting
/// Space's; globally, any visible Space's (the acting Space's when none is known).
pub fn eligible<S: Copy + PartialEq>(mode: WorkspaceMode, context: Context<'_, S>, on: S) -> bool {
    match mode {
        WorkspaceMode::PerDisplay => on == context.space,
        WorkspaceMode::Global => {
            context.visible.contains(&on) || (context.visible.is_empty() && on == context.space)
        }
    }
}

/// Which way stepping goes through the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Next,
    Previous,
}

/// Whether stepping passes over workspaces with no windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipEmpty {
    Skip,
    Keep,
}

/// Whether stepping past the end comes back round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrapping {
    Wrap,
    Stop,
}

/// The workspace a step from `current` reaches, through `catalog` (each workspace with its
/// Space, in catalog order): the next eligible one that way (non-empty if skipping), else, when
/// wrapping, the first from the other end (`current` itself if it is the only candidate).
/// Nothing if `current` is not an eligible catalog workspace.
pub fn step<K: Copy + PartialEq, S: Copy + PartialEq>(
    mode: WorkspaceMode,
    catalog: &[(K, S)],
    context: Context<'_, S>,
    current: K,
    way: Step,
    skip_empty: SkipEmpty,
    wrapping: Wrapping,
    has_windows: impl Fn(K) -> bool,
) -> Option<K> {
    let reachable = |&(_, on): &(K, S)| eligible(mode, context, on);
    if !catalog.iter().any(|entry| entry.0 == current && reachable(entry)) {
        return None;
    }
    let counts = |id| match skip_empty {
        SkipEmpty::Keep => true,
        SkipEmpty::Skip => has_windows(id),
    };
    let ordered: Box<dyn Iterator<Item = &(K, S)>> = match way {
        Step::Next => Box::new(catalog.iter()),
        Step::Previous => Box::new(catalog.iter().rev()),
    };
    let mut crossed = false;
    let mut wrapped = None;
    for &(id, _) in ordered.filter(|entry| reachable(entry)) {
        if id == current {
            crossed = true;
            if wrapped.is_none() && counts(id) {
                wrapped = Some(id);
            }
        } else if counts(id) {
            if crossed {
                return Some(id);
            }
            wrapped.get_or_insert(id);
        }
    }
    match wrapping {
        Wrapping::Wrap => wrapped,
        Wrapping::Stop => None,
    }
}

/// How a command names a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selector<N> {
    /// Its place in the scope.
    Index(usize),
    Name(N),
}

/// The workspace `selector` names in `scope` (the workspaces in the acting Space's namespace,
/// with their Space and name, in catalog order), if it is reachable from `context`.
pub fn resolve<K: Copy, S: Copy + PartialEq, N: PartialEq>(
    mode: WorkspaceMode,
    scope: &[(K, S, N)],
    context: Context<'_, S>,
    selector: &Selector<N>,
) -> Option<(K, S)> {
    let (id, on, _) = match selector {
        Selector::Index(index) => scope.get(*index)?,
        Selector::Name(name) => scope.iter().find(|(_, _, candidate)| candidate == name)?,
    };
    eligible(mode, context, *on).then_some((*id, *on))
}

/// The workspace navigation history points at, as a target: `last` (the acting Space's
/// previous workspace) on the Space it is on now, if that is reachable.
pub fn previous<K: Copy, S: Copy + PartialEq>(
    mode: WorkspaceMode,
    context: Context<'_, S>,
    last: Option<K>,
    space_of: impl FnOnce(K) -> Option<S>,
) -> Option<(K, S)> {
    let id = last?;
    let on = space_of(id)?;
    eligible(mode, context, on).then_some((id, on))
}

/// What a native Space shows (the platform's `SpaceActivation`, in the core's terms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation<K> {
    Showing {
        current: K,
        previous: Option<K>,
        remembered: Option<K>,
    },
    Vacant {
        remembered: Option<K>,
    },
}

/// A change to the workspaces' topology, as the platform makes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopologyEvent<K, S> {
    /// A workspace was created on a Space, at the end of the catalog.
    Created {
        workspace: K,
        space: S,
    },
    Removed(K),
    /// A workspace moved to another Space.
    Moved {
        workspace: K,
        space: S,
    },
    /// What a Space shows now.
    Activated {
        space: S,
        activation: Activation<K>,
    },
    /// A Space has no activation any more.
    Cleared(S),
}

/// The workspaces' topology as the core keeps it: the catalog (each workspace with its Space,
/// in catalog order) and what each Space shows.
#[derive(Debug, Clone)]
pub struct Topology<K, S> {
    catalog: Vec<(K, S)>,
    activations: Vec<(S, Activation<K>)>,
}

impl<K, S> Default for Topology<K, S> {
    fn default() -> Self {
        Topology {
            catalog: Vec::new(),
            activations: Vec::new(),
        }
    }
}

impl<K: Copy + PartialEq, S: Copy + PartialEq> Topology<K, S> {
    pub fn step(&mut self, event: TopologyEvent<K, S>) {
        match event {
            TopologyEvent::Created { workspace, space } => self.catalog.push((workspace, space)),
            TopologyEvent::Removed(workspace) => self.catalog.retain(|(id, _)| *id != workspace),
            TopologyEvent::Moved { workspace, space } => {
                if let Some(entry) = self.catalog.iter_mut().find(|(id, _)| *id == workspace) {
                    entry.1 = space;
                }
            }
            TopologyEvent::Activated { space, activation } => {
                match self.activations.iter_mut().find(|(on, _)| *on == space) {
                    Some(entry) => entry.1 = activation,
                    None => self.activations.push((space, activation)),
                }
            }
            TopologyEvent::Cleared(space) => self.activations.retain(|(on, _)| *on != space),
        }
    }

    /// Each workspace with its Space, in catalog order.
    pub fn catalog(&self) -> &[(K, S)] { &self.catalog }

    pub fn activation(&self, space: S) -> Option<Activation<K>> {
        self.activations
            .iter()
            .find(|(on, _)| *on == space)
            .map(|(_, activation)| *activation)
    }

    /// Every Space with an activation, in the order they first got one.
    pub fn activations(&self) -> &[(S, Activation<K>)] { &self.activations }
}

/// Who places a window, when somebody does: a workspace, or an external owner holding where
/// the window returns to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership<K, O> {
    Assigned(K),
    Claimed {
        owner: O,
        destination: Destination<K>,
    },
}

/// Where a claimed window returns to when its claim ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination<K> {
    Returns(K),
    Nowhere,
}

/// A window's ownership, or that nobody places it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owned<K, O> {
    Unowned,
    Owner(Ownership<K, O>),
}

/// A change to who places a window, as the platform makes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipEvent<W, K, O> {
    /// Who places the window now.
    Owned { window: W, owned: Owned<K, O> },
}

/// Who places each window, as the core keeps it. A window without an entry is unowned.
#[derive(Debug, Clone)]
pub struct Ownerships<W, K, O> {
    windows: std::collections::BTreeMap<W, Ownership<K, O>>,
}

impl<W, K, O> Default for Ownerships<W, K, O> {
    fn default() -> Self {
        Ownerships {
            windows: std::collections::BTreeMap::new(),
        }
    }
}

impl<W: Copy + Ord, K: Copy, O: Copy> Ownerships<W, K, O> {
    pub fn step(&mut self, event: OwnershipEvent<W, K, O>) {
        match event {
            OwnershipEvent::Owned { window, owned: Owned::Unowned } => {
                self.windows.remove(&window);
            }
            OwnershipEvent::Owned {
                window,
                owned: Owned::Owner(ownership),
            } => {
                self.windows.insert(window, ownership);
            }
        }
    }

    pub fn owned(&self, window: W) -> Owned<K, O> {
        match self.windows.get(&window) {
            Some(ownership) => Owned::Owner(*ownership),
            None => Owned::Unowned,
        }
    }

    /// Every owned window and its ownership, in window order.
    pub fn owners(&self) -> impl Iterator<Item = (W, Ownership<K, O>)> + '_ {
        self.windows.iter().map(|(window, ownership)| (*window, *ownership))
    }
}

/// Each owned window naming a workspace the topology's catalog lacks (as assigned to it, or as
/// a claim's destination), with that workspace, in window order. While ownership and the
/// catalog are separate values nothing rules this out; it is a disagreement to report.
pub fn orphans<W: Copy + Ord, K: Copy + PartialEq, O: Copy, S>(
    ownerships: &Ownerships<W, K, O>,
    topology: &Topology<K, S>,
) -> Vec<(W, K)> {
    ownerships
        .owners()
        .filter_map(|(window, ownership)| match ownership {
            Ownership::Assigned(workspace)
            | Ownership::Claimed {
                destination: Destination::Returns(workspace),
                ..
            } => Some((window, workspace)),
            Ownership::Claimed {
                destination: Destination::Nowhere,
                ..
            } => None,
        })
        .filter(|(_, workspace)| !topology.catalog.iter().any(|(id, _)| id == workspace))
        .collect()
}

/// Each decision as a question on plain data. Workspaces, Spaces and names are one word each.
pub mod plain {
    use super::{Context, Selector, SkipEmpty, Step, Topology, TopologyEvent, Wrapping};
    use crate::lifecycle::WorkspaceMode;

    /// [`super::step`]: the workspaces with windows.
    #[derive(Debug, Clone, PartialEq)]
    pub struct StepQuestion {
        pub mode: WorkspaceMode,
        pub catalog: Vec<(u64, u64)>,
        pub space: u64,
        pub visible: Vec<u64>,
        pub current: u64,
        pub way: Step,
        pub skip_empty: SkipEmpty,
        pub wrapping: Wrapping,
        pub with_windows: Vec<u64>,
    }

    impl StepQuestion {
        pub fn answer(&self) -> Option<u64> {
            super::step(
                self.mode,
                &self.catalog,
                Context {
                    space: self.space,
                    visible: &self.visible,
                },
                self.current,
                self.way,
                self.skip_empty,
                self.wrapping,
                |id| self.with_windows.contains(&id),
            )
        }
    }

    /// [`super::resolve`].
    #[derive(Debug, Clone, PartialEq)]
    pub struct ResolveQuestion {
        pub mode: WorkspaceMode,
        pub scope: Vec<(u64, u64, u64)>,
        pub space: u64,
        pub visible: Vec<u64>,
        pub selector: Selector<u64>,
    }

    impl ResolveQuestion {
        pub fn answer(&self) -> Option<(u64, u64)> {
            super::resolve(
                self.mode,
                &self.scope,
                Context {
                    space: self.space,
                    visible: &self.visible,
                },
                &self.selector,
            )
        }
    }

    /// [`super::previous`]: the Space the history's workspace is on, if it still exists.
    #[derive(Debug, Clone, PartialEq)]
    pub struct PreviousQuestion {
        pub mode: WorkspaceMode,
        pub space: u64,
        pub visible: Vec<u64>,
        pub last: Option<u64>,
        pub on: Option<u64>,
    }

    impl PreviousQuestion {
        pub fn answer(&self) -> Option<(u64, u64)> {
            super::previous(
                self.mode,
                Context {
                    space: self.space,
                    visible: &self.visible,
                },
                self.last,
                |_| self.on,
            )
        }
    }

    /// [`super::Topology::step`]: the topology an empty one becomes after these events, in
    /// order (every topology is reached this way).
    #[derive(Debug, Clone, PartialEq)]
    pub struct TopologyQuestion {
        pub events: Vec<TopologyEvent<u64, u64>>,
    }

    impl TopologyQuestion {
        pub fn answer(&self) -> Topology<u64, u64> {
            let mut topology = Topology::default();
            for event in &self.events {
                topology.step(*event);
            }
            topology
        }
    }
}

#[cfg(test)]
mod tests {
    use WorkspaceMode::{Global, PerDisplay};

    use super::*;

    #[test]
    fn the_topology_follows_the_catalog_and_what_each_space_shows() {
        let mut topology = Topology::<u32, u32>::default();
        topology.step(TopologyEvent::Created { workspace: 1, space: 10 });
        topology.step(TopologyEvent::Created { workspace: 2, space: 10 });
        topology.step(TopologyEvent::Created { workspace: 3, space: 20 });
        topology.step(TopologyEvent::Moved { workspace: 2, space: 20 });
        topology.step(TopologyEvent::Removed(1));
        assert_eq!(topology.catalog(), &[(2, 20), (3, 20)]);
        let showing = Activation::Showing {
            current: 2,
            previous: None,
            remembered: None,
        };
        topology.step(TopologyEvent::Activated { space: 20, activation: showing });
        topology.step(TopologyEvent::Activated {
            space: 10,
            activation: Activation::Vacant { remembered: Some(2) },
        });
        assert_eq!(topology.activation(20), Some(showing));
        let next = Activation::Showing {
            current: 3,
            previous: Some(2),
            remembered: None,
        };
        topology.step(TopologyEvent::Activated { space: 20, activation: next });
        assert_eq!(topology.activation(20), Some(next));
        topology.step(TopologyEvent::Cleared(10));
        assert_eq!(topology.activation(10), None);
        assert_eq!(topology.activations(), &[(20, next)]);
        // A move or removal of an unknown workspace changes nothing.
        topology.step(TopologyEvent::Moved { workspace: 9, space: 10 });
        topology.step(TopologyEvent::Removed(9));
        assert_eq!(topology.catalog(), &[(2, 20), (3, 20)]);
    }

    #[test]
    fn ownerships_follow_who_places_each_window() {
        let mut ownerships = Ownerships::<u32, u32, u32>::default();
        let claimed = Ownership::Claimed {
            owner: 7,
            destination: Destination::Returns(2),
        };
        ownerships.step(OwnershipEvent::Owned {
            window: 5,
            owned: Owned::Owner(Ownership::Assigned(1)),
        });
        ownerships.step(OwnershipEvent::Owned {
            window: 3,
            owned: Owned::Owner(claimed),
        });
        ownerships.step(OwnershipEvent::Owned {
            window: 5,
            owned: Owned::Owner(Ownership::Assigned(2)),
        });
        assert_eq!(ownerships.owned(5), Owned::Owner(Ownership::Assigned(2)));
        assert_eq!(ownerships.owners().collect::<Vec<_>>(), vec![
            (3, claimed),
            (5, Ownership::Assigned(2))
        ]);
        ownerships.step(OwnershipEvent::Owned {
            window: 3,
            owned: Owned::Unowned,
        });
        assert_eq!(ownerships.owned(3), Owned::Unowned);
        // Disowning a window nobody owns changes nothing.
        ownerships.step(OwnershipEvent::Owned {
            window: 9,
            owned: Owned::Unowned,
        });
        assert_eq!(ownerships.owners().collect::<Vec<_>>(), vec![(
            5,
            Ownership::Assigned(2)
        )]);
    }

    #[test]
    fn an_ownership_naming_a_workspace_the_catalog_lacks_is_an_orphan() {
        let mut topology = Topology::<u32, u32>::default();
        topology.step(TopologyEvent::Created { workspace: 1, space: 10 });
        let mut ownerships = Ownerships::<u32, u32, u32>::default();
        for (window, ownership) in [
            (1, Ownership::Assigned(1)),
            (2, Ownership::Assigned(2)),
            (3, Ownership::Claimed {
                owner: 7,
                destination: Destination::Returns(1),
            }),
            (4, Ownership::Claimed {
                owner: 7,
                destination: Destination::Returns(3),
            }),
            (5, Ownership::Claimed {
                owner: 7,
                destination: Destination::Nowhere,
            }),
        ] {
            ownerships.step(OwnershipEvent::Owned {
                window,
                owned: Owned::Owner(ownership),
            });
        }
        assert_eq!(orphans(&ownerships, &topology), vec![(2, 2), (4, 3)]);
    }

    #[test]
    fn a_workspace_is_reachable_on_the_acting_space_or_globally_on_a_visible_one() {
        let context = Context { space: 1_u32, visible: &[1, 2] };
        assert!(eligible(PerDisplay, context, 1));
        assert!(!eligible(PerDisplay, context, 2));
        assert!(eligible(Global, context, 2));
        assert!(!eligible(Global, context, 3));
        let unknown = Context { space: 1_u32, visible: &[] };
        assert!(eligible(Global, unknown, 1));
        assert!(!eligible(Global, unknown, 2));
    }

    #[test]
    fn stepping_goes_to_the_next_eligible_workspace_and_wraps_if_allowed() {
        // Workspaces 1..=5; 4 is on an invisible Space; 3 has no windows.
        let catalog = [(1_u32, 10_u32), (2, 10), (3, 20), (4, 30), (5, 20)];
        let context = Context {
            space: 10_u32,
            visible: &[10, 20],
        };
        let has_windows = |id| id != 3;
        let go = |current, way, skip, wrap| {
            step(Global, &catalog, context, current, way, skip, wrap, has_windows)
        };
        use SkipEmpty::{Keep, Skip};
        use Step::{Next, Previous};
        use Wrapping::{Stop, Wrap};
        assert_eq!(go(2, Next, Keep, Wrap), Some(3));
        assert_eq!(go(2, Next, Skip, Wrap), Some(5));
        assert_eq!(go(3, Next, Keep, Wrap), Some(5), "4 is not visible");
        assert_eq!(go(5, Next, Keep, Wrap), Some(1));
        assert_eq!(go(5, Next, Keep, Stop), None);
        assert_eq!(go(1, Previous, Keep, Wrap), Some(5));
        assert_eq!(go(5, Previous, Skip, Wrap), Some(2));
        // Not eligible, or not in the catalog: no step.
        assert_eq!(go(4, Next, Keep, Wrap), None);
        assert_eq!(go(9, Next, Keep, Wrap), None);
        // Per display, only the acting Space's workspaces.
        assert_eq!(
            step(PerDisplay, &catalog, context, 2, Next, Keep, Wrap, has_windows),
            Some(1)
        );
        // The only candidate wraps to itself; skipping, an empty current counts for nothing.
        let alone = [(7_u32, 10_u32)];
        assert_eq!(
            step(Global, &alone, context, 7, Next, Keep, Wrap, |_| true),
            Some(7)
        );
        assert_eq!(
            step(Global, &alone, context, 7, Next, Skip, Wrap, |_| false),
            None
        );
    }

    #[test]
    fn a_selector_names_a_workspace_in_scope_if_it_is_reachable() {
        let scope = [(1_u32, 10_u32, "a"), (2, 20, "b"), (3, 30, "c")];
        let context = Context {
            space: 10_u32,
            visible: &[10, 20],
        };
        assert_eq!(
            resolve(Global, &scope, context, &Selector::Index(1)),
            Some((2, 20))
        );
        assert_eq!(
            resolve(Global, &scope, context, &Selector::Name("a")),
            Some((1, 10))
        );
        assert_eq!(resolve(Global, &scope, context, &Selector::Name("c")), None);
        assert_eq!(resolve(Global, &scope, context, &Selector::Index(9)), None);
        assert_eq!(resolve(PerDisplay, &scope, context, &Selector::Index(1)), None);
        assert_eq!(
            previous(Global, context, Some(2_u32), |_| Some(20)),
            Some((2, 20))
        );
        assert_eq!(previous(Global, context, Some(3_u32), |_| Some(30)), None);
        assert_eq!(
            previous::<u32, u32>(Global, context, None, |_| panic!("not asked")),
            None
        );
        assert_eq!(previous(Global, context, Some(2_u32), |_| None), None);
    }
}
