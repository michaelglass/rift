//! A slot table with slotmap's (1.1) key, version and free-list rules: ids are allocated as a
//! `SlotMap` allocates them, so a core value keyed by it hands out the ids the platform's slotmap
//! did for the same sequence of inserts and removals, and its saved slot sequence (vacant slots
//! and their versions included, which slotmap does not expose) is slotmap's. The rules are
//! slotmap's, copied exactly; the platform's differential holds the table to a real `SlotMap`.

/// A key made of a slot index and a version (slotmap's `KeyData`, as its ffi form splits it).
pub trait SlotKey: Copy {
    fn from_parts(index: u32, version: u32) -> Self;
    fn parts(self) -> (u32, u32);
}

/// One slot: a value under an odd version, or vacant under an even one.
#[derive(Debug, Clone, PartialEq)]
enum Slot<V> {
    Occupied { version: u32, value: V },
    Vacant { version: u32 },
}

/// Why a saved slot sequence can't be read (slotmap's own refusals, with its messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotsError {
    TooMany,
    FirstNotEmpty,
    InconsistentOccupation,
    /// A free list that is not the table's vacant slots (not one of slotmap's refusals).
    NotTheVacantSlots,
}

impl std::fmt::Display for SlotsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SlotsError::TooMany => "too many slots",
            SlotsError::FirstNotEmpty => "first slot not empty",
            SlotsError::InconsistentOccupation => "inconsistent occupation in Slot",
            SlotsError::NotTheVacantSlots => "a free list that is not the vacant slots",
        })
    }
}

/// The version a vacant slot's next occupant gets.
fn occupied_version(vacant: u32) -> u32 { vacant | 1 }

/// The version a slot gets when its occupant is removed.
fn vacated_version(occupied: u32) -> u32 { occupied.wrapping_add(1) }

/// The key of slot `index` under `version`.
fn key<K: SlotKey>(index: usize, version: u32) -> K { K::from_parts(index as u32, version) }

/// A key's slot index and version.
fn parts<K: SlotKey>(key: K) -> (usize, u32) {
    let (index, version) = key.parts();
    (index as usize, version)
}

/// The free list of loaded slots, as slotmap rebuilds it: every vacant slot but the sentinel,
/// the last (the next taken) the highest.
fn free_list<V>(slots: &[Slot<V>]) -> Vec<u32> {
    (1..slots.len())
        .filter(|&index| matches!(slots[index], Slot::Vacant { .. }))
        .map(|index| index as u32)
        .collect()
}

/// Values by key, in slots. Slot 0 is slotmap's sentinel: always vacant, never handed out.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotTable<K, V> {
    slots: Vec<Slot<V>>,
    /// The vacant slots in the order they are taken: the last one first.
    free: Vec<u32>,
    _key: std::marker::PhantomData<K>,
}

impl<K: SlotKey, V> Default for SlotTable<K, V> {
    fn default() -> Self {
        SlotTable {
            slots: vec![Slot::Vacant { version: 0 }],
            free: Vec::new(),
            _key: std::marker::PhantomData,
        }
    }
}

impl<K: SlotKey, V> SlotTable<K, V> {
    /// Stores `value` in the most recently vacated slot, or a new one.
    pub fn insert(&mut self, value: V) -> K {
        match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                let version = match *slot {
                    Slot::Vacant { version } => occupied_version(version),
                    Slot::Occupied { .. } => unreachable!("the free list holds vacant slots"),
                };
                *slot = Slot::Occupied { version, value };
                key(index as usize, version)
            }
            None => {
                assert!(self.slots.len() < u32::MAX as usize, "SlotMap is full");
                let index = self.slots.len();
                self.slots.push(Slot::Occupied { version: 1, value });
                key(index, 1)
            }
        }
    }

    /// Takes `key`'s value out; its slot is the next one taken.
    pub fn remove(&mut self, key: K) -> Option<V> {
        let (index, version) = parts(key);
        let slot = self.slots.get_mut(index)?;
        if !matches!(*slot, Slot::Occupied { version: held, .. } if held == version) {
            return None;
        }
        let vacated = Slot::Vacant {
            version: vacated_version(version),
        };
        let Slot::Occupied { value, .. } = std::mem::replace(slot, vacated) else {
            unreachable!("checked occupied");
        };
        self.free.push(index as u32);
        Some(value)
    }

    /// The vacant slots in the order they are taken: the last one first.
    pub fn free(&self) -> &[u32] { &self.free }

    pub fn get(&self, key: K) -> Option<&V> {
        let (index, version) = parts(key);
        match self.slots.get(index)? {
            Slot::Occupied { version: held, value } if *held == version => Some(value),
            Slot::Occupied { .. } | Slot::Vacant { .. } => None,
        }
    }

    pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
        let (index, version) = parts(key);
        match self.slots.get_mut(index)? {
            Slot::Occupied { version: held, value } if *held == version => Some(value),
            Slot::Occupied { .. } | Slot::Vacant { .. } => None,
        }
    }

    pub fn contains_key(&self, key: K) -> bool { self.get(key).is_some() }

    pub fn len(&self) -> usize { self.iter().count() }

    pub fn is_empty(&self) -> bool { self.iter().next().is_none() }

    /// Every key and value, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (K, &V)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| match slot {
            Slot::Occupied { version, value } => Some((key(index, *version), value)),
            Slot::Vacant { .. } => None,
        })
    }

    /// Every key and value, in slot order, for writing.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (K, &mut V)> {
        self.slots.iter_mut().enumerate().filter_map(|(index, slot)| match slot {
            Slot::Occupied { version, value } => Some((key(index, *version), value)),
            Slot::Vacant { .. } => None,
        })
    }

    pub fn keys(&self) -> impl Iterator<Item = K> + '_ { self.iter().map(|(key, _)| key) }

    pub fn values(&self) -> impl Iterator<Item = &V> { self.iter().map(|(_, value)| value) }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.iter_mut().map(|(_, value)| value)
    }

    /// The saved slot sequence, slot by slot: each occupied slot's key and value, or none, and the
    /// version (slotmap's `SerdeSlot` fields; the platform writes them).
    pub fn saved(&self) -> impl Iterator<Item = (Option<(K, &V)>, u32)> {
        self.slots.iter().enumerate().map(|(index, slot)| match slot {
            Slot::Occupied { version, value } => (Some((key(index, *version), value)), *version),
            Slot::Vacant { version } => (None, *version),
        })
    }

    /// A saved slot sequence, refused as slotmap refuses it.
    pub fn parse(saved: Vec<(Option<V>, u32)>) -> Result<Self, SlotsError> {
        if saved.len() >= u32::MAX as usize {
            return Err(SlotsError::TooMany);
        }
        let mut slots = Vec::with_capacity(saved.len());
        for (value, version) in saved {
            slots.push(match (value, version % 2 == 1) {
                (Some(value), true) => Slot::Occupied { version, value },
                (None, false) => Slot::Vacant { version },
                (Some(_), false) | (None, true) => return Err(SlotsError::InconsistentOccupation),
            });
        }
        match slots.first_mut() {
            // The sentinel is held (and written) as slotmap holds it, whatever version it was
            // saved with.
            Some(first @ Slot::Vacant { .. }) => *first = Slot::Vacant { version: 0 },
            Some(Slot::Occupied { .. }) | None => return Err(SlotsError::FirstNotEmpty),
        }
        Ok(SlotTable {
            free: free_list(&slots),
            slots,
            _key: std::marker::PhantomData,
        })
    }

    /// The same slots, their vacant ones taken in `order`, the first taken first: a live
    /// table's free list, which a load does not rebuild (a load takes the highest first).
    /// Refused unless `order` names every vacant slot but the sentinel once.
    pub fn resumed(mut self, order: &[u32]) -> Result<Self, SlotsError> {
        let mut taken: Vec<u32> = order.to_vec();
        taken.sort_unstable();
        if taken != free_list(&self.slots) {
            return Err(SlotsError::NotTheVacantSlots);
        }
        self.free = order.iter().rev().copied().collect();
        Ok(self)
    }

    /// The same slots, each value made into another of its key and value.
    pub fn map<U, E>(
        self,
        mut change: impl FnMut(K, V) -> Result<U, E>,
    ) -> Result<SlotTable<K, U>, E> {
        let mut slots = Vec::with_capacity(self.slots.len());
        for (index, slot) in self.slots.into_iter().enumerate() {
            slots.push(match slot {
                Slot::Occupied { version, value } => Slot::Occupied {
                    version,
                    value: change(key(index, version), value)?,
                },
                Slot::Vacant { version } => Slot::Vacant { version },
            });
        }
        Ok(SlotTable {
            slots,
            free: self.free,
            _key: std::marker::PhantomData,
        })
    }
}

impl<K: SlotKey, V> std::ops::Index<K> for SlotTable<K, V> {
    type Output = V;

    fn index(&self, key: K) -> &V { self.get(key).expect("invalid SlotMap key used") }
}

impl<K: SlotKey, V> std::ops::IndexMut<K> for SlotTable<K, V> {
    fn index_mut(&mut self, key: K) -> &mut V {
        self.get_mut(key).expect("invalid SlotMap key used")
    }
}
