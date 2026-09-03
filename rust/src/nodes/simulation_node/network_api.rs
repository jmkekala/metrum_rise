// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: network_api.rs
//  script_path: rust/src/nodes/simulation_node/network_api.rs
//  module_name: network_api
//  version: 0.1.0
//  description: Network, road-tool, and traffic-editing Godot API methods.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Network, road-tool, and traffic-editing Godot API methods.

use super::*;

mod road_tool;

// ========================================================================
// NETWORK API
// ========================================================================

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns the ID of the edge hovered by the mouse.
    #[func]
    pub fn get_hovered_edge(&self, world_x: f32, world_z: f32) -> i32 {
        self.lock_core().get_hovered_edge_internal(world_x, world_z)
    }

    /// Returns the raycast depth against the road network.
    #[func]
    pub fn get_max_polygon_depth(
        &self,
        origin_x: f32,
        origin_z: f32,
        dir_x: f32,
        dir_z: f32,
        max_search: f32,
    ) -> f32 {
        self.lock_core()
            .get_max_polygon_depth_internal(origin_x, origin_z, dir_x, dir_z, max_search)
    }

    // ── Network ──

    /// Returns the closest boundary point on a road edge to the given position.
    #[func]
    pub fn get_closest_point_on_edge(&self, edge_idx: i32, point_x: f32, point_y: f32) -> Vector2 {
        self.lock_core()
            .get_closest_point_on_edge_internal(edge_idx, point_x, point_y)
    }

    /// Returns the physical segment geometry for a road edge.
    #[func]
    pub fn get_edge_geometry(&self, edge_idx: i32) -> PackedVector2Array {
        self.lock_core().get_edge_geometry_internal(edge_idx)
    }

    /// Returns the 3D geometry for a road edge.
    #[func]
    pub fn get_edge_geometry_3d(&self, edge_idx: i32) -> PackedVector3Array {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return PackedVector3Array::new();
        }
        let edge = core.region_graph.edge(edge_idx as usize);
        PackedVector3Array::from_iter(edge.physical_geometry.iter().cloned())
    }

    /// Returns the width of a specific road edge.
    #[func]
    pub fn get_edge_width(&self, edge_idx: i32) -> f32 {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return 6.0;
        }
        core.region_graph.edge(edge_idx as usize).width
    }

    /// Returns a curved frontage between two points on an edge.
    #[func]
    pub fn get_curved_frontage(
        &self,
        edge_idx: i32,
        start_p: Vector2,
        end_p: Vector2,
    ) -> PackedVector2Array {
        self.lock_core()
            .get_curved_frontage_internal(edge_idx, start_p, end_p)
    }

    /// Adds a new road segment to the network.
    #[func]
    pub fn add_road(&mut self, points: PackedVector3Array, fwd_lanes: i32, bkw_lanes: i32) {
        self.add_road_with_snap(points, fwd_lanes, bkw_lanes, true);
    }

    /// Adds a road from an authored cross-section.
    ///
    /// `cross_section` is seven integers per band: kind, direction, width in
    /// millimeters, mode bits, marking, turn set, and parking angle. The lane
    /// counts come from the section, so this is how the road builder places a
    /// road with a median, a bus lane, curbside parking, or a planted verge
    /// rather than the nearest pair of numbers.
    ///
    /// A malformed array falls back to a plain one-lane-each-way road, because
    /// a bad payload from the bridge should give an ordinary road rather than
    /// nothing at all.
    #[func]
    pub fn add_road_with_cross_section(
        &mut self,
        points: PackedVector3Array,
        cross_section: PackedInt32Array,
        snap_to_existing_roads: bool,
    ) {
        self.add_road_impl(
            points,
            1,
            1,
            Some(cross_section.to_vec()),
            snap_to_existing_roads,
        );
    }

    /// Adds a new road segment to the network with optional existing-road snapping.
    #[func]
    pub fn add_road_with_snap(
        &mut self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) {
        self.add_road_impl(points, fwd_lanes, bkw_lanes, None, snap_to_existing_roads);
    }

    fn add_road_impl(
        &mut self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        cross_section: Option<Vec<i32>>,
        snap_to_existing_roads: bool,
    ) {
        // Send to the background thread so the Godot main thread is never blocked
        // by the expensive lane-rebuild and zoning-obstruction passes (~500 ms).
        // The road appears on the next sim tick (~16 ms later) — imperceptible delay.
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let point_count = points.len();
        let clone_start = road_debug.then(Instant::now);
        let points = points.to_vec();
        let clone_ms = clone_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let send_start = road_debug.then(Instant::now);
        let send_ok = self
            .cmd_tx
            .send(crate::nodes::sim::core::SimCommand::AddRoad {
                points,
                fwd_lanes,
                bkw_lanes,
                cross_section,
                snap_to_existing_roads,
            })
            .is_ok();
        let send_ms = send_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "add_road_bridge points={} fwd_lanes={} bkw_lanes={} clone_ms={:.3} send_ms={:.3} send_ok={} total_ms={:.3}",
                point_count,
                fwd_lanes,
                bkw_lanes,
                clone_ms,
                send_ms,
                send_ok,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }

    /// Returns the node ID of the nearest graph node near the border, or -1.
    #[func]
    pub fn check_border_candidate(&self, pos: Vector3) -> i64 {
        // Almost every road endpoint is far from the world edge. Reject those against the
        // immutable render snapshot so an in-progress surface compile cannot stall the main
        // thread merely to repeat this bounds check under the authoritative core lock.
        let terrain_world_size = self.snapshot.read().unwrap().terrain_world_size;
        let snapshot_has_extents = terrain_world_size.x > 0.0 && terrain_world_size.y > 0.0;
        if snapshot_has_extents
            && !Self::road_tool_is_near_border(
                pos,
                terrain_world_size.x * 0.5,
                terrain_world_size.y * 0.5,
                config::BORDER_DETECTION_THRESHOLD,
            )
        {
            debug_log!(
                "economy",
                "border candidate rejected at pos=({:.1}, {:.1}, {:.1}) because it is not near the map boundary",
                pos.x,
                pos.y,
                pos.z
            );
            return -1;
        }
        self.lock_core().check_border_candidate_internal(pos)
    }

    /// Marks the node at `node_id` as an external border connection.
    #[func]
    pub fn set_border_connection(&mut self, node_id: i32) {
        self.lock_core().set_border_connection_internal(node_id);
    }

    /// Returns the world-space positions of all active border nodes as a flat float array.
    #[func]
    pub fn get_border_nodes(&self) -> PackedFloat32Array {
        self.lock_core().get_border_nodes_internal()
    }

    /// Returns the classification of an edge as an integer (0=Standard, 1=Bridge, 2=Tunnel).
    /// Returns 0 if the edge index is invalid.
    #[func]
    pub fn get_edge_class(&self, edge_idx: i32) -> u8 {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return 0;
        }
        match core.region_graph.edge(edge_idx as usize).class {
            crate::simulation::network::types::EdgeClass::Bridge => 1,
            crate::simulation::network::types::EdgeClass::Tunnel => 2,
            _ => 0,
        }
    }

    /// Sets the classification of an edge (Standard, Bridge, Tunnel).
    #[func]
    pub fn set_edge_class(&mut self, edge_idx: i32, class_int: u8) {
        self.lock_core()
            .set_edge_class_internal(edge_idx, class_int);
    }

    /// Sets or clears the no-building-spawn flag on an edge. When true the building
    /// allocator skips this edge. Player-toggleable; also auto-set for speed ≥ 80 km/h.
    #[func]
    pub fn set_no_building_spawn(&mut self, edge_idx: i32, enabled: bool) {
        self.lock_core()
            .set_no_building_spawn_internal(edge_idx, enabled);
    }

    /// Sets the vehicle frontage-access policy on an edge.
    ///
    /// `0 = SameSideOnly`, `1 = BothSides`. Invalid values are ignored.
    #[func]
    pub fn set_vehicle_frontage_access(&mut self, edge_idx: i32, access_int: u8) {
        self.lock_core()
            .set_vehicle_frontage_access_internal(edge_idx, access_int);
    }

    /// Returns true if the given edge has the no-building-spawn flag set.
    #[func]
    pub fn get_no_building_spawn(&self, edge_idx: i32) -> bool {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return false;
        }
        core.region_graph.edge(edge_idx as usize).no_building_spawn
    }

    /// Returns the vehicle frontage-access policy on an edge.
    ///
    /// Returns `1` (`BothSides`) if the edge index is invalid.
    #[func]
    pub fn get_vehicle_frontage_access(&self, edge_idx: i32) -> u8 {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return 1;
        }
        match core
            .region_graph
            .edge(edge_idx as usize)
            .vehicle_frontage_access
        {
            crate::simulation::network::types::VehicleFrontageAccess::SameSideOnly => 0,
            crate::simulation::network::types::VehicleFrontageAccess::BothSides => 1,
        }
    }

    /// Returns the start and end node indices of an edge as `Vector2i(start, end)`.
    /// Returns `(-1, -1)` if the edge index is invalid.
    #[func]
    pub fn get_edge_nodes(&self, edge_idx: i32) -> Vector2i {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return Vector2i::new(-1, -1);
        }
        let e = core.region_graph.edge(edge_idx as usize);
        Vector2i::new(e.start_node as i32, e.end_node as i32)
    }

    /// Returns no-build road line segments without waiting when the simulation is busy.
    #[func]
    pub fn try_get_no_building_spawn_lines(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("busy", true);
        let Some(core) = self.try_lock_core() else {
            return dict;
        };
        let mut lines = PackedVector3Array::new();
        for edge in core.region_graph.edges() {
            if edge.deleted || !edge.no_building_spawn {
                continue;
            }
            for segment in edge.physical_geometry.windows(2) {
                lines.push(segment[0]);
                lines.push(segment[1]);
            }
        }
        dict.set("busy", false);
        dict.set("line_vertices", lines);
        dict
    }

    /// Returns a generation-stamped road-mesh batch for the Godot chunk renderer.
    ///
    /// A full snapshot contains every occupied chunk and costs `O(total chunks + total vertices)`.
    /// An incremental response contains only accumulated upserts and removal tombstones since the
    /// last exact-generation acknowledgement. Each response also identifies whether it replaces
    /// all resident chunks and supplies the world-space chunk span used by its local vertices.
    #[func]
    pub fn get_road_mesh_data(&self, full_snapshot: bool) -> VarDictionary {
        if full_snapshot {
            // A renderer rebuild is rare and necessarily uploads the whole network. Clone only
            // the Arc handles under the sim lock, then pack Godot arrays after releasing it.
            let (chunks, generation, chunk_span_m) = {
                let core = self.lock_core();
                (
                    core.cached_road_mesh_chunks.clone(),
                    core.cached_road_mesh_generation,
                    core.transit_network.road_surface.chunk_span_m(),
                )
            };
            return SimCore::road_mesh_chunks_dict(
                &chunks,
                &BTreeSet::new(),
                true,
                generation,
                chunk_span_m,
            );
        }
        let (chunks, pending_chunks, full_replace, generation, chunk_span_m) = {
            let snapshot = self.snapshot.read().unwrap();
            (
                Arc::clone(&snapshot.road_mesh_chunks),
                Arc::clone(&snapshot.pending_road_mesh_chunks),
                snapshot.road_mesh_full_replace,
                snapshot.network_generation,
                snapshot.road_mesh_chunk_span_m,
            )
        };
        SimCore::road_mesh_chunks_dict(
            &chunks,
            &pending_chunks,
            full_replace,
            generation,
            chunk_span_m,
        )
    }

    /// Returns the ID of the closest network node.
    #[func]
    pub fn get_closest_node(&self, world_pos: Vector3, max_dist: f32) -> i32 {
        self.lock_core()
            .get_closest_node_internal(world_pos, max_dist)
    }

    /// Placeholder for cul-de-sac tools.
    #[func]
    pub fn set_node_cul_de_sac(&mut self, _node_id: i32, _enabled: bool, _radius: f32) {}

    /// Placeholder for cul-de-sac tools.
    #[func]
    pub fn has_cul_de_sac(&self, _node_id: i32) -> bool {
        false
    }

    /// Returns the number of road connections for a node.
    #[func]
    pub fn get_node_connection_count(&self, node_id: i32) -> i32 {
        self.lock_core().get_node_connection_count_internal(node_id)
    }

    /// Repositions a network node.
    #[func]
    pub fn move_network_node(&mut self, node_id: i32, pos: Vector3) {
        self.lock_core().move_network_node_internal(node_id, pos);
        self.refresh_snapshot_from_core();
    }

    /// Returns all junction node positions, read from the pre-computed snapshot.
    ///
    /// Reading from the snapshot (RwLock) avoids acquiring the SimCore mutex, which
    /// would stall the Godot main thread while `add_road_internal` holds the lock.
    #[func]
    pub fn get_network_nodes(&self) -> PackedVector3Array {
        PackedVector3Array::from_iter(self.snapshot.read().unwrap().node_positions.iter().copied())
    }

    /// Configures a lane connection rule at a junction.
    #[func]
    pub fn set_lane_connection(
        &mut self,
        node_id: u32,
        from_edge: i32,
        from_lane: i32,
        to_edge: i32,
        to_lane: i32,
    ) {
        self.lock_core()
            .set_lane_connection_internal(node_id, from_edge, from_lane, to_edge, to_lane);
    }

    /// Clears all lane rules at a junction node.
    #[func]
    pub fn clear_lane_connections(&mut self, node_id: u32) {
        self.lock_core().clear_lane_connections_internal(node_id);
    }

    /// Sets a priority sign on one approach arm: 0 main, 1 yield, 2 stop.
    ///
    /// A node carries either priority signs or a signal program, never both, so
    /// this discards any signal already on the node.
    #[func]
    pub fn set_junction_priority(&mut self, node_id: u32, edge_id: i32, sign: u8) {
        self.lock_core()
            .set_junction_priority_internal(node_id, edge_id, sign);
    }

    /// Appends a phase to a junction's signal program.
    ///
    /// `green_arms` holds the approach edges green for the phase. Discards any
    /// priority signs already on the node.
    #[func]
    pub fn add_junction_signal_phase(
        &mut self,
        node_id: u32,
        green_arms: PackedInt32Array,
        green_s: f32,
        amber_s: f32,
    ) {
        self.lock_core()
            .add_junction_signal_phase_internal(node_id, green_arms, green_s, amber_s);
    }

    /// Shifts a junction's signal cycle, which is how a green wave is built.
    #[func]
    pub fn set_junction_signal_offset(&mut self, node_id: u32, offset_s: f32) {
        self.lock_core()
            .set_junction_signal_offset_internal(node_id, offset_s);
    }

    /// Returns a junction to uncontrolled.
    #[func]
    pub fn clear_junction_control(&mut self, node_id: u32) {
        self.lock_core().clear_junction_control_internal(node_id);
    }

    /// Describes a junction's control. See the internal for the shape.
    #[func]
    pub fn get_junction_control(&self, node_id: u32) -> VarDictionary {
        self.lock_core().get_junction_control_internal(node_id)
    }

    /// Which junctions held cars this tick and why, worst first.
    ///
    /// Each row carries `node_id`, `total`, the dominant `cause` with its
    /// `cause_label` and `cause_count`, and a `causes` map of code to count.
    /// Causes: 0 signal, 1 priority sign, 2 gave way, 3 crossing traffic,
    /// 4 connector busy, 5 exit jammed. `limit` of zero or less returns every
    /// junction that held anyone.
    #[func]
    pub fn get_traffic_report(&self, limit: i32) -> VarArray {
        self.lock_core().get_traffic_report_internal(limit)
    }

    /// Aspect one arm shows at `sim_time`: 0 green, 1 amber, 2 red.
    #[func]
    pub fn get_junction_signal_aspect(&self, node_id: u32, edge_id: i32, sim_time: f32) -> u8 {
        self.lock_core()
            .get_junction_signal_aspect_internal(node_id, edge_id, sim_time)
    }

    /// Toggles a user override for a crosswalk at a specific road mouth.
    #[func]
    pub fn set_crosswalk_override(&mut self, node_id: u32, edge_id: i32, enabled: bool) {
        self.lock_core()
            .set_crosswalk_override_internal(node_id, edge_id, enabled);
    }

    /// Returns true if a crosswalk exists natively or by user override.
    #[func]
    pub fn has_crosswalk(&self, node_id: u32, edge_id: i32) -> bool {
        self.lock_core().has_crosswalk_internal(node_id, edge_id)
    }

    /// Returns the world-space position of a node.
    #[func]
    pub fn get_node_pos(&self, node_id: u32) -> Vector3 {
        self.lock_core().get_node_pos_internal(node_id)
    }

    /// Returns information about all lanes entering/leaving a junction.
    #[func]
    pub fn get_node_lanes(&self, node_id: u32) -> VarArray {
        self.lock_core().get_node_lanes_internal(node_id)
    }

    /// Returns an array of current lane turn restrictions at a node.
    #[func]
    pub fn get_lane_connections_array(&self, node_id: u32) -> VarArray {
        self.lock_core()
            .get_lane_connections_array_internal(node_id)
    }

    /// Clears lane rules for a specific source lane.
    #[func]
    pub fn clear_lane_source(&mut self, node_id: u32, from_edge: i32, from_lane: i32) {
        self.lock_core()
            .clear_lane_source_internal(node_id, from_edge, from_lane);
    }

    /// Returns the average network direction at a given point.
    #[func]
    pub fn get_network_direction_at_point(&self, pos: Vector3) -> Vector3 {
        self.lock_core()
            .get_network_direction_at_point_internal(pos)
    }
}
