// SPDX-License-Identifier: GPL-2.0-only

//! Network, road-tool, and traffic-editing Godot API methods.

use super::*;
use crate::nodes::sim::core::RoadRejection;
use crate::simulation::network::graph::RoadMedian;
use crate::simulation::network::road_types::RoadAttributes;

mod road_tool;
mod road_types;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns the ID of the edge hovered by the mouse.
    #[func]
    pub fn get_hovered_edge(&self, world_x: f32, world_z: f32) -> i32 {
        self.lock_core().get_hovered_edge_internal(world_x, world_z)
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

    /// Adds a new road segment to the network.
    #[func]
    pub fn add_road(&mut self, points: PackedVector3Array, fwd_lanes: i32, bkw_lanes: i32) {
        self.add_road_with_snap(points, fwd_lanes, bkw_lanes, true);
    }

    /// Queues a road and returns its completion identity; this does not report acceptance.
    /// Only the latest request retains feedback, polled once with `get_road_commit_result`.
    #[func]
    pub fn add_road_with_snap(
        &mut self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> i64 {
        // Send to the background thread so the Godot main thread is never blocked
        // by lane rebuilding and zoning work. The command wakes the existing simulation thread
        // without waiting for, or advancing, its next movement tick.
        let road_debug = crate::debug::Channel::Road.enabled();
        let total_start = road_debug.then(Instant::now);
        let point_count = points.len();
        let clone_start = road_debug.then(Instant::now);
        let points = points.to_vec();
        let clone_ms = clone_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let preview = self.road_tool.preview_for(
            &points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads,
            self.road_tool_tunnel,
            (self.road_tool_median, self.road_tool_attributes),
        );
        let send_start = road_debug.then(Instant::now);
        let (completion, result) = std::sync::mpsc::sync_channel(1);
        self.road_commit_request_id = self.road_commit_request_id.wrapping_add(1).max(1);
        self.road_commit_result = Some(result);
        let send_ok = self
            .cmd_tx
            .send(crate::nodes::sim::core::SimCommand::AddRoad {
                points,
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
                tunnel: self.road_tool_tunnel,
                median: self.road_tool_median,
                attributes: self.road_tool_attributes,
                enqueued_at: Instant::now(),
                completion,
                preview,
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
        self.road_commit_request_id
    }

    /// Consumes the latest placement's acceptance or rejection without locking the simulation:
    /// `committed`, `reason` (the stable key the `events` report uses) and `detail`. Nil means
    /// pending/already consumed; a superseded id or disconnected worker is rejected.
    #[func]
    pub fn get_road_commit_result(&mut self, request_id: i64) -> Variant {
        if request_id != self.road_commit_request_id {
            return vdict! {
                "committed": false,
                "reason": "road_request_replaced",
                "detail": "road_request_replaced",
            }
            .to_variant();
        }
        let Some(receiver) = self.road_commit_result.as_ref() else {
            return Variant::nil();
        };
        let result = match receiver.try_recv() {
            Ok(ack) => vdict! {
                "committed": ack.committed,
                "reason": ack.reason,
                "detail": ack.detail,
                "generation": i64::try_from(ack.generation).unwrap_or(i64::MAX),
            },
            Err(std::sync::mpsc::TryRecvError::Empty) => return Variant::nil(),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                vdict! {
                    "committed": false,
                    "reason": "road_command_unavailable",
                    "detail": "road_command_unavailable",
                }
            }
        };
        self.road_commit_result = None;
        result.to_variant()
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
    /// all resident chunks and supplies the world-space chunk span and origin used by its local
    /// vertices.
    #[func]
    pub fn get_road_mesh_data(&self, full_snapshot: bool) -> VarDictionary {
        if full_snapshot {
            // A renderer rebuild is rare and necessarily uploads the whole network. Share the
            // persistent chunk map under the sim lock, then pack Godot arrays after releasing it.
            let (chunks, generation, chunk_span_m, chunk_origin_x_m, chunk_origin_z_m) = {
                let core = self.lock_core();
                let (chunk_origin_x_m, chunk_origin_z_m) =
                    core.transit_network.roadbed.chunk_origin_m();
                (
                    core.cached_road_mesh_chunks.clone(),
                    core.cached_road_mesh_generation,
                    core.transit_network.roadbed.chunk_span_m(),
                    chunk_origin_x_m,
                    chunk_origin_z_m,
                )
            };
            return SimCore::road_mesh_chunks_dict(
                &chunks.into_iter().collect(),
                &BTreeSet::new(),
                true,
                generation,
                chunk_span_m,
                chunk_origin_x_m,
                chunk_origin_z_m,
            );
        }
        let (
            chunks,
            pending_chunks,
            full_replace,
            generation,
            chunk_span_m,
            chunk_origin_x_m,
            chunk_origin_z_m,
        ) = {
            let snapshot = self.snapshot.read().unwrap();
            (
                Arc::clone(&snapshot.road_mesh_chunks),
                Arc::clone(&snapshot.pending_road_mesh_chunks),
                snapshot.road_mesh_full_replace,
                snapshot.network_generation,
                snapshot.road_mesh_chunk_span_m,
                snapshot.road_mesh_chunk_origin_x_m,
                snapshot.road_mesh_chunk_origin_z_m,
            )
        };
        SimCore::road_mesh_chunks_dict(
            &chunks,
            &pending_chunks,
            full_replace,
            generation,
            chunk_span_m,
            chunk_origin_x_m,
            chunk_origin_z_m,
        )
    }

    /// Returns the ID of the closest network node.
    #[func]
    pub fn get_closest_node(&self, world_pos: Vector3, max_dist: f32) -> i32 {
        self.lock_core()
            .get_closest_node_internal(world_pos, max_dist)
    }

    /// Returns the number of road connections for a node.
    #[func]
    pub fn get_node_connection_count(&self, node_id: i32) -> i32 {
        self.lock_core().get_node_connection_count_internal(node_id)
    }

    /// Repositions a network node, returning false when reservations reject the move.
    #[func]
    pub fn move_network_node(&mut self, node_id: i32, pos: Vector3) -> bool {
        let moved = self.lock_core().move_network_node_internal(node_id, pos);
        if moved {
            self.refresh_snapshot_from_core();
        }
        moved
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

    /// Turns the crossing of road arm `edge_id` at `node_id` on or off (`ROAD-74`), with an undo
    /// record; off removes its zebra, strip, crossing lanes and signs. Returns whether it was
    /// applied: a refused toggle (see [`Self::preview_crosswalk`]) changes nothing.
    #[func]
    pub fn set_crosswalk_override(&mut self, node_id: u32, edge_id: i32, enabled: bool) -> bool {
        let (snapshot, publication) = {
            let mut core = self.lock_core();
            if !core.set_crosswalk_override_internal(node_id, edge_id, enabled) {
                return false;
            }
            // The toggle's pieces publish now, as a removal's do on the simulation thread, so
            // the renderer's next poll sees the new generation with its chunks (`ROAD-74`).
            let publication = core.finalize_network_render_products();
            (core.build_snapshot(), publication)
        };
        *self.snapshot.write().unwrap() = snapshot;
        if let Some(publication) = publication {
            self.road_tool.publish(publication);
        }
        true
    }

    /// The commit's verdict on turning arm `edge_id`'s crossing at `node_id` on or off, without
    /// applying it: `is_valid`, and when refused `invalid_reason` (stable key) and `detail`.
    /// The same check as [`Self::set_crosswalk_override`], so the preview is the commit.
    #[func]
    pub fn preview_crosswalk(&mut self, node_id: u32, edge_id: i32, enabled: bool) -> VarDictionary {
        match self
            .lock_core()
            .preview_crosswalk_override_internal(node_id, edge_id, enabled)
        {
            None => vdict! { "is_valid": true },
            Some(rejection) => vdict! {
                "is_valid": false,
                "invalid_reason": rejection.reason,
                "detail": rejection.detail,
            },
        }
    }

    /// Road `edge_id`'s cross-section for the Road Properties window (`ROAD-80`): `fwd_lanes`,
    /// `bkw_lanes`, `width` (carriageway metres), `class`, `median_style`, `median_width`,
    /// `editable` (a live road with vehicle lanes, which [`Self::set_edge_cross_section`] can
    /// change), `sidewalks` (`ROAD-107`: the sides with one in the road's forward direction, 0
    /// none, 1 left, 2 right, 3 both; 0 for a walkway),
    /// `speed_kmh` (the speed limit in whole km/h), `zoning` (lots and buildings along it) and
    /// `street_lights`, and `road_type` (the id of the first usable road type, built-ins first,
    /// whose layout builds exactly this cross-section, sidewalks included, at this speed with
    /// these street lights, else empty; zoning is not matched). Empty for an id that is not a
    /// live edge.
    #[func]
    pub fn get_edge_cross_section(&self, edge_id: i32) -> VarDictionary {
        let (mut result, section) = {
            let core = self.lock_core();
            let graph = &core.region_graph;
            let Some(edge) = usize::try_from(edge_id)
                .ok()
                .and_then(|edge| graph.get_edge(edge))
                .filter(|edge| !edge.deleted)
            else {
                return VarDictionary::new();
            };
            let mut result = vdict! {
                "fwd_lanes": i64::from(edge.fwd_lanes),
                "bkw_lanes": i64::from(edge.bkw_lanes),
                "width": edge.width,
                "class": edge.class as i64,
                "median_style": i64::from(RoadMedian::style_code(edge.median)),
                "median_width": edge.median.map_or(0.0, |median| median.width_m),
                "editable": edge
                    .validate_cross_section(edge.fwd_lanes, edge.bkw_lanes, edge.median)
                    .is_ok(),
            };
            let section =
                crate::simulation::network::road_types::RoadCrossSection::of_edge(edge);
            let speed_kmh = RoadAttributes::speed_kmh_of(edge.speed_limit);
            result.set("sidewalks", i64::from(section.sidewalks.bits()));
            result.set("speed_kmh", i64::from(speed_kmh));
            result.set("zoning", !edge.no_building_spawn);
            result.set("street_lights", !edge.no_street_lights);
            (result, (section, speed_kmh, !edge.no_street_lights))
        };
        // The library is matched without the core lock.
        result.set(
            "road_type",
            self.road_type_of(section.0, section.1, section.2),
        );
        result
    }

    /// The commit's verdict on giving road `edge_id` `fwd` forward and `bkw` backward lanes, the
    /// median `median_style` (0 none, 1 painted, 2 kerbed grass, 3 kerbed paving) of
    /// `median_width` metres, and a road type's `speed_kmh` (20 to 120 in steps of 10),
    /// `sidewalks` (`ROAD-107`: the road's forward frame, codes as in
    /// [`Self::set_road_tool_attributes`]), `zoning` and `street_lights` in place (`ROAD-80`,
    /// `ROAD-101`; pass the road's own values from
    /// [`Self::get_edge_cross_section`] for what stays), without applying it: `is_valid`; when
    /// valid `buildings_removed` (the buildings on the road that zoning off demolishes, for a
    /// confirmation), `demolition_permanent` (true with any `buildings_removed`: undo restores
    /// the road and its empty parcels, never the buildings) and `cost_delta` (what Apply charges: the build-cost difference over the
    /// road's length, never negative); when refused `invalid_reason` (stable key) and `detail`.
    /// The same check as [`Self::set_edge_cross_section`].
    #[func]
    #[allow(clippy::too_many_arguments)]
    pub fn preview_edge_cross_section(
        &mut self,
        edge_id: i32,
        fwd: i32,
        bkw: i32,
        median_style: i32,
        median_width: f32,
        speed_kmh: i32,
        sidewalks: i32,
        zoning: bool,
        street_lights: bool,
    ) -> VarDictionary {
        let attributes = (speed_kmh, sidewalks, zoning, street_lights);
        let verdict = cross_section_spec(median_style, median_width, attributes)
            .and_then(|(median, attributes)| {
                self.lock_core().preview_edge_cross_section_internal(
                    edge_id,
                    lanes_u8(fwd),
                    lanes_u8(bkw),
                    median,
                    attributes,
                )
            });
        match verdict {
            Ok(verdict) => vdict! {
                "is_valid": true,
                "buildings_removed": verdict.buildings_removed as i64,
                "demolition_permanent": verdict.demolition_permanent,
                "cost_delta": verdict.cost_delta,
            },
            Err(rejection) => vdict! {
                "is_valid": false,
                "invalid_reason": rejection.reason,
                "detail": rejection.detail,
            },
        }
    }

    /// Gives road `edge_id` `fwd` forward and `bkw` backward lanes, the median `median_style` of
    /// `median_width` metres and a road type's `speed_kmh`, `sidewalks`, `zoning` and
    /// `street_lights` in place
    /// (`ROAD-80`, `ROAD-101`), with an undo record that restores the whole road (buildings that
    /// zoning off demolished stay demolished, as with the "No buildings" checkbox); its pieces
    /// publish now, as a crossing toggle's do, and the verdict's `cost_delta` is charged.
    /// Returns whether it was applied: a refused edit (see [`Self::preview_edge_cross_section`])
    /// changes nothing.
    #[func]
    #[allow(clippy::too_many_arguments)]
    pub fn set_edge_cross_section(
        &mut self,
        edge_id: i32,
        fwd: i32,
        bkw: i32,
        median_style: i32,
        median_width: f32,
        speed_kmh: i32,
        sidewalks: i32,
        zoning: bool,
        street_lights: bool,
    ) -> bool {
        let attributes = (speed_kmh, sidewalks, zoning, street_lights);
        let Ok((median, attributes)) = cross_section_spec(median_style, median_width, attributes)
        else {
            return false;
        };
        let (snapshot, publication) = {
            let mut core = self.lock_core();
            if !core.set_edge_cross_section_internal(
                edge_id,
                lanes_u8(fwd),
                lanes_u8(bkw),
                median,
                attributes,
            ) {
                return false;
            }
            let publication = core.finalize_network_render_products();
            (core.build_snapshot(), publication)
        };
        *self.snapshot.write().unwrap() = snapshot;
        if let Some(publication) = publication {
            self.road_tool.publish(publication);
        }
        true
    }

    /// The turn pocket at end `end` (0 start, 1 end) of road `edge_id`, or reached through its
    /// lane transition there (`ROAD-95`): `has_pocket`, and with one `left`, `right`,
    /// `length_m` (its body from the junction, D7) and `pocket_edge`. For the Junction window's
    /// Turn pockets rows: `junction` (the node at this end), `road` (the road's polyline from
    /// the junction outwards, the pocket and its base road joined), `width_m` (its widest
    /// edge), `left_offered` / `right_offered` (whether the road can carry a pocket lane on
    /// that side at all: the edit's role checks, without the length, the junction's turns and
    /// the compile) with `left_refusal` / `right_refusal` (the key, empty when offered), and the
    /// length range `min_length_m`, `max_length_m`, `step_m`, `default_length_m`. Empty for an
    /// id that is not a live edge.
    #[func]
    pub fn get_road_pocket(&self, edge_id: i32, end: i32) -> VarDictionary {
        use crate::simulation::network::pockets::PocketEndView;
        let core = self.lock_core();
        usize::try_from(edge_id)
            .ok()
            .and_then(|edge| PocketEndView::of(&core.region_graph, edge, pocket_end(end)))
            .map_or_else(VarDictionary::new, |view| {
                pocket_end_dictionary(&core, &view, edge_id, pocket_end(end))
            })
    }

    /// The Junction window's Turn pockets rows at node `node_id` (`ROAD-95`, round R5c2): one
    /// dictionary per car road arm with a lane into the junction, clockwise from north by the
    /// direction it comes from (ties by edge id), each [`Self::get_road_pocket`]'s for that end
    /// plus `edge`, `end` (0 start, 1 end; a pocket edge's pocket end), `bearing_deg` (from the
    /// junction to the arm's point 20 m out, clockwise from north = -Z), `road_type` (the name of
    /// the road type its road without the pocket matches, as [`Self::get_edge_cross_section`]'s
    /// `road_type`; empty for a custom layout) and that road's `fwd_lanes`, `bkw_lanes`,
    /// `median_style` and `median_width`. Empty for a lane transition node, a node without
    /// such an arm, or an id that is not a node. `O(degree + arm points)`.
    #[func]
    pub fn get_junction_pockets(&self, node_id: u32) -> VarArray {
        let rows: Vec<_> = {
            let core = self.lock_core();
            core.junction_pocket_arms_internal(node_id)
                .into_iter()
                .map(|arm| {
                    let edge_id = arm.edge as i32;
                    let mut row = pocket_end_dictionary(&core, &arm.view, edge_id, arm.end);
                    let (section, ..) = arm.base;
                    row.set("edge", i64::from(edge_id));
                    row.set(
                        "end",
                        i64::from(arm.end == crate::simulation::network::roadbed::EdgeEnd::End),
                    );
                    row.set("bearing_deg", arm.bearing_deg);
                    row.set("fwd_lanes", i64::from(section.fwd_lanes));
                    row.set("bkw_lanes", i64::from(section.bkw_lanes));
                    row.set(
                        "median_style",
                        i64::from(RoadMedian::style_code(section.median)),
                    );
                    row.set(
                        "median_width",
                        section.median.map_or(0.0, |median| median.width_m),
                    );
                    (row, arm.base)
                })
                .collect()
        };
        // The library is matched without the core lock.
        let mut result = VarArray::new();
        for (mut row, (section, speed_kmh, street_lights)) in rows {
            row.set(
                "road_type",
                self.road_type_name_of(section, speed_kmh, street_lights),
            );
            result.push(&row.to_variant());
        }
        result
    }

    /// The commit's verdict on a turn pocket at end `end` (0 start, 1 end) of road `edge_id`
    /// with lanes on the drivers' `left` and `right` and a `length_m` body from the junction
    /// (20-300 m by 10; both off removes the pocket) (`ROAD-95`), without applying it:
    /// `is_valid`; when valid `cost_delta` (what Apply charges, never negative); when refused
    /// `invalid_reason` (stable key) and `detail`. The same check as [`Self::set_road_pocket`]
    /// (D11: no geometry preview).
    #[func]
    pub fn preview_road_pocket(
        &mut self,
        edge_id: i32,
        end: i32,
        left: bool,
        right: bool,
        length_m: f32,
    ) -> VarDictionary {
        let Some(request) = pocket_request(edge_id, end, left, right, length_m) else {
            return vdict! {
                "is_valid": false,
                "invalid_reason": "invalid_road",
                "detail": format!("edge {edge_id} is not a road"),
            };
        };
        match self.lock_core().preview_road_pocket_internal(request) {
            Ok(verdict) => vdict! {
                "is_valid": true,
                "cost_delta": verdict.cost_delta,
            },
            Err(rejection) => vdict! {
                "is_valid": false,
                "invalid_reason": rejection.reason,
                "detail": rejection.detail,
            },
        }
    }

    /// Gives end `end` of road `edge_id` the turn pocket [`Self::preview_road_pocket`] checks,
    /// resizes or changes its pocket, or removes it, with an undo record (`ROAD-95`); its pieces
    /// publish now, as a crossing toggle's do, and the verdict's `cost_delta` is charged.
    /// Returns whether it was applied: a refused edit changes nothing.
    #[func]
    pub fn set_road_pocket(
        &mut self,
        edge_id: i32,
        end: i32,
        left: bool,
        right: bool,
        length_m: f32,
    ) -> bool {
        let Some(request) = pocket_request(edge_id, end, left, right, length_m) else {
            return false;
        };
        let (snapshot, publication) = {
            let mut core = self.lock_core();
            if !core.set_road_pocket_internal(request) {
                return false;
            }
            let publication = core.finalize_network_render_products();
            (core.build_snapshot(), publication)
        };
        *self.snapshot.write().unwrap() = snapshot;
        if let Some(publication) = publication {
            self.road_tool.publish(publication);
        }
        true
    }

    /// Whether road arm `edge_id` at `node_id` can cross, so the select tool offers its
    /// crossing toggle (`ROAD-74`).
    #[func]
    pub fn can_toggle_crosswalk(&self, node_id: u32, edge_id: i32) -> bool {
        self.lock_core().can_toggle_crosswalk_internal(node_id, edge_id)
    }

    /// Whether the lanes carry a crossing on road arm `edge_id` at `node_id`: the zebra the
    /// roadbed paints there.
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
}

// A road end from Godot: 0 the start, anything else the end.
fn pocket_end(end: i32) -> crate::simulation::network::roadbed::EdgeEnd {
    use crate::simulation::network::roadbed::EdgeEnd;
    if end == 0 {
        EdgeEnd::Start
    } else {
        EdgeEnd::End
    }
}

// `get_road_pocket`'s dictionary for end `end` of road `edge_id`, whose pocket view is `view`.
fn pocket_end_dictionary(
    core: &crate::nodes::sim::core::SimCore,
    view: &crate::simulation::network::pockets::PocketEndView,
    edge_id: i32,
    end: crate::simulation::network::roadbed::EdgeEnd,
) -> VarDictionary {
    use crate::simulation::network::pockets::{
        POCKET_BODY_DEFAULT_M, POCKET_BODY_MAX_M, POCKET_BODY_MIN_M, POCKET_BODY_STEP_M,
    };
    let refusal = |role: Result<(), &'static str>| role.err().unwrap_or("");
    let mut result = vdict! {
        "has_pocket": false,
        "junction": i64::from(view.junction),
        "road": PackedVector3Array::from_iter(view.road.iter().copied()),
        "width_m": view.width_m,
        "left_offered": view.left.is_ok(),
        "right_offered": view.right.is_ok(),
        "left_refusal": refusal(view.left),
        "right_refusal": refusal(view.right),
        "min_length_m": POCKET_BODY_MIN_M,
        "max_length_m": POCKET_BODY_MAX_M,
        "step_m": POCKET_BODY_STEP_M,
        "default_length_m": POCKET_BODY_DEFAULT_M,
    };
    if let Some(state) = core.get_road_pocket_internal(edge_id, end) {
        result.set("has_pocket", true);
        result.set("left", state.pocket.left);
        result.set("right", state.pocket.right);
        result.set("length_m", state.body_m);
        result.set("pocket_edge", state.edge as i64);
    }
    result
}

// A pocket edit from Godot's arguments; `None` for a negative edge id.
fn pocket_request(
    edge_id: i32,
    end: i32,
    left: bool,
    right: bool,
    length_m: f32,
) -> Option<crate::simulation::network::pockets::PocketRequest> {
    Some(crate::simulation::network::pockets::PocketRequest {
        edge: usize::try_from(edge_id).ok()?,
        end: pocket_end(end),
        left,
        right,
        body_m: length_m,
    })
}

// A lane count from Godot; out of range saturates, so the edit's check refuses it.
fn lanes_u8(count: i32) -> u8 {
    u8::try_from(count).unwrap_or(u8::MAX)
}

// An in-place edit's median and road-type attributes (speed in km/h, sidewalk sides, zoning,
// street lights) from Godot's arguments, or the refusal (`unknown_median_style`,
// `median_width_out_of_range`, `speed_out_of_range`, `sidewalks_out_of_range`).
fn cross_section_spec(
    median_style: i32,
    median_width: f32,
    (speed_kmh, sidewalks, zoning, street_lights): (i32, i32, bool, bool),
) -> Result<(Option<RoadMedian>, RoadAttributes), RoadRejection> {
    let median = RoadMedian::from_tool(median_style, median_width)
        .map_err(|reason| RoadRejection::new(reason, "the median's style or width"))?;
    let attributes =
        RoadAttributes::from_tool(speed_kmh.into(), sidewalks.into(), zoning, street_lights)
            .map_err(|reason| {
                RoadRejection::new(reason, format!("{speed_kmh} km/h, sidewalks {sidewalks}"))
            })?;
    Ok((median, attributes))
}
