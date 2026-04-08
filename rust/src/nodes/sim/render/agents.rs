//! Agent-specific rendering and visual debug logic for Godot interaction.
//!
//! Handles pedestrian and car instance transform generation, and agent path visual debug.

use crate::nodes::sim::core::SimCore;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::MODE_CAR;
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

impl SimCore {
    // ── Agent Renderer ──

    /// Returns the 12-float transforms for all visible non-car agents.
    /// Kept for direct (non-snapshot) callers; `build_snapshot` is the hot path.
    pub fn get_agent_transforms_internal(&self) -> PackedFloat32Array {
        let mut buffer = Vec::with_capacity(self.agents.len() * 12);

        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        for i in 0..self.agents.len() {
            if !self.agents.is_visible[i] {
                continue;
            }
            if self.agents.transit_mode[i] == MODE_CAR {
                continue;
            }

            let world_x = self.agents.pos_x[i];
            let world_z = self.agents.pos_y[i];

            let map_x = (world_x + hw).clamp(0.0, w - 1.0) as usize;
            let map_z = (world_z + hh).clamp(0.0, h - 1.0) as usize;
            let terrain_y = self.heightmap.get_height(map_x, map_z) * 20.0 + 1.0;
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

        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        for i in 0..self.agents.len() {
            if !self.agents.is_visible[i] {
                continue;
            }
            if self.agents.transit_mode[i] != MODE_CAR {
                continue;
            }

            let v_type = self.agents.vehicle_type[i];
            let variant_id = (i % 5) as u8; // 5 color variants per model
            let model_key = (v_type * 10) + variant_id;

            let buffer = type_buffers.entry(model_key).or_insert_with(Vec::new);

            let world_x = self.agents.pos_x[i];
            let world_z = self.agents.pos_y[i];

            let mut basis_x = Vector3::RIGHT;
            let mut basis_y = Vector3::UP;
            let mut basis_z = Vector3::BACK;

            let map_x = (world_x + hw).clamp(0.0, w - 1.0) as usize;
            let map_z = (world_z + hh).clamp(0.0, h - 1.0) as usize;
            let terrain_y = self.heightmap.get_height(map_x, map_z) * 20.0 + 0.02;
            let mut world_y = terrain_y;

            let current_lane = self.agents.current_lane_id[i];
            if current_lane != usize::MAX
                && current_lane < self.transit_network.lane_system.lanes.len()
            {
                let l = &self.transit_network.lane_system.lanes[current_lane];
                let dist = self.agents.lane_distance[i];
                if l.geometry.len() >= 2 {
                    let mut curr = 0.0;
                    for j in 0..l.geometry.len() - 1 {
                        let p0 = l.geometry[j];
                        let p1 = l.geometry[j + 1];
                        let d = p0.distance_to(p1);
                        if curr + d >= dist || j == l.geometry.len() - 2 {
                            let t = if d > 1e-5 { (dist - curr) / d } else { 0.0 };
                            world_y = p0.y + (p1.y - p0.y) * t.clamp(0.0, 1.0) + 0.02;

                            let fwd = (p1 - p0).normalized();
                            if fwd.length_squared() > 1e-6 {
                                basis_z = -fwd;
                                basis_x = Vector3::UP.cross(basis_z).normalized();
                                basis_y = basis_z.cross(basis_x).normalized();
                            }
                            break;
                        }
                        curr += d;
                    }
                } else if !l.geometry.is_empty() {
                    world_y = l.geometry[0].y + 0.02;
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
        let mut displayed_count = 0;
        let max_display = 2500; // Limit to avoid 1M-agent frame drop

        let get_h = |pos: Vector3| -> Vector3 {
            let w = self.heightmap.width as f32;
            let h = self.heightmap.height as f32;
            let hw = (w - 1.0) * 0.5;
            let hh = (h - 1.0) * 0.5;
            let map_x = (pos.x + hw).clamp(0.0, w - 1.0) as usize;
            let map_z = (pos.z + hh).clamp(0.0, h - 1.0) as usize;
            let y = self.heightmap.get_height(map_x, map_z) * 20.0 + 1.2;
            Vector3::new(pos.x, y, pos.z)
        };

        let color_path = Color::from_rgb(0.2, 0.8, 1.0); // Cyan
        let color_direct = Color::from_rgb(1.0, 0.9, 0.2); // Yellow/Golden
        let _color_stuck = Color::from_rgb(1.0, 0.2, 0.2); // Red (Not used yet, but placeholder)

        use crate::simulation::economy::agents::{TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS};

        for i in 0..self.agents.len() {
            if self.agents.transit[i] != 0 {
                let current_pos = get_h(Vector3::new(
                    self.agents.pos_x[i],
                    0.0,
                    self.agents.pos_y[i],
                ));

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
        dict
    }
}
