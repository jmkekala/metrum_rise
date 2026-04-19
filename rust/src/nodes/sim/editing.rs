//! Logic for modifying simulation state (road placement, terrain sculpt, zoning, edge editing).

use crate::config;
use crate::debug_log;
use crate::nodes::sim::core::{ROAD_BUILD_COST_PER_METER, SimCore};
use crate::traffic_log;
use godot::prelude::*;
use std::collections::HashSet;
use std::time::Instant;

impl SimCore {
    fn rebuild_building_entrances_internal(&mut self) {
        self.allocator
            .rebuild_entrance_cache(&self.region_graph, &self.transit_network.lane_system);
    }

    /// Sculpts the terrain with a given radius and strength.
    pub fn sculpt_terrain_internal(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.push_undo_state(true, false, true, false);
        let (center_x, center_y) = self.heightmap.world_to_grid_coords(pos.x, pos.y);
        let radius_cells = radius / self.config.terrain_cell_m;
        self.heightmap
            .sculpt(center_x, center_y, radius_cells, strength);
        self.terrain_dirty = true;

        self.transit_network
            .sync_to_terrain(&mut self.region_graph, &self.heightmap);
        self.flatten_terrain_for_roads_internal();
        if self.has_authored_water_internal() {
            if let Err(err) = self.rebuild_authored_water_preview_internal() {
                debug_log!("world-editor", "rebuild_authored_water_after_sculpt failed: {}", err);
            }
        }
    }

    /// Adds water to the simulation at a given grid position.
    pub fn add_water_internal(&mut self, pos: Vector2, amount: f32) {
        self.push_undo_state(false, true, false, false);
        let (grid_x, grid_y) = self.watermap.world_to_grid_cell_clamped(pos.x, pos.y);
        self.watermap.add_water(grid_x, grid_y, amount);
    }

    /// Adds a water source to the simulation.
    pub fn add_water_source_internal(&mut self, pos: Vector2, rate_add: f32) {
        let (grid_x, grid_y) = self.watermap.world_to_grid_cell_clamped(pos.x, pos.y);
        self.watermap.update_source(grid_x, grid_y, rate_add);
        self.water_dirty = true;
    }

    /// Captures one zoning patch bounding box as packed little-endian runtime ids.
    pub fn capture_zoning_patch_internal(
        &self,
        grid_x: i32,
        grid_y: i32,
        width_cells: i32,
        height_cells: i32,
    ) -> Vec<u8> {
        if width_cells <= 0 || height_cells <= 0 {
            return Vec::new();
        }
        self.zoning
            .capture_patch(grid_x, grid_y, width_cells as usize, height_cells as usize)
    }

    /// Applies one masked zoning paint patch.
    pub fn apply_zoning_patch_internal(
        &mut self,
        grid_x: i32,
        grid_y: i32,
        width_cells: i32,
        height_cells: i32,
        target_profile_runtime_id: i32,
        write_mask: Vec<u8>,
    ) {
        if width_cells <= 0 || height_cells <= 0 {
            return;
        }
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return;
        };
        if runtime_id != 0
            && self
                .zoning
                .profiles
                .profile_by_runtime_id(runtime_id)
                .is_none()
        {
            return;
        }
        self.zoning.apply_patch(
            grid_x,
            grid_y,
            width_cells as usize,
            height_cells as usize,
            runtime_id,
            &write_mask,
        );
        self.allocator.dirty = true;
    }

    /// Restores one full zoning patch bounding box from packed little-endian runtime ids.
    pub fn restore_zoning_patch_internal(
        &mut self,
        grid_x: i32,
        grid_y: i32,
        width_cells: i32,
        height_cells: i32,
        profile_ids_le_u16: Vec<u8>,
    ) {
        if width_cells <= 0 || height_cells <= 0 {
            return;
        }
        self.zoning.restore_patch(
            grid_x,
            grid_y,
            width_cells as usize,
            height_cells as usize,
            &profile_ids_le_u16,
        );
        self.allocator.dirty = true;
    }

    /// Sets the classification of an edge.
    /// Sets or clears the no-building-spawn flag on an edge.
    pub fn set_no_building_spawn_internal(&mut self, edge_idx: i32, enabled: bool) {
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edge_count() {
            return;
        }
        self.region_graph
            .edge_mut(edge_idx as usize)
            .no_building_spawn = enabled;
        self.zoning.update_no_build_mask(&self.region_graph);
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

        let class = match class_int {
            1 => crate::simulation::network::types::EdgeClass::Bridge,
            2 => crate::simulation::network::types::EdgeClass::Tunnel,
            _ => crate::simulation::network::types::EdgeClass::Standard,
        };

        {
            let edge = self.region_graph.edge_mut(edge_idx as usize);
            edge.class = class;
        }

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
        let t_undo = Instant::now();
        if !self.benchmark_mode {
            self.push_undo_state(false, false, true, false);
        }
        let dt_undo_ms = t_undo.elapsed().as_micros();

        let mut fixed_points = points;

        let mut class = crate::simulation::network::types::EdgeClass::Standard;

        for p in &mut fixed_points {
            let terrain_h = self.heightmap.sample_height_world(p.x, p.z) * config::HEIGHT_SCALE;

            if p.y - terrain_h > 1.0 {
                class = crate::simulation::network::types::EdgeClass::Bridge;
            } else if terrain_h - p.y > 1.0 {
                class = crate::simulation::network::types::EdgeClass::Tunnel;
            }
        }

        // Only force grounded roads to terrain
        if class == crate::simulation::network::types::EdgeClass::Standard {
            for p in &mut fixed_points {
                p.y = self.heightmap.sample_height_world(p.x, p.z) * config::HEIGHT_SCALE;
            }
        }

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
            fwd_lanes as u8,
            bkw_lanes as u8,
            class,
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

        // Store partial timing so the AddRoad handler can append the remaining phases.
        // Zoning is NOT flushed here — create_edge_internal already called
        // invalidate_zoning_near_edge (125 m radius) for every new/split edge.
        // The AddRoad handler calls flush_zoning_updates once after lane rebuild,
        // batching all dirty edges into a single pass instead of N separate passes.
        self.last_road_timing = format!("undo={}µs topo={}µs", dt_undo_ms, dt_topo_us);
    }

    /// Repositions a network node in world space.
    pub fn move_network_node_internal(&mut self, node_id: i32, pos: Vector3) {
        if node_id >= 0 && (node_id as usize) < self.region_graph.node_count() {
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
            self.network_dirty = true;
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

    /// Flattens the terrain to match the grade of the road network.
    pub fn flatten_terrain_for_roads_internal(&mut self) {
        let size = self.get_terrain_world_size_internal();
        self.heightmap.reset_visuals_from_source();
        let mut flattened = self.heightmap.clone_visual_dense();
        self.transit_network.flatten_terrain(
            &self.region_graph,
            &self.heightmap,
            &mut flattened,
            size,
        );
        self.heightmap
            .replace_visual_from_dense(&flattened)
            .expect("road flatten output must match the live terrain dimensions");
        self.transit_network
            .sync_to_terrain(&mut self.region_graph, &self.heightmap);
        self.rebuild_building_entrances_internal();
        self.terrain_dirty = true;
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
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::water::WaterSystem;
    use godot::prelude::Vector3;
    use std::collections::VecDeque;

    fn test_core() -> SimCore {
        use crate::nodes::sim::core::CityTreasury;
        let config = WorldConfig::default();
        SimCore {
            time: TimeSystem::new(),
            heightmap: TerrainSystem::from_world_config(&config),
            watermap: WaterSystem::from_world_config(&config),
            region_graph: RegionGraph::new(),
            transit_network: TransitNetwork::new(),
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
            undo_stack: VecDeque::new(),
            world_water_boundary_points: Vec::new(),
            world_lake_fills: Vec::new(),
            world_open_water_fills: Vec::new(),
            world_lake_fill_preview: None,
            terrain_dirty: false,
            water_dirty: false,
            network_dirty: false,
            benchmark_mode: true,
            last_tick_duration: 0.0,
            last_agent_tick_us: 0,
            last_road_timing: String::new(),
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
}
