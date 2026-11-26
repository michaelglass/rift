//! The platform's side of [`rift_core::slot_table`]: the saved form (slotmap's `SerdeSlot`
//! sequence) and slotmap's keys as the table's keys. The differential tests below hold the core's
//! table to a real `SlotMap`: ids allocated, lookups, slot order and the bytes written.

pub(crate) use rift_core::slot_table::{SlotKey, SlotTable, SlotsError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use slotmap::SlotMap;

/// A slot as it is saved (slotmap's `SerdeSlot`): the value if occupied, and the version.
#[derive(Serialize, Deserialize)]
#[serde(rename = "SerdeSlot")]
pub(crate) struct SavedSlot<T> {
    pub(crate) value: Option<T>,
    pub(crate) version: u32,
}

/// Makes slotmap key types the table's keys: a key's ffi form is `version << 32 | index`.
macro_rules! slot_keys {
    ($($key:ty),* $(,)?) => {$(
        impl rift_core::slot_table::SlotKey for $key {
            fn from_parts(index: u32, version: u32) -> Self {
                slotmap::KeyData::from_ffi((u64::from(version) << 32) | u64::from(index)).into()
            }

            fn parts(self) -> (u32, u32) {
                let ffi = slotmap::Key::data(&self).as_ffi();
                ((ffi & 0xffff_ffff) as u32, (ffi >> 32) as u32)
            }
        }
    )*};
}

slot_keys!(
    crate::model::VirtualWorkspaceId,
    crate::model::tree::NodeId,
    crate::layout_engine::LayoutId,
);

/// The saved slot sequence, each value written as `write` makes it of its key and value.
pub(crate) fn saved<'a, K: SlotKey, V, T: Serialize + 'a>(
    table: &'a SlotTable<K, V>,
    write: impl Fn(K, &'a V) -> T + 'a,
) -> impl Iterator<Item = SavedSlot<T>> + 'a {
    table.saved().map(move |(occupant, version)| SavedSlot {
        value: occupant.map(|(key, value)| write(key, value)),
        version,
    })
}

/// A saved slot sequence, refused as slotmap refuses it.
pub(crate) fn parse<K: SlotKey, V>(
    saved: Vec<SavedSlot<V>>,
) -> Result<SlotTable<K, V>, SlotsError> {
    SlotTable::parse(saved.into_iter().map(|slot| (slot.value, slot.version)).collect())
}

/// A live slotmap as the core's table: its slots, vacant ones and their versions included (read
/// from its saved form), and its free list (the order a copy hands its vacant slots out; a load
/// rebuilds it otherwise).
pub(crate) fn of_slotmap<K: slotmap::Key + SlotKey, V: Clone + Serialize + DeserializeOwned, C>(
    map: &SlotMap<K, V>,
    dummy: V,
    convert: impl Fn(&V) -> C,
) -> SlotTable<K, C> {
    let written = ron::to_string(map).expect("a slotmap is written");
    let saved: Vec<SavedSlot<V>> = ron::from_str(&written).expect("a written slotmap is read");
    let vacant = saved.len() - 1 - map.len();
    let mut copy = map.clone();
    let order: Vec<u32> = (0..vacant).map(|_| copy.insert(dummy.clone()).parts().0).collect();
    parse::<K, V>(saved)
        .expect("a written slotmap is a slot sequence")
        .map(|_, value| Ok::<_, std::convert::Infallible>(convert(&value)))
        .unwrap_or_else(|never| match never {})
        .resumed(&order)
        .expect("a slotmap's free list is its vacant slots")
}

/// A core table as a slotmap with the same slots, versions and free list (a shadow re-seeded
/// from the core), each value written as `write` makes it (in the saved form of `V`). A slotmap
/// is only built through its saved form, whose load rebuilds the free list (the highest vacant
/// slot first), so each vacant slot is written two versions back, then taken (`| 1`) and freed
/// (`+ 1`) again in the core's order: the last freed is taken first, as the core's last.
pub(crate) fn to_slotmap<
    K: slotmap::Key + SlotKey,
    C,
    T: Serialize,
    V: Clone + DeserializeOwned,
>(
    table: &SlotTable<K, C>,
    write: impl Fn(K, &C) -> T,
    dummy: V,
) -> SlotMap<K, V> {
    let slots: Vec<SavedSlot<T>> = table
        .saved()
        .enumerate()
        .map(|(index, (occupant, version))| SavedSlot {
            version: match occupant {
                Some(_) => version,
                // The sentinel, never taken.
                None if index == 0 => version,
                None => version.wrapping_sub(2),
            },
            value: occupant.map(|(key, value)| write(key, value)),
        })
        .collect();
    let written = ron::to_string(&slots).expect("a slot sequence is written");
    let mut map: SlotMap<K, V> = ron::from_str(&written).expect("a core table is a slot sequence");
    let taken: std::collections::HashMap<u32, K> = table
        .free()
        .iter()
        .map(|_| {
            let key = map.insert(dummy.clone());
            (key.parts().0, key)
        })
        .collect();
    for index in table.free() {
        map.remove(taken[index]);
    }
    map
}

#[cfg(test)]
mod tests {
    use slotmap::SlotMap;

    use super::*;

    slotmap::new_key_type! {
        struct Id;
    }
    slot_keys!(Id);

    /// The table's saved slots, as the catalog writes them (each value as itself).
    fn written(table: &SlotTable<Id, u32>) -> String {
        let slots: Vec<_> = saved(table, |_, &value| value).collect();
        ron::to_string(&slots).unwrap()
    }

    fn read(file: &str) -> Result<SlotTable<Id, u32>, String> {
        let slots: Vec<SavedSlot<u32>> = ron::from_str(file).map_err(|error| error.to_string())?;
        parse(slots).map_err(|error| error.to_string())
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
            assert_eq!(table.is_empty(), map.is_empty(), "seed {seed} step {step}");
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
