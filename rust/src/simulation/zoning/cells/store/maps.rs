// SPDX-License-Identifier: GPL-2.0-only

//! The two map kinds a cell store keeps its entries in (`ROAD-68` open question 32): the live
//! store's plain hash maps, and the persistent maps of the road tool view's copy, whose clone is
//! `O(1)` and shares every entry until either side writes it. The store and every algorithm on
//! it are generic over [`CellMaps`], so the live store and the view's copy run one code path.
//! The persistent kind keeps each value behind an `Arc`, so a copied map node moves pointers
//! and a write copies only the value it changes; its writers read first, so an unchanged or
//! absent entry never copies a shared path.

use super::GridFrame;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

type Hasher = foldhash::fast::FixedState;

/// One keyed map of a cell store, implemented by both map kinds.
pub(crate) trait StoreMap<K, V>: Clone + Debug + Default + Send + Sync {
    /// The entry at `key`.
    fn get(&self, key: &K) -> Option<&V>;
    /// The entry at `key` for a write, copying a shared path in the persistent kind.
    fn get_mut(&mut self, key: &K) -> Option<&mut V>;
    /// The entry at `key` for a write when `changes` says the write changes it: one lookup in
    /// the plain kind; the persistent kind tests a shared read first.
    fn get_mut_if(&mut self, key: &K, changes: impl FnOnce(&V) -> bool) -> Option<&mut V>;
    /// The entry at `key`, inserting the default first.
    fn entry_or_default(&mut self, key: K) -> &mut V
    where
        V: Default;
    /// Sets the entry at `key`.
    fn insert(&mut self, key: K, value: V);
    /// Removes the entry at `key`, if any.
    fn remove(&mut self, key: &K);
    /// Whether an entry exists at `key`.
    fn contains_key(&self, key: &K) -> bool;
    /// Whether the map is empty.
    fn is_empty(&self) -> bool;
    /// Removes every entry.
    fn clear(&mut self);
    /// Every entry, in the map's own order (callers that publish sort).
    fn iter<'a>(&'a self) -> impl Iterator<Item = (&'a K, &'a V)>
    where
        K: 'a,
        V: 'a;
    /// Calls `visit` with every value for a write.
    fn for_each_mut(&mut self, visit: impl FnMut(&mut V));
}

/// The grid frames of a cell store, in id order (id = index + 1); append-only between clears.
pub(crate) trait FrameList: Clone + Debug + Default + Send + Sync {
    /// The number of registered frames.
    fn len(&self) -> usize;
    /// The frame at `index`.
    fn get(&self, index: usize) -> Option<GridFrame>;
    /// Appends a frame.
    fn push(&mut self, frame: GridFrame);
    /// Removes every frame.
    fn clear(&mut self);
    /// Every frame in id order.
    fn iter(&self) -> impl Iterator<Item = GridFrame>;
}

/// A cell store's map kind.
pub(crate) trait CellMaps: Clone + Debug + Default + Send + Sync + 'static {
    /// The keyed map of this kind.
    type Map<K: Clone + Debug + Eq + Hash + Send + Sync, V: Clone + Debug + Send + Sync>: StoreMap<K, V>;
    /// The frame list of this kind.
    type Frames: FrameList;
}

/// The live store's kind: plain hash maps on the fixed hasher and a vector of frames.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LiveMaps;

/// The road tool view's kind: persistent maps on the same hasher, values behind `Arc`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SharedMaps;

impl CellMaps for LiveMaps {
    type Map<K: Clone + Debug + Eq + Hash + Send + Sync, V: Clone + Debug + Send + Sync> =
        std::collections::HashMap<K, V, Hasher>;
    type Frames = Vec<GridFrame>;
}

impl CellMaps for SharedMaps {
    type Map<K: Clone + Debug + Eq + Hash + Send + Sync, V: Clone + Debug + Send + Sync> =
        SharedMap<K, V>;
    type Frames = imbl::Vector<GridFrame>;
}

impl<K, V> StoreMap<K, V> for std::collections::HashMap<K, V, Hasher>
where
    K: Clone + Debug + Eq + Hash + Send + Sync,
    V: Clone + Debug + Send + Sync,
{
    #[inline]
    fn get(&self, key: &K) -> Option<&V> {
        std::collections::HashMap::get(self, key)
    }

    #[inline]
    fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        std::collections::HashMap::get_mut(self, key)
    }

    #[inline]
    fn get_mut_if(&mut self, key: &K, changes: impl FnOnce(&V) -> bool) -> Option<&mut V> {
        std::collections::HashMap::get_mut(self, key).filter(|value| changes(value))
    }

    #[inline]
    fn entry_or_default(&mut self, key: K) -> &mut V
    where
        V: Default,
    {
        self.entry(key).or_default()
    }

    #[inline]
    fn insert(&mut self, key: K, value: V) {
        std::collections::HashMap::insert(self, key, value);
    }

    #[inline]
    fn remove(&mut self, key: &K) {
        std::collections::HashMap::remove(self, key);
    }

    #[inline]
    fn contains_key(&self, key: &K) -> bool {
        std::collections::HashMap::contains_key(self, key)
    }

    #[inline]
    fn is_empty(&self) -> bool {
        std::collections::HashMap::is_empty(self)
    }

    #[inline]
    fn clear(&mut self) {
        std::collections::HashMap::clear(self);
    }

    #[inline]
    fn iter<'a>(&'a self) -> impl Iterator<Item = (&'a K, &'a V)>
    where
        K: 'a,
        V: 'a,
    {
        std::collections::HashMap::iter(self)
    }

    #[inline]
    fn for_each_mut(&mut self, visit: impl FnMut(&mut V)) {
        self.values_mut().for_each(visit);
    }
}

/// A persistent hash map whose values sit behind `Arc` ([`SharedMaps`]).
#[derive(Clone, Debug)]
pub(crate) struct SharedMap<K, V>(
    imbl::GenericHashMap<K, Arc<V>, Hasher, imbl::shared_ptr::DefaultSharedPtr>,
);

impl<K, V> Default for SharedMap<K, V> {
    fn default() -> Self {
        Self(imbl::GenericHashMap::with_hasher(Hasher::default()))
    }
}

impl<K, V> StoreMap<K, V> for SharedMap<K, V>
where
    K: Clone + Debug + Eq + Hash + Send + Sync,
    V: Clone + Debug + Send + Sync,
{
    #[inline]
    fn get(&self, key: &K) -> Option<&V> {
        self.0.get(key).map(Arc::as_ref)
    }

    #[inline]
    fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.0.get_mut(key).map(Arc::make_mut)
    }

    #[inline]
    fn get_mut_if(&mut self, key: &K, changes: impl FnOnce(&V) -> bool) -> Option<&mut V> {
        // imbl's `get_mut` copies the shared path even when the caller changes nothing.
        if self.0.get(key).is_some_and(|value| changes(value)) {
            self.get_mut(key)
        } else {
            None
        }
    }

    #[inline]
    fn entry_or_default(&mut self, key: K) -> &mut V
    where
        V: Default,
    {
        Arc::make_mut(self.0.entry(key).or_default())
    }

    #[inline]
    fn insert(&mut self, key: K, value: V) {
        self.0.insert(key, Arc::new(value));
    }

    #[inline]
    fn remove(&mut self, key: &K) {
        // A persistent map's removal copies its shared path even for an absent key.
        if self.0.contains_key(key) {
            self.0.remove(key);
        }
    }

    #[inline]
    fn contains_key(&self, key: &K) -> bool {
        self.0.contains_key(key)
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    fn clear(&mut self) {
        self.0.clear();
    }

    #[inline]
    fn iter<'a>(&'a self) -> impl Iterator<Item = (&'a K, &'a V)>
    where
        K: 'a,
        V: 'a,
    {
        self.0.iter().map(|(key, value)| (key, value.as_ref()))
    }

    #[inline]
    fn for_each_mut(&mut self, mut visit: impl FnMut(&mut V)) {
        for (_, value) in self.0.iter_mut() {
            visit(Arc::make_mut(value));
        }
    }
}

impl FrameList for Vec<GridFrame> {
    #[inline]
    fn len(&self) -> usize {
        Vec::len(self)
    }

    #[inline]
    fn get(&self, index: usize) -> Option<GridFrame> {
        self.as_slice().get(index).copied()
    }

    #[inline]
    fn push(&mut self, frame: GridFrame) {
        Vec::push(self, frame);
    }

    #[inline]
    fn clear(&mut self) {
        Vec::clear(self);
    }

    #[inline]
    fn iter(&self) -> impl Iterator<Item = GridFrame> {
        self.as_slice().iter().copied()
    }
}

impl FrameList for imbl::Vector<GridFrame> {
    #[inline]
    fn len(&self) -> usize {
        imbl::Vector::len(self)
    }

    #[inline]
    fn get(&self, index: usize) -> Option<GridFrame> {
        imbl::Vector::get(self, index).copied()
    }

    #[inline]
    fn push(&mut self, frame: GridFrame) {
        self.push_back(frame);
    }

    #[inline]
    fn clear(&mut self) {
        imbl::Vector::clear(self);
    }

    #[inline]
    fn iter(&self) -> impl Iterator<Item = GridFrame> {
        imbl::Vector::iter(self).copied()
    }
}

/// Keys a store wrote since the last drain, for the view's copy (`ROAD-68` open question 32).
/// Consecutive repeats are dropped at once; the log sorts and dedups itself whenever it doubles,
/// so it holds at most twice the distinct keys written. Amortized `O(log n)` per write.
#[derive(Clone, Debug)]
pub(super) struct KeyLog<K> {
    keys: Vec<K>,
    compact_at: usize,
}

// A log below this length is never compacted.
const MIN_COMPACT_LEN: usize = 4096;

impl<K> Default for KeyLog<K> {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            compact_at: MIN_COMPACT_LEN,
        }
    }
}

impl<K: Copy + Ord> KeyLog<K> {
    #[inline]
    pub(super) fn push(&mut self, key: K) {
        if self.keys.last() == Some(&key) {
            return;
        }
        self.keys.push(key);
        if self.keys.len() >= self.compact_at {
            self.compact();
        }
    }

    fn compact(&mut self) {
        self.keys.sort_unstable();
        self.keys.dedup();
        self.compact_at = (2 * self.keys.len()).max(MIN_COMPACT_LEN);
    }

    /// The distinct keys written since the last drain, sorted; the log keeps its buffer.
    pub(super) fn drain_sorted(&mut self) -> std::vec::Drain<'_, K> {
        self.compact();
        self.compact_at = MIN_COMPACT_LEN;
        self.keys.drain(..)
    }

    pub(super) fn clear(&mut self) {
        self.keys.clear();
        self.compact_at = MIN_COMPACT_LEN;
    }
}
