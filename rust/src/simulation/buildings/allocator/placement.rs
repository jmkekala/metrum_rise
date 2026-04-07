//! Building placement and growth logic.

use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::core::config::MapConfig;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::buildings::allocator::{BuildingAllocator, Building, EdgeOccupancy, zone_type_to_zone_class};
use godot::prelude::Vector2;

impl BuildingAllocator {
    /// Scans road frontage cells for unoccupied zoned land and spawns new buildings.
    pub(super) fn spawn_new_buildings(
        &mut self,
        demand: &mut DemandSystem,
        zoning: &mut ZoningSystem,
        desirability: &DesirabilitySystem,
        graph: &RegionGraph,
        config: &MapConfig,
    ) {
        let mut spawned_this_tick = 0;
        let max_spawns = 10;
        let zone_cell_m = zoning.config.zone_cell_m;

        'edge_loop: for edge_idx in 0..graph.edge_count() {
            if spawned_this_tick >= max_spawns {
                break;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted || edge.no_building_spawn || edge.physical_length < 0.1 || edge.physical_geometry.len() < 2 {
                continue;
            }
            let edge_len   = edge.physical_length;
            let edge_width = edge.width;
            let cells_long = (edge_len / zone_cell_m).floor() as usize;
            if cells_long == 0 { continue; }

            'side_loop: for side in [1i8, -1i8] {
                for i in 0..cells_long {
                    // Alternate scan direction for visual variety.
                    let x = if i % 2 == 0 { (i / 2).min(cells_long - 1) } else { (cells_long - 1).saturating_sub(i / 2) };
                    if spawned_this_tick >= max_spawns {
                        break 'edge_loop;
                    }

                    // Fast pre-check: is the frontage column already taken by a building on this edge?
                    if let Some(occ) = self.edge_occupancy.get(&edge_idx) {
                        let slot = if side > 0 { &occ.left } else { &occ.right };
                        if x < slot.len() && slot[x] { continue; }
                    }

                    // Compute world position of frontage cell centre (row 0).
                    let t_col = (x as f32 + 0.5) * zone_cell_m / edge_len;
                    let world_pos = self.get_pos_on_edge(graph, edge_idx, t_col);
                    let tangent   = self.get_tangent_on_edge(graph, edge_idx, t_col);
                    let normal    = Vector2::new(tangent.y, -tangent.x) * (side as f32);
                    let curb_dist = edge_width * 0.5 + crate::config::SIDEWALK_WIDTH;
                    let frontage_center = world_pos + normal * (curb_dist + zone_cell_m * 0.5);

                    // Ownership check: if a closer road surface exists, that road's scan will
                    // claim this slot. Skip to avoid buildings facing the wrong road.
                    {
                        let road_dist = zoning.distance_to_road_world(frontage_center.x, frontage_center.y);
                        let expected: u8 = (crate::config::SIDEWALK_WIDTH + zone_cell_m * 0.5 - 1.5) as u8;
                        if road_dist < expected {
                            continue;
                        }
                    }

                    let z_type = zoning.get_zone_world(frontage_center.x, frontage_center.y);
                    if z_type == ZoneType::None {
                        continue;
                    }

                    let d_val = match z_type {
                        ZoneType::Residential => demand.residential,
                        ZoneType::Commercial  => demand.commercial,
                        ZoneType::Industrial  => demand.industrial,
                        ZoneType::Office      => demand.commercial * 0.5,
                        ZoneType::Mixed       => (demand.residential + demand.commercial) * 0.5,
                        _ => 0.0,
                    };
                    if d_val < 10.0 { continue; }

                    // Select a registered asset for this zone.
                    let zone_class = zone_type_to_zone_class(z_type);
                    let candidates = zone_class.map(|zc| self.registry.buildings_for_zone(zc)).unwrap_or(&[]);
                    if candidates.is_empty() { continue; }
                    let asset_id = candidates[(edge_idx ^ x) % candidates.len()].clone();
                    let (dw, dh) = self.registry.lot_size(&asset_id);

                    // Footprint must fit within remaining columns.
                    if x + dw > cells_long { continue; }

                    // Compute footprint centre in world space.
                    let t_center = (x as f32 + dw as f32 * 0.5) * zone_cell_m / edge_len;
                    let world_pos_on_edge = Self::sample_pos_on_edge(graph, edge_idx, t_center);
                    let tangent_c = Self::sample_tangent_on_edge(graph, edge_idx, t_center);
                    let normal_c  = Vector2::new(tangent_c.y, -tangent_c.x) * (side as f32);
                    let depth_offset = crate::config::SIDEWALK_WIDTH + (dh as f32 * 0.5) * zone_cell_m;
                    let center_2d = world_pos_on_edge + normal_c * (edge_width * 0.5 + depth_offset);

                    // Zone check: all footprint cells must share the same zone type.
                    let mut can_build = true;
                    'zone_check: for dx in 0..dw {
                        let t_dx = (x as f32 + dx as f32 + 0.5) * zone_cell_m / edge_len;
                        let wp = Self::sample_pos_on_edge(graph, edge_idx, t_dx);
                        let td = Self::sample_tangent_on_edge(graph, edge_idx, t_dx);
                        let nd = Vector2::new(td.y, -td.x) * (side as f32);
                        for dy in 0..dh {
                            let cell_center = wp + nd * (curb_dist + (dy as f32 + 0.5) * zone_cell_m);
                            if zoning.get_zone_world(cell_center.x, cell_center.y) != z_type {
                                can_build = false;
                                break 'zone_check;
                            }
                        }
                    }
                    if !can_build { continue; }

                    // Rotated-rect occupancy check on the world grid.
                    let width_m = dw as f32 * zone_cell_m;
                    let depth_m = dh as f32 * zone_cell_m;
                    if zoning.is_rect_occupied(center_2d.x, center_2d.y, tangent_c, width_m, depth_m) {
                        continue;
                    }

                    // Reject if the building centre is inside another road's carriageway.
                    {
                        let half_depth = depth_m * 0.5;
                        let road_dist = zoning.distance_to_road_world(center_2d.x, center_2d.y) as f32;
                        if road_dist < half_depth {
                            continue;
                        }
                    }

                    // Desirability Gate.
                    {
                        let (gx_raw, gy_raw) = config.world_to_env_grid(
                            center_2d.x, center_2d.y,
                            desirability.grid.width, desirability.grid.height,
                        );
                        let gx = (gx_raw.round() as usize).min(desirability.grid.width.saturating_sub(1));
                        let gy = (gy_raw.round() as usize).min(desirability.grid.height.saturating_sub(1));
                        if *desirability.grid.get(gx, gy).unwrap_or(&50.0) < 20.0 {
                            continue;
                        }
                    }

                    // All checks passed — place the building.
                    zoning.mark_occupied_rect(center_2d.x, center_2d.y, tangent_c, width_m, depth_m, true);

                    let occ = self.edge_occupancy.entry(edge_idx).or_insert_with(|| EdgeOccupancy {
                        cells_long,
                        left:  vec![false; cells_long],
                        right: vec![false; cells_long],
                    });
                    let slot = if side > 0 { &mut occ.left } else { &mut occ.right };
                    if x < slot.len() { slot[x] = true; }

                    let initial_level = self.registry.get(&asset_id)
                        .and_then(|e| e.manifest.building.as_ref())
                        .map(|b| b.level)
                        .unwrap_or(1);

                    self.buildings.push(Building {
                        zone_type: z_type,
                        facing_dir: normal_c,
                        frontage_t: t_center,
                        side_offset: edge_width * 0.5 + crate::config::SIDEWALK_WIDTH,
                        center_x: center_2d.x,
                        center_y: center_2d.y,
                        edge_idx,
                        side,
                        cell_x: x,
                        cell_y: 0,
                        width_cells: dw as u16,
                        depth_cells: dh as u16,
                        occupancy: 0,
                        worker_count: 0,
                        asset_id,
                        level: initial_level,
                        broken: false,
                        stock: 0.0,
                        revenue: 0.0,
                        operating_budget: 500.0,
                        utility_service_available: false,
                        abandoned_timer: 0,
                    });

                    spawned_this_tick += 1;
                    self.dirty_index = true;
                    self.dirty_zones[z_type as usize] = true;

                    match z_type {
                        ZoneType::Residential => demand.residential -= 5.0,
                        ZoneType::Commercial  => demand.commercial  -= 5.0,
                        ZoneType::Industrial  => demand.industrial  -= 5.0,
                        _ => {}
                    }

                    break 'side_loop;
                }
            }
        }
    }

    /// Buildings whose demand + desirability conditions are met level up
    /// in-place when a higher-tier asset in the same family is registered.
    pub(super) fn update_building_levels(
        &mut self,
        demand: &DemandSystem,
        desirability: &DesirabilitySystem,
        config: &MapConfig,
    ) {
        for b in &mut self.buildings {
            let Some(next_id) = self.registry.next_level(&b.asset_id) else { continue };
            let demand_ok = match b.zone_type {
                ZoneType::Residential => demand.residential > 50.0,
                ZoneType::Commercial  => demand.commercial  > 50.0,
                ZoneType::Industrial  => demand.industrial  > 50.0,
                _ => false,
            };
            if !demand_ok { continue; }
            let (gx_raw, gy_raw) = config.world_to_env_grid(
                b.center_x, b.center_y,
                desirability.grid.width, desirability.grid.height,
            );
            let gx = (gx_raw.round() as usize).min(desirability.grid.width.saturating_sub(1));
            let gy = (gy_raw.round() as usize).min(desirability.grid.height.saturating_sub(1));
            if *desirability.grid.get(gx, gy).unwrap_or(&0.0) < 60.0 { continue; }

            b.asset_id = next_id.to_owned();
            b.level = self.registry.get(&b.asset_id)
                .and_then(|e| e.manifest.building.as_ref())
                .map(|bd| bd.level)
                .unwrap_or(b.level + 1);
            self.dirty = true;
            self.dirty_zones[b.zone_type as usize] = true;
        }
    }
}
