// SPDX-License-Identifier: GPL-2.0-only

//! An insertion-ordered hash map with `O(1)` removal, iterating as Godot's `Dictionary` does: a
//! new key goes last, assigning an existing key keeps its place, a removed key that comes back
//! goes last. The graded renderer's port (`RENDER-32-I`) replays the GDScript renderer's traces
//! bit for bit, and wherever the GDScript iterated a dictionary the order of its outputs (page
//! ids, slots, request order) followed that dictionary's order.
//!
//! Entries live in a vector in insertion order, a removal leaving a hole; the index maps a key to
//! its entry. Holes are compacted away once they outnumber the live entries (amortised `O(1)` per
//! removal), and compaction keeps the order.

use std::collections::HashMap;
use std::hash::Hash;

type Index<K> = HashMap<K, usize, foldhash::fast::FixedState>;

/// The map. `K` is small and `Copy` (tile and patch keys).
#[derive(Clone, Debug)]
pub(crate) struct OrderedMap<K, V> {
    entries: Vec<Option<(K, V)>>,
    index: Index<K>,
}

impl<K, V> Default for OrderedMap<K, V> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            index: Index::default(),
        }
    }
}

impl<K: Copy + Eq + Hash, V> OrderedMap<K, V> {
    pub(crate) fn len(&self) -> usize {
        self.index.len()
    }

    pub(crate) fn contains_key(&self, key: &K) -> bool {
        self.index.contains_key(key)
    }

    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        let at = *self.index.get(key)?;
        self.entries[at].as_ref().map(|(_, value)| value)
    }

    pub(crate) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let at = *self.index.get(key)?;
        self.entries[at].as_mut().map(|(_, value)| value)
    }

    /// Sets `key` to `value`: in its place if present, else last. Returns the old value.
    pub(crate) fn insert(&mut self, key: K, value: V) -> Option<V> {
        if let Some(&at) = self.index.get(&key) {
            let slot = self.entries[at].as_mut().expect("indexed entries are live");
            return Some(std::mem::replace(&mut slot.1, value));
        }
        self.index.insert(key, self.entries.len());
        self.entries.push(Some((key, value)));
        None
    }

    /// Removes `key`, keeping every other entry's place. Amortised `O(1)`.
    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        let at = self.index.remove(key)?;
        let (_, value) = self.entries[at].take().expect("indexed entries are live");
        if self.index.is_empty() {
            self.entries.clear();
        } else if self.entries.len() > 2 * self.index.len() + 16 {
            self.compact();
        }
        Some(value)
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }

    #[cfg(test)] // Used by the port's next parts (copies, requests).
    /// The first entry in order.
    pub(crate) fn first(&self) -> Option<(&K, &V)> {
        self.iter().next()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries
            .iter()
            .filter_map(|entry| entry.as_ref().map(|(key, value)| (key, value)))
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(key, _)| key)
    }

    // Drops the holes, keeping the order. O(entries).
    fn compact(&mut self) {
        self.entries.retain(Option::is_some);
        for (at, entry) in self.entries.iter().enumerate() {
            let (key, _) = entry.as_ref().expect("retained entries are live");
            self.index.insert(*key, at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterates_as_a_godot_dictionary() {
        let mut map = OrderedMap::default();
        for key in 0..40 {
            map.insert(key, key * 10);
        }
        // Reassigning keeps the place; removing and re-adding goes last.
        map.insert(5, 7);
        for key in 10..35 {
            map.remove(&key);
        }
        map.insert(12, 1);
        map.remove(&0);
        let order: Vec<i32> = map.keys().copied().collect();
        let mut want: Vec<i32> = (1..10).chain(35..40).collect();
        want.push(12);
        assert_eq!(order, want);
        assert_eq!(map.get(&5), Some(&7));
        assert_eq!(map.len(), want.len());
        assert_eq!(map.first(), Some((&1, &10)));
        assert!(map.entries.len() <= 2 * map.len() + 16);
        // Compaction (past twice the live entries plus 16) keeps the order.
        for key in 1..10 {
            map.remove(&key);
        }
        assert!(map.entries.len() <= 2 * map.len() + 16);
        let order: Vec<i32> = map.keys().copied().collect();
        assert_eq!(order, vec![35, 36, 37, 38, 39, 12]);
        assert_eq!(map.get(&12), Some(&1));
        map.insert(1, 2);
        assert_eq!(map.keys().last(), Some(&1));
    }
}
