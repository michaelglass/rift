//! A slot table with slotmap's (1.1) key, version and free-list rules, which owns its saved form.
//!
//! The workspace catalog writes each workspace with facts the table does not hold (its Space is
//! the core's), so it writes the slot sequence itself: slotmap's format, a sequence of `(value,
//! version)` slots with the vacant ones included, whose versions slotmap does not expose. The
//! rules are slotmap's, copied exactly, so ids are allocated and files written as before; the
//! differential tests below hold the table to a real `SlotMap` for both.

use serde::{Deserialize, Serialize};
use slotmap::{Key, KeyData};

/// One slot: a value under an odd version, or vacant under an even one.
#[derive(Debug, Clone, PartialEq)]
enum Slot<V> {
    Occupied { version: u32, value: V },
    Vacant { version: u32 },
}

/// A slot as it is saved (slotmap's `SerdeSlot`): the value if occupied, and the version.
#[derive(Serialize, Deserialize)]
#[serde(rename = "SerdeSlot")]
pub(crate) struct SavedSlot<T> {
    value: Option<T>,
    version: u32,
}

/// Why a saved slot sequence can't be read (slotmap's own refusals, with its messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotsError {
    TooMany,
    FirstNotEmpty,
    InconsistentOccupation,
}

impl std::fmt::Display for SlotsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SlotsError::TooMany => "too many slots",
            SlotsError::FirstNotEmpty => "first slot not empty",
            SlotsError::InconsistentOccupation => "inconsistent occupation in Slot",
        })
    }
}

/// The version a vacant slot's next occupant gets.
fn occupied_version(vacant: u32) -> u32 { vacant | 1 }

/// The version a slot gets when its occupant is removed.
fn vacated_version(occupied: u32) -> u32 { occupied.wrapping_add(1) }

/// The key of slot `index` under `version`.
fn key<K: Key>(index: usize, version: u32) -> K {
    KeyData::from_ffi((u64::from(version) << 32) | index as u64).into()
}

/// A key's slot index and version.
fn parts<K: Key>(key: K) -> (usize, u32) {
    let ffi = key.data().as_ffi();
    ((ffi & 0xffff_ffff) as usize, (ffi >> 32) as u32)
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
pub(crate) struct SlotTable<K, V> {
    slots: Vec<Slot<V>>,
    /// The vacant slots in the order they are taken: the last one first.
    free: Vec<u32>,
    _key: std::marker::PhantomData<K>,
}

impl<K: Key, V> Default for SlotTable<K, V> {
    fn default() -> Self {
        SlotTable {
            slots: vec![Slot::Vacant { version: 0 }],
            free: Vec::new(),
            _key: std::marker::PhantomData,
        }
    }
}

impl<K: Key, V> SlotTable<K, V> {
    /// Stores `value` in the most recently vacated slot, or a new one.
    pub(crate) fn insert(&mut self, value: V) -> K {
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
    pub(crate) fn remove(&mut self, key: K) -> Option<V> {
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

    pub(crate) fn get(&self, key: K) -> Option<&V> {
        let (index, version) = parts(key);
        match self.slots.get(index)? {
            Slot::Occupied { version: held, value } if *held == version => Some(value),
            Slot::Occupied { .. } | Slot::Vacant { .. } => None,
        }
    }

    pub(crate) fn get_mut(&mut self, key: K) -> Option<&mut V> {
        let (index, version) = parts(key);
        match self.slots.get_mut(index)? {
            Slot::Occupied { version: held, value } if *held == version => Some(value),
            Slot::Occupied { .. } | Slot::Vacant { .. } => None,
        }
    }

    pub(crate) fn contains_key(&self, key: K) -> bool { self.get(key).is_some() }

    pub(crate) fn len(&self) -> usize { self.iter().count() }

    /// Every key and value, in slot order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (K, &V)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| match slot {
            Slot::Occupied { version, value } => Some((key(index, *version), value)),
            Slot::Vacant { .. } => None,
        })
    }

    /// Every key and value, in slot order, for writing.
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = (K, &mut V)> {
        self.slots.iter_mut().enumerate().filter_map(|(index, slot)| match slot {
            Slot::Occupied { version, value } => Some((key(index, *version), value)),
            Slot::Vacant { .. } => None,
        })
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = K> + '_ { self.iter().map(|(key, _)| key) }

    pub(crate) fn values(&self) -> impl Iterator<Item = &V> { self.iter().map(|(_, value)| value) }

    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.iter_mut().map(|(_, value)| value)
    }

    /// The saved slot sequence, each value written as `write` makes it of its key and value.
    pub(crate) fn saved<'a, T: Serialize + 'a>(
        &'a self,
        write: impl Fn(K, &'a V) -> T + 'a,
    ) -> impl Iterator<Item = SavedSlot<T>> + 'a {
        self.slots.iter().enumerate().map(move |(index, slot)| match slot {
            Slot::Occupied { version, value } => SavedSlot {
                value: Some(write(key(index, *version), value)),
                version: *version,
            },
            Slot::Vacant { version } => SavedSlot { value: None, version: *version },
        })
    }

    /// A saved slot sequence, refused as slotmap refuses it.
    pub(crate) fn parse(saved: Vec<SavedSlot<V>>) -> Result<Self, SlotsError> {
        if saved.len() >= u32::MAX as usize {
            return Err(SlotsError::TooMany);
        }
        let mut slots = Vec::with_capacity(saved.len());
        for SavedSlot { value, version } in saved {
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

    /// The same slots, each value made into another of its key and value.
    pub(crate) fn map<U, E>(
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

impl<K: Key, V> std::ops::Index<K> for SlotTable<K, V> {
    type Output = V;

    fn index(&self, key: K) -> &V { self.get(key).expect("invalid SlotMap key used") }
}

impl<K: Key, V> std::ops::IndexMut<K> for SlotTable<K, V> {
    fn index_mut(&mut self, key: K) -> &mut V {
        self.get_mut(key).expect("invalid SlotMap key used")
    }
}

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;

    use super::*;

    slotmap::new_key_type! {
        struct Id;
    }

    /// The table's saved slots, as the catalog writes them (each value as itself).
    fn written(table: &SlotTable<Id, u32>) -> String {
        let slots: Vec<_> = table.saved(|_, &value| value).collect();
        ron::to_string(&slots).unwrap()
    }

    fn read(file: &str) -> Result<SlotTable<Id, u32>, String> {
        let slots: Vec<SavedSlot<u32>> = ron::from_str(file).map_err(|error| error.to_string())?;
        SlotTable::parse(slots).map_err(|error| error.to_string())
    }

    /// A sequence of inserts and removals (biased toward reusing ids), run on both, with a save
    /// and a load (each read from the other's file) between some steps.
    fn run(seed: u64, steps: usize) {
        let mut state = seed | 1;
        let mut draw = |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        let mut table = SlotTable::<Id, u32>::default();
        let mut map = SlotMap::<Id, u32>::with_key();
        let mut keys: Vec<Id> = Vec::new();
        for step in 0..steps {
            match draw(10) {
                0..=3 => {
                    let value = step as u32;
                    let (ours, theirs) = (table.insert(value), map.insert(value));
                    assert_eq!(ours, theirs, "seed {seed} step {step}: insert's id");
                    keys.push(ours);
                }
                4..=7 if !keys.is_empty() => {
                    // A stale key now and then: removing it again must find nothing in both.
                    let key = keys[draw(keys.len() as u64) as usize];
                    assert_eq!(table.remove(key), map.remove(key), "seed {seed} step {step}");
                }
                8 => {
                    let file = written(&table);
                    assert_eq!(file, ron::to_string(&map).unwrap(), "seed {seed} step {step}");
                    // Each loads the other's file.
                    table = read(&file).unwrap();
                    map = ron::from_str(&file).unwrap();
                }
                _ => {
                    let key = keys.get(draw(keys.len().max(1) as u64) as usize).copied();
                    if let Some(key) = key {
                        assert_eq!(table.get(key), map.get(key), "seed {seed} step {step}");
                    }
                }
            }
            assert_eq!(table.len(), map.len(), "seed {seed} step {step}");
            assert!(
                table.iter().eq(map.iter()),
                "seed {seed} step {step}: slot order"
            );
            assert!(table.keys().eq(map.keys()), "seed {seed} step {step}");
            assert!(table.values().eq(map.values()), "seed {seed} step {step}");
            for &key in &keys {
                assert_eq!(table.contains_key(key), map.contains_key(key), "seed {seed}");
            }
            if step % 50 == 49 {
                // Writing through each walks the same slots.
                table.values_mut().for_each(|value| *value += 1);
                map.values_mut().for_each(|value| *value += 1);
                for ((ours, value), (theirs, _)) in table.iter_mut().zip(map.iter_mut()) {
                    assert_eq!(ours, theirs, "seed {seed} step {step}");
                    *value -= 1;
                }
                map.values_mut().for_each(|value| *value -= 1);
                // A map keeps every slot and the free list.
                table = table.map(|_, value| Ok::<_, ()>(value)).unwrap();
            }
        }
        assert_eq!(
            written(&table),
            ron::to_string(&map).unwrap(),
            "seed {seed}: the end"
        );
    }

    #[test]
    fn the_table_allocates_and_writes_as_slotmap_does() {
        for seed in 0..500 {
            run(seed, 200);
        }
    }

    #[test]
    fn a_saved_sequence_slotmap_refuses_is_refused_alike() {
        for file in [
            "[]",
            "[(value: Some(1), version: 1)]",
            "[(value: None, version: 0), (value: Some(1), version: 2)]",
            "[(value: None, version: 0), (value: None, version: 3)]",
        ] {
            let theirs = ron::from_str::<SlotMap<Id, u32>>(file).unwrap_err().to_string();
            let ours = read(file).unwrap_err();
            assert!(
                theirs.contains(&ours),
                "{file}: slotmap says {theirs:?}, the table {ours:?}"
            );
        }
        // A sentinel saved with another even version loads, and is written as slotmap writes it.
        let file = "[(value:None,version:4),(value:Some(7),version:3),(value:None,version:6)]";
        let theirs: SlotMap<Id, u32> = ron::from_str(file).unwrap();
        assert_eq!(written(&read(file).unwrap()), ron::to_string(&theirs).unwrap());
    }
}
