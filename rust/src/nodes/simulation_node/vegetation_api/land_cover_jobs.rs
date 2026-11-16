// SPDX-License-Identifier: GPL-2.0-only

//! Terrain land covers rebuilt on the background pool (`RENDER-40`): the terrain renderer asks
//! for a patch whose land cover went stale; one snapshot taker takes the asked patches'
//! [`PatchLandCover`] inputs under `try_lock` and bakes them off the core; the renderer's poll
//! hands over the ones current by the published revisions. Nothing of the query runs on Godot's
//! main thread, and nothing there waits for the core.
//!
//! Currency: a result's revisions are taken under the core with its inputs, and so is the
//! sequence of the core's latest publication (`FrameQueries`). At the poll, a result equal to
//! the published revisions is current and delivered. An unequal one is behind when the
//! publication is newer than its take (that publication saw the core at or after the take) and
//! its key waits for a new snapshot; otherwise the result is ahead of the publication and waits
//! for the next one. So a key is asked of the renderer once and delivered once, current.

use super::*;
use crate::nodes::sim::core::FrameQueries;
use std::collections::VecDeque;

/// The land covers asked for and the baked ones waiting for the poll.
#[derive(Default)]
pub(crate) struct LandCoverJobs {
    // Every live request: waiting, being taken and baked, or landed.
    asked: HashSet<Vector2i>,
    // Asked keys waiting for the next snapshot, in order of request.
    waiting: VecDeque<Vector2i>,
    // Baked results not yet delivered.
    landed: Vec<Landed>,
    // The snapshot taker is out.
    taking: bool,
    // Advanced by every clear: a taker's results from before it are dropped.
    world: u64,
}

// One baked land cover and the publication sequence the core held when its inputs were taken.
struct Landed {
    key: Vector2i,
    cover: PatchLandCover,
    sequence: u64,
}

// What a taker's batch came back as.
pub(super) struct Taken {
    world: u64,
    // Baked results with the sequence of their take.
    baked: Vec<(Vector2i, PatchLandCover)>,
    sequence: u64,
    // Keys the core could not answer yet (held, or the roadbed or site index not ready).
    busy: Vec<Vector2i>,
}

impl LandCoverJobs {
    /// Asks for `key`'s land cover; false when it is asked already. `O(1)`.
    pub(crate) fn request(&mut self, key: Vector2i) -> bool {
        if !self.asked.insert(key) {
            return false;
        }
        self.waiting.push_back(key);
        true
    }

    /// Drops `key`'s request: nothing is delivered for it. `O(waiting + landed)`.
    pub(crate) fn cancel(&mut self, key: Vector2i) {
        if self.asked.remove(&key) {
            self.waiting.retain(|&waiting| waiting != key);
            self.landed.retain(|landed| landed.key != key);
        }
    }

    /// Drops every request; results of snapshots already out are dropped as they land.
    pub(crate) fn clear(&mut self) {
        self.asked.clear();
        self.waiting.clear();
        self.landed.clear();
        self.world = self.world.wrapping_add(1);
    }

    /// Live requests: waiting, out or landed.
    pub(crate) fn asked(&self) -> usize {
        self.asked.len()
    }

    /// The landed results current by `published`, at most `max`, each with its key; behind ones
    /// send their key back to wait, ahead ones stay. `O(landed)`.
    pub(crate) fn deliver(
        &mut self,
        published: &FrameQueries,
        max: usize,
    ) -> Vec<(Vector2i, PatchLandCover)> {
        let mut delivered = Vec::new();
        let mut index = 0;
        while index < self.landed.len() {
            let landed = &self.landed[index];
            let revisions = published.land_cover_generations(landed.key);
            if *landed.cover.generations() == revisions {
                if delivered.len() == max {
                    index += 1;
                    continue;
                }
                let landed = self.landed.swap_remove(index);
                self.asked.remove(&landed.key);
                delivered.push((landed.key, landed.cover));
            } else if published.sequence > landed.sequence {
                let landed = self.landed.swap_remove(index);
                self.waiting.push_back(landed.key);
            } else {
                index += 1;
            }
        }
        delivered
    }

    // The keys of the next snapshot, all waiting ones, when no taker is out; marks it out.
    pub(super) fn start(&mut self) -> Option<(u64, Vec<Vector2i>)> {
        if self.taking || self.waiting.is_empty() {
            return None;
        }
        self.taking = true;
        Some((self.world, self.waiting.drain(..).collect()))
    }

    // Lands a taker's batch; whether keys wait that arrived while it was out (start another).
    // Results of an older world and of cancelled keys are dropped; busy keys go back to the
    // front, in order, for the next poll or request to try.
    pub(super) fn land(&mut self, taken: Taken) -> bool {
        self.taking = false;
        if taken.world != self.world {
            return !self.waiting.is_empty();
        }
        for (key, cover) in taken.baked {
            if self.asked.contains(&key) {
                self.landed.push(Landed {
                    key,
                    cover,
                    sequence: taken.sequence,
                });
            }
        }
        let retry = taken.busy.is_empty();
        for key in taken.busy.into_iter().rev() {
            if self.asked.contains(&key) {
                self.waiting.push_front(key);
            }
        }
        retry && !self.waiting.is_empty()
    }
}

/// What a land cover taker needs: the core, the jobs and the pool's tracker.
#[derive(Clone)]
pub(crate) struct LandCoverSpawner {
    /// The simulation core the inputs are taken from.
    pub(crate) core: Arc<Mutex<SimCore>>,
    /// The land cover jobs.
    pub(crate) jobs: Arc<Mutex<LandCoverJobs>>,
    /// The background pool's job tracker.
    pub(crate) async_jobs: Arc<super::super::async_jobs::AsyncJobs>,
}

/// Starts the snapshot taker when keys wait and none is out. It takes the core once under
/// `try_lock` and never waits: a busy core puts the batch back, and the next request or poll
/// tries again (a held core is not spun on). Inputs are taken in that one hold, the land
/// covers baked after it is released, sequentially. `O(1)` on the calling thread.
pub(crate) fn pump(spawner: &LandCoverSpawner) {
    let Some((world, keys)) = spawner.jobs.lock().unwrap().start() else {
        return;
    };
    let next = spawner.clone();
    spawner.async_jobs.spawn(move || {
        let taken = take_and_bake(&next.core, world, keys);
        if next.jobs.lock().unwrap().land(taken) {
            pump(&next);
        }
    });
}

// One taker's work: the inputs of every key under one `try_lock`, then the bakes off the core.
pub(super) fn take_and_bake(core: &Mutex<SimCore>, world: u64, keys: Vec<Vector2i>) -> Taken {
    let mut taken = Taken {
        world,
        baked: Vec::with_capacity(keys.len()),
        sequence: 0,
        busy: Vec::new(),
    };
    {
        let Some(mut core) = SimulationNode::try_lock_shared_core(core) else {
            taken.busy = keys;
            return taken;
        };
        taken.sequence = core.frame_queries().sequence;
        for key in keys {
            match PatchLandCover::take(&mut core, key) {
                Some(cover) => taken.baked.push((key, cover)),
                // Outside the world nothing is ever delivered: the request ends unanswered.
                None if !PatchLandCover::in_world(&core, key) => {}
                None => taken.busy.push(key),
            }
        }
    }
    for (_, cover) in &mut taken.baked {
        cover.bake();
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(x: i32) -> Vector2i {
        Vector2i::new(x, 0)
    }

    #[test]
    fn a_key_is_asked_once_and_a_cancel_or_clear_ends_it() {
        let mut jobs = LandCoverJobs::default();
        assert!(jobs.request(key(1)));
        assert!(!jobs.request(key(1)));
        assert!(jobs.request(key(2)));
        assert_eq!(jobs.asked(), 2);
        jobs.cancel(key(1));
        assert_eq!(jobs.asked(), 1);
        assert!(jobs.request(key(1)), "a cancelled key may be asked again");
        let (world, keys) = jobs.start().unwrap();
        assert_eq!(keys, [key(2), key(1)]);
        assert!(jobs.start().is_none(), "one taker at a time");
        jobs.clear();
        assert_eq!(jobs.asked(), 0);
        // The taker of the cleared world lands nothing and frees the taker.
        let stale = Taken {
            world,
            baked: Vec::new(),
            sequence: 0,
            busy: vec![key(2)],
        };
        assert!(!jobs.land(stale));
        assert!(jobs.waiting.is_empty());
        assert!(jobs.request(key(3)));
        assert!(jobs.start().is_some());
    }

    #[test]
    fn a_busy_batch_goes_back_in_order_and_waits_for_the_next_pump() {
        let mut jobs = LandCoverJobs::default();
        jobs.request(key(1));
        jobs.request(key(2));
        let (world, keys) = jobs.start().unwrap();
        jobs.request(key(3));
        let busy = Taken {
            world,
            baked: Vec::new(),
            sequence: 0,
            busy: keys,
        };
        // Busy: no new taker of its own, the batch first in the queue again.
        assert!(!jobs.land(busy));
        assert_eq!(Vec::from(jobs.waiting.clone()), [key(1), key(2), key(3)]);
        let (world, keys) = jobs.start().unwrap();
        assert_eq!(keys.len(), 3);
        jobs.request(key(4));
        let done = Taken {
            world,
            baked: Vec::new(),
            sequence: 0,
            busy: Vec::new(),
        };
        // A taker that answered starts the next for keys asked meanwhile.
        assert!(jobs.land(done));
    }
}
