//! Spatial queries and projection helpers for the simulation node.

use crate::config::{HEIGHT_SCALE, ZONING_DEPTH};
use crate::nodes::simulation_node::SimulationNode;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::interaction;
use godot::prelude::*;

fn is_canonical_node(graph: &RegionGraph, node_id: u32) -> bool {
    graph.get_valid_node(node_id) == node_id
}

fn get_closest_canonical_node(graph: &RegionGraph, world_pos: Vector3, max_dist: f32) -> i32 {
    let mut best_id = -1;
    let mut min_d = max_dist;
    for (i, node) in graph.nodes.iter().enumerate() {
        let node_id = i as u32;
        if !is_canonical_node(graph, node_id) {
            continue;
        }
        let d = node.pos.distance_to(world_pos);
        if d < min_d {
            min_d = d;
            best_id = node_id as i32;
        }
    }
    best_id
}

impl SimulationNode {
    /// Projects a world position onto a road edge.
    pub fn get_projection_data(
        &self,
        edge: &crate::simulation::network::graph::Edge,
        p: godot::prelude::Vector2,
    ) -> interaction::ProjectionData {
        let mut best_dist_sq = 1e10;
        let mut best_t = 0.0;
        let mut best_side = 1;
        let mut best_dist_from_road = 0.0;

        let geom = &edge.physical_geometry;
        let total_l = edge.physical_length;
        let mut curr_l = 0.0;

        for i in 0..geom.len() - 1 {
            let a = godot::prelude::Vector2::new(geom[i].x, geom[i].z);
            let b = godot::prelude::Vector2::new(geom[i + 1].x, geom[i + 1].z);
            let seg = b - a;
            let l2 = seg.length_squared();
            if l2 < 0.001 {
                continue;
            }

            let mut t_val = ((p.x - a.x) * seg.x + (p.y - a.y) * seg.y) / l2;
            t_val = t_val.clamp(0.0, 1.0);
            let proj = a + seg * t_val;

            let dist_sq = p.distance_squared_to(proj);
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best_t = (curr_l + t_val * f32::sqrt(l2)) / total_l;
                let tangent = seg.normalized();
                let normal = godot::prelude::Vector2::new(tangent.y, -tangent.x);
                let to_pt = p - proj;
                best_side = if to_pt.dot(normal) > 0.0 { 1 } else { -1 };
                best_dist_from_road = f32::sqrt(dist_sq);
            }
            curr_l += f32::sqrt(l2);
        }

        interaction::ProjectionData {
            t: best_t,
            side: best_side,
            dist_from_road: best_dist_from_road,
        }
    }

    /// Returns the position and tangent of a point on a road edge at parameter t.
    pub fn get_edge_pos_and_tangent(
        &self,
        edge_idx: usize,
        t: f32,
    ) -> (godot::prelude::Vector2, godot::prelude::Vector2) {
        let edge = &self.region_graph.edges[edge_idx];
        let geom = &edge.physical_geometry;
        if geom.len() < 2 {
            return (
                godot::prelude::Vector2::new(0.0, 0.0),
                godot::prelude::Vector2::new(1.0, 0.0),
            );
        }
        let total_l = edge.physical_length;
        let target_l = t * total_l;

        let mut curr_l = 0.0;
        for i in 0..geom.len() - 1 {
            let p1 = godot::prelude::Vector2::new(geom[i].x, geom[i].z);
            let p2 = godot::prelude::Vector2::new(geom[i + 1].x, geom[i + 1].z);
            let dist = p2 - p1;
            let d = dist.length();
            if curr_l + d >= target_l || i == geom.len() - 2 {
                let local_t = if d > 1e-6 {
                    ((target_l - curr_l) / d).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let tangent = if d > 1e-6 {
                    dist.normalized()
                } else {
                    godot::prelude::Vector2::new(1.0, 0.0)
                };
                return (p1 + dist * local_t, tangent);
            }
            curr_l += d;
        }
        (
            godot::prelude::Vector2::new(0.0, 0.0),
            godot::prelude::Vector2::new(1.0, 0.0),
        )
    }

    /// Returns obstacle polygons for zoning tool overlap checks.
    pub fn get_obstacle_polygons_internal(
        &self,
        _ignore_poly_id: i32,
        ignore_edge_idx: i32,
    ) -> PackedFloat32Array {
        let mut data = Vec::new();
        let mut count = 0.0;
        data.push(0.0); // Placeholder for count

        for (i, edge) in self.region_graph.edges.iter().enumerate() {
            if edge.deleted || i as i32 == ignore_edge_idx {
                continue;
            }
            let hw = edge.width / 2.0;

            let mut poly = Vec::new();
            if edge.physical_geometry.is_empty() {
                let n1 = godot::prelude::Vector2::new(
                    self.region_graph.nodes[edge.start_node as usize].pos.x,
                    self.region_graph.nodes[edge.start_node as usize].pos.z,
                );
                let n2 = godot::prelude::Vector2::new(
                    self.region_graph.nodes[edge.end_node as usize].pos.x,
                    self.region_graph.nodes[edge.end_node as usize].pos.z,
                );
                let dir = (n2 - n1).normalized();
                if dir.length_squared() > 0.0 {
                    let norm = godot::prelude::Vector2::new(-dir.y, dir.x);
                    poly.push(n1 + norm * hw);
                    poly.push(n2 + norm * hw);
                    poly.push(n2 - norm * hw);
                    poly.push(n1 - norm * hw);
                }
            } else {
                let mut left = Vec::new();
                let mut right = Vec::new();
                let len = edge.physical_geometry.len();
                for j in 0..len {
                    let curr = godot::prelude::Vector2::new(
                        edge.physical_geometry[j].x,
                        edge.physical_geometry[j].z,
                    );
                    let tangent = if len < 2 {
                        godot::prelude::Vector2::new(1.0, 0.0)
                    } else if j == 0 {
                        (godot::prelude::Vector2::new(
                            edge.physical_geometry[1].x,
                            edge.physical_geometry[1].z,
                        ) - curr)
                            .normalized()
                    } else if j == len - 1 {
                        (curr
                            - godot::prelude::Vector2::new(
                                edge.physical_geometry[j - 1].x,
                                edge.physical_geometry[j - 1].z,
                            ))
                        .normalized()
                    } else {
                        let prev = godot::prelude::Vector2::new(
                            edge.physical_geometry[j - 1].x,
                            edge.physical_geometry[j - 1].z,
                        );
                        let next = godot::prelude::Vector2::new(
                            edge.physical_geometry[j + 1].x,
                            edge.physical_geometry[j + 1].z,
                        );
                        let d1 = (curr - prev).normalized();
                        let d2 = (next - curr).normalized();
                        let t = d1 + d2;
                        if t.length_squared() > 0.0 {
                            t.normalized()
                        } else {
                            d2
                        }
                    };
                    let norm = godot::prelude::Vector2::new(-tangent.y, tangent.x);
                    left.push(curr + norm * hw);
                    right.push(curr - norm * hw);
                }
                for v in left {
                    poly.push(v);
                }
                for v in right.into_iter().rev() {
                    poly.push(v);
                }
            }

            if !poly.is_empty() {
                data.push(poly.len() as f32);
                for v in poly {
                    data.push(v.x);
                    data.push(v.y);
                }
                count += 1.0;
            }
        }

        // 3. Include Road Nodes as obstacles to prevent zoning through intersections
        for node in &self.region_graph.nodes {
            let r = 5.0; // Intersections are protected 5m radius
            data.push(8.0); // Simple octagon
            for j in 0..8 {
                let ang = (j as f32) * std::f32::consts::TAU / 8.0;
                data.push(node.pos.x + ang.cos() * r);
                data.push(node.pos.z + ang.sin() * r);
            }
            count += 1.0;
        }

        data[0] = count;
        PackedFloat32Array::from_iter(data)
    }

    /// Returns the ID of the edge closest to the given world position.
    pub fn get_hovered_edge_internal(&self, world_x: f32, world_z: f32) -> i32 {
        let pos = godot::prelude::Vector3::new(world_x, 0.0, world_z);
        let mut best_dist = f32::MAX;
        let mut best_edge = -1;

        for (i, edge) in self.region_graph.edges.iter().enumerate() {
            let pts = &edge.physical_geometry;
            if pts.len() < 2 {
                continue;
            }
            for j in 0..pts.len() - 1 {
                let p1 = pts[j];
                let p2 = pts[j + 1];
                let p1_2d = godot::prelude::Vector2::new(p1.x, p1.z);
                let p2_2d = godot::prelude::Vector2::new(p2.x, p2.z);
                let mouse_2d = godot::prelude::Vector2::new(pos.x, pos.z);

                let l2 = (p2_2d - p1_2d).length_squared();
                let dist_sq;
                if l2 == 0.0 {
                    dist_sq = (mouse_2d - p1_2d).length_squared();
                } else {
                    let t = ((mouse_2d.x - p1_2d.x) * (p2_2d.x - p1_2d.x)
                        + (mouse_2d.y - p1_2d.y) * (p2_2d.y - p1_2d.y))
                        / l2;
                    let t = t.clamp(0.0, 1.0);
                    let proj = godot::prelude::Vector2::new(
                        p1_2d.x + t * (p2_2d.x - p1_2d.x),
                        p1_2d.y + t * (p2_2d.y - p1_2d.y),
                    );
                    dist_sq = (mouse_2d - proj).length_squared();
                }

                if dist_sq < best_dist {
                    best_dist = dist_sq;
                    best_edge = i as i32;
                }
            }
        }

        // Return if mouse is near the road, or within the dynamic bounding limit (grid depth + padding)
        let max_depth = (ZONING_DEPTH as f32 * self.config.zone_cell_m) + 10.0;
        if best_dist <= (max_depth * max_depth) {
            best_edge
        } else {
            -1
        }
    }

    /// Returns the distance to the nearest road edge along a ray.
    pub fn get_max_polygon_depth_internal(
        &self,
        origin_x: f32,
        origin_z: f32,
        dir_x: f32,
        dir_z: f32,
        max_search: f32,
    ) -> f32 {
        let o = godot::prelude::Vector2::new(origin_x, origin_z);
        let d = godot::prelude::Vector2::new(dir_x, dir_z).normalized();

        let mut min_t = max_search;

        for edge in &self.region_graph.edges {
            let pts = &edge.physical_geometry;
            if pts.len() < 2 {
                continue;
            }
            for i in 0..pts.len() - 1 {
                let p1 = godot::prelude::Vector2::new(pts[i].x, pts[i].z);
                let p2 = godot::prelude::Vector2::new(pts[i + 1].x, pts[i + 1].z);

                let v = p2 - p1;
                let det = d.x * v.y - d.y * v.x;
                if det.abs() > 0.001 {
                    let diff = p1 - o;
                    let t = (diff.x * v.y - diff.y * v.x) / det;
                    let u = (diff.x * d.y - diff.y * d.x) / det;

                    if u >= 0.0 && u <= 1.0 && t > 0.1 && t < min_t {
                        min_t = t;
                    }
                }
            }
        }

        min_t
    }

    /// Returns the closest point on an edge's boundary to the given point.
    pub fn get_closest_point_on_edge_internal(
        &self,
        edge_idx: i32,
        point_x: f32,
        point_y: f32,
    ) -> godot::prelude::Vector2 {
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edges.len() {
            return godot::prelude::Vector2::new(point_x, point_y);
        }
        let edge = &self.region_graph.edges[edge_idx as usize];
        let hw = edge.width / 2.0;

        if edge.physical_geometry.is_empty() {
            let n1 = self.region_graph.nodes[edge.start_node as usize].pos;
            let n2 = self.region_graph.nodes[edge.end_node as usize].pos;
            let a = godot::prelude::Vector2::new(n1.x, n1.z);
            let b = godot::prelude::Vector2::new(n2.x, n2.z);
            let p = godot::prelude::Vector2::new(point_x, point_y);
            let ab = b - a;
            let dot = ab.dot(ab);
            let t = if dot > 0.0 {
                ((p - a).dot(ab) / dot).clamp(0.0, 1.0)
            } else {
                0.0
            };

            let proj = a + ab * t;
            let tangent = if dot > 0.0 {
                ab.normalized()
            } else {
                godot::prelude::Vector2::new(1.0, 0.0)
            };
            let normal = godot::prelude::Vector2::new(-tangent.y, tangent.x);
            let p1 = proj + normal * hw;
            let p2 = proj - normal * hw;
            return if (p - p1).length_squared() < (p - p2).length_squared() {
                p1
            } else {
                p2
            };
        } else {
            let mut best_dist = std::f32::MAX;
            let mut best_pt = godot::prelude::Vector2::new(point_x, point_y);
            let p = godot::prelude::Vector2::new(point_x, point_y);
            for i in 0..(edge.physical_geometry.len() - 1) {
                let a = edge.physical_geometry[i];
                let b = edge.physical_geometry[i + 1];
                let a_vec = godot::prelude::Vector2::new(a.x, a.z);
                let b_vec = godot::prelude::Vector2::new(b.x, b.z);
                let ab = b_vec - a_vec;
                let dot = ab.dot(ab);
                let t = if dot > 0.0 {
                    ((p - a_vec).dot(ab) / dot).clamp(0.0, 1.0)
                } else {
                    0.0
                };

                let proj = a_vec + ab * t;
                let tangent = if dot > 0.0 {
                    ab.normalized()
                } else {
                    godot::prelude::Vector2::new(1.0, 0.0)
                };
                let normal = godot::prelude::Vector2::new(-tangent.y, tangent.x);
                let p1 = proj + normal * hw;
                let p2 = proj - normal * hw;
                let e_proj = if (p - p1).length_squared() < (p - p2).length_squared() {
                    p1
                } else {
                    p2
                };

                let dist = (e_proj - p).length_squared();
                if dist < best_dist {
                    best_dist = dist;
                    best_pt = e_proj;
                }
            }
            return best_pt;
        }
    }

    /// Returns the physical geometry of a road edge as a sequence of points.
    pub fn get_edge_geometry_internal(&self, edge_idx: i32) -> PackedVector2Array {
        let mut arr = PackedVector2Array::new();
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edges.len() {
            return arr;
        }

        let edge = &self.region_graph.edges[edge_idx as usize];
        if edge.physical_geometry.is_empty() {
            let n1 = self.region_graph.nodes[edge.start_node as usize].pos;
            let n2 = self.region_graph.nodes[edge.end_node as usize].pos;
            arr.push(godot::prelude::Vector2::new(n1.x, n1.z));
            arr.push(godot::prelude::Vector2::new(n2.x, n2.z));
        } else {
            for v in &edge.physical_geometry {
                arr.push(godot::prelude::Vector2::new(v.x, v.z));
            }
        }
        arr
    }

    /// Returns a sequence of points representing the curved frontage between two points on an edge.
    pub fn get_curved_frontage_internal(
        &self,
        edge_idx: i32,
        start_p: godot::prelude::Vector2,
        end_p: godot::prelude::Vector2,
    ) -> PackedVector2Array {
        let mut arr = PackedVector2Array::new();
        if edge_idx < 0 || edge_idx as usize >= self.region_graph.edges.len() {
            return arr; // Explicitly Fail! No straight-line phantom frontages.
        }
        let edge = &self.region_graph.edges[edge_idx as usize];
        let hw = edge.width / 2.0;

        let get_proj = |p: godot::prelude::Vector2| -> (usize, f32, godot::prelude::Vector2, godot::prelude::Vector2) {
            let mut best_dist = std::f32::MAX;
            let mut best_i = 0;
            let mut best_t = 0.0;
            let mut best_side = godot::prelude::Vector2::new(0.0, 0.0);
            let mut best_normal = godot::prelude::Vector2::new(0.0, 0.0);

            let pts: Vec<godot::prelude::Vector2> = if edge.physical_geometry.is_empty() {
                vec![
                    godot::prelude::Vector2::new(self.region_graph.nodes[edge.start_node as usize].pos.x, self.region_graph.nodes[edge.start_node as usize].pos.z),
                    godot::prelude::Vector2::new(self.region_graph.nodes[edge.end_node as usize].pos.x, self.region_graph.nodes[edge.end_node as usize].pos.z)
                ]
            } else {
                edge.physical_geometry.iter().map(|v| godot::prelude::Vector2::new(v.x, v.z)).collect()
            };

            for i in 0..(pts.len() - 1) {
                let a = pts[i]; let b = pts[i+1];
                let ab = b - a;
                let dot = ab.dot(ab);
                let t = if dot > 0.0 { ((p - a).dot(ab) / dot).clamp(0.0, 1.0) } else { 0.0 };
                let proj = a + ab * t;
                let tangent = if dot > 0.0 { ab.normalized() } else { godot::prelude::Vector2::new(1.0, 0.0) };
                let normal = godot::prelude::Vector2::new(-tangent.y, tangent.x);
                let p1 = proj + normal * hw;
                let p2 = proj - normal * hw;

                let (e_proj, side_normal) = if (p - p1).length_squared() < (p - p2).length_squared() { (p1, normal) } else { (p2, -normal) };
                let dist = (e_proj - p).length_squared();
                if dist < best_dist {
                    best_dist = dist; best_i = i; best_t = t; best_side = e_proj; best_normal = side_normal;
                }
            }
            (best_i, best_t, best_side, best_normal)
        };

        let (mut i1, t1, e1, n1) = get_proj(start_p);
        let (mut i2, t2, e2, _) = get_proj(end_p);

        let mut reverse = false;
        if i1 > i2 || (i1 == i2 && t1 > t2) {
            std::mem::swap(&mut i1, &mut i2);
            reverse = true;
        }

        let pts: Vec<godot::prelude::Vector2> = if edge.physical_geometry.is_empty() {
            vec![
                godot::prelude::Vector2::new(
                    self.region_graph.nodes[edge.start_node as usize].pos.x,
                    self.region_graph.nodes[edge.start_node as usize].pos.z,
                ),
                godot::prelude::Vector2::new(
                    self.region_graph.nodes[edge.end_node as usize].pos.x,
                    self.region_graph.nodes[edge.end_node as usize].pos.z,
                ),
            ]
        } else {
            edge.physical_geometry
                .iter()
                .map(|v| godot::prelude::Vector2::new(v.x, v.z))
                .collect()
        };

        arr.push(if reverse { e2 } else { e1 });

        for idx in (i1 + 1)..=i2 {
            let a = pts[idx - 1];
            let b = pts[idx];
            let ab = b - a;
            let tangent = if ab.dot(ab) > 0.0 {
                ab.normalized()
            } else {
                godot::prelude::Vector2::new(1.0, 0.0)
            };
            let mut normal = godot::prelude::Vector2::new(-tangent.y, tangent.x);
            if normal.dot(n1) < 0.0 {
                normal = -normal;
            } // Keep side consistent!
            arr.push(pts[idx] + normal * hw);
        }

        arr.push(if reverse { e1 } else { e2 });

        if reverse {
            let mut rev_arr = PackedVector2Array::new();
            let slice = arr.as_slice();
            for i in (0..slice.len()).rev() {
                rev_arr.push(godot::prelude::Vector2::new(slice[i].x, slice[i].y));
            }
            return rev_arr;
        }

        arr
    }

    /// Returns the terrain height at the given world position.
    pub fn get_height_at_internal(&self, pos: Vector2) -> f32 {
        let size = self.get_heightmap_size_internal();
        let hw = (size.x - 1.0) * 0.5;
        let hh = (size.y - 1.0) * 0.5;
        let gx = pos.x + hw;
        let gz = pos.y + hh;
        self.heightmap.get_height_interpolated(gx, gz) * HEIGHT_SCALE
    }

    /// Returns the demographic statistics for the city.
    pub fn get_city_demographics_internal(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();

        // Calculate population
        let pop = self.agents.count;

        let mut employed = 0;
        let mut sum_happiness = 0.0;
        let mut sum_wealth = 0.0;

        if pop > 0 {
            for i in 0..pop {
                if self.agents.work_building[i] != usize::MAX {
                    employed += 1;
                }
                sum_happiness += self.agents.happiness[i];
                sum_wealth += self.agents.money[i];
            }
            let emp_rate = (employed as f32 / pop as f32) * 100.0;
            let avg_hap = sum_happiness / pop as f32;
            let avg_wealth = sum_wealth / pop as f32;

            dict.set("population", pop as i32);
            dict.set("employment_rate", emp_rate);
            dict.set("average_happiness", avg_hap);
            dict.set("average_wealth", avg_wealth);
        } else {
            dict.set("population", 0_i32);
            dict.set("employment_rate", 0.0_f32);
            dict.set("average_happiness", 100.0_f32);
            dict.set("average_wealth", 0.0_f32);
        }

        dict
    }

    /// Returns current residential, commercial, and industrial demand values (-100 to 100).
    pub fn get_demand_stats_internal(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("residential", self.demand.residential);
        dict.set("commercial", self.demand.commercial);
        dict.set("industrial", self.demand.industrial);
        dict
    }

    /// Returns the closest network point (node/edge) within range.
    pub fn get_closest_network_point_internal(
        &self,
        world_pos: Vector3,
        max_dist: f32,
    ) -> Option<Vector3> {
        interaction::get_closest_point(&self.region_graph, world_pos, max_dist)
    }

    /// Returns the ID of the closest network node.
    pub fn get_closest_node_internal(&self, world_pos: Vector3, max_dist: f32) -> i32 {
        get_closest_canonical_node(&self.region_graph, world_pos, max_dist)
    }

    /// Returns the number of road connections for a node.
    pub fn get_node_connection_count_internal(&self, node_id: i32) -> i32 {
        if node_id < 0 || node_id as usize >= self.region_graph.adjacency.len() {
            return 0;
        }
        self.region_graph.adjacency[node_id as usize].len() as i32
    }

    /// Returns all junction node positions.
    pub fn get_network_nodes_internal(&self) -> PackedVector3Array {
        let mut arr = PackedVector3Array::new();
        for (i, node) in self.region_graph.nodes.iter().enumerate() {
            if is_canonical_node(&self.region_graph, i as u32) {
                arr.push(node.pos);
            }
        }
        arr
    }

    /// Returns the world-space position of a node.
    pub fn get_node_pos_internal(&self, node_id: u32) -> Vector3 {
        let valid_id = self.region_graph.get_valid_node(node_id);
        if (valid_id as usize) < self.region_graph.nodes.len() {
            self.region_graph.nodes[valid_id as usize].pos
        } else {
            Vector3::ZERO
        }
    }

    /// Returns an array of current lane turn restrictions at a node.
    pub fn get_lane_connections_array_internal(&self, node_id: u32) -> VarArray {
        let mut arr = VarArray::new();
        if node_id as usize >= self.region_graph.nodes.len() {
            return arr;
        }
        let node = &self.region_graph.nodes[node_id as usize];

        for (src, targets) in &node.lane_connections {
            for tgt in targets {
                let mut dict = VarDictionary::new();
                dict.set("from_edge", src.0 as i32);
                dict.set("from_lane", src.1 as i32);
                dict.set("to_edge", tgt.0 as i32);
                dict.set("to_lane", tgt.1 as i32);
                arr.push(&dict.to_variant());
            }
        }
        arr
    }

    /// Returns the average network direction at a given point.
    pub fn get_network_direction_at_point_internal(&self, pos: Vector3) -> Vector3 {
        let mut avg_dir = Vector3::ZERO;
        let mut count = 0;

        for (i, node) in self.region_graph.nodes.iter().enumerate() {
            if node.pos.distance_to(pos) < 0.1 {
                for edge in &self.region_graph.edges {
                    if edge.start_node == i as u32 {
                        if edge.physical_geometry.len() >= 2 {
                            let dir = (edge.physical_geometry[1] - edge.physical_geometry[0])
                                .normalized();
                            avg_dir += dir;
                            count += 1;
                        }
                    } else if edge.end_node == i as u32 {
                        if edge.physical_geometry.len() >= 2 {
                            let last = edge.physical_geometry.len() - 1;
                            let dir = (edge.physical_geometry[last - 1]
                                - edge.physical_geometry[last])
                                .normalized();
                            avg_dir += dir;
                            count += 1;
                        }
                    }
                }
                break;
            }
        }

        if count > 0 {
            avg_dir / count as f32
        } else {
            Vector3::ZERO
        }
    }

    /// Raycasts against the terrain heightmap.
    pub fn intersect_terrain_internal(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        self.heightmap.raycast_terrain(ray_origin, ray_dir)
    }

    /// Returns a list of all lanes at the given node for visual tool feedback.
    pub fn get_node_lanes_internal(&self, node_id: u32) -> VarArray {
        let mut arr = VarArray::new();

        let valid_node_id = self.region_graph.get_valid_node(node_id);
        if valid_node_id as usize >= self.region_graph.nodes.len() {
            return arr;
        }

        let junction_pos = self.region_graph.nodes[valid_node_id as usize].pos;

        for (e_id, edge) in self.region_graph.edges.iter().enumerate() {
            // Check both ends independently
            let check_start = edge.start_node == valid_node_id;
            let check_end = edge.end_node == valid_node_id;

            if !check_start && !check_end {
                continue;
            }

            // PREFER LOGICAL GEOMETRY for robust visuals
            let geo = if edge.geometry.len() >= 2 {
                &edge.geometry
            } else {
                &edge.physical_geometry
            };
            if geo.len() < 2 {
                continue;
            }
            let lc = geo.len();

            // Process each end that matches this junction.
            // If both match (self-loop), we process it twice for both stub ends!
            let possible_ends = if check_start && check_end {
                vec![true, false]
            } else if check_start {
                vec![true]
            } else {
                vec![false]
            };

            for is_start_side in possible_ends {
                // 1. Establish robust "Into-the-Leg" direction
                // ANCHOR: We must skip the "stub" (points near the center from merged nodes)
                // Search for the first point at least 3.1m away (HUB_RADIUS + margin)
                const SEARCH_RADIUS: f32 = 3.1;
                let mut diff = Vector3::ZERO;
                let mut best_stub = Vector3::ZERO;

                if is_start_side {
                    for j in 0..lc {
                        let d = geo[j] - junction_pos;
                        if d.length() > SEARCH_RADIUS {
                            diff = d;
                            break;
                        }
                        if d.length() > 0.1 {
                            best_stub = d;
                        }
                    }
                } else {
                    for j in (0..lc).rev() {
                        let d = geo[j] - junction_pos;
                        if d.length() > SEARCH_RADIUS {
                            diff = d;
                            break;
                        }
                        if d.length() > 0.1 {
                            best_stub = d;
                        }
                    }
                }

                // Fallback: If the road is very short, use the best stub or just the other end.
                if diff.length_squared() < 0.01 {
                    if best_stub.length_squared() > 0.01 {
                        diff = best_stub;
                    } else {
                        // Absolute fallback: other node's pos
                        let other_node = if is_start_side {
                            edge.end_node
                        } else {
                            edge.start_node
                        };
                        diff = self.region_graph.nodes[other_node as usize].pos - junction_pos;
                    }
                }

                if diff.length_squared() < 1e-6 {
                    continue;
                }
                let dir_to_leg = diff.normalized();

                // ANCHOR: Use a CONSISTENT Forward Tangent to prevent side-flipping (criss-cross)
                // If at start, dir_to_leg is forward. If at end, -dir_to_leg is forward.
                let forward_tangent = if is_start_side {
                    dir_to_leg
                } else {
                    -dir_to_leg
                };
                let road_normal = Vector3::new(-forward_tangent.z, 0.0, forward_tangent.x);

                // 2. Base position offset (5.0m ensures it's clearly past the 3.0m hub)
                let mut current_pos = junction_pos + dir_to_leg * 5.0;
                current_pos.y += 0.4;

                let fwd_lanes = edge.fwd_lanes;
                let bkw_lanes = edge.bkw_lanes;
                let total_lanes = (fwd_lanes + bkw_lanes) as i32;
                let lane_w = 1.0;

                // Process ALL lanes at this end
                for l_idx in 0..total_lanes {
                    let is_fwd = l_idx < fwd_lanes as i32;
                    // RHT Logic: Fwd lanes (lower indices) stay on the Right (+lateral_offset)
                    let lateral_offset = (total_lanes as f32 * 0.5 - l_idx as f32 - 0.5) * lane_w;

                    // Always use road_normal for lateral placement
                    let mut lane_pos = current_pos + road_normal * lateral_offset;
                    lane_pos.y += 0.2; // Slightly lower spheres for schematic view

                    let mut dict = VarDictionary::new();
                    dict.set("edge_id", e_id as i32);
                    dict.set(
                        "lane_id",
                        if is_fwd {
                            l_idx
                        } else {
                            l_idx - fwd_lanes as i32
                        },
                    );
                    dict.set(
                        "is_incoming",
                        if is_fwd {
                            !is_start_side
                        } else {
                            is_start_side
                        },
                    );
                    dict.set("pos", lane_pos);
                    arr.push(&dict.to_variant());
                }
            }
        }
        arr
    }

    /// Returns the world-space positions of all active border (external connection) nodes.
    ///
    /// Each border node is returned as a flat `[x, y, z]` triple packed into a
    /// `PackedFloat32Array`. Use this to render external-connection icons in Godot.
    /// Only canonical (non-alias) nodes with `NodeType::Border` are included.
    pub fn get_border_nodes_internal(&self) -> PackedFloat32Array {
        let mut arr = PackedFloat32Array::new();
        for (i, node) in self.region_graph.nodes.iter().enumerate() {
            if node.node_type != crate::simulation::network::types::NodeType::Border {
                continue;
            }
            if !is_canonical_node(&self.region_graph, i as u32) {
                continue;
            }
            arr.push(node.pos.x);
            arr.push(node.pos.y);
            arr.push(node.pos.z);
        }
        arr
    }
}

#[cfg(test)]
mod tests {
    use super::{get_closest_canonical_node, is_canonical_node};
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::NodeType;
    use godot::prelude::Vector3;

    #[test]
    fn merged_alias_nodes_are_not_treated_as_live_query_nodes() {
        let mut graph = RegionGraph::new();
        let keep = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let remove = graph.add_node(Vector3::new(10.2, 0.0, 0.0), NodeType::Junction);
        graph.unite_nodes(keep, remove);

        assert!(is_canonical_node(&graph, keep));
        assert!(!is_canonical_node(&graph, remove));
        assert_eq!(
            get_closest_canonical_node(&graph, Vector3::new(10.1, 0.0, 0.0), 2.0),
            keep as i32
        );
    }
}
