// SPDX-License-Identifier: GPL-2.0-only

//! Exact-input local topology/profile planning shared by preview and authoritative insertion.

use super::road_preview::RoadPreviewRequest;
use super::road_terrain_plan::RoadTerrainSiteInputs;
use super::{RoadTerrainPlan, SimCore};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::road_edit::RoadTopologyPlan;
use crate::simulation::network::surface::{
    PreparedRoadInput, RoadPreviewTopologyReuse, RoadPreviewValidation,
};
use godot::prelude::Vector3;
use std::sync::{Arc, Mutex};

/// Immutable preparation and solved local graph delta against pinned road/source revisions.
///
/// Retains resolved splits/connections, terminal extensions, junction-adjusted profiles and clips
/// when the preview's mutation frontier is complete. A click shares these products and adds its
/// own local terrain tiles, patch buffers and stamps. Readiness pins every dependency. Commit verifies
/// the adopted products under the simulation lock, then publishes them together or restores the
/// bounded graph/visual checkpoint. Reference-only dependent updates remain subsystem work.
/// Exact request/revision matching is O(raw input points), identity checks O(local records).
#[derive(Debug)]
pub(crate) struct RoadEditPlan {
    road: Arc<RoadGeometryPlan>,
    terrain: Option<Arc<RoadTerrainPlan>>,
}

// Shared immutable road solve. Only adoption of surface products is single-use. Completing
// terrain on click never mutates the published preview or clones its geometry buffers.
#[derive(Debug)]
struct RoadGeometryPlan {
    request: RoadPreviewRequest,
    terrain_source_generation: u64,
    terrain_visual_generation: u64,
    prepared: PreparedRoadInput,
    validation: RoadPreviewValidation,
    topology: Option<RoadTopologyPlan>,
    topology_reuse: Mutex<Option<RoadPreviewTopologyReuse>>,
}

impl RoadEditPlan {
    /// Builds complete click products from borrowed live inputs when no preview is available.
    pub(crate) fn compile(core: &SimCore, request: RoadPreviewRequest) -> Self {
        Self::compile_road(core, request).complete_for_commit(core)
    }

    /// Builds the same local road solution as the worker without cloning the resident city.
    pub(crate) fn compile_road(core: &SimCore, request: RoadPreviewRequest) -> Self {
        use crate::simulation::network::surface::RoadSurfaceSystem;
        let prepared = RoadSurfaceSystem::prepare_road_input_for_tool(
            &request.points,
            &core.heightmap,
            &core.region_graph,
            &core.transit_network.road_surface,
            request.snap_to_existing_roads,
        );
        let (preview, reuse, _, topology) = core
            .transit_network
            .road_surface
            .compile_prepared_preview_surface_with_topology_reuse(
                &prepared,
                request.fwd_lanes.clamp(0, 255) as u8,
                request.bkw_lanes.clamp(0, 255) as u8,
                &core.heightmap,
                &core.region_graph,
                &core.transit_network.road_surface,
            );
        Self::new(
            request,
            core.heightmap.source_generation(),
            core.heightmap.visual_generation(),
            prepared,
            preview.validation,
            reuse,
            topology,
        )
    }

    /// Completes terrain against live local dependencies while sharing the exact road solve.
    /// The caller checks road/input reuse before this O(affected terrain patches/sites) work.
    pub(crate) fn complete_for_commit(&self, core: &SimCore) -> Self {
        if self.terrain.is_some() && self.status(core) == "ready" {
            return Self {
                road: Arc::clone(&self.road),
                terrain: self.terrain.clone(),
            };
        }
        if let Some(earthworks) = self.preview_earthworks(
            &core.heightmap,
            &core.region_graph,
            &core.transit_network.road_surface,
        ) {
            let sites = RoadTerrainSiteInputs::capture(core, &earthworks);
            return self.with_preview_terrain(
                &core.heightmap,
                &core.region_graph,
                &core.transit_network.road_surface,
                &earthworks,
                sites,
            );
        }
        Self {
            road: Arc::clone(&self.road),
            terrain: None,
        }
    }

    /// Builds bounded terrain dependencies off-lock for the optional full preview.
    pub(super) fn preview_earthworks(
        &self,
        terrain: &crate::simulation::terrain::TerrainSystem,
        graph: &RegionGraph,
        surface: &crate::simulation::network::surface::RoadSurfaceSystem,
    ) -> Option<Arc<crate::simulation::network::surface::RoadEarthworkPlan>> {
        self.road
            .topology
            .as_ref()?
            .roads()?
            .compile_earthworks(surface, graph, terrain)
            .map(Arc::new)
    }

    /// Shares the road solve and compiles only captured local terrain/site inputs off-lock.
    pub(super) fn with_preview_terrain(
        &self,
        terrain: &crate::simulation::terrain::TerrainSystem,
        graph: &RegionGraph,
        surface: &crate::simulation::network::surface::RoadSurfaceSystem,
        earthworks: &Arc<crate::simulation::network::surface::RoadEarthworkPlan>,
        sites: Option<RoadTerrainSiteInputs>,
    ) -> Self {
        Self {
            road: Arc::clone(&self.road),
            terrain: Some(Arc::new(RoadTerrainPlan::compile(
                terrain, earthworks, graph, surface, sites,
            ))),
        }
    }

    /// Complete backend readiness. Local water, parcel and field queries repeat under the commit
    /// lock, so changes in these independent systems cannot authorize an obsolete result.
    pub(crate) fn status(&self, core: &SimCore) -> &'static str {
        let state = self.road_status(core);
        if state != "ready" {
            return state;
        }
        let Some(terrain) = self.terrain.as_ref() else {
            return "provisional";
        };
        let state = terrain.status(core);
        if state != "compiled" {
            return state;
        }
        if !terrain.has_complete_products() {
            return "provisional";
        }
        "ready"
    }

    /// Road geometry readiness only; this never authorizes terrain adoption or a live commit.
    pub(crate) fn road_status(&self, core: &SimCore) -> &'static str {
        if !self.road.validation.is_valid {
            return "invalid";
        }
        if self.road.topology.is_none() {
            return "provisional";
        }
        if self.road.request.surface_generation != core.road_tool_surface_generation
            || self.road.terrain_source_generation != core.heightmap.source_generation()
            || self.road.terrain_visual_generation != core.heightmap.visual_generation()
            || self.topology_for(&core.region_graph).is_none()
        {
            return "stale";
        }
        if !core
            .transit_network
            .road_surface
            .published_generation_matches_source()
        {
            return "pending";
        }
        if self
            .road
            .topology_reuse
            .lock()
            .expect("road edit plan topology lock poisoned")
            .is_none()
        {
            return "consumed";
        }
        let fwd = self.road.request.fwd_lanes.clamp(0, 255) as u8;
        let bkw = self.road.request.bkw_lanes.clamp(0, 255) as u8;
        let validation = crate::nodes::sim::road_tool::validate_road_candidate_against_water(
            self.road.prepared.class,
            &self.road.prepared.points,
            fwd,
            bkw,
            &core.watermap,
            self.road.validation.clone(),
        );
        let width = (f32::from(fwd) + f32::from(bkw)) * crate::config::LANE_WIDTH;
        if !validation.is_valid
            || self.overlaps_fields(core)
            || self.overlaps_cell_zoning(core)
            || core.zoning.cells_overlap_road_corridor(
                &self.road.prepared.points,
                width.max(2.0) * 0.5 + crate::config::SIDEWALK_WIDTH,
            )
            || !core
                .zoning
                .parcel_ids_overlapping_road_corridor(
                    &self.road.prepared.points,
                    width.max(2.0) * 0.5 + crate::config::SIDEWALK_WIDTH,
                )
                .is_empty()
        {
            return "invalid";
        }
        "ready"
    }

    /// Rechecks current field reservations against final local road and junction footprints.
    pub(crate) fn overlaps_fields(&self, core: &SimCore) -> bool {
        self.road
            .topology
            .as_ref()
            .and_then(|topology| topology.roads())
            .is_some_and(|roads| roads.overlaps_fields(&core.allocator.field_clearance))
    }

    /// Revalidates cell reservations even when paint changed after this road preview was built.
    pub(crate) fn overlaps_cell_zoning(&self, core: &SimCore) -> bool {
        self.road
            .topology
            .as_ref()
            .and_then(|topology| topology.roads())
            .is_some_and(|roads| roads.overlaps_cell_zoning(&core.zoning))
    }

    /// Retains the worker's preparation, solved graph delta and optional canonical surface products.
    pub(super) fn new(
        request: RoadPreviewRequest,
        terrain_source_generation: u64,
        terrain_visual_generation: u64,
        prepared: PreparedRoadInput,
        validation: RoadPreviewValidation,
        topology_reuse: Option<RoadPreviewTopologyReuse>,
        topology: Option<RoadTopologyPlan>,
    ) -> Self {
        Self {
            road: Arc::new(RoadGeometryPlan {
                request,
                terrain_source_generation,
                terrain_visual_generation,
                prepared,
                validation,
                topology,
                topology_reuse: Mutex::new(topology_reuse),
            }),
            terrain: None,
        }
    }

    /// Borrows immutable terrain candidates after the caller has adopted this plan's topology.
    pub(crate) fn terrain(&self) -> Option<&RoadTerrainPlan> {
        self.terrain.as_deref()
    }

    /// Borrows the solved input only for an exact click on unchanged road/terrain dependencies.
    pub(crate) fn prepared_input_for(
        &self,
        surface_generation: u64,
        terrain_source_generation: u64,
        raw_points: &[Vector3],
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> Option<&PreparedRoadInput> {
        (self.road.validation.is_valid
            && self.road.request.surface_generation == surface_generation
            && self.road.terrain_source_generation == terrain_source_generation
            && self.road.request.fwd_lanes == fwd_lanes
            && self.road.request.bkw_lanes == bkw_lanes
            && self.road.request.snap_to_existing_roads == snap_to_existing_roads
            && self.road.request.points == raw_points)
            .then_some(&self.road.prepared)
    }

    /// Returns the compiled road validation after exact input/dependency matching.
    pub(crate) fn validation(&self) -> &RoadPreviewValidation {
        &self.road.validation
    }

    /// Borrows the solved local delta after exact request/revision matching by the caller.
    pub(crate) fn topology_for(&self, graph: &RegionGraph) -> Option<&RoadTopologyPlan> {
        self.road
            .topology
            .as_ref()
            .filter(|topology| topology.source_identities_match(graph))
    }

    /// Transfers optional surface products once, after the authoritative input check.
    pub(crate) fn take_topology_reuse(&self) -> Option<RoadPreviewTopologyReuse> {
        self.road
            .topology_reuse
            .lock()
            .expect("road edit plan topology lock poisoned")
            .take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::surface::{RoadExtensionReprofile, RoadSurfaceSystem};
    use crate::simulation::terrain::TerrainSystem;

    #[test]
    fn plan_requires_exact_raw_inputs_and_source_revisions() {
        let raw = vec![Vector3::ZERO, Vector3::new(24.0, 0.0, 0.0)];
        let prepared = RoadSurfaceSystem::prepare_road_input_for_tool(
            &raw,
            &TerrainSystem::new(65, 65),
            &RegionGraph::new(),
            &RoadSurfaceSystem::new(16.0),
            true,
        );
        let plan = RoadEditPlan::new(
            RoadPreviewRequest {
                include_terrain: false,
                request_id: 7,
                surface_generation: 11,
                points: raw.clone(),
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
            },
            3,
            5,
            prepared,
            RoadPreviewValidation::valid(0.0),
            None,
            None,
        );
        assert!(plan.prepared_input_for(11, 3, &raw, 1, 1, true).is_some());
        assert!(plan.prepared_input_for(12, 3, &raw, 1, 1, true).is_none());
        assert!(plan.prepared_input_for(11, 4, &raw, 1, 1, true).is_none());
        assert!(plan.prepared_input_for(11, 3, &raw, 2, 1, true).is_none());
        assert!(plan.prepared_input_for(11, 3, &raw, 1, 2, true).is_none());
        assert!(plan.prepared_input_for(11, 3, &raw, 1, 1, false).is_none());
        let mut changed = raw.clone();
        changed[0].y += 0.01; // May prepare identically, but is still a different authored input.
        assert!(
            plan.prepared_input_for(11, 3, &changed, 1, 1, true)
                .is_none()
        );
        assert!(
            plan.prepared_input_for(11, 3, &plan.road.prepared.points, 1, 1, true)
                .is_none()
        );
    }

    #[test]
    fn plan_retains_terminal_reprofile_without_repreparation() {
        let points = vec![Vector3::ZERO, Vector3::new(24.0, 0.0, 0.0)];
        let mut prepared = RoadSurfaceSystem::prepare_road_input_for_tool(
            &points,
            &TerrainSystem::new(65, 65),
            &RegionGraph::new(),
            &RoadSurfaceSystem::new(16.0),
            true,
        );
        prepared.extension = Some(RoadExtensionReprofile {
            snapped_node_id: 9,
            existing_edge_idx: 4,
            existing_points: points.clone(),
            snapped_node_pos: Vector3::ZERO,
        });
        let plan = RoadEditPlan::new(
            RoadPreviewRequest {
                include_terrain: false,
                request_id: 7,
                surface_generation: 11,
                points: points.clone(),
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
            },
            3,
            5,
            prepared,
            RoadPreviewValidation::valid(0.0),
            None,
            None,
        );
        let input = plan.prepared_input_for(11, 3, &points, 1, 1, true).unwrap();
        assert!(std::ptr::eq(input, &plan.road.prepared));
        assert_eq!(input.extension.as_ref().unwrap().existing_edge_idx, 4);
    }
}
