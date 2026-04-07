//! Building removal, immigration spawning, and coordinate restoration.

use crate::simulation::grid::zoning::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::buildings::allocator::{BuildingAllocator, building_depart_node};
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::types::NodeType;
use godot::prelude::Vector2;

impl BuildingAllocator {
    /// Removes buildings if their zone category has changed or their road edge no longer exists.
    pub(super) fn cleanup_stale_buildings(
        &mut self,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        graph: &RegionGraph,
    ) {
        let zone_cell_m = zoning.config.zone_cell_m;
        let mut i = 0;
        while i < self.buildings.len() {
            let b = &self.buildings[i];
            let remove = {
                let edge_ok = b.edge_idx < graph.edge_count() && !graph.edge(b.edge_idx).deleted;
                if !edge_ok {
                    true
                } else if graph.edge(b.edge_idx).no_building_spawn {
                    true
                } else {
                    let half_depth = b.depth_cells as f32 * zone_cell_m * 0.5;
                    let road_dist = zoning.distance_to_road_world(b.center_x, b.center_y) as f32;
                    if road_dist < half_depth {
                        true
                    } else {
                        let current_zone = zoning.get_zone_world(b.center_x, b.center_y);
                        current_zone != b.zone_type
                    }
                }
            };

            if remove {
                let b_edge_idx = b.edge_idx;
                let b_side = b.side;
                let b_cell_x = b.cell_x;
                let b_center_x = b.center_x;
                let b_center_y = b.center_y;
                let b_facing = b.facing_dir;
                let b_width = b.width_cells;
                let b_depth = b.depth_cells;
                let b_zone = b.zone_type;
                self.dirty_zones[b_zone as usize] = true;

                let tangent = Vector2::new(-b_facing.y, b_facing.x);
                let width_m  = b_width as f32 * zone_cell_m;
                let depth_m  = b_depth as f32 * zone_cell_m;
                zoning.mark_occupied_rect(b_center_x, b_center_y, tangent, width_m, depth_m, false);

                if let Some(occ) = self.edge_occupancy.get_mut(&b_edge_idx) {
                    let slot = if b_side > 0 { &mut occ.left } else { &mut occ.right };
                    if b_cell_x < slot.len() { slot[b_cell_x] = false; }
                }

                let last_idx = self.buildings.len() - 1;
                if i < last_idx {
                    self.dirty_zones[self.buildings[last_idx].zone_type as usize] = true;
                    let mut mapping = std::collections::HashMap::new();
                    mapping.insert(last_idx, i);
                    agents.remap_building_indices(&mapping);
                }

                self.buildings.swap_remove(i);
                self.dirty_index = true;
            } else {
                i += 1;
            }
        }
    }

    /// Spawns immigrant agents at border nodes and assigns them to available homes.
    pub(super) fn spawn_immigrants(
        &mut self,
        demand_residential: f32,
        agents: &mut AgentSystem,
        graph: &RegionGraph,
    ) {
        let total_capacity: usize = self
            .buildings
            .iter()
            .enumerate()
            .filter(|(_, b)| b.zone_type == ZoneType::Residential || b.zone_type == ZoneType::Mixed)
            .fold(0, |acc, (idx, b)| {
                if b.broken { return acc; }
                let cap = self.resident_capacity(idx);
                let cap = if cap == 0 { 6 } else { cap } as usize;
                acc + cap.saturating_sub(b.occupancy as usize)
            });

        if agents.len() < total_capacity {
            let demand_factor = (demand_residential / 100.0).max(0.0).min(1.0);
            let gap = total_capacity - agents.len();
            let num_to_spawn = ((gap as f32 * 0.2 * demand_factor) as usize).max(1).min(10);

            let border_nodes: Vec<u32> = graph
                .nodes()
                .iter()
                .enumerate()
                .filter_map(|(i, node)| {
                    if node.node_type != NodeType::Border {
                        return None;
                    }
                    let connected = graph
                        .node_adjacency(i as u32)
                        .iter().any(|&e| !graph.edge(e).deleted);
                    if connected { Some(i as u32) } else { None }
                })
                .collect();

            if !border_nodes.is_empty() {
                let mut rng = rand::thread_rng();
                for _ in 0..num_to_spawn {
                    if let Some(home_idx) = agents.find_available_home(self) {
                        let spawn_node =
                            border_nodes[rand::Rng::gen_range(&mut rng, 0..border_nodes.len())];
                        let mut spawn_pos = graph.node(spawn_node).pos;
                        
                        if let Some(&edge_idx) = graph.node_adjacency(spawn_node).get(0) {
                            let edge = graph.edge(edge_idx);
                            if edge.physical_geometry.len() >= 2 {
                                let dir = if edge.start_node == spawn_node {
                                    (edge.physical_geometry[1] - edge.physical_geometry[0]).normalized()
                                } else {
                                    (edge.physical_geometry[edge.physical_geometry.len()-2] - edge.physical_geometry[edge.physical_geometry.len()-1]).normalized()
                                };
                                let side_mul = if crate::config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
                                let normal = godot::prelude::Vector3::new(-dir.z, 0.0, dir.x);
                                spawn_pos += normal * (crate::config::LANE_WIDTH * 0.5 * side_mul);
                            }
                        }

                        let home_bldg = &self.buildings[home_idx];
                        let home_node = building_depart_node(home_bldg, graph);

                        agents.spawn_agent(
                            home_idx,
                            home_node,
                            0.0,
                            0.0,
                            spawn_node,
                            spawn_pos.x,
                            spawn_pos.z,
                        );
                    } else {
                        break;
                    }
                }
            }
        }
    }

    /// Recomputes world-space building transforms from saved frontage attachment data.
    pub(crate) fn recompute_derived_transforms(
        &mut self,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
    ) -> Result<(), String> {
        for building in &mut self.buildings {
            if building.edge_idx >= graph.edge_count() {
                return Err(format!(
                    "building edge {} out of bounds for {} edges",
                    building.edge_idx,
                    graph.edge_count()
                ));
            }

            let edge = graph.edge(building.edge_idx);
            if edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
                return Err(format!(
                    "building edge {} has insufficient geometry for transform rebuild",
                    building.edge_idx
                ));
            }

            let zone_cell_m = zoning.config.zone_cell_m;
            let width_cells = building.width_cells as f32;
            let depth_cells = building.depth_cells as f32;
            let along_offset = width_cells * 0.5 * zone_cell_m;
            let depth_offset = crate::config::SIDEWALK_WIDTH
                + (building.cell_y as f32 + depth_cells * 0.5) * zone_cell_m;
            let edge_t =
                (building.cell_x as f32 * zone_cell_m / edge.physical_length).clamp(0.0, 1.0);

            let world_pos_on_edge = Self::sample_pos_on_edge(graph, building.edge_idx, edge_t);
            let tangent = Self::sample_tangent_on_edge(graph, building.edge_idx, edge_t);
            let normal = Vector2::new(tangent.y, -tangent.x) * building.side as f32;
            let center_2d = world_pos_on_edge
                + normal * (edge.width * 0.5 + depth_offset)
                + tangent * along_offset;

            building.center_x = center_2d.x;
            building.center_y = center_2d.y;
            building.facing_dir = normal;
            building.side_offset = building.side as f32;
        }

        self.dirty = true;
        Ok(())
    }
}
