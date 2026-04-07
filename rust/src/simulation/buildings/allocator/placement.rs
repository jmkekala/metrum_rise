//! Explicit player-driven building placement logic.

use crate::debug_log;
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::buildings::allocator::{
    BuildingAllocator, Building, EdgeOccupancy, zone_class_to_zone_type,
};
use godot::prelude::Vector2;

const MAX_EXPLICIT_PLACE_EDGE_DISTANCE_M: f32 = 40.0;

impl BuildingAllocator {
    /// Places one explicit player-requested building on valid zoned frontage near `world_pos`.
    pub fn place_explicit_building_near_world_pos(
        &mut self,
        asset_id: &str,
        world_pos: Vector2,
        zoning: &mut ZoningSystem,
        graph: &RegionGraph,
    ) -> Result<usize, String> {
        let (zone_type, dw, dh, initial_level) = {
            let Some(entry) = self.registry.get(asset_id) else {
                return Err(format!("unknown building asset '{asset_id}'"));
            };
            let Some(building_data) = entry.manifest.building.as_ref() else {
                return Err(format!("asset '{asset_id}' is not a building"));
            };
            (
                zone_class_to_zone_type(building_data.zone_type),
                building_data.lot_width_cells as usize,
                building_data.lot_depth_cells as usize,
                building_data.level,
            )
        };

        let Some((edge_idx, projection)) = closest_placeable_edge(graph, world_pos) else {
            return Err("no nearby road frontage found".to_owned());
        };
        if projection.dist_from_road > MAX_EXPLICIT_PLACE_EDGE_DISTANCE_M {
            return Err("cursor is too far from a buildable road frontage".to_owned());
        }

        let edge = graph.edge(edge_idx);
        let edge_len = edge.physical_length;
        let cells_long = (edge_len / zoning.config.zone_cell_m).floor() as usize;
        if cells_long == 0 || dw > cells_long {
            return Err("selected road frontage is too short for that asset".to_owned());
        }

        let preferred_x = ((projection.t * edge_len) / zoning.config.zone_cell_m).floor() as isize;
        let max_leading = cells_long.saturating_sub(dw) as isize;
        let preferred_x = preferred_x.clamp(0, max_leading);

        let mut side_order = vec![projection.side];
        if projection.side != -projection.side {
            side_order.push(-projection.side);
        }

        for side in side_order {
            for x in leading_column_candidates(preferred_x, max_leading) {
                if let Some(building_idx) = self.try_place_explicit_on_slot(
                    asset_id,
                    zone_type,
                    initial_level,
                    edge_idx,
                    side,
                    x,
                    dw,
                    dh,
                    zoning,
                    graph,
                ) {
                    return Ok(building_idx);
                }
            }
        }

        Err(format!(
            "no valid {:?} frontage slot was found on the selected road for '{}'",
            zone_type, asset_id
        ))
    }

    fn try_place_explicit_on_slot(
        &mut self,
        asset_id: &str,
        zone_type: ZoneType,
        initial_level: u8,
        edge_idx: usize,
        side: i8,
        x: usize,
        dw: usize,
        dh: usize,
        zoning: &mut ZoningSystem,
        graph: &RegionGraph,
    ) -> Option<usize> {
        let edge = graph.edge(edge_idx);
        if edge.deleted || edge.no_building_spawn || edge.physical_length < 0.1 || edge.physical_geometry.len() < 2 {
            return None;
        }

        let edge_len = edge.physical_length;
        let edge_width = edge.width;
        let zone_cell_m = zoning.config.zone_cell_m;
        let cells_long = (edge_len / zone_cell_m).floor() as usize;
        if cells_long == 0 || x + dw > cells_long {
            return None;
        }

        if let Some(occ) = self.edge_occupancy.get(&edge_idx) {
            let slot = if side > 0 { &occ.left } else { &occ.right };
            if x < slot.len() && slot[x] {
                return None;
            }
        }

        let t_col = (x as f32 + 0.5) * zone_cell_m / edge_len;
        let world_pos = self.get_pos_on_edge(graph, edge_idx, t_col);
        let tangent = self.get_tangent_on_edge(graph, edge_idx, t_col);
        let normal = Vector2::new(tangent.y, -tangent.x) * (side as f32);
        let curb_dist = edge_width * 0.5 + crate::config::SIDEWALK_WIDTH;
        let frontage_center = world_pos + normal * (curb_dist + zone_cell_m * 0.5);

        let z_type = zoning.get_zone_world(frontage_center.x, frontage_center.y);
        if z_type != zone_type {
            return None;
        }

        let t_center = (x as f32 + dw as f32 * 0.5) * zone_cell_m / edge_len;
        let world_pos_on_edge = Self::sample_pos_on_edge(graph, edge_idx, t_center);
        let tangent_c = Self::sample_tangent_on_edge(graph, edge_idx, t_center);
        let normal_c = Vector2::new(tangent_c.y, -tangent_c.x) * (side as f32);
        let depth_offset = crate::config::SIDEWALK_WIDTH + (dh as f32 * 0.5) * zone_cell_m;
        let center_2d = world_pos_on_edge + normal_c * (edge_width * 0.5 + depth_offset);

        let mut can_build = true;
        'zone_check: for dx in 0..dw {
            let t_dx = (x as f32 + dx as f32 + 0.5) * zone_cell_m / edge_len;
            let wp = Self::sample_pos_on_edge(graph, edge_idx, t_dx);
            let td = Self::sample_tangent_on_edge(graph, edge_idx, t_dx);
            let nd = Vector2::new(td.y, -td.x) * (side as f32);
            for dy in 0..dh {
                let cell_center = wp + nd * (curb_dist + (dy as f32 + 0.5) * zone_cell_m);
                if zoning.get_zone_world(cell_center.x, cell_center.y) != zone_type {
                    can_build = false;
                    break 'zone_check;
                }
            }
        }
        if !can_build {
            return None;
        }

        let width_m = dw as f32 * zone_cell_m;
        let depth_m = dh as f32 * zone_cell_m;
        if zoning.is_rect_occupied(center_2d.x, center_2d.y, tangent_c, width_m, depth_m) {
            return None;
        }

        let half_depth = depth_m * 0.5;
        let road_dist = zoning.distance_to_road_world(center_2d.x, center_2d.y) as f32;
        if road_dist < half_depth {
            return None;
        }

        zoning.mark_occupied_rect(center_2d.x, center_2d.y, tangent_c, width_m, depth_m, true);

        let occ = self.edge_occupancy.entry(edge_idx).or_insert_with(|| EdgeOccupancy {
            cells_long,
            left: vec![false; cells_long],
            right: vec![false; cells_long],
        });
        let slot = if side > 0 { &mut occ.left } else { &mut occ.right };
        if x < slot.len() {
            slot[x] = true;
        }

        let building_idx = self.place_building_instance(
            asset_id.to_owned(),
            zone_type,
            initial_level,
            edge_idx,
            side,
            x,
            dw,
            dh,
            center_2d,
            normal_c,
            t_center,
            edge_width,
        );
        self.dirty = true;
        self.dirty_index = true;
        self.dirty_zones[zone_type as usize] = true;
        debug_log!(
            "economy",
            "player placed startup building idx={} asset_id={} zone={:?} edge={} cell=({}, {}) center=({:.1}, {:.1})",
            building_idx,
            asset_id,
            zone_type,
            edge_idx,
            x,
            0,
            center_2d.x,
            center_2d.y
        );
        Some(building_idx)
    }

    fn place_building_instance(
        &mut self,
        asset_id: String,
        zone_type: ZoneType,
        initial_level: u8,
        edge_idx: usize,
        side: i8,
        cell_x: usize,
        width_cells: usize,
        depth_cells: usize,
        center_2d: Vector2,
        facing_dir: Vector2,
        frontage_t: f32,
        edge_width: f32,
    ) -> usize {
        self.buildings.push(Building {
            zone_type,
            facing_dir,
            frontage_t,
            side_offset: edge_width * 0.5 + crate::config::SIDEWALK_WIDTH,
            center_x: center_2d.x,
            center_y: center_2d.y,
            edge_idx,
            side,
            cell_x,
            cell_y: 0,
            width_cells: width_cells as u16,
            depth_cells: depth_cells as u16,
            occupancy: 0,
            worker_count: 0,
            asset_id,
            level: initial_level,
            broken: false,
            stock: 0.0,
            revenue: 0.0,
            operating_budget: 0.0,
            utility_service_available: false,
            shipment_cooldown_days: 0,
            abandoned_timer: 0,
        });
        self.buildings.len() - 1
    }

}

fn projection_on_edge(
    graph: &RegionGraph,
    edge_idx: usize,
    point: Vector2,
) -> EdgeProjection {
    let edge = graph.edge(edge_idx);
    let mut best_dist_sq = f32::MAX;
    let mut best_t = 0.0;
    let mut best_side = 1i8;
    let mut best_dist = f32::MAX;
    let mut curr_l = 0.0;

    for i in 0..edge.physical_geometry.len() - 1 {
        let a = Vector2::new(edge.physical_geometry[i].x, edge.physical_geometry[i].z);
        let b = Vector2::new(edge.physical_geometry[i + 1].x, edge.physical_geometry[i + 1].z);
        let seg = b - a;
        let len_sq = seg.length_squared();
        if len_sq < 0.001 {
            continue;
        }

        let local_t = ((point - a).dot(seg) / len_sq).clamp(0.0, 1.0);
        let proj = a + seg * local_t;
        let dist_sq = point.distance_squared_to(proj);
        if dist_sq < best_dist_sq {
            let seg_len = len_sq.sqrt();
            best_dist_sq = dist_sq;
            best_t = ((curr_l + local_t * seg_len) / edge.physical_length).clamp(0.0, 1.0);
            let tangent = seg.normalized();
            let normal = Vector2::new(tangent.y, -tangent.x);
            best_side = if (point - proj).dot(normal) >= 0.0 { 1 } else { -1 };
            best_dist = dist_sq.sqrt();
        }
        curr_l += len_sq.sqrt();
    }

    EdgeProjection {
        t: best_t,
        side: best_side,
        dist_from_road: best_dist,
    }
}

fn closest_placeable_edge(graph: &RegionGraph, point: Vector2) -> Option<(usize, EdgeProjection)> {
    let mut best: Option<(usize, EdgeProjection)> = None;
    let mut best_dist_sq = f32::MAX;

    for edge_idx in 0..graph.edge_count() {
        let edge = graph.edge(edge_idx);
        if edge.deleted || edge.no_building_spawn || edge.physical_geometry.len() < 2 || edge.physical_length < 0.1 {
            continue;
        }
        let projection = projection_on_edge(graph, edge_idx, point);
        let dist_sq = projection.dist_from_road * projection.dist_from_road;
        if dist_sq < best_dist_sq {
            best_dist_sq = dist_sq;
            best = Some((edge_idx, projection));
        }
    }

    best
}

fn leading_column_candidates(preferred: isize, max_leading: isize) -> Vec<usize> {
    let mut out = Vec::with_capacity(max_leading.saturating_add(1) as usize);
    let mut seen = std::collections::HashSet::new();
    for delta in 0..=max_leading {
        for candidate in [preferred - delta, preferred + delta] {
            if candidate < 0 || candidate > max_leading {
                continue;
            }
            if seen.insert(candidate) {
                out.push(candidate as usize);
            }
        }
    }
    out
}

struct EdgeProjection {
    t: f32,
    side: i8,
    dist_from_road: f32,
}
