// SPDX-License-Identifier: GPL-2.0-only

//! Sparse, derived 64 m input lists for graded point queries (`TERRAIN-07-F`, T37 (a)).
//!
//! Entries contain inputs, never sampled heights or answers. They are rebuilt from the live
//! roadbed and site index and are not saved. A local edit replaces only entries within the same
//! cap/apron reach as dirty-tile selection; a world replacement rebuilds the sparse set.

use super::grade::CAP_M;
use super::seams::road_seam;
use super::sites::SiteSource;
use super::tile::{TILE_M, TileKey, grown};
use crate::simulation::buildings::allocator::{BuildingAllocator, SiteChange};
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem, TerrainChanges};
use std::collections::{BTreeMap, BTreeSet};

// The rendered `GL0` tile's one-lattice-point apron also covers a point normal's central
// differences. The list then takes the same cap and one-metre window pad as `query::prepare`.
pub(super) const QUERY_APRON_M: f64 = 0.5;
const WINDOW_PAD_M: f64 = 1.0;
// Covers the query grids' independent rounding pads as dirty selection's shared site reach does;
// the tile rebuild itself still filters every input against the exact complete-tile window.
const CHANGE_PAD_M: f64 = QUERY_APRON_M + WINDOW_PAD_M + 2.0;

/// Owner references any graded query point in one 64 m tile can need. Road seams and walls are
/// read from `road_pieces`; coverage keeps its narrower piece list so point preparation retains
/// each piece's exact triangle-grid candidate order. Sites stay in the allocator's `SiteCells`.
#[derive(Clone, Default)]
pub(super) struct QueryTileInputs {
    pub(super) road_pieces: Vec<PieceId>,
    pub(super) coverage_pieces: Vec<PieceId>,
}

impl QueryTileInputs {
    fn is_empty(&self) -> bool {
        self.road_pieces.is_empty() && self.coverage_pieces.is_empty()
    }

    #[cfg(test)]
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.road_pieces.capacity() * std::mem::size_of::<PieceId>()
            + self.coverage_pieces.capacity() * std::mem::size_of::<PieceId>()
    }
}

/// Sparse per-tile candidate inputs used by every production graded point query.
///
/// A lookup is `O(log active tiles)`. Rebuilding is `O(changed tiles * local road/site inputs)`;
/// full rebuild is `O(active input bounds * tiles in their cap reach + local inputs)`. Entries
/// and their vectors allocate only on rebuild, never on the query path.
pub(crate) struct TerrainQueryIndex {
    tiles: BTreeMap<TileKey, QueryTileInputs>,
    ready: bool,
    generation: u64,
}

impl Default for TerrainQueryIndex {
    fn default() -> Self {
        Self {
            tiles: BTreeMap::new(),
            ready: true,
            generation: 1,
        }
    }
}

impl TerrainQueryIndex {
    /// Collects the canonical owner ids from every indexed tile `rect` touches. A point reads
    /// one tile; a prepared vehicle-support window may read up to four. Duplicate owners at a
    /// tile boundary collapse back to the roadbed gather's ascending piece order.
    pub(super) fn gather(
        &self,
        rect: [f64; 4],
        road_pieces: &mut Vec<PieceId>,
        coverage_pieces: &mut Vec<PieceId>,
    ) -> bool {
        road_pieces.clear();
        coverage_pieces.clear();
        if !self.ready {
            return false;
        }
        let first = TileKey::at(rect[0], rect[1]);
        let last = TileKey::at(rect[2], rect[3]);
        let mut found = false;
        for x in first.x..=last.x {
            for z in first.z..=last.z {
                let Some(inputs) = self.tiles.get(&TileKey { x, z }) else {
                    continue;
                };
                found = true;
                road_pieces.extend_from_slice(&inputs.road_pieces);
                coverage_pieces.extend_from_slice(&inputs.coverage_pieces);
            }
        }
        road_pieces.sort_unstable();
        road_pieces.dedup();
        coverage_pieces.sort_unstable();
        coverage_pieces.dedup();
        found
    }

    pub(super) fn ready(&self) -> bool {
        self.ready
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// Rebuilds the complete sparse index after load or a world replacement.
    pub(crate) fn rebuild_all(&mut self, roadbed: &RoadbedSystem, sites: &BuildingAllocator) {
        if !sites.ready() {
            self.tiles.clear();
            self.ready = false;
            self.generation = self.generation.wrapping_add(1);
            return;
        }
        let mut keys = BTreeSet::new();
        for bounds in roadbed.terrain_query_input_bounds() {
            push_tiles(grown(bounds, CAP_M + CHANGE_PAD_M), &mut keys);
        }
        for bounds in sites.graded_query_input_bounds() {
            push_tiles(grown(bounds, CAP_M + CHANGE_PAD_M), &mut keys);
        }
        self.tiles.clear();
        for key in keys {
            self.rebuild_tile(key, roadbed, sites);
        }
        self.ready = true;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Rebuilds only tiles a road install or restore can reach.
    pub(crate) fn update_roads(
        &mut self,
        roadbed: &RoadbedSystem,
        sites: &BuildingAllocator,
        changes: &TerrainChanges,
    ) {
        if !self.ready || !sites.ready() {
            self.rebuild_all(roadbed, sites);
            return;
        }
        let mut keys = BTreeSet::new();
        for &bounds in changes.pieces.iter().chain(&changes.portals) {
            push_tiles(grown(bounds, CAP_M + CHANGE_PAD_M), &mut keys);
        }
        for &[a, b] in &changes.segments {
            let seam = road_seam(a, b);
            push_tiles(grown(seam.box_xz(), CAP_M + CHANGE_PAD_M), &mut keys);
        }
        self.rebuild_keys(keys, roadbed, sites);
        self.generation = self.generation.wrapping_add(1);
    }

    /// Rebuilds only tiles a set of site mutations can reach.
    pub(crate) fn update_sites(
        &mut self,
        roadbed: &RoadbedSystem,
        sites: &BuildingAllocator,
        changes: &[SiteChange],
    ) {
        if !self.ready || !sites.ready() {
            self.rebuild_all(roadbed, sites);
            return;
        }
        let mut keys = BTreeSet::new();
        for change in changes {
            push_tiles(
                grown(change.bounds.map(f64::from), CAP_M + CHANGE_PAD_M),
                &mut keys,
            );
        }
        self.rebuild_keys(keys, roadbed, sites);
        self.generation = self.generation.wrapping_add(1);
    }

    /// Rebuilds the query-input entries selected by a base edit. Candidate geometry is
    /// unchanged, but doing the local replacement at the publication boundary keeps the
    /// index's atomic edit contract explicit and lets debug checks compare a fresh rebuild.
    pub(crate) fn update_base_tiles(
        &mut self,
        _keys: impl IntoIterator<Item = TileKey>,
        roadbed: &RoadbedSystem,
        sites: &BuildingAllocator,
    ) {
        if !self.ready || !sites.ready() {
            self.rebuild_all(roadbed, sites);
        } else {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    fn rebuild_keys(
        &mut self,
        keys: impl IntoIterator<Item = TileKey>,
        roadbed: &RoadbedSystem,
        sites: &BuildingAllocator,
    ) {
        for key in keys {
            self.rebuild_tile(key, roadbed, sites);
        }
    }

    fn rebuild_tile(&mut self, key: TileKey, roadbed: &RoadbedSystem, sites: &BuildingAllocator) {
        let [x, z] = key.origin();
        let rect = grown([x, z, x + TILE_M, z + TILE_M], QUERY_APRON_M);
        let window = grown(rect, CAP_M + WINDOW_PAD_M);
        let mut inputs = QueryTileInputs::default();
        roadbed.grounded_pieces_in_bounds(window, &mut inputs.road_pieces);
        roadbed.grounded_pieces_in_bounds(grown(rect, 0.02), &mut inputs.coverage_pieces);
        let has_sites = sites.any_graded_site_in_bounds(window);
        if inputs.is_empty() && !has_sites {
            self.tiles.remove(&key);
        } else {
            self.tiles.insert(key, inputs);
        }
    }

    /// Active tile count and retained bytes, including vector capacities and a conservative
    /// per-entry allowance for the `BTreeMap` keys, links and partially filled nodes.
    #[cfg(test)]
    pub(crate) fn memory(&self) -> (usize, usize) {
        (
            self.tiles.len(),
            std::mem::size_of::<Self>()
                + self
                    .tiles
                    .values()
                    .map(|inputs| {
                        inputs.bytes()
                            + std::mem::size_of::<TileKey>()
                            + 3 * std::mem::size_of::<usize>()
                    })
                    .sum::<usize>(),
        )
    }

    #[cfg(test)]
    pub(crate) fn same_inputs(&self, other: &Self) -> bool {
        if self.ready != other.ready || self.tiles.len() != other.tiles.len() {
            return false;
        }
        self.tiles.iter().all(|(key, a)| {
            let Some(b) = other.tiles.get(key) else {
                return false;
            };
            a.road_pieces == b.road_pieces && a.coverage_pieces == b.coverage_pieces
        })
    }

    #[cfg(test)]
    pub(crate) fn difference(&self, other: &Self) -> String {
        if self.ready != other.ready || self.tiles.len() != other.tiles.len() {
            return format!(
                "ready {}/{} tiles {}/{}",
                self.ready,
                other.ready,
                self.tiles.len(),
                other.tiles.len()
            );
        }
        for (key, a) in &self.tiles {
            let Some(b) = other.tiles.get(key) else {
                return format!("missing tile {key:?}");
            };
            let equal = [
                a.road_pieces == b.road_pieces,
                a.coverage_pieces == b.coverage_pieces,
            ];
            if !equal.into_iter().all(|value| value) {
                return format!(
                    "tile {key:?} equal {equal:?} lengths {:?}/{:?}",
                    [a.road_pieces.len(), a.coverage_pieces.len()],
                    [b.road_pieces.len(), b.coverage_pieces.len()],
                );
            }
        }
        "unknown".into()
    }
}

fn push_tiles([min_x, min_z, max_x, max_z]: [f64; 4], out: &mut BTreeSet<TileKey>) {
    let first = TileKey::at(min_x, min_z);
    let last = TileKey::at(max_x, max_z);
    for x in first.x..=last.x {
        for z in first.z..=last.z {
            out.insert(TileKey { x, z });
        }
    }
}
