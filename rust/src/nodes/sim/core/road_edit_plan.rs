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
use std::sync::Mutex;

/// Immutable preparation and solved local graph delta against pinned road/source revisions.
///
/// Retains resolved splits/connections, terminal extensions, junction-adjusted profiles and clips
/// when the preview's mutation frontier is complete. Also owns local road/site CDT tiles, joined
/// patch buffers and structural stamps. Readiness pins every geometric dependency. Commit verifies
/// the adopted products under the simulation lock, then publishes them together or restores the
/// bounded graph/visual checkpoint. Reference-only dependent updates remain subsystem work.
/// Exact request/revision matching is O(raw input points), identity checks O(local records).
#[derive(Debug)]
pub(crate) struct RoadEditPlan {
    request: RoadPreviewRequest,
    terrain_source_generation: u64,
    prepared: PreparedRoadInput,
    validation: RoadPreviewValidation,
    topology: Option<RoadTopologyPlan>,
    terrain: Option<RoadTerrainPlan>,
    topology_reuse: Mutex<Option<RoadPreviewTopologyReuse>>,
}

impl RoadEditPlan {
    /// Builds the same local solution as the worker against borrowed live inputs for a stale or
    /// missing click plan. No resident graph, terrain or building collection is cloned.
    pub(crate) fn compile(core: &SimCore, request: RoadPreviewRequest) -> Self {
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
        let mut plan = Self::new(
            request,
            core.heightmap.source_generation(),
            prepared,
            preview.validation,
            reuse,
            topology,
        );
        let sites = plan
            .topology
            .as_ref()
            .and_then(|topology| topology.earthworks())
            .and_then(|earthworks| RoadTerrainSiteInputs::capture(core, earthworks));
        plan.compile_terrain(
            &core.heightmap,
            &core.region_graph,
            &core.transit_network.road_surface,
            sites,
        );
        plan
    }

    /// Complete backend readiness. Local water and parcel queries are repeated under the commit
    /// lock, so changes in these independent systems cannot authorize an obsolete result.
    pub(crate) fn status(&self, core: &SimCore) -> &'static str {
        if !self.validation.is_valid {
            return "invalid";
        }
        if self.topology.is_none() {
            return "provisional";
        }
        if self.request.surface_generation != core.road_tool_surface_generation
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
        let Some(terrain) = self.terrain.as_ref() else {
            return "provisional";
        };
        let state = terrain.status(core);
        if state != "compiled" {
            return state;
        }
        if self
            .topology_reuse
            .lock()
            .expect("road edit plan topology lock poisoned")
            .is_none()
        {
            return "consumed";
        }
        let fwd = self.request.fwd_lanes.clamp(0, 255) as u8;
        let bkw = self.request.bkw_lanes.clamp(0, 255) as u8;
        let validation = crate::nodes::sim::road_tool::validate_road_candidate_against_water(
            self.prepared.class,
            &self.prepared.points,
            fwd,
            bkw,
            &core.watermap,
            self.validation.clone(),
        );
        let width = (f32::from(fwd) + f32::from(bkw)) * crate::config::LANE_WIDTH;
        if !validation.is_valid
            || !core
                .zoning
                .parcel_ids_overlapping_road_corridor(
                    &self.prepared.points,
                    width.max(2.0) * 0.5 + crate::config::SIDEWALK_WIDTH,
                )
                .is_empty()
        {
            return "invalid";
        }
        if !terrain.has_complete_products() {
            return "provisional";
        }
        "ready"
    }

    /// Retains the worker's preparation, solved graph delta and optional canonical surface products.
    pub(super) fn new(
        request: RoadPreviewRequest,
        terrain_source_generation: u64,
        prepared: PreparedRoadInput,
        validation: RoadPreviewValidation,
        topology_reuse: Option<RoadPreviewTopologyReuse>,
        topology: Option<RoadTopologyPlan>,
    ) -> Self {
        Self {
            request,
            terrain_source_generation,
            prepared,
            validation,
            topology,
            terrain: None,
            topology_reuse: Mutex::new(topology_reuse),
        }
    }

    /// Compiles captured local site and road products outside the simulation lock.
    pub(super) fn compile_terrain(
        &mut self,
        terrain: &crate::simulation::terrain::TerrainSystem,
        graph: &RegionGraph,
        surface: &crate::simulation::network::surface::RoadSurfaceSystem,
        sites: Option<RoadTerrainSiteInputs>,
    ) {
        self.terrain = self
            .topology
            .as_ref()
            .and_then(|plan| plan.earthworks())
            .map(|earthworks| RoadTerrainPlan::compile(terrain, earthworks, graph, surface, sites));
    }

    /// Borrows immutable terrain candidates after the caller has adopted this plan's topology.
    pub(crate) fn terrain(&self) -> Option<&RoadTerrainPlan> {
        self.terrain.as_ref()
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
        (self.validation.is_valid
            && self.request.surface_generation == surface_generation
            && self.terrain_source_generation == terrain_source_generation
            && self.request.fwd_lanes == fwd_lanes
            && self.request.bkw_lanes == bkw_lanes
            && self.request.snap_to_existing_roads == snap_to_existing_roads
            && self.request.points == raw_points)
            .then_some(&self.prepared)
    }

    /// Returns the compiled road validation after exact input/dependency matching.
    pub(crate) fn validation(&self) -> &RoadPreviewValidation {
        &self.validation
    }

    /// Borrows the solved local delta after exact request/revision matching by the caller.
    pub(crate) fn topology_for(&self, graph: &RegionGraph) -> Option<&RoadTopologyPlan> {
        self.topology
            .as_ref()
            .filter(|topology| topology.source_identities_match(graph))
    }

    /// Transfers optional surface products once, after the authoritative input check.
    pub(crate) fn take_topology_reuse(&self) -> Option<RoadPreviewTopologyReuse> {
        self.topology_reuse
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
                request_id: 7,
                surface_generation: 11,
                points: raw.clone(),
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
            },
            3,
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
            plan.prepared_input_for(11, 3, &plan.prepared.points, 1, 1, true)
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
                request_id: 7,
                surface_generation: 11,
                points: points.clone(),
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
            },
            3,
            prepared,
            RoadPreviewValidation::valid(0.0),
            None,
            None,
        );
        let input = plan.prepared_input_for(11, 3, &points, 1, 1, true).unwrap();
        assert!(std::ptr::eq(input, &plan.prepared));
        assert_eq!(input.extension.as_ref().unwrap().existing_edge_idx, 4);
    }
}
