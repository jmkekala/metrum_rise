// SPDX-License-Identifier: GPL-2.0-only

//! Curve guide lookup and lifetime using the existing cell chunk index.

use super::*;
use std::collections::HashSet;

impl CellStore {
    /// Resolves a canonical frame through its existing constant-time registry.
    pub(crate) fn grid_for_frame(&self, frame: GridFrame) -> Option<u64> {
        self.frame_ids.get(&frame).copied()
    }
    /// Returns local guides in stable order; each guide covers at most four by six cells.
    pub(crate) fn curve_sources_in_bounds(&self, bounds: CellBounds) -> Vec<Arc<CellCurveSource>> {
        if !bounds.is_valid() {
            return Vec::new();
        }
        let mut keys = Vec::new();
        for chunk in chunks_for_bounds(bounds) {
            if let Some(entry) = self.chunks.get(&chunk) {
                for &key in &entry.sources {
                    if key.bounds().intersects(bounds) {
                        keys.push(key);
                    }
                }
            }
        }
        keys.sort_unstable();
        keys.dedup();
        keys.into_iter()
            .filter_map(|key| self.curve_sources.get(&key).cloned())
            .collect()
    }

    /// Tests only the bounded group, keeping unrelated city reservations off edit paths.
    pub(crate) fn curve_source_reserved(&self, source: &CellCurveSource) -> bool {
        self.frame_ids.get(&source.key.frame).is_some_and(|&grid| {
            source.key.cells(grid).any(|key| {
                self.profile(key).is_some_and(|p| p != 0)
                    || self.lot(key).is_some_and(|lot| lot != 0)
            })
        })
    }

    /// Publishes source provenance after cells pass geometry and reservation validation.
    pub(crate) fn publish_curve_source(&mut self, source: Arc<CellCurveSource>) {
        let key = source.key;
        if !self
            .frame_ids
            .get(&key.frame)
            .is_some_and(|&grid| key.cells(grid).any(|cell| self.profile(cell).is_some()))
        {
            return;
        }
        if !self.curve_sources.contains_key(&key) {
            for chunk in chunks_for_bounds(key.bounds()) {
                self.chunks.entry(chunk).or_default().sources.push(key);
            }
        }
        self.curve_sources.insert(key, source);
    }

    /// Drops locally evicted empty guides; paint and occupied coverage retain their originals.
    pub(crate) fn prune_curve_sources(&mut self, bounds: CellBounds) {
        for source in self.curve_sources_in_bounds(bounds) {
            let key = source.key;
            if self
                .frame_ids
                .get(&key.frame)
                .is_some_and(|&grid| key.cells(grid).any(|cell| self.profile(cell).is_some()))
            {
                continue;
            }
            self.curve_sources.remove(&key);
            for chunk in chunks_for_bounds(key.bounds()) {
                if let Some(entry) = self.chunks.get_mut(&chunk) {
                    entry.sources.retain(|&candidate| candidate != key);
                }
            }
        }
    }

    /// Serializes only authoritative groups; unpainted, unclaimed guides remain rebuildable.
    pub(crate) fn saved_curve_sources(&self) -> Vec<&CellCurveSource> {
        let mut sources: Vec<_> = self
            .curve_sources
            .values()
            .filter(|source| self.curve_source_reserved(source))
            .map(Arc::as_ref)
            .collect();
        sources.sort_unstable_by_key(|source| source.key);
        sources
    }

    /// Rejects malformed or duplicate saved guides before adding their chunk references.
    pub(crate) fn restore_curve_source(&mut self, source: CellCurveSource) -> bool {
        if !source.is_valid()
            || self.curve_sources.contains_key(&source.key)
            || !self.frame_ids.get(&source.key.frame).is_some_and(|&grid| {
                source
                    .key
                    .cells(grid)
                    .any(|key| self.profile(key).is_some())
            })
        {
            return false;
        }
        self.publish_curve_source(Arc::new(source));
        true
    }

    /// Captures only touched groups for erase undo, querying each touched chunk once.
    pub(crate) fn capture_curve_sources(&self, cells: &[CellKey]) -> Vec<Arc<CellCurveSource>> {
        if self.curve_sources.is_empty() || cells.is_empty() {
            return Vec::new();
        }
        let mut chunks = HashSet::new();
        for &key in cells {
            if let Some(frame) = self.frame(key.grid) {
                chunks.extend(chunks_for_bounds(CellBounds::from_points(
                    frame.corners(key.x, key.y),
                )));
            }
        }
        let mut keys = Vec::new();
        for chunk in chunks {
            if let Some(entry) = self.chunks.get(&chunk) {
                keys.extend(entry.sources.iter().copied());
            }
        }
        keys.sort_unstable();
        keys.dedup();
        if keys.is_empty() {
            return Vec::new();
        }
        let selected: HashSet<_> = cells.iter().copied().collect();
        keys.into_iter()
            .filter(|key| {
                self.frame_ids
                    .get(&key.frame)
                    .is_some_and(|&grid| key.cells(grid).any(|cell| selected.contains(&cell)))
            })
            .filter_map(|key| self.curve_sources.get(&key).cloned())
            .collect()
    }
}
