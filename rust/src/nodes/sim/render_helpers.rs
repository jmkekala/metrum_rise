//! Rendering and visual helper logic for Godot interaction.
//!
//! ### Method Mapping
//!
//! | Category | Method | Godot Caller |
//! |----------|--------|--------------|
//! | **Agents**    | `get_agent_transforms_internal`   | `agent_renderer.gd` |
//! |              | `get_car_transforms_internal`     | `agent_renderer.gd` |
//! |              | `get_agent_paths_debug_internal`  | `agents.gd` (Draw3D debug) |
//! | **Buildings** | `get_building_transforms_internal` | `building_renderer.gd` |
//! |              | `get_building_plot_transforms_internal` | `building_renderer.gd` |
//! | **Network**   | `get_road_mesh_data_internal`     | `network_renderer.gd` |
//! |              | `get_connection_rust`             | Internal |

use crate::nodes::sim::core::SimCore;
use crate::simulation::grid::zoning::ZoneType;
use godot::prelude::*;

impl SimCore {
    // ── Agent Renderer ──

    /// Returns the 12-float transforms for all visible non-car agents.
    /// Kept for direct (non-snapshot) callers; `build_snapshot` is the hot path.
    pub fn get_agent_transforms_internal(&self) -> PackedFloat32Array {
        use crate::simulation::economy::agents::MODE_CAR;
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
        use crate::simulation::economy::agents::MODE_CAR;
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
            if current_lane != usize::MAX && current_lane < self.transit_network.lane_system.lanes.len() {
                let l = &self.transit_network.lane_system.lanes[current_lane];
                let dist = self.agents.lane_distance[i];
                if l.geometry.len() >= 2 {
                    let mut curr = 0.0;
                    for j in 0..l.geometry.len() - 1 {
                        let p0 = l.geometry[j];
                        let p1 = l.geometry[j+1];
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
                use crate::simulation::economy::agents::{TRANSIT_ARRIVING, TRANSIT_DEPARTING};
                let transit = self.agents.transit[i];
                if transit == TRANSIT_DEPARTING {
                    let node_idx = self.agents.current_node[i] as usize;
                    if node_idx < self.region_graph.node_count() {
                        let npos = self.region_graph.node(node_idx as u32).pos;
                        let dir = Vector3::new(npos.x - world_x, 0.0, npos.z - world_z);
                        if dir.length_squared() > 1e-6 {
                            basis_z = -dir.normalized();
                            basis_x = Vector3::UP.cross(basis_z).normalized();
                            basis_y = basis_z.cross(basis_x).normalized();
                        }
                    }
                } else if transit == TRANSIT_ARRIVING {
                    let b_id = self.agents.target_building[i];
                    if b_id != usize::MAX && b_id < self.allocator.buildings.len() {
                        let b = &self.allocator.buildings[b_id];
                        let dir = Vector3::new(b.center_x - world_x, 0.0, b.center_y - world_z);
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

        use crate::simulation::economy::agents::{TRANSIT_ARRIVING, TRANSIT_DEPARTING};

        for i in 0..self.agents.len() {
            if self.agents.transit[i] != 0 {
                let current_pos = get_h(Vector3::new(
                    self.agents.pos_x[i],
                    0.0,
                    self.agents.pos_y[i],
                ));

                // A. DEPARTING / ARRIVING direct lines (Yellow)
                if self.agents.transit[i] == TRANSIT_DEPARTING {
                    // Heading to node current_node + possibly a lane offset point
                    let target_node = self.agents.current_node[i] as usize;
                    if target_node < self.region_graph.node_count() {
                        let target_pos = get_h(self.region_graph.node(target_node as u32).pos);
                        points.push(current_pos);
                        points.push(target_pos);
                        colors.push(color_direct);
                        colors.push(color_direct);
                    }
                } else if self.agents.transit[i] == TRANSIT_ARRIVING {
                    let b_id = self.agents.target_building[i];
                    if b_id != usize::MAX && b_id < self.allocator.buildings.len() {
                        let b = &self.allocator.buildings[b_id];
                        let target_pos = get_h(Vector3::new(b.center_x, 0.0, b.center_y));
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
                            let next_node_pos = get_h(self.region_graph.node(next_node_idx as u32).pos);
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

    // ── Building Renderer ──

    /// Returns the 12-float transforms for all placed buildings with the given asset ID.
    pub fn get_building_transforms_for_asset_internal(&self, asset_id: &str) -> PackedFloat32Array {
        let mut buffer = Vec::new();
        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        for b in &self.allocator.buildings {
            if asset_id == "broken:error" {
                if !b.broken { continue; }
            } else {
                if b.broken || b.asset_id != asset_id {
                    continue;
                }
            }

            let world_x = b.center_x;
            let world_z = b.center_y;

            let grid_x = b.center_x + hw;
            let grid_y = b.center_y + hh;
            let safe_gx = grid_x.round().clamp(0.0, w - 1.0) as usize;
            let safe_gy = grid_y.round().clamp(0.0, h - 1.0) as usize;

            let world_y = self.heightmap.get_height(safe_gx, safe_gy) * 20.0;

            let fd = b.facing_dir.normalized();
            let b_zx = fd.x;
            let b_zz = fd.y;
            let b_xx = fd.y;
            let b_xz = -fd.x;

            // Scale: prefer per-asset preview_scale, fall back to BUILDING_VISUAL_SCALE.
            let entry = self.allocator.registry.get(asset_id);
            let s = entry
                .and_then(|e| e.manifest.building.as_ref())
                .and_then(|b| b.preview_scale)
                .unwrap_or(crate::config::BUILDING_VISUAL_SCALE);
            let (sx, sy, sz) = (s, s, s);

            // Pivot offset: centres the mesh over the lot cell and grounds it at Y=0.
            // Stored in model units; applied here in world space after scale+rotation.
            let (po_x, po_y, po_z) = entry
                .and_then(|e| e.manifest.pivot_offset)
                .map(|[x, y, z]| (x, y, z))
                .unwrap_or((0.0, 0.0, 0.0));
            let tx = world_x + (b_xx * po_x + b_zx * po_z) * sx;
            let ty = world_y + po_y * sy;
            let tz = world_z + (b_xz * po_x + b_zz * po_z) * sz;

            buffer.push(b_xx * sx);
            buffer.push(0.0);
            buffer.push(b_zx * sz);
            buffer.push(tx);

            buffer.push(0.0);
            buffer.push(sy);
            buffer.push(0.0);
            buffer.push(ty);

            buffer.push(b_xz * sx);
            buffer.push(0.0);
            buffer.push(b_zz * sz);
            buffer.push(tz);
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns the 12-float transforms for building plot/foundation MultiMeshes (visualizing item 53).
    pub fn get_building_plot_transforms_internal(&self, zone_type_int: u8) -> PackedFloat32Array {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            4 => ZoneType::Office,
            5 => ZoneType::Mixed,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return PackedFloat32Array::new();
        }

        let mut buffer = Vec::new();
        let w = self.heightmap.width as f32;
        let h = self.heightmap.height as f32;
        let hw = (w - 1.0) * 0.5;
        let hh = (h - 1.0) * 0.5;

        for b in &self.allocator.buildings {
            if b.zone_type == target_zone {
                let world_x = b.center_x;
                let world_z = b.center_y;

                let grid_x = b.center_x + hw;
                let grid_y = b.center_y + hh;
                let safe_gx = grid_x.round().clamp(0.0, w - 1.0) as usize;
                let safe_gy = grid_y.round().clamp(0.0, h - 1.0) as usize;

                let world_y = self.heightmap.get_height(safe_gx, safe_gy) * 20.0 + 0.02; // Slightly above terrain

                let fd = b.facing_dir.normalized();
                let b_zx = fd.x;
                let b_zz = fd.y;
                let b_xx = fd.y;
                let b_xz = -fd.x;

                // Plot size is 10m * cell count (default 3x3 = 30x30)
                let cell_size = self.config.zone_cell_m;
                let sx = b.width_cells as f32 * cell_size;
                let sz = b.depth_cells as f32 * cell_size;
                let sy = 0.5; // Thin foundation box

                buffer.push(b_xx * sx);
                buffer.push(0.0);
                buffer.push(b_zx * sz);
                buffer.push(world_x);

                buffer.push(0.0);
                buffer.push(sy);
                buffer.push(0.0);
                buffer.push(world_y);

                buffer.push(b_xz * sx);
                buffer.push(0.0);
                buffer.push(b_zz * sz);
                buffer.push(world_z);
            }
        }

        PackedFloat32Array::from_iter(buffer)
    }

    // ── Network Renderer ──

    /// Returns dictionary of road/intersection mesh data.
    pub fn get_road_mesh_data_internal(&self) -> VarDictionary {
        let mesh_data = self
            .transit_network
            .generate_mesh_data(&self.region_graph, &self.heightmap);
        let mut dict = VarDictionary::new();
        dict.set(
            "sidewalk_vertices",
            PackedVector3Array::from_iter(mesh_data.sidewalk_vertices),
        );
        dict.set(
            "sidewalk_normals",
            PackedVector3Array::from_iter(mesh_data.sidewalk_normals),
        );
        dict.set(
            "sidewalk_uvs",
            PackedVector2Array::from_iter(mesh_data.sidewalk_uvs),
        );
        dict.set(
            "sidewalk_colors",
            PackedColorArray::from_iter(mesh_data.sidewalk_colors),
        );
        dict.set(
            "road_vertices",
            PackedVector3Array::from_iter(mesh_data.road_vertices),
        );
        dict.set(
            "road_normals",
            PackedVector3Array::from_iter(mesh_data.road_normals),
        );
        dict.set(
            "road_uvs",
            PackedVector2Array::from_iter(mesh_data.road_uvs),
        );
        dict.set(
            "road_colors",
            PackedColorArray::from_iter(mesh_data.road_colors),
        );

        dict.set(
            "marking_vertices",
            PackedVector3Array::from_iter(mesh_data.marking_vertices),
        );
        dict.set(
            "marking_normals",
            PackedVector3Array::from_iter(mesh_data.marking_normals),
        );
        dict.set(
            "marking_uvs",
            PackedVector2Array::from_iter(mesh_data.marking_uvs),
        );
        dict.set(
            "marking_colors",
            PackedColorArray::from_iter(mesh_data.marking_colors),
        );

        dict.set(
            "concrete_vertices",
            PackedVector3Array::from_iter(mesh_data.concrete_vertices),
        );
        dict.set(
            "concrete_normals",
            PackedVector3Array::from_iter(mesh_data.concrete_normals),
        );
        dict.set(
            "concrete_uvs",
            PackedVector2Array::from_iter(mesh_data.concrete_uvs),
        );
        dict.set(
            "concrete_colors",
            PackedColorArray::from_iter(mesh_data.concrete_colors),
        );
        dict
    }

    /// Calculates the normalized T-coordinates of the connection between two edges.
    pub fn get_connection_rust(&self, edge_a: usize, edge_b: usize) -> (f32, f32) {
        let (p_a0, _) = self.get_edge_pos_and_tangent(edge_a, 0.0);
        let (p_a1, _) = self.get_edge_pos_and_tangent(edge_a, 1.0);
        let (p_b0, _) = self.get_edge_pos_and_tangent(edge_b, 0.0);
        let (p_b1, _) = self.get_edge_pos_and_tangent(edge_b, 1.0);

        let thr = 400.0;
        if p_a1.distance_squared_to(p_b0) < thr {
            (1.0, 0.0)
        } else if p_a1.distance_squared_to(p_b1) < thr {
            (1.0, 1.0)
        } else if p_a0.distance_squared_to(p_b0) < thr {
            (0.0, 0.0)
        } else if p_a0.distance_squared_to(p_b1) < thr {
            (0.0, 1.0)
        } else {
            (1.0, 0.0)
        }
    }
}

/// Returns the scale factor for a building.
/// Standard assets use 1:10 scale (1 unit = 10m), so we scale by [`crate::config::BUILDING_VISUAL_SCALE`].
pub fn get_building_visual_scale() -> (f32, f32, f32) {
    let s = crate::config::BUILDING_VISUAL_SCALE;
    (s, s, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_building_visual_scale_is_adequate() {
        // This test ensures that buildings are not "miniature" by verifying the scale factor
        // returned from our logic is at least 10.0 (the standard for current assets).
        let (sx, sy, sz) = get_building_visual_scale();
        
        assert!(sy >= 10.0, "Building vertical scale must be at least 10.0 to match asset scale");
        assert!(sx >= 10.0, "Building horizontal scale must be at least 10.0 to match asset scale");
        assert!(sz >= 10.0, "Building depth scale must be at least 10.0 to match asset scale");
    }
}
