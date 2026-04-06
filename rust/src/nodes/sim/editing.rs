//! Logic for modifying simulation state (road placement, terrain sculpt, zoning).

use crate::config;
use crate::nodes::sim::core::SimCore;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::*;
use std::time::Instant;

impl SimCore {
    /// Sculpts the terrain with a given radius and strength.
    pub fn sculpt_terrain_internal(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.push_undo_state(true, false, true, false);
        self.heightmap.sculpt(pos.x, pos.y, radius, strength);
        self.terrain_dirty = true;

        self.transit_network
            .sync_to_terrain(&mut self.region_graph, &self.heightmap);
        self.flatten_terrain_for_roads_internal();
    }

    /// Adds water to the simulation at a given grid position.
    pub fn add_water_internal(&mut self, pos: Vector2, amount: f32) {
        self.push_undo_state(false, true, false, false);
        self.watermap
            .add_water(pos.x as usize, pos.y as usize, amount);
    }

    /// Adds a water source to the simulation.
    pub fn add_water_source_internal(&mut self, pos: Vector2, rate_add: f32) {
        self.watermap
            .update_source(pos.x as usize, pos.y as usize, rate_add);
        self.water_dirty = true;
    }

    /// Paints a world-space rectangle with the given zone type.
    ///
    /// Coordinates in metres, snapped to the 10 m cell grid. `zone_type_int` 0 = erase.
    pub fn set_zone_rect_internal(
        &mut self,
        x_min: f32,
        z_min: f32,
        x_max: f32,
        z_max: f32,
        zone_type_int: u8,
    ) {
        self.push_undo_state(false, false, false, true);
        let zone_type = ZoneType::from_u8(zone_type_int);
        self.zoning.set_zone_rect(x_min, z_min, x_max, z_max, zone_type);
        self.allocator.dirty = true;
    }

    /// Restores a raw zone sub-rectangle. Used exclusively by the GDScript undo path.
    pub fn set_zone_rect_raw_internal(
        &mut self,
        x_min: f32,
        z_min: f32,
        x_max: f32,
        z_max: f32,
        bytes: Vec<u8>,
    ) {
        self.zoning.set_zone_rect_raw(x_min, z_min, x_max, z_max, &bytes);
        self.allocator.dirty = true;
    }

    /// Captures the zone bytes of a sub-rectangle. Called before painting for undo state.
    pub fn get_zone_subrect_internal(
        &self,
        x_min: f32,
        z_min: f32,
        x_max: f32,
        z_max: f32,
    ) -> Vec<u8> {
        self.zoning.get_zone_subrect(x_min, z_min, x_max, z_max)
    }


    /// Sets the classification of an edge.
    /// Sets or clears the no-building-spawn flag on an edge.
    pub fn set_no_building_spawn_internal(&mut self, edge_idx: i32, enabled: bool) {
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edge_count() {
            return;
        }
        self.region_graph.edge_mut(edge_idx as usize).no_building_spawn = enabled;
        self.zoning.update_no_build_mask(&self.region_graph);
        self.allocator.dirty = true;
    }

    /// Sets the classification of an edge by integer class code.
    pub fn set_edge_class_internal(&mut self, edge_idx: i32, class_int: u8) {
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edge_count() {
            return;
        }

        let class = match class_int {
            1 => crate::simulation::network::types::EdgeClass::Bridge,
            2 => crate::simulation::network::types::EdgeClass::Tunnel,
            _ => crate::simulation::network::types::EdgeClass::Standard,
        };

        {
            let edge = self.region_graph.edge_mut(edge_idx as usize);
            edge.class = class;
        }

        self.transit_network.cch_graph =
            crate::simulation::pathing::cch::CchGraph::build(&self.region_graph);
    }

    /// Adds a new road segment to the transit network.
    pub fn add_road_internal(
        &mut self,
        points: Vec<godot::prelude::Vector3>,
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) {
        let t_undo = Instant::now();
        if !self.benchmark_mode {
            self.push_undo_state(false, false, true, false);
        }
        let dt_undo_ms = t_undo.elapsed().as_micros();

        let mut fixed_points = points;

        let w = self.heightmap.width;
        let h = self.heightmap.height;
        let hw = (w - 1) as f32 * 0.5;
        let hh = (h - 1) as f32 * 0.5;

        let mut class = crate::simulation::network::types::EdgeClass::Standard;

        for p in &mut fixed_points {
            let gx = p.x + hw;
            let gz = p.z + hh;
            let terrain_h = self.heightmap.get_height_interpolated(gx, gz) * config::HEIGHT_SCALE;

            if p.y - terrain_h > 1.0 {
                class = crate::simulation::network::types::EdgeClass::Bridge;
            } else if terrain_h - p.y > 1.0 {
                class = crate::simulation::network::types::EdgeClass::Tunnel;
            }
        }

        // Only force grounded roads to terrain
        if class == crate::simulation::network::types::EdgeClass::Standard {
            for p in &mut fixed_points {
                let gx = p.x + hw;
                let gz = p.z + hh;
                p.y = self.heightmap.get_height_interpolated(gx, gz) * config::HEIGHT_SCALE;
            }
        }

        let t_topo = Instant::now();
        self.transit_network.add_road(
            &mut self.region_graph,
            fixed_points,
            fwd_lanes as u8,
            bkw_lanes as u8,
            class,
            &mut self.zoning,
            &mut self.allocator,
        );
        let dt_topo_us = t_topo.elapsed().as_micros();

        self.network_dirty = true;

        // Store partial timing so the AddRoad handler can append the remaining phases.
        // Zoning is NOT flushed here — create_edge_internal already called
        // invalidate_zoning_near_edge (125 m radius) for every new/split edge.
        // The AddRoad handler calls flush_zoning_updates once after lane rebuild,
        // batching all dirty edges into a single pass instead of N separate passes.
        self.last_road_timing = format!(
            "undo={}µs topo={}µs",
            dt_undo_ms, dt_topo_us
        );
    }

    /// Repositions a network node in world space.
    pub fn move_network_node_internal(&mut self, node_id: i32, pos: Vector3) {
        if node_id >= 0 && (node_id as usize) < self.region_graph.node_count() {
            self.region_graph.move_node(node_id as u32, pos);
            self.region_graph.rebuild_intersection_clips();
            self.push_undo_state(false, false, true, false);

            // Recalculate zoning for all affected edges
            let affected_edges: Vec<usize> = self
                .region_graph
                .edges()
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    !e.deleted && (e.start_node == node_id as u32 || e.end_node == node_id as u32)
                })
                .map(|(i, _)| i)
                .collect();

            // Zone lookup is now world-grid based; no per-edge recalculation needed.
            let _ = affected_edges;
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
        if (node_id as usize) < self.region_graph.node_count() {
            let key = (from_edge as usize, from_lane as i8);
            let target = (to_edge as usize, to_lane as i8);
            let already = self.region_graph.node(node_id)
                .lane_connections
                .get(&key)
                .map_or(false, |v| v.contains(&target));
            if !already {
                self.region_graph.add_lane_connection(node_id, key.0, key.1, target.0, target.1);
            }
        }
        self.transit_network.cch_graph =
            crate::simulation::pathing::cch::CchGraph::build(&self.region_graph);
    }

    /// Clears all lane connections at a junction node.
    pub fn clear_lane_connections_internal(&mut self, node_id: u32) {
        self.push_undo_state(false, false, true, false);
        if (node_id as usize) < self.region_graph.node_count() {
            let keys: Vec<_> = self.region_graph.node(node_id).lane_connections.keys().copied().collect();
            for key in keys {
                self.region_graph.remove_lane_connection(node_id, key);
            }
        }
        self.transit_network.cch_graph =
            crate::simulation::pathing::cch::CchGraph::build(&self.region_graph);
    }

    /// Clears lane connections for a specific source edge/lane at a junction.
    pub fn clear_lane_source_internal(&mut self, node_id: u32, from_edge: i32, from_lane: i32) {
        if node_id as usize >= self.region_graph.node_count() {
            return;
        }

        self.region_graph
            .remove_lane_connection(node_id, (from_edge as usize, from_lane as i8));

        self.transit_network.cch_graph =
            crate::simulation::pathing::cch::CchGraph::build(&self.region_graph);
    }

    /// Flattens the terrain to match the grade of the road network.
    pub fn flatten_terrain_for_roads_internal(&mut self) {
        let size = self.get_heightmap_size_internal();
        self.heightmap.reset_visuals_from_source();

        let ref_terrain = TerrainSystem {
            width: self.heightmap.width,
            height: self.heightmap.height,
            data: self.heightmap.data.clone(),
            source_data: self.heightmap.source_data.clone(),
        };
        self.transit_network.flatten_terrain(
            &self.region_graph,
            &ref_terrain,
            &mut self.heightmap.data,
            size,
        );
        self.transit_network
            .sync_to_terrain(&mut self.region_graph, &self.heightmap);
        self.terrain_dirty = true;
    }

    /// Loads raw heightmap data into the simulation.
    pub fn load_heightmap_data_internal(&mut self, data: PackedFloat32Array) {
        if (data.len() as usize) == self.heightmap.width * self.heightmap.height {
            self.heightmap.data = data.to_vec();
            self.transit_network
                .sync_to_terrain(&mut self.region_graph, &self.heightmap);
            self.flatten_terrain_for_roads_internal();
            self.terrain_dirty = true;
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
        let half_w = (self.heightmap.width as f32 - 1.0) * 0.5;
        let half_h = (self.heightmap.height as f32 - 1.0) * 0.5;
        let t = config::BORDER_DETECTION_THRESHOLD;

        let near_border = pos.x < -half_w + t
            || pos.x > half_w - t
            || pos.z < -half_h + t
            || pos.z > half_h - t;

        if !near_border {
            return -1;
        }

        // Use a generous tolerance: the node was snapped during add_road so it should be
        // very close, but terrain raycast imprecision may add a few metres of offset.
        match crate::simulation::network::interaction::get_closest_node(
            &self.region_graph,
            pos,
            config::SNAP_TOLERANCE * 5.0,
        ) {
            Some(id) => id as i64,
            None => -1,
        }
    }

    /// Designates the node at `node_id` as an external border connection.
    ///
    /// After this call the node's type becomes [`NodeType::Border`] and it will be used as an
    /// immigrant spawn point by [`BuildingAllocator::tick`] as long as the road remains connected.
    pub fn set_border_connection_internal(&mut self, node_id: i32) {
        if node_id < 0 || (node_id as usize) >= self.region_graph.node_count() {
            return;
        }

        self.region_graph
            .set_node_type(node_id as u32, crate::simulation::network::types::NodeType::Border);

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
                    edge.base_cost = crate::simulation::pathing::cost::CostCalculator::calculate_costs(edge).0;
                    rebuild_needed = true;
                }
            }
        }

        if rebuild_needed {
            self.transit_network.lane_system.rebuild(&mut self.region_graph);
            self.transit_network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&self.region_graph);
        }
    }
}
