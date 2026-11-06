// SPDX-License-Identifier: GPL-2.0-only

//! Versioned parked-only MultiMesh chunks, retained until the exact version is acknowledged.

use super::*;
use rayon::prelude::*;
use std::collections::BTreeMap;

/// One chunk's immutable publication; an empty bucket map removes its previous instances.
#[derive(Debug, Default)]
pub(crate) struct ParkedChunk {
    pub version: u64,
    pub buckets: BTreeMap<u8, Vec<f32>>,
    pub identities: BTreeMap<u8, Vec<u64>>,
}

impl ParkingSystem {
    /// O(D log D + sum(Pc log Pc) + cars), parallel independent chunks. Zero work/payload when
    /// unchanged; publications survive skipped snapshots until version-specific acknowledgement.
    #[cfg(test)]
    pub(crate) fn build_dirty_chunks(&mut self) {
        self.build_dirty_chunks_with(|_, [x, y, z], heading| {
            let (s, c) = heading.sin_cos();
            [c, 0.0, s, x, 0.0, 1.0, 0.0, y, -s, 0.0, c, z]
        });
    }

    /// Builds support poses once per dirty publication; Q is the existing ground-support solve.
    /// O(D log D + sum(Pc log Pc) + parked cars × Q), with D dirty chunks and Pc places
    /// in each chunk. Sorting fixes payload order; no steady-state iteration.
    pub(crate) fn build_dirty_chunks_with(
        &mut self,
        pose: impl Fn(u8, [f32; 3], f32) -> [f32; 12] + Sync,
    ) {
        if self.dirty.is_empty() {
            return;
        }
        self.dirty.sort_unstable();
        let built: Vec<_> = crate::thread_pools::install(|| {
            self.dirty
                .par_iter()
                .map(|&key| {
                    let chunk = &self.chunks[&key];
                    let mut payload = ParkedChunk {
                        version: chunk.version + 1,
                        ..ParkedChunk::default()
                    };
                    let mut handles = chunk.places.clone();
                    handles.sort_unstable();
                    for handle in handles {
                        let p = self.place_handles.resolve(handle).expect("render bay");
                        let Some(v) = self.vehicle_handles.resolve(self.places.occupant[p]) else {
                            continue;
                        };
                        if self.vehicles.moving[v] {
                            continue;
                        }
                        let bucket = self.vehicles.model[v] * 10 + self.vehicles.colour[v];
                        let buffer = payload.buckets.entry(bucket).or_default();
                        buffer.extend_from_slice(&pose(
                            self.vehicles.model[v],
                            self.places.position[p],
                            self.places.heading[p],
                        ));
                        payload
                            .identities
                            .entry(bucket)
                            .or_default()
                            .push(self.places.occupant[p].to_bits() | (1_u64 << 62));
                    }
                    (key, Arc::new(payload))
                })
                .collect()
        });
        for (key, payload) in built {
            let chunk = self.chunks.get_mut(&key).expect("dirty chunk");
            chunk.version = payload.version;
            chunk.dirty = false;
            self.published.insert(key, Arc::clone(&payload));
            self.pending.insert(key, payload);
        }
        self.dirty.clear();
    }

    /// Acknowledges exactly the installed chunk version; O(log pending chunks).
    pub(crate) fn acknowledge(&mut self, key: (i32, i32), version: u64) {
        if self.pending.get(&key).is_some_and(|p| p.version == version) {
            self.pending.remove(&key);
        }
    }
}

impl ParkingSystem {
    /// O(pending chunks), or O(all chunks) for a renderer reconnect; empty unchanged batches
    /// allocate no payload. Immutable buffers are shared until Godot packs its upload.
    pub(crate) fn publications(&self, full: bool) -> Vec<((i32, i32), Arc<ParkedChunk>)> {
        let map = if full { &self.published } else { &self.pending };
        map.iter()
            .map(|(&key, value)| (key, Arc::clone(value)))
            .collect()
    }

    /// O(off-map home chunks) inspection; never visits sleeping parked cars or supply.
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        (
            self.usable_places,
            self.vehicles.len(),
            self.stored.values().map(std::collections::BTreeSet::len).sum(),
        )
    }
}

impl ParkingSystem {
    /// O(1) moving/parked custody revision, used to reject mixed publication frames.
    pub(crate) fn publication_epoch(&self) -> u64 {
        self.publication_epoch
    }

    /// O(1) persisted model, palette and stable identity for moving/parked handoff.
    pub(crate) fn vehicle_visual(&self, car: Handle<Vehicle>) -> Option<(u8, u8, u64)> {
        let v = self.vehicle_handles.resolve(car)?;
        Some((
            self.vehicles.model[v],
            self.vehicles.colour[v],
            car.to_bits() | (1_u64 << 62),
        ))
    }
}
