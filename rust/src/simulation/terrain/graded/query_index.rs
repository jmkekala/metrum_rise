// SPDX-License-Identifier: GPL-2.0-only

//! Sparse, derived 64 m input lists for graded point queries (`TERRAIN-07-F`, T37 (a)).
//!
//! Entries contain inputs, never sampled heights or answers. They are rebuilt from the live
//! roadbed and site index and are not saved. A local edit replaces only entries within the same
//! cap/apron reach as dirty-tile selection; a world replacement rebuilds the sparse set.

use super::grade::CAP_M;
use super::seams::{Seam, road_seam};
use super::sites::{SiteBuffer, SiteSource};
use super::tile::{TILE_M, TileKey, grown};
use crate::simulation::buildings::allocator::{BuildingAllocator, SiteChange};
use crate::simulation::network::roadbed::{
    PieceId, RoadVec3, RoadbedSystem, TerrainChanges, WallSegment,
};
use std::collections::{BTreeMap, BTreeSet};

// The rendered `GL0` tile's one-lattice-point apron also covers a point normal's central
// differences. The list then takes the same cap and one-metre window pad as `query::prepare`.
pub(super) const QUERY_APRON_M: f64 = 0.5;
const WINDOW_PAD_M: f64 = 1.0;
// Covers the query grids' independent rounding pads as dirty selection's shared site reach does;
// the tile rebuild itself still filters every input against the exact complete-tile window.
const CHANGE_PAD_M: f64 = QUERY_APRON_M + WINDOW_PAD_M + 2.0;

/// Inputs any graded query point in one 64 m tile can need. Coverage keeps the local piece ids
/// so point preparation can retain each piece's exact triangle-grid candidate order.
#[derive(Clone, Default)]
pub(super) struct QueryTileInputs {
    pub(super) road_seams: Vec<Seam>,
    pub(super) coverage_pieces: Vec<PieceId>,
    pub(super) walls: Vec<Seam>,
    pub(super) sites: SiteBuffer,
}

impl QueryTileInputs {
    fn is_empty(&self) -> bool {
        self.road_seams.is_empty()
            && self.coverage_pieces.is_empty()
            && self.walls.is_empty()
            && self.sites.is_empty()
    }

    #[cfg(test)]
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.road_seams.capacity() * std::mem::size_of::<Seam>()
            + self.coverage_pieces.capacity() * std::mem::size_of::<PieceId>()
            + self.walls.capacity() * std::mem::size_of::<Seam>()
            + self.sites.estimated_bytes()
    }
}

#[derive(Default)]
struct BuildScratch {
    pieces: Vec<PieceId>,
    segments: Vec<[RoadVec3; 2]>,
    wall_segments: Vec<WallSegment>,
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
    pub(super) fn get(&self, tile: TileKey) -> Option<&QueryTileInputs> {
        if self.ready {
            self.tiles.get(&tile)
        } else {
            None
        }
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
        let mut scratch = BuildScratch::default();
        for key in keys {
            self.rebuild_tile(key, roadbed, sites, &mut scratch);
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
        let mut scratch = BuildScratch::default();
        for key in keys {
            self.rebuild_tile(key, roadbed, sites, &mut scratch);
        }
    }

    fn rebuild_tile(
        &mut self,
        key: TileKey,
        roadbed: &RoadbedSystem,
        sites: &BuildingAllocator,
        scratch: &mut BuildScratch,
    ) {
        let [x, z] = key.origin();
        let rect = grown([x, z, x + TILE_M, z + TILE_M], QUERY_APRON_M);
        let window = grown(rect, CAP_M + WINDOW_PAD_M);
        let mut inputs = QueryTileInputs::default();
        roadbed.grounded_seams_in_bounds(window, &mut scratch.pieces, &mut scratch.segments);
        inputs
            .road_seams
            .extend(scratch.segments.iter().map(|&[a, b]| road_seam(a, b)));
        roadbed.grounded_pieces_in_bounds(grown(rect, 0.02), &mut scratch.pieces);
        inputs.coverage_pieces.extend_from_slice(&scratch.pieces);
        inputs.sites.gather(sites, window);
        roadbed.grounded_pieces_in_bounds(window, &mut scratch.pieces);
        roadbed.wall_seams_in_bounds(window, &scratch.pieces, &mut scratch.wall_segments);
        inputs
            .walls
            .extend(scratch.wall_segments.iter().map(|wall| {
                let [a, b] = wall.points;
                Seam {
                    kind: super::seams::SeamKind::Wall,
                    ..road_seam(a, b)
                }
            }));
        if inputs.is_empty() {
            self.tiles.remove(&key);
        } else {
            self.tiles.insert(key, inputs);
        }
    }

    /// Active tile count and retained input bytes (including vector capacities, excluding tree
    /// node allocator overhead).
    #[cfg(test)]
    pub(crate) fn memory(&self) -> (usize, usize) {
        (
            self.tiles.len(),
            self.tiles.values().map(|inputs| inputs.bytes()).sum(),
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
            a.road_seams == b.road_seams
                && a.coverage_pieces == b.coverage_pieces
                && a.walls == b.walls
                && a.sites.same_inputs(&b.sites)
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
                a.road_seams == b.road_seams,
                a.coverage_pieces == b.coverage_pieces,
                a.walls == b.walls,
                a.sites.same_inputs(&b.sites),
            ];
            if !equal.into_iter().all(|value| value) {
                return format!(
                    "tile {key:?} equal {equal:?} lengths {:?}/{:?}",
                    [a.road_seams.len(), a.coverage_pieces.len(), a.walls.len()],
                    [b.road_seams.len(), b.coverage_pieces.len(), b.walls.len()],
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
