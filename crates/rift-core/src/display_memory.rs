//! What the layout engine remembers of displays (engine slice E3): the display each Space shows
//! on, and the Space each display last showed. The second outlives the first: it is how a saved
//! Space is found again on a display whose Spaces macOS renumbered.
//!
//! The layout engine steps this machine with what the topology reports and reads its maps back;
//! it was the engine's `space_display_map` and `display_last_space`.

use std::collections::BTreeMap;

/// Whether a display's last Space follows the Space it now shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastSpace {
    /// The display's last Space is now the Space shown.
    Follow,
    /// The display's last Space is a saved Space not resolved yet: it is still the evidence that
    /// resolves it, so it stays.
    Hold,
}

/// What the layout engine learns of a Space's display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayEvent<S, D> {
    /// `space` shows on `display`.
    Shown {
        space: S,
        display: D,
        last: LastSpace,
    },
    /// `space` shows on no known display.
    Hidden(S),
    /// `from` is replaced by `to`: its display, and every display whose last Space it was, follow.
    Remapped { from: S, to: S },
    /// `space` is gone: its display, and every display's memory of it, go.
    Forgotten(S),
}

/// Each Space's display and each display's last Space.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayMemory<S, D> {
    display_of: BTreeMap<S, D>,
    last_space: BTreeMap<D, S>,
}

impl<S, D> Default for DisplayMemory<S, D> {
    fn default() -> Self {
        DisplayMemory {
            display_of: BTreeMap::new(),
            last_space: BTreeMap::new(),
        }
    }
}

impl<S: Ord + Copy, D: Ord + Clone> DisplayMemory<S, D> {
    /// A machine holding `display_of` and `last_space`.
    pub fn from_parts(
        display_of: impl IntoIterator<Item = (S, D)>,
        last_space: impl IntoIterator<Item = (D, S)>,
    ) -> Self {
        DisplayMemory {
            display_of: display_of.into_iter().collect(),
            last_space: last_space.into_iter().collect(),
        }
    }

    /// One event; whether either map changed.
    pub fn step(&mut self, event: DisplayEvent<S, D>) -> bool {
        match event {
            DisplayEvent::Shown { space, display, last } => {
                let shown = self.display_of.get(&space) != Some(&display);
                let followed = match last {
                    LastSpace::Follow => {
                        self.last_space.insert(display.clone(), space) != Some(space)
                    }
                    LastSpace::Hold => false,
                };
                self.display_of.insert(space, display);
                shown || followed
            }
            DisplayEvent::Hidden(space) => self.display_of.remove(&space).is_some(),
            DisplayEvent::Remapped { from, to } if from == to => false,
            DisplayEvent::Remapped { from, to } => {
                let moved = match self.display_of.remove(&from) {
                    Some(display) => {
                        self.display_of.insert(to, display);
                        true
                    }
                    None => false,
                };
                let mut followed = false;
                for space in self.last_space.values_mut().filter(|space| **space == from) {
                    *space = to;
                    followed = true;
                }
                moved || followed
            }
            DisplayEvent::Forgotten(space) => {
                let shown = self.display_of.remove(&space).is_some();
                let before = self.last_space.len();
                self.last_space.retain(|_, last| *last != space);
                shown || self.last_space.len() != before
            }
        }
    }

    /// The display `space` shows on, if known.
    pub fn display_of(&self, space: S) -> Option<&D> { self.display_of.get(&space) }

    /// The Space `display` last showed, if any.
    pub fn last_space<Q>(&self, display: &Q) -> Option<S>
    where
        D: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized, {
        self.last_space.get(display).copied()
    }

    /// Every Space's display, in Space order.
    pub fn displays(&self) -> impl Iterator<Item = (S, &D)> + '_ {
        self.display_of.iter().map(|(&space, display)| (space, display))
    }

    /// Every display's last Space, in display order.
    pub fn last_spaces(&self) -> impl Iterator<Item = (&D, S)> + '_ {
        self.last_space.iter().map(|(display, &space)| (display, space))
    }
}

/// Whether a workspace on a Space that a display now shows has a home display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Homed {
    /// It has one: it keeps it.
    Homed,
    /// It has none: it takes this display as its home.
    Homeless,
}

/// The workspaces of `on_space` that take the display showing it as their home: those without
/// one, in order.
pub fn rehomed<K: Copy>(on_space: &[(K, Homed)]) -> Vec<K> {
    on_space
        .iter()
        .filter(|(_, homed)| *homed == Homed::Homeless)
        .map(|&(id, _)| id)
        .collect()
}

/// Whether a Space's scrolling widths are synced again after a display update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resync {
    /// The display memory changed, or a workspace took the display as its home.
    Resync,
    Unchanged,
}

/// Whether a display update resyncs its Space's widths: `shown`, the display memory changed;
/// `rehomed`, how many workspaces took the display as their home.
pub fn resync(shown: bool, rehomed: usize) -> Resync {
    if shown || rehomed > 0 {
        Resync::Resync
    } else {
        Resync::Unchanged
    }
}

/// The layout engine's display and departure choices, as a question (the wire form a Roc core
/// answers): workspaces `K` and Spaces `S`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayQuestion<K, S> {
    Rehomed(RehomedAsk<K>),
    Resync(ResyncAsk),
    Move(MoveAsk<S>),
}

/// A display or departure choice's answer, in its wire form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayChoice<K> {
    Rehomed(Vec<K>),
    Resync(Resync),
    Move(crate::native_memory::Move),
}

/// [`rehomed`]'s facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RehomedAsk<K>(pub Vec<(K, Homed)>);

/// [`resync`]'s facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResyncAsk {
    pub shown: bool,
    pub rehomed: usize,
}

/// [`crate::native_memory::native_move`]'s facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveAsk<S> {
    pub origin: crate::native_memory::Origin<S>,
    pub target: S,
}

/// The answer to `question` (the wire form's), from the functions above.
pub fn choose_display<K: Copy, S: PartialEq>(question: DisplayQuestion<K, S>) -> DisplayChoice<K> {
    match question {
        DisplayQuestion::Rehomed(ask) => DisplayChoice::Rehomed(rehomed(&ask.0)),
        DisplayQuestion::Resync(ask) => DisplayChoice::Resync(resync(ask.shown, ask.rehomed)),
        DisplayQuestion::Move(ask) => {
            DisplayChoice::Move(crate::native_memory::native_move(ask.origin, ask.target))
        }
    }
}

impl<K: Copy, S: Copy> DisplayQuestion<K, S> {
    /// The question with its workspaces and Spaces renamed.
    pub fn map<K2, S2>(self, k: impl Fn(K) -> K2, s: impl Fn(S) -> S2) -> DisplayQuestion<K2, S2> {
        use crate::native_memory::Origin;
        match self {
            DisplayQuestion::Rehomed(ask) => DisplayQuestion::Rehomed(RehomedAsk(
                ask.0.into_iter().map(|(id, homed)| (k(id), homed)).collect(),
            )),
            DisplayQuestion::Resync(ask) => DisplayQuestion::Resync(ask),
            DisplayQuestion::Move(ask) => DisplayQuestion::Move(MoveAsk {
                origin: match ask.origin {
                    Origin::Unassigned => Origin::Unassigned,
                    Origin::On(space) => Origin::On(s(space)),
                },
                target: s(ask.target),
            }),
        }
    }
}

impl<K> DisplayChoice<K> {
    /// The answer with its workspaces renamed; none where a word names nothing.
    pub fn map<K2>(self, k: impl Fn(K) -> Option<K2>) -> Option<DisplayChoice<K2>> {
        Some(match self {
            DisplayChoice::Rehomed(ids) => {
                DisplayChoice::Rehomed(ids.into_iter().map(k).collect::<Option<_>>()?)
            }
            DisplayChoice::Resync(r) => DisplayChoice::Resync(r),
            DisplayChoice::Move(m) => DisplayChoice::Move(m),
        })
    }
}

/// One kind of display question with its own answer: callers see `Answer`, never a
/// [`DisplayChoice`]; `of` takes only a choice of its own kind (any other: a twin's bug).
pub trait DisplayAsk<K, S> {
    type Answer: PartialEq + std::fmt::Debug;
    /// The Rust core's answer.
    fn answer(&self) -> Self::Answer;
    /// The question in its wire form.
    fn question(&self) -> DisplayQuestion<K, S>;
    /// An answer in its wire form.
    fn choice(answer: &Self::Answer) -> DisplayChoice<K>;
    /// A wire answer as this kind's answer.
    fn of(&self, choice: DisplayChoice<K>) -> Option<Self::Answer>;
}

impl<K: Copy + PartialEq + std::fmt::Debug, S> DisplayAsk<K, S> for RehomedAsk<K> {
    type Answer = Vec<K>;

    fn answer(&self) -> Vec<K> { rehomed(&self.0) }

    fn question(&self) -> DisplayQuestion<K, S> { DisplayQuestion::Rehomed(self.clone()) }

    fn choice(answer: &Vec<K>) -> DisplayChoice<K> { DisplayChoice::Rehomed(answer.clone()) }

    fn of(&self, choice: DisplayChoice<K>) -> Option<Vec<K>> {
        match choice {
            DisplayChoice::Rehomed(ids) => Some(ids),
            DisplayChoice::Resync(_) | DisplayChoice::Move(_) => None,
        }
    }
}

impl<K, S> DisplayAsk<K, S> for ResyncAsk {
    type Answer = Resync;

    fn answer(&self) -> Resync { resync(self.shown, self.rehomed) }

    fn question(&self) -> DisplayQuestion<K, S> { DisplayQuestion::Resync(*self) }

    fn choice(answer: &Resync) -> DisplayChoice<K> { DisplayChoice::Resync(*answer) }

    fn of(&self, choice: DisplayChoice<K>) -> Option<Resync> {
        match choice {
            DisplayChoice::Resync(r) => Some(r),
            DisplayChoice::Rehomed(_) | DisplayChoice::Move(_) => None,
        }
    }
}

impl<K, S: Copy + PartialEq> DisplayAsk<K, S> for MoveAsk<S> {
    type Answer = crate::native_memory::Move;

    fn answer(&self) -> crate::native_memory::Move {
        crate::native_memory::native_move(self.origin, self.target)
    }

    fn question(&self) -> DisplayQuestion<K, S> { DisplayQuestion::Move(*self) }

    fn choice(answer: &crate::native_memory::Move) -> DisplayChoice<K> {
        DisplayChoice::Move(*answer)
    }

    fn of(&self, choice: DisplayChoice<K>) -> Option<crate::native_memory::Move> {
        match choice {
            DisplayChoice::Move(m) => Some(m),
            DisplayChoice::Rehomed(_) | DisplayChoice::Resync(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shown_display_rehomes_only_homeless_workspaces_and_resyncs_on_any_change() {
        use super::{Homed, Resync, rehomed, resync};
        assert_eq!(
            rehomed(&[
                (1, Homed::Homed),
                (2, Homed::Homeless),
                (3, Homed::Homeless)
            ]),
            vec![2, 3]
        );
        assert_eq!(rehomed::<u8>(&[(1, Homed::Homed)]), Vec::<u8>::new());
        assert_eq!(resync(true, 0), Resync::Resync);
        assert_eq!(resync(false, 2), Resync::Resync);
        assert_eq!(resync(false, 0), Resync::Unchanged);
    }

    type Machine = DisplayMemory<u32, &'static str>;

    fn shown(
        space: u32,
        display: &'static str,
        last: LastSpace,
    ) -> DisplayEvent<u32, &'static str> {
        DisplayEvent::Shown { space, display, last }
    }

    #[test]
    fn a_shown_space_records_its_display_and_the_displays_last_space() {
        let mut m = Machine::default();
        assert!(m.step(shown(1, "a", LastSpace::Follow)));
        assert_eq!((m.display_of(1), m.last_space(&"a")), (Some(&"a"), Some(1)));
        assert!(!m.step(shown(1, "a", LastSpace::Follow)));
        // Another Space on the display becomes its last.
        assert!(m.step(shown(2, "a", LastSpace::Follow)));
        assert_eq!(m.last_space(&"a"), Some(2));
        // A hidden Space loses its display; the display keeps its last Space.
        assert!(m.step(DisplayEvent::Hidden(2)));
        assert_eq!((m.display_of(2), m.last_space(&"a")), (None, Some(2)));
        assert!(!m.step(DisplayEvent::Hidden(2)));
    }

    #[test]
    fn a_held_last_space_stays_while_the_shown_space_is_recorded() {
        let mut m = Machine::default();
        m.step(shown(1, "a", LastSpace::Follow));
        assert!(m.step(shown(5, "a", LastSpace::Hold)));
        assert_eq!((m.display_of(5), m.last_space(&"a")), (Some(&"a"), Some(1)));
        assert!(!m.step(shown(5, "a", LastSpace::Hold)));
        // Held on a display with no last Space, none is recorded.
        m.step(shown(6, "b", LastSpace::Hold));
        assert_eq!(m.last_space(&"b"), None);
    }

    #[test]
    fn a_remapped_space_takes_its_display_and_last_space_entries() {
        let mut m = Machine::default();
        m.step(shown(1, "a", LastSpace::Follow));
        m.step(shown(2, "b", LastSpace::Follow));
        m.step(shown(1, "b", LastSpace::Hold));
        assert!(m.step(DisplayEvent::Remapped { from: 1, to: 9 }));
        assert_eq!((m.display_of(1), m.display_of(9)), (None, Some(&"b")));
        assert_eq!((m.last_space(&"a"), m.last_space(&"b")), (Some(9), Some(2)));
        assert!(!m.step(DisplayEvent::Remapped { from: 9, to: 9 }));
        assert!(!m.step(DisplayEvent::Remapped { from: 1, to: 7 }));
        // A remap onto a Space with a display, from one without, leaves it.
        m.step(shown(3, "c", LastSpace::Hold));
        assert!(!m.step(DisplayEvent::Remapped { from: 4, to: 3 }));
        assert_eq!(m.display_of(3), Some(&"c"));
    }

    #[test]
    fn a_forgotten_space_leaves_no_display_and_no_last_space() {
        let mut m = Machine::default();
        m.step(shown(1, "a", LastSpace::Follow));
        m.step(shown(2, "a", LastSpace::Hold));
        m.step(shown(1, "b", LastSpace::Follow));
        assert!(m.step(DisplayEvent::Forgotten(1)));
        assert_eq!(m.display_of(1), None);
        assert_eq!((m.last_space(&"a"), m.last_space(&"b")), (None, None));
        assert_eq!(m.display_of(2), Some(&"a"));
        assert!(!m.step(DisplayEvent::Forgotten(1)));
    }
}
