// SPDX-License-Identifier: GPL-2.0-only

//! The drawn ground's lattice blocks kept from pose to pose (`ROAD-123` round HF3). A drag's
//! poses all read the same published view, and its footprint moves only a little from pose to
//! pose, so most of the `GL0` lattice points a pose's hue reads (round HF's lattice reader,
//! [`GradedSurface::ground_at_lattice`]) were read by the pose before it. The reader's 8 m
//! blocks (each its coverage raster, its evaluated vertices and its reads) are kept while the
//! view's ground is the same ([`GroundKey`]), so a pose evaluates only the points its
//! predecessor did not read and prepares only the blocks new to it. A read is a pure function of
//! the view and the lattice point, so a kept value is the bits a fresh read gives.
//!
//! Kept: the blocks of the last pose. Cost per pose: `O(B log B)` for the `B` blocks it reads
//! (their lookup and the kept set's update) plus the reads the blocks do not hold yet. Every
//! buffer is reused, so a pose allocates nothing once they have grown (to the most blocks two
//! poses in turn read).

use crate::nodes::sim::core::GroundKey;
use crate::simulation::terrain::graded::{GradedSurface, LatticeBlock, QueryScratch};

use super::surface::GROUND_LATTICE_M;

/// The lattice blocks the last pose read, for the next pose on the same view's ground.
#[derive(Default)]
pub(crate) struct LatticeCache {
    // The ground the kept blocks read, `None` before the first pose.
    key: Option<GroundKey>,
    blocks: Vec<LatticeBlock>,
    // The last pose's blocks by key (`(key, slot)`, sorted), this pose's in the order read, and
    // the slots neither holds.
    kept: Vec<(u64, u32)>,
    read: Vec<(u64, u32)>,
    free: Vec<u32>,
    // The block the last read was in, and the highest key read this pose.
    current: Option<(u64, u32)>,
    highest: Option<u64>,
}

impl std::fmt::Debug for LatticeCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LatticeCache")
            .field("key", &self.key)
            .field("blocks", &self.blocks.len())
            .finish()
    }
}

impl LatticeCache {
    /// Starts a pose reading the ground of `key`: the kept blocks stay only when it is the
    /// ground they read. `O(1)`, or `O(kept blocks)` when the ground changed.
    pub(crate) fn begin(&mut self, key: GroundKey) {
        if self.key != Some(key) {
            self.key = Some(key);
            self.free.extend(self.kept.drain(..).map(|(_, slot)| slot));
            for &slot in &self.free {
                self.blocks[slot as usize].forget();
            }
        }
        self.read.clear();
        self.current = None;
        self.highest = None;
    }

    /// The pose's drawn-ground reader (the hue's `drawn`, round HU3): bitwise
    /// `GradedSurface::ground_height_near` at the `GL0` lattice points, through the block that
    /// holds each point, kept or new; elsewhere the point query.
    pub(crate) fn reader<'a>(
        &'a mut self,
        surface: &'a GradedSurface<'a>,
        scratch: &'a mut QueryScratch,
    ) -> impl FnMut(f64, f64) -> f64 + 'a {
        move |x, z| {
            let [i, j] = [x, z].map(|value| (value / GROUND_LATTICE_M).round() as i64);
            if i as f64 * GROUND_LATTICE_M != x || j as f64 * GROUND_LATTICE_M != z {
                return surface.ground_height_at(x, z, scratch);
            }
            let slot = self.block(LatticeBlock::origin_of([i, j]));
            surface.ground_at_lattice([i, j], &mut self.blocks[slot as usize], scratch)
        }
    }

    /// Ends the pose: keeps the blocks it read and frees the rest. `O(B log B)`.
    pub(crate) fn end(&mut self) {
        self.read.sort_unstable();
        self.read.dedup();
        // The kept blocks this pose did not read are free again; both lists are sorted.
        let mut read = self.read.iter().peekable();
        for &(key, slot) in &self.kept {
            while read.next_if(|&&(other, _)| other < key).is_some() {}
            if read.peek().is_none_or(|&&(other, _)| other != key) {
                self.free.push(slot);
            }
        }
        std::mem::swap(&mut self.kept, &mut self.read);
    }

    // The slot of the block at `origin`: the last read's, a kept one, or a new one. `O(log B)`.
    fn block(&mut self, origin: [i64; 2]) -> u32 {
        let key = block_key(origin);
        if let Some((current, slot)) = self.current
            && current == key
        {
            return slot;
        }
        // The hue reads block by block in key order, so a key above every key read this pose
        // is new to it; only one below is searched among them.
        let earlier = self.highest.is_some_and(|highest| key <= highest);
        let slot = match self.kept.binary_search_by_key(&key, |&(kept, _)| kept) {
            Ok(index) => self.kept[index].1,
            Err(_) if earlier => match self.read.iter().find(|&&(read, _)| read == key) {
                Some(&(_, slot)) => slot,
                None => self.fresh(),
            },
            Err(_) => self.fresh(),
        };
        self.read.push((key, slot));
        self.current = Some((key, slot));
        self.highest = Some(self.highest.map_or(key, |highest| highest.max(key)));
        slot
    }

    // A block holding nothing: a free slot, or a new one.
    fn fresh(&mut self) -> u32 {
        match self.free.pop() {
            Some(slot) => {
                self.blocks[slot as usize].forget();
                slot
            }
            None => {
                self.blocks.push(LatticeBlock::default());
                (self.blocks.len() - 1) as u32
            }
        }
    }
}

// An order-preserving key of a block's first lattice point (indices within `i32`).
fn block_key(origin: [i64; 2]) -> u64 {
    let half = |index: i64| u64::from(index as i32 as u32 ^ 0x8000_0000);
    (half(origin[0]) << 32) | half(origin[1])
}
