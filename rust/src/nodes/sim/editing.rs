// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: editing.rs
//  script_path: rust/src/nodes/sim/editing.rs
//  module_name: editing
//  version: 0.1.0
//  description: All mutating operations the player can drive on the
//           simulation: road placement, terrain sculpt, zoning, edge
//           editing, and bulldoze. Split out of SimCore as an impl block
//           so the read-only query and snapshot paths stay in separate
//           files; every entry point here validates input before touching
//           the graph, then triggers a surface recompile, because a
//           half-applied edit leaves lanes and terrain disagreeing.
//  kind: module
//  spec: none
//  internal_dependencies: [core, surface, buildings]
//  external_dependencies: [godot]
//  features: [road-placement, terrain-sculpt, zoning, bulldoze, undo]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// =========================================================================

//! Logic for modifying simulation state (road placement, terrain sculpt, zoning, edge editing).

use crate::config;
use crate::debug_log;
use crate::nodes::sim::core::{
    ROAD_BUILD_COST_PER_METER, ROAD_LOCKED_TERRAIN_RENDER_STEP_M, SERVICE_BUILD_COST_PER_LOT_CELL,
    SimCore,
};
use crate::nodes::sim::road_tool::validate_road_candidate_against_water;
use crate::simulation::buildings::allocator::{
    ExplicitServicePlacementPreview, ExplicitServicePlacementRejection,
};
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::graph::{
    JunctionControl, LaneLayout, PrioritySign, SignalAspect, SignalPhase, SignalProgram,
};
use crate::simulation::network::surface::{
    RoadExtensionReprofile, RoadSurfaceCompileReason, RoadSurfaceSystem,
};
use crate::traffic_log;
use godot::prelude::*;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

const BULLDOZE_HIGHLIGHT_Y_OFFSET_M: f32 = 0.08;
const BULLDOZE_ROAD_PICK_RADIUS_M: f32 = 24.0;
const BULLDOZE_ROAD_PICK_MARGIN_M: f32 = 0.75;
const ROAD_UNDO_TOPOLOGY_MARGIN_M: f32 = 40.0;

/// Result of a road placement attempt after synchronous input validation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RoadAddOutcome {
    /// True when topology was mutated and the command must run road finalization.
    pub(crate) committed: bool,
    /// Deferred treasury charge for the committed physical road length.
    pub(crate) build_cost: f64,
}

impl RoadAddOutcome {
    fn rejected() -> Self {
        Self {
            committed: false,
            build_cost: 0.0,
        }
    }
}

impl SimCore {
    /// Rolls back the last road commit when its final surface compiler output cannot be published.
    pub(crate) fn rollback_unpublishable_road_commit(&mut self) -> bool {
        if self
            .transit_network
            .road_surface
            .published_generation_matches_source()
        {
            return false;
        }

        let failure_suffix = self
            .transit_network
            .road_surface
            .last_compile_failure_label()
            .map(|label| format!(" surface_failure={label}"))
            .unwrap_or_default();
        let rolled_back = !self.benchmark_mode && self.undo_action_internal();
        debug_log!(
            "road",
            "road_commit_rejected reason=surface_geometry_invalid_after_commit rollback={} graph_edges={}{}",
            rolled_back,
            self.region_graph.edge_count(),
            failure_suffix
        );
        self.last_road_timing =
            format!("rejected=surface_geometry_invalid_after_commit{failure_suffix}");
        if rolled_back {
            self.rebuild_network_surface_terrain_internal_with_entrance_rebuild(false);
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BulldozeTargetKind {
    Building,
    Road,
}

impl BulldozeTargetKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::Road => "road",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BulldozeTarget {
    kind: BulldozeTargetKind,
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
    const ROAD_GEOMETRY_DUMP_DEFAULT_MAX_BYTES: usize = 256 * 1024;

    fn road_geometry_dump_enabled() -> bool {
        std::env::var("METRUM_DEBUG_ROAD_GEOMETRY_DUMP")
            .map(|value| !value.is_empty() && value != "0")
            .unwrap_or(false)
    }

    fn road_geometry_dump_max_bytes() -> Option<usize> {
        if std::env::var("METRUM_DEBUG_ROAD_GEOMETRY_DUMP_FULL")
            .map(|value| !value.is_empty() && value != "0")
            .unwrap_or(false)
        {
            return None;
        }
        std::env::var("METRUM_DEBUG_ROAD_GEOMETRY_DUMP_MAX_BYTES")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .or(Some(Self::ROAD_GEOMETRY_DUMP_DEFAULT_MAX_BYTES))
    }

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
        self.terrain_dirty = true;
    }

    fn finish_terrain_authoring_edit_internal(&mut self) {
        self.terrain_dirty = true;
        self.mark_local_network_render_dirty();

        self.transit_network
            .sync_to_terrain(&mut self.region_graph, &self.heightmap);
        let road_surface_had_dirty_work =
            self.transit_network.road_surface.has_pending_rebuild_work();
        let dirty_chunks = self
            .transit_network
            .rebuild_dirty_terrain_earthworks_with_reason(
                &self.region_graph,
                &mut self.heightmap,
                RoadSurfaceCompileReason::TerrainEarthwork,
            );
        let dirty_patch_keys = self
            .transit_network
            .road_surface
            .mark_render_patches_for_chunk_grading_envelopes(
                &mut self.heightmap,
                &dirty_chunks,
                ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
            );
        if road_surface_had_dirty_work {
            self.bump_terrain_payload_patch_generations(&dirty_patch_keys);
        }
        // Terrain-authoring APIs publish their snapshot immediately rather than passing through the
        // simulation command finalizer. Build the matching road delta here so NetworkRenderer does
        // not repeatedly observe a newer source generation paired with the previous mesh.
        self.precompute_road_mesh_data();
        self.rebuild_building_entrances_internal();
        if self.has_authored_water_internal() {
            if let Err(err) = self.rebuild_authored_water_preview_internal() {
                debug_log!(
                    "world-editor",
                    "rebuild_authored_water_after_sculpt failed: {}",
                    err
                );
            }
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
        self.allocator
            .repair_road_attachments_after_topology_edit(&self.region_graph, &mut self.zoning);
        self.allocator
            .rebuild_entrance_cache(&self.region_graph, &self.transit_network.lane_system);
    }

    fn run_building_allocator_maintenance_internal(&mut self) {
        self.allocator.tick(
            &mut self.zoning,
            &mut self.agents,
            &mut self.households,
            &mut self.logistics,
            &mut self.transit_network,
            &mut self.region_graph,
        );
        use crate::simulation::buildings::allocator::BASELINE_PRIVATE_ZONES;
        for (zone_idx, zone) in BASELINE_PRIVATE_ZONES.iter().enumerate() {
            if self.allocator.dirty_zones[zone_idx] {
                self.allocator.dirty_zones[zone_idx] = false;
                self.transit_network.flow_fields.mark_zone_dirty(*zone);
            }
        }
    }

    /// Returns the deterministic building-or-road target for the bulldoze cursor.
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
        self.prepare_bulldoze_target_indices();
        let current = self.resolve_bulldoze_target(target.center.x, target.center.z)?;
        if current.kind != target.kind || current.id != target.id {
            return None;
        }
        let road_deleted = current.kind == BulldozeTargetKind::Road;
        self.delete_bulldoze_target(&current)
            .then_some(road_deleted)
    }

    fn delete_bulldoze_target(&mut self, target: &BulldozeTarget) -> bool {
        match target.kind {
            BulldozeTargetKind::Building => self.bulldoze_building(target.id),
            BulldozeTargetKind::Road => self.bulldoze_road_edge(target.id),
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
    }

    fn resolve_bulldoze_building_target(&self, pos: Vector2) -> Option<BulldozeTarget> {
        let candidates = self
            .allocator
            .site_candidate_indices_for_bounds(pos.x, pos.y, pos.x, pos.y);
        let mut best: Option<(usize, f32)> = None;
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
                .map(|(best_idx, best_dist_sq)| {
                    dist_sq < best_dist_sq - f32::EPSILON
                        || ((dist_sq - best_dist_sq).abs() <= f32::EPSILON && idx < best_idx)
                })
                .unwrap_or(true);
            if replace {
                best = Some((idx, dist_sq));
            }
        }

        let (building_idx, _) = best?;
        let site = self.allocator.building_sites.get(building_idx)?;
        let mut points = Vec::with_capacity(site.footprint_world.len());
        for point in &site.footprint_world {
            points.push(Vector3::new(
                point.x,
                site.support_height_m + BULLDOZE_HIGHLIGHT_Y_OFFSET_M,
                point.y,
            ));
        }
        let building = self.allocator.buildings.get(building_idx)?;
        Some(BulldozeTarget {
            kind: BulldozeTargetKind::Building,
            id: building_idx,
            center: Vector3::new(
                building.center_x,
                site.support_height_m + BULLDOZE_HIGHLIGHT_Y_OFFSET_M,
                building.center_y,
            ),
            points,
            width_m: 0.0,
        })
    }

    fn resolve_bulldoze_road_target(&self, pos: Vector2) -> Option<BulldozeTarget> {
        let query_pos = Vector3::new(pos.x, 0.0, pos.y);
        let mut best: Option<(usize, f32, Vector3)> = None;
        for edge_idx in self
            .region_graph
            .get_edges_near_point(query_pos, BULLDOZE_ROAD_PICK_RADIUS_M)
        {
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
                let replace = best
                    .map(|(best_idx, best_dist_sq, _)| {
                        dist_sq < best_dist_sq - f32::EPSILON
                            || ((dist_sq - best_dist_sq).abs() <= f32::EPSILON
                                && edge_idx < best_idx)
                    })
                    .unwrap_or(true);
                if replace {
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
                .road_surface
                .sample_visible_surface_height(
                    &self.region_graph,
                    &self.heightmap,
                    point.x,
                    point.z,
                )
                .unwrap_or(point.y);
            points.push(Vector3::new(
                point.x,
                y + BULLDOZE_HIGHLIGHT_Y_OFFSET_M,
                point.z,
            ));
        }
        let center_y = self
            .transit_network
            .road_surface
            .sample_visible_surface_height(
                &self.region_graph,
                &self.heightmap,
                closest.x,
                closest.z,
            )
            .unwrap_or(closest.y);
        Some(BulldozeTarget {
            kind: BulldozeTargetKind::Road,
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

    fn bulldoze_building(&mut self, building_idx: usize) -> bool {
        self.remove_building_for_edit(building_idx, true)
    }

    fn remove_building_for_edit(&mut self, building_idx: usize, record_undo: bool) -> bool {
        if building_idx >= self.allocator.buildings.len() {
            return false;
        }
        let dirty_bounds = self.allocator.site_world_bounds(building_idx);
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
        let last_building_idx_before_remove = self.allocator.buildings.len().saturating_sub(1);
        let removed = self.allocator.remove_building_for_bulldoze(
            building_idx,
            &mut self.zoning,
            &mut self.agents,
            &mut self.households,
            &mut self.logistics,
        );
        if !removed {
            if record_undo {
                self.undo_stack.pop_back();
            }
            return false;
        }
        self.resource_extraction
            .remove_building_after_swap_remove(building_idx, last_building_idx_before_remove);
        self.agriculture
            .remove_building_after_swap_remove(building_idx, last_building_idx_before_remove);
        if let Some(bounds) = dirty_bounds {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
        self.rebuild_building_entrances_internal();
        if record_undo {
            self.seal_building_removal_undo();
        }
        self.transit_network.flow_fields.mark_all_dirty();
        self.terrain_dirty = true;
        true
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

        self.push_road_removal_undo(affected_edges.clone(), affected_nodes.clone(), edge_idx);
        self.transit_network.mark_surface_dirty_from_sets(
            &self.region_graph,
            &affected_edges,
            &affected_nodes,
        );
        for point in &edge_points {
            self.transit_network.mark_point_dirty(*point);
            self.transit_network.mark_surface_point_dirty(*point);
        }

        self.region_graph.remove_from_spatial_index(edge_idx);
        self.region_graph.edge_mut(edge_idx).deleted = true;
        self.zoning.remove_parcels_attached_to_edge(edge_idx);
        self.region_graph.rebuild_adjacency_list();
        self.region_graph
            .rebuild_intersection_clips_for_nodes(&affected_nodes);
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
        self.transit_network
            .lane_system
            .rebuild(&mut self.region_graph);
        self.agents.reattach_invalidated_lanes_for_edges(
            &affected_edges,
            &self.transit_network.lane_system,
            &self.region_graph,
        );
        self.rebuild_building_entrances_internal();
        self.transit_network
            .rebuild_cch_and_check(&self.region_graph);
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
        self.transit_network
            .mark_surface_dirty_for_terrain_edit(&self.region_graph, pos, radius);
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
        self.transit_network
            .mark_surface_dirty_for_terrain_edit(&self.region_graph, pos, radius);
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
        self.transit_network
            .mark_surface_dirty_for_terrain_edit(&self.region_graph, pos, radius);
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
        self.transit_network
            .mark_surface_dirty_for_terrain_edit(&self.region_graph, pos, radius);
        self.mark_terrain_authoring_payload_bounds(pos, radius);
        self.finish_terrain_authoring_edit_internal();
    }

    /// Moves terrain toward a slope defined by two clicked world-space anchor points.
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
        self.transit_network
            .mark_surface_dirty_for_terrain_edit(&self.region_graph, pos, radius);
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
        self.transit_network
            .mark_surface_dirty_for_terrain_edit(&self.region_graph, pos, radius);
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
        self.transit_network
            .mark_surface_dirty_for_terrain_edit(&self.region_graph, pos, radius);
        self.mark_terrain_authoring_payload_bounds(pos, radius);
    }

    /// Applies one batched terrain-slope step without running deferred rebuild work yet.
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
        self.transit_network
            .mark_surface_dirty_for_terrain_edit(&self.region_graph, pos, radius);
        self.mark_terrain_authoring_payload_bounds(pos, radius);
    }

    /// Sets the classification of an edge.
    /// Sets or clears the no-building-spawn flag on an edge.
    pub fn set_no_building_spawn_internal(&mut self, edge_idx: i32, enabled: bool) {
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edge_count() {
            return;
        }
        let edge_idx = edge_idx as usize;
        self.region_graph.edge_mut(edge_idx).no_building_spawn = enabled;
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
            let half_width_m = RoadSurfaceSystem::visual_roadbed_half_width_m(edge);
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

        let mut affected_nodes = HashSet::new();
        {
            let edge = self.region_graph.edge_mut(edge_idx);
            affected_nodes.insert(edge.start_node);
            affected_nodes.insert(edge.end_node);
            edge.class = class;
        }
        self.transit_network
            .mark_surface_dirty_for_nodes(&self.region_graph, &affected_nodes);

        self.rebuild_building_entrances_internal();
        self.transit_network
            .rebuild_cch_and_check(&self.region_graph);
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
        self.add_road_internal_with_cross_section(
            points,
            fwd_lanes,
            bkw_lanes,
            None,
            snap_to_existing_roads,
        )
    }

    /// Adds a road from an authored cross-section.
    ///
    /// `cross_section` is the flat form `LaneLayout::from_flat` reads. When it
    /// is present and well-formed the lane counts are derived from it and the
    /// two count arguments are ignored, which is what lets the road tool place
    /// a road with a median, a bus lane, or curbside parking rather than the
    /// nearest pair of numbers. A malformed run falls back to the counts, so a
    /// bad payload from the bridge gives an ordinary road rather than nothing.
    pub(crate) fn add_road_internal_with_cross_section(
        &mut self,
        points: Vec<godot::prelude::Vector3>,
        fwd_lanes: i32,
        bkw_lanes: i32,
        cross_section: Option<&[i32]>,
        snap_to_existing_roads: bool,
    ) -> RoadAddOutcome {
        let authored_layout = cross_section.and_then(LaneLayout::from_flat);
        let (fwd_lanes, bkw_lanes) = match authored_layout.as_ref() {
            Some(layout) => (i32::from(layout.fwd_count()), i32::from(layout.bkw_count())),
            None => (fwd_lanes, bkw_lanes),
        };
        let fwd_lanes_u8 = fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let bkw_lanes_u8 = bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let prepared_input = RoadSurfaceSystem::prepare_road_input_for_tool(
            &points,
            &self.heightmap,
            &self.region_graph,
            &self.transit_network.road_surface,
            snap_to_existing_roads,
        );
        let fixed_points = prepared_input.points.clone();

        let fast_validation = self
            .transit_network
            .road_surface
            .validate_prepared_road_candidate_fast(
                &prepared_input,
                fwd_lanes_u8,
                bkw_lanes_u8,
                &self.heightmap,
                &self.region_graph,
            );
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
            let full_validation_start = Instant::now();
            let new_edge_validation = self
                .transit_network
                .road_surface
                .validate_prepared_road_surface(
                    &prepared_input.points,
                    prepared_input.class,
                    fwd_lanes_u8,
                    bkw_lanes_u8,
                    &self.heightmap,
                );
            let full_validation = self
                .transit_network
                .road_surface
                .validate_prepared_road_input_against_graph_with_compile_reason(
                    &prepared_input,
                    fwd_lanes_u8,
                    bkw_lanes_u8,
                    &self.heightmap,
                    &self.region_graph,
                    new_edge_validation,
                    RoadSurfaceCompileReason::CommitValidator,
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
            return RoadAddOutcome::rejected();
        }

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
            return RoadAddOutcome::rejected();
        }

        let t_undo = Instant::now();
        if !self.benchmark_mode {
            self.push_network_undo_for_polyline(&fixed_points, ROAD_UNDO_TOPOLOGY_MARGIN_M);
        }
        let dt_undo_ms = t_undo.elapsed().as_micros();

        self.apply_road_extension_reprofile(prepared_input.extension.as_ref());

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
        let edges_before = self.region_graph.edge_count();
        self.transit_network.add_road(
            &mut self.region_graph,
            fixed_points,
            fwd_lanes_u8,
            bkw_lanes_u8,
            prepared_input.class,
            &mut self.zoning,
            &mut self.allocator,
        );
        // The counts above placed the road; the authored section replaces the
        // layout they implied. Applied per created edge because one drawn road
        // becomes several edges wherever it crosses or snaps to an existing
        // one, and every piece of one drawn road carries the same section.
        if let Some(layout) = authored_layout {
            for edge_idx in edges_before..self.region_graph.edge_count() {
                let mut edge_layout = layout.clone();
                edge_layout.set_sidewalk_width(layout.authored_sidewalk_width());
                self.region_graph.edge_mut(edge_idx).set_lane_layout(edge_layout);
            }
        }
        let dt_topo_us = t_topo.elapsed().as_micros();

        self.mark_local_network_render_dirty();

        // Store partial timing so the AddRoad handler can append the remaining phases.
        // Zoning is NOT flushed here — create_edge_internal already called
        // invalidate_zoning_near_edge (125 m radius) for every new/split edge.
        // The AddRoad handler calls flush_zoning_updates once after lane rebuild,
        // batching all dirty edges into a single pass instead of N separate passes.
        self.last_road_timing = format!("undo={}µs topo={}µs", dt_undo_ms, dt_topo_us);
        RoadAddOutcome {
            committed: true,
            build_cost: build_length_m * ROAD_BUILD_COST_PER_METER,
        }
    }

    fn apply_road_extension_reprofile(
        &mut self,
        extension: Option<&RoadExtensionReprofile>,
    ) -> HashSet<usize> {
        let Some(extension) = extension else {
            return HashSet::new();
        };
        if extension.existing_edge_idx >= self.region_graph.edge_count()
            || (extension.snapped_node_id as usize) >= self.region_graph.node_count()
            || extension.existing_points.len() < 2
        {
            return HashSet::new();
        }

        let edge_idx = extension.existing_edge_idx;
        if self.region_graph.edge(edge_idx).deleted {
            return HashSet::new();
        }

        let old_node_pos = self.region_graph.node(extension.snapped_node_id).pos;
        let old_chunks = self.region_graph.get_edge_chunks(edge_idx);
        self.region_graph
            .set_node_pos(extension.snapped_node_id, extension.snapped_node_pos);
        self.region_graph.remove_from_spatial_index(edge_idx);
        {
            let edge = self.region_graph.edge_mut(edge_idx);
            edge.geometry = extension.existing_points.clone();
            edge.physical_geometry = extension.existing_points.clone();
            let (cost, length) =
                crate::simulation::pathing::cost::CostCalculator::calculate_costs(edge);
            edge.base_cost = cost;
            edge.physical_length = length;
        }
        self.region_graph.add_to_spatial_index(edge_idx);

        let affected_edges = HashSet::from([edge_idx]);
        let edge = self.region_graph.edge(edge_idx);
        let affected_nodes = HashSet::from([
            self.region_graph.get_valid_node(extension.snapped_node_id),
            self.region_graph.get_valid_node(edge.start_node),
            self.region_graph.get_valid_node(edge.end_node),
        ]);

        for chunk in old_chunks
            .into_iter()
            .chain(self.region_graph.get_edge_chunks(edge_idx))
        {
            self.transit_network.cch_dirty_chunks.insert(chunk);
        }
        self.transit_network.mark_point_dirty(old_node_pos);
        self.transit_network
            .mark_point_dirty(extension.snapped_node_pos);
        self.transit_network.mark_surface_dirty_from_sets(
            &self.region_graph,
            &affected_edges,
            &affected_nodes,
        );
        self.transit_network.mark_surface_point_dirty(old_node_pos);
        self.transit_network
            .mark_surface_point_dirty(extension.snapped_node_pos);

        if self.transit_network.bulk_load {
            self.transit_network.bulk_dirty_edges.insert(edge_idx);
        } else {
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
        }

        debug_log!(
            "road",
            "road_extension_reprofile node={} edge={} old_y={:.3} new_y={:.3} points={}",
            extension.snapped_node_id,
            edge_idx,
            old_node_pos.y,
            extension.snapped_node_pos.y,
            extension.existing_points.len()
        );
        affected_edges
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
            self.zoning.config.zone_cell_m,
            &self.region_graph,
            &self.transit_network.road_surface,
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
            self.zoning.config.zone_cell_m,
            &self.region_graph,
            &self.transit_network.road_surface,
            &self.heightmap,
            &catalog,
            &tuning,
        )?;

        if !self.benchmark_mode {
            if let Some(building) = self.allocator.buildings.get(building_idx) {
                let lot_cells = f64::from(building.width_cells) * f64::from(building.depth_cells);
                self.treasury
                    .deduct_build_cost(lot_cells * SERVICE_BUILD_COST_PER_LOT_CELL);
            }
        }
        self.rebuild_building_entrances_internal();
        if let Some(bounds) = self.allocator.take_pending_site_dirty_bounds() {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
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
            self.zoning.config.zone_cell_m,
            &self.region_graph,
            &self.transit_network.road_surface,
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
            self.zoning.config.zone_cell_m,
            &self.region_graph,
            &self.transit_network.road_surface,
            &self.heightmap,
            &catalog,
            &tuning,
        )?;

        if !self.benchmark_mode {
            if let Some(building) = self.allocator.buildings.get(building_idx) {
                let lot_cells = f64::from(building.width_cells) * f64::from(building.depth_cells);
                self.treasury
                    .deduct_build_cost(lot_cells * SERVICE_BUILD_COST_PER_LOT_CELL);
            }
        }
        self.rebuild_building_entrances_internal();
        if let Some(bounds) = self.allocator.take_pending_site_dirty_bounds() {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
        Ok(building_idx)
    }

    /// Commits or replaces a resource-extraction polygon for one placed industry building.
    pub(crate) fn commit_extractor_polygon_internal(
        &mut self,
        building_idx: usize,
        polygon_world: Vec<Vector2>,
    ) -> Result<crate::simulation::extraction::ExtractorSiteSummary, String> {
        self.resource_extraction.commit_site(
            building_idx,
            polygon_world,
            &self.resource_deposits,
            &mut self.allocator,
            self.zoning.config.zone_cell_m,
        )
    }

    /// Commits or replaces a field polygon for one placed agricultural building.
    pub(crate) fn commit_field_polygon_internal(
        &mut self,
        building_idx: usize,
        polygon_world: Vec<Vector2>,
    ) -> Result<crate::simulation::agriculture::FieldSiteSummary, String> {
        self.agriculture.commit_site(
            building_idx,
            polygon_world,
            &mut self.allocator,
            self.zoning.config.zone_cell_m,
        )
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

    /// Repositions a network node in world space.
    pub fn move_network_node_internal(&mut self, node_id: i32, pos: Vector3) {
        if node_id >= 0 && (node_id as usize) < self.region_graph.node_count() {
            let old_pos = self.region_graph.node(node_id as u32).pos;
            let affected_edges: HashSet<usize> = self
                .region_graph
                .node_adjacency(node_id as u32)
                .iter()
                .copied()
                .collect();

            self.push_network_undo_for_local_topology(
                affected_edges.clone(),
                HashSet::from([node_id as u32]),
            );
            self.region_graph.move_node(node_id as u32, pos);
            for &edge_idx in &affected_edges {
                let length = self
                    .region_graph
                    .calculate_length(&self.region_graph.edge(edge_idx).physical_geometry);
                self.region_graph.edge_mut(edge_idx).physical_length = length;
            }
            self.region_graph.rebuild_intersection_clips();
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
            self.transit_network
                .rebuild_cch_and_check(&self.region_graph);
            self.transit_network.flow_fields.mark_all_dirty();
            let mut affected_nodes = HashSet::from([node_id as u32]);
            for &edge_idx in &affected_edges {
                let edge = self.region_graph.edge(edge_idx);
                affected_nodes.insert(edge.start_node);
                affected_nodes.insert(edge.end_node);
            }
            self.transit_network.mark_surface_dirty_from_sets(
                &self.region_graph,
                &affected_edges,
                &affected_nodes,
            );
            self.transit_network.mark_surface_point_dirty(old_pos);
            self.mark_local_network_render_dirty();
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
                .map_or(false, |v| v.contains(&target));
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
        if crate::debug::is_traffic_enabled() {
            if (node_id as usize) < self.region_graph.node_count() {
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
        }
        self.transit_network
            .rebuild_cch_and_check(&self.region_graph);
        self.transit_network.flow_fields.mark_all_dirty();
        if (node_id as usize) < self.region_graph.node_count() {
            let pos = self.region_graph.node(node_id).pos;
            self.transit_network.mark_point_dirty(pos);
        }
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
        self.transit_network
            .rebuild_cch_and_check(&self.region_graph);
        self.transit_network.flow_fields.mark_all_dirty();
        if (node_id as usize) < self.region_graph.node_count() {
            let pos = self.region_graph.node(node_id).pos;
            self.transit_network.mark_point_dirty(pos);
        }
    }

    /// Sets a priority sign on one approach arm of a junction.
    ///
    /// `sign`: 0 main, 1 yield, 2 stop. Switching a node from a signal to
    /// priority discards the signal program, since the two schemes are
    /// exclusive. Control changes alter neither geometry nor connectivity, so
    /// no lane, entrance, or routing rebuild is required.
    pub fn set_junction_priority_internal(&mut self, node_id: u32, edge_id: i32, sign: u8) {
        let node_id = self.region_graph.get_valid_node(node_id);
        if (node_id as usize) >= self.region_graph.node_count() || edge_id < 0 {
            return;
        }
        let sign = match sign {
            0 => PrioritySign::Main,
            2 => PrioritySign::Stop,
            _ => PrioritySign::Yield,
        };
        if !matches!(
            self.region_graph.node(node_id).control,
            JunctionControl::Priority(_)
        ) {
            self.region_graph
                .set_node_control(node_id, JunctionControl::Priority(HashMap::new()));
        }
        if let JunctionControl::Priority(signs) = self.region_graph.node_control_mut(node_id) {
            signs.insert(edge_id as usize, sign);
        }
    }

    /// Appends a phase to a junction's signal program.
    ///
    /// Switching a node from priority to a signal discards the priority signs.
    /// `green_arms` is the set of approach edges holding green for the phase.
    pub fn add_junction_signal_phase_internal(
        &mut self,
        node_id: u32,
        green_arms: PackedInt32Array,
        green_s: f32,
        amber_s: f32,
    ) {
        let node_id = self.region_graph.get_valid_node(node_id);
        if (node_id as usize) >= self.region_graph.node_count() {
            return;
        }
        let arms: Vec<usize> = green_arms
            .as_slice()
            .iter()
            .filter(|&&e| e >= 0)
            .map(|&e| e as usize)
            .collect();
        if !matches!(
            self.region_graph.node(node_id).control,
            JunctionControl::Signal(_)
        ) {
            self.region_graph
                .set_node_control(node_id, JunctionControl::Signal(SignalProgram::default()));
        }
        if let JunctionControl::Signal(program) = self.region_graph.node_control_mut(node_id) {
            program.phases.push(SignalPhase {
                green_arms: arms,
                green_s: green_s.max(0.0),
                amber_s: amber_s.max(0.0),
            });
        }
    }

    /// Sets the cycle offset of a junction's signal program, in seconds.
    ///
    /// Offsetting neighboring junctions along a corridor is what produces a
    /// green wave. Does nothing at a node that carries no signal.
    pub fn set_junction_signal_offset_internal(&mut self, node_id: u32, offset_s: f32) {
        let node_id = self.region_graph.get_valid_node(node_id);
        if (node_id as usize) >= self.region_graph.node_count() {
            return;
        }
        if let JunctionControl::Signal(program) = self.region_graph.node_control_mut(node_id) {
            program.offset_s = offset_s;
        }
    }

    /// Removes all control from a junction, returning it to uncontrolled.
    pub fn clear_junction_control_internal(&mut self, node_id: u32) {
        let node_id = self.region_graph.get_valid_node(node_id);
        if (node_id as usize) < self.region_graph.node_count() {
            self.region_graph
                .set_node_control(node_id, JunctionControl::Uncontrolled);
        }
    }

    /// Describes a junction's control for the UI.
    ///
    /// Returns `kind` as `"uncontrolled"`, `"priority"`, or `"signal"`. A
    /// priority node adds `signs`, a dictionary of edge id to sign integer. A
    /// signal node adds `offset_s`, `cycle_s`, and `phases`, each phase holding
    /// its green arms and durations.
    pub fn get_junction_control_internal(&self, node_id: u32) -> VarDictionary {
        let mut out = VarDictionary::new();
        let node_id = self.region_graph.get_valid_node(node_id);
        if (node_id as usize) >= self.region_graph.node_count() {
            out.set("kind", "uncontrolled");
            return out;
        }
        match &self.region_graph.node(node_id).control {
            JunctionControl::Uncontrolled => {
                out.set("kind", "uncontrolled");
            }
            JunctionControl::Priority(signs) => {
                out.set("kind", "priority");
                let mut map = VarDictionary::new();
                for (&edge, &sign) in signs {
                    let code: u8 = match sign {
                        PrioritySign::Main => 0,
                        PrioritySign::Yield => 1,
                        PrioritySign::Stop => 2,
                    };
                    map.set(edge as i64, code as i64);
                }
                out.set("signs", map);
            }
            JunctionControl::Signal(program) => {
                out.set("kind", "signal");
                out.set("offset_s", program.offset_s);
                out.set("cycle_s", program.cycle_s());
                let mut phases = VarArray::new();
                for phase in &program.phases {
                    let mut p = VarDictionary::new();
                    let arms: PackedInt32Array =
                        phase.green_arms.iter().map(|&e| e as i32).collect();
                    p.set("green_arms", arms);
                    p.set("green_s", phase.green_s);
                    p.set("amber_s", phase.amber_s);
                    phases.push(&p.to_variant());
                }
                out.set("phases", phases);
            }
        }
        out
    }

    /// Returns the aspect one approach arm is showing at `sim_time`.
    ///
    /// `0` green, `1` amber, `2` red. An uncontrolled or priority junction
    /// reports green, because neither shows an aspect.
    pub fn get_junction_signal_aspect_internal(
        &self,
        node_id: u32,
        edge_id: i32,
        sim_time: f32,
    ) -> u8 {
        let node_id = self.region_graph.get_valid_node(node_id);
        if (node_id as usize) >= self.region_graph.node_count() || edge_id < 0 {
            return 0;
        }
        match &self.region_graph.node(node_id).control {
            JunctionControl::Signal(program) => {
                match program.aspect_at(edge_id as usize, sim_time) {
                    SignalAspect::Green => 0,
                    SignalAspect::Amber => 1,
                    SignalAspect::Red => 2,
                }
            }
            _ => 0,
        }
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
        self.transit_network
            .rebuild_cch_and_check(&self.region_graph);
        self.transit_network.flow_fields.mark_all_dirty();
        if (node_id as usize) < self.region_graph.node_count() {
            let pos = self.region_graph.node(node_id).pos;
            self.transit_network.mark_point_dirty(pos);
        }
    }

    /// Toggles a user override for a crosswalk at a specific road mouth.
    pub fn set_crosswalk_override_internal(&mut self, node_id: u32, edge_id: i32, enabled: bool) {
        if node_id as usize >= self.region_graph.node_count() || edge_id < 0 {
            return;
        }
        self.push_network_undo_for_local_topology(
            HashSet::from([edge_id as usize]),
            HashSet::from([node_id]),
        );
        self.region_graph
            .set_crosswalk_override(node_id, edge_id as usize, enabled);

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
        if (node_id as usize) < self.region_graph.node_count() {
            let pos = self.region_graph.node(node_id).pos;
            self.transit_network.mark_point_dirty(pos);
        }
    }

    /// Rebuilds road-surface-driven visual terrain after network edits.
    pub fn rebuild_network_surface_terrain_internal(&mut self) {
        self.rebuild_network_surface_terrain_internal_with_entrance_rebuild(true);
    }

    pub(crate) fn rebuild_network_surface_terrain_internal_with_entrance_rebuild(
        &mut self,
        rebuild_entrances: bool,
    ) {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let debug_edges = std::mem::take(&mut self.last_surface_debug_edges);

        let earthwork_start = road_debug.then(Instant::now);
        let road_surface_had_dirty_work =
            self.transit_network.road_surface.has_pending_rebuild_work();
        let dirty_chunks = self
            .transit_network
            .rebuild_dirty_terrain_earthworks_with_reason(
                &self.region_graph,
                &mut self.heightmap,
                RoadSurfaceCompileReason::SimCommit,
            );
        let road_locked_dirty_patch_keys = self
            .transit_network
            .road_surface
            .mark_render_patches_for_chunk_grading_envelopes(
                &mut self.heightmap,
                &dirty_chunks,
                ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
            );
        if road_surface_had_dirty_work {
            let dirty_query_chunks = self
                .transit_network
                .road_surface
                .last_rebuilt_query_chunks()
                .to_vec();
            self.bump_local_road_terrain_payload_generations(
                &road_locked_dirty_patch_keys,
                &dirty_query_chunks,
            );
        }
        let road_locked_dirty_patches = road_locked_dirty_patch_keys.len();
        let earthwork_ms = earthwork_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let entrances_start = road_debug.then(Instant::now);
        if rebuild_entrances {
            self.rebuild_building_entrances_internal();
        }
        let entrances_ms = entrances_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let mut dump_build_ms = 0.0;
        let mut dump_print_ms = 0.0;
        let mut dump_bytes = 0usize;
        if crate::debug::category_enabled("road")
            && Self::road_geometry_dump_enabled()
            && !debug_edges.is_empty()
        {
            let dump_start = Instant::now();
            for line in self
                .transit_network
                .road_surface
                .build_road_cut_fill_debug_lines(&self.region_graph, &self.heightmap, &debug_edges)
            {
                debug_log!("road", "{}", line);
            }
            match Self::road_geometry_dump_max_bytes() {
                Some(max_bytes) => {
                    dump_build_ms = dump_start.elapsed().as_secs_f64() * 1000.0;
                    let dump_print_start = Instant::now();
                    debug_log!(
                        "road",
                        "road_geometry_dump_skipped dump_edges={} max_bytes={} reason=capped_mode enable_full_with=METRUM_DEBUG_ROAD_GEOMETRY_DUMP_FULL",
                        debug_edges.len(),
                        max_bytes
                    );
                    dump_print_ms = dump_print_start.elapsed().as_secs_f64() * 1000.0;
                }
                None => {
                    let dump = self
                        .transit_network
                        .road_surface
                        .build_edge_geometry_debug_dump(
                            &self.region_graph,
                            &self.heightmap,
                            &debug_edges,
                        );
                    dump_build_ms = dump_start.elapsed().as_secs_f64() * 1000.0;
                    dump_bytes = dump.len();
                    let dump_print_start = Instant::now();
                    debug_log!("road", "{}", dump);
                    dump_print_ms = dump_print_start.elapsed().as_secs_f64() * 1000.0;
                }
            }
        }
        self.terrain_dirty = true;
        if road_debug {
            debug_log!(
                "road",
                "terrain_rebuild_detail earthworks_ms={:.3} dirty_chunks={} road_locked_dirty_patches={} dirty_render_patches={} entrances_ms={:.3} dump_edges={} dump_bytes={} dump_build_ms={:.3} dump_print_ms={:.3} total_ms={:.3}",
                earthwork_ms,
                dirty_chunks.len(),
                road_locked_dirty_patches,
                self.heightmap.dirty_render_patches().len(),
                entrances_ms,
                debug_edges.len(),
                dump_bytes,
                dump_build_ms,
                dump_print_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }

    /// Returns the ID of the nearest node to `pos` if `pos` is within
    /// [`config::BORDER_DETECTION_THRESHOLD`] metres of any map edge, or `-1` if not.
    ///
    /// Call this after [`add_road_internal`] with the road's start or end position to find
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
    /// After this call the node's type becomes [`NodeType::Border`] and it will be used as an
    /// immigrant spawn point by [`BuildingAllocator::tick`] as long as the road remains connected.
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
            self.transit_network
                .rebuild_cch_and_check(&self.region_graph);
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
            self.transit_network.mark_surface_dirty_from_sets(
                &self.region_graph,
                &affected_edges,
                &affected_nodes,
            );
            self.transit_network.mark_surface_point_dirty(old_pos);
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
    use super::{BulldozeTargetKind, ROAD_LOCKED_TERRAIN_RENDER_STEP_M, SimCore};
    use crate::nodes::sim::core::PendingDemandSpawnAction;
    use crate::simulation::agriculture::AgricultureSystem;
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::core::time::TimeSystem;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::economy::demand::{DemandSpawnAction, DemandSystem};
    use crate::simulation::economy::households::HouseholdSystem;
    use crate::simulation::economy::logistics::ShipmentSystem;
    use crate::simulation::extraction::ResourceExtractionSystem;
    use crate::simulation::grid::desirability::DesirabilitySystem;
    use crate::simulation::grid::noise::NoiseSystem;
    use crate::simulation::grid::pollution::PollutionSystem;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::resources::ResourceDepositSystem;
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::water::WaterSystem;
    use crate::simulation::zoning::{ZoneType, ZoningSystem};
    use godot::prelude::{Vector2, Vector3};
    use std::collections::HashSet;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Arc;

    fn test_core() -> SimCore {
        use crate::nodes::sim::core::CityTreasury;
        let config = WorldConfig::default();
        SimCore {
            time: TimeSystem::new(),
            heightmap: TerrainSystem::from_world_config(&config),
            watermap: WaterSystem::from_world_config(&config),
            region_graph: RegionGraph::new(),
            transit_network: TransitNetwork::new_with_surface_chunk_span(
                config.terrain_render_chunk_span_m(),
            ),
            zoning: ZoningSystem::new(&config),
            pollution: PollutionSystem::new(&config),
            noise: NoiseSystem::new(&config),
            desirability: DesirabilitySystem::new(&config),
            demand: DemandSystem::new(),
            pending_demand_spawns: VecDeque::new(),
            allocator: BuildingAllocator::new(),
            agents: AgentSystem::new(),
            households: HouseholdSystem::new(),
            logistics: ShipmentSystem::new(),
            config,
            treasury: CityTreasury::new(0.0),
            service_policy: Default::default(),
            fiscal_policy: Default::default(),
            budget_history: VecDeque::new(),
            budget_last_lifetime_build_cost: 0.0,
            debug_household_admissions_since_daily: 0,
            undo_stack: VecDeque::new(),
            world_lake_fills: Vec::new(),
            world_open_water_fills: Vec::new(),
            resource_deposits: ResourceDepositSystem::from_world_config(&config),
            resource_extraction: ResourceExtractionSystem::new(),
            agriculture: AgricultureSystem::new(),
            world_lake_fill_preview: None,
            authored_water_patch_fill_debug_cache: HashMap::new(),
            terrain_stroke_active: false,
            terrain_stroke_has_changes: false,
            terrain_dirty: false,
            water_dirty: false,
            network_dirty: false,
            benchmark_mode: true,
            last_tick_duration: 0.0,
            last_agent_tick_us: 0,
            last_road_timing: String::new(),
            last_surface_debug_edges: Vec::new(),
            refined_terrain_patch_cache: HashMap::new(),
            road_locked_terrain_patch_keys: Vec::new(),
            road_locked_terrain_patch_margins: std::collections::BTreeMap::new(),
            building_site_owned_terrain_patch_keys: HashSet::new(),
            engineered_terrain_patch_keys: Vec::new(),
            engineered_terrain_patch_margins: std::collections::BTreeMap::new(),
            terrain_payload_generation_counter: 1,
            terrain_payload_global_generation: 1,
            terrain_payload_patch_generations: HashMap::new(),
            refined_terrain_assembly_ledgers: HashMap::new(),
            cached_road_mesh_chunks: std::collections::BTreeMap::new(),
            published_road_mesh_chunks: Arc::new(std::collections::BTreeMap::new()),
            pending_road_mesh_chunks: Arc::new(std::collections::BTreeSet::new()),
            road_mesh_full_replace: true,
            cached_road_mesh_generation: 0,
            cached_network_node_positions: std::sync::Arc::new(Vec::new()),
            cached_network_node_positions_dirty: true,
            road_tool_surface_generation: 1,
            camera_aabb: (0.0, 0.0, 0.0, 0.0),
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
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
            speed_limit: 50.0,
            base_cost: start.distance_to(end),
            physical_length: start.distance_to(end),
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![start, end],
            physical_geometry: vec![start, end],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            frontage_class: Default::default(),
        })
    }

    fn test_building(asset_id: &str, center_x: f32, support_height_m: f32) -> Building {
        Building {
            center_x,
            center_y: 0.0,
            support_height_m,
            width_cells: 2,
            depth_cells: 2,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 1.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx: usize::MAX,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            service_funding_override: -1.0,
            asset_id: asset_id.to_owned(),
            level: 1,
            construction_total_hours: 0,
            construction_remaining_hours: 0,
            broken: false,
            economy_profile_runtime_id: 0,
            economy_broken: false,
            resource_inventory: Vec::new(),
            revenue: 0.0,
            operating_budget: 500.0,
            profit_tax_budget_baseline: 500.0,
            last_day_profit: 0.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            daily_city_funded_input_cost: 0.0,
            daily_household_sales_value: 0.0,
            daily_power_service_units: 0.0,
            daily_power_served_units: 0.0,
            recent_power_service_units: 0.0,
            recent_power_served_units: 0.0,
            recent_household_sales_value: 0.0,
            commercial_activity_floor_scale: 0.0,
            work_area_scale: 1.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        }
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
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
            speed_limit: 50.0,
            base_cost: 10.0,
            physical_length: 10.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            frontage_class: Default::default(),
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
    fn set_no_building_spawn_internal_removes_attached_zoning_parcels() {
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
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
            speed_limit: 50.0,
            base_cost: 120.0,
            physical_length: 120.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            frontage_class: Default::default(),
        });
        let residential = core
            .zoning
            .profiles
            .default_runtime_id_for_zone_type(ZoneType::Residential)
            .unwrap();
        core.zoning
            .place_or_rezone_default_parcel_at(0.0, -20.0, residential, &core.region_graph)
            .expect("parcel");

        core.set_no_building_spawn_internal(0, true);

        assert!(core.region_graph.edge(0).no_building_spawn);
        assert!(core.zoning.parcels().is_empty());
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
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
            speed_limit: 50.0,
            base_cost: 120.0,
            physical_length: 120.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            frontage_class: Default::default(),
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

    #[test]
    fn runtime_undo_restores_pending_demand_spawn_queue() {
        let mut core = test_core();
        core.pending_demand_spawns
            .push_back(PendingDemandSpawnAction {
                due_minute: 42,
                zone_type: ZoneType::Residential,
                action: DemandSpawnAction {
                    parcel_id: 7,
                    asset_id: "building.residential.test".to_owned(),
                },
                planned_day_index: 2,
                planned_minute_of_day: 60,
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
    fn building_bulldoze_undo_restores_swap_removed_building_and_site() {
        let mut core = test_core();
        core.allocator.buildings = vec![
            test_building("building.removed", -20.0, 3.0),
            test_building("building.moved", 20.0, 7.0),
        ];
        core.allocator
            .rebuild_building_site_clients(core.zoning.config.zone_cell_m);
        core.allocator.rebuild_zone_index();
        let original_sites = core.allocator.building_sites.clone();

        assert!(core.bulldoze_building(0));
        assert_eq!(core.allocator.buildings.len(), 1);
        assert_eq!(core.allocator.buildings[0].asset_id, "building.moved");

        assert!(core.undo_action_internal());
        assert_eq!(core.allocator.buildings.len(), 2);
        assert_eq!(core.allocator.buildings[0].asset_id, "building.removed");
        assert_eq!(core.allocator.buildings[1].asset_id, "building.moved");
        assert_eq!(core.allocator.building_sites.len(), original_sites.len());
        for (restored, original) in core.allocator.building_sites.iter().zip(&original_sites) {
            assert_eq!(restored.footprint_world, original.footprint_world);
            assert_eq!(restored.lot_footprint_world, original.lot_footprint_world);
            assert_eq!(restored.support_height_m, original.support_height_m);
        }
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
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
            speed_limit: 50.0,
            base_cost: 120.0,
            physical_length: 20.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            frontage_class: Default::default(),
        });
        core.region_graph.rebuild_adjacency_list();

        core.move_network_node_internal(n0 as i32, Vector3::new(-20.0, 0.0, 0.0));
        assert_eq!(core.region_graph.node(n0).pos.x, -20.0);

        assert!(core.undo_action_internal());
        assert_eq!(
            core.region_graph.node(n0).pos,
            Vector3::new(-10.0, 0.0, 0.0)
        );
        assert_eq!(
            core.region_graph.edge(0).physical_geometry[0],
            Vector3::new(-10.0, 0.0, 0.0)
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

    #[test]
    fn unpublishable_committed_road_rolls_back_without_treasury_charge() {
        let mut core = test_core();
        core.benchmark_mode = false;
        core.treasury.balance = 10_000.0;
        let balance_before = core.treasury.balance;
        let lifetime_build_cost_before = core.treasury.lifetime_build_cost;

        let outcome = core.add_road_internal(
            vec![Vector3::new(-40.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)],
            1,
            1,
        );
        assert!(outcome.committed);
        assert!(outcome.build_cost > 0.0);
        assert_eq!(core.region_graph.edge_count(), 1);
        assert_eq!(core.undo_stack.len(), 1);

        core.transit_network.road_surface.clear();
        assert!(core.rollback_unpublishable_road_commit());

        assert_eq!(core.region_graph.edge_count(), 0);
        assert!(core.undo_stack.is_empty());
        assert_eq!(core.treasury.balance, balance_before);
        assert_eq!(
            core.treasury.lifetime_build_cost,
            lifetime_build_cost_before
        );
        assert_eq!(
            core.last_road_timing,
            "rejected=surface_geometry_invalid_after_commit"
        );
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
            core.transit_network.road_surface.compiled_once,
            "bounded road undo must not force the next surface pass through compile_all"
        );
        assert!(
            core.transit_network.road_surface.has_pending_rebuild_work(),
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
            assert!(!ledger.road_query_chunk_dirty_at.is_empty());
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
        assert!(core.transit_network.road_surface.compiled_once);
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

        let baseline_piece = core
            .transit_network
            .road_surface
            .compiled_visual_node_pieces
            .get(&center)
            .expect("three-mouth JunctionN must compile")
            .clone();
        let baseline_input = core
            .transit_network
            .road_surface
            .compiled_visual_node_inputs
            .get(&center)
            .expect("three-mouth JunctionN must retain its compiler input")
            .clone();
        let baseline_boundaries = core
            .transit_network
            .road_surface
            .compiled_visual_node_earthwork_boundaries
            .get(&center)
            .expect("JunctionN must retain its earthwork boundary")
            .clone();
        let baseline_topology = core
            .transit_network
            .road_surface
            .compiled_visual_node_topologies
            .get(&center)
            .expect("JunctionN must retain its canonical topology")
            .clone();
        let baseline_mesh_vertices = (
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.earthwork_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.curb_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.raised_step_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.sidewalk_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.road_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.marking_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.concrete_vertices.iter().copied())
                .collect::<Vec<_>>(),
        );

        core.benchmark_mode = false;
        let appended_node = core.region_graph.node_count() as u32;
        let appended_edge = core.region_graph.edge_count();
        core.add_road_internal(
            vec![Vector3::new(0.0, 0.0, -36.0), Vector3::new(0.0, 0.0, 0.0)],
            1,
            1,
        );
        assert!(
            core.undo_stack
                .back()
                .expect("network undo snapshot")
                .road_surface_topology
                .is_some(),
            "a clean compiled surface must capture a bounded pre-edit checkpoint"
        );

        finalize_network_render_for_test(&mut core);
        assert_eq!(core.region_graph.node_count(), appended_node as usize + 1);
        assert_eq!(core.region_graph.edge_count(), 4);
        assert!(
            !Arc::ptr_eq(
                core.transit_network
                    .road_surface
                    .compiled_visual_node_topologies
                    .get(&center)
                    .expect("four-mouth JunctionN topology"),
                &baseline_topology,
            ),
            "the edit must replace the center JunctionN cache before undo"
        );

        assert!(core.undo_action_internal());
        assert_eq!(core.region_graph.edge_count(), 3);
        assert_eq!(
            core.transit_network
                .road_surface
                .compiled_visual_node_pieces
                .get(&center),
            Some(&baseline_piece),
            "undo must restore the exact pre-edit JunctionN piece without a cold compile"
        );
        assert_eq!(
            core.transit_network
                .road_surface
                .compiled_visual_node_inputs
                .get(&center),
            Some(&baseline_input),
            "undo must restore the exact pre-edit JunctionN compiler input"
        );
        assert!(Arc::ptr_eq(
            core.transit_network
                .road_surface
                .compiled_visual_node_earthwork_boundaries
                .get(&center)
                .expect("restored earthwork boundary"),
            &baseline_boundaries,
        ));
        assert!(Arc::ptr_eq(
            core.transit_network
                .road_surface
                .compiled_visual_node_topologies
                .get(&center)
                .expect("restored canonical topology"),
            &baseline_topology,
        ));
        assert!(
            !core
                .transit_network
                .road_surface
                .compiled_sections
                .contains_key(&appended_edge)
                && !core
                    .transit_network
                    .road_surface
                    .compiled_visual_span_pieces
                    .contains_key(&appended_edge)
                && !core
                    .transit_network
                    .road_surface
                    .compiled_visual_node_pieces
                    .contains_key(&appended_node)
                && !core
                    .transit_network
                    .road_surface
                    .compiled_visual_node_inputs
                    .contains_key(&appended_node)
                && !core
                    .transit_network
                    .road_surface
                    .compiled_visual_node_earthwork_boundaries
                    .contains_key(&appended_node)
                && !core
                    .transit_network
                    .road_surface
                    .compiled_visual_node_topologies
                    .contains_key(&appended_node)
                && !core
                    .transit_network
                    .road_surface
                    .surface_span_chunks
                    .contains_key(&appended_edge)
                && !core
                    .transit_network
                    .road_surface
                    .earthwork_span_chunks
                    .contains_key(&appended_edge)
                && !core
                    .transit_network
                    .road_surface
                    .query_span_chunks
                    .contains_key(&appended_edge)
                && !core
                    .transit_network
                    .road_surface
                    .surface_node_chunks
                    .contains_key(&appended_node)
                && !core
                    .transit_network
                    .road_surface
                    .earthwork_node_chunks
                    .contains_key(&appended_node)
                && !core
                    .transit_network
                    .road_surface
                    .query_node_chunks
                    .contains_key(&appended_node),
            "appended post-edit surface owners must be removed"
        );
        assert!(
            core.transit_network.road_surface.dirty_edges.is_empty()
                && core.transit_network.road_surface.dirty_nodes.is_empty(),
            "an exact cache restore must not enqueue owner recompilation"
        );
        assert!(
            !core
                .transit_network
                .road_surface
                .dirty_surface_chunks
                .is_empty()
                && !core
                    .transit_network
                    .road_surface
                    .dirty_terrain_chunks
                    .is_empty()
                && !core
                    .transit_network
                    .road_surface
                    .dirty_query_chunks
                    .is_empty(),
            "old and new owner coverage must enqueue only chunk-shell rebuilds"
        );

        core.rebuild_network_surface_terrain_internal();
        core.precompute_road_mesh_data();
        assert!(
            core.transit_network.road_surface.dirty_edges.is_empty()
                && core.transit_network.road_surface.dirty_nodes.is_empty()
                && core
                    .transit_network
                    .road_surface
                    .dirty_surface_chunks
                    .is_empty()
                && core
                    .transit_network
                    .road_surface
                    .dirty_terrain_chunks
                    .is_empty()
                && core
                    .transit_network
                    .road_surface
                    .dirty_query_chunks
                    .is_empty(),
            "terrain finalization must rebuild dirty chunk shells without compiling owners"
        );
        assert!(
            !core
                .transit_network
                .road_surface
                .last_rebuilt_surface_chunks
                .is_empty()
                && !core
                    .transit_network
                    .road_surface
                    .last_rebuilt_terrain_chunks
                    .is_empty()
                && !core
                    .transit_network
                    .road_surface
                    .last_rebuilt_query_chunks
                    .is_empty(),
            "the restored old/new coverage must be committed to every chunk cache"
        );
        assert!(
            core.transit_network
                .road_surface
                .surface_chunk_cache
                .values()
                .all(|entry| !entry.edge_indices.contains(&appended_edge)
                    && !entry.node_ids.contains(&appended_node))
                && core
                    .transit_network
                    .road_surface
                    .earthwork_chunk_cache
                    .values()
                    .all(|entry| !entry.edge_indices.contains(&appended_edge)
                        && !entry.node_ids.contains(&appended_node)),
            "rebuilt chunk shells must not retain appended owner IDs"
        );
        let restored_mesh_vertices = (
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.earthwork_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.curb_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.raised_step_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.sidewalk_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.road_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.marking_vertices.iter().copied())
                .collect::<Vec<_>>(),
            core.cached_road_mesh_chunks
                .values()
                .flat_map(|mesh| mesh.concrete_vertices.iter().copied())
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            restored_mesh_vertices, baseline_mesh_vertices,
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

        core.transit_network
            .road_surface
            .mark_edge_dirty(&core.region_graph, edge_idx);
        core.push_network_undo_for_local_topology(
            HashSet::from([edge_idx]),
            HashSet::from([start]),
        );
        assert!(
            core.undo_stack
                .back()
                .expect("network undo snapshot")
                .road_surface_topology
                .is_none(),
            "pending compiler work must reject exact cache capture"
        );

        core.region_graph
            .move_node(start, Vector3::new(-30.0, 0.0, 0.0));
        assert!(core.undo_action_internal());
        assert_eq!(
            core.region_graph.node(start).pos,
            Vector3::new(-24.0, 0.0, 0.0)
        );
        assert!(
            core.transit_network
                .road_surface
                .dirty_edges
                .contains(&edge_idx)
                && core
                    .transit_network
                    .road_surface
                    .dirty_nodes
                    .contains(&start),
            "invalid cache capture must fall back to the bounded owner recompile path"
        );
    }

    #[test]
    fn sculpt_terrain_marks_road_surface_dirty() {
        let mut core = test_core();
        let n0 = core
            .region_graph
            .add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = core
            .region_graph
            .add_node(Vector3::new(8.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = core.region_graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
            speed_limit: 50.0,
            base_cost: 10.0,
            physical_length: 8.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(8.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(8.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            frontage_class: Default::default(),
        });
        core.region_graph.rebuild_all_indices();

        core.sculpt_terrain_stroke_step_internal(Vector2::new(4.0, 0.0), 5.0, 0.5);

        assert!(
            core.transit_network
                .road_surface
                .dirty_edges()
                .contains(&edge_idx)
        );
        assert!(
            core.transit_network
                .road_surface
                .dirty_nodes()
                .contains(&n0)
        );
        assert!(
            core.transit_network
                .road_surface
                .dirty_nodes()
                .contains(&n1)
        );
        assert!(
            !core
                .transit_network
                .road_surface
                .dirty_terrain_chunks()
                .is_empty()
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
