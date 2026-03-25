//! Logic for modifying simulation state (road placement, terrain sculpt, zoning).

use godot::prelude::*;
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::grid::zoning::{ZoneType};
use crate::config;

impl SimulationNode {
    /// Sculpts the terrain with a given radius and strength.
    pub fn sculpt_terrain_internal(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.push_undo_state(true, false, true, false); 
        self.heightmap.sculpt(pos.x, pos.y, radius, strength);
        self.terrain_dirty = true;
        
        self.transit_network.sync_to_terrain(&mut self.region_graph, &self.heightmap);
        self.flatten_terrain_for_roads_internal();
    }

    /// Adds water to the simulation at a given grid position.
    pub fn add_water_internal(&mut self, pos: Vector2, amount: f32) {
        self.push_undo_state(false, true, false, false);
        self.watermap.add_water(pos.x as usize, pos.y as usize, amount);
    }

    /// Adds a water source to the simulation.
    pub fn add_water_source_internal(&mut self, pos: Vector2, rate_add: f32) {
        self.watermap.update_source(pos.x as usize, pos.y as usize, rate_add);
        self.water_dirty = true;
    }

    /// Sets the zone type for a specific cell.
    pub fn set_zoning_cell_internal(&mut self, edge_idx: i32, side: i8, x: i32, y: i32, zone_type_int: u8) {
        self.push_undo_state(false, false, false, true);
        let zone_type = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            4 => ZoneType::Office,
            5 => ZoneType::Mixed,
            _ => ZoneType::None,
        };
        self.zoning.set_cell(edge_idx as usize, side, x as usize, y as usize, zone_type);
        self.allocator.dirty = true;
    }

    /// Enables or disables zoning on a specific side of a road edge.
    pub fn set_zoning_enabled_internal(&mut self, edge_idx: i32, side: i32, enabled: bool) {
        if let Some(edge) = self.region_graph.edges.get_mut(edge_idx as usize) {
            if side >= 1 { edge.zoning_left = enabled; }
            else if side <= -1 { edge.zoning_right = enabled; }
        }
        self.recalculate_zoning_local(edge_idx as usize);
    }

    /// Adds a new road segment to the transit network.
    pub fn add_road_internal(&mut self, points: PackedVector3Array, fwd_lanes: i32, bkw_lanes: i32, zoning_left: bool, zoning_right: bool) {
        self.push_undo_state(false, false, true, false);
        let mut fixed_points = points.to_vec();
        
        let w = self.heightmap.width;
        let h = self.heightmap.height;
        let hw = (w - 1) as f32 * 0.5;
        let hh = (h - 1) as f32 * 0.5;
        
        for p in &mut fixed_points {
            let gx = p.x + hw;
            let gz = p.z + hh;
            let terrain_h = self.heightmap.get_height_interpolated(gx, gz) * config::HEIGHT_SCALE;
            p.y = terrain_h;
        }

        self.transit_network.add_road(&mut self.region_graph, fixed_points, fwd_lanes as u8, bkw_lanes as u8, zoning_left, zoning_right, &mut self.zoning, &mut self.allocator);

        // Robustly update zoning for all nearby area
        if let Some(first_pt) = points.to_vec().first() {
            let nearby = self.region_graph.get_edges_near_point(*first_pt, 200.0);
            for edge_idx in nearby {
                self.recalculate_zoning_local(edge_idx);
            }
        }
        if let Some(last_pt) = points.to_vec().last() {
            let nearby = self.region_graph.get_edges_near_point(*last_pt, 200.0);
            for edge_idx in nearby {
                self.recalculate_zoning_local(edge_idx);
            }
        }

        // AUTO-COMPACT if technical debt (deleted edges) grows too large
        let total_edges = self.region_graph.edges.len();
        let deleted_edges = self.region_graph.edges.iter().filter(|e| e.deleted).count();
        if deleted_edges > 50 || (total_edges > 0 && deleted_edges as f32 / total_edges as f32 > 0.2) {
            self.perform_edge_compaction_internal();
        }
    }

    /// Repositions a network node in world space.
    pub fn move_network_node_internal(&mut self, node_id: i32, pos: Vector3) {
        if node_id >= 0 && (node_id as usize) < self.region_graph.nodes.len() {
            self.region_graph.move_node(node_id as u32, pos);
            self.push_undo_state(false, false, true, false);
            
            // Recalculate zoning for all affected edges
            let affected_edges: Vec<usize> = self.region_graph.edges.iter().enumerate()
                .filter(|(_, e)| !e.deleted && (e.start_node == node_id as u32 || e.end_node == node_id as u32))
                .map(|(i, _)| i)
                .collect();
            
            for i in affected_edges {
                self.recalculate_zoning_local(i);
            }
        }
    }

    /// Sets a lane connection rule at a junction node.
    pub fn set_lane_connection_internal(&mut self, node_id: u32, from_edge: i32, from_lane: i32, to_edge: i32, to_lane: i32) {
        self.push_undo_state(false, false, true, false);
        if let Some(node) = self.region_graph.nodes.get_mut(node_id as usize) {
            let key = (from_edge as usize, from_lane as i8);
            let target = (to_edge as usize, to_lane as i8);
            if !node.lane_connections.entry(key).or_default().contains(&target) {
                node.lane_connections.get_mut(&key).unwrap().push(target);
            }
        }
        self.transit_network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&self.region_graph);
    }

    /// Clears all lane connections at a junction node.
    pub fn clear_lane_connections_internal(&mut self, node_id: u32) {
        self.push_undo_state(false, false, true, false);
        if let Some(node) = self.region_graph.nodes.get_mut(node_id as usize) {
            node.lane_connections.clear();
        }
        self.transit_network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&self.region_graph);
    }

    /// Clears lane connections for a specific source edge/lane at a junction.
    pub fn clear_lane_source_internal(&mut self, node_id: u32, from_edge: i32, from_lane: i32) {
        if node_id as usize >= self.region_graph.nodes.len() { return; }
        
        {
            let node = &mut self.region_graph.nodes[node_id as usize];
            let key = (from_edge as usize, from_lane as i8);
            node.lane_connections.remove(&key);
        }
        
        self.transit_network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&self.region_graph);
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
        self.transit_network.flatten_terrain(&self.region_graph, &ref_terrain, &mut self.heightmap.data, size);
        self.transit_network.sync_to_terrain(&mut self.region_graph, &self.heightmap);
        self.terrain_dirty = true;
    }

    /// Loads raw heightmap data into the simulation.
    pub fn load_heightmap_data_internal(&mut self, data: PackedFloat32Array) {
        if (data.len() as usize) == self.heightmap.width * self.heightmap.height {
            self.heightmap.data = data.to_vec();
            self.transit_network.sync_to_terrain(&mut self.region_graph, &self.heightmap);
            self.flatten_terrain_for_roads_internal();
            self.terrain_dirty = true;
        }
    }
}
