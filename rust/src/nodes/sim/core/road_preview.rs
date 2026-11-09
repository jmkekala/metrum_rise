// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool preview requests, the immutable road-tool query snapshot and the synchronous
//! preview (`ROAD-68-E`): the commit's own plan, compiled pieces and their dependency
//! fingerprint, built on the road tool's context without the core lock.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use godot::prelude::Vector3;

use super::graded_stage::GradedPose;
use super::road_edit_plan::{
    CoreLand, PlottedOverlap, RoadEditPlan, RoadLandTake, first_plotted_overlap,
};
use super::road_tool_land::ViewLand;
use super::road_terrain_inputs::RoadTerrainInputs;
use super::road_tool_context::RoadPreviewContext;
use super::state::SimCore;
use crate::nodes::sim::road_tool::validate_road_candidate_against_water;
use crate::simulation::network::graph::{Edge, RegionGraph, RoadMedian, RoadParking};
use crate::simulation::network::render::RoadChunkMap;
use crate::simulation::network::road_types::RoadAttributes;
use crate::simulation::network::render::road::{PreviewChunk, RoadRenderer};
use crate::simulation::network::road_edit::{
    PreparedRoadInput, RoadPreviewValidation, prepare_road_input_for_tool,
    validate_road_candidate_fast,
};
use crate::simulation::network::roadbed::{
    PieceMesh, RoadbedDelta, RoadbedRestore, RoadbedSystem, SurfaceChunkKey,
};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::{GradedSurface, QueryScratch};
use crate::simulation::water::WaterSystem;

/// One road-tool stroke submitted for preview or commit planning.
#[derive(Debug)]
pub(crate) struct RoadPreviewRequest {
    pub(crate) surface_generation: u64,
    pub(crate) points: Vec<Vector3>,
    pub(crate) fwd_lanes: i32,
    pub(crate) bkw_lanes: i32,
    pub(crate) snap_to_existing_roads: bool,
    /// The road tool's tunnel intent (`ROAD-61`): part of the preview's and the plan's identity.
    pub(crate) tunnel: bool,
    /// The road tool's median (`ROAD-80`): part of the preview's and the plan's identity; the
    /// stroke's edge carries it and its corridor checks widen by it.
    pub(crate) median: Option<RoadMedian>,
    /// The road tool's kerbside parking (`PARK-05`), part of the identity like `median`.
    pub(crate) parking: RoadParking,
    /// The road tool's road-type attributes (`ROAD-101`): part of the preview's and the plan's
    /// identity; the stroke's edge and its pieces carry them.
    pub(crate) attributes: RoadAttributes,
}

/// Corridor clearance of a stroke against live parcels and plotted reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoadCorridorClearance {
    /// Parcels the corridor overlaps: informational, since a stroke takes zoned land
    /// (`ZONE-09`, decision D8).
    pub(crate) overlapping_parcels: usize,
    /// Lowest overlapping parcel id, present when any parcel overlaps.
    pub(crate) first_overlapping_parcel: Option<u64>,
    /// Whether the corridor overlaps a committed field reservation (`field_overlap`).
    pub(crate) field_overlap: bool,
    /// Whether the corridor overlaps a mine's extraction area (`extraction_area_overlap`).
    pub(crate) extraction_overlap: bool,
    /// Whether the corridor overlaps the lot of an explicitly placed building
    /// (`plotted_building_overlap`).
    pub(crate) plotted_overlap: bool,
    /// Zoning overlay revision the parcel query read.
    pub(crate) zoning_revision: u64,
    /// Agriculture field revision the field query read.
    pub(crate) field_revision: u64,
}

/// The pieces a stroke's plan installs and those it replaces, as its commit would install them
/// on the road tool's view (`installed`, `replaced`).
pub(crate) type StrokePieces<'a> = (&'a [&'a PieceMesh], &'a [&'a PieceMesh]);

impl RoadCorridorClearance {
    /// Queries the parcel chunk index along the corridor that the lanes, the `median` and the
    /// sidewalks (or a road without them its shoulders, `attributes`) cover, and the plotted
    /// land: with the stroke's `pieces` the commit's own test of them against the live fields,
    /// extraction areas and explicit sites, so a bridge's deck refuses nothing its commit
    /// accepts (`ZONE-09` decisions D2, D8); without (no complete plan on the view) the
    /// corridor's. O(corridor segments × parcels in the touched chunks) plus O(pieces ×
    /// reservations in their bounds), or the corridor's O(segments × polygons and sites in the
    /// touched chunks); every site is a candidate while the site query index is stale.
    pub(crate) fn evaluate(
        core: &SimCore,
        points: &[Vector3],
        (fwd_lanes, bkw_lanes): (i32, i32),
        (median, parking, attributes): (Option<RoadMedian>, RoadParking, RoadAttributes),
        pieces: Option<StrokePieces<'_>>,
    ) -> Self {
        let lanes = |count: i32| count.clamp(0, i32::from(u8::MAX)) as u8;
        let (fwd, bkw) = (lanes(fwd_lanes), lanes(bkw_lanes));
        let half_width = Edge::cross_section_width_m(fwd, bkw, median, parking) * 0.5
            + attributes.corridor_side_m(fwd == 0 && bkw == 0);
        let parcels = core
            .zoning
            .parcel_ids_overlapping_road_corridor(points, half_width);
        let (field_overlap, extraction_overlap, plotted_overlap) = match pieces {
            Some((installed, replaced)) => {
                let plotted = first_plotted_overlap(&CoreLand(core), installed, replaced);
                (
                    plotted == Some(PlottedOverlap::Field),
                    plotted == Some(PlottedOverlap::ExtractionArea),
                    matches!(plotted, Some(PlottedOverlap::Building(_))),
                )
            }
            None => (
                core.allocator
                    .field_clearance
                    .overlaps_road_corridor(points, half_width),
                core.allocator
                    .extraction_clearance
                    .overlaps_road_corridor(points, half_width),
                corridor_overlaps_explicit_site(core, points, half_width),
            ),
        };
        Self {
            overlapping_parcels: parcels.len(),
            first_overlapping_parcel: parcels.first().copied(),
            field_overlap,
            extraction_overlap,
            plotted_overlap,
            zoning_revision: core.zoning.overlay_revision(),
            field_revision: core.agriculture.visual_revision(),
        }
    }
}

// Whether a segment of the corridor of `half_width` along `points` overlaps an explicit site's
// lot.
fn corridor_overlaps_explicit_site(core: &SimCore, points: &[Vector3], half_width: f32) -> bool {
    use crate::simulation::agriculture::PolygonFootprint;
    use godot::prelude::Vector2;
    points.windows(2).any(|pair| {
        let start = Vector2::new(pair[0].x, pair[0].z);
        let end = Vector2::new(pair[1].x, pair[1].z);
        let delta = end - start;
        if delta.length_squared() <= f32::EPSILON {
            return false;
        }
        let normal = Vector2::new(delta.y, -delta.x).normalized() * half_width;
        let quad = [start - normal, end - normal, end + normal, start + normal];
        let corridor = PolygonFootprint::new(&quad);
        core.allocator
            .first_explicit_site_lot(corridor.min, corridor.max, core.config.zone_cell_m, |lot| {
                corridor.overlaps(&PolygonFootprint::new(lot))
            })
            .is_some()
    })
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
    /// Every published road render chunk of `roadbed`'s generation, as Godot shows them once it
    /// installed `road_mesh_surface_generation`: a preview's overlays replace runs of these
    /// (`ROAD-68-H` round 3); shared in `O(1)`.
    pub(crate) road_mesh_chunks: RoadChunkMap,
    /// The roadbed generation `road_mesh_chunks` were published at; the preview builds its
    /// overlays only on chunks of its own roadbed generation.
    pub(crate) road_mesh_generation: Option<u64>,
    /// The road mesh generation Godot records once it installed `road_mesh_chunks`; a preview's
    /// hidden runs index the chunks of exactly this generation.
    pub(crate) road_mesh_surface_generation: u64,
    /// What the road-touched graded terrain reads besides the heightmap, per render patch;
    /// shared in `O(1)` (`ROAD-68-E`).
    pub(crate) terrain_inputs: RoadTerrainInputs,
    /// A persistent copy of the zoning cell store, shared in `O(1)` and brought up to date at
    /// each publication from the live store's write journal (open question 32): the preview
    /// tests its pieces against its reserved cells as the commit does (open question 25).
    pub(crate) cells:
        crate::simulation::zoning::cells::CellStore<crate::simulation::zoning::cells::SharedMaps>,
    pub(crate) surface_generation: u64,
}

impl RoadToolQuerySnapshot {
    /// Runs a query against the preview snapshot's canonical graded surface, reusing one
    /// scratch buffer per road-tool thread. The snapshot has no persistent query index, so the
    /// bounded source gathers read its captured roadbed and site patches directly.
    pub(crate) fn with_graded_surface<T>(
        &self,
        query: impl FnOnce(&GradedSurface<'_>, &mut QueryScratch) -> T,
    ) -> T {
        thread_local! {
            static SCRATCH: RefCell<QueryScratch> = RefCell::default();
        }
        let sites = super::graded_stage::PreviewSites::new(&self.terrain, &self.terrain_inputs);
        let surface = GradedSurface {
            terrain: &self.terrain,
            roadbed: &self.roadbed,
            sites: &sites,
            query_index: None,
        };
        SCRATCH.with_borrow_mut(|scratch| query(&surface, scratch))
    }

    /// Prepares and validates one stroke against this snapshot: the profile, endpoint and water
    /// checks of the hover path. Returns the prepared input (its points and class: the stroke's
    /// cost) and the validation. O(stroke points + endpoint snap lookups).
    pub(crate) fn validate(
        &self,
        request: &RoadPreviewRequest,
    ) -> (PreparedRoadInput, RoadPreviewValidation) {
        let fwd_lanes = request.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let bkw_lanes = request.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let mut prepared = prepare_road_input_for_tool(
            &request.points,
            &self.terrain,
            &self.region_graph,
            &self.roadbed,
            request.snap_to_existing_roads,
            request.tunnel,
        );
        prepared.median = request.median;
        prepared.attributes = request.attributes;
        let validation = validate_road_candidate_fast(
            &prepared,
            (fwd_lanes, bkw_lanes),
            &self.terrain,
            &self.region_graph,
        );
        let mut validation = validate_road_candidate_against_water(
            &prepared,
            fwd_lanes,
            bkw_lanes,
            &self.water,
            validation,
        );
        if request.surface_generation != self.surface_generation {
            validation = validation.with_invalid_reason("stale_surface_generation");
        }
        (prepared, validation)
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
        road_mesh_chunks: core.cached_road_mesh_chunks.clone(),
        road_mesh_generation: core.road_mesh_lane_generation,
        road_mesh_surface_generation: core.cached_road_mesh_generation,
        terrain_inputs: core.road_terrain_inputs.current(),
        cells: crate::simulation::zoning::cells::CellStore::copy_of(&core.zoning.cells),
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
    /// The graded tiles of the pose's frame (`TERRAIN-07-E`, T27 (d)): the dirty tile
    /// selection and the frame's share of the pose's tiles, gathered and adopted or baked.
    pub(crate) graded_ms: f64,
}

/// The pieces a preview compiled for its dirty closure.
pub(crate) struct RoadPreviewPieces {
    /// What the commit installs.
    pub(crate) delta: RoadbedDelta,
    /// The context's roadbed with the delta installed: what the preview shows.
    pub(crate) roadbed: RoadbedSystem,
    /// Render chunks the delta changed, in ascending order.
    pub(crate) changed_chunks: Vec<SurfaceChunkKey>,
    /// The pose's staged graded tiles (`TERRAIN-07-E`): the commit adopts those whose inputs
    /// equal its own.
    pub(crate) graded: GradedPose,
    /// The changed chunks below the chunk (`ROAD-68-H` round 3): each one's overlay of rebuilt
    /// pieces and the published runs it replaces, which compose the chunk the commit publishes
    /// from these pieces (its lights and signs aside); a changed chunk missing here changes
    /// nothing Godot shows. `None` if the view's chunks are not of its roadbed's generation or
    /// a published run has no owner.
    pub(crate) chunks: Option<BTreeMap<SurfaceChunkKey, PreviewChunk>>,
    /// The land the delta's pieces take (`ZONE-09`), by the commit's own rules against the
    /// view's cells and the road tool's land: a plotted reservation refuses the stroke, else the
    /// lots it removes and the buildings (by handle) it demolishes.
    pub(crate) take: RoadLandTake<u64>,
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn is_for(
        &self,
        points: &[Vector3],
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
        tunnel: bool,
        (median, parking, attributes): (Option<RoadMedian>, RoadParking, RoadAttributes),
    ) -> bool {
        let request = self.plan.request();
        request.points == points
            && request.fwd_lanes == fwd_lanes
            && request.bkw_lanes == bkw_lanes
            && request.snap_to_existing_roads == snap_to_existing_roads
            && request.tunnel == tunnel
            && request.median == median
            && request.parking == parking
            && request.attributes == attributes
    }
}

impl RoadPreviewContext {
    /// `preview_road(context, stroke)`: plans the stroke on the context, applies the plan to the
    /// replica, compiles the dirty closure into an `O(1)` copy of the published roadbed, stages
    /// the frame's share of its graded tiles and restores the replica. The plan, marks and
    /// compile are the commit's own functions, so a commit with a current fingerprint installs
    /// these pieces bitwise. No core lock; cost
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
                plan.prepared(),
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
        let plotted = pieces.as_ref().and_then(|pieces| pieces.take.plotted);
        if pieces.is_none() || plotted.is_some() {
            // A pose that draws no road (no pieces, or invalid) supersedes the last pose's
            // unstaged graded tiles and shows no preview terrain (T60 (2)).
            self.graded.supersede();
        }
        // The commit would refuse plotted land; the preview shows it before the click.
        let validation = match plotted {
            Some(plotted) => validation.with_invalid_reason(plotted.reason()),
            None => validation,
        };
        RoadPreview {
            plan,
            validation,
            fingerprint,
            pieces,
            times,
        }
    }

    /// One frame without a new pose (T27 (d)): stages the next share of the latest pose's
    /// graded tiles and returns the tiles shown after it; the latest preview then offers them
    /// to its commit, unless a commit already holds it. `None` once the shown tiles are the
    /// pose's. No core lock; `O(share's gathers and bakes + wanted log wanted)`.
    pub(crate) fn catch_up_graded(&mut self) -> Option<GradedPose> {
        let pose = self.graded.catch_up()?;
        if let Some(pieces) = self
            .last_preview
            .as_mut()
            .and_then(Arc::get_mut)
            .and_then(|preview| preview.pieces.as_mut())
        {
            pieces.graded = pose.clone();
        }
        Some(pose)
    }

    /// The pieces `plan` installs and those they replace on the view, compiled on the replica
    /// as a preview compiles them, handed to `check`; `None` without a complete topology. The
    /// one-shot stroke checks judge plotted land on them (`ZONE-09` decision D8). No graded or
    /// render work; `O(stroke + local excerpt + closure pieces)`.
    pub(crate) fn with_plan_pieces<T>(
        &mut self,
        plan: &RoadEditPlan,
        check: impl FnOnce(StrokePieces<'_>) -> T,
    ) -> Option<T> {
        let compiled = self.compile_on_replica(plan)?;
        let before: Vec<_> = compiled.restore.replaced_pieces().collect();
        let installed: Vec<_> = compiled
            .roadbed
            .installed_pieces(&compiled.restore)
            .collect();
        let result = check((&installed, &before));
        self.replica.restore_undo_delta_local(&compiled.checkpoint);
        Some(result)
    }

    // Applies the plan's topology to the replica and compiles what it dirtied into an `O(1)`
    // copy of the view's roadbed with the delta installed. The caller restores the replica from
    // the returned checkpoint, the one the commit's undo record captures.
    fn compile_on_replica(&mut self, plan: &RoadEditPlan) -> Option<CompiledPose> {
        let topology = plan.topology_for(&self.replica)?;
        let (edges, nodes) = topology.source_topology_ids();
        let checkpoint = self.replica.capture_undo_for_local_topology(edges, nodes);
        let Some(applied) = topology.apply_to_graph(&mut self.replica) else {
            self.replica.restore_undo_delta_local(&checkpoint);
            return None;
        };
        let mut roadbed = self.view.roadbed.snapshot();
        applied.mark_pieces(&mut roadbed);
        let delta = roadbed.compile_delta_with(&self.replica, &mut self.roadbed_scratch);
        let (changed_chunks, restore) = roadbed.install(&delta);
        Some(CompiledPose {
            checkpoint,
            delta,
            roadbed,
            changed_chunks,
            restore,
        })
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
        let CompiledPose {
            checkpoint,
            delta,
            mut roadbed,
            changed_chunks,
            restore,
        } = self.compile_on_replica(plan)?;
        // The commit's land take on the same installed and replaced pieces.
        let before: Vec<_> = restore.replaced_pieces().collect();
        let installed: Vec<_> = roadbed.installed_pieces(&restore).collect();
        let take = ViewLand {
            land: &self.land,
            cells: &self.view.cells,
        }
        .take(&installed, &before);
        times.roadbed_ms = roadbed_start.elapsed().as_secs_f64() * 1000.0;
        let terrain_changes = roadbed.take_terrain_changes();
        let graded_start = Instant::now();
        let end = plan
            .request()
            .points
            .last()
            .map_or([0.0; 2], |end| [f64::from(end.x), f64::from(end.z)]);
        // Every owner the delta covers, a failed node's too: the pieces this pose can differ in.
        let owners = delta.pieces().map(|(id, _)| id).chain(
            delta
                .failed_nodes()
                .map(|(node, _)| crate::simulation::network::roadbed::PieceId::Node(node)),
        );
        let graded = self.graded.stage_pose(
            &self.view.terrain,
            &self.view.roadbed,
            &roadbed,
            &self.view.terrain_inputs,
            &terrain_changes,
            owners,
            end,
        );
        times.graded_ms = graded_start.elapsed().as_secs_f64() * 1000.0;
        let payload_start = Instant::now();
        let target_chunks: BTreeSet<_> = changed_chunks.iter().copied().collect();
        let built_after = self.view.roadbed.generation();
        let chunks = (self.view.road_mesh_generation == Some(built_after))
            .then(|| {
                let sites = super::graded_stage::PreviewSites::new(
                    &self.view.terrain,
                    &self.view.terrain_inputs,
                );
                // Every owner the delta covers, with whether it built a piece: a failed node's
                // published piece goes too.
                let closure: Vec<_> = delta
                    .pieces()
                    .map(|(id, mesh)| (id, mesh.is_some()))
                    .chain(
                        delta
                            .failed_nodes()
                            .map(|(node, _)| (crate::simulation::network::roadbed::PieceId::Node(node), false)),
                    )
                    .collect();
                RoadRenderer.generate_preview_overlay(
                    &self.replica,
                    &self.view.terrain,
                    &sites,
                    &roadbed,
                    &target_chunks,
                    &closure,
                    built_after,
                    &self.view.road_mesh_chunks,
                )
            })
            .flatten();
        times.payload_ms = payload_start.elapsed().as_secs_f64() * 1000.0;
        let restore_start = Instant::now();
        self.replica.restore_undo_delta_local(&checkpoint);
        times.roadbed_ms += restore_start.elapsed().as_secs_f64() * 1000.0;
        Some(RoadPreviewPieces {
            delta,
            roadbed,
            changed_chunks,
            graded,
            chunks,
            take,
        })
    }
}

// One pose compiled on the replica: the replica's checkpoint, the delta, the view's roadbed
// with it installed, the changed chunks and the install's restore.
struct CompiledPose {
    checkpoint: crate::simulation::network::graph::RegionGraphUndoDelta,
    delta: RoadbedDelta,
    roadbed: RoadbedSystem,
    changed_chunks: Vec<SurfaceChunkKey>,
    restore: RoadbedRestore,
}
