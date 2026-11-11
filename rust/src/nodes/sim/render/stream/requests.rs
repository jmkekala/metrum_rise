// SPDX-License-Identifier: GPL-2.0-only

//! The graded tile bake request queue (`TERRAIN-13` part 1, round G1). The renderer hands over
//! every tile it can install once, with its level and token; the queue holds it pending (a few
//! bytes) and starts bakes nearest the view first as in-flight places free, at most
//! [`MAX_IN_FLIGHT`] at once. A newer request for a tile replaces its pending one and a cancel
//! drops it, so a re-levelled or cancelled tile is never baked at the old request. A request
//! whose tile and level is still in flight (an edit's newer token) waits until that bake is
//! polled.
//!
//! Pending tiles are kept per cell of the 512 m chunk grid (8 x 8 tiles, as
//! `network::graph::spatial::CHUNK_SIZE`). Starting bakes is a best-first walk: chunks in order
//! of their nearest point, a chunk's tiles entering a heap of exact distances once no unopened
//! chunk can hold a nearer tile. Issue order: ground distance from the view to the tile's
//! centre, then tile key. A start costs `O(c log c + t log t)` for `c` chunks holding pending
//! tiles and `t` tiles of the chunks opened; a push, replace or cancel `O(1)`. Nothing runs
//! unless a request arrived or an in-flight place freed since the last start (a still frame
//! costs one flag test).

use super::view::StreamView;
use crate::simulation::terrain::graded::{Level, TILE_M, TileKey};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// Most tiles in flight at once, from the bake's start until Godot polls its result (baking or
/// finished): the bound on the payloads Rust holds and the packing buffers its pool keeps
/// (`TERRAIN-07-G` round 6: a finished result left the count before its poll, so the results
/// queued behind Godot's install budget after a load grew without bound).
pub(crate) const MAX_IN_FLIGHT: usize = 96;
// Tiles per side of a chunk of the 512 m chunk grid.
const CHUNK_TILES: i32 = 8;

/// A tile bake: the tile, its level and the token Godot compares on arrival.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TileRequest {
    /// The tile.
    pub tile: TileKey,
    /// The level to bake it at.
    pub level: Level,
    /// Godot's token (an edit serial; 0 before any edit).
    pub token: i64,
}

// A pending request and its tile's index in its chunk's list.
#[derive(Clone, Copy, Debug)]
struct Pending {
    level: Level,
    token: i64,
    slot: u32,
}

// A tile ready to start, ordered nearest first, then by key (a max-heap pops the greatest).
#[derive(Clone, Copy, Debug)]
struct Candidate {
    distance_sq: f64,
    tile: TileKey,
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance_sq
            .total_cmp(&self.distance_sq)
            .then_with(|| (other.tile.x, other.tile.z).cmp(&(self.tile.x, self.tile.z)))
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Candidate {}

/// The work starts did (tests read it: invariant 3, zero work on a still frame).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct QueueWork {
    /// Starts that walked the chunks.
    pub walks: u64,
    /// Chunks opened by those walks.
    pub chunks_opened: u64,
    /// Bakes started.
    pub started: u64,
}

/// Pending tile requests and the bakes in flight.
#[derive(Default)]
pub(crate) struct TileRequestQueue {
    view: StreamView,
    pending: Map<TileKey, Pending>,
    // Chunk -> its pending tiles; a tile's `slot` is its index there.
    chunks: Map<(i32, i32), Vec<TileKey>>,
    spare: Vec<Vec<TileKey>>,
    in_flight: HashSet<(TileKey, Level), foldhash::fast::FixedState>,
    // A request arrived or a place freed since the last start.
    dirty: bool,
    // Reused by the starts: chunks by their nearest point, and the candidates.
    order: Vec<(f64, (i32, i32))>,
    heap: BinaryHeap<Candidate>,
    work: QueueWork,
}

fn chunk_of(tile: TileKey) -> (i32, i32) {
    (tile.x.div_euclid(CHUNK_TILES), tile.z.div_euclid(CHUNK_TILES))
}

fn chunk_rect((cx, cz): (i32, i32)) -> [f64; 4] {
    let side = f64::from(CHUNK_TILES) * TILE_M;
    let (x, z) = (f64::from(cx) * side, f64::from(cz) * side);
    [x, z, x + side, z + side]
}

impl TileRequestQueue {
    /// Sets the view the starts measure from (the renderer's camera, as the levels'). `O(1)`.
    pub fn set_view(&mut self, camera: [f64; 3]) {
        self.view.move_to(camera);
    }

    /// Queues `request`, replacing its tile's pending request. Returns the replaced request's
    /// level, which will not be baked. `O(1)`.
    pub fn push(&mut self, request: TileRequest) -> Option<Level> {
        let replaced = self.cancel(request.tile);
        let chunk = chunk_of(request.tile);
        let spare = &mut self.spare;
        let tiles = self
            .chunks
            .entry(chunk)
            .or_insert_with(|| spare.pop().unwrap_or_default());
        let slot = tiles.len() as u32;
        tiles.push(request.tile);
        self.pending.insert(
            request.tile,
            Pending {
                level: request.level,
                token: request.token,
                slot,
            },
        );
        self.dirty = true;
        replaced
    }

    /// Drops `tile`'s pending request. Returns its level. A bake in flight runs on; Godot
    /// ignores its result. `O(1)`.
    pub fn cancel(&mut self, tile: TileKey) -> Option<Level> {
        let pending = self.pending.remove(&tile)?;
        let chunk = chunk_of(tile);
        let tiles = self.chunks.get_mut(&chunk).expect("a pending tile's chunk");
        tiles.swap_remove(pending.slot as usize);
        if let Some(&moved) = tiles.get(pending.slot as usize) {
            self.pending
                .get_mut(&moved)
                .expect("a chunk's tiles are pending")
                .slot = pending.slot;
        }
        if tiles.is_empty() {
            let tiles = self.chunks.remove(&chunk).expect("present");
            self.spare.push(tiles);
        }
        Some(pending.level)
    }

    /// Drops every pending request (the renderer reset); bakes in flight run on.
    pub fn clear_pending(&mut self) {
        self.pending.clear();
        for (_, mut tiles) in self.chunks.drain() {
            tiles.clear();
            self.spare.push(tiles);
        }
    }

    /// A bake left the in-flight count (polled, or failed). `O(1)`.
    pub fn finish(&mut self, tile: TileKey, level: Level) {
        if self.in_flight.remove(&(tile, level)) {
            self.dirty = true;
        }
    }

    /// The pending requests.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// The bakes in flight.
    pub fn in_flight_len(&self) -> usize {
        self.in_flight.len()
    }

    /// The work the starts did so far.
    #[cfg(test)]
    pub fn work(&self) -> QueueWork {
        self.work
    }

    /// Starts the nearest pending requests the free in-flight places take, appending them to
    /// `out` nearest first; each counts in flight until [`Self::finish`]. A request whose tile
    /// and level is in flight is skipped. Does nothing unless a request arrived or a place
    /// freed since the last call.
    pub fn start(&mut self, out: &mut Vec<TileRequest>) {
        if !std::mem::take(&mut self.dirty) || self.pending.is_empty() {
            return;
        }
        let free = MAX_IN_FLIGHT.saturating_sub(self.in_flight.len());
        if free == 0 {
            return;
        }
        self.work.walks += 1;
        self.order.clear();
        let view = self.view;
        self.order.extend(
            self.chunks
                .keys()
                .map(|&chunk| (view.rect_ground_distance_sq(chunk_rect(chunk)), chunk)),
        );
        self.order
            .sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        self.heap.clear();
        let mut next = 0;
        let mut started = 0;
        while started < free {
            let open = next < self.order.len()
                && self
                    .heap
                    .peek()
                    .is_none_or(|top| self.order[next].0 <= top.distance_sq);
            if open {
                // No tile of a later chunk can be nearer than this chunk's nearest point.
                self.work.chunks_opened += 1;
                for &tile in &self.chunks[&self.order[next].1] {
                    let level = self.pending[&tile].level;
                    if !self.in_flight.contains(&(tile, level)) {
                        self.heap.push(Candidate {
                            distance_sq: view.tile_ground_distance_sq(tile),
                            tile,
                        });
                    }
                }
                next += 1;
                continue;
            }
            let Some(candidate) = self.heap.pop() else {
                break;
            };
            let Pending { level, token, .. } = self.pending[&candidate.tile];
            self.cancel(candidate.tile);
            self.in_flight.insert((candidate.tile, level));
            out.push(TileRequest {
                tile: candidate.tile,
                level,
                token,
            });
            started += 1;
        }
        self.work.started += started as u64;
    }
}

#[cfg(test)]
mod tests;
