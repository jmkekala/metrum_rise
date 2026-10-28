// SPDX-License-Identifier: GPL-2.0-only

//! Exact-input local topology/profile planning shared by preview and authoritative insertion.

use super::road_preview::RoadPreviewRequest;
use super::{RoadRejection, SimCore};
use crate::simulation::agriculture::FieldClearanceIndex;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::road_edit::{
    PreparedRoadInput, RoadPreviewValidation, RoadTopologyPlan, plan_road_edit,
};
use crate::simulation::network::roadbed::{PieceMesh, RoadbedSystem};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::{
    ZoningSystem,
    cells::{CellMaps, CellStore},
};
use godot::prelude::Vector3;

/// Immutable preparation and solved local graph delta against pinned road/source revisions.
///
/// Retains resolved splits/connections, terminal extensions, junction-adjusted profiles and clips
/// when the edit's mutation frontier is complete. Readiness pins every dependency; commit
/// verifies the plan under the simulation lock before adopting it.
/// Exact request/revision matching is O(raw input points), identity checks O(local records).
#[derive(Debug)]
pub(crate) struct RoadEditPlan {
    request: RoadPreviewRequest,
    terrain_source_generation: u64,
    prepared: PreparedRoadInput,
    validation: RoadPreviewValidation,
    topology: Option<RoadTopologyPlan>,
}

// Which live-state check refused a plan; no allocation, since readiness checks poll it.
#[derive(Clone, Copy)]
enum FailedCheck {
    Validation(&'static str),
    Parcels { count: usize, first: u64 },
}

/// A committed field or painted zoning cell under an edit's road pieces (`ROAD-68` open
/// question 1). The pieces are the ones the edit publishes, so junction corners and terminal
/// caps count as well as the spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReservedOverlap {
    /// A committed field reservation (`field_overlap`).
    Field,
    /// Painted zoning cells or their occupied grace claims (`zoning_cell_overlap`).
    CellZoning,
}

impl ReservedOverlap {
    /// The first reservation the edit's new pieces newly cover, piece by piece in order, fields
    /// before cells; `before` are the pieces the edit replaces. A painted cell is refused when a
    /// new piece covers it and no replaced piece did ([`ZoningSystem::cells_newly_covered_by_road_piece`]);
    /// fields when a new piece overlaps one and no replaced piece overlapped any. The commit
    /// passes the live indices and the road tool's preview the view's copies of them (`ROAD-68`
    /// open question 25). O(pieces * reservations in their bounds * top triangles); a piece with
    /// no reservation in its bounds costs one bounds query each.
    pub(crate) fn first<'a, M: CellMaps>(
        fields: &FieldClearanceIndex,
        cells: &CellStore<M>,
        pieces: impl IntoIterator<Item = &'a PieceMesh>,
        before: &[&PieceMesh],
    ) -> Option<Self> {
        let fields_before = before.iter().any(|mesh| fields.overlaps_road_piece(mesh));
        pieces.into_iter().find_map(|mesh| {
            if !fields_before && fields.overlaps_road_piece(mesh) {
                Some(Self::Field)
            } else if ZoningSystem::cells_newly_covered_by_road_piece(cells, mesh, before) {
                Some(Self::CellZoning)
            } else {
                None
            }
        })
    }

    /// [`Self::first`] against the live core's fields and cells, as the commit checks.
    pub(crate) fn first_in_core<'a>(
        core: &SimCore,
        pieces: impl IntoIterator<Item = &'a PieceMesh>,
        before: &[&PieceMesh],
    ) -> Option<Self> {
        Self::first(
            &core.allocator.field_clearance,
            &core.zoning.cells,
            pieces,
            before,
        )
    }

    /// The reason the commit reports and the preview shows for this overlap.
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Field => "field_overlap",
            Self::CellZoning => "zoning_cell_overlap",
        }
    }

    /// The commit's report of this overlap.
    pub(crate) fn rejection(self) -> RoadRejection {
        let detail = match self {
            Self::Field => "the road or its junctions overlap a farm field",
            Self::CellZoning => "the road or its junctions overlap painted zoning cells",
        };
        RoadRejection::new(self.reason(), detail)
    }
}

impl RoadEditPlan {
    /// Builds the local road solution from borrowed live inputs without cloning the city.
    pub(crate) fn compile(core: &SimCore, request: RoadPreviewRequest) -> Self {
        Self::compile_from(
            &core.heightmap,
            &core.region_graph,
            &core.transit_network.roadbed,
            request,
        )
    }

    /// Builds the local road solution from one generation's terrain, graph and roadbed: the
    /// live core's at commit, the road tool's context in a preview (`ROAD-68-E`), through the
    /// same preparation and validation, so the stroke reads no piece its edit rebuilds (`ROAD-68`
    /// open question 16). O(passes * (stroke points + local excerpt records)), one or two passes.
    pub(crate) fn compile_from(
        terrain: &TerrainSystem,
        graph: &RegionGraph,
        roadbed: &RoadbedSystem,
        request: RoadPreviewRequest,
    ) -> Self {
        let (prepared, validation, topology) = plan_road_edit(
            &request.points,
            request.fwd_lanes.clamp(0, 255) as u8,
            request.bkw_lanes.clamp(0, 255) as u8,
            terrain,
            graph,
            roadbed,
            request.snap_to_existing_roads,
        );
        Self::new(
            request,
            terrain.source_generation(),
            prepared,
            validation,
            topology,
        )
    }

    /// The stroke this plan was compiled for.
    pub(crate) fn request(&self) -> &RoadPreviewRequest {
        &self.request
    }

    /// The prepared stroke.
    pub(crate) fn prepared(&self) -> &PreparedRoadInput {
        &self.prepared
    }

    /// Readiness against the live core. Local water and parcel queries repeat under the commit
    /// lock, so changes in these independent systems cannot authorize an obsolete result. Fields
    /// and painted cells are tested against the pieces the commit installs ([`ReservedOverlap`],
    /// `ROAD-68` open question 1), which a plan alone does not have.
    pub(crate) fn status(&self, core: &SimCore) -> &'static str {
        if !self.validation.is_valid {
            return "invalid";
        }
        if self.topology.is_none() {
            return "provisional";
        }
        if self.request.surface_generation != core.road_tool_surface_generation
            || self.terrain_source_generation != core.heightmap.source_generation()
            || self.topology_for(&core.region_graph).is_none()
        {
            return "stale";
        }
        if !core.transit_network.roadbed.is_current() {
            return "pending";
        }
        if self.failed_check(core).is_some() {
            return "invalid";
        }
        "ready"
    }

    // The first live-state check that refuses this road: water or parcels.
    fn failed_check(&self, core: &SimCore) -> Option<FailedCheck> {
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
        if !validation.is_valid {
            return Some(FailedCheck::Validation(validation.invalid_reason));
        }
        let width = (f32::from(fwd) + f32::from(bkw)) * crate::config::LANE_WIDTH;
        let half_width_m = width.max(2.0) * 0.5 + crate::config::SIDEWALK_WIDTH;
        let parcels = core
            .zoning
            .parcel_ids_overlapping_road_corridor(&self.prepared.points, half_width_m);
        parcels.first().map(|&first| FailedCheck::Parcels {
            count: parcels.len(),
            first,
        })
    }

    /// Explains why this plan cannot be committed: the failed check when one failed, otherwise
    /// the plan's state (`road_plan_stale`, `road_plan_pending`, ...). Built only on refusal.
    pub(crate) fn rejection(&self, core: &SimCore) -> RoadRejection {
        let validation = &self.validation;
        let prepared = &self.prepared.points;
        if !validation.is_valid {
            return RoadRejection::from_validation(validation.invalid_reason, validation, prepared);
        }
        let status = self.status(core);
        if status == "invalid"
            && let Some(check) = self.failed_check(core)
        {
            return match check {
                FailedCheck::Validation(reason) => {
                    RoadRejection::from_validation(reason, validation, prepared)
                }
                FailedCheck::Parcels { count, first } => {
                    RoadRejection::from_validation("parcel_overlap", validation, prepared)
                        .with_note(format!("{count} parcels, first {first}"))
                }
            };
        }
        let (reason, detail) = match status {
            "stale" => (
                "road_plan_stale",
                "the road or terrain changed after the stroke was prepared",
            ),
            "pending" => (
                "road_plan_pending",
                "the roadbed was still being rebuilt for an earlier edit",
            ),
            "provisional" => (
                "road_plan_provisional",
                "the stroke's local road plan was incomplete",
            ),
            _ => (
                "road_plan_mismatch",
                "the prepared plan does not match the submitted stroke",
            ),
        };
        RoadRejection::new(reason, detail)
    }

    /// Retains the preparation and solved graph delta.
    pub(super) fn new(
        request: RoadPreviewRequest,
        terrain_source_generation: u64,
        prepared: PreparedRoadInput,
        validation: RoadPreviewValidation,
        topology: Option<RoadTopologyPlan>,
    ) -> Self {
        Self {
            request,
            terrain_source_generation,
            prepared,
            validation,
            topology,
        }
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

    /// Returns the plan's road validation after exact input/dependency matching.
    pub(crate) fn validation(&self) -> &RoadPreviewValidation {
        &self.validation
    }

    /// Borrows the solved local delta after exact request/revision matching by the caller.
    pub(crate) fn topology_for(&self, graph: &RegionGraph) -> Option<&RoadTopologyPlan> {
        self.topology
            .as_ref()
            .filter(|topology| topology.source_identities_match(graph))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::road_edit::prepare_road_input_for_tool;
    use crate::simulation::network::roadbed::RoadbedSystem;
    use crate::simulation::terrain::TerrainSystem;

    #[test]
    fn plan_requires_exact_raw_inputs_and_source_revisions() {
        let raw = vec![Vector3::ZERO, Vector3::new(24.0, 0.0, 0.0)];
        let prepared = prepare_road_input_for_tool(
            &raw,
            &TerrainSystem::new(65, 65),
            &RegionGraph::new(),
            &RoadbedSystem::new(16.0),
            true,
        );
        let plan = RoadEditPlan::new(
            RoadPreviewRequest {
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
}
