// SPDX-License-Identifier: GPL-2.0-only

//! Generational entity handles (`SIM-02`, `docs/simulation.md#entity-handles`).
//!
//! Entity stores stay dense and swap-remove in O(1); a [`HandleTable`] beside each store maps a
//! stable [`Handle`] to the entity's current dense index. Freeing a handle bumps its slot's
//! generation, so a handle kept after its entity left resolves to `None` instead of naming
//! whichever entity moved into the slot. Freed slots are reused last-freed first, and the whole
//! table is saved, so a loaded game resolves every handle to the same entity.
//!
//! Stores append entities through their own lifecycle helpers and may also be filled directly
//! (loading, tests). [`HandleTable::register_appended`] gives every untracked trailing entity a
//! handle in dense order, so a table is always brought up to date before it is read or before
//! an entity leaves. The table also buckets live handles by daily phase ([`PhaseBuckets`]), so
//! phase membership changes exactly when a handle is allocated or freed.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use super::phases::PhaseBuckets;

// Marks a free slot in `Slot::dense` and the absent handle's index.
const NONE_INDEX: u32 = u32::MAX;

/// Stable reference to one entity of kind `K`; dense indices are valid within one tick only.
pub(crate) struct Handle<K> {
    index: u32,
    generation: u32,
    kind: PhantomData<fn() -> K>,
}

impl<K> Handle<K> {
    /// The absent handle, like `usize::MAX` for a dense index.
    pub(crate) const NONE: Self = Self::from_parts(NONE_INDEX, 0);

    pub(crate) const fn from_parts(index: u32, generation: u32) -> Self {
        Self {
            index,
            generation,
            kind: PhantomData,
        }
    }

    /// Handle slot, stable for the entity's lifetime and reused after it leaves.
    pub(crate) fn index(self) -> u32 {
        self.index
    }

    /// Times the slot was freed before this entity took it.
    pub(crate) fn generation(self) -> u32 {
        self.generation
    }

    pub(crate) fn is_none(self) -> bool {
        self.index == NONE_INDEX
    }

    /// Packs the handle into one integer, `index << 32 | generation`, for saved event rows.
    pub(crate) fn to_bits(self) -> u64 {
        u64::from(self.index) << 32 | u64::from(self.generation)
    }

    /// Inverse of [`Self::to_bits`].
    pub(crate) fn from_bits(bits: u64) -> Self {
        Self::from_parts((bits >> 32) as u32, bits as u32)
    }
}

// Manual impls: derives would require the marker type itself to implement each trait.
impl<K> Clone for Handle<K> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<K> Copy for Handle<K> {}
impl<K> PartialEq for Handle<K> {
    fn eq(&self, other: &Self) -> bool {
        (self.index, self.generation) == (other.index, other.generation)
    }
}
impl<K> Eq for Handle<K> {}
impl<K> PartialOrd for Handle<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<K> Ord for Handle<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.index, self.generation).cmp(&(other.index, other.generation))
    }
}
impl<K> Hash for Handle<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.index, self.generation).hash(state);
    }
}
impl<K> fmt::Debug for Handle<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_none() {
            f.write_str("Handle(none)")
        } else {
            write!(f, "Handle({}g{})", self.index, self.generation)
        }
    }
}
impl<K> Default for Handle<K> {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    generation: u32,
    // Current dense index, or `NONE_INDEX` while the slot is free.
    dense: u32,
}

/// Handle slots of one dense entity store. Every operation is O(1) amortized.
pub(crate) struct HandleTable<K> {
    slots: Vec<Slot>,
    // Dense index -> handle slot.
    dense_slots: Vec<u32>,
    // Freed slots, reused last first.
    free: Vec<u32>,
    // Live handles by daily phase; derived, rebuilt on restore.
    phases: PhaseBuckets<K>,
    kind: PhantomData<fn() -> K>,
}

impl<K> Clone for HandleTable<K> {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            dense_slots: self.dense_slots.clone(),
            free: self.free.clone(),
            phases: self.phases.clone(),
            kind: PhantomData,
        }
    }
}

impl<K> fmt::Debug for HandleTable<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandleTable")
            .field("slots", &self.slots)
            .field("dense_slots", &self.dense_slots)
            .field("free", &self.free)
            .field("phases", &self.phases)
            .finish()
    }
}

impl<K> Default for HandleTable<K> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            dense_slots: Vec::new(),
            free: Vec::new(),
            phases: PhaseBuckets::default(),
            kind: PhantomData,
        }
    }
}

/// Saved form of a [`HandleTable`]: per slot its generation and dense index (`None` when
/// free), and the free list in reuse order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SavedHandleTable {
    pub(crate) slots: Vec<(u32, Option<u32>)>,
    pub(crate) free: Vec<u32>,
}

impl<K> HandleTable<K> {
    // Takes the last freed slot, or a new one, for an entity at `dense`.
    fn allocate(&mut self, dense: u32) -> u32 {
        let slot = match self.free.pop() {
            Some(slot) => {
                self.slots[slot as usize].dense = dense;
                slot
            }
            None => {
                self.slots.push(Slot {
                    generation: 0,
                    dense,
                });
                (self.slots.len() - 1) as u32
            }
        };
        self.phases.insert(Handle::from_parts(
            slot,
            self.slots[slot as usize].generation,
        ));
        slot
    }

    /// Gives each entity at dense index `self.len()..len` a handle, in dense order.
    pub(crate) fn register_appended(&mut self, len: usize) {
        while self.dense_slots.len() < len {
            let slot = self.allocate(self.dense_slots.len() as u32);
            self.dense_slots.push(slot);
        }
    }

    /// Live handles by daily phase.
    // Maintained from `SIM-02-C`; the first `EntityPhase` system reads it in `SIM-02-F`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn phases(&self) -> &PhaseBuckets<K> {
        &self.phases
    }

    /// Mirrors `store.swap_remove(dense)` on a store of `len` entities before the removal:
    /// frees the removed entity's handle and moves the last entity's handle to `dense`.
    pub(crate) fn swap_remove(&mut self, dense: usize, len: usize) -> Handle<K> {
        self.register_appended(len);
        let slot = self.dense_slots.swap_remove(dense);
        let removed = &mut self.slots[slot as usize];
        let handle = Handle::from_parts(slot, removed.generation);
        self.phases.remove(handle);
        removed.generation = removed.generation.wrapping_add(1);
        removed.dense = NONE_INDEX;
        self.free.push(slot);
        if let Some(&moved) = self.dense_slots.get(dense) {
            self.slots[moved as usize].dense = dense as u32;
        }
        handle
    }

    /// Mirrors re-inserting an entity at `dense` of a store of `len` entities by moving the
    /// current occupant to the end (undo restores entities at their old dense index): the
    /// occupant keeps its handle and the re-inserted entity gets a new one.
    pub(crate) fn insert_displacing(&mut self, dense: usize, len: usize) -> Handle<K> {
        self.register_appended(len);
        if dense == len {
            self.register_appended(len + 1);
            return self.handle(dense);
        }
        let occupant = self.dense_slots[dense];
        self.dense_slots.push(occupant);
        self.slots[occupant as usize].dense = len as u32;
        let slot = self.allocate(dense as u32);
        self.dense_slots[dense] = slot;
        self.handle(dense)
    }

    /// Forgets every handle, for a store that is cleared or replaced wholesale.
    pub(crate) fn clear(&mut self) {
        self.slots.clear();
        self.dense_slots.clear();
        self.free.clear();
        self.phases.clear();
    }

    /// Handle of the entity at `dense`, or [`Handle::NONE`] when it has none yet.
    pub(crate) fn handle(&self, dense: usize) -> Handle<K> {
        self.dense_slots.get(dense).map_or(Handle::NONE, |&slot| {
            Handle::from_parts(slot, self.slots[slot as usize].generation)
        })
    }

    /// Current dense index of `handle`'s entity, or `None` when it has left.
    pub(crate) fn resolve(&self, handle: Handle<K>) -> Option<usize> {
        let slot = self.slots.get(handle.index as usize)?;
        (slot.generation == handle.generation && slot.dense != NONE_INDEX)
            .then_some(slot.dense as usize)
    }

    /// Handle slots, live or free: the capacity a rolling snapshot freezes at its period start.
    pub(crate) fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Entities with a handle: the store's first entities, up to those appended since the last
    /// [`Self::register_appended`].
    pub(crate) fn registered(&self) -> usize {
        self.dense_slots.len()
    }

    /// Dense index of the entity in handle slot `index`, whatever its generation; `None` while
    /// the slot is free or beyond the capacity.
    pub(crate) fn dense_at(&self, index: usize) -> Option<usize> {
        let slot = self.slots.get(index)?;
        (slot.dense != NONE_INDEX).then_some(slot.dense as usize)
    }

    /// Saved form; [`Self::restore`] rebuilds an identical table.
    pub(crate) fn save(&self) -> SavedHandleTable {
        SavedHandleTable {
            slots: self
                .slots
                .iter()
                .map(|slot| {
                    (
                        slot.generation,
                        (slot.dense != NONE_INDEX).then_some(slot.dense),
                    )
                })
                .collect(),
            free: self.free.clone(),
        }
    }

    /// Rebuilds a table for a store of `len` entities, rejecting inconsistent data.
    pub(crate) fn restore(saved: &SavedHandleTable, len: usize) -> Result<Self, String> {
        let mut table = Self {
            dense_slots: vec![NONE_INDEX; len],
            ..Self::default()
        };
        let mut free_slots = 0_usize;
        for (index, &(generation, dense)) in saved.slots.iter().enumerate() {
            let dense = match dense {
                Some(dense) => {
                    let target = table
                        .dense_slots
                        .get_mut(dense as usize)
                        .ok_or_else(|| format!("handle slot {index} names dense index {dense}"))?;
                    if *target != NONE_INDEX {
                        return Err(format!("dense index {dense} has two handle slots"));
                    }
                    *target = index as u32;
                    dense
                }
                None => {
                    free_slots += 1;
                    NONE_INDEX
                }
            };
            table.slots.push(Slot { generation, dense });
        }
        if let Some(dense) = table
            .dense_slots
            .iter()
            .position(|&slot| slot == NONE_INDEX)
        {
            return Err(format!("dense index {dense} has no handle slot"));
        }
        let mut seen = vec![false; table.slots.len()];
        for &slot in &saved.free {
            let free = table
                .slots
                .get(slot as usize)
                .is_some_and(|entry| entry.dense == NONE_INDEX);
            if !free || std::mem::replace(&mut seen[slot as usize], true) {
                return Err(format!("free handle slot {slot} is not free exactly once"));
            }
        }
        if saved.free.len() != free_slots {
            return Err("free handle list does not name every free slot".into());
        }
        table.free = saved.free.clone();
        for (index, slot) in table.slots.iter().enumerate() {
            if slot.dense != NONE_INDEX {
                table
                    .phases
                    .insert(Handle::from_parts(index as u32, slot.generation));
            }
        }
        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Thing;

    // A dense store that removes like the entity stores, labelled by its creation order.
    fn remove(store: &mut Vec<u32>, table: &mut HandleTable<Thing>, dense: usize) -> Handle<Thing> {
        let handle = table.swap_remove(dense, store.len());
        store.swap_remove(dense);
        handle
    }

    #[test]
    fn handles_follow_swap_remove_and_go_stale() {
        let mut store: Vec<u32> = (0..4).collect();
        let mut table = HandleTable::<Thing>::default();
        table.register_appended(store.len());
        let handles: Vec<_> = (0..4).map(|dense| table.handle(dense)).collect();
        let removed = remove(&mut store, &mut table, 1);
        assert_eq!(removed, handles[1]);
        assert_eq!(table.resolve(handles[1]), None);
        for (label, &handle) in handles.iter().enumerate() {
            if label != 1 {
                assert_eq!(store[table.resolve(handle).unwrap()], label as u32);
            }
        }
        // The freed slot is reused with a new generation; the stale handle stays dead.
        store.push(9);
        table.register_appended(store.len());
        let reused = table.handle(store.len() - 1);
        assert_eq!(reused.index(), handles[1].index());
        assert_ne!(reused, handles[1]);
        assert_eq!(table.resolve(handles[1]), None);
        assert_eq!(store[table.resolve(reused).unwrap()], 9);
        // Phase buckets follow allocation and release.
        assert_eq!(table.phases().len(), store.len());
        let mut bucket = Vec::new();
        table.phases().daily(
            crate::simulation::core::phases::daily_phase(reused),
            &mut bucket,
        );
        assert!(bucket.contains(&reused));
    }

    #[test]
    fn removing_the_last_entity_and_untracked_entities_stay_consistent() {
        let mut store: Vec<u32> = (0..3).collect();
        let mut table = HandleTable::<Thing>::default();
        let last = {
            table.register_appended(store.len());
            table.handle(2)
        };
        remove(&mut store, &mut table, 2);
        assert_eq!(table.resolve(last), None);
        // Entities appended directly get handles before one of them leaves.
        store.extend([5, 6]);
        remove(&mut store, &mut table, 0);
        assert_eq!(table.dense_slots.len(), store.len());
        for dense in 0..store.len() {
            assert_eq!(table.resolve(table.handle(dense)), Some(dense));
        }
    }

    #[test]
    fn displacing_insert_keeps_the_occupant_handle() {
        let mut store: Vec<u32> = (0..3).collect();
        let mut table = HandleTable::<Thing>::default();
        table.register_appended(store.len());
        let occupant = table.handle(1);
        let handle = table.insert_displacing(1, store.len());
        let moved = store[1];
        store.push(moved);
        store[1] = 7;
        assert_eq!(store[table.resolve(occupant).unwrap()], moved);
        assert_eq!(store[table.resolve(handle).unwrap()], 7);
        let appended = table.insert_displacing(store.len(), store.len());
        store.push(8);
        assert_eq!(store[table.resolve(appended).unwrap()], 8);
        assert_eq!(table.dense_slots.len(), store.len());
    }

    #[test]
    fn saved_tables_restore_identically_and_reject_corruption() {
        let mut store: Vec<u32> = (0..6).collect();
        let mut table = HandleTable::<Thing>::default();
        table.register_appended(store.len());
        remove(&mut store, &mut table, 4);
        remove(&mut store, &mut table, 0);
        let saved = table.save();
        let restored = HandleTable::<Thing>::restore(&saved, store.len()).unwrap();
        assert_eq!(restored.save(), saved);
        for dense in 0..store.len() {
            assert_eq!(restored.handle(dense), table.handle(dense));
        }
        let mut broken = saved.clone();
        broken.free.pop();
        assert!(HandleTable::<Thing>::restore(&broken, store.len()).is_err());
        assert!(HandleTable::<Thing>::restore(&saved, store.len() + 1).is_err());
    }
}
