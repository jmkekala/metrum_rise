// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool preview requests, the immutable road-tool query snapshot and the synchronous
//! preview (`ROAD-68-E`): the commit's own plan, compiled pieces and their dependency
//! fingerprint, built on the road tool's context without the core lock.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use godot::prelude::Vector3;

use super::road_edit_plan::RoadEditPlan;
use super::road_tool_context::RoadPreviewContext;
use super::state::SimCore;
use crate::nodes::sim::road_tool::validate_road_candidate_against_water;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::render::NetworkMeshData;
use crate::simulation::network::render::road::RoadRenderer;
use crate::simulation::network::road_edit::{
    RoadPreviewValidation, prepare_road_input_for_tool, validate_road_candidate_fast,
};
use crate::simulation::network::roadbed::{RoadbedDelta, RoadbedSystem, SurfaceChunkKey};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;

/// One road-tool stroke submitted for preview or commit planning.
#[derive(Debug)]
pub(crate) struct RoadPreviewRequest {
    pub(crate) surface_generation: u64,
    pub(crate) points: Vec<Vector3>,
    pub(crate) fwd_lanes: i32,
    pub(crate) bkw_lanes: i32,
    pub(crate) snap_to_existing_roads: bool,
}

/// Corridor clearance of a stroke against live parcels and field reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoadCorridorClearance {
    /// Parcels the corridor overlaps; any overlap refuses the stroke (`parcel_overlap`).
    pub(crate) overlapping_parcels: usize,
    /// Lowest overlapping parcel id, present when any parcel overlaps.
    pub(crate) first_overlapping_parcel: Option<u64>,
    /// Whether the corridor overlaps a committed field reservation (`field_overlap`).
    pub(crate) field_overlap: bool,
    /// Zoning overlay revision the parcel query read.
    pub(crate) zoning_revision: u64,
    /// Agriculture field revision the field query read.
    pub(crate) field_revision: u64,
}

impl RoadCorridorClearance {
    /// Queries the parcel chunk index and the field reservations along the corridor that the
    /// lanes and sidewalks cover. O(corridor segments × parcels and fields in the touched chunks).
    pub(crate) fn evaluate(
        core: &SimCore,
        points: &[Vector3],
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> Self {
        let lanes = fwd_lanes.clamp(0, i32::from(u8::MAX)) + bkw_lanes.clamp(0, i32::from(u8::MAX));
        let half_width = (lanes as f32 * crate::config::LANE_WIDTH).max(2.0) * 0.5
            + crate::config::SIDEWALK_WIDTH;
        let parcels = core
            .zoning
            .parcel_ids_overlapping_road_corridor(points, half_width);
        Self {
            overlapping_parcels: parcels.len(),
            first_overlapping_parcel: parcels.first().copied(),
            field_overlap: core
                .allocator
                .field_clearance
                .overlaps_road_corridor(points, half_width),
            zoning_revision: core.zoning.overlay_revision(),
            field_revision: core.agriculture.visual_revision(),
        }
    }
}

/// Immutable road-tool inputs from one published simulation generation.
#[derive(Clone)]
pub(crate) struct RoadToolQuerySnapshot {
    /// Persistent curb frames from the same publication as the graph.
    pub(crate) zoning_grid: crate::simulation::zoning::cells::RoadGridSnap,
    pub(crate) terrain: Arc<TerrainSystem>,
    pub(crate) region_graph: Arc<RegionGraph>,
    pub(crate) roadbed: Arc<RoadbedSystem>,
    /// Immutable authored-water state used by road placement validation.
    pub(crate) water: Arc<WaterSystem>,
    /// The published road render chunks of `roadbed`'s generation, which a preview splices
    /// its pieces into (`ROAD-68-E`); shared in `O(1)`.
    pub(crate) road_mesh_chunks: Arc<BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>>,
    /// The roadbed generation `road_mesh_chunks` were published at; the preview splices only
    /// into chunks of its own roadbed generation.
    pub(crate) road_mesh_generation: Option<u64>,
    pub(crate) surface_generation: u64,
}

impl RoadToolQuerySnapshot {
    /// Prepares and validates one stroke against this snapshot: the profile, endpoint and water
    /// checks of the hover path. Returns the prepared points and the validation.
    /// O(stroke points + endpoint snap lookups).
    pub(crate) fn validate(
        &self,
        request: &RoadPreviewRequest,
    ) -> (Vec<Vector3>, RoadPreviewValidation) {
        let fwd_lanes = request.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let bkw_lanes = request.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let prepared = prepare_road_input_for_tool(
            &request.points,
            &self.terrain,
            &self.region_graph,
            &self.roadbed,
            request.snap_to_existing_roads,
        );
        let validation = validate_road_candidate_fast(&prepared, &self.terrain, &self.region_graph);
        let mut validation = validate_road_candidate_against_water(
            prepared.class,
            &prepared.points,
            fwd_lanes,
            bkw_lanes,
            &self.water,
            validation,
        );
        if request.surface_generation != self.surface_generation {
            validation = validation.with_invalid_reason("stale_surface_generation");
        }
        (prepared.points, validation)
    }
}

/// Captures the road-tool query snapshot once the roadbed matches the graph.
pub(crate) fn road_tool_snapshot_from_core(core: &SimCore) -> Option<RoadToolQuerySnapshot> {
    let roadbed = &core.transit_network.roadbed;
    if !roadbed.is_current() {
        return None;
    }
    Some(RoadToolQuerySnapshot {
        zoning_grid: core
            .zoning
            .cells
            .road_grid_snap(f64::from(core.config.zone_cell_m)),
        terrain: Arc::new(core.heightmap.clone()),
        region_graph: Arc::new(core.region_graph.clone()),
        roadbed: Arc::new(core.transit_network.roadbed.clone()),
        water: Arc::new(core.watermap.clone()),
        road_mesh_chunks: Arc::clone(&core.published_road_mesh_chunks),
        road_mesh_generation: core.road_mesh_lane_generation,
        surface_generation: core.road_tool_surface_generation,
    })
}

/// The generations a preview's plan and pieces depend on (`ROAD-68-E`): the road-tool surface,
/// the published roadbed and the source and visual terrain. A commit installs a preview's pieces
/// only when the live core shows the same fingerprint; otherwise it recompiles locally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoadPreviewFingerprint {
    pub(crate) surface_generation: u64,
    pub(crate) roadbed_generation: u64,
    pub(crate) terrain_source_generation: u64,
    pub(crate) terrain_visual_generation: u64,
}

impl RoadPreviewFingerprint {
    /// The fingerprint of one generation's inputs; the one definition preview and commit share.
    pub(crate) fn of(
        surface_generation: u64,
        roadbed: &RoadbedSystem,
        terrain: &TerrainSystem,
    ) -> Self {
        Self {
            surface_generation,
            roadbed_generation: roadbed.generation(),
            terrain_source_generation: terrain.source_generation(),
            terrain_visual_generation: terrain.visual_generation(),
        }
    }

    /// The live core's fingerprint.
    pub(crate) fn live(core: &SimCore) -> Self {
        Self::of(
            core.road_tool_surface_generation,
            &core.transit_network.roadbed,
            &core.heightmap,
        )
    }
}

/// Wall-clock milliseconds of one preview's stages, against `docs/roads.md` "Stage budgets".
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RoadPreviewTimes {
    /// Preparation, validation and the topology and profile solve.
    pub(crate) plan_ms: f64,
    /// Layout and pieces of the dirty closure, with the replica's apply and restore.
    pub(crate) roadbed_ms: f64,
    /// The changed render chunks: the pieces emitted and spliced into the published chunks.
    pub(crate) payload_ms: f64,
}

/// The pieces a preview compiled for its dirty closure.
pub(crate) struct RoadPreviewPieces {
    /// What the commit installs.
    pub(crate) delta: RoadbedDelta,
    /// The context's roadbed with the delta installed: what the preview shows.
    pub(crate) roadbed: RoadbedSystem,
    /// Render chunks the delta changed, in ascending order. The XZ bounds of the replaced and
    /// new pieces wait in `roadbed` for terrain patch selection.
    pub(crate) changed_chunks: Vec<SurfaceChunkKey>,
    /// The changed chunks' meshes as the commit publishes them, without the lane-derived layers
    /// (crosswalk paint, road signs) of the rebuilt pieces; a changed chunk missing here is
    /// empty. Sealed (render-validated) as the commit publishes them. `None` if the published
    /// chunks could not be spliced.
    pub(crate) chunks: Option<BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>>,
}

/// One synchronous road preview: the plan the click submits, the pieces the commit installs and
/// the fingerprint that decides whether it may (`ROAD-68-E`).
pub(crate) struct RoadPreview {
    pub(crate) plan: RoadEditPlan,
    /// The plan's validation with the water check.
    pub(crate) validation: RoadPreviewValidation,
    pub(crate) fingerprint: RoadPreviewFingerprint,
    /// `None` when the stroke is invalid or its plan incomplete.
    pub(crate) pieces: Option<RoadPreviewPieces>,
    pub(crate) times: RoadPreviewTimes,
}

impl RoadPreview {
    /// True when this preview was made for exactly this stroke.
    pub(crate) fn is_for(
        &self,
        points: &[Vector3],
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> bool {
        let request = self.plan.request();
        request.points == points
            && request.fwd_lanes == fwd_lanes
            && request.bkw_lanes == bkw_lanes
            && request.snap_to_existing_roads == snap_to_existing_roads
    }
}

impl RoadPreviewContext {
    /// `preview_road(context, stroke)`: plans the stroke on the context, applies the plan to the
    /// replica, compiles the dirty closure into an `O(1)` copy of the published roadbed and
    /// restores the replica. The plan, marks and compile are the commit's own functions, so a
    /// commit with a current fingerprint installs these pieces bitwise. No core lock; cost
    /// `O(stroke + local excerpt + closure pieces)`, independent of the city.
    pub(crate) fn preview_road(&mut self, request: RoadPreviewRequest) -> RoadPreview {
        let plan_start = Instant::now();
        let fingerprint = RoadPreviewFingerprint::of(
            self.view.surface_generation,
            &self.view.roadbed,
            &self.view.terrain,
        );
        let plan = RoadEditPlan::compile_from(
            &self.view.terrain,
            &self.replica,
            &self.view.roadbed,
            request,
        );
        let validation = {
            let request = plan.request();
            validate_road_candidate_against_water(
                plan.prepared().class,
                &plan.prepared().points,
                request.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                request.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                &self.view.water,
                plan.validation().clone(),
            )
        };
        let plan_ms = plan_start.elapsed().as_secs_f64() * 1000.0;
        let mut times = RoadPreviewTimes {
            plan_ms,
            ..Default::default()
        };
        let pieces = validation
            .is_valid
            .then(|| self.compile_plan_pieces(&plan, &mut times))
            .flatten();
        RoadPreview {
            plan,
            validation,
            fingerprint,
            pieces,
            times,
        }
    }

    // Applies the plan's topology to the replica, compiles what it dirtied, emits the changed
    // chunks on the edited replica and restores it from the same local checkpoint the commit's
    // undo record captures.
    fn compile_plan_pieces(
        &mut self,
        plan: &RoadEditPlan,
        times: &mut RoadPreviewTimes,
    ) -> Option<RoadPreviewPieces> {
        let roadbed_start = Instant::now();
        let topology = plan.topology_for(&self.replica)?;
        let (edges, nodes) = topology.source_topology_ids();
        let checkpoint = self.replica.capture_undo_for_local_topology(edges, nodes);
        let Some(applied) = topology.apply_to_graph(&mut self.replica) else {
            self.replica.restore_undo_delta_local(&checkpoint);
            return None;
        };
        let mut roadbed = self.view.roadbed.snapshot();
        applied.mark_pieces(&mut roadbed);
        let delta = roadbed.compile_delta(&self.replica);
        let (changed_chunks, _) = roadbed.install(&delta);
        times.roadbed_ms = roadbed_start.elapsed().as_secs_f64() * 1000.0;
        let payload_start = Instant::now();
        let target_chunks: BTreeSet<_> = changed_chunks.iter().copied().collect();
        let built_after = self.view.roadbed.generation();
        let chunks = (self.view.road_mesh_generation == Some(built_after))
            .then(|| {
                RoadRenderer.generate_preview_chunks(
                    &self.replica,
                    &self.view.terrain,
                    &roadbed,
                    &target_chunks,
                    built_after,
                    &self.view.road_mesh_chunks,
                )
            })
            .flatten()
            .map(|chunks| {
                chunks
                    .into_iter()
                    .map(|(chunk, mesh)| (chunk, mesh.seal()))
                    .collect()
            });
        times.payload_ms = payload_start.elapsed().as_secs_f64() * 1000.0;
        let restore_start = Instant::now();
        self.replica.restore_undo_delta_local(&checkpoint);
        times.roadbed_ms += restore_start.elapsed().as_secs_f64() * 1000.0;
        Some(RoadPreviewPieces {
            delta,
            roadbed,
            changed_chunks,
            chunks,
        })
    }
}
