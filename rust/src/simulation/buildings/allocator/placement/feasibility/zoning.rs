// SPDX-License-Identifier: GPL-2.0-only

//! Asset-independent parcel terrain checks using the shared grading solver and local cache.

use super::*;
use crate::simulation::buildings::allocator::site::{
    solve_building_site_support_height, zoning_support_footprint,
};
use crate::simulation::zoning::parcels::geometry_for_parcel;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ZoningSiteKey {
    parcel: u64,
    edge: usize,
    side: i8,
    replaced_building: Option<usize>,
    attachment: u32,
    cell_size: u32,
    footprint: [[u32; 2]; 4],
}

impl ZoningSiteKey {
    fn new(geometry: &ParcelGeometry, existing: Option<&ZoningParcel>, cell_size: f32) -> Self {
        Self {
            parcel: existing.map_or(0, |p| p.id().raw()),
            edge: geometry.edge_idx,
            side: geometry.side,
            replaced_building: existing.and_then(ZoningParcel::occupied_building),
            attachment: geometry.frontage_center_t.to_bits(),
            cell_size: cell_size.to_bits(),
            footprint: zoning_support_footprint(geometry).map(|p| [p.x.to_bits(), p.y.to_bits()]),
        }
    }
}

/// Parcel poses share terrain results across all profiles and installed asset selections.
#[derive(Default)]
pub(super) struct ZoningSiteCache {
    entries: Mutex<HashMap<ZoningSiteKey, CachedSite>>,
    previews: Mutex<HashMap<ZoningSiteKey, CachedSite>>,
}

impl ZoningSiteCache {
    fn poses(&self, parcel: u64) -> &Mutex<HashMap<ZoningSiteKey, CachedSite>> {
        if parcel == 0 {
            &self.previews
        } else {
            &self.entries
        }
    }

    /// Removes obsolete stored-parcel poses during the existing hourly cache maintenance pass.
    pub(super) fn prune(&self, zoning: &ZoningSystem) {
        self.entries.lock().unwrap().retain(|key, _| {
            zoning.parcel_by_raw_id(key.parcel).is_some_and(|p| {
                *key == ZoningSiteKey::new(
                    &geometry_for_parcel(p),
                    Some(p),
                    zoning.config.zone_cell_m,
                )
            })
        });
    }
}

impl BuildingAllocator {
    /// Checks the selected lot's level interior and grading strip identically for every profile.
    /// Assets are irrelevant to zoning; actual building growth still validates its own footprint.
    pub(crate) fn zoning_site_feasibility(
        &self,
        geometry: &ParcelGeometry,
        profile_id: u16,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
        env: BuildingSiteEnvironment<'_>,
    ) -> Result<(), &'static str> {
        if self.field_clearance.overlaps_polygon(&geometry.corners) {
            return Err("field_overlap");
        }
        if self.parcel_geometry_overlaps_explicit_site(geometry, zoning.config.zone_cell_m) {
            return Err("parcel overlaps a placed building site");
        }
        // Clearing a zoning designation must remain possible even on unusable terrain.
        if profile_id == 0 {
            return Ok(());
        }
        let profile = zoning
            .profiles
            .profile_by_runtime_id(profile_id)
            .ok_or("unknown zoning profile")?;
        zone_type_to_zone_class(profile.zone_type).ok_or("unsupported zoning profile")?;
        let existing = zoning.parcel_at(geometry.center);
        let key = ZoningSiteKey::new(geometry, existing, zoning.config.zone_cell_m);
        self.cached_zoning_terrain(geometry, key, graph, env)
            .map(|_| ())
            .map_err(|error| match error {
                DemandSpawnPlacementRejection::SiteSupportTieInInvalid => {
                    "level lot cannot connect to road and terrain within the grading envelope"
                }
                _ => "lot cannot connect to the frontage road or neighboring sites",
            })
    }

    fn cached_zoning_terrain(
        &self,
        geometry: &ParcelGeometry,
        key: ZoningSiteKey,
        graph: &RegionGraph,
        env: BuildingSiteEnvironment<'_>,
    ) -> Verdict {
        if !env.road_surface.published_generation_matches_source() {
            return Err(DemandSpawnPlacementRejection::FrontageRoadSurfaceMissing);
        }
        let epoch = (
            env.terrain.source_generation(),
            env.terrain.visual_generation(),
            env.road_surface.compile_invalidation_generation,
            self.building_ref_revision(),
            0, // Asset registration cannot change this footprint or its terrain verdict.
        );
        let poses = self.site_feasibility.zoning.poses(key.parcel);
        let previous = poses.lock().unwrap().get(&key).cloned();
        if let Some(previous) = previous.filter(|old| old.is_current(epoch, self, env)) {
            if previous.epoch != epoch
                && let Some(old) = poses.lock().unwrap().get_mut(&key)
            {
                old.epoch = epoch;
            }
            return previous.verdict;
        }
        #[cfg(test)]
        self.site_feasibility
            .solves
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let verdict = self.solve_zoning_terrain(key, graph, env);
        let bounds = (
            geometry.aabb_min.x,
            geometry.aabb_min.y,
            geometry.aabb_max.x,
            geometry.aabb_max.y,
        );
        let dependencies = Arc::new(Dependencies::capture_bounds(self, bounds, env));
        let mut poses = poses.lock().unwrap();
        // Use the same bounded cursor policy as building previews; stored parcels are pruned hourly.
        if key.parcel == 0 && poses.len() >= 256 {
            poses.clear();
        }
        poses.insert(
            key,
            CachedSite {
                epoch,
                dependencies,
                verdict,
            },
        );
        verdict
    }

    fn solve_zoning_terrain(
        &self,
        key: ZoningSiteKey,
        graph: &RegionGraph,
        env: BuildingSiteEnvironment<'_>,
    ) -> Verdict {
        let point = Self::claimed_road_side_connection_pos(
            graph,
            key.edge,
            key.side,
            f32::from_bits(key.attachment),
        )
        .ok_or(DemandSpawnPlacementRejection::FrontageRoadSurfaceMissing)?;
        let preferred = env
            .road_surface
            .sample_visible_surface_height(graph, env.terrain, point.x, point.y)
            .ok_or(DemandSpawnPlacementRejection::FrontageRoadSurfaceMissing)?;
        let footprint = key
            .footprint
            .map(|p| Vector2::new(f32::from_bits(p[0]), f32::from_bits(p[1])));
        let height = solve_building_site_support_height(
            &footprint,
            preferred,
            &[(point, preferred)],
            env.terrain,
            graph,
            env.road_surface,
        )
        .ok_or(DemandSpawnPlacementRejection::SiteSupportTieInInvalid)?;
        self.validate_footprint_neighbor_height(
            &footprint,
            height,
            f32::from_bits(key.cell_size),
            key.replaced_building,
        )?;
        if !building_site_support_tie_in_is_valid(
            &footprint,
            height,
            env.terrain,
            graph,
            env.road_surface,
        ) {
            return Err(DemandSpawnPlacementRejection::SiteSupportTieInInvalid);
        }
        Ok(height)
    }
}
