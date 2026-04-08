//! Building removal, immigration spawning, and coordinate restoration.

use crate::debug_log;
use crate::simulation::buildings::allocator::{BuildingAllocator, building_depart_node};
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::households::{DEFAULT_IMMIGRANT_HOUSEHOLD_SIZE, HouseholdSystem};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::grid::zoning::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::types::NodeType;
use godot::prelude::Vector2;

const IMMIGRATION_BASE_INFLOW: f32 = 1.0;
const MAX_IMMIGRANT_HOUSEHOLDS_PER_DAY: usize = 4;
const HOME_PICK_SAMPLE_ATTEMPTS: usize = 8;
const PLAYER_STARTUP_POPULATION_TARGET: usize = 8;

impl BuildingAllocator {
    /// Removes buildings if their zone category has changed or their road edge no longer exists.
    pub(super) fn cleanup_stale_buildings(
        &mut self,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        logistics: &mut ShipmentSystem,
        graph: &RegionGraph,
        lanes: &LaneSystem,
    ) {
        let zone_cell_m = zoning.config.zone_cell_m;
        let mut removed_any = false;
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
                let width_m = b_width as f32 * zone_cell_m;
                let depth_m = b_depth as f32 * zone_cell_m;
                zoning.mark_occupied_rect(b_center_x, b_center_y, tangent, width_m, depth_m, false);

                if let Some(occ) = self.edge_occupancy.get_mut(&b_edge_idx) {
                    let slot = if b_side > 0 {
                        &mut occ.left
                    } else {
                        &mut occ.right
                    };
                    if b_cell_x < slot.len() {
                        slot[b_cell_x] = false;
                    }
                }

                logistics.invalidate_building(i, self);
                let last_idx = self.buildings.len() - 1;
                if i < last_idx {
                    self.dirty_zones[self.buildings[last_idx].zone_type as usize] = true;
                    let mut mapping = std::collections::HashMap::new();
                    mapping.insert(last_idx, i);
                    agents.remap_building_indices(&mapping);
                    logistics.remap_building_indices(&mapping);
                }

                self.buildings.swap_remove(i);
                self.dirty_index = true;
                self.entrances_dirty = true;
                removed_any = true;
            } else {
                i += 1;
            }
        }
        if removed_any {
            self.rebuild_entrance_cache(graph, lanes);
        }
    }

    /// Admits immigrant households through border nodes and assigns them to available homes.
    pub(super) fn spawn_immigrants(
        &mut self,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        graph: &RegionGraph,
    ) {
        let vacant_resident_slots: usize = self
            .buildings
            .iter()
            .enumerate()
            .filter(|(_, b)| b.zone_type == ZoneType::Residential || b.zone_type == ZoneType::Mixed)
            .fold(0, |acc, (idx, b)| {
                if b.broken {
                    return acc;
                }
                let cap = self.resident_capacity(idx);
                let cap = if cap == 0 { 6 } else { cap } as usize;
                acc + cap.saturating_sub(b.occupancy as usize)
            });

        if vacant_resident_slots == 0 {
            debug_log!("economy", "immigration blocked: no vacant resident slots");
            return;
        }

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
                    .iter()
                    .any(|&e| !graph.edge(e).deleted);
                if connected { Some(i as u32) } else { None }
            })
            .collect();
        if border_nodes.is_empty() {
            debug_log!("economy", "immigration blocked: no connected Border nodes");
            return;
        }

        let resident_count: f32 = households
            .households
            .iter()
            .filter(|household| household.member_count > 0)
            .map(|household| household.member_count as f32)
            .sum();
        let open_job_slots: f32 = self
            .buildings
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                !b.broken
                    && matches!(
                        b.zone_type,
                        ZoneType::Commercial
                            | ZoneType::Industrial
                            | ZoneType::Office
                            | ZoneType::Mixed
                    )
            })
            .map(|(idx, building)| {
                self.worker_capacity(idx)
                    .saturating_sub(building.worker_count) as f32
            })
            .sum();

        let housing_factor = (vacant_resident_slots as f32
            / (vacant_resident_slots as f32 + resident_count.max(1.0)))
        .clamp(0.0, 1.0);
        let job_factor = if resident_count == 0.0 {
            1.0
        } else {
            (open_job_slots / (open_job_slots + DEFAULT_IMMIGRANT_HOUSEHOLD_SIZE as f32))
                .clamp(0.0, 1.0)
        };

        let mut stock_stability_sum = 0.0;
        let mut utility_stability_sum = 0.0;
        let mut active_households = 0.0;
        for household in &households.households {
            if household.member_count == 0 {
                continue;
            }
            stock_stability_sum += (household.stock_days / 3.0).clamp(0.0, 1.0);
            let utility_ok = household.home_building_id < self.buildings.len()
                && self.buildings[household.home_building_id].utility_service_available;
            utility_stability_sum += if utility_ok { 1.0 } else { 0.0 };
            active_households += 1.0;
        }
        let city_stability_factor = if active_households > 0.0 {
            (0.6 * (stock_stability_sum / active_households)
                + 0.4 * (utility_stability_sum / active_households))
                .clamp(0.0, 1.0)
        } else {
            1.0
        };

        let mut households_to_spawn =
            (IMMIGRATION_BASE_INFLOW * housing_factor * job_factor * city_stability_factor).round()
                as usize;
        let startup_ready = resident_count < PLAYER_STARTUP_POPULATION_TARGET as f32
            && vacant_resident_slots > 0
            && open_job_slots > 0.0;
        if startup_ready {
            households_to_spawn = households_to_spawn.max(1);
        }
        if households_to_spawn == 0 {
            debug_log!(
                "economy",
                "immigration blocked: formula rounded to zero (housing_factor={:.2}, job_factor={:.2}, stability={:.2}, vacant_slots={}, border_nodes={})",
                housing_factor,
                job_factor,
                city_stability_factor,
                vacant_resident_slots,
                border_nodes.len()
            );
            return;
        }

        let households_to_spawn = households_to_spawn
            .min(MAX_IMMIGRANT_HOUSEHOLDS_PER_DAY)
            .min(vacant_resident_slots);
        debug_log!(
            "economy",
            "immigration planning: households_to_spawn={} vacant_slots={} border_nodes={} resident_count={} open_job_slots={:.1} housing_factor={:.2} job_factor={:.2} stability={:.2}",
            households_to_spawn,
            vacant_resident_slots,
            border_nodes.len(),
            resident_count as usize,
            open_job_slots,
            housing_factor,
            job_factor,
            city_stability_factor
        );

        let mut rng = rand::thread_rng();
        for _ in 0..households_to_spawn {
            let Some((home_idx, household_size)) =
                self.claim_home_for_household(DEFAULT_IMMIGRANT_HOUSEHOLD_SIZE as u32, &mut rng)
            else {
                debug_log!(
                    "economy",
                    "immigration aborted mid-pass: could not claim a home from vacancy index"
                );
                break;
            };
            let spawn_node = border_nodes[rand::Rng::gen_range(&mut rng, 0..border_nodes.len())];
            let mut spawn_pos = graph.node(spawn_node).pos;

            if let Some(&edge_idx) = graph.node_adjacency(spawn_node).get(0) {
                let edge = graph.edge(edge_idx);
                if edge.physical_geometry.len() >= 2 {
                    let dir = if edge.start_node == spawn_node {
                        (edge.physical_geometry[1] - edge.physical_geometry[0]).normalized()
                    } else {
                        (edge.physical_geometry[edge.physical_geometry.len() - 2]
                            - edge.physical_geometry[edge.physical_geometry.len() - 1])
                            .normalized()
                    };
                    let side_mul = if crate::config::DRIVE_ON_LEFT {
                        -1.0
                    } else {
                        1.0
                    };
                    let normal = godot::prelude::Vector3::new(-dir.z, 0.0, dir.x);
                    spawn_pos += normal * (crate::config::LANE_WIDTH * 0.5 * side_mul);
                }
            }

            let home_bldg = &self.buildings[home_idx];
            let home_node = building_depart_node(home_bldg, graph);
            let household_id = households.admit_immigrant_household(home_idx, household_size);
            debug_log!(
                "economy",
                "immigration admitted household_id={} size={} home_building={} spawn_node={} home_node={}",
                household_id,
                household_size,
                home_idx,
                spawn_node,
                home_node
            );

            for _ in 0..household_size {
                let agent_idx = agents.spawn_agent(
                    home_idx,
                    home_node,
                    0.0,
                    0.0,
                    spawn_node,
                    spawn_pos.x,
                    spawn_pos.z,
                );
                agents.household_id[agent_idx] = household_id;
                debug_log!(
                    "economy",
                    "immigration spawned agent_idx={} household_id={} current_node={} target_node={} pos=({:.1}, {:.1})",
                    agent_idx,
                    household_id,
                    spawn_node,
                    home_node,
                    spawn_pos.x,
                    spawn_pos.z
                );
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

impl BuildingAllocator {
    fn claim_home_for_household(
        &mut self,
        desired_size: u32,
        rng: &mut impl rand::Rng,
    ) -> Option<(usize, u16)> {
        let target_zones = [ZoneType::Residential, ZoneType::Mixed];
        let total_candidates: usize = target_zones
            .iter()
            .map(|&zone| self.vacancy_index[zone as usize].len())
            .sum();
        if total_candidates == 0 {
            return None;
        }

        for _ in 0..HOME_PICK_SAMPLE_ATTEMPTS {
            let mut pick = rng.gen_range(0..total_candidates);
            let mut building_idx = usize::MAX;
            for &zone in &target_zones {
                let list = &self.vacancy_index[zone as usize];
                if pick < list.len() {
                    building_idx = list[pick];
                    break;
                }
                pick -= list.len();
            }
            if building_idx == usize::MAX {
                continue;
            }

            let free_slots = self
                .resident_capacity(building_idx)
                .saturating_sub(self.buildings[building_idx].occupancy);
            if free_slots == 0 {
                continue;
            }
            let admitted_size = free_slots.min(desired_size).max(1) as u16;
            for _ in 0..admitted_size {
                self.claim_vacancy(building_idx);
            }
            return Some((building_idx, admitted_size));
        }

        let mut fallback_idx = usize::MAX;
        let mut fallback_size = 0_u16;
        'fallback: for &zone in &target_zones {
            for &building_idx in &self.vacancy_index[zone as usize] {
                let free_slots = self
                    .resident_capacity(building_idx)
                    .saturating_sub(self.buildings[building_idx].occupancy);
                if free_slots == 0 {
                    continue;
                }
                fallback_idx = building_idx;
                fallback_size = free_slots.min(desired_size).max(1) as u16;
                break 'fallback;
            }
        }
        if fallback_idx == usize::MAX {
            return None;
        }
        for _ in 0..fallback_size {
            self.claim_vacancy(fallback_idx);
        }
        Some((fallback_idx, fallback_size))
    }
}
