// SPDX-License-Identifier: GPL-2.0-only

//! Logic for modifying simulation state (road placement, terrain sculpt, zoning, edge editing).

use crate::config;
use crate::debug_log;
use crate::nodes::sim::core::{
    ROAD_BUILD_COST_PER_METER, RoadEditPlan, RoadRejection, ReservedOverlap,
    SERVICE_BUILD_COST_PER_LOT_CELL, SimCore,
};
use crate::nodes::sim::benchmark::road_edit::RoadEditMetrics;
use crate::nodes::sim::road_tool::validate_road_candidate_against_water;
use crate::nodes::simulation_node::vegetation_api::{self, CANOPY_CLEAR_RADIUS_M};
use crate::simulation::buildings::allocator::{
    ExplicitServicePlacementPreview, ExplicitServicePlacementRejection,
};
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::graph::RegionGraphUndoDelta;
use crate::simulation::network::road_edit::{
    FinalizedRoadGeometry, plan_road_edit,
    validate_road_candidate_fast, validate_road_edit,
};
use crate::simulation::network::roadbed::{
    RoadbedDelta, RoadbedRestore, visual_roadbed_half_width_m,
};
use crate::simulation::vegetation::edits::pack_patch_key;
use crate::traffic_log;
use godot::prelude::*;
use std::collections::HashSet;
use std::time::Instant;

const BULLDOZE_HIGHLIGHT_Y_OFFSET_M: f32 = 0.08;
const BULLDOZE_ROAD_PICK_RADIUS_M: f32 = 24.0;
const BULLDOZE_ROAD_PICK_MARGIN_M: f32 = 0.75;
const ROAD_UNDO_TOPOLOGY_MARGIN_M: f32 = 40.0;

/// Axis-aligned world bounds of one player polygon, or `None` for fewer than three points.
pub(crate) fn polygon_world_bounds(points: &[Vector2]) -> Option<(Vector2, Vector2)> {
    if points.len() < 3 {
        return None;
    }
    Some(points.iter().fold((points[0], points[0]), |(lo, hi), p| {
        (
            Vector2::new(lo.x.min(p.x), lo.y.min(p.y)),
            Vector2::new(hi.x.max(p.x), hi.y.max(p.y)),
        )
    }))
}

/// Result of a road placement attempt after synchronous input validation.
#[derive(Clone, Debug)]
pub(crate) struct RoadAddOutcome {
    /// True when topology was mutated and the command must run road finalization.
    pub(crate) committed: bool,
    /// Deferred treasury charge for the committed physical road length.
    pub(crate) build_cost: f64,
    /// Already adopted local geometry; absent when authoritative preparation must finalize it.
    pub(crate) finalized_geometry: Option<FinalizedRoadGeometry>,
    /// Wall-clock milliseconds of the undo record, the topology mutation (graph, parcels and
    /// adopted plan) and the cell-lot marks, for the commit breakdown (`ROAD-68-G`).
    pub(crate) stage_ms: RoadAddStageTimes,
}

/// Wall-clock cost of the stages of one accepted road addition.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RoadAddStageTimes {
    pub(crate) undo_ms: f64,
    pub(crate) topology_ms: f64,
    pub(crate) zoning_ms: f64,
}

impl RoadAddOutcome {
    fn rejected() -> Self {
        Self {
            committed: false,
            build_cost: 0.0,
            finalized_geometry: None,
            stage_ms: RoadAddStageTimes::default(),
        }
    }
}

impl SimCore {
    /// Accepts a road only after its roadbed builds, restoring the staged checkpoint otherwise.
    #[cfg(test)]
    pub(crate) fn validate_staged_road_render(&mut self) -> bool {
        self.validate_staged_road_render_installing(None, None, &mut Default::default())
    }

    /// As [`Self::validate_staged_road_render`], installing a preview's pieces when they still
    /// apply (`ROAD-68-E`). The installed pieces must not overlap fields or painted cells
    /// (`ROAD-68` open question 1), and every node piece must build. No terrain failure rejects
    /// the road (`TERRAIN-07-E`): an accepted edit stages its graded tiles, adopting the
    /// preview's `graded` tiles whose inputs equal the committed ones. The undo record keeps
    /// what the install replaced. True when the edit is accepted. `metrics` receives the
    /// sub-stages' times (`surface_roadbed_ms`, `surface_overlap_ms`, `surface_graded_ms`).
    pub(crate) fn validate_staged_road_render_installing(
        &mut self,
        delta: Option<&RoadbedDelta>,
        graded: Option<&crate::nodes::sim::core::GradedPose>,
        metrics: &mut RoadEditMetrics,
    ) -> bool {
        let roadbed_start = Instant::now();
        // The graded selection reads the roadbeds before and after the install (`O(1)`).
        let roadbed_before = self.transit_network.roadbed.snapshot();
        let (_, restore, _) = self.publish_network_surface(false, delta);
        metrics.surface_roadbed_ms = roadbed_start.elapsed().as_secs_f64() * 1000.0;
        let overlap_start = Instant::now();
        let roadbed = &self.transit_network.roadbed;
        let before: Vec<_> = restore.replaced_pieces().collect();
        let overlap =
            ReservedOverlap::first_in_core(self, roadbed.installed_pieces(&restore), &before);
        metrics.surface_overlap_ms = overlap_start.elapsed().as_secs_f64() * 1000.0;
        if let Some(overlap) = overlap {
            self.rollback_staged_road_edit(Some(&restore));
            let rejection = overlap.rejection();
            self.last_road_timing = format!("rejected={}", rejection.reason);
            debug_log!(
                "road",
                "road_commit_rejected reason={} rollback=true",
                rejection.reason
            );
            self.last_road_rejection = Some(rejection);
            return false;
        }
        if let Some(&node) = roadbed.failed_nodes().first() {
            let reason = format!(
                "node_piece_failed node={node} error={:?}",
                roadbed.node_error(node)
            );
            self.rollback_staged_road_edit(Some(&restore));
            self.last_road_timing = format!("rejected=render_geometry_invalid {reason}");
            self.last_road_rejection = Some(RoadRejection::new(
                "render_geometry_invalid",
                format!(
                    "the road surface failed to build after the edit ({reason}); the edit was \
                     rolled back"
                ),
            ));
            debug_log!(
                "road",
                "road_commit_rejected reason=render_geometry_invalid detail={} rollback=true",
                reason
            );
            return false;
        }
        let graded_start = Instant::now();
        let graded = self.stage_committed_graded_tiles(roadbed_before, graded);
        metrics.surface_graded_ms = graded_start.elapsed().as_secs_f64() * 1000.0;
        let stats = self.last_graded_stats;
        metrics.graded_select_ms = stats.select_ms;
        metrics.graded_bake_ms = stats.bake_ms;
        metrics.graded_adopted = stats.adopted;
        metrics.graded_baked = stats.baked;
        if self.transit_network.road_edit_is_staged()
            && let Some(record) = self.undo_stack.back_mut()
        {
            record.roadbed = Some(restore);
            record.graded = graded;
        }
        self.accept_staged_road_edit();
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BulldozeTargetKind {
    Building,
    Road,
    Vegetation,
}

impl BulldozeTargetKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::Road => "road",
            Self::Vegetation => "vegetation",
        }
    }
}

/// One resolved object and its immutable hover geometry for a queued bulldoze command.
#[derive(Clone, Debug)]
pub(crate) struct BulldozeTarget {
    kind: BulldozeTargetKind,
    vegetation: Option<vegetation_api::PlantTarget>,
    id: usize,
    center: Vector3,
    points: Vec<Vector3>,
    width_m: f32,
}

fn point_segment_distance_sq_xz(pos: Vector2, a: Vector3, b: Vector3) -> (f32, Vector3) {
    let dx = b.x - a.x;
    let dz = b.z - a.z;
    let len_sq = dx * dx + dz * dz;
    let t = if len_sq > f32::EPSILON {
        (((pos.x - a.x) * dx + (pos.y - a.z) * dz) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest = Vector3::new(a.x + dx * t, a.y + (b.y - a.y) * t, a.z + dz * t);
    let off_x = pos.x - closest.x;
    let off_z = pos.y - closest.z;
    (off_x * off_x + off_z * off_z, closest)
}

impl SimCore {
    fn road_commit_full_validation_debug_enabled() -> bool {
        std::env::var("METRUM_DEBUG_ROAD_COMMIT_FULL_VALIDATION")
            .map(|value| !value.is_empty() && value != "0")
            .unwrap_or(false)
    }

    fn begin_terrain_stroke_internal(&mut self) {
        self.terrain_stroke_active = true;
        self.terrain_stroke_has_changes = false;
    }

    fn prepare_batched_terrain_edit_internal(&mut self) {
        if !self.terrain_stroke_active {
            self.begin_terrain_stroke_internal();
        }
        if !self.terrain_stroke_has_changes {
            self.push_undo_state(true, false, true);
            self.terrain_stroke_has_changes = true;
        }
    }

    fn mark_terrain_authoring_payload_bounds(&mut self, pos: Vector2, radius: f32) {
        let radius = radius.max(0.0) + self.heightmap.render_patch_border_margin_m();
        self.record_graded_base_edit(
            [pos.x - radius, pos.y - radius, pos.x + radius, pos.y + radius].map(f64::from),
        );
        self.zoning
            .mark_cell_lots_dirty(crate::simulation::zoning::cells::CellBounds {
                min: glam::DVec2::new(f64::from(pos.x - radius), f64::from(pos.y - radius)),
                max: glam::DVec2::new(f64::from(pos.x + radius), f64::from(pos.y + radius)),
            });
        let patch_keys = self.heightmap.render_patch_keys_for_world_bounds(
            pos.x - radius,
            pos.y - radius,
            pos.x + radius,
            pos.y + radius,
        );
        for &(patch_x, patch_z) in &patch_keys {
            self.heightmap.mark_render_patch_dirty(patch_x, patch_z);
        }
        self.bump_terrain_payload_patch_generations(&patch_keys);
        // Road grading rays probe up to the envelope cap beyond loops taken up to the cap
        // (the tile halo) beyond their tile, so new heights move the road margins of patches
        // within twice the cap; the stroke's end refreshes them (`ROAD-68-D`).
        let reach_m =
            radius + 2.0 * crate::simulation::terrain::grading::GRADING_ENVELOPE_MAX_MARGIN_M;
        self.road_ownership_pending_patch_keys.extend(
            self.heightmap.render_patch_keys_for_world_bounds(
                pos.x - reach_m,
                pos.y - reach_m,
                pos.x + reach_m,
                pos.y + reach_m,
            ),
        );
        self.terrain_dirty = true;
    }

    fn finish_terrain_authoring_edit_internal(&mut self) {
        // Godot's main thread finishes terrain edits; the rebuilds are parallel (CODE-17).
        crate::thread_pools::install(|| self.finish_terrain_authoring_edit_in_pool());
    }

    fn finish_terrain_authoring_edit_in_pool(&mut self) {
        self.terrain_dirty = true;
        self.mark_local_network_render_dirty();

        // The sync re-grades every road; an edit that leaves one steeper than 1:3, and steeper
        // than before, is refused below (`roads.md` question 37, `ROAD-68-K15` (1)). O(edges),
        // like the sync.
        let roads: HashSet<usize> = (0..self.region_graph.edge_count())
            .filter(|&edge| {
                let edge = self.region_graph.edge(edge);
                !edge.deleted
                    && edge.primary_type == crate::simulation::network::types::TransitType::Road
            })
            .collect();
        let grades_before: std::collections::HashMap<usize, f32> = roads
            .iter()
            .map(|&edge| (edge, self.edge_grade(edge)))
            .collect();
        // The sync rebuilds every lane and renumbers them.
        let lane_identities = self
            .agents
            .capture_lane_identities(&self.transit_network.lane_system);
        self.transit_network
            .sync_to_terrain(&mut self.region_graph, &self.heightmap);
        self.agents
            .restore_lane_identities(&lane_identities, &self.transit_network.lane_system);
        // Terrain edits never move placed roads; only the graph sync above re-grades them.
        self.transit_network
            .roadbed
            .compile_dirty(&self.region_graph);
        // An edit that leaves a node without a layout or profile is rolled back from the undo
        // record it pushed before it began, before anything publishes (`ROAD-68` open question
        // 11 (c)).
        if let Some(&node) = self.transit_network.roadbed.failed_nodes().first() {
            let error = self.transit_network.roadbed.node_error(node);
            debug_log!(
                "world-editor",
                "terrain_edit_rolled_back node={} error={:?}",
                node,
                error
            );
            self.last_road_rejection = Some(RoadRejection::new(
                "node_piece_failed",
                format!(
                    "the terrain edit leaves node {node} without a layout or profile: {error:?}"
                ),
            ));
            self.undo_action_in_pool();
            return;
        }
        // Rolled back the same way from the undo record, which restores the source terrain
        // the sync and the lots read (`TERRAIN-07-E`).
        if let Some(rejection) = self.steepened_profile(&roads, &grades_before) {
            debug_log!(
                "world-editor",
                "terrain_edit_rolled_back reason={} {}",
                rejection.reason,
                rejection.detail
            );
            self.last_road_rejection = Some(rejection);
            self.undo_action_in_pool();
            return;
        }
        self.refresh_road_locked_terrain_patch_state(
            crate::nodes::sim::core::ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
        );
        // Terrain-authoring APIs publish their snapshot immediately rather than passing through the
        // simulation command finalizer. Build the matching road delta here so NetworkRenderer does
        // not repeatedly observe a newer source generation paired with the previous mesh.
        self.precompute_road_mesh_data();
        self.rebuild_building_entrances_internal();
        if self.has_authored_water_internal()
            && let Err(err) = self.rebuild_authored_water_preview_internal()
        {
            debug_log!(
                "world-editor",
                "rebuild_authored_water_after_sculpt failed: {}",
                err
            );
        }
    }

    /// Begins one batched terrain brush stroke.
    pub fn start_terrain_stroke_internal(&mut self) {
        if !self.terrain_stroke_active {
            self.begin_terrain_stroke_internal();
        }
    }

    /// Finalizes one batched terrain brush stroke and runs deferred rebuild work once.
    pub fn end_terrain_stroke_internal(&mut self) -> bool {
        if !self.terrain_stroke_active {
            return false;
        }
        self.terrain_stroke_active = false;
        let had_changes = self.terrain_stroke_has_changes;
        self.terrain_stroke_has_changes = false;
        if had_changes {
            self.finish_terrain_authoring_edit_internal();
        }
        had_changes
    }

    pub(crate) fn rebuild_building_entrances_internal(&mut self) {
        // Godot's main thread runs this after its network edits; the lot refresh, attachment
        // repair and entrance rebuild are parallel: shared pool (CODE-17).
        crate::thread_pools::install(|| {
            // Refresh queued frontage links before copying parcel attachments into live
            // buildings. The cell refresh defers all publication while a road transaction is
            // still staged.
            self.prepare_cell_lots_internal();
            self.allocator
                .repair_road_attachments_after_topology_edit(&self.region_graph, &mut self.zoning);
            self.allocator
                .rebuild_entrance_cache(&self.region_graph, &self.transit_network.lane_system);
        });
    }

    fn run_building_allocator_maintenance_internal(&mut self) {
        // Main-thread edits run maintenance, whose removals reach parallel rebuilds (CODE-17).
        crate::thread_pools::install(|| {
            self.allocator.maintain(
                0,
                &mut self.zoning,
                &mut self.agents,
                &mut self.households,
                &mut self.logistics,
                &mut self.treasury.balance,
                &mut self.transit_network,
                &mut self.region_graph,
            );
            self.publish_pending_production_site_removals();
            self.mark_changed_zone_flow_fields();
        });
    }

    /// Returns the deterministic building, road or vegetation target for the bulldoze cursor.
    pub(crate) fn get_bulldoze_target_at_internal(
        &mut self,
        world_x: f32,
        world_z: f32,
    ) -> VarDictionary {
        self.prepare_bulldoze_target_indices();
        self.resolve_bulldoze_target(world_x, world_z)
            .map(|target| self.bulldoze_target_dict(&target, false))
            .unwrap_or_else(Self::empty_bulldoze_target_dict)
    }

    /// Resolves and captures one immutable bulldoze command for background execution.
    pub(crate) fn prepare_bulldoze_command_internal(
        &mut self,
        world_x: f32,
        world_z: f32,
    ) -> Option<(BulldozeTarget, VarDictionary)> {
        self.prepare_bulldoze_target_indices();
        let target = self.resolve_bulldoze_target(world_x, world_z)?;
        let payload = self.bulldoze_target_dict(&target, false);
        Some((target, payload))
    }

    /// Executes a previously resolved target if it still identifies the same object.
    ///
    /// Returns whether the deleted object was a road, or `None` when validation/deletion failed.
    pub(crate) fn bulldoze_prepared_target_internal(
        &mut self,
        target: BulldozeTarget,
    ) -> Option<bool> {
        // Index repair and the deletion's rebuilds are parallel: shared pool (CODE-17).
        crate::thread_pools::install(|| {
            self.prepare_bulldoze_target_indices();
            let current = self.resolve_bulldoze_target(target.center.x, target.center.z)?;
            if current.kind != target.kind
                || current.id != target.id
                || current.vegetation != target.vegetation
            {
                return None;
            }
            let road_deleted = current.kind == BulldozeTargetKind::Road;
            self.delete_bulldoze_target(&current)
                .then_some(road_deleted)
        })
    }

    fn delete_bulldoze_target(&mut self, target: &BulldozeTarget) -> bool {
        match target.kind {
            BulldozeTargetKind::Building => self.bulldoze_building(target.id),
            BulldozeTargetKind::Road => self.bulldoze_road_edge(target.id),
            BulldozeTargetKind::Vegetation => target
                .vegetation
                .is_some_and(|plant| vegetation_api::remove_target(self, plant)),
        }
    }

    fn prepare_bulldoze_target_indices(&mut self) {
        self.allocator
            .prepare_building_site_query_index(self.zoning.config.zone_cell_m);
    }

    fn resolve_bulldoze_target(&self, world_x: f32, world_z: f32) -> Option<BulldozeTarget> {
        let pos = Vector2::new(world_x, world_z);
        self.resolve_bulldoze_building_target(pos)
            .or_else(|| self.resolve_bulldoze_road_target(pos))
            .or_else(|| self.resolve_bulldoze_vegetation_target(pos))
    }

    fn resolve_bulldoze_vegetation_target(&self, pos: Vector2) -> Option<BulldozeTarget> {
        let (selected, radius) = vegetation_api::plant_at(self, pos)?;
        let plant = selected.plant;
        // Pack the stored f32 position bits, never the compacted authored-vector index.
        // The center retains those exact coordinates, so resolving there selects this
        // zero-distance plant and reproduces its ID after unrelated authored removals.
        let id = ((u64::from(plant.x.to_bits()) << 32) | u64::from(plant.z.to_bits())) as usize;
        let height = |x, z| {
            self.heightmap.sample_visual_height_world(x, z) * config::HEIGHT_SCALE
                + BULLDOZE_HIGHLIGHT_Y_OFFSET_M
        };
        let points = (0..16)
            .map(|i| {
                let angle = std::f32::consts::TAU * i as f32 / 16.0;
                let x = plant.x + radius * angle.cos();
                let z = plant.z + radius * angle.sin();
                Vector3::new(x, height(x, z), z)
            })
            .collect();
        Some(BulldozeTarget {
            kind: BulldozeTargetKind::Vegetation,
            vegetation: Some(selected),
            id,
            center: Vector3::new(plant.x, height(plant.x, plant.z), plant.z),
            points,
            width_m: 0.0,
        })
    }

    fn resolve_bulldoze_building_target(&self, pos: Vector2) -> Option<BulldozeTarget> {
        let candidates = self
            .allocator
            .site_candidate_indices_for_bounds(pos.x, pos.y, pos.x, pos.y);
        let mut best: Option<(usize, f32, Vector2)> = None;
        for idx in candidates {
            let Some(site) = self.allocator.building_sites.get(idx) else {
                continue;
            };
            if !site.contains_point(pos) {
                continue;
            }
            let center = site
                .footprint_world
                .iter()
                .copied()
                .fold(Vector2::ZERO, |acc, point| acc + point)
                / site.footprint_world.len().max(1) as f32;
            let dist_sq = (center.x - pos.x).mul_add(center.x - pos.x, (center.y - pos.y).powi(2));
            let replace = best
                .map(|(best_idx, best_dist_sq, _)| {
                    dist_sq < best_dist_sq - f32::EPSILON
                        || ((dist_sq - best_dist_sq).abs() <= f32::EPSILON && idx < best_idx)
                })
                .unwrap_or(true);
            if replace {
                best = Some((idx, dist_sq, center));
            }
        }

        let (building_idx, _, center) = best?;
        let site = self.allocator.building_sites.get(building_idx)?;
        let mut points = Vec::with_capacity(site.footprint_world.len());
        for point in &site.footprint_world {
            points.push(Vector3::new(
                point.x,
                site.support_height_m + BULLDOZE_HIGHLIGHT_Y_OFFSET_M,
                point.y,
            ));
        }
        Some(BulldozeTarget {
            kind: BulldozeTargetKind::Building,
            vegetation: None,
            id: building_idx,
            center: Vector3::new(
                center.x,
                site.support_height_m + BULLDOZE_HIGHLIGHT_Y_OFFSET_M,
                center.y,
            ),
            points,
            width_m: 0.0,
        })
    }

    fn resolve_bulldoze_road_target(&self, pos: Vector2) -> Option<BulldozeTarget> {
        let query_pos = Vector3::new(pos.x, 0.0, pos.y);
        let mut best: Option<(usize, f32, Vector3)> = None;
        // The R-tree's order is unspecified, so the candidates ascend by edge id and the strict
        // comparison below picks by exact distance, then the lowest edge id (`ROAD-68` open
        // question 37).
        let mut candidates = self
            .region_graph
            .get_edges_near_point(query_pos, BULLDOZE_ROAD_PICK_RADIUS_M);
        candidates.sort_unstable();
        for edge_idx in candidates {
            if edge_idx >= self.region_graph.edge_count() {
                continue;
            }
            let edge = self.region_graph.edge(edge_idx);
            if edge.deleted || edge.physical_geometry.len() < 2 {
                continue;
            }
            let sidewalk_width = if edge.allowed_types
                & crate::simulation::network::types::TransitFlags::FOOT
                != 0
            {
                config::SIDEWALK_WIDTH
            } else {
                0.0
            };
            let half_width = edge.width.max(config::LANE_WIDTH) * 0.5 + sidewalk_width;
            let threshold_sq = (half_width + BULLDOZE_ROAD_PICK_MARGIN_M).powi(2);
            for pair in edge.physical_geometry.windows(2) {
                let (dist_sq, closest) = point_segment_distance_sq_xz(pos, pair[0], pair[1]);
                if dist_sq > threshold_sq {
                    continue;
                }
                if best.is_none_or(|(_, best_dist_sq, _)| dist_sq < best_dist_sq) {
                    best = Some((edge_idx, dist_sq, closest));
                }
            }
        }

        let (edge_idx, _, closest) = best?;
        let edge = self.region_graph.edge(edge_idx);
        let sidewalk_width =
            if edge.allowed_types & crate::simulation::network::types::TransitFlags::FOOT != 0 {
                config::SIDEWALK_WIDTH
            } else {
                0.0
            };
        let width_m = edge.width.max(config::LANE_WIDTH) + sidewalk_width * 2.0;
        let mut points = Vec::with_capacity(edge.physical_geometry.len());
        for point in &edge.physical_geometry {
            let y = self
                .transit_network
                .roadbed
                .height_at(point.x, point.z)
                .unwrap_or(point.y);
            points.push(Vector3::new(
                point.x,
                y + BULLDOZE_HIGHLIGHT_Y_OFFSET_M,
                point.z,
            ));
        }
        let center_y = self
            .transit_network
            .roadbed
            .height_at(closest.x, closest.z)
            .unwrap_or(closest.y);
        Some(BulldozeTarget {
            kind: BulldozeTargetKind::Road,
            vegetation: None,
            id: edge_idx,
            center: Vector3::new(
                closest.x,
                center_y + BULLDOZE_HIGHLIGHT_Y_OFFSET_M,
                closest.z,
            ),
            points,
            width_m,
        })
    }

    /// Removes one building with an undo entry, as the bulldoze tool does.
    pub(crate) fn bulldoze_building(&mut self, building_idx: usize) -> bool {
        self.remove_building_for_edit(building_idx, true)
    }

    fn remove_building_for_edit(&mut self, building_idx: usize, record_undo: bool) -> bool {
        if building_idx >= self.allocator.buildings.len() {
            return false;
        }
        // The undo capture and the removal's remaps are parallel; Godot's main thread cancels
        // pending industry outside any pool (CODE-17).
        crate::thread_pools::install(|| self.remove_building_in_pool(building_idx, record_undo))
    }

    fn remove_building_in_pool(&mut self, building_idx: usize, record_undo: bool) -> bool {
        let dirty_bounds = self.allocator.site_dirty_bounds(building_idx);
        // Read while the farm still owns its field: removing it gives the ground back, and the
        // plants the field was hiding have to return with it.
        let field_bounds = self
            .agriculture
            .site_for_building(building_idx)
            .and_then(|site| polygon_world_bounds(&site.polygon_world));
        self.allocator
            .accumulate_pending_site_dirty_bounds(dirty_bounds);
        if record_undo && !self.push_building_removal_undo(building_idx) {
            return false;
        }
        let _ = self.allocator.take_pending_site_dirty_bounds();
        self.agents.evacuate_building_for_removal(
            building_idx,
            &mut self.households,
            &mut self.allocator,
            &self.transit_network,
            &self.region_graph,
        );
        let treasury_before = self.treasury.balance;
        let removed = self.allocator.remove_building_for_bulldoze(
            building_idx,
            &mut self.zoning,
            &mut self.agents,
            &mut self.households,
            &mut self.logistics,
            &mut self.treasury.balance,
        );
        if !removed {
            if record_undo {
                self.undo_stack.pop_back();
            }
            return false;
        }
        self.publish_pending_production_site_removals();
        crate::nodes::simulation_node::vegetation_api::publish_yards(self);
        if let Some(bounds) = field_bounds {
            self.invalidate_vegetation_over(bounds);
        }
        if let Some(bounds) = dirty_bounds {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
        self.rebuild_building_entrances_internal();
        if record_undo {
            self.seal_building_removal_undo(self.treasury.balance - treasury_before);
        }
        self.transit_network.flow_fields.mark_all_dirty();
        self.terrain_dirty = true;
        true
    }

    /// Test driver: bulldozes road edge `edge_idx` as the bulldoze tool does.
    #[cfg(test)]
    pub(crate) fn bulldoze_road_edge_for_test(&mut self, edge_idx: usize) -> bool {
        self.bulldoze_road_edge(edge_idx)
    }

    fn bulldoze_road_edge(&mut self, edge_idx: usize) -> bool {
        if edge_idx >= self.region_graph.edge_count() || self.region_graph.edge(edge_idx).deleted {
            return false;
        }

        let edge = self.region_graph.edge(edge_idx).clone();
        let edge_points = edge.physical_geometry.clone();
        let old_chunks = self.region_graph.get_edge_chunks(edge_idx);
        let start_node = self.region_graph.get_valid_node(edge.start_node);
        let end_node = self.region_graph.get_valid_node(edge.end_node);
        let mut affected_nodes = HashSet::from([start_node, end_node]);
        let mut affected_edges = HashSet::from([edge_idx]);
        for node_id in [start_node, end_node] {
            if node_id as usize >= self.region_graph.node_adjacency_count() {
                continue;
            }
            for &adj_edge_idx in self.region_graph.node_adjacency(node_id) {
                affected_edges.insert(adj_edge_idx);
                let adj_edge = self.region_graph.edge(adj_edge_idx);
                affected_nodes.insert(self.region_graph.get_valid_node(adj_edge.start_node));
                affected_nodes.insert(self.region_graph.get_valid_node(adj_edge.end_node));
            }
        }

        // A removal that leaves a node at either end without a layout or profile is refused,
        // with nothing touched (`ROAD-68` open question 11 (c)).
        let ends = HashSet::from([start_node, end_node]);
        let tried = self.try_graph_edit(&affected_edges, &affected_nodes, |core| {
            let graph = &mut core.region_graph;
            graph.delete_edge_locally(edge_idx);
            graph.rebuild_intersection_clips_for_nodes(&affected_nodes);
            graph.finalize_junction_endpoint_profiles(
                &ends,
                Some(&core.heightmap),
                &mut HashSet::new(),
            );
            HashSet::new()
        });
        if let Err(rejection) = tried {
            debug_log!(
                "road",
                "road_removal_rejected edge={} reason={} detail={}",
                edge_idx,
                rejection.reason,
                rejection.detail
            );
            self.last_road_rejection = Some(rejection);
            return false;
        }

        self.push_road_removal_undo(affected_edges.clone(), affected_nodes.clone(), edge_idx);
        self.zoning
            .mark_cell_lots_for_roads(&self.region_graph, affected_edges.iter().copied());
        self.transit_network
            .mark_surface_dirty_from_sets(&affected_edges, &affected_nodes);
        for point in &edge_points {
            self.transit_network.mark_point_dirty(*point);
        }

        self.region_graph.remove_from_spatial_index(edge_idx);
        self.region_graph.edge_mut(edge_idx).deleted = true;
        self.zoning.remove_parcels_attached_to_edge(edge_idx);
        self.region_graph.rebuild_adjacency_list();
        self.region_graph
            .rebuild_intersection_clips_for_nodes(&affected_nodes);
        // The arms left at both ends re-derive their profiles (`ROAD-68` invariant 6); every
        // edge the solve derives touches one of them, so it is in the undo record and rebuilt.
        let mut changed = HashSet::new();
        self.region_graph.finalize_junction_endpoint_profiles(
            &ends,
            Some(&self.heightmap),
            &mut changed,
        );
        debug_assert!(changed.is_subset(&affected_edges));
        for chunk in old_chunks {
            self.transit_network.cch_dirty_chunks.insert(chunk);
        }
        for &adj_edge_idx in &affected_edges {
            if adj_edge_idx >= self.region_graph.edge_count()
                || self.region_graph.edge(adj_edge_idx).deleted
            {
                continue;
            }
            for chunk in self.region_graph.get_edge_chunks(adj_edge_idx) {
                self.transit_network.cch_dirty_chunks.insert(chunk);
            }
        }

        self.agents.invalidate_lane_ids_for_edges(
            &affected_edges,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        // Rebuild the same closure we invalidated; a full rebuild renumbers distant live lanes.
        self.transit_network
            .lane_system
            .rebuild_edges_incremental(&mut self.region_graph, &affected_edges);
        self.agents.reattach_invalidated_lanes_for_edges(
            &affected_edges,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        self.rebuild_building_entrances_internal();
        // Deferred like a commit's (D7, `ROAD-68-G`); the road tool's view shares its graph
        // copy (`ROAD-68-E`).
        let routing_graph = self.schedule_cch_rebuild_of_live_graph();
        self.road_tool_publisher.offer_graph(routing_graph);
        self.transit_network.flow_fields.mark_all_dirty();
        self.mark_local_network_render_dirty();
        self.terrain_dirty = true;
        true
    }

    fn empty_bulldoze_target_dict() -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("valid", false);
        dict.set("deleted", false);
        dict.set("kind", GString::new());
        dict
    }

    fn bulldoze_target_dict(&self, target: &BulldozeTarget, deleted: bool) -> VarDictionary {
        let mut points = PackedVector3Array::new();
        for point in &target.points {
            points.push(*point);
        }
        let mut dict = VarDictionary::new();
        dict.set("valid", true);
        dict.set("deleted", deleted);
        dict.set("kind", GString::from(target.kind.as_str()));
        dict.set("id", target.id as i64);
        dict.set("center", target.center);
        dict.set("points", points);
        dict.set("width_m", f64::from(target.width_m));
        dict
    }

    /// Sculpts the terrain with a given radius and strength.
    pub fn sculpt_terrain_internal(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.push_undo_state(true, false, true);
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap
            .sculpt(center_x, center_y, radius_cells, strength);
        self.mark_terrain_authoring_payload_bounds(pos, radius);
        self.finish_terrain_authoring_edit_internal();
    }

    /// Applies one batched terrain sculpt step without running deferred rebuild work yet.
    pub fn sculpt_terrain_stroke_step_internal(
        &mut self,
        pos: Vector2,
        radius: f32,
        strength: f32,
    ) {
        self.prepare_batched_terrain_edit_internal();
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap
            .sculpt(center_x, center_y, radius_cells, strength);
        self.mark_terrain_authoring_payload_bounds(pos, radius);
    }

    /// Moves terrain toward one target rendered height in a circular area.
    pub fn level_terrain_internal(
        &mut self,
        pos: Vector2,
        radius: f32,
        target_height_m: f32,
        strength: f32,
    ) {
        self.push_undo_state(true, false, true);
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap.level_to_height(
            center_x,
            center_y,
            radius_cells,
            target_height_m / config::HEIGHT_SCALE,
            strength,
        );
        self.mark_terrain_authoring_payload_bounds(pos, radius);
        self.finish_terrain_authoring_edit_internal();
    }

    /// Smooths terrain toward the local neighborhood average in a circular area.
    pub fn smooth_terrain_internal(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.push_undo_state(true, false, true);
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap
            .smooth(center_x, center_y, radius_cells, strength);
        self.mark_terrain_authoring_payload_bounds(pos, radius);
        self.finish_terrain_authoring_edit_internal();
    }

    /// Moves terrain toward a slope defined by two clicked world-space anchor points.
    #[allow(clippy::too_many_arguments)]
    pub fn slope_terrain_internal(
        &mut self,
        pos: Vector2,
        radius: f32,
        start_world: Vector2,
        start_height_m: f32,
        end_world: Vector2,
        end_height_m: f32,
        strength: f32,
    ) {
        self.push_undo_state(true, false, true);
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let (start_x, start_y) = self
            .heightmap
            .world_to_grid_coords(start_world.x, start_world.y);
        let (end_x, end_y) = self
            .heightmap
            .world_to_grid_coords(end_world.x, end_world.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap.slope_to_segment(
            center_x,
            center_y,
            radius_cells,
            start_x,
            start_y,
            start_height_m / config::HEIGHT_SCALE,
            end_x,
            end_y,
            end_height_m / config::HEIGHT_SCALE,
            strength,
        );
        self.mark_terrain_authoring_payload_bounds(pos, radius);
        self.finish_terrain_authoring_edit_internal();
    }

    /// Applies one batched terrain-level step without running deferred rebuild work yet.
    pub fn level_terrain_stroke_step_internal(
        &mut self,
        pos: Vector2,
        radius: f32,
        target_height_m: f32,
        strength: f32,
    ) {
        self.prepare_batched_terrain_edit_internal();
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap.level_to_height(
            center_x,
            center_y,
            radius_cells,
            target_height_m / config::HEIGHT_SCALE,
            strength,
        );
        self.mark_terrain_authoring_payload_bounds(pos, radius);
    }

    /// Applies one batched terrain-smooth step without running deferred rebuild work yet.
    pub fn smooth_terrain_stroke_step_internal(
        &mut self,
        pos: Vector2,
        radius: f32,
        strength: f32,
    ) {
        self.prepare_batched_terrain_edit_internal();
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap
            .smooth(center_x, center_y, radius_cells, strength);
        self.mark_terrain_authoring_payload_bounds(pos, radius);
    }

    /// Applies one batched terrain-slope step without running deferred rebuild work yet.
    #[allow(clippy::too_many_arguments)]
    pub fn slope_terrain_stroke_step_internal(
        &mut self,
        pos: Vector2,
        radius: f32,
        start_world: Vector2,
        start_height_m: f32,
        end_world: Vector2,
        end_height_m: f32,
        strength: f32,
    ) {
        self.prepare_batched_terrain_edit_internal();
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let (start_x, start_y) = self
            .heightmap
            .world_to_grid_coords(start_world.x, start_world.y);
        let (end_x, end_y) = self
            .heightmap
            .world_to_grid_coords(end_world.x, end_world.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap.slope_to_segment(
            center_x,
            center_y,
            radius_cells,
            start_x,
            start_y,
            start_height_m / config::HEIGHT_SCALE,
            end_x,
            end_y,
            end_height_m / config::HEIGHT_SCALE,
            strength,
        );
        self.mark_terrain_authoring_payload_bounds(pos, radius);
    }

    /// Sets or clears the no-building-spawn flag on an edge.
    pub fn set_no_building_spawn_internal(&mut self, edge_idx: i32, enabled: bool) {
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edge_count() {
            return;
        }
        let edge_idx = edge_idx as usize;
        self.region_graph.edge_mut(edge_idx).no_building_spawn = enabled;
        self.zoning
            .mark_cell_lots_for_roads(&self.region_graph, [edge_idx]);
        if enabled {
            self.run_building_allocator_maintenance_internal();
            self.zoning.remove_parcels_attached_to_edge(edge_idx);
        }
        self.allocator.dirty = true;
        self.rebuild_building_entrances_internal();
    }

    /// Sets the frontage-access policy of an edge by integer enum code.
    pub fn set_vehicle_frontage_access_internal(&mut self, edge_idx: i32, access_int: u8) {
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edge_count() {
            return;
        }

        let access = match access_int {
            0 => crate::simulation::network::types::VehicleFrontageAccess::SameSideOnly,
            1 => crate::simulation::network::types::VehicleFrontageAccess::BothSides,
            _ => return,
        };

        self.region_graph
            .edge_mut(edge_idx as usize)
            .vehicle_frontage_access = access;
        self.allocator.dirty = true;
        self.rebuild_building_entrances_internal();
    }

    /// Sets the classification of an edge by integer class code.
    pub fn set_edge_class_internal(&mut self, edge_idx: i32, class_int: u8) {
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edge_count() {
            return;
        }
        let edge_idx = edge_idx as usize;

        let class = match class_int {
            1 => crate::simulation::network::types::EdgeClass::Bridge,
            2 => crate::simulation::network::types::EdgeClass::Tunnel,
            _ => crate::simulation::network::types::EdgeClass::Standard,
        };
        if class == crate::simulation::network::types::EdgeClass::Standard {
            let edge = self.region_graph.edge(edge_idx);
            let points = if edge.physical_geometry.len() >= 2 {
                edge.physical_geometry.as_slice()
            } else {
                edge.geometry.as_slice()
            };
            let half_width_m = visual_roadbed_half_width_m(edge);
            if self
                .watermap
                .road_corridor_overlaps_visible_water(points, half_width_m)
            {
                debug_log!(
                    "road",
                    "edge_class_change_rejected edge={} reason=water_requires_bridge",
                    edge_idx
                );
                return;
            }
        }

        let edge = self.region_graph.edge(edge_idx);
        let affected_nodes = HashSet::from([edge.start_node, edge.end_node]);
        // A class whose pieces would not compile at either end, or would overlap fields or
        // painted cells, is refused (`ROAD-68` open question 11 (c)).
        let incident = affected_nodes
            .iter()
            .flat_map(|&node| {
                let node = self.region_graph.get_valid_node(node);
                self.region_graph.node_adjacency(node).iter().copied()
            })
            .filter(|&edge| !self.region_graph.edge(edge).deleted)
            .chain([edge_idx])
            .collect();
        let tried = self.try_graph_edit(&incident, &affected_nodes, |core| {
            core.region_graph.edge_mut(edge_idx).class = class;
            HashSet::new()
        });
        if let Err(rejection) = tried {
            debug_log!(
                "road",
                "edge_class_change_rejected edge={} reason={}",
                edge_idx,
                rejection.reason
            );
            return;
        }
        self.region_graph.edge_mut(edge_idx).class = class;
        self.zoning
            .mark_cell_lots_for_roads(&self.region_graph, [edge_idx]);
        self.transit_network
            .mark_surface_dirty_for_nodes(&self.region_graph, &affected_nodes);

        self.rebuild_building_entrances_internal();
        // Deferred like a commit's (D7, `ROAD-68-G`); it supersedes a pending one.
        self.schedule_cch_rebuild_of_live_graph();
    }

    /// Adds a new road segment to the transit network.
    pub(crate) fn add_road_internal(
        &mut self,
        points: Vec<godot::prelude::Vector3>,
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> RoadAddOutcome {
        self.add_road_internal_with_snap(points, fwd_lanes, bkw_lanes, true)
    }

    /// Adds a new road segment to the transit network with optional existing-road snapping.
    pub(crate) fn add_road_internal_with_snap(
        &mut self,
        points: Vec<godot::prelude::Vector3>,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> RoadAddOutcome {
        self.add_road_internal_with_snap_and_validation(
            points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads,
            None,
        )
    }

    /// Adds a road while reusing an exact preview only when its authoritative inputs still match.
    pub(crate) fn add_road_internal_with_snap_and_validation(
        &mut self,
        points: Vec<godot::prelude::Vector3>,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
        edit_plan: Option<&RoadEditPlan>,
    ) -> RoadAddOutcome {
        // Non-preview callers must also validate final junction geometry, not only the stroke.
        let fallback_plan = if edit_plan.is_none()
            && (!self.allocator.field_clearance.is_empty() || self.zoning.cells.has_reservations())
        {
            self.allocator
                .prepare_building_site_query_index(self.config.zone_cell_m);
            Some(RoadEditPlan::compile(
                self,
                crate::nodes::sim::core::RoadPreviewRequest {
                    surface_generation: self.road_tool_surface_generation,
                    points: points.clone(),
                    fwd_lanes,
                    bkw_lanes,
                    snap_to_existing_roads,
                },
            ))
        } else {
            None
        };
        let edit_plan = edit_plan.or(fallback_plan.as_ref());
        if let Some(plan) = edit_plan
            && (plan.status(self) != "ready"
                || plan
                    .prepared_input_for(
                        self.road_tool_surface_generation,
                        self.heightmap.source_generation(),
                        &points,
                        fwd_lanes,
                        bkw_lanes,
                        snap_to_existing_roads,
                    )
                    .is_none())
        {
            self.last_road_timing = format!("rejected=road_plan_{}", plan.status(self));
            self.last_road_rejection = Some(plan.rejection(self));
            return RoadAddOutcome::rejected();
        }
        let fwd_lanes_u8 = fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let bkw_lanes_u8 = bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let planned_input = edit_plan.and_then(|plan| {
            plan.prepared_input_for(
                self.road_tool_surface_generation,
                self.heightmap.source_generation(),
                &points,
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
            )
        });
        // Borrow the exact ready solve. Interactive stale/missing plans are rebuilt by the
        // command handler before mutation; plan-less subsystem callers retain full validation.
        let fresh_input;
        let prepared_input = if let Some(prepared) = planned_input {
            prepared
        } else {
            fresh_input = plan_road_edit(
                &points,
                fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                &self.heightmap,
                &self.region_graph,
                &self.transit_network.roadbed,
                snap_to_existing_roads,
            )
            .0;
            &fresh_input
        };
        let fixed_points = prepared_input.points.clone();
        debug_log!(
            "road",
            "road_edit_plan reused={} raw_points={} prepared_points={} generation={}",
            planned_input.is_some(),
            points.len(),
            fixed_points.len(),
            self.road_tool_surface_generation
        );

        let fast_validation =
            validate_road_candidate_fast(prepared_input, &self.heightmap, &self.region_graph);
        let fast_validation = validate_road_candidate_against_water(
            prepared_input.class,
            &prepared_input.points,
            fwd_lanes_u8,
            bkw_lanes_u8,
            &self.watermap,
            fast_validation,
        );
        let mut validation = fast_validation.clone();
        if validation.is_valid {
            if let Some(plan) = edit_plan.filter(|_| planned_input.is_some()) {
                validation = plan.validation().clone();
            } else {
                let full_validation_start = Instant::now();
                let (full_validation, _) = validate_road_edit(
                    prepared_input,
                    fwd_lanes_u8,
                    bkw_lanes_u8,
                    &self.heightmap,
                    &self.region_graph,
                );
                let full_validation = validate_road_candidate_against_water(
                    prepared_input.class,
                    &prepared_input.points,
                    fwd_lanes_u8,
                    bkw_lanes_u8,
                    &self.watermap,
                    full_validation,
                );
                let full_validation_ms = full_validation_start.elapsed().as_secs_f64() * 1000.0;
                if full_validation.is_valid != fast_validation.is_valid
                    || full_validation.invalid_reason != fast_validation.invalid_reason
                {
                    debug_log!(
                        "road",
                        "road_commit_validation_contract_mismatch prepared_points={} fwd_lanes={} bkw_lanes={} fast_valid={} fast_reason={} full_valid={} full_reason={} fast_endpoint_snap=({},{}) full_endpoint_snap=({},{}) full_validation_ms={:.3}",
                        fixed_points.len(),
                        fwd_lanes_u8,
                        bkw_lanes_u8,
                        fast_validation.is_valid,
                        fast_validation.invalid_reason,
                        full_validation.is_valid,
                        full_validation.invalid_reason,
                        fast_validation.start_endpoint_snapped_node_id,
                        fast_validation.end_endpoint_snapped_node_id,
                        full_validation.start_endpoint_snapped_node_id,
                        full_validation.end_endpoint_snapped_node_id,
                        full_validation_ms
                    );
                } else if Self::road_commit_full_validation_debug_enabled() {
                    debug_log!(
                        "road",
                        "road_commit_full_validation_debug prepared_points={} fwd_lanes={} bkw_lanes={} valid={} reason={} full_validation_ms={:.3}",
                        fixed_points.len(),
                        fwd_lanes_u8,
                        bkw_lanes_u8,
                        full_validation.is_valid,
                        full_validation.invalid_reason,
                        full_validation_ms
                    );
                }
                validation = full_validation;
            }
        }
        if !validation.is_valid {
            debug_log!(
                "road",
                "road_commit_rejected reason={} prepared_points={} max_grade={:.3} allowed_grade={:.3} span=({:.3},{:.3}) run={:.3} dy={:.3} span_y=({:.3},{:.3}) span_terrain=({:.3},{:.3}) span_delta=({:.3},{:.3}) endpoint_snap=({},{}) endpoint_delta=({:.3},{:.3}) clearance={:.3} required_clearance={:.3}",
                validation.invalid_reason,
                fixed_points.len(),
                validation.max_grade,
                validation.allowed_grade,
                validation.offending_span_start_m,
                validation.offending_span_end_m,
                validation.offending_span_run_m,
                validation.offending_span_height_delta_m,
                validation.offending_span_start_height_m,
                validation.offending_span_end_height_m,
                validation.offending_span_start_terrain_height_m,
                validation.offending_span_end_terrain_height_m,
                validation.offending_span_start_support_delta_m,
                validation.offending_span_end_support_delta_m,
                validation.start_endpoint_snapped_node_id,
                validation.end_endpoint_snapped_node_id,
                validation.start_endpoint_support_delta_m,
                validation.end_endpoint_support_delta_m,
                validation.clearance_m,
                validation.required_clearance_m
            );
            self.last_road_timing = format!(
                "rejected={} max_grade={:.3} allowed_grade={:.3} span=({:.3},{:.3}) endpoint_delta=({:.3},{:.3})",
                validation.invalid_reason,
                validation.max_grade,
                validation.allowed_grade,
                validation.offending_span_start_m,
                validation.offending_span_end_m,
                validation.start_endpoint_support_delta_m,
                validation.end_endpoint_support_delta_m
            );
            self.last_road_rejection = Some(RoadRejection::from_validation(
                validation.invalid_reason,
                &validation,
                &fixed_points,
            ));
            return RoadAddOutcome::rejected();
        }

        // Fields and painted cells are tested against the pieces the edit installs, in
        // `validate_staged_road_render_installing` (`ROAD-68` open question 1).
        let road_width_m = (f32::from(fwd_lanes_u8) + f32::from(bkw_lanes_u8)) * config::LANE_WIDTH;
        let corridor_half_width_m = road_width_m.max(2.0) * 0.5 + config::SIDEWALK_WIDTH;
        let overlapping_parcels = self
            .zoning
            .parcel_ids_overlapping_road_corridor(&fixed_points, corridor_half_width_m);
        if !overlapping_parcels.is_empty() {
            debug_log!(
                "road",
                "road_commit_rejected reason=parcel_overlap parcels={} first_parcel={}",
                overlapping_parcels.len(),
                overlapping_parcels[0]
            );
            self.last_road_timing = format!(
                "rejected=parcel_overlap parcels={}",
                overlapping_parcels.len()
            );
            self.last_road_rejection = Some(
                RoadRejection::from_validation("parcel_overlap", &validation, &fixed_points)
                    .with_note(format!(
                        "{} parcels, first {}",
                        overlapping_parcels.len(),
                        overlapping_parcels[0]
                    )),
            );
            return RoadAddOutcome::rejected();
        }

        let t_undo = Instant::now();
        let topology_plan = edit_plan
            .filter(|_| planned_input.is_some())
            .and_then(|plan| plan.topology_for(&self.region_graph));
        if !self.benchmark_mode || self.transit_network.road_edit_is_staged() {
            if let Some(plan) = topology_plan {
                let (edges, nodes) = plan.source_topology_ids();
                self.push_network_undo_for_local_topology(edges, nodes);
            } else {
                self.push_network_undo_for_polyline(&fixed_points, ROAD_UNDO_TOPOLOGY_MARGIN_M);
            }
        }
        let dt_undo_ms = t_undo.elapsed().as_micros();

        // Compute polyline length before fixed_points is moved into add_road.
        let build_length_m: f64 = fixed_points
            .windows(2)
            .map(|w| {
                let dx = (w[1].x - w[0].x) as f64;
                let dy = (w[1].y - w[0].y) as f64;
                let dz = (w[1].z - w[0].z) as f64;
                (dx * dx + dy * dy + dz * dz).sqrt()
            })
            .sum();

        let t_topo = Instant::now();
        // A bulk load keeps every edge it has dirtied until `finalize_bulk_load`; hold those
        // aside so this road's cell lots are marked from its own edges, not the whole load
        // (`ROAD-52`). An adopted plan clears the set itself.
        let earlier_bulk_dirty_edges = if topology_plan.is_none() {
            std::mem::take(&mut self.transit_network.bulk_dirty_edges)
        } else {
            HashSet::new()
        };
        let finalized_geometry = if let Some(plan) = topology_plan {
            Some(
                self.transit_network
                    .adopt_road_topology_plan(
                        &mut self.region_graph,
                        plan,
                        &self.zoning,
                        &mut self.allocator,
                    )
                    .expect("plan source identities checked under the same simulation lock"),
            )
        } else {
            self.transit_network.add_road(
                &mut self.region_graph,
                fixed_points,
                fwd_lanes_u8,
                bkw_lanes_u8,
                prepared_input.class,
                &mut self.zoning,
                &mut self.allocator,
            );
            None
        };
        debug_log!(
            "road",
            "road_edit_topology adopted={}",
            finalized_geometry.is_some()
        );
        let dt_topo_us = t_topo.elapsed().as_micros();

        let t_zoning = Instant::now();
        let dirty = finalized_geometry
            .as_ref()
            .map(|geometry| &geometry.dirty_edges)
            .unwrap_or(&self.transit_network.bulk_dirty_edges);
        // An adopted plan names the dirty edges whose geometry changed (`ROAD-68-G`); the
        // others' cells change only near those, so they queue no lot regions of their own.
        match finalized_geometry
            .as_ref()
            .and_then(|geometry| geometry.changed_edges.as_ref())
        {
            Some(changed) => self.zoning.mark_cell_lots_for_road_edit(
                &self.region_graph,
                &dirty.iter().copied().collect(),
                changed,
            ),
            None => self
                .zoning
                .mark_cell_lots_for_roads(&self.region_graph, dirty.iter().copied()),
        }
        let road_dirty_edges = std::mem::replace(
            &mut self.transit_network.bulk_dirty_edges,
            earlier_bulk_dirty_edges,
        );
        self.transit_network
            .bulk_dirty_edges
            .extend(road_dirty_edges);
        let zoning_ms = t_zoning.elapsed().as_secs_f64() * 1000.0;

        self.mark_local_network_render_dirty();

        // The AddRoad handler validates staged road/terrain products before refreshing lanes,
        // agents, routing and building references, for both adopted and freshly solved edits.
        self.last_road_timing = format!("undo={}µs topo={}µs", dt_undo_ms, dt_topo_us);
        RoadAddOutcome {
            committed: true,
            build_cost: build_length_m * ROAD_BUILD_COST_PER_METER,
            finalized_geometry,
            stage_ms: RoadAddStageTimes {
                undo_ms: dt_undo_ms as f64 / 1000.0,
                topology_ms: dt_topo_us as f64 / 1000.0,
                zoning_ms,
            },
        }
    }

    /// Returns a road-frontage snapped preview for one explicit service building asset.
    pub(crate) fn get_service_building_placement_preview_internal(
        &self,
        asset_id: &str,
        world_x: f32,
        world_z: f32,
    ) -> Result<ExplicitServicePlacementPreview, ExplicitServicePlacementRejection> {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        self.allocator.preview_explicit_service_placement(
            asset_id,
            Vector2::new(world_x, world_z),
            &self.zoning,
            &self.region_graph,
            &self.transit_network.roadbed,
            &self.heightmap,
            &catalog,
        )
    }

    /// Places one explicit service building, charging its build cost to the city treasury.
    pub(crate) fn place_service_building_internal(
        &mut self,
        asset_id: &str,
        world_x: f32,
        world_z: f32,
    ) -> Result<usize, ExplicitServicePlacementRejection> {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let building_idx = self.allocator.execute_explicit_service_placement(
            asset_id,
            Vector2::new(world_x, world_z),
            &self.zoning,
            &self.region_graph,
            &self.transit_network.roadbed,
            &self.heightmap,
            &catalog,
            &tuning,
        )?;

        self.register_service_input_capacity(building_idx);

        if !self.benchmark_mode
            && let Some(building) = self.allocator.buildings.get(building_idx)
        {
            let lot_cells = f64::from(building.width_cells) * f64::from(building.depth_cells);
            self.treasury
                .deduct_build_cost(lot_cells * SERVICE_BUILD_COST_PER_LOT_CELL);
        }
        self.rebuild_building_entrances_internal();
        self.publish_pending_building_site_changes();
        Ok(building_idx)
    }

    /// Returns a road-frontage snapped preview for one explicit industry building asset.
    pub(crate) fn get_industry_building_placement_preview_internal(
        &self,
        asset_id: &str,
        world_x: f32,
        world_z: f32,
    ) -> Result<ExplicitServicePlacementPreview, ExplicitServicePlacementRejection> {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        self.allocator.preview_explicit_industry_placement(
            asset_id,
            Vector2::new(world_x, world_z),
            &self.zoning,
            &self.region_graph,
            &self.transit_network.roadbed,
            &self.heightmap,
            &catalog,
        )
    }

    /// Places one explicit industry building, charging its build cost to the city treasury.
    pub(crate) fn place_industry_building_internal(
        &mut self,
        asset_id: &str,
        world_x: f32,
        world_z: f32,
    ) -> Result<usize, ExplicitServicePlacementRejection> {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let building_idx = self.allocator.execute_explicit_industry_placement(
            asset_id,
            Vector2::new(world_x, world_z),
            &self.zoning,
            &self.region_graph,
            &self.transit_network.roadbed,
            &self.heightmap,
            &catalog,
            &tuning,
        )?;

        if !self.benchmark_mode
            && let Some(building) = self.allocator.buildings.get(building_idx)
        {
            let lot_cells = f64::from(building.width_cells) * f64::from(building.depth_cells);
            self.treasury
                .deduct_build_cost(lot_cells * SERVICE_BUILD_COST_PER_LOT_CELL);
        }
        self.rebuild_building_entrances_internal();
        self.publish_pending_building_site_changes();
        Ok(building_idx)
    }

    /// Commits or replaces a resource-extraction polygon for one placed industry building.
    pub(crate) fn commit_extractor_polygon_internal(
        &mut self,
        building_idx: usize,
        polygon_world: Vec<Vector2>,
    ) -> Result<crate::simulation::extraction::ExtractorSiteSummary, String> {
        let previous_area = self
            .resource_extraction
            .site_for_building(building_idx)
            .map_or(0.0, |site| site.area_m2);
        let summary = self.resource_extraction.commit_site(
            building_idx,
            polygon_world,
            &self.resource_deposits,
            &mut self.allocator,
        )?;
        self.charge_industry_area_expansion(building_idx, previous_area, summary.area_m2);
        Ok(summary)
    }

    /// Quotes the extra construction charge and all-import daily utilities for a valid area.
    /// The footprint is paid separately; reshaping/shrinking an existing area gives no refund.
    pub(crate) fn industry_area_quote_internal(
        &self,
        building_idx: usize,
        area_m2: f32,
    ) -> Result<(f64, f32), String> {
        if !area_m2.is_finite() || area_m2 < 0.0 {
            return Err("production area must be finite and nonnegative".to_owned());
        }
        let building = self
            .allocator
            .buildings
            .get(building_idx)
            .ok_or_else(|| "industry building does not exist".to_owned())?;
        let catalog = self.demand.runtime_catalog();
        let profile = catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .filter(|p| crate::simulation::work_area::profile_kind_uses_explicit_work_area(p.kind))
            .ok_or_else(|| "building does not use a production area".to_owned())?;
        profile.validate_field_area(area_m2)?;
        let previous_area = self
            .agriculture
            .site_for_building(building_idx)
            .map(|site| site.area_m2)
            .or_else(|| {
                self.resource_extraction
                    .site_for_building(building_idx)
                    .map(|site| site.area_m2)
            })
            .unwrap_or(0.0);
        let tuning = load_runtime_economy_tuning().expect("runtime tuning loaded");
        let utilities = profile
            .utility_demand_units_per_day(area_m2 / 10_000.0)
            .into_iter()
            .zip(crate::simulation::economy::households::local_utility_service_costs(catalog))
            .map(|(units, price)| units * price * tuning.owa_import_price_multiplier)
            .sum();
        Ok((
            profile.area_expansion_cost(previous_area, area_m2),
            utilities,
        ))
    }

    fn charge_industry_area_expansion(
        &mut self,
        building_idx: usize,
        previous_area: f32,
        area: f32,
    ) {
        if self.benchmark_mode {
            return;
        }
        let profile = self
            .demand
            .runtime_catalog()
            .profile_by_runtime_id(
                self.allocator.buildings[building_idx].economy_profile_runtime_id,
            )
            .expect("committed area has a resolved economy profile");
        self.treasury
            .deduct_build_cost(profile.area_expansion_cost(previous_area, area));
    }

    fn prepare_field_polygon_validation(&mut self) -> Result<(), String> {
        if !self.transit_network.roadbed.is_current() {
            return Err("road surfaces are updating; try again shortly".to_owned());
        }
        self.allocator
            .prepare_building_site_query_index(self.zoning.config.zone_cell_m);
        Ok(())
    }

    /// Validates a field preview without changing its saved polygon or work area.
    pub(crate) fn validate_field_polygon_internal(
        &mut self,
        building_idx: usize,
        polygon_world: &[Vector2],
    ) -> Result<f32, String> {
        self.prepare_field_polygon_validation()?;
        self.agriculture.validate_site(
            building_idx,
            polygon_world,
            &self.allocator,
            &self.zoning,
            &self.transit_network.roadbed,
        )
    }

    /// Replaces only the field and farm identity captured when the drag began.
    pub(crate) fn resize_field_polygon_internal(
        &mut self,
        building_idx: usize,
        expected_center: Vector2,
        expected_polygon: &[Vector2],
        polygon_world: Vec<Vector2>,
    ) -> Result<crate::simulation::agriculture::FieldSiteSummary, String> {
        let same_building = self
            .allocator
            .buildings
            .get(building_idx)
            .is_some_and(|building| {
                Vector2::new(building.center_x, building.center_y) == expected_center
            });
        let same_field = self
            .agriculture
            .site_for_building(building_idx)
            .is_some_and(|site| site.polygon_world == expected_polygon);
        if !same_building || !same_field {
            return Err("the farm or field changed; reopen Edit Field".to_owned());
        }
        self.commit_field_polygon_internal(building_idx, polygon_world)
    }

    /// Commits a validated field and immediately rebuilds its area and placement reservation.
    pub(crate) fn commit_field_polygon_internal(
        &mut self,
        building_idx: usize,
        polygon_world: Vec<Vector2>,
    ) -> Result<crate::simulation::agriculture::FieldSiteSummary, String> {
        self.prepare_field_polygon_validation()?;
        let previous_area = self
            .agriculture
            .site_for_building(building_idx)
            .map_or(0.0, |site| site.area_m2);
        // Captured before the commit: a resize has to give back the ground the old polygon
        // covered as well as take the ground the new one covers.
        let previous = self
            .agriculture
            .site_for_building(building_idx)
            .and_then(|site| polygon_world_bounds(&site.polygon_world));
        let committed = polygon_world_bounds(&polygon_world);
        let summary = self.agriculture.commit_site(
            building_idx,
            polygon_world,
            &mut self.allocator,
            &self.zoning,
            &self.transit_network.roadbed,
        )?;
        self.charge_industry_area_expansion(building_idx, previous_area, summary.area_m2);
        for bounds in [previous, committed].into_iter().flatten() {
            self.zoning.mark_cell_lots_dirty(
                crate::simulation::zoning::cells::CellBounds::from_points(
                    [bounds.0, bounds.1].map(|p| glam::DVec2::new(f64::from(p.x), f64::from(p.y))),
                ),
            );
            self.invalidate_vegetation_over(bounds);
        }
        Ok(summary)
    }

    /// Restales every vegetation patch a world-space box touches, so plants a field now covers
    /// disappear on the next patch fetch rather than on the next load.
    ///
    /// A field hides its plants through the same `placement_clear` predicate a road deck or a
    /// building pad uses. Those two already restale their patches through the terrain surface
    /// generation, because both move ground; a field only paints it, so it advances the
    /// vegetation revision itself. One increment per covered patch, evaluating no plants.
    pub(crate) fn invalidate_vegetation_over(&mut self, bounds: (Vector2, Vector2)) {
        let (min, max) = bounds;
        let keys = self.heightmap.render_patch_keys_for_world_bounds(
            min.x - CANOPY_CLEAR_RADIUS_M,
            min.y - CANOPY_CLEAR_RADIUS_M,
            max.x + CANOPY_CLEAR_RADIUS_M,
            max.y + CANOPY_CLEAR_RADIUS_M,
        );
        for (patch_x, patch_z) in keys {
            self.vegetation_edits
                .bump_patch(pack_patch_key(patch_x as i32, patch_z as i32));
        }
    }

    /// Removes an unfinalized industry area placement before its polygon is committed.
    pub(crate) fn cancel_pending_industry_building_internal(
        &mut self,
        building_idx: usize,
    ) -> bool {
        let Some(building) = self.allocator.buildings.get(building_idx) else {
            return false;
        };
        if self
            .resource_extraction
            .site_for_building(building_idx)
            .is_some()
            || self.agriculture.site_for_building(building_idx).is_some()
            || !self
                .allocator
                .registry
                .is_industry_area_asset(&building.asset_id)
        {
            return false;
        }

        let lot_cells = f64::from(building.width_cells) * f64::from(building.depth_cells);
        let refund = lot_cells * SERVICE_BUILD_COST_PER_LOT_CELL;
        if !self.remove_building_for_edit(building_idx, false) {
            return false;
        }
        if !self.benchmark_mode {
            self.treasury.refund_build_cost(refund);
        }
        true
    }

    /// Repositions a network node. Unchanged moves leave authoritative state and undo history
    /// untouched.
    pub fn move_network_node_internal(&mut self, node_id: i32, pos: Vector3) -> bool {
        if node_id >= 0 && (node_id as usize) < self.region_graph.node_count() && pos.is_finite() {
            let node_id = self.region_graph.get_valid_node(node_id as u32);
            let old_pos = self.region_graph.node(node_id).pos;
            if old_pos == pos {
                return false;
            }
            let mut affected_edges: HashSet<usize> = self
                .region_graph
                .node_adjacency(node_id)
                .iter()
                .copied()
                .filter(|&id| !self.region_graph.edge(id).deleted)
                .collect();
            let mut affected_nodes = HashSet::from([node_id]);
            for &edge_idx in &affected_edges {
                let edge = self.region_graph.edge(edge_idx);
                affected_nodes.insert(edge.start_node);
                affected_nodes.insert(edge.end_node);
            }
            // Endpoint deformation can change the clips of other approaches at either end.
            // Keep their cell frontage, lane and surface dependents in the same transaction.
            for &node in &affected_nodes {
                affected_edges.extend(
                    self.region_graph
                        .node_adjacency(node)
                        .iter()
                        .copied()
                        .filter(|&id| !self.region_graph.edge(id).deleted),
                );
            }

            // The moved pieces must compile and keep clear of fields and painted cells
            // (`ROAD-68` open questions 2 and 11 (c)).
            let tried = self.try_graph_edit(&affected_edges, &affected_nodes, |core| {
                core.apply_node_move(node_id, pos, &affected_edges, &affected_nodes);
                HashSet::new()
            });
            let checkpoint = match tried {
                Ok(checkpoint) => checkpoint,
                Err(rejection) => {
                    debug_log!(
                        "road",
                        "node_move_rejected node={} reason={} detail={}",
                        node_id,
                        rejection.reason,
                        rejection.detail
                    );
                    self.last_road_rejection = Some(rejection);
                    return false;
                }
            };
            self.push_network_undo_delta(checkpoint, None);
            // Invalidate both corridors; moving a node can leave its old cached cells behind.
            self.zoning
                .mark_cell_lots_for_roads(&self.region_graph, affected_edges.iter().copied());
            self.apply_node_move(node_id, pos, &affected_edges, &affected_nodes);
            self.zoning
                .mark_cell_lots_for_roads(&self.region_graph, affected_edges.iter().copied());
            self.agents.invalidate_lane_ids_for_edges(
                &affected_edges,
                &self.transit_network.lane_system,
                &self.region_graph,
            );
            self.transit_network
                .lane_system
                .rebuild_edges_incremental(&mut self.region_graph, &affected_edges);
            self.agents.reattach_invalidated_lanes_for_edges(
                &affected_edges,
                &self.transit_network.lane_system,
                &self.region_graph,
            );
            self.rebuild_building_entrances_internal();
            // Deferred like a commit's (D7, `ROAD-68-G`); it supersedes a pending one.
            self.schedule_cch_rebuild_of_live_graph();
            self.transit_network.flow_fields.mark_all_dirty();
            self.transit_network
                .mark_surface_dirty_from_sets(&affected_edges, &affected_nodes);
            self.mark_local_network_render_dirty();
            return true;
        }
        false
    }

    // Moves a node on the graph alone: its position, the clips at the affected nodes and the
    // profiles the move reaches. Every edge the move deformed has both ends among the affected
    // nodes, so the solve re-derives each profile whose pins the move can reach, from its
    // control polyline (`ROAD-68` invariant 6); all of them are in the undo record.
    fn apply_node_move(
        &mut self,
        node_id: u32,
        pos: Vector3,
        affected_edges: &HashSet<usize>,
        affected_nodes: &HashSet<u32>,
    ) {
        self.region_graph.move_node(node_id, pos);
        self.region_graph
            .rebuild_intersection_clips_for_nodes(affected_nodes);
        let mut changed = HashSet::new();
        self.region_graph.finalize_junction_endpoint_profiles(
            affected_nodes,
            Some(&self.heightmap),
            &mut changed,
        );
        debug_assert!(changed.is_subset(affected_edges));
    }

    // Tries a graph-only edit that dirties nodes (`ROAD-68` open questions 2 and 11 (c)): applies
    // it, compiles the closure of `edges`, `nodes` and the edges `apply` returns into a scratch
    // roadbed, and puts the graph back from a local checkpoint of `edges` and `nodes`, which must
    // cover every record `apply` changes. `Ok` with that checkpoint, the edit's undo record, when
    // the closure compiles, keeps clear of fields and painted cells and steepens no profile past
    // the 1:3 maximum grade; nothing outside the graph has changed either way. O(local records +
    // closure pieces).
    fn try_graph_edit(
        &mut self,
        edges: &HashSet<usize>,
        nodes: &HashSet<u32>,
        apply: impl FnOnce(&mut Self) -> HashSet<usize>,
    ) -> Result<RegionGraphUndoDelta, RoadRejection> {
        let checkpoint = self
            .region_graph
            .capture_undo_for_local_topology(edges.clone(), nodes.clone());
        let grades_before: std::collections::HashMap<usize, f32> = edges
            .iter()
            .filter(|&&edge| edge < self.region_graph.edge_count())
            .map(|&edge| (edge, self.edge_grade(edge)))
            .collect();
        let changed = apply(self);
        let dirty: HashSet<usize> = edges.iter().copied().chain(changed).collect();
        let refusal = self
            .closure_refusal(&dirty, nodes)
            .or_else(|| self.steepened_profile(&dirty, &grades_before));
        self.region_graph.restore_undo_delta_local(&checkpoint);
        refusal.map_or(Ok(checkpoint), Err)
    }

    // The grade of one edge's profile by the one grade (`grade::steepest_grade`); zero
    // without a graded segment.
    fn edge_grade(&self, edge: usize) -> f32 {
        crate::simulation::network::grade::steepest_grade(
            crate::simulation::network::validate::centreline(self.region_graph.edge(edge)),
        )
        .map_or(0.0, |steepest| steepest.grade)
    }

    // The first of `edges`, in id order, whose profile a node edit made illegally steep
    // (`ROAD-68` open question 37): above the 1:3 maximum and steeper than `before` held it. A
    // profile already that steep is the existing road's, so editing beside it stays possible.
    // O(points on `edges`).
    fn steepened_profile(
        &self,
        edges: &HashSet<usize>,
        before: &std::collections::HashMap<usize, f32>,
    ) -> Option<RoadRejection> {
        use crate::simulation::network::validate::{
            RoadGeometryLimits, RoadGeometryTier, RoadGeometryViolation, grade_violation,
        };
        let graph = &self.region_graph;
        let mut edges: Vec<usize> = edges
            .iter()
            .copied()
            .filter(|&edge| edge < graph.edge_count())
            .collect();
        edges.sort_unstable();
        let limits = RoadGeometryLimits::default();
        edges.into_iter().find_map(|edge| {
            let old = before.get(&edge).copied().unwrap_or(-1.0);
            match grade_violation(graph, edge, &limits)? {
                RoadGeometryViolation::Grade {
                    measured,
                    limit,
                    tier: RoadGeometryTier::Illegal,
                    ..
                } if measured > old => Some(RoadRejection::new(
                    "road_too_steep",
                    format!(
                        "edge {edge} would climb at {:.0}%, steeper than the {:.0}% (1:3) \
                         maximum grade",
                        measured * 100.0,
                        limit * 100.0
                    ),
                )),
                _ => None,
            }
        })
    }

    // Why the pieces of a node-dirtying edit cannot publish (`ROAD-68` open questions 2 and
    // 11 (c)): a node of the closure fails its layout or profile solve, or a piece overlaps a
    // field or painted cells. Compiles the closure of `edges` and `nodes` into an `O(1)` copy of
    // the published roadbed, with the graph as it stands. O(closure pieces).
    fn closure_refusal(
        &self,
        edges: &HashSet<usize>,
        nodes: &HashSet<u32>,
    ) -> Option<RoadRejection> {
        let delta = self.compile_closure(edges, nodes);
        if let Some((node, error)) = delta.failed_nodes().next() {
            return Some(RoadRejection::new(
                "node_piece_failed",
                format!("node {node} fails its junction layout or profile solve: {error:?}"),
            ));
        }
        let roadbed = &self.transit_network.roadbed;
        let before: Vec<_> = delta
            .pieces()
            .filter_map(|(id, _)| roadbed.piece_mesh(id))
            .collect();
        ReservedOverlap::first_in_core(self, delta.pieces().filter_map(|(_, mesh)| mesh), &before)
            .map(ReservedOverlap::rejection)
    }

    // The closure of `edges` and `nodes` compiled, unpublished, into an `O(1)` copy of the
    // published roadbed with the graph as it stands; ids past the graph's records are skipped.
    // O(closure pieces).
    fn compile_closure(
        &self,
        edges: &HashSet<usize>,
        nodes: &HashSet<u32>,
    ) -> crate::simulation::network::roadbed::RoadbedDelta {
        let graph = &self.region_graph;
        let mut scratch = self.transit_network.roadbed.snapshot();
        for &edge in edges.iter().filter(|&&edge| edge < graph.edge_count()) {
            scratch.mark_edge_dirty(edge);
        }
        for &node in nodes
            .iter()
            .filter(|&&node| (node as usize) < graph.node_count())
        {
            scratch.mark_node_dirty(graph.get_valid_node(node));
        }
        scratch.compile_delta(graph)
    }

    // Undo restores a graph that compiled when it was published, so its closure compiles again
    // and a failed node there is a bug (`ROAD-68` open question 11 (c)). Debug builds only.
    // O(closure pieces).
    pub(super) fn debug_assert_closure_compiles(
        &self,
        edges: &HashSet<usize>,
        nodes: &HashSet<u32>,
    ) {
        if cfg!(debug_assertions) {
            let delta = self.compile_closure(edges, nodes);
            let failed: Vec<_> = delta.failed_nodes().collect();
            debug_assert!(failed.is_empty(), "undo restored failing nodes: {failed:?}");
        }
    }

    /// Sets a lane connection rule at a junction node.
    pub fn set_lane_connection_internal(
        &mut self,
        node_id: u32,
        from_edge: i32,
        from_lane: i32,
        to_edge: i32,
        to_lane: i32,
    ) {
        traffic_log!(
            "[LANE_EDIT] set_lane_connection: node={node_id} from_edge={from_edge} from_lane={from_lane} to_edge={to_edge} to_lane={to_lane}"
        );
        if (node_id as usize) < self.region_graph.node_count() {
            self.push_network_undo_for_local_topology(HashSet::new(), HashSet::from([node_id]));
            let key = (from_edge as usize, from_lane as i8);
            let target = (to_edge as usize, to_lane as i8);
            let already = self
                .region_graph
                .node(node_id)
                .lane_connections
                .get(&key)
                .is_some_and(|v| v.contains(&target));
            if !already {
                self.region_graph
                    .add_lane_connection(node_id, key.0, key.1, target.0, target.1);
            }
        }
        let affected: HashSet<usize> = self
            .region_graph
            .node_adjacency(node_id)
            .iter()
            .copied()
            .collect();
        self.agents.invalidate_lane_ids_for_edges(
            &affected,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        self.transit_network
            .lane_system
            .rebuild_edges_incremental(&mut self.region_graph, &affected);
        self.agents.reattach_invalidated_lanes_for_edges(
            &affected,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        self.rebuild_building_entrances_internal();
        if crate::debug::Channel::Traffic.enabled()
            && (node_id as usize) < self.region_graph.node_count()
        {
            let conns = &self.region_graph.node(node_id).lane_connections;
            let mut entries: Vec<_> = conns
                .iter()
                .map(|(&(e, l), targets)| format!("  (edge={e},lane={l}) -> {:?}", targets))
                .collect();
            entries.sort();
            eprintln!("[LANE_EDIT] node={node_id} lane_connections after rebuild:");
            for s in &entries {
                eprintln!("{s}");
            }
        }
        // Deferred like a commit's (D7, `ROAD-68-G`): the job's graph copy holds this edit, so
        // no chunk is left marked for a second, synchronous rebuild.
        self.schedule_cch_rebuild_of_live_graph();
        self.transit_network.flow_fields.mark_all_dirty();
    }

    /// Clears all lane connections at a junction node.
    pub fn clear_lane_connections_internal(&mut self, node_id: u32) {
        if (node_id as usize) < self.region_graph.node_count() {
            self.push_network_undo_for_local_topology(HashSet::new(), HashSet::from([node_id]));
            let keys: Vec<_> = self
                .region_graph
                .node(node_id)
                .lane_connections
                .keys()
                .copied()
                .collect();
            for key in keys {
                self.region_graph.remove_lane_connection(node_id, key);
            }
        }
        let affected: HashSet<usize> = self
            .region_graph
            .node_adjacency(node_id)
            .iter()
            .copied()
            .collect();
        self.agents.invalidate_lane_ids_for_edges(
            &affected,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        self.transit_network
            .lane_system
            .rebuild_edges_incremental(&mut self.region_graph, &affected);
        self.agents.reattach_invalidated_lanes_for_edges(
            &affected,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        self.rebuild_building_entrances_internal();
        // Deferred like a commit's (D7, `ROAD-68-G`); its graph copy holds this edit.
        self.schedule_cch_rebuild_of_live_graph();
        self.transit_network.flow_fields.mark_all_dirty();
    }

    /// Clears lane connections for a specific source edge/lane at a junction.
    pub fn clear_lane_source_internal(&mut self, node_id: u32, from_edge: i32, from_lane: i32) {
        if node_id as usize >= self.region_graph.node_count() {
            return;
        }

        self.push_network_undo_for_local_topology(HashSet::new(), HashSet::from([node_id]));
        self.region_graph
            .remove_lane_connection(node_id, (from_edge as usize, from_lane as i8));

        let affected: HashSet<usize> = self
            .region_graph
            .node_adjacency(node_id)
            .iter()
            .copied()
            .collect();
        self.agents.invalidate_lane_ids_for_edges(
            &affected,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        self.transit_network
            .lane_system
            .rebuild_edges_incremental(&mut self.region_graph, &affected);
        self.agents.reattach_invalidated_lanes_for_edges(
            &affected,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        self.rebuild_building_entrances_internal();
        // Deferred like a commit's (D7, `ROAD-68-G`); its graph copy holds this edit.
        self.schedule_cch_rebuild_of_live_graph();
        self.transit_network.flow_fields.mark_all_dirty();
    }

    /// Toggles a user override for a crosswalk at a specific road mouth.
    ///
    /// A crossing moves its arm's mouth in the junction layout (D8), so the toggle re-lays the
    /// node out as a road edit does: clips, the profile solve that pins the arms up to them, the
    /// node's roadbed closure and its lanes. A toggle whose closure would not compile, or whose
    /// pieces would overlap fields or painted cells, is refused.
    pub fn set_crosswalk_override_internal(&mut self, node_id: u32, edge_id: i32, enabled: bool) {
        if node_id as usize >= self.region_graph.node_count() || edge_id < 0 {
            return;
        }
        let node = self.region_graph.get_valid_node(node_id);
        let edge_id = edge_id as usize;
        let affected: HashSet<usize> = self
            .region_graph
            .node_adjacency(node)
            .iter()
            .copied()
            .filter(|&edge| !self.region_graph.edge(edge).deleted)
            .collect();
        let edges = affected.iter().copied().chain([edge_id]).collect();
        // The layout reads the overrides of the node's surviving id. A toggle whose node no
        // longer compiles, or whose moved mouths overlap reserved land, is refused (`ROAD-68`
        // open question 11 (c)).
        let toggle = |core: &mut Self| {
            core.region_graph
                .set_crosswalk_override(node, edge_id, enabled);
            let nodes = HashSet::from([node]);
            core.region_graph
                .rebuild_intersection_clips_for_nodes(&nodes);
            // A crossing moves the mouths, so the node's arms re-derive their profiles.
            let mut changed = HashSet::new();
            core.region_graph.finalize_junction_endpoint_profiles(
                &nodes,
                Some(&core.heightmap),
                &mut changed,
            );
            changed
        };
        let checkpoint =
            match self.try_graph_edit(&edges, &HashSet::from([node_id, node]), toggle) {
                Ok(checkpoint) => checkpoint,
                Err(rejection) => {
                    debug_log!(
                        "road",
                        "crosswalk_override_rejected node={} edge={} reason={}",
                        node,
                        edge_id,
                        rejection.reason
                    );
                    return;
                }
            };
        self.push_network_undo_delta(checkpoint, None);
        let changed = toggle(self);
        let nodes = HashSet::from([node]);
        self.transit_network
            .mark_surface_dirty_for_nodes(&self.region_graph, &nodes);
        self.transit_network
            .mark_surface_dirty_from_sets(&changed, &HashSet::new());
        let affected: HashSet<usize> = affected.into_iter().chain(changed).collect();
        self.agents.invalidate_lane_ids_for_edges(
            &affected,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        self.transit_network
            .lane_system
            .rebuild_edges_incremental(&mut self.region_graph, &affected);
        self.agents.reattach_invalidated_lanes_for_edges(
            &affected,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        self.rebuild_building_entrances_internal();
        // Deferred like a commit's (D7, `ROAD-68-G`), not left to the next frame's synchronous
        // rebuild of marked chunks.
        self.schedule_cch_rebuild_of_live_graph();
    }

    /// Rebuilds road-surface-driven visual terrain after network edits.
    pub fn rebuild_network_surface_terrain_internal(&mut self) {
        self.rebuild_network_surface_terrain_internal_with_entrance_rebuild(true);
    }

    pub(crate) fn rebuild_network_surface_terrain_internal_with_entrance_rebuild(
        &mut self,
        rebuild_entrances: bool,
    ) -> Vec<(usize, usize)> {
        self.publish_network_surface(rebuild_entrances, None).0
    }

    /// Publishes the roadbed and dirties the terrain patches its changed pieces touch. Installs
    /// `delta` (a preview's pieces, `ROAD-68-E`) when the roadbed still stands where it was
    /// compiled, otherwise compiles the dirty closure. Returns the dirtied patches, what the
    /// install replaced, for undo, and whether `delta` was the one installed.
    pub(crate) fn publish_network_surface(
        &mut self,
        rebuild_entrances: bool,
        delta: Option<&RoadbedDelta>,
    ) -> (Vec<(usize, usize)>, RoadbedRestore, bool) {
        let road_debug = crate::debug::Channel::Road.enabled();
        let total_start = road_debug.then(Instant::now);

        let compile_start = road_debug.then(Instant::now);
        let roadbed = &mut self.transit_network.roadbed;
        let compiled;
        let preview = delta.filter(|delta| roadbed.can_install(delta));
        let installed = preview.is_some();
        let delta = match preview {
            Some(delta) => delta,
            None => {
                compiled = roadbed.compile_delta(&self.region_graph);
                &compiled
            }
        };
        let (changed_chunks, restore) = roadbed.install(delta);
        let road_terrain_patches = self.publish_roadbed_terrain_changes();
        // Generated trees keep clear of walkways and poles beside them, possibly from a
        // neighboring terrain patch; the canopy margin covers both setbacks.
        for &chunk in &changed_chunks {
            let [min_x, min_z, max_x, max_z] = self.transit_network.roadbed.chunk_bounds(chunk);
            self.invalidate_vegetation_over((
                Vector2::new(min_x as f32, min_z as f32),
                Vector2::new(max_x as f32, max_z as f32),
            ));
        }
        let compile_ms = compile_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let entrances_start = road_debug.then(Instant::now);
        if rebuild_entrances {
            self.rebuild_building_entrances_internal();
        }
        let entrances_ms = entrances_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        self.terrain_dirty = true;
        if road_debug {
            debug_log!(
                "road",
                "terrain_rebuild_detail roadbed_ms={:.3} changed_chunks={} road_terrain_patches={} dirty_render_patches={} entrances_ms={:.3} total_ms={:.3}",
                compile_ms,
                changed_chunks.len(),
                road_terrain_patches.len(),
                self.heightmap.dirty_render_patches().len(),
                entrances_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
        (road_terrain_patches, restore, installed)
    }

    /// Publishes every pending canonical roadbed change to its terrain consumers.
    ///
    /// The work is local to the union of each replaced piece's old and new cap/apron reach:
    /// `O(changed inputs * covered tiles * log local inputs)`, independent of city size.
    pub(crate) fn publish_roadbed_terrain_changes(&mut self) -> Vec<(usize, usize)> {
        let changes = self.transit_network.roadbed.take_terrain_changes();
        if changes.pieces.is_empty()
            && changes.chains.is_empty()
            && changes.segments.is_empty()
            && changes.portals.is_empty()
        {
            return Vec::new();
        }
        let patches = self.bump_road_terrain_payload_generations(&changes);
        self.record_graded_road_edit(&changes);
        self.last_road_terrain_changes = changes;
        patches
    }

    /// Returns the ID of the nearest node to `pos` if `pos` is within
    /// [`config::BORDER_DETECTION_THRESHOLD`] metres of any map edge, or `-1` if not.
    ///
    /// Call this after road insertion with the road's start or end position to find
    /// out whether a border-connection dialog should be presented to the player.
    pub fn check_border_candidate_internal(&self, pos: Vector3) -> i64 {
        // The actual world-space boundary is derived from the heightmap dimensions, not
        // config.width_m (which is a logical grid size, not the terrain world extent).
        let (half_w, half_h) = self.heightmap.half_world_extents();
        let t = config::BORDER_DETECTION_THRESHOLD;

        let near_border =
            pos.x < -half_w + t || pos.x > half_w - t || pos.z < -half_h + t || pos.z > half_h - t;

        if !near_border {
            debug_log!(
                "economy",
                "border candidate rejected at pos=({:.1}, {:.1}, {:.1}) because it is not near the map boundary",
                pos.x,
                pos.y,
                pos.z
            );
            return -1;
        }

        // Use a generous tolerance: the node was snapped during add_road so it should be
        // very close, but terrain raycast imprecision may add a few metres of offset.
        let candidate = match crate::simulation::network::interaction::get_closest_node(
            &self.region_graph,
            pos,
            config::SNAP_TOLERANCE * 5.0,
        ) {
            Some(id) => id as i64,
            None => -1,
        };
        debug_log!(
            "economy",
            "border candidate check at pos=({:.1}, {:.1}, {:.1}) -> node_id={}",
            pos.x,
            pos.y,
            pos.z,
            candidate
        );
        candidate
    }

    /// Designates the node at `node_id` as an external border connection.
    ///
    /// Marks the node as [`Border`](crate::simulation::network::types::NodeType::Border),
    /// making a live connected road eligible for demand-driven household arrivals and external trips.
    pub fn set_border_connection_internal(&mut self, node_id: i32) {
        if node_id < 0 || (node_id as usize) >= self.region_graph.node_count() {
            debug_log!(
                "economy",
                "set_border_connection ignored for invalid node_id={}",
                node_id
            );
            return;
        }
        let old_pos = self.region_graph.node(node_id as u32).pos;

        self.region_graph.set_node_type(
            node_id as u32,
            crate::simulation::network::types::NodeType::Border,
        );
        debug_log!(
            "economy",
            "border connection created at node_id={} pos=({:.1}, {:.1}, {:.1})",
            node_id,
            old_pos.x,
            old_pos.y,
            old_pos.z
        );

        // Auto-extend it 10 meters further from the connecting road so agents spawn visually off-screen
        let mut rebuild_needed = false;
        let adj: Vec<usize> = self.region_graph.node_adjacency(node_id as u32).to_vec();
        {
            let mut valid_edges = Vec::new();
            for &e_idx in &adj {
                if !self.region_graph.edge(e_idx).deleted {
                    valid_edges.push(e_idx);
                }
            }
            if valid_edges.len() == 1 {
                let e_idx = valid_edges[0];
                let is_end = self.region_graph.edge(e_idx).end_node == (node_id as u32);
                let other_node = if is_end {
                    self.region_graph.edge(e_idx).start_node
                } else {
                    self.region_graph.edge(e_idx).end_node
                };

                let p1 = self.region_graph.node(other_node).pos;
                let p2 = self.region_graph.node(node_id as u32).pos;

                let dir_vec = p2 - p1;
                if dir_vec.length_squared() > 0.001 {
                    let dir = dir_vec.normalized();
                    let new_p2 = p2 + dir * crate::config::BORDER_EXTENSION_M;
                    self.region_graph.set_node_pos(node_id as u32, new_p2);

                    let edge = self.region_graph.edge_mut(e_idx);
                    if is_end {
                        if let Some(last) = edge.geometry.last_mut() {
                            *last = new_p2;
                        }
                        if let Some(last) = edge.physical_geometry.last_mut() {
                            *last = new_p2;
                        }
                    } else {
                        if let Some(first) = edge.geometry.first_mut() {
                            *first = new_p2;
                        }
                        if let Some(first) = edge.physical_geometry.first_mut() {
                            *first = new_p2;
                        }
                    }

                    // Recalculate length and cost
                    let mut new_len = 0.0;
                    for i in 0..edge.physical_geometry.len() - 1 {
                        let pa = edge.physical_geometry[i];
                        let pb = edge.physical_geometry[i + 1];
                        let dx = pb.x - pa.x;
                        let dz = pb.z - pa.z;
                        new_len += (dx * dx + dz * dz).sqrt();
                    }
                    edge.physical_length = new_len;
                    edge.base_cost =
                        crate::simulation::pathing::cost::CostCalculator::calculate_costs(edge).0;
                    rebuild_needed = true;
                }
            }
        }

        if rebuild_needed {
            self.transit_network
                .lane_system
                .rebuild(&mut self.region_graph);
            // Deferred like a commit's (D7, `ROAD-68-G`); it supersedes a pending one.
            self.schedule_cch_rebuild_of_live_graph();
            let mut affected_nodes = HashSet::from([node_id as u32]);
            let mut affected_edges = HashSet::new();
            for &edge_idx in &adj {
                if !self.region_graph.edge(edge_idx).deleted {
                    let edge = self.region_graph.edge(edge_idx);
                    affected_nodes.insert(edge.start_node);
                    affected_nodes.insert(edge.end_node);
                    affected_edges.insert(edge_idx);
                }
            }
            self.transit_network
                .mark_surface_dirty_from_sets(&affected_edges, &affected_nodes);
            // The moved end changes the road's grid alignment source; a stale one fails to load.
            self.zoning
                .mark_cell_lots_for_roads(&self.region_graph, affected_edges.iter().copied());
            let new_pos = self.region_graph.node(node_id as u32).pos;
            debug_log!(
                "economy",
                "border connection lane/CCH rebuild complete for node_id={} new_pos=({:.1}, {:.1}, {:.1})",
                node_id,
                new_pos.x,
                new_pos.y,
                new_pos.z
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BulldozeTargetKind, SimCore};
    use crate::nodes::sim::core::{PendingDemandSpawnAction, ROAD_LOCKED_TERRAIN_RENDER_STEP_M};

    use crate::simulation::buildings::allocator::Building;
    use crate::simulation::core::config::WorldConfig;

    use crate::simulation::economy::demand::DemandSpawnAction;

    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::render::NetworkMeshData;
    use crate::simulation::network::roadbed::PieceId;
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };

    use crate::simulation::terrain::TerrainSystem;

    use crate::simulation::zoning::ZoneType;
    use godot::prelude::{Vector2, Vector3};
    use std::collections::HashSet;

    use std::sync::Arc;

    fn test_core() -> SimCore {
        use crate::nodes::sim::core::CityTreasury;
        let config = WorldConfig::default();
        SimCore {
            treasury: CityTreasury::new(0.0),
            terrain_dirty: false,
            water_dirty: false,
            benchmark_mode: true,
            ..SimCore::new(config)
        }
    }

    fn add_test_road_edge(graph: &mut RegionGraph, start_node: u32, end_node: u32) -> usize {
        let start = graph.node(start_node).pos;
        let end = graph.node(end_node).pos;
        graph.add_edge(Edge {
            start_node,
            end_node,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: start.distance_to(end),
            physical_length: start.distance_to(end),
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![start, end],
            physical_geometry: vec![start, end],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        })
    }

    fn test_building(asset_id: &str, center_x: f32, support_height_m: f32) -> Building {
        let mut building = crate::simulation::buildings::allocator::indexed_test_building(
            asset_id.to_owned(),
            ZoneType::Residential,
            0,
        );
        building.center_x = center_x;
        building.support_height_m = support_height_m;
        building.width_cells = 2;
        building.depth_cells = 2;
        building.side_offset = 1.0;
        building.edge_idx = usize::MAX;
        building
    }

    #[test]
    fn set_vehicle_frontage_access_internal_updates_edge_and_ignores_invalid_codes() {
        let mut core = test_core();
        let n0 = core
            .region_graph
            .add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = core
            .region_graph
            .add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        core.region_graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 10.0,
            physical_length: 10.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });

        core.set_vehicle_frontage_access_internal(0, 0);
        assert_eq!(
            core.region_graph.edge(0).vehicle_frontage_access,
            VehicleFrontageAccess::SameSideOnly
        );

        core.set_vehicle_frontage_access_internal(0, 9);
        assert_eq!(
            core.region_graph.edge(0).vehicle_frontage_access,
            VehicleFrontageAccess::SameSideOnly
        );
    }

    #[test]
    fn set_no_building_spawn_preserves_unrelated_rezone_grace() {
        use crate::assets::AssetManifest;

        let mut core = test_core();
        let manifest: AssetManifest = serde_json::from_value(serde_json::json!({
            "asset_id": "house", "display_name": "House",
            "building": {
                "placement_mode": "zoned_private", "zone_type": "residential", "density": "low",
                "lot_width_cells": 1, "lot_depth_cells": 1,
                "flat_size_m2": 80.0, "household_capacity": 1
            },
            "mesh_parts": [{"name": "main", "lods": [{"file": "lod0.glb", "distance_min_m": 0.0}]}],
            "anchors": [{"type": "entrance", "name": "main", "position": [0.0, 0.0, 0.5], "forward": [0.0, 0.0, 1.0]}]
        })).unwrap();
        manifest.validate().unwrap();
        core.allocator
            .registry
            .register("test", manifest, String::new());
        let mut parcel_ids = Vec::new();
        for (idx, (z, zone)) in [(0.0, ZoneType::Residential), (200.0, ZoneType::Commercial)]
            .into_iter()
            .enumerate()
        {
            let start = core
                .region_graph
                .add_node(Vector3::new(-60.0, 0.0, z), NodeType::Junction);
            let end = core
                .region_graph
                .add_node(Vector3::new(60.0, 0.0, z), NodeType::Junction);
            let edge = add_test_road_edge(&mut core.region_graph, start, end);
            let profile = core
                .zoning
                .profiles
                .default_runtime_id_for_zone_type(zone)
                .unwrap();
            let id = core
                .zoning
                .place_or_rezone_default_parcel_at(0.0, z - 20.0, profile, &core.region_graph)
                .unwrap();
            let parcel = core.zoning.parcel_by_raw_id(id.raw()).unwrap();
            let center = parcel.front_center() + parcel.normal() * (core.config.zone_cell_m * 0.5);
            let mut building = test_building("test:house", center.x, 0.0);
            building.center_y = center.y;
            building.width_cells = 1;
            building.depth_cells = 1;
            building.edge_idx = edge;
            building.parcel_id = id.raw();
            building.zone_profile_runtime_id = profile;
            building.frontage_t = parcel.frontage_center_t();
            building.facing_dir = -parcel.normal();
            building.side = parcel.side();
            building.side_offset =
                core.region_graph.edge(edge).width * 0.5 + crate::config::SIDEWALK_WIDTH;
            core.allocator.buildings.push(building);
            assert!(core.zoning.occupy_parcel(id.raw(), idx));
            parcel_ids.push(id.raw());
        }
        core.allocator
            .rebuild_building_site_clients(core.config.zone_cell_m);
        core.allocator.rebuild_zone_index();

        core.set_no_building_spawn_internal(0, true);

        assert!(core.region_graph.edge(0).no_building_spawn);
        assert!(core.zoning.parcel_by_raw_id(parcel_ids[0]).is_none());
        assert_eq!(core.allocator.buildings.len(), 1);
        assert_eq!(core.allocator.buildings[0].parcel_id, parcel_ids[1]);
        assert_eq!(
            core.zoning
                .parcel_by_raw_id(parcel_ids[1])
                .unwrap()
                .occupied_building(),
            Some(0)
        );
        for _ in 0..3 {
            core.set_no_building_spawn_internal(0, false);
            core.set_no_building_spawn_internal(0, true);
            assert_eq!(core.allocator.buildings.len(), 1);
            let building = &core.allocator.buildings[0];
            assert!(building.pending_redevelopment);
            assert_eq!(building.rezone_grace_days_remaining, 3);
        }
        core.allocator.maintain(
            1,
            &mut core.zoning,
            &mut core.agents,
            &mut core.households,
            &mut core.logistics,
            &mut core.treasury.balance,
            &mut core.transit_network,
            &mut core.region_graph,
        );
        assert_eq!(core.allocator.buildings[0].rezone_grace_days_remaining, 2);
    }

    #[test]
    fn bulldoze_preserves_remote_lane_owners() {
        let mut core = test_core();
        for z in [0.0, 200.0] {
            let start = core
                .region_graph
                .add_node(Vector3::new(-60.0, 0.0, z), NodeType::Junction);
            let end = core
                .region_graph
                .add_node(Vector3::new(60.0, 0.0, z), NodeType::Junction);
            add_test_road_edge(&mut core.region_graph, start, end);
        }
        core.region_graph.rebuild_adjacency_list();
        core.transit_network
            .lane_system
            .rebuild(&mut core.region_graph);
        let remote_lanes = core.transit_network.lane_system.edge_lanes[&1].clone();

        assert!(core.bulldoze_road_edge(0));

        assert_eq!(
            core.transit_network.lane_system.edge_lanes[&1], remote_lanes,
            "ROAD-13: local invalidation must not renumber a distant agent's live lane"
        );
        for id in remote_lanes {
            assert_eq!(core.transit_network.lane_system.lanes[id].edge_id, 1);
        }
        assert!(!core.transit_network.lane_system.edge_lanes.contains_key(&0));
    }

    #[test]
    fn bulldoze_road_targets_tightly_deletes_and_undo_restores() {
        let mut core = test_core();
        let n0 = core
            .region_graph
            .add_node(Vector3::new(-60.0, 0.0, 0.0), NodeType::Junction);
        let n1 = core
            .region_graph
            .add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
        core.region_graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 120.0,
            physical_length: 120.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
        core.region_graph.rebuild_adjacency_list();
        let residential = core
            .zoning
            .profiles
            .default_runtime_id_for_zone_type(ZoneType::Residential)
            .unwrap();
        core.zoning
            .place_or_rezone_default_parcel_at(0.0, -20.0, residential, &core.region_graph)
            .expect("parcel");
        core.zoning
            .place_or_rezone_default_parcel_at(0.0, 20.0, residential, &core.region_graph)
            .expect("second parcel");
        let original_parcel_ids = core
            .zoning
            .parcels()
            .iter()
            .map(|parcel| parcel.id())
            .collect::<Vec<_>>();
        assert_eq!(original_parcel_ids.len(), 2);

        let target = core.resolve_bulldoze_target(0.0, 0.0).expect("road target");
        assert_eq!(target.kind, BulldozeTargetKind::Road);
        assert_eq!(target.id, 0);
        assert!(
            core.resolve_bulldoze_target(0.0, 20.0).is_none(),
            "bulldoze targeting must stay close to the actual road footprint"
        );

        assert_eq!(core.bulldoze_prepared_target_internal(target), Some(true));
        assert!(core.region_graph.edge(0).deleted);
        assert!(core.zoning.parcels().is_empty());
        assert!(
            !core
                .region_graph
                .get_edges_near_point(Vector3::new(0.0, 0.0, 0.0), 8.0)
                .contains(&0)
        );
        assert!(core.network_dirty);
        assert!(core.terrain_dirty);

        assert!(core.undo_action_internal());
        assert!(!core.region_graph.edge(0).deleted);
        assert_eq!(
            core.zoning
                .parcels()
                .iter()
                .map(|parcel| parcel.id())
                .collect::<Vec<_>>(),
            original_parcel_ids,
            "sparse zoning undo must preserve stable parcel ids and storage order"
        );
        assert!(
            core.region_graph
                .get_edges_near_point(Vector3::new(0.0, 0.0, 0.0), 8.0)
                .contains(&0)
        );
    }

    // `ROAD-68` open question 37 (b): the road pick compares distances exactly, then takes the
    // lower edge id, whichever order the R-tree holds the edges in.
    #[test]
    fn bulldoze_road_pick_compares_exactly_and_ties_take_the_lowest_edge_id() {
        let mut core = test_core();
        // Edges 0 and 1 lie 2 m either side of x = 1000. Edge 3 lies 0.25 m from the origin and
        // edge 2 three ulps further, its squared distance within `f32::EPSILON` of edge 3's.
        let near = f32::from_bits(0.25_f32.to_bits() + 3);
        for x in [998.0, 1002.0, -near, 0.25] {
            let start = core
                .region_graph
                .add_node(Vector3::new(x, 0.0, -60.0), NodeType::Junction);
            let end = core
                .region_graph
                .add_node(Vector3::new(x, 0.0, 60.0), NodeType::Junction);
            add_test_road_edge(&mut core.region_graph, start, end);
        }
        for order in [[0, 1, 2, 3], [3, 2, 1, 0]] {
            for id in 0..4 {
                core.region_graph.remove_from_spatial_index(id);
            }
            for id in order {
                core.region_graph.add_to_spatial_index(id);
            }
            let pick = |x: f32| {
                core.resolve_bulldoze_target(x, 0.0)
                    .map(|target| (target.kind, target.id))
            };
            assert_eq!(
                pick(1000.0),
                Some((BulldozeTargetKind::Road, 0)),
                "exact tie, tree insertion order {order:?}"
            );
            assert_eq!(
                pick(0.0),
                Some((BulldozeTargetKind::Road, 3)),
                "nearer by less than f32::EPSILON, tree insertion order {order:?}"
            );
        }
    }

    #[test]
    fn runtime_undo_restores_pending_demand_spawn_queue() {
        let mut core = test_core();
        core.pending_demand_spawns
            .push_back(PendingDemandSpawnAction {
                id: 0,
                due_minute: 42,
                zone_type: ZoneType::Residential,
                action: DemandSpawnAction {
                    parcel_id: 7,
                    asset_id: "building.residential.test".to_owned(),
                },
                planned_day_index: 2,
                planned_minute_of_day: 60,
                permit: Default::default(),
                counted: false,
            });

        core.push_undo_state_with_runtime(false, false, false, true);
        core.pending_demand_spawns.clear();

        assert!(core.undo_action_internal());
        let pending = core
            .pending_demand_spawns
            .front()
            .expect("runtime undo must restore delayed demand spawns");
        assert_eq!(pending.due_minute, 42);
        assert_eq!(pending.zone_type, ZoneType::Residential);
        assert_eq!(pending.action.parcel_id, 7);
        assert_eq!(pending.action.asset_id, "building.residential.test");
        assert_eq!(pending.planned_day_index, 2);
        assert_eq!(pending.planned_minute_of_day, 60);
    }

    #[test]
    fn building_bulldoze_undo_restores_erased_cell_lot_and_generation() {
        use crate::simulation::zoning::cells::{
            CellBounds, CellFrontage, CellLot, CellSelectionShape,
        };
        use glam::DVec2;
        let mut core = test_core();
        let start = core
            .region_graph
            .add_node(Vector3::new(-60.0, 0.0, 0.0), NodeType::Junction);
        let end = core
            .region_graph
            .add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
        let edge = add_test_road_edge(&mut core.region_graph, start, end);
        core.zoning.generate_cells(
            &core.region_graph,
            CellBounds {
                min: DVec2::splat(-100.0),
                max: DVec2::splat(100.0),
            },
            |_| false,
        );
        let point = DVec2::new(-25.0, 10.0);
        let selection = core.zoning.cells.select(CellSelectionShape::Fill, &[point]);
        core.zoning.paint_cells(&selection, 1, |_| false).unwrap();
        let lot = CellLot::new(
            core.zoning.cells.pick(point).unwrap(),
            2,
            2,
            CellFrontage::MinY,
        )
        .unwrap();
        let id = core
            .zoning
            .install_prevalidated_cell_lot(lot, edge, -1, 1.0 / 3.0, 1)
            .unwrap();
        core.zoning.restore_parcel_build_generation(id.raw(), 9);
        let parcel = core.zoning.parcels.get(id).unwrap();
        let mut building = test_building("building.cell_removed", parcel.center().x, 0.0);
        building.center_y = parcel.center().y;
        building.facing_dir = -parcel.normal();
        building.parcel_id = id.raw();
        building.edge_idx = edge;
        building.side = -1;
        building.frontage_t = parcel.frontage_center_t();
        building.build_generation = 9;
        building.pending_redevelopment = true;
        building.rezone_grace_days_remaining = 11;
        core.allocator.buildings.push(building);
        assert!(core.zoning.occupy_parcel(id.raw(), 0));
        let selection = core.zoning.cells.select(CellSelectionShape::Cell, &[point]);
        core.zoning.paint_cells(&selection, 0, |_| false).unwrap();
        core.allocator
            .rebuild_building_site_clients(core.zoning.config.zone_cell_m);
        core.allocator.rebuild_zone_index();
        let paint = core.zoning.cells.saved_cells();
        assert!(core.bulldoze_building(0));
        assert!(core.zoning.parcels.get(id).is_none());
        assert!(lot.cells().all(|key| core.zoning.cells.lot(key) == Some(0)));
        assert!(core.undo_action_internal());
        let parcel = core.zoning.parcels.get(id).unwrap();
        assert_eq!(parcel.occupied_building(), Some(0));
        assert_eq!(parcel.zone_profile_runtime_id(), 0);
        assert_eq!(parcel.build_generation(), 9);
        assert_eq!(core.zoning.cells.saved_cells(), paint);
        assert!(
            lot.cells()
                .all(|key| core.zoning.cells.lot(key) == Some(id.raw()))
        );
        assert!(core.allocator.buildings[0].pending_redevelopment);
        assert_eq!(core.allocator.buildings[0].rezone_grace_days_remaining, 11);
    }

    #[test]
    fn curved_road_bulldoze_and_undo_keep_occupied_cell_claims_until_building_removal() {
        use crate::simulation::zoning::cells::{CellBounds, CellLotSize, CellSelectionShape};
        use glam::DVec2;

        for (angle, erased) in [0.0_f64, 0.35]
            .into_iter()
            .flat_map(|angle| [false, true].map(|erased| (angle, erased)))
        {
            let mut core = test_core();
            core.benchmark_mode = false;
            let u = DVec2::new(angle.cos(), angle.sin());
            let points = [(-60.0, 0.0), (-20.0, 2.0), (20.0, 8.0), (60.0, 18.0)]
                .map(|(x, y)| {
                    let p = u * x + u.perp() * y;
                    Vector3::new(p.x as f32, 0.0, p.y as f32)
                })
                .to_vec();
            assert!(core.add_road_internal(points, 1, 1).committed);
            finalize_network_render_for_test(&mut core);
            let bounds = CellBounds {
                min: DVec2::splat(-200.0),
                max: DVec2::splat(200.0),
            };
            core.zoning
                .generate_cells(&core.region_graph, bounds, |_| false);
            let selection = core.zoning.cells.select(
                CellSelectionShape::Brush { radius_m: 200.0 },
                &[DVec2::ZERO],
            );
            core.zoning.paint_cells(&selection, 1, |_| false).unwrap();
            let lots = core.zoning.derive_cell_lots(
                &core.region_graph,
                bounds,
                &[CellLotSize {
                    profile: 1,
                    width: 2,
                    depth: 2,
                }],
                |_, _| false,
            );
            let id = lots.created[0];
            let parcel = core.zoning.parcels.get(id).unwrap().clone();
            let lot = parcel.cell_lot().unwrap();
            let mut building =
                test_building("building.curved_road_removed", parcel.center().x, 0.0);
            building.center_y = parcel.center().y;
            building.facing_dir = -parcel.normal();
            building.parcel_id = id.raw();
            building.edge_idx = parcel.edge_idx();
            building.side = parcel.side();
            building.frontage_t = parcel.frontage_center_t();
            let pose = (building.center_x, building.center_y, building.facing_dir);
            core.allocator.buildings.push(building);
            assert!(core.zoning.occupy_parcel(id.raw(), 0));
            core.allocator
                .rebuild_building_site_clients(core.config.zone_cell_m);
            core.allocator.rebuild_zone_index();
            if erased {
                let selection = core.zoning.cells.select(
                    CellSelectionShape::Brush { radius_m: 200.0 },
                    &[DVec2::ZERO],
                );
                core.zoning.paint_cells(&selection, 0, |_| false).unwrap();
            }
            let paint = core.zoning.cells.saved_cells();
            assert!(core.bulldoze_road_edge(0));
            assert!(core.region_graph.edge(0).deleted);
            for undo in [false, true] {
                if undo {
                    assert!(core.undo_action_internal(), "angle={angle} erased={erased}");
                    core.rebuild_network_surface_terrain_internal();
                }
                let parcel = core
                    .zoning
                    .parcels
                    .get(id)
                    .expect("occupied lot stays reserved");
                assert_eq!(parcel.occupied_building(), Some(0));
                assert_eq!(parcel.cell_lot(), Some(lot));
                assert!(
                    lot.cells()
                        .all(|key| core.zoning.cells.lot(key) == Some(id.raw()))
                );
                assert_eq!(core.zoning.cells.saved_cells(), paint);
                let building = &core.allocator.buildings[0];
                assert_eq!(
                    (building.center_x, building.center_y, building.facing_dir),
                    pose
                );
            }
            assert!(core.bulldoze_road_edge(0));
            core.run_building_allocator_maintenance_internal();
            assert!(core.allocator.buildings.is_empty());
            core.prepare_cell_lots_internal();
            assert!(core.zoning.parcels.get(id).is_none());
            assert!(
                lot.cells()
                    .all(|key| core.zoning.cells.lot(key).is_none_or(|id| id == 0))
            );
        }
    }

    #[test]
    fn building_bulldoze_undo_restores_swap_removed_building_and_site() {
        let mut core = test_core();
        core.allocator.buildings = vec![
            test_building("building.removed", -20.0, 3.0),
            test_building("building.moved", 20.0, 7.0),
        ];
        let start = core
            .region_graph
            .add_node(Vector3::new(-60.0, 0.0, 0.0), NodeType::Junction);
        let end = core
            .region_graph
            .add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
        let edge = add_test_road_edge(&mut core.region_graph, start, end);
        let mut parcel_ids = Vec::new();
        for (idx, x) in [-20.0, 20.0].into_iter().enumerate() {
            let id = core
                .zoning
                .place_or_rezone_default_parcel_at(x, -20.0, 0, &core.region_graph)
                .unwrap();
            let parcel = core.zoning.parcel_by_raw_id(id.raw()).unwrap();
            let building = &mut core.allocator.buildings[idx];
            building.parcel_id = id.raw();
            building.edge_idx = edge;
            building.frontage_t = parcel.frontage_center_t();
            building.center_y = parcel.center().y;
            assert!(core.zoning.occupy_parcel(id.raw(), idx));
            parcel_ids.push(id.raw());
        }
        core.allocator
            .rebuild_building_site_clients(core.zoning.config.zone_cell_m);
        core.allocator.rebuild_zone_index();
        let original_sites = core.allocator.building_sites.clone();
        // The removed building is still a construction site.
        let complete_slot =
            core.allocator.now_slot + 4 * crate::simulation::core::scheduler::SLOTS_PER_HOUR;
        core.allocator.buildings[0].construction_total_hours = 4;
        core.allocator.buildings[0].construction_complete_slot = complete_slot;
        core.allocator.building_handles.register_appended(2);
        let removed = core.allocator.building_handles.handle(0);
        let moved = core.allocator.building_handles.handle(1);

        assert!(core.bulldoze_building(0));
        assert_eq!(core.allocator.buildings.len(), 1);
        assert_eq!(core.allocator.buildings[0].asset_id, "building.moved");
        assert_eq!(
            core.zoning
                .parcel_by_raw_id(parcel_ids[0])
                .unwrap()
                .occupied_building(),
            None
        );
        assert_eq!(
            core.zoning
                .parcel_by_raw_id(parcel_ids[1])
                .unwrap()
                .occupied_building(),
            Some(0)
        );

        assert!(core.undo_action_internal());
        assert_eq!(core.allocator.buildings.len(), 2);
        assert_eq!(core.allocator.buildings[0].asset_id, "building.removed");
        assert_eq!(core.allocator.buildings[1].asset_id, "building.moved");
        // The moved building keeps its handle; the restored one takes a new handle and lists
        // its construction deadline again.
        let handles = &core.allocator.building_handles;
        assert_eq!(handles.resolve(moved), Some(1));
        assert_eq!(handles.resolve(removed), None);
        assert_eq!(handles.resolve(handles.handle(0)), Some(0));
        assert_eq!(
            core.allocator.pending_construction,
            [(handles.handle(0), complete_slot)]
        );
        for (idx, id) in parcel_ids.into_iter().enumerate() {
            assert_eq!(
                core.zoning
                    .parcel_by_raw_id(id)
                    .unwrap()
                    .occupied_building(),
                Some(idx)
            );
        }
        assert_eq!(core.allocator.building_sites.len(), original_sites.len());
        for (restored, original) in core.allocator.building_sites.iter().zip(&original_sites) {
            assert_eq!(restored.footprint_world, original.footprint_world);
            assert_eq!(restored.lot_footprint_world, original.lot_footprint_world);
            assert_eq!(restored.support_height_m, original.support_height_m);
        }
    }

    #[test]
    fn building_bulldoze_undo_reverses_only_its_city_freight_refund() {
        use crate::assets::AssetManifest;
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;
        use crate::simulation::economy::logistics::{
            CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus,
        };

        for removed in [0, 1] {
            let mut core = test_core();
            let manifest: AssetManifest = serde_json::from_value(serde_json::json!({
                "asset_id": "water", "display_name": "Water plant",
                "building": {
                    "placement_mode": "explicit", "lot_width_cells": 2, "lot_depth_cells": 2,
                    "service_class": "water", "economy_profile": "water_plant_basic"
                },
                "mesh_parts": [{"name": "main", "lods": [{"file": "lod0.glb", "distance_min_m": 0.0}]}],
                "anchors": [{"type": "entrance", "name": "main", "position": [0.0, 0.0, 0.5], "forward": [0.0, 0.0, 1.0]}]
            })).unwrap();
            manifest.validate().unwrap();
            core.allocator
                .registry
                .register("test", manifest, String::new());
            let catalog = load_runtime_economy_catalog().unwrap();
            let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
            let mut water = test_building("test:water", 20.0, 7.0);
            water.zone_type = ZoneType::None;
            water.economy_profile_runtime_id = catalog
                .profile_for_id("water_plant_basic")
                .unwrap()
                .runtime_id;
            water.daily_city_funded_input_cost = 800.0;
            core.allocator.buildings = vec![test_building("supplier", -20.0, 3.0), water];
            core.allocator
                .rebuild_building_site_clients(core.config.zone_cell_m);
            core.allocator.rebuild_zone_index();
            core.treasury.balance = 1_200.0;
            core.logistics.shipments.push(Shipment {
                id: 0,
                resource_runtime_id: machinery,
                amount: 40.0,
                source: ShipmentEndpoint::Building(0),
                destination: ShipmentEndpoint::Building(1),
                carrier_class: CarrierClass::Truck,
                status: ShipmentStatus::InTransit,
                carrier: crate::simulation::economy::agents::AgentHandle::NONE,
                total_cost: 800.0,
                freight_cost: 0.0,
                eta_hours: 1,
                leg_started_hour: 0,
                trip_event_slot: 0,
                cargo_loaded: false,
            });

            assert!(core.bulldoze_building(removed));
            assert_eq!(core.treasury.balance, 2_000.0);
            assert!(core.logistics.shipments.is_empty());
            core.treasury.balance += 9.0;

            assert!(core.undo_action_internal());
            assert_eq!(core.treasury.balance, 1_209.0);
            assert_eq!(core.allocator.buildings[1].asset_id, "test:water");
            assert_eq!(
                core.allocator.buildings[1].daily_city_funded_input_cost,
                800.0
            );
            assert_eq!(core.logistics.shipments.len(), 1);
            assert_eq!(
                core.logistics.shipments[0].destination,
                ShipmentEndpoint::Building(1)
            );
        }
    }

    #[test]
    fn building_bulldoze_undo_rebinds_the_restored_carrier_handle() {
        use crate::simulation::economy::agents::VEHICLE_FREIGHT_DELIVERY;
        use crate::simulation::economy::logistics::{
            CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus,
        };

        let mut core = test_core();
        core.allocator.buildings = vec![
            test_building("supplier", -20.0, 3.0),
            test_building("buyer", 20.0, 7.0),
        ];
        core.allocator
            .rebuild_building_site_clients(core.config.zone_cell_m);
        core.allocator.rebuild_zone_index();
        let carrier = core.agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        core.agents.vehicle_type[carrier] = VEHICLE_FREIGHT_DELIVERY;
        core.agents.freight_shipment_id[carrier] = 0;
        // Removing the carrier moves this agent into its slot; undo moves it to the end.
        let other = core.agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        let other_handle = core.agents.agent_handle(other);
        let other_seed = core.agents.schedule_seed[other];
        let carrier_handle = core.agents.agent_handle(carrier);
        core.logistics.shipments.push(Shipment {
            id: 0,
            resource_runtime_id: 1,
            amount: 40.0,
            source: ShipmentEndpoint::Building(0),
            destination: ShipmentEndpoint::Building(1),
            carrier_class: CarrierClass::Truck,
            status: ShipmentStatus::InTransit,
            carrier: carrier_handle,
            total_cost: 0.0,
            freight_cost: 0.0,
            eta_hours: 1,
            leg_started_hour: 0,
            trip_event_slot: 0,
            cargo_loaded: true,
        });

        assert!(core.bulldoze_building(1));
        assert_eq!(core.agents.len(), 1);
        assert!(core.undo_action_internal());
        // The removed shipment's checks found nothing; undo checks the carrier again at the
        // next slot (`SIM-02-J`). A freight check saves as kind 3 with the shipment id.
        let next_slot = core.time.absolute_slot() + 1;
        assert!(
            core.events
                .pending()
                .iter()
                .any(|timed| timed.slot == next_slot && timed.event.to_saved() == (3, 0))
        );
        assert!(core.logistics.trip_event_due(0, next_slot));

        let restored = core
            .agents
            .resolve_agent(core.logistics.shipments[0].carrier)
            .expect("undo rebinds the shipment to its restored carrier");
        assert_eq!(restored, carrier);
        assert_eq!(core.agents.freight_shipment_id[restored], 0);
        assert_eq!(core.agents.resolve_agent(carrier_handle), None);
        let moved = core.agents.resolve_agent(other_handle).unwrap();
        assert_eq!(core.agents.schedule_seed[moved], other_seed);
    }

    #[test]
    fn bulldoze_revalidates_offset_building_footprint() {
        let mut core = test_core();
        core.allocator.buildings = vec![test_building("offset", -20.0, 3.0)];
        core.allocator
            .rebuild_building_site_clients(core.config.zone_cell_m);
        let site = &mut core.allocator.building_sites[0];
        for point in &mut site.footprint_world {
            *point += Vector2::new(0.0, 40.0);
        }
        let point = site.footprint_world.iter().copied().sum::<Vector2>()
            / site.footprint_world.len() as f32;
        core.allocator.recompute_max_site_radius_m();
        core.allocator.reindex_site_clients();
        core.allocator.rebuild_zone_index();
        let target = core.resolve_bulldoze_target(point.x, point.y).unwrap();
        assert_eq!(Vector2::new(target.center.x, target.center.z), point);
        assert_eq!(core.bulldoze_prepared_target_internal(target), Some(false));
        assert!(core.undo_action_internal());
    }

    #[test]
    fn bulldoze_vegetation_matches_brush_clear_and_preserves_neighbours() {
        use crate::nodes::simulation_node::vegetation_api;
        use crate::simulation::vegetation::edits::{
            AuthoredPlant, VegetationCell, VegetationLayer,
        };

        let mut core = test_core();
        core.heightmap = TerrainSystem::with_chunking(257, 257, 10.0, 65, 20.0);
        core.vegetation.config.enabled = false;
        core.prepare_bulldoze_target_indices();
        let cell = VegetationCell {
            layer: VegetationLayer::Canopy,
            x: 0,
            z: 0,
        };
        for species in 0..=3 {
            let plant = AuthoredPlant {
                x: 1.0,
                z: 1.0,
                yaw: 0.0,
                scale: 1.0,
                species,
                variant: 0,
            };
            let neighbour = AuthoredPlant { x: 1.005, ..plant };
            // Insert the neighbour first: clearing it later compacts the target's index.
            core.vegetation_edits.add(cell, neighbour);
            core.vegetation_edits.add(cell, plant);
            let target = core.resolve_bulldoze_target(plant.x, plant.z).unwrap();
            assert_eq!(target.kind, BulldozeTargetKind::Vegetation);
            assert_eq!(target.kind.as_str(), "vegetation");
            assert_eq!(target.points.len(), 16);
            assert!(
                target
                    .points
                    .iter()
                    .all(|p| p.is_finite() && p.y > 20.0 * crate::config::HEIGHT_SCALE)
            );
            let radius = if species == 0 {
                3.0
            } else if species == 1 {
                3.5
            } else {
                1.0
            };
            assert!((target.points[0].x - target.center.x - radius).abs() < 0.001);
            let undo_depth = core.undo_stack.len();
            assert_eq!(core.bulldoze_prepared_target_internal(target), Some(false));
            assert_eq!(core.undo_stack.len(), undo_depth + 1);
            assert_eq!(core.vegetation_edits.cell(cell).1, &[neighbour]);
            let bulldozed = core.vegetation_edits.snapshot_cell(cell);
            assert!(core.undo_action_internal());
            assert_eq!(
                vegetation_api::remove_at(&mut core, Vector2::new(plant.x, plant.z), 0.0, 0),
                1
            );
            assert_eq!(core.vegetation_edits.snapshot_cell(cell), bulldozed);
            assert!(core.undo_action_internal());
            let target = core.resolve_bulldoze_target(plant.x, plant.z).unwrap();
            assert_eq!(
                vegetation_api::remove_at(
                    &mut core,
                    Vector2::new(neighbour.x, neighbour.z),
                    0.0,
                    0
                ),
                1
            );
            let resolved = core
                .resolve_bulldoze_target(target.center.x, target.center.z)
                .unwrap();
            assert_eq!(resolved.id, target.id);
            assert_eq!(core.bulldoze_prepared_target_internal(target), Some(false));
            assert!(core.vegetation_edits.cell(cell).1.is_empty());
        }
    }

    #[test]
    fn bulldoze_coincident_classes_removes_one_and_rejects_stale_target() {
        use crate::simulation::vegetation::edits::{
            AuthoredPlant, VegetationCell, VegetationLayer,
        };
        let mut core = test_core();
        core.vegetation.config.enabled = false;
        core.prepare_bulldoze_target_indices();
        let cell = VegetationCell {
            layer: VegetationLayer::Canopy,
            x: 0,
            z: 0,
        };
        for species in [2, 0] {
            core.vegetation_edits.add(
                cell,
                AuthoredPlant {
                    x: 1.0,
                    z: 1.0,
                    yaw: 0.0,
                    scale: 1.0,
                    species,
                    variant: 0,
                },
            );
        }
        let target = core.resolve_bulldoze_target(1.0, 1.0).unwrap();
        assert_eq!(
            core.bulldoze_prepared_target_internal(target.clone()),
            Some(false)
        );
        assert_eq!(core.vegetation_edits.cell(cell).1.len(), 1);
        assert_eq!(core.bulldoze_prepared_target_internal(target), None);
        assert!(core.undo_action_internal());
        assert_eq!(core.vegetation_edits.cell(cell).1.len(), 2);
    }

    #[test]
    fn bulldoze_generated_vegetation_writes_the_brush_tombstone() {
        use crate::nodes::simulation_node::vegetation_api;
        use crate::simulation::vegetation::edits::{VegetationCell, VegetationLayer};

        let mut core = test_core();
        core.heightmap = TerrainSystem::with_chunking(257, 257, 10.0, 65, 20.0);
        core.prepare_bulldoze_target_indices();
        let (plant, _) = (-16..=16)
            .find_map(|i| vegetation_api::plant_at(&core, Vector2::new(i as f32 * 4.0, 0.0)))
            .expect("generated vegetation on flat ground");
        let plant = plant.plant;
        let pos = Vector2::new(plant.x, plant.z);
        let target = core.resolve_bulldoze_target(pos.x, pos.y).unwrap();
        assert_eq!(target.kind, BulldozeTargetKind::Vegetation);
        assert_eq!(core.bulldoze_prepared_target_internal(target), Some(false));
        let cells = core.vegetation_edits.sorted_cells();
        assert_eq!(cells.len(), 1);
        let cell = cells[0];
        let cell_m = match cell.layer {
            VegetationLayer::Canopy => core.vegetation.canopy_cell_m,
            VegetationLayer::Understory => core.vegetation.understory_cell_m,
        };
        assert_eq!(
            cell,
            VegetationCell {
                layer: cell.layer,
                x: (pos.x / cell_m).floor() as i32,
                z: (pos.y / cell_m).floor() as i32,
            }
        );
        assert!(core.vegetation_edits.cell(cell).0);
        let bulldozed = core.vegetation_edits.snapshot_cell(cell);
        assert!(core.undo_action_internal());
        assert!(!core.vegetation_edits.cell(cell).0);
        assert_eq!(vegetation_api::remove_at(&mut core, pos, 0.0, 0), 1);
        assert_eq!(core.vegetation_edits.snapshot_cell(cell), bulldozed);
    }

    #[test]
    fn move_network_node_undo_restores_pre_move_graph_state() {
        let mut core = test_core();
        let n0 = core
            .region_graph
            .add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
        let n1 = core
            .region_graph
            .add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        core.region_graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 120.0,
            physical_length: 20.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
        core.region_graph.rebuild_adjacency_list();

        core.zoning
            .mark_cell_lots_for_roads(&core.region_graph, [0]);
        let frames = core.zoning.cells.saved_road_alignments();
        assert_eq!(frames.len(), 1);
        let revision = core.zoning.cell_external_revision();
        core.move_network_node_internal(n0 as i32, Vector3::new(-20.0, 0.0, 0.0));
        assert_eq!(core.region_graph.node(n0).pos.x, -20.0);
        assert_ne!(core.zoning.cells.saved_road_alignments(), frames);
        assert!(core.zoning.cell_external_revision() > revision);

        assert!(core.undo_action_internal());
        assert_eq!(
            core.region_graph.node(n0).pos,
            Vector3::new(-10.0, 0.0, 0.0)
        );
        assert_eq!(
            core.region_graph.edge(0).physical_geometry[0],
            Vector3::new(-10.0, 0.0, 0.0)
        );
        assert_eq!(core.zoning.cells.saved_road_alignments(), frames);
    }

    #[test]
    fn border_extension_keeps_the_road_grid_alignment_loadable() {
        let mut core = test_core();
        let n0 = core
            .region_graph
            .add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
        let n1 = core
            .region_graph
            .add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        core.region_graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 120.0,
            physical_length: 20.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
        core.region_graph.rebuild_adjacency_list();
        core.zoning
            .mark_cell_lots_for_roads(&core.region_graph, [0]);

        // The border pushes the road's end outward; the saved alignment must describe the
        // extended road, or the save is rejected on load.
        core.set_border_connection_internal(n0 as i32);
        let path = std::env::temp_dir().join(format!(
            "metrum_border_alignment_{}.sqlite",
            std::process::id()
        ));
        // The extension's routing rebuild is pending (D7); the save installs it first.
        core.save_game_settled(path.to_str().unwrap(), None)
            .unwrap();
        let mut loaded = test_core();
        let result = loaded.load_game_internal(path.to_str().unwrap());
        std::fs::remove_file(&path).unwrap();
        result.unwrap();
        assert_eq!(
            loaded.zoning.cells.saved_road_alignments(),
            core.zoning.cells.saved_road_alignments()
        );
    }

    #[test]
    fn terrain_stroke_batching_pushes_one_undo_snapshot_and_finalizes_on_end() {
        let mut core = test_core();
        core.start_terrain_stroke_internal();
        core.sculpt_terrain_stroke_step_internal(Vector2::new(0.0, 0.0), 15.0, 0.5);
        core.sculpt_terrain_stroke_step_internal(Vector2::new(2.0, 1.0), 15.0, 0.5);

        assert_eq!(core.undo_stack.len(), 1);
        assert!(core.terrain_stroke_active);
        assert!(core.terrain_stroke_has_changes);
        assert!(core.terrain_dirty);

        assert!(core.end_terrain_stroke_internal());
        assert!(!core.terrain_stroke_active);
        assert!(!core.terrain_stroke_has_changes);
    }

    #[test]
    fn terrain_authoring_payload_bounds_include_patch_texture_border() {
        let mut core = test_core();
        let (_, min_z, max_x, max_z) = core
            .heightmap
            .render_patch_world_bounds(0, 0)
            .expect("default test terrain should have patch (0,0)");
        let pos = Vector2::new(max_x - 20.0, (min_z + max_z) * 0.5);
        let neighbor_generation_before = core.terrain_payload_generation_for_patch(1, 0);

        core.start_terrain_stroke_internal();
        core.sculpt_terrain_stroke_step_internal(pos, 1.0, 0.5);

        assert!(core.heightmap.dirty_render_patches().contains(&(0, 0)));
        assert!(
            core.heightmap.dirty_render_patches().contains(&(1, 0)),
            "neighbor patch border ring must be refreshed when nearby samples change"
        );
        assert!(
            core.terrain_payload_generation_for_patch(1, 0) > neighbor_generation_before,
            "neighbor patch payload generation must also advance"
        );
    }

    #[test]
    fn toggling_a_crossing_moves_its_mouth_and_lanes_follow_the_node_piece() {
        use crate::simulation::network::roadbed::{ArmKey, EdgeEnd};
        let mut core = test_core();
        core.treasury.balance = 10_000.0;
        for points in [
            vec![
                Vector3::new(-100.0, 0.0, 0.0),
                Vector3::new(100.0, 0.0, 0.0),
            ],
            vec![Vector3::ZERO, Vector3::new(0.0, 0.0, 100.0)],
        ] {
            assert!(core.add_road_internal(points, 1, 1).committed);
        }
        core.precompute_road_mesh_data();
        let graph = &core.region_graph;
        let live = |node: u32| {
            graph
                .node_adjacency(node)
                .iter()
                .copied()
                .filter(|&edge| !graph.edge(edge).deleted)
                .collect::<Vec<_>>()
        };
        let node = (0..graph.node_count() as u32)
            .find(|&node| graph.get_valid_node(node) == node && live(node).len() == 3)
            .expect("T junction");
        let edge = live(node)[0];
        let key = ArmKey {
            edge,
            end: if graph.get_valid_node(graph.edge(edge).start_node) == node {
                EdgeEnd::Start
            } else {
                EdgeEnd::End
            },
        };
        let clip = |core: &SimCore| {
            let arm = core.region_graph.edge(edge);
            match key.end {
                EdgeEnd::Start => arm.start_clip,
                EdgeEnd::End => arm.end_clip,
            }
        };
        // Lanes publish exactly the node piece's crossings, and only those.
        let agree = |core: &SimCore| {
            let piece = core.transit_network.roadbed.node_piece(node).unwrap();
            let lanes = &core.transit_network.lane_system;
            let mut marked: Vec<_> = lanes.node_lanes[&(node as usize)]
                .iter()
                .filter_map(|&lane| lanes.crosswalk_marking(lane))
                .map(|marking| {
                    let mut kerbs =
                        [marking.start, marking.end].map(|p| p.to_array().map(f32::to_bits));
                    kerbs.sort_unstable();
                    (marking.edge_id, kerbs)
                })
                .collect();
            marked.sort_unstable();
            let mut built: Vec<_> = piece
                .layout()
                .arms
                .iter()
                .filter_map(|arm| {
                    let mut kerbs = piece
                        .crosswalk_kerbs(arm.key)?
                        .map(|p| p.to_array().map(f32::to_bits));
                    kerbs.sort_unstable();
                    Some((arm.key.edge, kerbs))
                })
                .collect();
            built.sort_unstable();
            assert_eq!(marked, built);
            piece.crosswalk_kerbs(key).is_some()
        };
        assert!(core.transit_network.roadbed.failed_nodes().is_empty());
        assert!(agree(&core));
        let crossed = clip(&core);

        core.set_crosswalk_override_internal(node, edge as i32, false);
        core.precompute_road_mesh_data();
        assert!(core.transit_network.roadbed.failed_nodes().is_empty());
        assert!(!agree(&core));
        assert!(
            (crossed - clip(&core) - 3.0).abs() < 1e-4,
            "the mouth moves in 3 m"
        );

        core.set_crosswalk_override_internal(node, edge as i32, true);
        core.precompute_road_mesh_data();
        assert!(core.transit_network.roadbed.failed_nodes().is_empty());
        assert!(agree(&core));
        assert!((clip(&core) - crossed).abs() < 1e-4);
    }

    fn finalize_network_render_for_test(core: &mut SimCore) {
        core.region_graph.rebuild_intersection_clips();
        core.transit_network
            .lane_system
            .rebuild(&mut core.region_graph);
        core.transit_network
            .rebuild_cch_and_check(&core.region_graph);
        core.rebuild_network_surface_terrain_internal();
        core.precompute_road_mesh_data();
        let cache_inputs =
            core.collect_refined_terrain_patch_build_inputs(ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
        let cache_entries = SimCore::build_refined_terrain_patch_cache_entries(cache_inputs);
        core.insert_refined_terrain_patch_cache_entries(cache_entries);
    }

    // `TERRAIN-07-E`: the commit stages its graded tiles, and its undo record keeps the ones it
    // replaced, before lanes publish.
    #[test]
    fn accepted_road_has_its_graded_tiles_before_publication() {
        let mut core = test_core();
        core.benchmark_mode = false;
        core.transit_network.begin_road_edit();
        core.transit_network.bulk_load = true;
        assert!(
            core.add_road_internal(
                vec![Vector3::new(-40.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)],
                1,
                1,
            )
            .committed
        );
        core.transit_network.bulk_load = false;
        core.region_graph.rebuild_intersection_clips();
        assert!(
            core.validate_staged_road_render(),
            "{}",
            core.last_road_timing
        );
        let tiles = core.last_graded_tiles.as_ref().expect("the tiles bake");
        assert!(!tiles.is_empty() && tiles.tiles().iter().all(|held| held.payload.graded));
        assert_eq!(core.undo_stack.len(), 1);
        assert!(core.undo_stack[0].graded.is_some());
        assert!(!core.transit_network.road_edit_is_staged());
        assert!(
            core.transit_network.lane_system.lanes.is_empty(),
            "render validation precedes lane publication"
        );
    }

    // Since `TERRAIN-07-E` no terrain failure refuses a road: a field reserved under the new
    // road after its topology mutation is the refusal.
    #[test]
    fn rejected_commit_restores_split_dependents_and_preserves_undo_history() {
        use crate::simulation::buildings::allocator::EdgeOccupancy;
        let mut core = test_core();
        core.benchmark_mode = false;
        core.treasury.balance = 10_000.0;
        assert!(
            core.add_road_internal(
                vec![
                    Vector3::new(-100.0, 0.0, 0.0),
                    Vector3::new(100.0, 0.0, 0.0)
                ],
                1,
                1
            )
            .committed
        );
        finalize_network_render_for_test(&mut core);
        let original_edge = core.region_graph.edge(0).clone();
        let original_cell_frames = core.zoning.cells.saved_road_alignments();
        assert_eq!(original_cell_frames.len(), 1);
        let mut building = test_building("test", 70.0, 0.0);
        building.edge_idx = 0;
        building.cell_x = 20;
        building.frontage_t = 0.85;
        core.allocator.buildings.push(building);
        let original_building = core.allocator.buildings[0].clone();
        core.allocator.edge_occupancy.insert(
            0,
            EdgeOccupancy {
                cells_long: 32,
                left: vec![false; 32],
                right: vec![true; 32],
            },
        );
        for _ in 1..30 {
            core.push_network_undo_for_polyline(&[], 0.0);
        }
        let lane_count = core.transit_network.lane_system.lanes.len();
        core.transit_network.begin_road_edit();
        core.transit_network.bulk_load = true;
        assert!(
            core.add_road_internal(
                vec![Vector3::new(0.0, 0.0, -80.0), Vector3::new(0.0, 0.0, 0.0)],
                1,
                1
            )
            .committed
        );
        assert_eq!(core.undo_stack.len(), 31);
        assert_ne!(core.allocator.buildings[0].edge_idx, 0);
        core.transit_network.bulk_load = false;
        core.region_graph.rebuild_intersection_clips();
        core.allocator.field_clearance_mut().set(
            0,
            &[
                Vector2::new(-0.5, -40.5),
                Vector2::new(0.5, -40.5),
                Vector2::new(0.5, -39.5),
                Vector2::new(-0.5, -39.5),
            ],
        );
        assert!(!core.validate_staged_road_render());
        assert!(core.last_road_timing.contains("field_overlap"));
        assert_eq!(core.undo_stack.len(), 30);
        assert_eq!(core.region_graph.edge_count(), 1);
        assert_eq!(
            core.zoning.cells.saved_road_alignments(),
            original_cell_frames
        );
        assert_eq!(
            core.region_graph.edge(0).physical_geometry,
            original_edge.physical_geometry
        );
        let restored = &core.allocator.buildings[0];
        assert_eq!(restored.edge_idx, original_building.edge_idx);
        assert_eq!(restored.cell_x, original_building.cell_x);
        assert_eq!(restored.frontage_t, original_building.frontage_t);
        assert_eq!(core.allocator.edge_occupancy.len(), 1);
        assert_eq!(core.allocator.edge_occupancy[&0].cells_long, 32);
        assert_eq!(core.allocator.edge_occupancy[&0].right, vec![true; 32]);
        assert_eq!(core.transit_network.lane_system.lanes.len(), lane_count);
        assert_eq!(core.treasury.balance, 10_000.0);
        assert_eq!(core.treasury.lifetime_build_cost, 0.0);
        assert!(!core.transit_network.road_edit_is_staged());
    }

    #[test]
    fn undoing_road_restore_invalidates_road_locked_terrain_state() {
        let mut core = test_core();
        core.benchmark_mode = false;
        core.add_road_internal(
            vec![Vector3::new(-40.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)],
            1,
            1,
        );
        assert_eq!(core.region_graph.edge_count(), 1);

        finalize_network_render_for_test(&mut core);
        let previous_mesh = core.cached_road_mesh_chunks.clone();
        assert!(!previous_mesh.is_empty());

        let stale_road_locked_patches = core.road_locked_terrain_patch_keys.clone();
        assert!(!stale_road_locked_patches.is_empty());
        assert!(!core.refined_terrain_patch_cache.is_empty());
        let dirty_keys = core
            .heightmap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for (patch_x, patch_z) in dirty_keys {
            core.heightmap.clear_render_patch_dirty(patch_x, patch_z);
        }

        assert!(core.undo_action_internal());
        assert_eq!(core.region_graph.edge_count(), 0);
        assert!(
            !core.refined_terrain_patch_cache.is_empty(),
            "bounded road undo must retain the immutable previous terrain generation"
        );
        assert!(
            !core.transit_network.roadbed.is_current(),
            "removed road coverage must remain queued for local cleanup"
        );
        assert_eq!(core.cached_road_mesh_chunks.len(), previous_mesh.len());
        assert!(previous_mesh.iter().all(|(chunk, mesh)| {
            Arc::ptr_eq(
                core.cached_road_mesh_chunks
                    .get(chunk)
                    .expect("undo invalidation must retain the cached chunk"),
                mesh,
            )
        }));
        assert!(core.terrain_dirty);
        assert!(core.network_dirty);

        core.rebuild_network_surface_terrain_internal();
        for key in &stale_road_locked_patches {
            assert!(
                core.heightmap.dirty_render_patches().contains(key),
                "former road-locked patch {key:?} must be re-uploaded after undo"
            );
        }

        let cache_inputs =
            core.collect_refined_terrain_patch_build_inputs(ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
        assert!(cache_inputs.is_empty());
        assert!(core.road_locked_terrain_patch_keys.is_empty());
        assert!(
            core.refined_terrain_patch_cache.is_empty(),
            "ownership refresh may discard a previous patch only after no engineered owner remains"
        );
    }

    #[test]
    fn undoing_road_restore_invalidates_derived_render_cache() {
        let mut core = test_core();
        core.benchmark_mode = false;
        core.add_road_internal(
            vec![
                Vector3::new(-40.0, 0.0, 32.0),
                Vector3::new(40.0, 0.0, 32.0),
            ],
            1,
            1,
        );
        finalize_network_render_for_test(&mut core);
        assert!(!core.road_locked_terrain_patch_keys.is_empty());
        assert!(!core.refined_terrain_patch_cache.is_empty());

        core.add_road_internal(
            vec![
                Vector3::new(-40.0, 0.0, 160.0),
                Vector3::new(40.0, 0.0, 160.0),
            ],
            1,
            1,
        );
        finalize_network_render_for_test(&mut core);
        assert_eq!(core.region_graph.edge_count(), 2);
        for key in &core.road_locked_terrain_patch_keys {
            let ledger = core
                .refined_terrain_assembly_ledgers
                .get(key)
                .expect("road-owned dirty patches must retain their local assembly scope");
            assert!(
                ledger.full_dirty_at.is_none(),
                "road-only ownership refresh must not replace exact query chunks with full scope"
            );
            assert!(!ledger.road_tile_dirty_at.is_empty());
        }
        let previous_windows = core
            .refined_terrain_patch_cache
            .values()
            .flat_map(|patch| patch.windows.iter().cloned())
            .collect::<Vec<_>>();
        let previous_mesh = core.cached_road_mesh_chunks.clone();
        assert!(!previous_mesh.is_empty());
        let global_generation_before_undo = core.terrain_payload_global_generation;

        assert!(core.undo_action_internal());
        assert_eq!(core.region_graph.edge_count(), 1);
        assert_eq!(
            core.terrain_payload_global_generation, global_generation_before_undo,
            "bounded road undo must preserve patch-local generation semantics"
        );
        assert!(
            !core.refined_terrain_patch_cache.is_empty(),
            "undo must keep the complete post-edit generation as an immutable reuse source"
        );
        assert_eq!(core.cached_road_mesh_chunks.len(), previous_mesh.len());
        assert!(previous_mesh.iter().all(|(chunk, mesh)| {
            Arc::ptr_eq(
                core.cached_road_mesh_chunks
                    .get(chunk)
                    .expect("undo invalidation must retain the cached chunk"),
                mesh,
            )
        }));
        assert!(core.network_dirty);

        core.rebuild_network_surface_terrain_internal();
        let cache_inputs =
            core.collect_refined_terrain_patch_build_inputs(ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
        assert!(
            !cache_inputs.is_empty(),
            "undo must enqueue at least one locally revised refined patch"
        );
        let cache_entries = SimCore::build_refined_terrain_patch_cache_entries(cache_inputs);
        let entry_count = cache_entries.len();
        let entry_keys = cache_entries
            .iter()
            .map(|entry| entry.key)
            .collect::<Vec<_>>();
        assert!(
            cache_entries.iter().any(|entry| entry.reused_windows > 0),
            "undoing remote road coverage must reuse unchanged fixed terrain tiles"
        );
        assert!(
            cache_entries
                .iter()
                .flat_map(|entry| &entry.windows)
                .any(|window| {
                    previous_windows
                        .iter()
                        .any(|previous| Arc::ptr_eq(window, previous))
                }),
            "unchanged undo tiles must retain their previous-generation Arc identity"
        );
        assert_eq!(
            core.insert_refined_terrain_patch_cache_entries(cache_entries),
            entry_count,
            "every locally rebuilt undo patch must be complete, current, and publishable"
        );
        for key in entry_keys {
            let published = core
                .refined_terrain_patch_cache
                .get(&key)
                .expect("publishable undo generation must replace its cache entry");
            assert_eq!(
                published.surface_generation,
                core.terrain_payload_generation_for_patch(key.patch_x, key.patch_z)
            );
        }
    }

    #[test]
    fn undoing_fourth_junction_mouth_restores_exact_pre_edit_surface_topology() {
        let mut core = test_core();
        let center = core
            .region_graph
            .add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        for endpoint in [
            Vector3::new(-36.0, 0.0, 0.0),
            Vector3::new(36.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 36.0),
        ] {
            let endpoint = core.region_graph.add_node(endpoint, NodeType::Junction);
            add_test_road_edge(&mut core.region_graph, center, endpoint);
        }
        core.region_graph.rebuild_adjacency_list();
        finalize_network_render_for_test(&mut core);
        let mesh_vertices = |core: &SimCore| {
            let layer = |vertices: fn(&NetworkMeshData) -> &Vec<Vector3>| {
                core.cached_road_mesh_chunks
                    .values()
                    .flat_map(|mesh| vertices(mesh).iter().copied())
                    .collect::<Vec<_>>()
            };
            [
                layer(|mesh| &mesh.skirt_vertices),
                layer(|mesh| &mesh.curb_vertices),
                layer(|mesh| &mesh.raised_step_vertices),
                layer(|mesh| &mesh.sidewalk_vertices),
                layer(|mesh| &mesh.road_vertices),
                layer(|mesh| &mesh.marking_vertices),
                layer(|mesh| &mesh.concrete_vertices),
            ]
        };
        let occupied_chunks = |core: &SimCore| {
            core.transit_network
                .roadbed
                .occupied_chunks()
                .collect::<Vec<_>>()
        };
        let baseline_mesh_vertices = mesh_vertices(&core);
        let baseline_chunks = occupied_chunks(&core);
        assert!(
            !baseline_chunks.is_empty(),
            "the three-mouth JunctionN must publish pieces"
        );

        core.benchmark_mode = false;
        let appended_node = core.region_graph.node_count() as u32;
        let appended_edge = core.region_graph.edge_count();
        core.add_road_internal(
            vec![Vector3::new(0.0, 0.0, -36.0), Vector3::new(0.0, 0.0, 0.0)],
            1,
            1,
        );
        finalize_network_render_for_test(&mut core);
        assert_eq!(core.region_graph.node_count(), appended_node as usize + 1);
        assert_eq!(core.region_graph.edge_count(), 4);

        assert!(core.undo_action_internal());
        assert_eq!(core.region_graph.edge_count(), 3);
        core.rebuild_network_surface_terrain_internal();
        core.precompute_road_mesh_data();
        let roadbed = &core.transit_network.roadbed;
        assert!(roadbed.is_current());
        assert!(
            roadbed.sections(appended_edge).is_none()
                && roadbed
                    .pieces_in_bounds([f64::MIN, f64::MIN, f64::MAX, f64::MAX])
                    .all(|(piece, _)| {
                        piece != PieceId::Span(appended_edge)
                            && piece != PieceId::Node(appended_node)
                    }),
            "appended post-edit pieces must be removed"
        );
        assert_eq!(occupied_chunks(&core), baseline_chunks);
        assert_eq!(
            mesh_vertices(&core),
            baseline_mesh_vertices,
            "the final rendered mesh must contain exactly the baseline three-mouth geometry"
        );
    }

    #[test]
    fn network_undo_falls_back_to_owner_recompile_when_surface_capture_is_dirty() {
        let mut core = test_core();
        let start = core
            .region_graph
            .add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let end = core
            .region_graph
            .add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = add_test_road_edge(&mut core.region_graph, start, end);
        core.region_graph.rebuild_adjacency_list();
        finalize_network_render_for_test(&mut core);
        let before = core
            .transit_network
            .roadbed
            .sections(edge_idx)
            .unwrap()
            .to_vec();

        core.transit_network.roadbed.mark_edge_dirty(edge_idx);
        core.push_network_undo_for_local_topology(
            HashSet::from([edge_idx]),
            HashSet::from([start]),
        );

        core.region_graph
            .move_node(start, Vector3::new(-30.0, 0.0, 0.0));
        assert!(core.undo_action_internal());
        assert_eq!(
            core.region_graph.node(start).pos,
            Vector3::new(-24.0, 0.0, 0.0)
        );
        assert!(
            !core.transit_network.roadbed.is_current()
                && core.transit_network.roadbed.dirty_nodes().contains(&start),
            "undo must queue the bounded owner recompile"
        );
        core.transit_network
            .roadbed
            .compile_dirty(&core.region_graph);
        assert_eq!(
            core.transit_network.roadbed.sections(edge_idx),
            Some(before.as_slice()),
            "the owner recompile restores the span"
        );
    }

    #[test]
    fn finished_terrain_edit_publishes_matching_road_mesh_generation() {
        let mut core = test_core();
        let outcome = core.add_road_internal(
            vec![Vector3::new(-40.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)],
            1,
            1,
        );
        assert!(outcome.committed);
        finalize_network_render_for_test(&mut core);
        assert!(core.acknowledge_network_render_generation(core.road_tool_surface_generation));
        let baseline_generation = core.cached_road_mesh_generation;

        core.sculpt_terrain_internal(Vector2::ZERO, 12.0, 0.05);

        assert!(core.road_tool_surface_generation > baseline_generation);
        assert_eq!(
            core.cached_road_mesh_generation, core.road_tool_surface_generation,
            "terrain finalization must publish the road delta before its immediate snapshot"
        );
        assert!(!core.pending_road_mesh_chunks.is_empty());
        assert!(!core.published_road_mesh_chunks.is_empty());
    }
}
