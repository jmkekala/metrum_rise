// SPDX-License-Identifier: GPL-2.0-only

//! Spatial lookup and ray-query operations for derived building sites.

use super::geometry::{point_in_polygon_slice, site_lot_center};
#[cfg(test)]
use super::geometry::update_site_plane_ray_hit;
use super::model::{
    BuildingSiteClient, BuildingSiteTerrainClient, BuildingSiteTerrainSnapshot, SiteChange,
    site_surface_bounds,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::graded::{SiteBuffer, SiteSource};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector2;
#[cfg(test)]
use godot::prelude::Vector3;

// Covers the polygon test's `SITE_POINT_EPS_M` edge tolerance and the f32 rounding of lot
// centers, radii and distances, with a wide margin; a larger value only tests more sites.
const SITE_REACH_SLACK_M: f32 = 0.5;
// Growth of a graded terrain rectangle before its f32 site query, metres.
const SITE_RECT_SLACK_M: f64 = 0.01;

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

/// The allocator's point-in-footprint predicate ([`BuildingSiteClient::contains_point`]) on a
/// footprint's corners: inside, or within `SITE_POINT_EPS_M` of a side. The graded terrain's
/// `Pad` class reads it (`TERRAIN-07-C`), so the pad is the allocator's footprint exactly.
pub(crate) fn site_footprint_contains(footprint: &[Vector2], pos: Vector2) -> bool {
    point_in_polygon_slice(pos, footprint)
}

// The graded terrain's sites (`TERRAIN-07-C`): the sites whose bounds meet a rectangle, through
// the 64 m site cells (`TERRAIN-07-F`, `docs/terrain.md` T40), with their footprints, support
// heights and surfaces in authored order. No answer while the index is being rebuilt: the bake
// never scans every site. `SiteCells` changes atomically with the derived clients, independently
// of the allocator's coarser zone index.
impl SiteSource for BuildingAllocator {
    fn ready(&self) -> bool {
        self.building_sites.len() == self.buildings.len()
    }

    fn sites_in_bounds(&self, rect: [f64; 4], out: &mut SiteBuffer) {
        self.visit_graded_sites(rect, |site| push_graded_site(site, out));
    }
}

impl BuildingAllocator {
    /// Bounds of every derived site input, for rebuilding the unsaved graded-query index.
    pub(crate) fn graded_query_input_bounds(&self) -> impl Iterator<Item = [f64; 4]> + '_ {
        self.building_sites.iter().map(|site| {
            let (min_x, min_z, max_x, max_z) = site.bounds();
            [min_x, min_z, max_x, max_z].map(f64::from)
        })
    }

    /// Whether a graded terrain query of `rect` reads at least one site. This walks the same
    /// bounded `SiteCells` candidates as [`SiteSource::sites_in_bounds`] without copying a site.
    pub(crate) fn any_graded_site_in_bounds(&self, rect: [f64; 4]) -> bool {
        let query = SiteRectQuery::new(rect, self.max_site_radius_m);
        let mut found = false;
        self.site_cells.visit(query.rect, |idx| {
            found |= self
                .building_sites
                .get(idx)
                .is_some_and(|site| query.holds(site));
        });
        found
    }

    // Visits the sites a graded terrain query of `rect` reads, through the 64 m site cells.
    fn visit_graded_sites(&self, rect: [f64; 4], mut visit: impl FnMut(&BuildingSiteClient)) {
        let query = SiteRectQuery::new(rect, self.max_site_radius_m);
        self.site_cells.visit(query.rect, |idx| {
            if let Some(site) = self.building_sites.get(idx)
                && query.holds(site)
            {
                visit(site);
            }
        });
    }

    /// Copies the sites the graded terrain bakes and tile lists of rectangles inside `rects`
    /// read, so they can run off the simulation lock (`TERRAIN-07-D`, `TERRAIN-07-E`'s undo
    /// record); `None` while the site index is being rebuilt. `O(sites meeting each rectangle)`,
    /// each site copied once.
    pub(crate) fn graded_site_snapshot(&self, rects: Vec<[f64; 4]>) -> Option<SiteSnapshot> {
        if !self.building_site_query_index_ready() {
            return None;
        }
        let mut indices = Vec::new();
        for &rect in &rects {
            let query = SiteRectQuery::new(rect, self.max_site_radius_m);
            let [min_x, min_z, max_x, max_z] = query.rect;
            self.visit_indexed_site_candidates_for_bounds(min_x, min_z, max_x, max_z, |idx| {
                if self
                    .building_sites
                    .get(idx)
                    .is_some_and(|site| query.holds(site))
                {
                    indices.push(idx);
                }
            });
        }
        indices.sort_unstable();
        indices.dedup();
        Some(SiteSnapshot {
            rects,
            max_site_radius_m: self.max_site_radius_m,
            sites: indices
                .into_iter()
                .map(|idx| self.building_sites[idx].clone())
                .collect(),
        })
    }
}

// What a graded terrain site query keeps: the rectangle grown past its f32 rounding, and the
// test the allocator and its snapshots share.
struct SiteRectQuery {
    rect: [f32; 4],
    reach: f32,
}

impl SiteRectQuery {
    fn new(rect: [f64; 4], max_site_radius_m: f32) -> Self {
        // Grown past the f32 rounding of the rectangle: a site more only adds inputs.
        let rect = [
            rect[0] - SITE_RECT_SLACK_M,
            rect[1] - SITE_RECT_SLACK_M,
            rect[2] + SITE_RECT_SLACK_M,
            rect[3] + SITE_RECT_SLACK_M,
        ]
        .map(|value| value as f32);
        Self {
            rect,
            reach: max_site_radius_m.max(0.0) + SITE_REACH_SLACK_M,
        }
    }

    fn holds(&self, site: &BuildingSiteClient) -> bool {
        self.holds_parts(site_lot_center(site), site.bounds())
    }

    // Every footprint, surface and planting corner lies within the maximum site radius of its
    // lot centre, so a lot centre farther from the rectangle (plus the polygon test's slack)
    // rules the site out before its polygons are read, as `site_at` does; the rest are the
    // sites whose footprint and surface bounds meet it.
    fn holds_parts(&self, centre: Vector2, bounds: (f32, f32, f32, f32)) -> bool {
        let [min_x, min_z, max_x, max_z] = self.rect;
        let dx = (min_x - centre.x).max(centre.x - max_x).max(0.0);
        let dz = (min_z - centre.y).max(centre.y - max_z).max(0.0);
        let (site_min_x, site_min_z, site_max_x, site_max_z) = bounds;
        dx * dx + dz * dz <= self.reach * self.reach
            && site_min_x <= max_x
            && site_max_x >= min_x
            && site_min_z <= max_z
            && site_max_z >= min_z
    }
}

impl BuildingSiteTerrainSnapshot {
    /// Pushes to `out` the captured sites of `snapshots` that a graded terrain site query of
    /// `rect` reads (`TERRAIN-07-E`, the road tool's preview): the allocator's own test
    /// ([`SiteRectQuery`]) at each snapshot's captured radius, a site captured by several
    /// snapshots once (by content: snapshots taken at different times may number it
    /// differently). For a rectangle inside the union of the snapshots' bounds whose sites have
    /// not changed since, these are the allocator's sites, so the bake's inputs are the
    /// allocator's. `O(captured sites of the snapshots + n log n)` for the `n` sites kept.
    pub(crate) fn push_graded_sites(
        snapshots: &[std::sync::Arc<BuildingSiteTerrainSnapshot>],
        rect: [f64; 4],
        out: &mut SiteBuffer,
    ) {
        thread_local! {
            static CANDIDATES: std::cell::RefCell<Vec<(u32, u32)>> =
                const { std::cell::RefCell::new(Vec::new()) };
        }
        let client =
            |&(snapshot, site): &(u32, u32)| &snapshots[snapshot as usize].sites[site as usize];
        CANDIDATES.with_borrow_mut(|candidates| {
            candidates.clear();
            for (index, snapshot) in snapshots.iter().enumerate() {
                let query = SiteRectQuery::new(rect, snapshot.max_site_radius_m);
                candidates.extend(
                    snapshot
                        .sites
                        .iter()
                        .enumerate()
                        .filter(|(_, site)| {
                            query.holds_parts(
                                site.lot_center,
                                site_surface_bounds(&site.footprint_world, &site.surfaces),
                            )
                        })
                        .map(|(site, _)| (index as u32, site as u32)),
                );
            }
            candidates.sort_unstable_by(|a, b| client(a).graded_content_cmp(client(b)));
            candidates.dedup_by(|a, b| client(a).graded_content_cmp(client(b)).is_eq());
            for candidate in candidates.iter() {
                let site = client(candidate);
                out.push_site(&site.footprint_world, site.support_height_m);
                for surface in &site.surfaces {
                    out.push_surface(surface.material, &surface.vertices_world);
                }
            }
        });
    }
}

impl BuildingSiteTerrainClient {
    // A total order on what the graded terrain reads of a site (support height, footprint and
    // surfaces, by bits), so equal content sorts together whatever the building index.
    fn graded_content_cmp(&self, other: &Self) -> std::cmp::Ordering {
        fn bits(points: &[Vector2]) -> impl Iterator<Item = [u32; 2]> + '_ {
            points.iter().map(|p| [p.x.to_bits(), p.y.to_bits()])
        }
        self.support_height_m
            .to_bits()
            .cmp(&other.support_height_m.to_bits())
            .then_with(|| bits(&self.footprint_world).cmp(bits(&other.footprint_world)))
            .then_with(|| self.surfaces.len().cmp(&other.surfaces.len()))
            .then_with(|| {
                self.surfaces
                    .iter()
                    .zip(&other.surfaces)
                    .map(|(a, b)| {
                        (a.material as u8)
                            .cmp(&(b.material as u8))
                            .then_with(|| bits(&a.vertices_world).cmp(bits(&b.vertices_world)))
                    })
                    .find(|order| order.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

fn push_graded_site(site: &BuildingSiteClient, out: &mut SiteBuffer) {
    out.push_site(&site.footprint_world, site.support_height_m);
    for surface in &site.surfaces {
        out.push_surface(surface.material, &surface.vertices_world);
    }
}

impl BuildingAllocator {
    pub(crate) fn terrain_site_snapshot_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> BuildingSiteTerrainSnapshot {
        let sites = self
            .site_candidate_indices_for_bounds(min_x, min_z, max_x, max_z)
            .into_iter()
            .filter_map(|building_idx| {
                let site = self.building_sites.get(building_idx)?;
                site.overlaps_bounds(min_x, min_z, max_x, max_z).then(|| {
                    BuildingSiteTerrainClient {
                        building_idx,
                        lot_center: site_lot_center(site),
                        footprint_world: site.footprint_world.clone(),
                        support_height_m: site.support_height_m,
                        surfaces: site.surfaces.clone(),
                    }
                })
            })
            .collect();
        BuildingSiteTerrainSnapshot {
            sites,
            max_site_radius_m: self.max_site_radius_m,
        }
    }

    pub(crate) fn append_building_site_bounds_for_world_bounds(
        &self,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        out: &mut Vec<[f32; 4]>,
    ) {
        out.extend(
            self.site_candidate_indices_for_bounds(min_x, min_z, max_x, max_z)
                .into_iter()
                .filter_map(|building_idx| {
                    let bounds = self.building_sites.get(building_idx)?.bounds();
                    overlaps(bounds, (min_x, min_z, max_x, max_z))
                        .then_some(<[f32; 4]>::from(bounds))
                }),
        );
    }

    pub(crate) fn mark_building_site_terrain_bounds_dirty(
        &self,
        terrain: &mut TerrainSystem,
        bounds: (f32, f32, f32, f32),
        margin_m: f32,
    ) {
        let (min_x, min_z, max_x, max_z) = bounds;
        let margin_m = margin_m.max(0.0);
        terrain.mark_render_patches_for_world_bounds(
            min_x - margin_m,
            min_z - margin_m,
            max_x + margin_m,
            max_z + margin_m,
        );
    }
}

impl BuildingSiteTerrainSnapshot {
    /// Returns ordered paving regions from the captured local sites.
    #[cfg(test)]
    pub(crate) fn paving_surfaces(&self) -> Vec<super::model::BuildingSiteSurfaceClient> {
        self.sites
            .iter()
            .flat_map(|site| site.surfaces.iter().cloned())
            .collect()
    }
}

fn overlaps(site: (f32, f32, f32, f32), query: (f32, f32, f32, f32)) -> bool {
    site.0 <= query.2 && site.2 >= query.0 && site.1 <= query.3 && site.3 >= query.1
}

/// The building sites meeting some rectangles, copied from the allocator
/// ([`BuildingAllocator::graded_site_snapshot`]): within any of them it answers every graded
/// terrain query with the sites the allocator would give, so a bake from it equals the bake from
/// the allocator bitwise, inputs included. A query inside none of the rectangles is a caller
/// error (debug-asserted).
pub(crate) struct SiteSnapshot {
    rects: Vec<[f64; 4]>,
    max_site_radius_m: f32,
    sites: Vec<BuildingSiteClient>,
}

impl SiteSource for SiteSnapshot {
    fn ready(&self) -> bool {
        true
    }

    fn sites_in_bounds(&self, rect: [f64; 4], out: &mut SiteBuffer) {
        debug_assert!(
            self.rects.iter().any(|outer| {
                rect[0] >= outer[0]
                    && rect[1] >= outer[1]
                    && rect[2] <= outer[2]
                    && rect[3] <= outer[3]
            }),
            "graded site snapshot queried outside its rectangles"
        );
        let query = SiteRectQuery::new(rect, self.max_site_radius_m);
        for site in self.sites.iter().filter(|site| query.holds(site)) {
            push_graded_site(site, out);
        }
    }
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

    #[cfg(test)]
    pub(super) fn height_at(&self, pos: Vector2) -> Option<f32> {
        // Aprons are owned by the graded terrain, not an independent flat plane.
        self.contains_point(pos).then_some(self.support_height_m)
    }

    #[cfg(test)]
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
            && (!self.building_site_query_index_ready() || !self.explicit_cell_sites.is_empty())
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

    /// The site's side of a change for the graded terrain ([`SiteChange`]): its
    /// [`Self::site_dirty_bounds`] and support height now. Read before a change for the old
    /// site and after it for the new one.
    pub(crate) fn site_change(&self, building_idx: usize) -> Option<SiteChange> {
        let bounds = self.site_dirty_bounds(building_idx)?;
        Some(SiteChange {
            bounds: bounds.into(),
            support_height_m: self.building_sites[building_idx].support_height_m,
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

    /// Records terrain changes for every site and cell exclusions only for explicit sites:
    /// the change's bounds in the legacy outbox and the whole [`SiteChange`] in the graded
    /// terrain's (`TERRAIN-07-C`). Ordinary parcels own their cell claims; clearing occupancy
    /// already queues their local frontage revalidation. Geometry work is bounded by edited
    /// sites, not their union's area.
    pub(crate) fn record_building_site_change(
        &mut self,
        building_idx: usize,
        change: Option<SiteChange>,
    ) {
        let bounds = change.map(|change| {
            let [min_x, min_z, max_x, max_z] = change.bounds;
            (min_x, min_z, max_x, max_z)
        });
        let handle = self.building_handles.handle(building_idx);
        if !handle.is_none() {
            self.parking_site_changes.push(handle);
        }
        self.graded_site_changes.extend(change);
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

    #[cfg(test)]
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

    #[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
