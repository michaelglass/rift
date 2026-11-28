//! What the layout engine keeps of a workspace's arrangement for a departed display's return
//! (engine slice E3c): one snapshot per workspace, and, while a display's windows are being
//! reconciled, the workspaces whose restore waits for the batch to end.
//!
//! A snapshot is a value the host makes (its layout system's state as the cores hold it), held
//! here and handed back; which windows it holds is computed from the value ([`Held`]), never kept
//! beside it. The machine decides whether a departure keeps one, when a restore waits for the
//! batch, which of the workspace's current windows the restore takes back, and what it changes
//! (the windows to remove from the snapshot's arrangement, those to add, whether the snapshot
//! is spent); the host applies the plan through the layout system. A snapshot holding no window
//! is dropped.

use std::collections::{BTreeMap, BTreeSet};

/// A snapshot's windows, as its value holds them.
pub trait Held<W> {
    fn windows(&self) -> Vec<W>;
}

/// Whether a reconcile batch is under way, and the workspaces waiting for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Batch<Ws> {
    #[default]
    Idle,
    Collecting(BTreeSet<Ws>),
}

/// One of a workspace's current windows, with the facts a restore reads of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate<W> {
    pub window: W,
    /// Assigned (or claimed) to the workspace.
    pub in_workspace: bool,
    /// Admitted, or only temporarily unavailable.
    pub present: bool,
    pub floating: bool,
}

/// The workspace a restore is asked for, as it is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Current<W> {
    /// Not laid out: nothing to restore into.
    Unlaid,
    /// Laid out, with its layout's windows in order.
    Laid(Vec<Candidate<W>>),
}

/// What a restore changes, applied by the host to a system built from `snapshot`.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan<W, S> {
    pub snapshot: S,
    /// The snapshot's windows not taken back, in the snapshot's order.
    pub remove: Vec<W>,
    /// The windows taken back, in the workspace's order.
    pub ordered: Vec<W>,
    /// Of those, the ones the snapshot does not hold, to add.
    pub add: Vec<W>,
    /// Every window the snapshot held is back: it is dropped.
    pub spent: bool,
}

/// What the layout engine tells the native memory.
#[derive(Clone, Debug, PartialEq)]
pub enum MemoryEvent<W, Ws, S> {
    /// A departure's snapshot of `workspace` (kept only if it holds a window and none is held).
    Kept { workspace: Ws, snapshot: S },
    /// A held snapshot's value after an edit made through its system (dropped if it holds none).
    Edited { workspace: Ws, snapshot: S },
    /// `workspace`'s snapshot goes, and its wait for the batch.
    Forgotten(Ws),
    /// Every snapshot goes (the layout settings changed).
    Cleared,
    /// A reconcile batch begins: restores wait for its end.
    ReconcileBegan,
    /// The batch ends: answers the workspaces whose restore waited.
    ReconcileEnded,
    /// A restore of `workspace` is asked (for `returning`, if one window is returning).
    RestoreAsked {
        workspace: Ws,
        returning: Option<W>,
        current: Current<W>,
    },
}

/// What the native memory answered.
#[derive(Clone, Debug, PartialEq)]
pub enum MemoryAnswer<W, Ws, S> {
    Nothing,
    /// The restore waits for the batch's end.
    Deferred,
    Restore(Plan<W, S>),
    /// The workspaces whose restore waited, to restore now.
    Waited(Vec<Ws>),
}

/// Every workspace's snapshot and the reconcile batch.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeMemory<Ws, S> {
    snapshots: BTreeMap<Ws, S>,
    batch: Batch<Ws>,
}

impl<Ws, S> Default for NativeMemory<Ws, S> {
    fn default() -> Self {
        NativeMemory {
            snapshots: BTreeMap::new(),
            batch: Batch::Idle,
        }
    }
}

impl<Ws: Ord + Copy, S: Clone> NativeMemory<Ws, S> {
    /// A machine holding exactly `snapshots` and `batch` (a twin's reseed).
    pub fn from_parts(snapshots: impl IntoIterator<Item = (Ws, S)>, batch: Batch<Ws>) -> Self {
        NativeMemory {
            snapshots: snapshots.into_iter().collect(),
            batch,
        }
    }

    /// Whether a departure of a window of `workspace` keeps a snapshot: not for a floating
    /// window, and not over one already held.
    pub fn keeps(&self, workspace: Ws, floating: bool) -> bool {
        !floating && !self.snapshots.contains_key(&workspace)
    }

    /// The workspaces whose snapshot holds `window`.
    pub fn holding<W: PartialEq>(&self, window: W) -> Vec<Ws>
    where S: Held<W> {
        (self.snapshots.iter())
            .filter(|(_, snapshot)| snapshot.windows().contains(&window))
            .map(|(workspace, _)| *workspace)
            .collect()
    }

    pub fn get(&self, workspace: Ws) -> Option<&S> { self.snapshots.get(&workspace) }

    pub fn snapshots(&self) -> impl Iterator<Item = (Ws, &S)> + '_ {
        self.snapshots.iter().map(|(workspace, snapshot)| (*workspace, snapshot))
    }

    pub fn batch(&self) -> &Batch<Ws> { &self.batch }

    pub fn is_empty(&self) -> bool { self.snapshots.is_empty() }

    pub fn step<W: Ord + Copy>(&mut self, event: MemoryEvent<W, Ws, S>) -> MemoryAnswer<W, Ws, S>
    where S: Held<W> {
        match event {
            MemoryEvent::Kept { workspace, snapshot } => {
                if !self.snapshots.contains_key(&workspace) && !snapshot.windows().is_empty() {
                    self.snapshots.insert(workspace, snapshot);
                }
            }
            MemoryEvent::Edited { workspace, snapshot } => {
                if snapshot.windows().is_empty() {
                    self.snapshots.remove(&workspace);
                } else if let Some(held) = self.snapshots.get_mut(&workspace) {
                    *held = snapshot;
                }
            }
            MemoryEvent::Forgotten(workspace) => {
                self.snapshots.remove(&workspace);
                if let Batch::Collecting(waiting) = &mut self.batch {
                    waiting.remove(&workspace);
                }
            }
            MemoryEvent::Cleared => self.snapshots.clear(),
            MemoryEvent::ReconcileBegan => self.batch = Batch::Collecting(BTreeSet::new()),
            MemoryEvent::ReconcileEnded => {
                let waited = match std::mem::take(&mut self.batch) {
                    Batch::Collecting(waited) => waited.into_iter().collect(),
                    Batch::Idle => Vec::new(),
                };
                return MemoryAnswer::Waited(waited);
            }
            MemoryEvent::RestoreAsked { workspace, returning, current } => {
                return self.restore(workspace, returning, current);
            }
        }
        MemoryAnswer::Nothing
    }

    fn restore<W: Ord + Copy>(
        &mut self,
        workspace: Ws,
        returning: Option<W>,
        current: Current<W>,
    ) -> MemoryAnswer<W, Ws, S>
    where
        S: Held<W>,
    {
        let Some(snapshot) = self.snapshots.get(&workspace) else {
            return MemoryAnswer::Nothing;
        };
        let original = snapshot.windows();
        if returning.is_some_and(|window| !original.contains(&window)) {
            return MemoryAnswer::Nothing;
        }
        if let Batch::Collecting(waiting) = &mut self.batch {
            waiting.insert(workspace);
            return MemoryAnswer::Deferred;
        }
        let Current::Laid(candidates) = current else {
            return MemoryAnswer::Nothing;
        };
        let mut desired = BTreeSet::new();
        let ordered: Vec<W> = (candidates.into_iter())
            .filter(|c| c.in_workspace && c.present && !c.floating && desired.insert(c.window))
            .map(|c| c.window)
            .collect();
        let held: BTreeSet<W> = original.iter().copied().collect();
        if held.is_disjoint(&desired) {
            return MemoryAnswer::Nothing;
        }
        let plan = Plan {
            snapshot: snapshot.clone(),
            remove: original.iter().copied().filter(|w| !desired.contains(w)).collect(),
            add: ordered.iter().copied().filter(|w| !held.contains(w)).collect(),
            ordered,
            spent: held.is_subset(&desired),
        };
        if plan.spent {
            self.snapshots.remove(&workspace);
        }
        MemoryAnswer::Restore(plan)
    }
}

// The machine as a Roc core owns it: snapshots' values stay with the host, so an event crosses
// with the windows it reads (computed from the values), and the Roc core holds only which
// workspaces hold a snapshot and the batch.

/// An event as it crosses to a core that holds no snapshot values.
#[derive(Clone, Debug, PartialEq)]
pub enum NativeEvent<W, Ws> {
    Kept {
        workspace: Ws,
        windows: Vec<W>,
    },
    Edited {
        workspace: Ws,
        windows: Vec<W>,
    },
    Forgotten(Ws),
    Cleared,
    ReconcileBegan,
    ReconcileEnded,
    /// `held`: the windows of the workspace's snapshot, none if it holds none.
    RestoreAsked {
        workspace: Ws,
        returning: Option<W>,
        current: Current<W>,
        held: Vec<W>,
    },
}

/// What such a core owns: the workspaces holding a snapshot, and the batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owned<Ws> {
    pub held: BTreeSet<Ws>,
    pub batch: Batch<Ws>,
}

/// A plan without the snapshot it is applied to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision<W> {
    pub remove: Vec<W>,
    pub ordered: Vec<W>,
    pub add: Vec<W>,
    pub spent: bool,
}

/// An answer without the snapshot value: what such a core answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decided<W, Ws> {
    Nothing,
    Deferred,
    Restore(Decision<W>),
    Waited(Vec<Ws>),
}

impl<W: Clone, Ws: Clone, S> MemoryAnswer<W, Ws, S> {
    pub fn decided(&self) -> Decided<W, Ws> {
        match self {
            MemoryAnswer::Nothing => Decided::Nothing,
            MemoryAnswer::Deferred => Decided::Deferred,
            MemoryAnswer::Restore(plan) => Decided::Restore(Decision {
                remove: plan.remove.clone(),
                ordered: plan.ordered.clone(),
                add: plan.add.clone(),
                spent: plan.spent,
            }),
            MemoryAnswer::Waited(waited) => Decided::Waited(waited.clone()),
        }
    }
}

impl<Ws: Ord + Copy, S: Clone> NativeMemory<Ws, S> {
    /// What the machine owns apart from the values.
    pub fn owned(&self) -> Owned<Ws> {
        Owned {
            held: self.snapshots.keys().copied().collect(),
            batch: self.batch.clone(),
        }
    }

    /// `event` as it crosses, read against the machine before it steps.
    pub fn crossing<W: Clone>(&self, event: &MemoryEvent<W, Ws, S>) -> NativeEvent<W, Ws>
    where S: Held<W> {
        match event {
            MemoryEvent::Kept { workspace, snapshot } => NativeEvent::Kept {
                workspace: *workspace,
                windows: snapshot.windows(),
            },
            MemoryEvent::Edited { workspace, snapshot } => NativeEvent::Edited {
                workspace: *workspace,
                windows: snapshot.windows(),
            },
            MemoryEvent::Forgotten(workspace) => NativeEvent::Forgotten(*workspace),
            MemoryEvent::Cleared => NativeEvent::Cleared,
            MemoryEvent::ReconcileBegan => NativeEvent::ReconcileBegan,
            MemoryEvent::ReconcileEnded => NativeEvent::ReconcileEnded,
            MemoryEvent::RestoreAsked { workspace, returning, current } => {
                NativeEvent::RestoreAsked {
                    workspace: *workspace,
                    returning: returning.clone(),
                    current: current.clone(),
                    held: self.get(*workspace).map(Held::windows).unwrap_or_default(),
                }
            }
        }
    }
}

impl<W> Held<W> for Vec<W>
where W: Clone
{
    fn windows(&self) -> Vec<W> { self.clone() }
}

/// Where a window a reconcile batch moves comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin<S> {
    /// It has no workspace.
    Unassigned,
    /// Its workspace is on Space `S` (every workspace rift holds is on one).
    On(S),
}

/// What a move in a reconcile batch is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// It leaves its workspace's Space: its source is captured before any arrival edits a tree.
    Departure,
    /// It moves within its workspace's Space.
    Within,
    /// It has no workspace to depart from.
    Unassigned,
}

/// What moving a window from `origin` to Space `target` is.
pub fn native_move<S: PartialEq>(origin: Origin<S>, target: S) -> Move {
    match origin {
        Origin::Unassigned => Move::Unassigned,
        Origin::On(space) if space == target => Move::Within,
        Origin::On(_) => Move::Departure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_off_its_workspaces_space_is_a_departure() {
        use super::{Move, Origin, native_move};
        assert_eq!(native_move(Origin::On(1), 2), Move::Departure);
        assert_eq!(native_move(Origin::On(2), 2), Move::Within);
        assert_eq!(native_move(Origin::<u8>::Unassigned, 2), Move::Unassigned);
    }

    /// A snapshot as a plain list of windows.
    #[derive(Clone, Debug, PartialEq)]
    struct Kept(Vec<u32>);

    impl Held<u32> for Kept {
        fn windows(&self) -> Vec<u32> { self.0.clone() }
    }

    type Memory = NativeMemory<u8, Kept>;

    fn candidate(window: u32) -> Candidate<u32> {
        Candidate {
            window,
            in_workspace: true,
            present: true,
            floating: false,
        }
    }

    fn ask(m: &mut Memory, returning: Option<u32>, windows: &[u32]) -> MemoryAnswer<u32, u8, Kept> {
        let current = Current::Laid(windows.iter().map(|&w| candidate(w)).collect());
        m.step(MemoryEvent::RestoreAsked {
            workspace: 1,
            returning,
            current,
        })
    }

    #[test]
    fn a_departure_keeps_one_snapshot_and_none_of_nothing_or_of_a_floating_window() {
        let mut m = Memory::default();
        assert!(m.keeps(1, false) && !m.keeps(1, true));
        m.step(MemoryEvent::Kept {
            workspace: 1,
            snapshot: Kept(vec![]),
        });
        assert!(m.is_empty(), "a snapshot holding nothing");
        m.step(MemoryEvent::Kept {
            workspace: 1,
            snapshot: Kept(vec![1, 2]),
        });
        m.step(MemoryEvent::Kept {
            workspace: 1,
            snapshot: Kept(vec![9]),
        });
        assert_eq!(m.get(1), Some(&Kept(vec![1, 2])));
        assert!(!m.keeps(1, false));
        assert_eq!((m.holding(2), m.holding(9)), (vec![1], vec![]));
    }

    #[test]
    fn a_restore_takes_back_the_present_windows_and_is_spent_when_all_are_back() {
        let mut m = Memory::default();
        m.step(MemoryEvent::Kept {
            workspace: 1,
            snapshot: Kept(vec![1, 2, 3]),
        });
        // Window 3 not back, window 4 new: a partial restore keeps the snapshot.
        let MemoryAnswer::Restore(plan) = ask(&mut m, None, &[2, 4, 1, 2]) else {
            panic!()
        };
        assert_eq!(
            (plan.remove, plan.ordered, plan.add, plan.spent),
            (vec![3], vec![2, 4, 1], vec![4], false)
        );
        assert!(m.get(1).is_some());
        // A floating, absent or foreign window is not taken back.
        let current = Current::Laid(vec![
            candidate(1),
            Candidate { floating: true, ..candidate(2) },
            Candidate { present: false, ..candidate(3) },
            Candidate {
                in_workspace: false,
                ..candidate(5)
            },
        ]);
        let answer = m.step(MemoryEvent::RestoreAsked {
            workspace: 1,
            returning: None,
            current,
        });
        let MemoryAnswer::Restore(plan) = answer else { panic!() };
        assert_eq!((plan.remove, plan.ordered), (vec![2, 3], vec![1]));
        // None of its windows back, or not laid out: nothing.
        assert_eq!(ask(&mut m, None, &[7]), MemoryAnswer::Nothing);
        let answer = m.step(MemoryEvent::RestoreAsked {
            workspace: 1,
            returning: None,
            current: Current::Unlaid,
        });
        assert_eq!(answer, MemoryAnswer::Nothing);
        // A returning window it does not hold: nothing.
        assert_eq!(ask(&mut m, Some(7), &[1, 2, 3]), MemoryAnswer::Nothing);
        let MemoryAnswer::Restore(plan) = ask(&mut m, Some(3), &[1, 2, 3]) else {
            panic!()
        };
        assert!(plan.spent && m.is_empty());
    }

    #[test]
    fn a_restore_during_a_batch_waits_for_its_end() {
        let mut m = Memory::default();
        m.step(MemoryEvent::Kept {
            workspace: 1,
            snapshot: Kept(vec![1]),
        });
        m.step(MemoryEvent::ReconcileBegan);
        assert_eq!(ask(&mut m, None, &[1]), MemoryAnswer::Deferred);
        assert_eq!(
            m.step(MemoryEvent::ReconcileEnded),
            MemoryAnswer::Waited(vec![1])
        );
        assert_eq!(*m.batch(), Batch::Idle);
        assert!(matches!(ask(&mut m, None, &[1]), MemoryAnswer::Restore(_)));
    }

    #[test]
    fn a_forgotten_workspace_no_longer_waits_for_the_batch() {
        let mut m = Memory::default();
        m.step(MemoryEvent::Kept {
            workspace: 1,
            snapshot: Kept(vec![1]),
        });
        m.step(MemoryEvent::ReconcileBegan);
        assert_eq!(ask(&mut m, None, &[1]), MemoryAnswer::Deferred);
        m.step(MemoryEvent::Forgotten(1));
        assert_eq!(m.step(MemoryEvent::ReconcileEnded), MemoryAnswer::Waited(vec![]));
    }

    #[test]
    fn an_edited_snapshot_holding_nothing_is_dropped() {
        let mut m = Memory::default();
        m.step(MemoryEvent::Kept {
            workspace: 1,
            snapshot: Kept(vec![1, 2]),
        });
        m.step(MemoryEvent::Edited {
            workspace: 1,
            snapshot: Kept(vec![2]),
        });
        assert_eq!(m.get(1), Some(&Kept(vec![2])));
        m.step(MemoryEvent::Edited {
            workspace: 1,
            snapshot: Kept(vec![]),
        });
        assert!(m.is_empty());
        m.step(MemoryEvent::Kept {
            workspace: 1,
            snapshot: Kept(vec![1]),
        });
        m.step(MemoryEvent::Kept {
            workspace: 2,
            snapshot: Kept(vec![2]),
        });
        m.step(MemoryEvent::Forgotten(1));
        assert_eq!(m.snapshots().map(|(w, _)| w).collect::<Vec<_>>(), [2]);
        m.step(MemoryEvent::Cleared);
        assert!(m.is_empty());
    }
}
