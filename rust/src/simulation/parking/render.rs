// SPDX-License-Identifier: GPL-2.0-only

//! Versioned parked-only MultiMesh chunks, published through the render snapshot (`PARK-02`).
//!
//! The sim thread rebuilds dirty chunks at the end of a step and hands them to the snapshot it
//! is building, so moving cars and parked chunks reach Godot from one custody state. Godot never
//! takes the core lock for them: it answers through [`ParkedAck`]'s atomics.

use super::data::GROUND_UNSOLVED;
use super::*;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// One chunk's immutable publication; an empty bucket map removes its previous instances.
#[derive(Debug, Default)]
pub(crate) struct ParkedChunk {
    pub version: u64,
    pub buckets: BTreeMap<u8, Vec<f32>>,
    pub identities: BTreeMap<u8, Vec<u64>>,
}

/// One chunk of a publication and the serial that published it. The buffers are shared.
#[derive(Clone, Debug)]
pub(crate) struct ParkedUpdate {
    pub key: (i32, i32),
    pub serial: u64,
    pub chunk: Arc<ParkedChunk>,
}

/// What the renderer tells the sim thread, without any lock.
#[derive(Debug, Default)]
pub(crate) struct ParkedAck {
    // The newest serial Godot has installed. Only ever rises, so a late answer cannot discard a
    // newer chunk.
    installed: AtomicU64,
    full: AtomicBool,
}

/// The parked half of a render snapshot. Chunks stay listed until the renderer acknowledges
/// them, so a snapshot Godot never read loses nothing. Cloning it copies two `Arc`s.
#[derive(Clone, Debug, Default)]
pub(crate) struct ParkedPublication {
    /// Newest serial published, the custody state this snapshot's moving cars agree with.
    pub serial: u64,
    /// Serial of the last complete republication.
    pub full_serial: u64,
    /// Every chunk not yet acknowledged when the snapshot was built, in chunk order.
    pub updates: Arc<Vec<ParkedUpdate>>,
    ack: Arc<ParkedAck>,
}

impl ParkedPublication {
    /// True when this snapshot holds chunks the renderer has not installed; `full` asks for a
    /// complete republication unless this snapshot already carries one. O(1), allocation-free.
    pub(crate) fn offer(&self, full: bool) -> bool {
        let installed = self.ack.installed.load(Ordering::Relaxed);
        if full && self.full_serial <= installed {
            self.ack.full.store(true, Ordering::Relaxed);
        }
        self.serial > installed
    }

    /// The listed chunks the renderer has not installed. O(listed chunks).
    pub(crate) fn unseen(&self) -> impl Iterator<Item = &ParkedUpdate> {
        let installed = self.ack.installed.load(Ordering::Relaxed);
        self.updates
            .iter()
            .filter(move |update| update.serial > installed)
    }

    /// True when [`Self::unseen`] holds a complete republication. O(1).
    pub(crate) fn holds_full(&self) -> bool {
        self.full_serial > self.ack.installed.load(Ordering::Relaxed)
    }

    /// Records that the renderer installed everything up to `serial`. O(1), lock-free.
    pub(crate) fn acknowledge(&self, serial: u64) {
        self.ack.installed.fetch_max(serial, Ordering::Relaxed);
    }
}

// A published car for tests: its model, its bay's source, position and heading, and its pose.
#[cfg(test)]
pub(crate) type PublishedPose = (u8, PlaceSource, [f32; 3], f32, [f32; 12]);

// One rebuilt chunk and the poses it had to solve, applied after the parallel build.
pub(crate) struct BuiltChunk {
    key: (i32, i32),
    payload: ParkedChunk,
    solved: Vec<(usize, u8, [f32; 12])>,
}

impl ParkingSystem {
    /// True when a publication has chunks to rebuild. O(1).
    pub(crate) fn has_dirty_chunks(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Rebuilds the dirty chunks with row-major flat poses, for tests without a ground.
    #[cfg(test)]
    pub(crate) fn build_dirty_chunks(&mut self) {
        self.build_dirty_chunks_with(|_, _, [x, y, z], heading| {
            let (s, c) = heading.sin_cos();
            [c, 0.0, s, x, 0.0, 1.0, 0.0, y, -s, 0.0, c, z]
        });
    }

    /// [`Self::solve_dirty_chunks`] and [`Self::install_dirty_chunks`] with one pose function.
    #[cfg(test)]
    pub(crate) fn build_dirty_chunks_with(
        &mut self,
        pose: impl Fn(u8, PlaceSource, [f32; 3], f32) -> [f32; 12] + Sync,
    ) {
        let built = self.solve_dirty_chunks(pose);
        self.install_dirty_chunks(built);
    }

    /// Builds every dirty chunk's payload, in parallel: a copy of each parked car's cached pose.
    /// O(sum(Pc + C log C)) for Pc places and C parked cars per dirty chunk, plus Q, the ground
    /// solve, only for a car on a place with no pose for its model: a new or moved-ground place,
    /// or a car of another model arrived since the last solve. Sorting fixes payload order.
    pub(crate) fn solve_dirty_chunks(
        &self,
        pose: impl Fn(u8, PlaceSource, [f32; 3], f32) -> [f32; 12] + Sync,
    ) -> Vec<BuiltChunk> {
        crate::thread_pools::install(|| {
            self.dirty
                .par_iter()
                .map(|&key| self.build_chunk(key, &pose))
                .collect()
        })
    }

    fn build_chunk(
        &self,
        key: (i32, i32),
        pose: &(impl Fn(u8, PlaceSource, [f32; 3], f32) -> [f32; 12] + Sync),
    ) -> BuiltChunk {
        let chunk = &self.chunks[&key];
        let mut parked: Vec<_> = chunk
            .places
            .iter()
            .filter_map(|&handle| {
                let p = self.place_handles.resolve(handle).expect("render bay");
                let v = self.vehicle_handles.resolve(self.places.occupant[p])?;
                (!self.vehicles.moving[v]).then_some((handle, p, v))
            })
            .collect();
        parked.sort_unstable();
        let mut built = BuiltChunk {
            key,
            payload: ParkedChunk {
                version: chunk.version + 1,
                ..ParkedChunk::default()
            },
            solved: Vec::new(),
        };
        for (_, p, v) in parked {
            let model = self.vehicles.model[v];
            let grounded = if self.places.ground_model[p] == model {
                self.places.ground[p]
            } else {
                let solved = pose(
                    model,
                    self.places.source[p],
                    self.places.position[p],
                    self.places.heading[p],
                );
                built.solved.push((p, model, solved));
                solved
            };
            let bucket = model * 10 + self.vehicles.colour[v];
            let payload = &mut built.payload;
            payload
                .buckets
                .entry(bucket)
                .or_default()
                .extend_from_slice(&grounded);
            payload
                .identities
                .entry(bucket)
                .or_default()
                .push(self.places.occupant[p].to_bits() | (1_u64 << 62));
        }
        built
    }

    /// Installs built chunks under one new serial, with the poses they solved. O(D log pending).
    pub(crate) fn install_dirty_chunks(&mut self, built: Vec<BuiltChunk>) {
        if built.is_empty() {
            return;
        }
        self.publication.serial += 1;
        let serial = self.publication.serial;
        for BuiltChunk {
            key,
            payload,
            solved,
        } in built
        {
            for (p, model, pose) in solved {
                self.places.ground[p] = pose;
                self.places.ground_model[p] = model;
            }
            let chunk = self.chunks.get_mut(&key).expect("dirty chunk");
            chunk.version = payload.version;
            chunk.dirty = false;
            let chunk = Arc::new(payload);
            self.published.insert(key, Arc::clone(&chunk));
            self.pending
                .insert(key, ParkedUpdate { key, serial, chunk });
        }
        self.pending_changed = true;
        self.dirty.clear();
    }

    /// Applies the renderer's answers since the last snapshot: drops acknowledged chunks and
    /// lists every chunk again on a full request. O(1) without answers, O(pending chunks) on an
    /// acknowledgement, O(all chunks) on a renderer reconnect.
    pub(crate) fn take_renderer_answers(&mut self) {
        let installed = self.publication.ack.installed.load(Ordering::Relaxed);
        if installed != self.acknowledged {
            self.acknowledged = installed;
            let listed = self.pending.len();
            self.pending.retain(|_, update| update.serial > installed);
            self.pending_changed |= self.pending.len() != listed;
        }
        if self.publication.ack.full.swap(false, Ordering::Relaxed) {
            self.publication.serial += 1;
            let serial = self.publication.serial;
            self.publication.full_serial = serial;
            for (&key, chunk) in &self.published {
                let chunk = Arc::clone(chunk);
                self.pending
                    .insert(key, ParkedUpdate { key, serial, chunk });
            }
            self.pending_changed = true;
        }
    }

    /// The publication for a snapshot. O(1) when no chunk changed, else O(pending chunks) `Arc`
    /// copies: buffers are never copied, and the list is built once for every snapshot after.
    pub(crate) fn publication(&mut self) -> ParkedPublication {
        if std::mem::take(&mut self.pending_changed) {
            self.publication.updates = Arc::new(self.pending.values().cloned().collect());
        }
        self.publication.clone()
    }

    /// Forgets the pose of every place within `rect` (world `[min_x, min_z, max_x, max_z]`),
    /// because the ground there changed, and dirties chunks with a car on such a place.
    /// O(chunks the rectangle touches + their places), at edit boundaries only.
    pub(crate) fn invalidate_ground(&mut self, rect: [f64; 4]) {
        let [min_x, min_z, max_x, max_z] = rect.map(|v| v as f32);
        let (first, last) = (chunk_at([min_x, min_z]), chunk_at([max_x, max_z]));
        for z in first.1..=last.1 {
            for x in first.0..=last.0 {
                let Some(chunk) = self.chunks.get(&(x, z)) else {
                    continue;
                };
                let mut occupied = false;
                for &handle in &chunk.places {
                    let p = self.place_handles.resolve(handle).expect("indexed bay");
                    let [px, _, pz] = self.places.position[p];
                    if (min_x..=max_x).contains(&px) && (min_z..=max_z).contains(&pz) {
                        self.places.ground_model[p] = GROUND_UNSOLVED;
                        occupied |= !self.places.occupant[p].is_none();
                    }
                }
                if occupied {
                    self.mark_dirty((x, z));
                }
            }
        }
    }

    /// Forgets every pose: a new or reloaded world, a dense terrain undo, a new car profile.
    /// O(places), at those events only.
    pub(crate) fn invalidate_all_ground(&mut self) {
        self.places.ground_model.fill(GROUND_UNSOLVED);
        for p in 0..self.places.len() {
            if !self.places.occupant[p].is_none() {
                let [x, _, z] = self.places.position[p];
                self.mark_dirty(chunk_at([x, z]));
            }
        }
    }
}

impl ParkingSystem {
    /// The listed or all published chunks, for assertions. O(chunks).
    #[cfg(test)]
    pub(crate) fn publications(&self, full: bool) -> Vec<((i32, i32), Arc<ParkedChunk>)> {
        if full {
            self.published
                .iter()
                .map(|(&key, value)| (key, Arc::clone(value)))
                .collect()
        } else {
            self.pending
                .iter()
                .map(|(&key, update)| (key, Arc::clone(&update.chunk)))
                .collect()
        }
    }

    /// Overwrites every cached pose with a value no solve gives.
    #[cfg(test)]
    pub(crate) fn poison_cached_poses_for_test(&mut self) {
        self.places.ground.fill([1234.0; 12]);
    }

    /// The first car standing on a bay.
    #[cfg(test)]
    pub(crate) fn first_parked_car_for_test(&self) -> Handle<Vehicle> {
        *self
            .places
            .occupant
            .iter()
            .find(|car| !car.is_none())
            .expect("a parked car")
    }

    /// How many places hold a pose.
    #[cfg(test)]
    pub(crate) fn solved_poses_for_test(&self) -> usize {
        self.places
            .ground_model
            .iter()
            .filter(|&&model| model != GROUND_UNSOLVED)
            .count()
    }

    /// Every published parked car: its model, bay position and heading and the pose it shows.
    #[cfg(test)]
    pub(crate) fn published_poses(&self) -> Vec<PublishedPose> {
        let mut poses = Vec::new();
        for chunk in self.published.values() {
            for (bucket, ids) in &chunk.identities {
                let poses_of_bucket = chunk.buckets[bucket].as_chunks::<12>().0;
                for (&id, &pose) in ids.iter().zip(poses_of_bucket) {
                    let car = Handle::from_bits(id & !(1_u64 << 62));
                    let v = self.vehicle_handles.resolve(car).expect("published car");
                    let p = self
                        .place_handles
                        .resolve(self.vehicles.place[v])
                        .expect("its bay");
                    let (position, heading) = (self.places.position[p], self.places.heading[p]);
                    let source = self.places.source[p];
                    poses.push((self.vehicles.model[v], source, position, heading, pose));
                }
            }
        }
        poses
    }

    /// O(off-map home chunks) inspection; never visits sleeping parked cars or supply.
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        (
            self.usable_places,
            self.vehicles.len(),
            self.stored
                .values()
                .map(std::collections::BTreeSet::len)
                .sum(),
        )
    }
}

impl ParkingSystem {
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
