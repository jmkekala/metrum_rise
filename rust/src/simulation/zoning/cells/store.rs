// SPDX-License-Identifier: GPL-2.0-only

//! Compact oriented cell blocks indexed using the existing 512 m world chunk convention.

mod alignment;
mod chunks;
mod maps;
mod sources;

pub(crate) use alignment::RoadCellAlignment;
pub(crate) use maps::{CellMaps, LiveMaps, SharedMaps};

use super::sources::{CellCurveSource, CurveSourceKey};
use super::{
    BLOCK_CELLS, BLOCK_SIDE, BlockKey, CellBounds, CellKey, CellLot, CellRoadFrontage, GridFrame,
};
use crate::simulation::network::graph::RegionGraph;
use glam::DVec2;
use maps::{FrameList, KeyLog, StoreMap};
use std::sync::Arc;

#[derive(Clone, Debug)]
struct CellBlock {
    valid: u64,
    reserved: u64,
    profiles: [u16; BLOCK_CELLS],
    // A lot retains the claim while its building is in redevelopment grace, even when the
    // underlying paint has been erased. An unpainted, unclaimed cell reserves nothing.
    lots: [u64; BLOCK_CELLS],
}

impl Default for CellBlock {
    fn default() -> Self {
        Self {
            valid: 0,
            reserved: 0,
            profiles: [0; BLOCK_CELLS],
            lots: [0; BLOCK_CELLS],
        }
    }
}

impl CellBlock {
    fn refresh_reservation(&mut self, offset: usize) {
        let bit = 1_u64 << offset;
        if self.profiles[offset] != 0 || self.lots[offset] != 0 {
            self.reserved |= bit;
        } else {
            self.reserved &= !bit;
        }
    }
}

#[derive(Clone, Debug, Default)]
struct CellChunk {
    blocks: Vec<BlockKey>,
    sources: Vec<CurveSourceKey>,
    revision: u64,
    generated: bool,
    // None with generated=false means a cold/full rebuild; otherwise this is the union
    // of invalidated world bounds clipped to this previously completed chunk.
    dirty_bounds: Option<CellBounds>,
}

// The keys a store wrote since the road tool view's copy last followed it (`ROAD-68` open
// question 32). Frames are append-only between clears, so the copy takes the new suffix, and
// road alignments are already persistent, so it shares them.
#[derive(Clone, Debug, Default)]
struct CellJournal {
    // A clear or a whole-world invalidation since the last drain: the copy is rebuilt whole.
    whole: bool,
    blocks: KeyLog<BlockKey>,
    chunks: KeyLog<(i32, i32)>,
    frontages: KeyLog<CellKey>,
    sources: KeyLog<CurveSourceKey>,
}

impl CellJournal {
    fn clear(&mut self) {
        self.whole = false;
        self.blocks.clear();
        self.chunks.clear();
        self.frontages.clear();
        self.sources.clear();
    }
}

/// Sparse authoritative paint and lot coverage; geometry is derived from persistent frames.
/// Generic over its map kind ([`CellMaps`]): the live store keeps plain maps ([`LiveMaps`]);
/// the road tool's view keeps a persistent copy ([`SharedMaps`]) that each publication brings
/// up to date from the live store's write journal ([`CellStore::follow`]), so every algorithm
/// on the store, lot planning included, runs one function on either (`ROAD-68` open question
/// 32). Every write records its key in that journal.
#[derive(Clone, Debug)]
pub(crate) struct CellStore<M: CellMaps = LiveMaps> {
    frames: M::Frames,
    frame_ids: M::Map<GridFrame, u64>,
    blocks: M::Map<BlockKey, CellBlock>,
    chunks: M::Map<(i32, i32), CellChunk>,
    // Warm chunks invalidated since the last drain; bounded by the edits' footprints.
    stale_chunks: Vec<(i32, i32)>,
    frontages: M::Map<CellKey, Vec<CellRoadFrontage>>,
    curve_sources: M::Map<CurveSourceKey, Arc<CellCurveSource>>,
    road_alignments: imbl::HashMap<usize, RoadCellAlignment>,
    journal: CellJournal,
    painted_count: usize,
    claimed_count: usize,
    revision: u64,
}

// Only the live kind has a default, so `CellStore::default()` names the live store; the view's
// copy starts from [`CellStore::copy_of`].
impl Default for CellStore {
    fn default() -> Self {
        Self::empty()
    }
}

impl CellStore<SharedMaps> {
    /// A persistent copy of `live`, with an empty journal. O(entries log entries).
    pub(crate) fn copy_of(live: &CellStore) -> Self {
        let mut copy = Self::empty();
        for frame in live.frames.iter() {
            copy.frames.push(frame);
        }
        for (&frame, &id) in live.frame_ids.iter() {
            copy.frame_ids.insert(frame, id);
        }
        for (&key, block) in live.blocks.iter() {
            copy.blocks.insert(key, block.clone());
        }
        for (&key, chunk) in live.chunks.iter() {
            copy.chunks.insert(key, chunk.clone());
        }
        for (&key, links) in live.frontages.iter() {
            copy.frontages.insert(key, links.clone());
        }
        for (&key, source) in live.curve_sources.iter() {
            copy.curve_sources.insert(key, Arc::clone(source));
        }
        copy.follow_scalars(live);
        copy
    }

    /// Brings this copy up to date with `live`, of which it was a copy when `live`'s journal
    /// was last drained, and drains the journal: every journaled entry is copied or removed,
    /// frames after the copy's last are appended, alignments are shared. Exact, never stale.
    /// O(written keys * log entries); a clear or whole-world invalidation copies `live` whole.
    pub(crate) fn follow(&mut self, live: &mut CellStore) {
        if live.journal.whole {
            *self = Self::copy_of(live);
            live.journal.clear();
            return;
        }
        for index in self.frames.len()..live.frames.len() {
            let frame = live.frames[index];
            self.frames.push(frame);
            self.frame_ids.insert(frame, index as u64 + 1);
        }
        fn copy_entries<K, V>(
            log: &mut KeyLog<K>,
            from: &impl StoreMap<K, V>,
            to: &mut impl StoreMap<K, V>,
        ) where
            K: Copy + Ord,
            V: Clone,
        {
            for key in log.drain_sorted() {
                match from.get(&key) {
                    Some(value) => to.insert(key, value.clone()),
                    None => to.remove(&key),
                }
            }
        }
        let journal = &mut live.journal;
        copy_entries(&mut journal.blocks, &live.blocks, &mut self.blocks);
        copy_entries(&mut journal.chunks, &live.chunks, &mut self.chunks);
        copy_entries(&mut journal.frontages, &live.frontages, &mut self.frontages);
        copy_entries(
            &mut journal.sources,
            &live.curve_sources,
            &mut self.curve_sources,
        );
        self.follow_scalars(live);
    }

    // Shares the alignments and copies the counts and revision; the stale-chunk list stays the
    // live overlay's.
    fn follow_scalars(&mut self, live: &CellStore) {
        self.road_alignments = live.road_alignments.clone();
        self.painted_count = live.painted_count;
        self.claimed_count = live.claimed_count;
        self.revision = live.revision;
    }
}

impl CellStore {
    /// Forgets the journal after the road tool's view took a whole copy of this store.
    pub(crate) fn clear_journal(&mut self) {
        self.journal.clear();
    }
}

impl<M: CellMaps> CellStore<M> {
    fn empty() -> Self {
        Self {
            frames: Default::default(),
            frame_ids: Default::default(),
            blocks: Default::default(),
            chunks: Default::default(),
            stale_chunks: Vec::new(),
            frontages: Default::default(),
            curve_sources: Default::default(),
            road_alignments: imbl::HashMap::new(),
            journal: CellJournal::default(),
            painted_count: 0,
            claimed_count: 0,
            revision: 0,
        }
    }

    /// Visits persistent frames in id order for serialization, including temporarily empty ones.
    pub(crate) fn saved_frames(&self) -> impl Iterator<Item = (u64, GridFrame)> + '_ {
        self.frames
            .iter()
            .enumerate()
            .map(|(i, frame)| (i as u64 + 1, frame))
    }

    /// Captures only paint and occupied coverage; generated empty geometry is a rebuildable cache.
    pub(crate) fn saved_cells(&self) -> Vec<(CellKey, u16)> {
        let mut result = Vec::new();
        for (&key, block) in self.blocks.iter() {
            let mut mask = block.valid;
            while mask != 0 {
                let offset = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                if block.profiles[offset] != 0 || block.lots[offset] != 0 {
                    result.push((key.cell(offset), block.profiles[offset]));
                }
            }
        }
        result.sort_unstable_by_key(|&(key, _)| key);
        result
    }

    /// Test-only digests, independent of map order: the authority (frames, and every cell's
    /// paint and lot claim) and the generated cache (valid cells and road frontages).
    #[cfg(test)]
    pub(crate) fn digests(&self) -> (u64, u64) {
        use crate::debug::work::debug_hash;
        let mut claims = Vec::new();
        let mut valid = Vec::new();
        for (&key, block) in self.blocks.iter() {
            valid.push((key, block.valid));
            for offset in 0..BLOCK_CELLS {
                if block.profiles[offset] != 0 || block.lots[offset] != 0 {
                    claims.push((key.cell(offset), block.profiles[offset], block.lots[offset]));
                }
            }
        }
        claims.sort_unstable();
        valid.sort_unstable();
        let mut frontages: Vec<_> = self.frontages.iter().collect();
        frontages.sort_unstable();
        let frames: Vec<_> = self.frames.iter().collect();
        (
            debug_hash(&(&frames, claims)),
            debug_hash(&(valid, frontages)),
        )
    }

    /// Test-only digest of everything a copy must equal, independent of map order: frames,
    /// every block, chunk entry (its lists in their order), frontage list and curve source,
    /// the alignments, counts and revision. The stale-chunk list and the journal are left out.
    #[cfg(test)]
    pub(crate) fn state_digest(&self) -> u64 {
        fn sorted<'a, K: Ord + 'a, V: 'a>(
            entries: impl Iterator<Item = (&'a K, &'a V)>,
        ) -> Vec<(&'a K, &'a V)> {
            let mut entries: Vec<_> = entries.collect();
            entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
            entries
        }
        let frames: Vec<_> = self.frames.iter().collect();
        let mut frame_ids: Vec<_> = self.frame_ids.iter().map(|(&f, &id)| (id, f)).collect();
        frame_ids.sort_unstable_by_key(|&(id, _)| id);
        let blocks = sorted(self.blocks.iter());
        let chunks = sorted(self.chunks.iter());
        let frontages = sorted(self.frontages.iter());
        let sources = sorted(self.curve_sources.iter());
        let alignments = self.saved_road_alignments();
        crate::debug::work::debug_hash(&(
            (
                frames, frame_ids, blocks, chunks, frontages, sources, alignments,
            ),
            (self.painted_count, self.claimed_count, self.revision),
        ))
    }

    /// Test-only memory estimate (`ROAD-68` open question 32): the entries of every map and the
    /// bytes of their keys, values and owned lists, as either kind stores them, without map
    /// nodes, spare capacity or `Arc` headers. Frames count twice (list and id map).
    #[cfg(test)]
    pub(crate) fn memory_estimate(&self) -> (usize, usize) {
        use std::mem::size_of;
        let mut entries = 2 * self.frames.len();
        let mut bytes = entries * size_of::<GridFrame>() + self.frames.len() * size_of::<u64>();
        for (_, block) in self.blocks.iter() {
            entries += 1;
            bytes += size_of::<BlockKey>() + size_of_val(block);
        }
        for (_, chunk) in self.chunks.iter() {
            entries += 1;
            bytes += size_of::<(i32, i32)>()
                + size_of_val(chunk)
                + chunk.blocks.len() * size_of::<BlockKey>()
                + chunk.sources.len() * size_of::<CurveSourceKey>();
        }
        for (_, links) in self.frontages.iter() {
            entries += 1;
            bytes += size_of::<CellKey>()
                + size_of_val(links)
                + links.len() * size_of::<CellRoadFrontage>();
        }
        for (_, source) in self.curve_sources.iter() {
            entries += 1;
            bytes += size_of::<CurveSourceKey>() + size_of_val(source);
        }
        (entries, bytes)
    }

    /// Restores consecutive frame identities, rejecting duplicates and invalid serialized bases.
    pub(crate) fn restore_saved_frame(&mut self, grid: u64, frame: GridFrame) -> bool {
        if grid != self.frames.len() as u64 + 1
            || !frame.is_valid()
            || self.frame_ids.contains_key(&frame)
        {
            return false;
        }
        self.register_frame(frame);
        true
    }

    /// Restores one reserved cell, rejecting missing frames, duplicate addresses and overlap.
    pub(crate) fn restore_saved_cell(&mut self, key: CellKey, profile: u16) -> bool {
        let Some(frame) = self.frame(key.grid) else {
            return false;
        };
        if self.profile(key).is_some() {
            return false;
        }
        let corners = frame.corners(key.x, key.y);
        let bounds = CellBounds::from_points(corners);
        if !bounds.is_valid() {
            return false;
        }
        let mut overlaps = false;
        self.visit_in_bounds(bounds, |other| {
            if !overlaps && let Some(other_frame) = self.frame(other.grid) {
                overlaps =
                    super::interiors_overlap(&corners, &other_frame.corners(other.x, other.y));
            }
        });
        if overlaps || !self.insert(key) {
            return false;
        }
        self.set_profile(key, profile);
        true
    }

    /// Clears world-owned geometry and paint while invalidating outstanding gestures.
    pub(crate) fn clear(&mut self) {
        self.frames.clear();
        self.frame_ids.clear();
        self.blocks.clear();
        self.chunks.clear();
        self.frontages.clear();
        self.curve_sources.clear();
        self.road_alignments.clear();
        self.painted_count = 0;
        self.claimed_count = 0;
        self.journal.clear();
        self.journal.whole = true;
        self.bump_revision();
    }

    /// Returns a persistent id, reusing a frame with the same canonical orientation and phase.
    pub(crate) fn register_frame(&mut self, frame: GridFrame) -> u64 {
        if let Some(&id) = self.frame_ids.get(&frame) {
            return id;
        }
        let id = self.frames.len() as u64 + 1;
        self.frames.push(frame);
        self.frame_ids.insert(frame, id);
        id
    }

    /// Whether `frame` already has a persistent id.
    pub(super) fn has_frame(&self, frame: GridFrame) -> bool {
        self.frame_ids.contains_key(&frame)
    }

    /// Resolves a grid without depending on road storage order.
    pub(crate) fn frame(&self, grid: u64) -> Option<GridFrame> {
        let index = usize::try_from(grid.checked_sub(1)?).ok()?;
        self.frames.get(index)
    }

    /// Generation and paint edits advance this epoch; stale gestures cannot target new cells.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Constant-time idle/empty-world check used before scheduling asset-driven lot work.
    pub(crate) fn has_paint(&self) -> bool {
        self.painted_count != 0
    }

    /// Constant-time reservation presence, including fully erased occupied lots during grace.
    pub(crate) fn has_reservations(&self) -> bool {
        self.painted_count != 0 || self.claimed_count != 0
    }

    /// Visits painted storage tiles during an explicit whole-catalog refresh, without sorting cells.
    pub(crate) fn visit_painted_block_bounds(&self, mut visit: impl FnMut(CellBounds)) {
        for (&key, block) in self.blocks.iter() {
            let mut mask = block.valid;
            let mut painted = false;
            while mask != 0 {
                let index = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                if block.profiles[index] != 0 {
                    painted = true;
                    break;
                }
            }
            if painted && let Some(frame) = self.frame(key.grid) {
                visit(block_bounds(&frame.geometry(), key));
            }
        }
    }

    /// Returns current road-facing boundaries; rear cells and orphaned paint have none.
    pub(crate) fn frontages(&self, key: CellKey) -> &[CellRoadFrontage] {
        self.frontages.get(&key).map_or(&[], Vec::as_slice)
    }

    /// Replaces sorted transient road support and advances the epoch only when it changes.
    pub(super) fn set_frontages(&mut self, key: CellKey, mut frontages: Vec<CellRoadFrontage>) {
        frontages.sort_unstable_by_key(|link| {
            (link.boundary, link.start, link.end, link.edge, link.side)
        });
        frontages.dedup();
        if self.frontages(key) == frontages || self.profile(key).is_none() {
            return;
        }
        if frontages.is_empty() {
            self.frontages.remove(&key);
        } else {
            self.frontages.insert(key, frontages);
        }
        self.journal.frontages.push(key);
        self.bump_revision();
    }

    /// Publishes a cell after generation and reservation validation. Existing paint is retained.
    pub(super) fn insert(&mut self, key: CellKey) -> bool {
        let inserted = self.insert_cell::<true>(key);
        if inserted {
            self.revision = self.revision.wrapping_add(1);
            self.mark_cell_chunk_changed(key);
        }
        inserted
    }

    // [`Self::insert`] without advancing the store's revision, stamping the cell's rendering
    // chunk or journaling: for a generation's temporary candidate index, which nothing reads
    // after the generation.
    pub(super) fn insert_unstamped(&mut self, key: CellKey) -> bool {
        self.insert_cell::<false>(key)
    }

    fn insert_cell<const JOURNAL: bool>(&mut self, key: CellKey) -> bool {
        let Some(frame) = self.frame(key.grid) else {
            return false;
        };
        let block_key = key.block();
        let bit = 1_u64 << key.offset();
        match self.blocks.get(&block_key) {
            Some(block) if block.valid & bit != 0 => return false,
            Some(_) => {}
            None => {
                let bounds = block_bounds(&frame.geometry(), block_key);
                if !bounds.is_valid() {
                    return false;
                }
                for chunk in chunks_for_bounds(bounds) {
                    self.chunks.entry_or_default(chunk).blocks.push(block_key);
                    if JOURNAL {
                        self.journal.chunks.push(chunk);
                    }
                }
            }
        }
        self.blocks.entry_or_default(block_key).valid |= bit;
        if JOURNAL {
            self.journal.blocks.push(block_key);
        }
        true
    }

    /// Removes only unreserved cells. Painted/occupied geometry requires lifecycle handling.
    pub(super) fn remove_unreserved(&mut self, key: CellKey) -> bool {
        let block_key = key.block();
        let offset = key.offset();
        let bit = 1_u64 << offset;
        let Some(block) = self.blocks.get_mut_if(&block_key, |block| {
            block.valid & bit != 0 && block.profiles[offset] == 0 && block.lots[offset] == 0
        }) else {
            return false;
        };
        block.valid &= !bit;
        let remove_block = block.valid == 0;
        self.frontages.remove(&key);
        self.journal.frontages.push(key);
        self.journal.blocks.push(block_key);
        if remove_block {
            self.blocks.remove(&block_key);
            if let Some(frame) = self.frame(key.grid) {
                for chunk in chunks_for_bounds(block_bounds(&frame.geometry(), block_key)) {
                    if let Some(entry) = self.chunks.get_mut(&chunk) {
                        entry.blocks.retain(|candidate| *candidate != block_key);
                        self.journal.chunks.push(chunk);
                    }
                }
            }
        }
        self.revision = self.revision.wrapping_add(1);
        self.mark_cell_chunk_changed(key);
        true
    }

    /// Returns paint only for a currently valid cell; profile zero is available unpainted land.
    pub(crate) fn profile(&self, key: CellKey) -> Option<u16> {
        let block = self.blocks.get(&key.block())?;
        (block.valid & (1_u64 << key.offset()) != 0).then_some(block.profiles[key.offset()])
    }

    /// Returns the lot reserving a valid cell, including an erased lot awaiting redevelopment.
    pub(crate) fn lot(&self, key: CellKey) -> Option<u64> {
        let block = self.blocks.get(&key.block())?;
        (block.valid & (1_u64 << key.offset()) != 0).then_some(block.lots[key.offset()])
    }

    /// Looks up an existing claim by canonical geometry when a temporary store has other ids.
    pub(super) fn has_claim_at(&self, frame: GridFrame, x: i32, y: i32) -> bool {
        self.claimed_count != 0
            && self
                .frame_ids
                .get(&frame)
                .is_some_and(|&grid| self.lot(CellKey { grid, x, y }).is_some_and(|lot| lot != 0))
    }

    /// Updates an already validated cell and only its owning overlay chunk.
    pub(super) fn set_profile(&mut self, key: CellKey, profile: u16) {
        let offset = key.offset();
        let Some(block) = self.blocks.get_mut_if(&key.block(), |block| {
            block.valid & (1_u64 << offset) != 0 && block.profiles[offset] != profile
        }) else {
            return;
        };
        self.painted_count -= usize::from(block.profiles[offset] != 0);
        self.painted_count += usize::from(profile != 0);
        block.profiles[offset] = profile;
        block.refresh_reservation(offset);
        self.journal.blocks.push(key.block());
        self.bump_revision();
        self.mark_cell_chunk_changed(key);
    }

    /// Invalidates outstanding selections after an authoritative batch of cell mutations.
    pub(super) fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Writes previously validated coverage and refreshes its count, masks and selection epoch.
    pub(super) fn assign_lot(&mut self, lot: CellLot, parcel_id: u64) {
        for key in lot.cells() {
            let offset = key.offset();
            // An unchanged claim leaves the counts as they are.
            if let Some(block) = self
                .blocks
                .get_mut_if(&key.block(), |block| block.lots[offset] != parcel_id)
            {
                self.claimed_count -= usize::from(block.lots[offset] != 0);
                self.claimed_count += usize::from(parcel_id != 0);
                block.lots[offset] = parcel_id;
                block.refresh_reservation(offset);
                self.journal.blocks.push(key.block());
            }
        }
        self.bump_revision();
    }

    /// Visits each intersecting block once without query-time allocations or a city-wide scan.
    #[inline]
    pub(crate) fn visit_in_bounds(&self, bounds: CellBounds, mut visit: impl FnMut(CellKey)) {
        self.visit_mask_in_bounds::<false>(bounds, None, |key, _| visit(key));
    }

    /// Uses each block's reservation mask so external placement skips generated empty cells.
    #[inline]
    pub(crate) fn visit_reserved_in_bounds(
        &self,
        bounds: CellBounds,
        mut visit: impl FnMut(CellKey),
    ) {
        self.visit_mask_in_bounds::<true>(bounds, None, |key, _| visit(key));
    }

    /// Reuses the canonical footprint already computed for the spatial query's exact envelope.
    #[inline]
    pub(crate) fn visit_reserved_footprints_in_bounds(
        &self,
        bounds: CellBounds,
        visit: impl FnMut(CellKey, [DVec2; 4]),
    ) {
        self.visit_mask_in_bounds::<true>(bounds, None, visit);
    }

    /// Visits intersecting footprints on other canonical frames, optionally only reservations.
    /// Skips the excluded frame before deriving block/cell geometry; O(local chunk blocks +
    /// other-frame candidate cells), with the ordinary query's order and no allocation.
    pub(super) fn visit_other_frame_footprints_in_bounds(
        &self,
        bounds: CellBounds,
        excluded: GridFrame,
        reserved_only: bool,
        visit: impl FnMut(CellKey, [DVec2; 4]),
    ) {
        let excluded = self.frame_ids.get(&excluded).copied();
        if reserved_only {
            self.visit_mask_in_bounds::<true>(bounds, excluded, visit);
        } else {
            self.visit_mask_in_bounds::<false>(bounds, excluded, visit);
        }
    }

    #[inline]
    fn visit_mask_in_bounds<const RESERVED_ONLY: bool>(
        &self,
        bounds: CellBounds,
        excluded: Option<u64>,
        mut visit: impl FnMut(CellKey, [DVec2; 4]),
    ) {
        if !bounds.is_valid() {
            return;
        }
        for chunk in chunks_for_bounds(bounds) {
            let Some(entry) = self.chunks.get(&chunk) else {
                continue;
            };
            for &key in &entry.blocks {
                if Some(key.grid) == excluded {
                    continue;
                }
                let mut mask = 0;
                if RESERVED_ONLY {
                    let Some(block) = self.blocks.get(&key) else {
                        continue;
                    };
                    mask = block.reserved;
                    if mask == 0 {
                        continue;
                    }
                }
                let Some(frame) = self.frame(key.grid) else {
                    continue;
                };
                let geometry = frame.geometry();
                let block_bounds = block_bounds(&geometry, key);
                if !block_bounds.intersects(bounds) {
                    continue;
                }
                // A block spanning several chunks is visited only in the first chunk shared
                // by its bounds and this query. No per-query HashSet is required.
                let first = chunk_at(block_bounds.min.max(bounds.min));
                if first != chunk {
                    continue;
                }
                if !RESERVED_ONLY {
                    let Some(block) = self.blocks.get(&key) else {
                        continue;
                    };
                    mask = block.valid;
                }
                while mask != 0 {
                    let offset = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    let cell = key.cell(offset);
                    let corners =
                        geometry.rectangle_corners(f64::from(cell.x), f64::from(cell.y), 1.0, 1.0);
                    if CellBounds::from_points(corners).intersects(bounds) {
                        visit(cell, corners);
                    }
                }
            }
        }
    }

    /// Picks exact footprints and breaks shared-edge ties by stable address.
    pub(crate) fn pick(&self, point: DVec2) -> Option<CellKey> {
        if !super::geometry::valid_point(point) {
            return None;
        }
        let point = super::geometry::canonical_point(point);
        let mut picked = None;
        self.visit_in_bounds(
            CellBounds {
                min: point,
                max: point,
            },
            |key| {
                if self
                    .frame(key.grid)
                    .is_some_and(|frame| frame.contains(key.x, key.y, point))
                    && picked.is_none_or(|previous| key < previous)
                {
                    picked = Some(key);
                }
            },
        );
        picked
    }

    /// Queries only paint/lot reservations; generated empty cells deliberately do not block land.
    pub(crate) fn overlaps_reserved(&self, polygon: &[DVec2]) -> bool {
        let bounds = CellBounds::from_points(polygon.iter().copied());
        let mut overlaps = false;
        self.visit_reserved_footprints_in_bounds(bounds, |_, corners| {
            if !overlaps {
                overlaps = super::geometry::interiors_overlap(&corners, polygon);
            }
        });
        overlaps
    }
}

fn block_bounds(frame: &super::geometry::FrameGeometry, key: BlockKey) -> CellBounds {
    let x = f64::from(key.x) * f64::from(BLOCK_SIDE);
    let y = f64::from(key.y) * f64::from(BLOCK_SIDE);
    let side = f64::from(BLOCK_SIDE);
    CellBounds::from_points(frame.rectangle_corners(x, y, side, side))
        // Canonical vertices may round outward by half a micrometre.
        .expanded(0.5e-6)
}

fn chunk_at(point: DVec2) -> (i32, i32) {
    let cell = f64::from(RegionGraph::CHUNK_SIZE);
    (
        (point.x / cell).floor() as i32,
        (point.y / cell).floor() as i32,
    )
}

fn chunks_for_bounds(bounds: CellBounds) -> impl Iterator<Item = (i32, i32)> {
    let (min_x, min_y) = chunk_at(bounds.min);
    let (max_x, max_y) = chunk_at(bounds.max);
    (min_x..=max_x).flat_map(move |x| (min_y..=max_y).map(move |y| (x, y)))
}
