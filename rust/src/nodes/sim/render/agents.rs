// SPDX-License-Identifier: GPL-2.0-only

//! Agent-specific rendering and visual debug logic for Godot interaction.
//!
//! Handles pedestrian and car instance transform generation, and agent path visual debug.

use crate::nodes::sim::core::SimCore;
use crate::nodes::sim::render::lane_pose::{sample_lane_change_pose, sample_lane_pose};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{
    MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS, TRANSIT_IMMIGRATING,
    TRANSIT_IN_BUILDING, TRANSIT_INTERSECTION, TRANSIT_NETWORK, transit_is_visible,
};
use godot::prelude::*;

fn access_phase_target(core: &SimCore, agent_idx: usize, egress: bool) -> Option<Vector3> {
    let building_id = if egress {
        core.agents.current_building[agent_idx]
    } else {
        core.agents.target_building[agent_idx]
    };
    let entrance = core.allocator.entrances.get(building_id)?;
    if egress {
        if core.agents.transit_mode[agent_idx] == MODE_CAR {
            let lane_id = core.agents.planned_attach_lane_id[agent_idx] as usize;
            let lane_d = core.agents.planned_attach_lane_d[agent_idx];
            let lane = core.transit_network.lane_system.lanes.get(lane_id)?;
            let lane_pos = BuildingAllocator::sample_pos_on_lane(lane, lane_d);
            Some(Vector3::new(lane_pos.x, 0.0, lane_pos.y))
        } else {
            Some(Vector3::new(entrance.curb_pos.x, 0.0, entrance.curb_pos.y))
        }
    } else {
        Some(Vector3::new(entrance.door_pos.x, 0.0, entrance.door_pos.y))
    }
}

fn append_debug_polyline<F>(
    points: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
    geometry: &[Vector3],
    color: Color,
    get_h: &F,
) where
    F: Fn(Vector3) -> Vector3,
{
    for segment in geometry.windows(2) {
        points.push(get_h(segment[0]));
        points.push(get_h(segment[1]));
        colors.push(color);
        colors.push(color);
    }
}

fn append_debug_marker(points: &mut Vec<Vector3>, colors: &mut Vec<Color>, center: Vector3) {
    let color = Color::from_rgb(1.0, 0.25, 1.0);
    let arm_x = Vector3::new(0.9, 0.0, 0.0);
    let arm_z = Vector3::new(0.0, 0.0, 0.9);
    let arm_y = Vector3::new(0.0, 1.0, 0.0);

    points.push(center - arm_x);
    points.push(center + arm_x);
    colors.push(color);
    colors.push(color);

    points.push(center - arm_z);
    points.push(center + arm_z);
    colors.push(color);
    colors.push(color);

    points.push(center);
    points.push(center + arm_y);
    colors.push(color);
    colors.push(color);
}

fn desired_next_edge_for_debug(core: &SimCore, agent_idx: usize, lane_id: usize) -> Option<usize> {
    let lane = core.transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX {
        return None;
    }
    let edge = core.region_graph.get_edge(lane.edge_id)?;
    let terminal = if lane.is_fwd {
        edge.end_node
    } else {
        edge.start_node
    };
    let path = &core.agents.current_path[agent_idx];
    let path_idx = core.agents.current_path_index[agent_idx];
    if path_idx >= path.len() {
        return None;
    }

    let next_idx = if path[path_idx] == terminal {
        path_idx + 1
    } else {
        path_idx
    };
    path.get(next_idx).and_then(|&next_node| {
        core.region_graph
            .get_edge_between_nodes(terminal, next_node)
    })
}

fn connector_target_lane_for_debug(core: &SimCore, connector_lane_id: usize) -> Option<usize> {
    let connector = core
        .transit_network
        .lane_system
        .lanes
        .get(connector_lane_id)?;
    connector.next_lanes.first().copied()
}

fn connector_target_edge_for_debug(core: &SimCore, connector_lane_id: usize) -> Option<usize> {
    let target_lane_id = connector_target_lane_for_debug(core, connector_lane_id)?;
    let target_lane = core.transit_network.lane_system.lanes.get(target_lane_id)?;
    if target_lane.edge_id == usize::MAX {
        None
    } else {
        Some(target_lane.edge_id)
    }
}

fn transit_state_label(transit: u8) -> &'static str {
    match transit {
        TRANSIT_IN_BUILDING => "building",
        TRANSIT_ACCESS_EGRESS => "egress",
        TRANSIT_NETWORK => "network",
        TRANSIT_ACCESS_INGRESS => "ingress",
        TRANSIT_IMMIGRATING => "immigrating",
        TRANSIT_INTERSECTION => "junction",
        _ => "unknown",
    }
}

fn debug_id_label(id: usize) -> String {
    if id == usize::MAX {
        "-".to_string()
    } else {
        id.to_string()
    }
}

impl SimCore {
    // ── Agent Renderer ──

    /// Returns the 12-float transforms for all visible non-car agents.
    /// Kept for direct (non-snapshot) callers; `build_snapshot` is the hot path.
    pub fn get_agent_transforms_internal(&self) -> PackedFloat32Array {
        let mut buffer = Vec::with_capacity(self.agents.len() * 12);

        for i in 0..self.agents.len() {
            if !transit_is_visible(self.agents.transit[i]) {
                continue;
            }
            if self.agents.transit_mode[i] == MODE_CAR {
                continue;
            }

            let world_x = self.agents.pos_x[i];
            let world_z = self.agents.pos_y[i];
            let terrain_y = self.heightmap.sample_height_world(world_x, world_z) * 20.0 + 1.0;
            let world_y = terrain_y;

            buffer.push(1.0_f32);
            buffer.push(0.0_f32);
            buffer.push(0.0_f32);
            buffer.push(world_x);
            buffer.push(0.0_f32);
            buffer.push(1.0_f32);
            buffer.push(0.0_f32);
            buffer.push(world_y);
            buffer.push(0.0_f32);
            buffer.push(0.0_f32);
            buffer.push(1.0_f32);
            buffer.push(world_z);
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns a Dictionary where keys are vehicle type IDs (u8) and values are PackedFloat32Array
    /// containing the 12-float transforms for all visible car agents of that type.
    pub fn get_car_transforms_internal(&self) -> VarDictionary {
        let mut type_buffers: std::collections::HashMap<u8, Vec<f32>> =
            std::collections::HashMap::new();

        for i in 0..self.agents.len() {
            if !transit_is_visible(self.agents.transit[i]) {
                continue;
            }
            if self.agents.transit_mode[i] != MODE_CAR {
                continue;
            }

            let v_type = self.agents.vehicle_type[i];
            let variant_id = (self.agents.render_id[i] % 5) as u8; // 5 stable color variants per model
            let model_key = (v_type * 10) + variant_id;

            let buffer = type_buffers.entry(model_key).or_insert_with(Vec::new);

            let mut world_x = self.agents.pos_x[i];
            let mut world_z = self.agents.pos_y[i];
            let mut lane_pose = None;
            let current_lane = self.agents.current_lane_id[i];
            if current_lane != usize::MAX
                && current_lane < self.transit_network.lane_system.lanes.len()
            {
                let lane = &self.transit_network.lane_system.lanes[current_lane];
                let source_lane_id = self.agents.lane_change_from_lane_id[i];
                lane_pose = if self.agents.transit[i] == TRANSIT_NETWORK
                    && source_lane_id != u32::MAX
                    && (source_lane_id as usize) < self.transit_network.lane_system.lanes.len()
                    && self.agents.lane_distance[i]
                        < self.agents.lane_change_start_d[i] + self.agents.lane_change_length_m[i]
                {
                    let source_lane =
                        &self.transit_network.lane_system.lanes[source_lane_id as usize];
                    if source_lane.edge_id != usize::MAX
                        && source_lane.edge_id == lane.edge_id
                        && source_lane.is_fwd == lane.is_fwd
                        && source_lane.lane_type == lane.lane_type
                    {
                        sample_lane_change_pose(
                            source_lane,
                            lane,
                            self.agents.lane_distance[i],
                            self.agents.lane_change_start_d[i],
                            self.agents.lane_change_length_m[i],
                        )
                    } else {
                        sample_lane_pose(lane, self.agents.lane_distance[i])
                    }
                } else {
                    sample_lane_pose(lane, self.agents.lane_distance[i])
                };
                if let Some((pos, _)) = lane_pose {
                    world_x = pos.x;
                    world_z = pos.z;
                }
            }

            let mut basis_x = Vector3::RIGHT;
            let mut basis_y = Vector3::UP;
            let mut basis_z = Vector3::BACK;
            let terrain_y = self.heightmap.sample_height_world(world_x, world_z) * 20.0 + 0.02;
            let mut world_y = terrain_y;

            if let Some((pos, tangent)) = lane_pose {
                world_y = pos.y + 0.02;
                basis_z = -tangent;
                let right = Vector3::UP.cross(basis_z);
                if right.length_squared() > 1e-6 {
                    basis_x = right.normalized();
                    basis_y = basis_z.cross(basis_x).normalized();
                }
            } else {
                use crate::simulation::economy::agents::{
                    TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS,
                };
                let transit = self.agents.transit[i];
                if transit == TRANSIT_ACCESS_EGRESS {
                    if let Some(target) = access_phase_target(self, i, true) {
                        let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                        if dir.length_squared() > 1e-6 {
                            basis_z = -dir.normalized();
                            basis_x = Vector3::UP.cross(basis_z).normalized();
                            basis_y = basis_z.cross(basis_x).normalized();
                        }
                    }
                } else if transit == TRANSIT_ACCESS_INGRESS {
                    if let Some(target) = access_phase_target(self, i, false) {
                        let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                        if dir.length_squared() > 1e-6 {
                            basis_z = -dir.normalized();
                            basis_x = Vector3::UP.cross(basis_z).normalized();
                            basis_y = basis_z.cross(basis_x).normalized();
                        }
                    }
                }
            }

            buffer.push(basis_x.x);
            buffer.push(basis_y.x);
            buffer.push(basis_z.x);
            buffer.push(world_x);
            buffer.push(basis_x.y);
            buffer.push(basis_y.y);
            buffer.push(basis_z.y);
            buffer.push(world_y);
            buffer.push(basis_x.z);
            buffer.push(basis_y.z);
            buffer.push(basis_z.z);
            buffer.push(world_z);
        }

        let mut dict = VarDictionary::new();
        for (v_type, buffer) in type_buffers {
            dict.set(v_type as i32, PackedFloat32Array::from_iter(buffer));
        }
        dict
    }

    /// Returns debug path geometry for active agents.
    pub fn get_agent_paths_debug_internal(&self) -> VarDictionary {
        let mut points = Vec::new();
        let mut colors = Vec::new();
        let mut label_positions = Vec::new();
        let mut labels = Vec::new();
        let mut displayed_count = 0;
        let traffic_visual = crate::debug::is_traffic_enabled();
        let max_display = if traffic_visual { 250 } else { 2500 };
        const MAX_TRAFFIC_LABELS: usize = 96;

        let get_h = |pos: Vector3| -> Vector3 {
            let y = self.heightmap.sample_height_world(pos.x, pos.z) * 20.0 + 1.2;
            Vector3::new(pos.x, y, pos.z)
        };

        let color_path = Color::from_rgb(0.2, 0.8, 1.0); // Cyan
        let color_direct = Color::from_rgb(1.0, 0.9, 0.2); // Yellow/Golden
        let color_current_lane = Color::from_rgb(0.1, 0.45, 1.0); // Blue
        let color_connector = Color::from_rgb(1.0, 0.75, 0.1); // Yellow
        let color_next_lane = Color::from_rgb(0.15, 1.0, 0.35); // Green
        let _color_stuck = Color::from_rgb(1.0, 0.2, 0.2); // Red (Not used yet, but placeholder)

        for i in 0..self.agents.len() {
            if self.agents.transit[i] != 0 {
                let current_lane_id = self.agents.current_lane_id[i];
                let current_lane = self.transit_network.lane_system.lanes.get(current_lane_id);
                let lane_pose = current_lane
                    .and_then(|lane| sample_lane_pose(lane, self.agents.lane_distance[i]));
                let raw_pos = lane_pose.map(|(pos, _)| pos).unwrap_or_else(|| {
                    Vector3::new(self.agents.pos_x[i], 0.0, self.agents.pos_y[i])
                });
                let current_pos = get_h(raw_pos);

                // A. DEPARTING / ARRIVING direct lines (Yellow)
                if self.agents.transit[i] == TRANSIT_ACCESS_EGRESS {
                    if let Some(target) = access_phase_target(self, i, true) {
                        let target_pos = get_h(target);
                        points.push(current_pos);
                        points.push(target_pos);
                        colors.push(color_direct);
                        colors.push(color_direct);
                    }
                } else if self.agents.transit[i] == TRANSIT_ACCESS_INGRESS {
                    if let Some(target) = access_phase_target(self, i, false) {
                        let target_pos = get_h(target);
                        points.push(current_pos);
                        points.push(target_pos);
                        colors.push(color_direct);
                        colors.push(color_direct);
                    }
                }

                if traffic_visual && self.agents.transit_mode[i] == MODE_CAR {
                    append_debug_marker(&mut points, &mut colors, current_pos);

                    if let Some(lane) = current_lane {
                        let lane_color = if lane.edge_id == usize::MAX {
                            color_connector
                        } else {
                            color_current_lane
                        };
                        append_debug_polyline(
                            &mut points,
                            &mut colors,
                            &lane.geometry,
                            lane_color,
                            &get_h,
                        );

                        if lane.edge_id == usize::MAX {
                            if let Some(target_lane_id) =
                                connector_target_lane_for_debug(self, current_lane_id)
                            {
                                if let Some(target_lane) =
                                    self.transit_network.lane_system.lanes.get(target_lane_id)
                                {
                                    append_debug_polyline(
                                        &mut points,
                                        &mut colors,
                                        &target_lane.geometry,
                                        color_next_lane,
                                        &get_h,
                                    );
                                }
                            }
                        } else {
                            let desired_next_edge =
                                desired_next_edge_for_debug(self, i, current_lane_id);
                            let mut drew_connector = false;
                            for &connector_id in &lane.next_lanes {
                                let connector_edge =
                                    connector_target_edge_for_debug(self, connector_id);
                                if desired_next_edge.is_some()
                                    && connector_edge != desired_next_edge
                                {
                                    continue;
                                }
                                if let Some(connector) =
                                    self.transit_network.lane_system.lanes.get(connector_id)
                                {
                                    if connector.edge_id == usize::MAX {
                                        append_debug_polyline(
                                            &mut points,
                                            &mut colors,
                                            &connector.geometry,
                                            color_connector,
                                            &get_h,
                                        );
                                        drew_connector = true;
                                        if let Some(target_lane_id) =
                                            connector_target_lane_for_debug(self, connector_id)
                                        {
                                            if let Some(target_lane) = self
                                                .transit_network
                                                .lane_system
                                                .lanes
                                                .get(target_lane_id)
                                            {
                                                append_debug_polyline(
                                                    &mut points,
                                                    &mut colors,
                                                    &target_lane.geometry,
                                                    color_next_lane,
                                                    &get_h,
                                                );
                                            }
                                        }
                                    }
                                }
                            }

                            if !drew_connector && desired_next_edge.is_some() {
                                for &connector_id in &lane.next_lanes {
                                    if let Some(connector) =
                                        self.transit_network.lane_system.lanes.get(connector_id)
                                    {
                                        if connector.edge_id == usize::MAX {
                                            append_debug_polyline(
                                                &mut points,
                                                &mut colors,
                                                &connector.geometry,
                                                color_connector,
                                                &get_h,
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        if labels.len() < MAX_TRAFFIC_LABELS {
                            let desired_next_edge =
                                desired_next_edge_for_debug(self, i, current_lane_id)
                                    .or_else(|| {
                                        connector_target_edge_for_debug(self, current_lane_id)
                                    })
                                    .unwrap_or(usize::MAX);
                            let text = format!(
                                "#{} r{} {}\nlane={} d={:.1}/{:.1} v={:.1}m/s\nedge={} next={} path={}/{}",
                                i,
                                self.agents.render_id[i],
                                transit_state_label(self.agents.transit[i]),
                                debug_id_label(current_lane_id),
                                self.agents.lane_distance[i],
                                lane.length,
                                self.agents.speed[i],
                                debug_id_label(lane.edge_id),
                                debug_id_label(desired_next_edge),
                                self.agents.current_path_index[i],
                                self.agents.current_path[i].len(),
                            );
                            label_positions.push(current_pos + Vector3::new(0.0, 2.2, 0.0));
                            labels.push(GString::from(text.as_str()));
                        }
                    } else if labels.len() < MAX_TRAFFIC_LABELS {
                        let text = format!(
                            "#{} r{} {}\nlane={} d={:.1} v={:.1}m/s\nedge={} path={}/{}",
                            i,
                            self.agents.render_id[i],
                            transit_state_label(self.agents.transit[i]),
                            debug_id_label(current_lane_id),
                            self.agents.lane_distance[i],
                            self.agents.speed[i],
                            debug_id_label(self.agents.current_edge[i]),
                            self.agents.current_path_index[i],
                            self.agents.current_path[i].len(),
                        );
                        label_positions.push(current_pos + Vector3::new(0.0, 2.2, 0.0));
                        labels.push(GString::from(text.as_str()));
                    }
                }

                // B. Remainder of the CCH path (Cyan)
                if !self.agents.current_path[i].is_empty() {
                    let path = &self.agents.current_path[i];
                    let idx = self.agents.current_path_index[i];

                    if idx < path.len() {
                        // Segment from current position to the next node in the path
                        let next_node_idx = path[idx] as usize;
                        if next_node_idx < self.region_graph.node_count() {
                            let next_node_pos =
                                get_h(self.region_graph.node(next_node_idx as u32).pos);
                            points.push(current_pos);
                            points.push(next_node_pos);
                            colors.push(color_path);
                            colors.push(color_path);

                            // Remaining segments in the path
                            let mut prev_pos = next_node_pos;
                            for j in (idx + 1)..path.len() {
                                let n_idx = path[j] as usize;
                                if n_idx < self.region_graph.node_count() {
                                    let np = get_h(self.region_graph.node(n_idx as u32).pos);
                                    points.push(prev_pos);
                                    points.push(np);
                                    colors.push(color_path);
                                    colors.push(color_path);
                                    prev_pos = np;
                                }
                            }
                        }
                    }
                }

                displayed_count += 1;
                if displayed_count >= max_display {
                    break;
                }
            }
        }

        let mut dict = VarDictionary::new();
        dict.set("points", PackedVector3Array::from_iter(points));
        dict.set("colors", PackedColorArray::from_iter(colors));
        dict.set(
            "label_positions",
            PackedVector3Array::from_iter(label_positions),
        );
        dict.set("labels", PackedStringArray::from_iter(labels));
        dict
    }
}
