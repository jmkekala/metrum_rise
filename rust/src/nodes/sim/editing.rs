//! Logic for modifying simulation state (road placement, terrain sculpt, zoning, edge editing).

use crate::config;
use crate::debug_log;
use crate::nodes::sim::core::{
    ROAD_BUILD_COST_PER_METER, ROAD_LOCKED_TERRAIN_RENDER_STEP_M, SERVICE_BUILD_COST_PER_LOT_CELL,
    SimCore,
};
use crate::simulation::buildings::allocator::{
    ExplicitServicePlacementPreview, ExplicitServicePlacementRejection,
};
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::surface::{RoadExtensionReprofile, RoadSurfaceSystem};
use crate::traffic_log;
use godot::prelude::*;
use std::collections::HashSet;
use std::time::Instant;

impl SimCore {
    fn road_geometry_dump_enabled() -> bool {
        std::env::var("METRUM_DEBUG_ROAD_GEOMETRY_DUMP")
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
            self.push_undo_state(true, false, true, false);
            self.terrain_stroke_has_changes = true;
        }
    }

    fn finish_terrain_authoring_edit_internal(&mut self) {
        self.terrain_dirty = true;
        self.cached_road_mesh_data = None;

        self.transit_network
            .sync_to_terrain(&mut self.region_graph, &self.heightmap);
        self.transit_network
            .rebuild_dirty_terrain_earthworks(&self.region_graph, &mut self.heightmap);
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

    fn rebuild_building_entrances_internal(&mut self) {
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

    /// Sculpts the terrain with a given radius and strength.
    pub fn sculpt_terrain_internal(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.push_undo_state(true, false, true, false);
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap
            .sculpt(center_x, center_y, radius_cells, strength);
        self.transit_network
            .mark_surface_dirty_for_terrain_edit(&self.region_graph, pos, radius);
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
        self.terrain_dirty = true;
    }

    /// Moves terrain toward one target rendered height in a circular area.
    pub fn level_terrain_internal(
        &mut self,
        pos: Vector2,
        radius: f32,
        target_height_m: f32,
        strength: f32,
    ) {
        self.push_undo_state(true, false, true, false);
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
        self.finish_terrain_authoring_edit_internal();
    }

    /// Smooths terrain toward the local neighborhood average in a circular area.
    pub fn smooth_terrain_internal(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.push_undo_state(true, false, true, false);
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap
            .smooth(center_x, center_y, radius_cells, strength);
        self.transit_network
            .mark_surface_dirty_for_terrain_edit(&self.region_graph, pos, radius);
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
        self.push_undo_state(true, false, true, false);
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
        self.terrain_dirty = true;
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
        self.terrain_dirty = true;
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
        self.terrain_dirty = true;
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
    pub fn add_road_internal(
        &mut self,
        points: Vec<godot::prelude::Vector3>,
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) {
        let fwd_lanes_u8 = fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let bkw_lanes_u8 = bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let prepared_input =
            RoadSurfaceSystem::prepare_road_input_with_extension_to_visible_surface(
                &points,
                &self.heightmap,
                &self.region_graph,
                &self.transit_network.road_surface,
            );
        let fixed_points = prepared_input.points.clone();

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
        let validation = self
            .transit_network
            .road_surface
            .validate_prepared_road_input_against_graph(
                &prepared_input,
                fwd_lanes_u8,
                bkw_lanes_u8,
                &self.heightmap,
                &self.region_graph,
                new_edge_validation,
            );
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
            return;
        }

        let t_undo = Instant::now();
        if !self.benchmark_mode {
            self.push_undo_state(false, false, true, false);
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
        self.transit_network.add_road(
            &mut self.region_graph,
            fixed_points,
            fwd_lanes_u8,
            bkw_lanes_u8,
            prepared_input.class,
            &mut self.zoning,
            &mut self.allocator,
        );
        let dt_topo_us = t_topo.elapsed().as_micros();

        // Deduct road build cost from city treasury (skipped in benchmark mode).
        if !self.benchmark_mode {
            self.treasury
                .deduct_build_cost(build_length_m * ROAD_BUILD_COST_PER_METER);
        }

        self.network_dirty = true;
        self.cached_road_mesh_data = None;

        // Store partial timing so the AddRoad handler can append the remaining phases.
        // Zoning is NOT flushed here — create_edge_internal already called
        // invalidate_zoning_near_edge (125 m radius) for every new/split edge.
        // The AddRoad handler calls flush_zoning_updates once after lane rebuild,
        // batching all dirty edges into a single pass instead of N separate passes.
        self.last_road_timing = format!("undo={}µs topo={}µs", dt_undo_ms, dt_topo_us);
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
            self.agents
                .invalidate_lane_ids_for_edges(&affected_edges, &self.transit_network.lane_system);
            self.transit_network
                .lane_system
                .rebuild_edges_incremental(&mut self.region_graph, &affected_edges);
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

            self.region_graph.move_node(node_id as u32, pos);
            for &edge_idx in &affected_edges {
                let length = self
                    .region_graph
                    .calculate_length(&self.region_graph.edge(edge_idx).physical_geometry);
                self.region_graph.edge_mut(edge_idx).physical_length = length;
            }
            self.region_graph.rebuild_intersection_clips();
            self.push_undo_state(false, false, true, false);
            self.agents
                .invalidate_lane_ids_for_edges(&affected_edges, &self.transit_network.lane_system);
            self.transit_network
                .lane_system
                .rebuild_edges_incremental(&mut self.region_graph, &affected_edges);
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
            self.network_dirty = true;
            self.cached_road_mesh_data = None;
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
        self.push_undo_state(false, false, true, false);
        traffic_log!(
            "[LANE_EDIT] set_lane_connection: node={node_id} from_edge={from_edge} from_lane={from_lane} to_edge={to_edge} to_lane={to_lane}"
        );
        if (node_id as usize) < self.region_graph.node_count() {
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
        self.agents
            .invalidate_lane_ids_for_edges(&affected, &self.transit_network.lane_system);
        self.transit_network
            .lane_system
            .rebuild_edges_incremental(&mut self.region_graph, &affected);
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
        self.push_undo_state(false, false, true, false);
        if (node_id as usize) < self.region_graph.node_count() {
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
        self.agents
            .invalidate_lane_ids_for_edges(&affected, &self.transit_network.lane_system);
        self.transit_network
            .lane_system
            .rebuild_edges_incremental(&mut self.region_graph, &affected);
        self.rebuild_building_entrances_internal();
        self.transit_network
            .rebuild_cch_and_check(&self.region_graph);
        self.transit_network.flow_fields.mark_all_dirty();
        if (node_id as usize) < self.region_graph.node_count() {
            let pos = self.region_graph.node(node_id).pos;
            self.transit_network.mark_point_dirty(pos);
        }
    }

    /// Clears lane connections for a specific source edge/lane at a junction.
    pub fn clear_lane_source_internal(&mut self, node_id: u32, from_edge: i32, from_lane: i32) {
        if node_id as usize >= self.region_graph.node_count() {
            return;
        }

        self.push_undo_state(false, false, true, false);
        self.region_graph
            .remove_lane_connection(node_id, (from_edge as usize, from_lane as i8));

        let affected: HashSet<usize> = self
            .region_graph
            .node_adjacency(node_id)
            .iter()
            .copied()
            .collect();
        self.agents
            .invalidate_lane_ids_for_edges(&affected, &self.transit_network.lane_system);
        self.transit_network
            .lane_system
            .rebuild_edges_incremental(&mut self.region_graph, &affected);
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
        self.push_undo_state(false, false, true, false);
        self.region_graph
            .set_crosswalk_override(node_id, edge_id as usize, enabled);

        let affected: HashSet<usize> = self
            .region_graph
            .node_adjacency(node_id)
            .iter()
            .copied()
            .collect();
        self.agents
            .invalidate_lane_ids_for_edges(&affected, &self.transit_network.lane_system);
        self.transit_network
            .lane_system
            .rebuild_edges_incremental(&mut self.region_graph, &affected);
        self.rebuild_building_entrances_internal();
        if (node_id as usize) < self.region_graph.node_count() {
            let pos = self.region_graph.node(node_id).pos;
            self.transit_network.mark_point_dirty(pos);
        }
    }

    /// Rebuilds road-surface-driven visual terrain after network edits.
    pub fn rebuild_network_surface_terrain_internal(&mut self) {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let debug_edges = std::mem::take(&mut self.last_surface_debug_edges);

        let earthwork_start = road_debug.then(Instant::now);
        let dirty_chunks = self
            .transit_network
            .rebuild_dirty_terrain_earthworks(&self.region_graph, &mut self.heightmap);
        let road_locked_dirty_patches = self
            .transit_network
            .road_surface
            .mark_render_patches_for_chunk_grading_envelopes(
                &self.region_graph,
                &mut self.heightmap,
                &dirty_chunks,
                ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
            );
        let earthwork_ms = earthwork_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let entrances_start = road_debug.then(Instant::now);
        self.rebuild_building_entrances_internal();
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
            let dump = self
                .transit_network
                .road_surface
                .build_edge_geometry_debug_dump(&self.region_graph, &self.heightmap, &debug_edges);
            dump_build_ms = dump_start.elapsed().as_secs_f64() * 1000.0;
            dump_bytes = dump.len();
            let dump_print_start = Instant::now();
            debug_log!("road", "{}", dump);
            dump_print_ms = dump_print_start.elapsed().as_secs_f64() * 1000.0;
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
    use super::SimCore;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::core::time::TimeSystem;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::economy::demand::DemandSystem;
    use crate::simulation::economy::households::HouseholdSystem;
    use crate::simulation::economy::logistics::ShipmentSystem;
    use crate::simulation::grid::desirability::DesirabilitySystem;
    use crate::simulation::grid::noise::NoiseSystem;
    use crate::simulation::grid::pollution::PollutionSystem;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::water::WaterSystem;
    use crate::simulation::zoning::{ZoneType, ZoningSystem};
    use godot::prelude::{Vector2, Vector3};
    use std::collections::{HashMap, VecDeque};

    fn test_core() -> SimCore {
        use crate::nodes::sim::core::CityTreasury;
        let config = WorldConfig::default();
        SimCore {
            time: TimeSystem::new(),
            heightmap: TerrainSystem::from_world_config(&config),
            watermap: WaterSystem::from_world_config(&config),
            region_graph: RegionGraph::new(),
            transit_network: TransitNetwork::new_with_surface_chunk_span(config.terrain_chunk_m),
            zoning: ZoningSystem::new(&config),
            pollution: PollutionSystem::new(&config),
            noise: NoiseSystem::new(&config),
            desirability: DesirabilitySystem::new(&config),
            demand: DemandSystem::new(),
            allocator: BuildingAllocator::new(),
            agents: AgentSystem::new(),
            households: HouseholdSystem::new(),
            logistics: ShipmentSystem::new(),
            config,
            treasury: CityTreasury::new(0.0),
            debug_household_admissions_since_daily: 0,
            undo_stack: VecDeque::new(),
            world_lake_fills: Vec::new(),
            world_open_water_fills: Vec::new(),
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
            water_patch_mesh_cache: HashMap::new(),
            road_locked_terrain_patch_keys: Vec::new(),
            cached_road_mesh_data: None,
            camera_aabb: (0.0, 0.0, 0.0, 0.0),
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
            fwd_lanes: 1,
            bkw_lanes: 1,
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
            fwd_lanes: 1,
            bkw_lanes: 1,
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
            fwd_lanes: 1,
            bkw_lanes: 1,
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
}
