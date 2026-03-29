//! Rendering and visual helper logic for Godot interaction.

use crate::config::{self, ZONING_DEPTH};
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::types::TransitFlags;
use godot::classes::MultiMesh;
use godot::prelude::*;

impl SimulationNode {
    /// Updates the zoning visual MultiMeshes for tool feedback.
    pub fn update_zoning_visuals_internal(
        &self,
        mut grid_mm: Gd<MultiMesh>,
        mut paint_mm: Gd<MultiMesh>,
        hovered_edges: VariantArray,
        is_painting: bool,
        side: i32,
        t1: f32,
        t2: f32,
        depth: i32,
        zone_type: u8,
    ) {
        let graph = &self.region_graph;
        let cell_size = self.config.zone_cell_m;
        let res_step = 1.0;

        // 1. PREVIEW RIBBON
        let mut preview_instances = Vec::new();
        if !hovered_edges.is_empty() && side != 0 {
            let total_depth = depth as f32 * cell_size;
            let edge_count = hovered_edges.len();

            for i in 0..edge_count {
                let edge_idx = hovered_edges.get(i).expect("Valid edge index").to::<i32>();
                if let Some(edge) = graph.edges.get(edge_idx as usize) {
                    let mut current_side_sign = if side > 0 { 1.0 } else { -1.0 };

                    // B7: Track side-flips for previous edges
                    for j in 0..i {
                        let e_a = hovered_edges.get(j).expect("Valid edge").to::<i32>();
                        let e_b = hovered_edges.get(j + 1).expect("Valid edge").to::<i32>();
                        let (ta, tb) = self.get_connection_rust(e_a as usize, e_b as usize);
                        if (ta - tb).abs() < 0.1 {
                            current_side_sign = -current_side_sign;
                        }
                    }

                    // B6: Determine range for this specific edge in the path
                    let (s_t, e_t) = if edge_count == 1 {
                        (t1.min(t2), t1.max(t2))
                    } else if i == 0 {
                        let e_next = hovered_edges.get(1).expect("Valid edge").to::<i32>();
                        let (ta, _) = self.get_connection_rust(edge_idx as usize, e_next as usize);
                        (t1.min(ta), t1.max(ta))
                    } else if i == edge_count - 1 {
                        let e_prev = hovered_edges.get(i - 1).expect("Valid edge").to::<i32>();
                        let (_, tb) = self.get_connection_rust(e_prev as usize, edge_idx as usize);
                        (tb.min(t2), tb.max(t2))
                    } else {
                        let e_prev = hovered_edges.get(i - 1).expect("Valid edge").to::<i32>();
                        let e_next = hovered_edges.get(i + 1).expect("Valid edge").to::<i32>();
                        let (_, tb_in) =
                            self.get_connection_rust(e_prev as usize, edge_idx as usize);
                        let (ta_out, _) =
                            self.get_connection_rust(edge_idx as usize, e_next as usize);
                        (tb_in.min(ta_out), tb_in.max(ta_out))
                    };

                    let start_m = s_t * edge.physical_length;
                    let end_m = e_t * edge.physical_length;

                    let w = self.heightmap.width as f32;
                    let h = self.heightmap.height as f32;

                    let mut m = start_m;
                    while m < end_m {
                        let t_param = (m + res_step * 0.5) / edge.physical_length;
                        if t_param > 1.0 {
                            break;
                        }
                        let (pos_on_edge, tangent) =
                            self.get_edge_pos_and_tangent(edge_idx as usize, t_param);
                        let normal =
                            godot::prelude::Vector2::new(tangent.y, -tangent.x) * current_side_sign;

                        let sw = res_step * 1.05;
                        let sd = 0.8;
                        let color = self.get_zone_color_rust(zone_type);

                        let curb_dist = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH + 0.2;

                        // A. INNER RIBBON (Against road)
                        let center_2d = pos_on_edge + normal * curb_dist;
                        let world_y = self.get_safe_height(center_2d.x, center_2d.y, w, h) + 0.6;
                        self.push_mm_transform(
                            &mut preview_instances,
                            center_2d,
                            world_y,
                            tangent,
                            normal,
                            sw,
                            sd,
                            2.0,
                            color,
                            1.0,
                        );

                        // B. OUTER BOUNDARY (Show only while painting)
                        if is_painting {
                            let depth_m = total_depth - 0.5;
                            let center_2d_outer = pos_on_edge + normal * (curb_dist + depth_m);
                            let world_y_outer =
                                self.get_safe_height(center_2d_outer.x, center_2d_outer.y, w, h)
                                    + 0.6;
                            self.push_mm_transform(
                                &mut preview_instances,
                                center_2d_outer,
                                world_y_outer,
                                tangent,
                                normal,
                                sw,
                                0.4,
                                0.5,
                                color,
                                0.4,
                            );
                        }

                        m += res_step;
                    }
                }
            }
        }

        let grid_count = (preview_instances.len() / 16) as i32;
        grid_mm.set_instance_count(grid_count);
        if grid_count > 0 {
            grid_mm.set_buffer(&PackedFloat32Array::from_iter(preview_instances));
        }

        // 2. PAINTED ZONES
        let mut paint_instances = Vec::new();
        for (&edge_idx, grid) in &self.zoning.edge_grids {
            if let Some(edge) = graph.edges.get(edge_idx) {
                if edge.deleted {
                    continue;
                }

                for side_idx in 0..2 {
                    let side_sign: f32 = if side_idx == 0 { 1.0 } else { -1.0 };
                    let data = if side_sign > 0.0 {
                        &grid.left_side
                    } else {
                        &grid.right_side
                    };
                    if (side_sign > 0.0 && !edge.zoning_left)
                        || (side_sign < 0.0 && !edge.zoning_right)
                    {
                        continue;
                    }

                    let mut x = 0;
                    while x < grid.cells_long {
                        let z_type = data[x * ZONING_DEPTH];
                        if z_type == ZoneType::None {
                            x += 1;
                            continue;
                        }

                        let start_x = x;
                        while x < grid.cells_long && data[x * ZONING_DEPTH] == z_type {
                            x += 1;
                        }
                        let end_x = x;

                        let w = self.heightmap.width as f32;
                        let h = self.heightmap.height as f32;

                        let start_m = start_x as f32 * cell_size;
                        let end_m = if end_x >= grid.cells_long {
                            edge.physical_length
                        } else {
                            end_x as f32 * cell_size
                        };

                        let mut m = start_m;
                        while m < end_m {
                            let t_param = (m + res_step * 0.5) / edge.physical_length;
                            if t_param > 1.0 {
                                break;
                            }
                            let (pos_on_edge, tangent) =
                                self.get_edge_pos_and_tangent(edge_idx, t_param);
                            let normal =
                                godot::prelude::Vector2::new(tangent.y, -tangent.x) * side_sign;

                            let sw = res_step * 1.05;
                            let sd = 0.8;
                            let color = self.get_zone_color_rust(z_type as u8);

                            // INNER RIBBON
                            let curb_dist = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH + 0.2;
                            let center_2d = pos_on_edge + normal * curb_dist;
                            let world_y =
                                self.get_safe_height(center_2d.x, center_2d.y, w, h) + 0.5;
                            self.push_mm_transform(
                                &mut paint_instances,
                                center_2d,
                                world_y,
                                tangent,
                                normal,
                                sw,
                                sd,
                                2.0,
                                color,
                                1.0,
                            );

                            m += res_step;
                        }
                    }
                }
            }
        }

        let paint_count = (paint_instances.len() / 16) as i32;
        paint_mm.set_instance_count(paint_count);
        if paint_count > 0 {
            paint_mm.set_buffer(&PackedFloat32Array::from_iter(paint_instances));
        }
    }

    fn push_mm_transform(
        &self,
        buffer: &mut Vec<f32>,
        pos_2d: Vector2,
        y: f32,
        tangent: Vector2,
        normal: Vector2,
        sw: f32,
        sd: f32,
        sy: f32,
        color: godot::prelude::Color,
        alpha: f32,
    ) {
        // MultiMesh TRANSFORM_3D buffer layout:
        // Row 0: [ x.x, y.x, z.x, origin.x ]
        // Row 1: [ x.y, y.y, z.y, origin.y ]
        // Row 2: [ x.z, y.z, z.z, origin.z ]

        // Basis X = tangent (along road), Row 0.x, Row 1.x=0, Row 2.x=tangent.y
        buffer.push(tangent.x * sw);
        buffer.push(0.0);
        buffer.push(normal.x * sd);
        buffer.push(pos_2d.x);
        buffer.push(0.0);
        buffer.push(sy);
        buffer.push(0.0);
        buffer.push(y);
        buffer.push(tangent.y * sw);
        buffer.push(0.0);
        buffer.push(normal.y * sd);
        buffer.push(pos_2d.y);

        buffer.push(color.r);
        buffer.push(color.g);
        buffer.push(color.b);
        buffer.push(alpha);
    }

    fn get_safe_height(&self, x: f32, z: f32, w: f32, h: f32) -> f32 {
        let gx = (x + (w - 1.0) * 0.5).round().clamp(0.0, w - 1.0) as usize;
        let gz = (z + (h - 1.0) * 0.5).round().clamp(0.0, h - 1.0) as usize;
        self.heightmap.get_height(gx, gz) * 20.0
    }

    /// Returns the Godot Color associated with a ZoneType ID.
    pub fn get_zone_color_rust(&self, z_type: u8) -> godot::prelude::Color {
        match z_type {
            1 => Color::from_rgb(0.0, 1.0, 0.0),
            2 => Color::from_rgb(0.0, 0.5, 1.0),
            3 => Color::from_rgb(1.0, 1.0, 0.0),
            4 => Color::from_rgb(0.1, 0.8, 0.8),
            5 => Color::from_rgb(0.8, 0.0, 0.8),
            _ => Color::from_rgb(1.0, 1.0, 1.0),
        }
    }

    /// Returns the visual alpha for a zoning cell at a given depth.
    pub fn get_depth_alpha(&self, y: i32) -> f32 {
        if y < 4 {
            return 1.0;
        }
        let zoning_depth = ZONING_DEPTH as i32;
        let t = (y - 4) as f32 / (zoning_depth - 1 - 4) as f32;
        1.0 + t * (0.1 - 1.0)
    }

    /// Returns the 12-float transforms for all visible non-car agents (walkers, cyclists, etc.).
    /// Car agents are excluded — use `get_car_transforms_internal` for those.
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
                    if node_idx < self.region_graph.nodes.len() {
                        let npos = self.region_graph.nodes[node_idx].pos;
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
                    if target_node < self.region_graph.nodes.len() {
                        let target_pos = get_h(self.region_graph.nodes[target_node].pos);
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
                        if next_node_idx < self.region_graph.nodes.len() {
                            let next_node_pos = get_h(self.region_graph.nodes[next_node_idx].pos);
                            points.push(current_pos);
                            points.push(next_node_pos);
                            colors.push(color_path);
                            colors.push(color_path);

                            // Remaining segments in the path
                            let mut prev_pos = next_node_pos;
                            for j in (idx + 1)..path.len() {
                                let n_idx = path[j] as usize;
                                if n_idx < self.region_graph.nodes.len() {
                                    let np = get_h(self.region_graph.nodes[n_idx].pos);
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

    /// Returns the 12-float transforms for building MultiMeshes.
    pub fn get_building_transforms_internal(&self, zone_type_int: u8) -> PackedFloat32Array {
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

                let world_y = self.heightmap.get_height(safe_gx, safe_gy) * 20.0;

                let fd = b.facing_dir.normalized();
                let b_zx = -fd.x;
                let b_zz = -fd.y;
                let b_xx = -fd.y;
                let b_xz = fd.x;

                let hash = ((b.center_x * 1000.0) as u32)
                    .wrapping_mul(12345)
                    .wrapping_add((b.center_y * 1000.0) as u32)
                    .wrapping_mul(67890);
                let height_scalar = 0.5 + (hash % 100) as f32 / 40.0;

                let sx = b.width as f32 * 0.95;
                let sy = height_scalar;
                let sz = b.depth as f32 * 0.95;

                buffer.push(b_xx * sx);
                buffer.push(0.0);
                buffer.push(b_zx * sz);
                buffer.push(world_x);

                buffer.push(0.0);
                buffer.push(sy);
                buffer.push(0.0);
                buffer.push(world_y + (5.0 * sy) / 2.0);

                buffer.push(b_xz * sx);
                buffer.push(0.0);
                buffer.push(b_zz * sz);
                buffer.push(world_z);
            }
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns a dictionary containing all road mesh geometry for Godot.
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
