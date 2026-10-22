// SPDX-License-Identifier: GPL-2.0-only

//! Spatial lookup and ray-query operations for derived building sites.

use super::geometry::{point_in_polygon_slice, site_lot_center, update_site_plane_ray_hit};
use super::model::BuildingSiteClient;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::{Vector2, Vector3};

// Covers the polygon test's `SITE_POINT_EPS_M` edge tolerance and the f32 rounding of lot
// centers, radii and distances, with a wide margin; a larger value only tests more sites.
const SITE_REACH_SLACK_M: f32 = 0.5;
// Covers the rounding of contact offsets around a gathered centre, with a wide margin.
const NEARBY_REACH_SLACK_M: f32 = 1.0;

/// Sites gathered around one point by [`BuildingAllocator::gather_nearby_sites`], each with the
/// 512 m chunk it is indexed in. Fixed capacity, so gathering allocates nothing.
pub(crate) struct NearbySites {
    len: usize,
    sites: [(usize, [i32; 2]); NearbySites::CAPACITY],
}

impl NearbySites {
    /// Sites one gather holds; a denser neighbourhood falls back to per-point queries.
    pub(crate) const CAPACITY: usize = 64;

    /// An empty set, filled by [`BuildingAllocator::gather_nearby_sites`].
    pub(crate) fn new() -> Self {
        Self {
            len: 0,
            sites: [(0, [0; 2]); Self::CAPACITY],
        }
    }
}

// The 512 m chunks whose indexed sites a query within `margin_m` of `pos` visits.
fn site_chunk_range(pos: Vector2, margin_m: f32) -> [i32; 4] {
    let chunk_size = RegionGraph::CHUNK_SIZE;
    [
        ((pos.x - margin_m) / chunk_size).floor() as i32,
        ((pos.x + margin_m) / chunk_size).floor() as i32,
        ((pos.y - margin_m) / chunk_size).floor() as i32,
        ((pos.y + margin_m) / chunk_size).floor() as i32,
    ]
}

impl BuildingSiteClient {
    pub(crate) fn contains_point(&self, pos: Vector2) -> bool {
        point_in_polygon_slice(pos, &self.footprint_world)
    }

    /// Walls and paving, where even a yard plant cannot stand. A site without imported mesh
    /// bounds knows no walls, so its whole support stays closed as before.
    pub(crate) fn blocks_yard_plant(&self, pos: Vector2) -> bool {
        self.structure_world.is_empty()
            || self
                .structure_world
                .iter()
                .any(|quad| point_in_polygon_slice(pos, quad))
            || self
                .surfaces
                .iter()
                .any(|surface| point_in_polygon_slice(pos, &surface.vertices_world))
    }

    /// Whether the point lies in an authored planting polygon, using the site's edge tolerance.
    pub(crate) fn contains_yard_planting(&self, pos: Vector2) -> bool {
        self.planting_world
            .iter()
            .any(|area| point_in_polygon_slice(pos, area))
    }

    pub(super) fn height_at(&self, pos: Vector2) -> Option<f32> {
        // Aprons are owned by engineered terrain, not an independent flat plane.
        self.contains_point(pos).then_some(self.support_height_m)
    }

    pub(crate) fn raycast(&self, ray_origin: Vector3, ray_dir: Vector3) -> Option<Vector3> {
        if ray_dir.length_squared() <= f32::EPSILON {
            return None;
        }

        let mut best: Option<(f32, Vector3)> = None;
        update_site_plane_ray_hit(
            &mut best,
            ray_origin,
            ray_dir,
            self.support_height_m,
            &self.footprint_world,
        );
        best.map(|(_, hit)| hit)
    }
}

impl BuildingAllocator {
    /// Conservative O(1) exclusion check; stale indices retain the ordinary exact query.
    pub(crate) fn has_explicit_cell_sites(&self) -> bool {
        !self.building_sites.is_empty()
            && (!self.building_site_query_index_ready() || self.explicit_cell_site_count != 0)
    }

    /// Whether bounded site queries can run without rebuilding or scanning the building store.
    /// Preview workers must defer unprepared inputs to normal allocator/terrain maintenance.
    pub(crate) fn building_site_query_index_ready(&self) -> bool {
        self.building_sites.len() == self.buildings.len()
            && (self.buildings.is_empty()
                || (!self.dirty_index && !self.building_chunks.is_empty()))
    }

    /// Rebuilds the derived site clients and shared building index when a site query needs them.
    pub(crate) fn prepare_building_site_query_index(&mut self, zone_cell_m: f32) {
        if self.building_sites.len() != self.buildings.len() {
            self.rebuild_building_site_clients(zone_cell_m);
        }
        if self.zone_index_needs_rebuild() {
            self.rebuild_zone_index();
        }
    }

    /// Whether [`Self::prepare_building_site_query_index`] would rebuild the zone index. The
    /// rebuild also resets the vacancy lists to building order, which admission reads, so work
    /// that may run before the index's next reader (the census book's preparation for planning,
    /// `SIM-02-K`) waits until that reader has rebuilt it. O(1).
    pub(crate) fn zone_index_needs_rebuild(&self) -> bool {
        self.dirty_index || (self.building_chunks.is_empty() && !self.buildings.is_empty())
    }

    pub(crate) fn site_world_bounds(&self, building_idx: usize) -> Option<(f32, f32, f32, f32)> {
        self.building_sites
            .get(building_idx)
            .map(|site| site.lot_bounds())
    }

    /// Ground a site draws on: its lot and any paving run on past the lot line to the road.
    pub(crate) fn site_dirty_bounds(&self, building_idx: usize) -> Option<(f32, f32, f32, f32)> {
        self.building_sites.get(building_idx).map(|site| {
            let (a, b) = (site.lot_bounds(), site.bounds());
            (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
        })
    }

    /// Queues one site change's bounds in the terrain outbox. Boxes stay separate: a box
    /// merged over changes far apart would refresh every terrain patch between them
    /// (`SIM-02-K`). O(1) amortized.
    pub(crate) fn accumulate_pending_site_dirty_bounds(
        &mut self,
        bounds: Option<(f32, f32, f32, f32)>,
    ) {
        if let Some(bounds) = bounds {
            self.building_site_dirty_bounds.push(bounds);
        }
    }

    /// Records terrain changes for every site and cell exclusions only for explicit sites.
    /// Ordinary parcels own their cell claims; clearing occupancy already queues their local
    /// frontage revalidation. Geometry work is bounded by edited sites, not their union's area.
    pub(crate) fn record_building_site_change(
        &mut self,
        building_idx: usize,
        bounds: Option<(f32, f32, f32, f32)>,
    ) {
        self.accumulate_pending_site_dirty_bounds(bounds);
        if self.buildings.get(building_idx).is_some_and(|building| {
            building.parcel_id == 0
                && building.zone_type == crate::simulation::zoning::ZoneType::None
        }) && let Some(bounds) = bounds
        {
            self.cell_site_dirty_bounds.push(bounds);
        }
    }

    /// Drains one explicit-site exclusion edit, retaining the event buffer's capacity.
    pub(crate) fn pop_pending_cell_site_dirty_bounds(&mut self) -> Option<(f32, f32, f32, f32)> {
        self.cell_site_dirty_bounds.pop()
    }

    /// Takes the queued per-change site boxes, leaving the outbox empty; hand the storage back
    /// with [`Self::return_site_dirty_bounds_storage`] to keep its capacity.
    pub(crate) fn take_pending_site_dirty_bounds(&mut self) -> Vec<(f32, f32, f32, f32)> {
        std::mem::take(&mut self.building_site_dirty_bounds)
    }

    /// Returns a drained outbox's storage when nothing was queued meanwhile. O(1).
    pub(crate) fn return_site_dirty_bounds_storage(
        &mut self,
        mut storage: Vec<(f32, f32, f32, f32)>,
    ) {
        if self.building_site_dirty_bounds.is_empty() {
            storage.clear();
            self.building_site_dirty_bounds = storage;
        }
    }

    pub(crate) fn sample_building_site_height(&self, pos: Vector2) -> Option<f32> {
        self.site_at(pos, |site| site.height_at(pos))
    }

    /// Whether a site stops an authored plant at `pos`: inside its flat support, only on the
    /// building's own walls or on authored paving, so a yard's lawn takes hedges, shrubs and
    /// trees. Wild vegetation keeps the whole support clear through
    /// [`Self::sample_building_site_height`]. Same bounded chunk lookup.
    pub(crate) fn building_site_blocks_yard_plant(&self, pos: Vector2) -> bool {
        self.site_at(pos, |site| {
            site.contains_point(pos)
                .then(|| site.blocks_yard_plant(pos))
        })
        .unwrap_or(false)
    }

    /// Whether `pos` lies in a building's yard planting area. Same bounded chunk lookup.
    pub(crate) fn in_yard_planting(&self, pos: Vector2) -> bool {
        self.site_at(pos, |site| site.contains_yard_planting(pos).then_some(()))
            .is_some()
    }

    // The lowest-index site for which `probe` answers at `pos`, through the 512 m chunk index;
    // a linear scan only while the index is being rebuilt. Every probe needs `pos` in one of the
    // site's footprint, surface or planting polygons, whose vertices lie within the exact
    // `max_site_radius_m` of the lot center; the polygon test accepts points up to
    // `SITE_POINT_EPS_M` outside an edge. Sites whose lot center is farther away than that,
    // plus `SITE_REACH_SLACK_M` for the tolerance and f32 rounding, cannot answer and are
    // skipped before their polygons are read (`SIM-02-K`).
    fn site_at<T>(
        &self,
        pos: Vector2,
        probe: impl Fn(&BuildingSiteClient) -> Option<T>,
    ) -> Option<T> {
        if self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
        {
            return self.building_sites.iter().find_map(probe);
        }

        let margin_m = self.max_site_radius_m.max(0.0);
        let reach_sq = (margin_m + SITE_REACH_SLACK_M) * (margin_m + SITE_REACH_SLACK_M);
        let [min_chunk_x, max_chunk_x, min_chunk_z, max_chunk_z] = site_chunk_range(pos, margin_m);

        let mut best_idx = usize::MAX;
        let mut best = None;
        for chunk_x in min_chunk_x..=max_chunk_x {
            for chunk_z in min_chunk_z..=max_chunk_z {
                let Some(indices) = self.building_chunks.get(&(chunk_x, chunk_z)) else {
                    continue;
                };
                for &idx in indices {
                    if idx >= best_idx || idx >= self.building_sites.len() {
                        continue;
                    }
                    let site = &self.building_sites[idx];
                    if site_lot_center(site).distance_squared_to(pos) > reach_sq {
                        continue;
                    }
                    let Some(answer) = probe(site) else {
                        continue;
                    };
                    best_idx = idx;
                    best = Some(answer);
                }
            }
        }

        best
    }

    /// Gathers, with one chunk scan, every site that [`Self::sample_building_site_height`] can
    /// consider at any point within `reach_m` of `center`, so a vehicle's contact points share
    /// it (`SIM-02-K`). Returns false, leaving the per-point query in charge, while the index is
    /// being rebuilt, for a non-finite input, or when more than [`NearbySites::CAPACITY`] sites
    /// qualify. O(sites in the chunks within the maximum site radius of `center`).
    pub(crate) fn gather_nearby_sites(
        &self,
        center: Vector2,
        reach_m: f32,
        nearby: &mut NearbySites,
    ) -> bool {
        nearby.len = 0;
        if self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
            || !center.x.is_finite()
            || !center.y.is_finite()
            || !reach_m.is_finite()
        {
            return false;
        }
        let margin_m = self.max_site_radius_m.max(0.0);
        // Covers every per-point chunk range and lot-centre test within `reach_m`, with slack
        // for the rounding of the caller's contact offsets.
        let span_m = margin_m + reach_m.max(0.0) + NEARBY_REACH_SLACK_M;
        let keep_sq = (span_m + SITE_REACH_SLACK_M) * (span_m + SITE_REACH_SLACK_M);
        let [min_chunk_x, max_chunk_x, min_chunk_z, max_chunk_z] = site_chunk_range(center, span_m);
        for chunk_x in min_chunk_x..=max_chunk_x {
            for chunk_z in min_chunk_z..=max_chunk_z {
                let Some(indices) = self.building_chunks.get(&(chunk_x, chunk_z)) else {
                    continue;
                };
                for &idx in indices {
                    if idx >= self.building_sites.len()
                        || site_lot_center(&self.building_sites[idx]).distance_squared_to(center)
                            > keep_sq
                    {
                        continue;
                    }
                    if nearby.len == NearbySites::CAPACITY {
                        nearby.len = 0;
                        return false;
                    }
                    nearby.sites[nearby.len] = (idx, [chunk_x, chunk_z]);
                    nearby.len += 1;
                }
            }
        }
        true
    }

    /// [`Self::sample_building_site_height`] at `pos`, within the reach of a successful
    /// [`Self::gather_nearby_sites`]: the same candidates, chunk ranges and lowest-index rule,
    /// over the gathered sites only. O(gathered sites).
    pub(crate) fn sample_building_site_height_near(
        &self,
        pos: Vector2,
        nearby: &NearbySites,
    ) -> Option<f32> {
        let margin_m = self.max_site_radius_m.max(0.0);
        let reach_sq = (margin_m + SITE_REACH_SLACK_M) * (margin_m + SITE_REACH_SLACK_M);
        let [min_chunk_x, max_chunk_x, min_chunk_z, max_chunk_z] = site_chunk_range(pos, margin_m);
        let mut best: Option<(usize, f32)> = None;
        for &(idx, [chunk_x, chunk_z]) in &nearby.sites[..nearby.len] {
            if chunk_x < min_chunk_x
                || chunk_x > max_chunk_x
                || chunk_z < min_chunk_z
                || chunk_z > max_chunk_z
                || best.is_some_and(|(best_idx, _)| idx >= best_idx)
            {
                continue;
            }
            let site = &self.building_sites[idx];
            if site_lot_center(site).distance_squared_to(pos) > reach_sq {
                continue;
            }
            if let Some(height) = site.height_at(pos) {
                best = Some((idx, height));
            }
        }
        best.map(|(_, height)| height)
    }

    /// The support height, yard-plant block and planting answers at `pos`, by `site_at` and by
    /// the unfiltered scan it replaced, for regressions that compare the two.
    #[cfg(test)]
    pub(crate) fn site_answers_both_ways(&self, pos: Vector2) -> [(Option<f32>, bool, bool); 2] {
        let blocks = |site: &BuildingSiteClient| {
            site.contains_point(pos)
                .then(|| site.blocks_yard_plant(pos))
        };
        let planting = |site: &BuildingSiteClient| {
            site.planting_world
                .iter()
                .any(|area| point_in_polygon_slice(pos, area))
                .then_some(())
        };
        [
            (
                self.site_at(pos, |site| site.height_at(pos)),
                self.site_at(pos, blocks).unwrap_or(false),
                self.site_at(pos, planting).is_some(),
            ),
            (
                self.site_at_unfiltered(pos, |site| site.height_at(pos)),
                self.site_at_unfiltered(pos, blocks).unwrap_or(false),
                self.site_at_unfiltered(pos, planting).is_some(),
            ),
        ]
    }

    // `site_at` before the lot-centre skip (`SIM-02-K`), unchanged.
    #[cfg(test)]
    fn site_at_unfiltered<T>(
        &self,
        pos: Vector2,
        probe: impl Fn(&BuildingSiteClient) -> Option<T>,
    ) -> Option<T> {
        if self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
        {
            return self.building_sites.iter().find_map(probe);
        }

        let margin_m = self.max_site_radius_m.max(0.0);
        let chunk_size = RegionGraph::CHUNK_SIZE;
        let min_chunk_x = ((pos.x - margin_m) / chunk_size).floor() as i32;
        let max_chunk_x = ((pos.x + margin_m) / chunk_size).floor() as i32;
        let min_chunk_z = ((pos.y - margin_m) / chunk_size).floor() as i32;
        let max_chunk_z = ((pos.y + margin_m) / chunk_size).floor() as i32;

        let mut best_idx = usize::MAX;
        let mut best = None;
        for chunk_x in min_chunk_x..=max_chunk_x {
            for chunk_z in min_chunk_z..=max_chunk_z {
                let Some(indices) = self.building_chunks.get(&(chunk_x, chunk_z)) else {
                    continue;
                };
                for &idx in indices {
                    if idx >= best_idx || idx >= self.building_sites.len() {
                        continue;
                    }
                    let Some(answer) = probe(&self.building_sites[idx]) else {
                        continue;
                    };
                    best_idx = idx;
                    best = Some(answer);
                }
            }
        }

        best
    }

    pub(crate) fn raycast_building_site_surface(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
        world_bounds: (f32, f32, f32, f32),
    ) -> Option<Vector3> {
        let ray_denom = ray_dir.length_squared();
        if !ray_origin.y.is_finite()
            || !ray_dir.y.is_finite()
            || !ray_denom.is_finite()
            || ray_denom <= f32::EPSILON
            || self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
        {
            return None;
        }

        let (min_x, min_z, max_x, max_z) = projected_ray_bounds_in_world(
            ray_origin,
            ray_dir,
            world_bounds.0,
            world_bounds.1,
            world_bounds.2,
            world_bounds.3,
        )?;
        let mut best: Option<(f32, usize, Vector3)> = None;
        self.visit_indexed_site_candidates_for_bounds(min_x, min_z, max_x, max_z, |building_idx| {
            let Some(site) = self.building_sites.get(building_idx) else {
                return;
            };
            let Some(hit) = site.raycast(ray_origin, ray_dir) else {
                return;
            };
            let t = (hit - ray_origin).dot(ray_dir) / ray_denom;
            if t < 0.0 {
                return;
            }
            if best.as_ref().is_none_or(|(best_t, best_idx, _)| {
                t < *best_t || (t == *best_t && building_idx < *best_idx)
            }) {
                best = Some((t, building_idx, hit));
            }
        });
        best.map(|(_, _, hit)| hit)
    }

    fn visit_indexed_site_candidates_for_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        mut visit: impl FnMut(usize),
    ) {
        let margin_m = self.max_site_radius_m.max(0.0);
        let chunk_size = RegionGraph::CHUNK_SIZE;
        let min_chunk_x = ((min_x.min(max_x) - margin_m) / chunk_size).floor() as i32;
        let max_chunk_x = ((min_x.max(max_x) + margin_m) / chunk_size).floor() as i32;
        let min_chunk_z = ((min_z.min(max_z) - margin_m) / chunk_size).floor() as i32;
        let max_chunk_z = ((min_z.max(max_z) + margin_m) / chunk_size).floor() as i32;

        for chunk_x in min_chunk_x..=max_chunk_x {
            for chunk_z in min_chunk_z..=max_chunk_z {
                let Some(indices) = self.building_chunks.get(&(chunk_x, chunk_z)) else {
                    continue;
                };
                for &building_idx in indices {
                    if building_idx < self.building_sites.len() {
                        visit(building_idx);
                    }
                }
            }
        }
    }

    /// Returns site indices whose chunk coverage may overlap the world bounds.
    pub(crate) fn site_candidate_indices_for_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<usize> {
        if self.dirty_index
            || self.building_sites.len() != self.buildings.len()
            || self.building_chunks.is_empty()
        {
            return (0..self.building_sites.len()).collect();
        }

        let mut candidates = Vec::new();
        self.visit_indexed_site_candidates_for_bounds(min_x, min_z, max_x, max_z, |building_idx| {
            candidates.push(building_idx)
        });
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

fn projected_ray_bounds_in_world(
    ray_origin: Vector3,
    ray_dir: Vector3,
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
) -> Option<(f32, f32, f32, f32)> {
    if !ray_origin.x.is_finite()
        || !ray_origin.z.is_finite()
        || !ray_dir.x.is_finite()
        || !ray_dir.z.is_finite()
    {
        return None;
    }

    let mut entry_t = f32::NEG_INFINITY;
    let mut exit_t = f32::INFINITY;
    clip_projected_ray_axis(
        ray_origin.x,
        ray_dir.x,
        min_x.min(max_x),
        min_x.max(max_x),
        &mut entry_t,
        &mut exit_t,
    )?;
    clip_projected_ray_axis(
        ray_origin.z,
        ray_dir.z,
        min_z.min(max_z),
        min_z.max(max_z),
        &mut entry_t,
        &mut exit_t,
    )?;
    if exit_t < entry_t || exit_t < 0.0 {
        return None;
    }
    if !entry_t.is_finite() || !exit_t.is_finite() {
        return Some((ray_origin.x, ray_origin.z, ray_origin.x, ray_origin.z));
    }

    let start_t = entry_t.max(0.0);
    let end_t = exit_t.max(start_t);
    let start = ray_origin + ray_dir * start_t;
    let end = ray_origin + ray_dir * end_t;
    Some((
        start.x.min(end.x),
        start.z.min(end.z),
        start.x.max(end.x),
        start.z.max(end.z),
    ))
}

fn clip_projected_ray_axis(
    origin: f32,
    direction: f32,
    min: f32,
    max: f32,
    entry_t: &mut f32,
    exit_t: &mut f32,
) -> Option<()> {
    if direction.abs() <= f32::EPSILON {
        return (origin >= min && origin <= max).then_some(());
    }

    let t0 = (min - origin) / direction;
    let t1 = (max - origin) / direction;
    *entry_t = entry_t.max(t0.min(t1));
    *exit_t = exit_t.min(t0.max(t1));
    Some(())
}
