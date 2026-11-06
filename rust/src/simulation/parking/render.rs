// SPDX-License-Identifier: GPL-2.0-only

//! Versioned parked-only MultiMesh chunks, published through the render snapshot (`PARK-02`).
//!
//! The sim thread rebuilds dirty chunks at the end of a step and hands them to the snapshot it
//! is building, so moving cars and parked chunks reach Godot from one custody state. Godot never
//! takes the core lock for them: it answers through [`ParkedAck`]'s atomics.

use super::data::{GROUND_UNSOLVED, NOT_PARKED};
use super::*;
use rayon::prelude::*;
use std::cell::Cell;
use std::cmp::Reverse;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// One chunk's immutable publication; no buckets removes its previous instances. Three flat
/// buffers, so the next rebuild can reuse them whole.
#[derive(Debug, Default)]
pub(crate) struct ParkedChunk {
    pub version: u64,
    // Twelve floats and one identity a car, bucket by bucket, each bucket in bay handle order.
    poses: Vec<f32>,
    identities: Vec<u64>,
    // Each bucket present, ascending, and the car index it ends at.
    runs: Vec<(u8, u32)>,
}

impl ParkedChunk {
    /// Each bucket with its poses and identities, ascending. O(buckets), no copy.
    pub(crate) fn buckets(&self) -> impl Iterator<Item = (u8, &[f32], &[u64])> {
        let mut start = 0;
        self.runs.iter().map(move |&(bucket, end)| {
            let cars = start as usize..end as usize;
            start = end;
            (
                bucket,
                &self.poses[cars.start * 12..cars.end * 12],
                &self.identities[cars],
            )
        })
    }

    /// How many cars this chunk shows. O(1).
    #[cfg(test)]
    pub(crate) fn cars(&self) -> usize {
        self.identities.len()
    }

    /// What this chunk's buffers can hold, summed.
    #[cfg(test)]
    pub(crate) fn capacities(&self) -> usize {
        self.poses.capacity() + self.identities.capacity() + self.runs.capacity()
    }

    fn clear(&mut self) {
        self.poses.clear();
        self.identities.clear();
        self.runs.clear();
    }

    fn push(&mut self, bucket: u8, pose: &[f32; 12], identity: u64) {
        self.poses.extend_from_slice(pose);
        self.identities.push(identity);
        let cars = self.identities.len() as u32;
        match self.runs.last_mut() {
            Some(run) if run.0 == bucket => run.1 = cars,
            _ => self.runs.push((bucket, cars)),
        }
    }
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

// A slot of one frame's rebuild: the chunk, the buffers it writes, which nothing else holds, and
// the poses it had to solve, applied after the parallel build.
#[derive(Debug, Default)]
pub(crate) struct ChunkJob {
    key: (i32, i32),
    chunk: Option<Arc<ParkedChunk>>,
    solved: Vec<(usize, u8, [f32; 12])>,
}

#[cfg(test)]
impl ChunkJob {
    /// What this slot's solved list can hold.
    pub(crate) fn solved_capacity(&self) -> usize {
        self.solved.capacity()
    }
}

// A load solves every car at once; a slot keeps no more than a busy frame's arrivals need.
const SOLVED_KEPT: usize = 64;

// Bucket, bay, place row and car row of each car of the chunk being rebuilt.
type ParkedOrder = Vec<(u8, Handle<Place>, usize, usize)>;

thread_local! {
    // One per worker: its size follows the largest chunk the worker met, however many there are.
    static PARKED_ORDER: Cell<ParkedOrder> = const { Cell::new(Vec::new()) };
}

// A car's pose on flat ground with no contacts, for tests.
#[cfg(test)]
pub(super) fn flat_pose(_: u8, _: PlaceSource, [x, y, z]: [f32; 3], heading: f32) -> [f32; 12] {
    let (s, c) = heading.sin_cos();
    [c, 0.0, s, x, 0.0, 1.0, 0.0, y, -s, 0.0, c, z]
}

impl ParkingSystem {
    /// True when a publication has chunks to rebuild. O(1).
    pub(crate) fn has_dirty_chunks(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Rebuilds the dirty chunks with row-major flat poses, for tests without a ground.
    #[cfg(test)]
    pub(crate) fn build_dirty_chunks(&mut self) {
        self.build_dirty_chunks_with(flat_pose);
    }

    /// Puts a bay on its chunk's parked list or takes it off; a repeat changes nothing. Every
    /// custody event comes here. O(1), and `add_place` has reserved the room, so no allocation.
    pub(super) fn set_parked(&mut self, p: usize, parked: bool) {
        let slot = self.places.parked_slot[p];
        if parked == (slot != NOT_PARKED) {
            return;
        }
        let [x, _, z] = self.places.position[p];
        let chunk = self.chunks.get_mut(&chunk_at([x, z])).expect("indexed bay");
        if parked {
            self.places.parked_slot[p] = chunk.parked.len() as u32;
            chunk.parked.push(self.place_handles.handle(p));
        } else {
            self.places.parked_slot[p] = NOT_PARKED;
            chunk.parked.swap_remove(slot as usize);
            if let Some(&moved) = chunk.parked.get(slot as usize) {
                let moved = self.place_handles.resolve(moved).expect("parked bay");
                self.places.parked_slot[moved] = slot;
            }
        }
    }

    /// [`Self::solve_dirty_chunks`] and [`Self::install_dirty_chunks`] with one pose function.
    #[cfg(test)]
    pub(crate) fn build_dirty_chunks_with(
        &mut self,
        pose: impl Fn(u8, PlaceSource, [f32; 3], f32) -> [f32; 12] + Sync,
    ) {
        let mut jobs = self.take_dirty_jobs();
        self.solve_dirty_chunks(&mut jobs, pose);
        self.install_dirty_chunks(jobs);
    }

    /// One rebuild slot per dirty chunk, each with buffers only it holds: the chunk's spare when
    /// the last holder has let it go. O(dirty chunks). A new buffer means a chunk's first rebuild,
    /// or a snapshot outliving two publications, which the sim thread's two never do.
    pub(crate) fn take_dirty_jobs(&mut self) -> Vec<ChunkJob> {
        #[cfg(test)]
        for key in &self.dirty {
            self.check_parked_list(*key, &self.chunks[key]).unwrap();
        }
        let mut jobs = std::mem::take(&mut self.jobs);
        if jobs.len() < self.dirty.len() {
            jobs.resize_with(self.dirty.len(), ChunkJob::default);
        }
        for (job, &key) in jobs.iter_mut().zip(&self.dirty) {
            let spare = self.chunks.get_mut(&key).expect("dirty chunk").spare.take();
            let free = spare.filter(|chunk| Arc::strong_count(chunk) == 1);
            #[cfg(test)]
            {
                self.buffer_creations += usize::from(free.is_none());
            }
            job.key = key;
            job.chunk = Some(free.unwrap_or_default());
        }
        jobs
    }

    /// Builds every dirty chunk's payload, in parallel: a copy of each parked car's cached pose.
    /// O(sum(C log C)) for C parked cars per dirty chunk, empty bays never visited, plus Q, the
    /// ground solve, only for a car on a place with no pose for its model: a new or moved-ground
    /// place, or a car of another model arrived since the last solve. The log is the sort: the
    /// list is in no order, and the payload's order is bay handle order within each bucket.
    pub(crate) fn solve_dirty_chunks(
        &self,
        jobs: &mut [ChunkJob],
        pose: impl Fn(u8, PlaceSource, [f32; 3], f32) -> [f32; 12] + Sync,
    ) {
        let jobs = &mut jobs[..self.dirty.len()];
        crate::thread_pools::install(|| {
            jobs.par_iter_mut()
                .for_each(|job| self.build_chunk(job, &pose));
        });
    }

    fn build_chunk(
        &self,
        job: &mut ChunkJob,
        pose: &(impl Fn(u8, PlaceSource, [f32; 3], f32) -> [f32; 12] + Sync),
    ) {
        let chunk = &self.chunks[&job.key];
        // Taken, not borrowed: a ground solve must stay free to run Rayon work on this thread.
        let mut parked = PARKED_ORDER.take();
        parked.clear();
        parked.extend(chunk.parked.iter().map(|&handle| {
            let p = self.place_handles.resolve(handle).expect("parked bay");
            let v = self
                .vehicle_handles
                .resolve(self.places.occupant[p])
                .expect("parked car");
            (self.vehicles.model[v] * 10 + self.vehicles.colour[v], handle, p, v)
        }));
        parked.sort_unstable();
        let shared = job.chunk.as_mut().expect("a prepared job");
        let payload = Arc::get_mut(shared).expect("a job's buffers are its own");
        payload.clear();
        payload.version = chunk.version + 1;
        for &(bucket, _, p, v) in &parked {
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
                job.solved.push((p, model, solved));
                solved
            };
            let identity = self.places.occupant[p].to_bits() | (1_u64 << 62);
            payload.push(bucket, &grounded, identity);
        }
        PARKED_ORDER.set(parked);
    }

    /// Installs built chunks under one new serial, with the poses they solved, and keeps each
    /// chunk's previous publication as its spare. O(D + (P + D) log (P + D)), P pending chunks.
    pub(crate) fn install_dirty_chunks(&mut self, mut jobs: Vec<ChunkJob>) {
        if self.dirty.is_empty() {
            self.jobs = jobs;
            return;
        }
        self.publication.serial += 1;
        let serial = self.publication.serial;
        for job in &mut jobs[..self.dirty.len()] {
            for (p, model, pose) in job.solved.drain(..) {
                self.places.ground[p] = pose;
                self.places.ground_model[p] = model;
            }
            if job.solved.capacity() > SOLVED_KEPT {
                job.solved = Vec::new();
            }
            let (key, built) = (job.key, job.chunk.take().expect("a built job"));
            let chunk = self.chunks.get_mut(&key).expect("dirty chunk");
            chunk.version = built.version;
            chunk.dirty = false;
            chunk.spare = self.published.insert(key, Arc::clone(&built));
            let chunk = built;
            self.pending.push(ParkedUpdate { key, serial, chunk });
        }
        // Newest first within a chunk, so the older update is the one dropped.
        self.pending
            .sort_unstable_by_key(|update| (update.key, Reverse(update.serial)));
        self.pending.dedup_by_key(|update| update.key);
        self.pending_changed = true;
        self.dirty.clear();
        self.jobs = jobs;
    }

    /// Points a snapshot about to be rebuilt at the current publication, so that the list it held
    /// frees its chunks before this frame's rebuild wants them. O(1), allocation-free.
    pub(crate) fn release_snapshot(&self, parked: &mut ParkedPublication) {
        parked.clone_from(&self.publication);
    }

    /// Applies the renderer's answers since the last snapshot: drops acknowledged chunks and
    /// lists every chunk again on a full request, and lets the spare list go of its chunks.
    /// O(1) without answers, O(pending chunks) on an acknowledgement, O(all chunks) on a renderer
    /// reconnect, plus O(spare list) once after a change.
    pub(crate) fn take_renderer_answers(&mut self) {
        if let Some(list) = Arc::get_mut(&mut self.spare_updates) {
            list.clear();
        }
        let installed = self.publication.ack.installed.load(Ordering::Relaxed);
        if installed != self.acknowledged {
            self.acknowledged = installed;
            let listed = self.pending.len();
            self.pending.retain(|update| update.serial > installed);
            self.pending_changed |= self.pending.len() != listed;
        }
        if self.publication.ack.full.swap(false, Ordering::Relaxed) {
            self.publication.serial += 1;
            let serial = self.publication.serial;
            self.publication.full_serial = serial;
            // Every pending chunk is a published one, so this is the whole list, in chunk order.
            self.pending.clear();
            for (&key, chunk) in &self.published {
                let chunk = Arc::clone(chunk);
                self.pending.push(ParkedUpdate { key, serial, chunk });
            }
            self.pending_changed = true;
        }
    }

    /// The publication for a snapshot. O(1) when no chunk changed, else O(pending chunks) `Arc`
    /// copies: buffers are never copied, and the list is built once for every snapshot after.
    /// Two lists take turns, as the sim thread's two snapshots do; a third is made only if a
    /// snapshot outlives them.
    pub(crate) fn publication(&mut self) -> ParkedPublication {
        if std::mem::take(&mut self.pending_changed) {
            if Arc::get_mut(&mut self.spare_updates).is_none() {
                self.spare_updates = Arc::default();
                #[cfg(test)]
                {
                    self.buffer_creations += 1;
                }
            }
            let list = Arc::get_mut(&mut self.spare_updates).expect("the spare list is free");
            list.clear();
            list.extend_from_slice(&self.pending);
            std::mem::swap(&mut self.publication.updates, &mut self.spare_updates);
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
                .map(|update| (update.key, Arc::clone(&update.chunk)))
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
            for (_, poses_of_bucket, ids) in chunk.buckets() {
                for (&id, &pose) in ids.iter().zip(poses_of_bucket.as_chunks::<12>().0) {
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

    /// Chunk buffers and update lists made so far because no spare was free.
    #[cfg(test)]
    pub(crate) fn buffer_creations(&self) -> usize {
        self.buffer_creations
    }

    /// Every chunk's parked list against a scan of its bays. O(places), test builds only.
    #[cfg(test)]
    pub(crate) fn check_parked_lists(&self) -> Result<(), String> {
        self.chunks
            .iter()
            .try_for_each(|(&key, chunk)| self.check_parked_list(key, chunk))
    }

    // Reciprocal, as `Place` and `Vehicle` are: a bay is listed exactly when a car stands on it,
    // at the slot it records, and nothing else is listed.
    #[cfg(test)]
    fn check_parked_list(&self, key: (i32, i32), chunk: &data::Chunk) -> Result<(), String> {
        let mut listed = 0;
        for &handle in &chunk.places {
            let p = self.place_handles.resolve(handle).ok_or("stale chunk bay")?;
            let car = self.vehicle_handles.resolve(self.places.occupant[p]);
            let slot = self.places.parked_slot[p];
            if car.is_some_and(|v| !self.vehicles.moving[v]) {
                listed += 1;
                if chunk.parked.get(slot as usize) != Some(&handle) {
                    return Err(format!("{key:?}: parked bay {handle:?} is not at slot {slot}"));
                }
            } else if slot != NOT_PARKED {
                return Err(format!("{key:?}: bay {handle:?} is listed with no parked car"));
            }
        }
        if listed != chunk.parked.len() {
            return Err(format!("{key:?}: {} listed, {listed} parked", chunk.parked.len()));
        }
        Ok(())
    }

    /// The update of one listed chunk.
    #[cfg(test)]
    pub(crate) fn pending_update(&self, key: (i32, i32)) -> &ParkedUpdate {
        self.pending
            .iter()
            .find(|update| update.key == key)
            .expect("a listed chunk")
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
