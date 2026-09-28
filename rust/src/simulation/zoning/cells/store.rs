// SPDX-License-Identifier: GPL-2.0-only

//! Compact oriented cell blocks indexed using the existing 512 m world chunk convention.

mod alignment;
mod chunks;
mod sources;

pub(crate) use alignment::RoadCellAlignment;

use super::sources::{CellCurveSource, CurveSourceKey};
use super::{
    BLOCK_CELLS, BLOCK_SIDE, BlockKey, CellBounds, CellKey, CellLot, CellRoadFrontage, GridFrame,
};
use crate::simulation::network::graph::RegionGraph;
use glam::DVec2;
use std::collections::HashMap;
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

/// Sparse authoritative paint and lot coverage; geometry is derived from persistent frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct CellStore {
    frames: Vec<GridFrame>,
    frame_ids: HashMap<GridFrame, u64>,
    blocks: HashMap<BlockKey, CellBlock>,
    chunks: HashMap<(i32, i32), CellChunk>,
    frontages: HashMap<CellKey, Vec<CellRoadFrontage>>,
    curve_sources: HashMap<CurveSourceKey, Arc<CellCurveSource>>,
    road_alignments: HashMap<usize, RoadCellAlignment>,
    painted_count: usize,
    claimed_count: usize,
    revision: u64,
}

impl CellStore {
    /// Visits persistent frames in id order for serialization, including temporarily empty ones.
    pub(crate) fn saved_frames(&self) -> impl Iterator<Item = (u64, GridFrame)> + '_ {
        self.frames
            .iter()
            .copied()
            .enumerate()
            .map(|(i, frame)| (i as u64 + 1, frame))
    }

    /// Captures only paint and occupied coverage; generated empty geometry is a rebuildable cache.
    pub(crate) fn saved_cells(&self) -> Vec<(CellKey, u16)> {
        let mut result = Vec::new();
        for (&key, block) in &self.blocks {
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

    /// Resolves a grid without depending on road storage order.
    pub(crate) fn frame(&self, grid: u64) -> Option<GridFrame> {
        let index = usize::try_from(grid.checked_sub(1)?).ok()?;
        self.frames.get(index).copied()
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
        for (&key, block) in &self.blocks {
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
        self.bump_revision();
    }

    /// Publishes a cell after generation and reservation validation. Existing paint is retained.
    pub(super) fn insert(&mut self, key: CellKey) -> bool {
        let Some(frame) = self.frame(key.grid) else {
            return false;
        };
        let block_key = key.block();
        if !self.blocks.contains_key(&block_key) {
            let bounds = block_bounds(&frame.geometry(), block_key);
            if !bounds.is_valid() {
                return false;
            }
            for chunk in chunks_for_bounds(bounds) {
                self.chunks.entry(chunk).or_default().blocks.push(block_key);
            }
        }
        let block = self.blocks.entry(block_key).or_default();
        let bit = 1_u64 << key.offset();
        if block.valid & bit != 0 {
            return false;
        }
        block.valid |= bit;
        self.revision = self.revision.wrapping_add(1);
        self.mark_cell_chunk_changed(key);
        true
    }

    /// Removes only unreserved cells. Painted/occupied geometry requires lifecycle handling.
    pub(super) fn remove_unreserved(&mut self, key: CellKey) -> bool {
        let block_key = key.block();
        let Some(block) = self.blocks.get_mut(&block_key) else {
            return false;
        };
        let offset = key.offset();
        let bit = 1_u64 << offset;
        if block.valid & bit == 0 || block.profiles[offset] != 0 || block.lots[offset] != 0 {
            return false;
        }
        block.valid &= !bit;
        self.frontages.remove(&key);
        let remove_block = block.valid == 0;
        if remove_block {
            self.blocks.remove(&block_key);
            if let Some(frame) = self.frame(key.grid) {
                for chunk in chunks_for_bounds(block_bounds(&frame.geometry(), block_key)) {
                    if let Some(entry) = self.chunks.get_mut(&chunk) {
                        entry.blocks.retain(|candidate| *candidate != block_key);
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
            && self.frame_ids.get(&frame).is_some_and(|&grid| {
                self.lot(CellKey { grid, x, y }).is_some_and(|lot| lot != 0)
            })
    }

    /// Updates an already validated cell and only its owning overlay chunk.
    pub(super) fn set_profile(&mut self, key: CellKey, profile: u16) {
        if let Some(block) = self.blocks.get_mut(&key.block()) {
            if block.valid & (1_u64 << key.offset()) == 0 || block.profiles[key.offset()] == profile
            {
                return;
            }
            self.painted_count -= usize::from(block.profiles[key.offset()] != 0);
            self.painted_count += usize::from(profile != 0);
            block.profiles[key.offset()] = profile;
            block.refresh_reservation(key.offset());
            self.bump_revision();
            self.mark_cell_chunk_changed(key);
        }
    }

    /// Invalidates outstanding selections after an authoritative batch of cell mutations.
    pub(super) fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Writes previously validated coverage and refreshes its count, masks and selection epoch.
    pub(super) fn assign_lot(&mut self, lot: CellLot, parcel_id: u64) {
        for key in lot.cells() {
            if let Some(block) = self.blocks.get_mut(&key.block()) {
                self.claimed_count -= usize::from(block.lots[key.offset()] != 0);
                self.claimed_count += usize::from(parcel_id != 0);
                block.lots[key.offset()] = parcel_id;
                block.refresh_reservation(key.offset());
            }
        }
        self.bump_revision();
    }

    /// Visits each intersecting block once without query-time allocations or a city-wide scan.
    #[inline]
    pub(crate) fn visit_in_bounds(&self, bounds: CellBounds, mut visit: impl FnMut(CellKey)) {
        self.visit_mask_in_bounds::<false>(bounds, |key, _| visit(key));
    }

    /// Uses each block's reservation mask so external placement skips generated empty cells.
    #[inline]
    pub(crate) fn visit_reserved_in_bounds(
        &self,
        bounds: CellBounds,
        mut visit: impl FnMut(CellKey),
    ) {
        self.visit_mask_in_bounds::<true>(bounds, |key, _| visit(key));
    }

    /// Reuses the canonical footprint already computed for the spatial query's exact envelope.
    #[inline]
    pub(crate) fn visit_reserved_footprints_in_bounds(
        &self,
        bounds: CellBounds,
        visit: impl FnMut(CellKey, [DVec2; 4]),
    ) {
        self.visit_mask_in_bounds::<true>(bounds, visit);
    }

    #[inline]
    fn visit_mask_in_bounds<const RESERVED_ONLY: bool>(
        &self,
        bounds: CellBounds,
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
