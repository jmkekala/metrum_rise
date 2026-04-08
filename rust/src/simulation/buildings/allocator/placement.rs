//! One-time founding bootstrap placement and frontage-slot resolution.

use crate::assets::ZoneClass;
use crate::debug_log;
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, EdgeOccupancy, zone_class_to_zone_type,
};
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::NodeType;
use godot::prelude::Vector2;

const FOUNDING_RESIDENTIAL_ZONE: ZoneClass = ZoneClass::Residential;
const FOUNDING_COMMERCIAL_ZONE: ZoneClass = ZoneClass::Commercial;

impl BuildingAllocator {
    /// Places the one-time founding bootstrap when the city has a border
    /// connection plus valid residential and commercial zoned frontage.
    pub fn place_founding_bootstrap_if_ready(
        &mut self,
        zoning: &mut ZoningSystem,
        graph: &RegionGraph,
    ) {
        if self.founding_bootstrap_consumed {
            return;
        }

        if !self.buildings.is_empty() {
            self.founding_bootstrap_consumed = true;
            debug_log!(
                "economy",
                "founding bootstrap skipped permanently: city already has {} building(s)",
                self.buildings.len()
            );
            return;
        }

        if !has_connected_border_node(graph) {
            debug_log!(
                "economy",
                "founding bootstrap waiting: no connected external Border node"
            );
            return;
        }

        let Some(residential_asset_id) =
            self.preferred_founding_asset_id(FOUNDING_RESIDENTIAL_ZONE)
        else {
            debug_log!(
                "economy",
                "founding bootstrap blocked: no registered residential building asset"
            );
            return;
        };
        let Some(commercial_asset_id) = self.preferred_founding_asset_id(FOUNDING_COMMERCIAL_ZONE)
        else {
            debug_log!(
                "economy",
                "founding bootstrap blocked: no registered commercial building asset"
            );
            return;
        };

        let Some(residential_slot) =
            self.resolve_first_valid_slot_for_asset(&residential_asset_id, zoning, graph)
        else {
            debug_log!(
                "economy",
                "founding bootstrap waiting: no valid residential zoned frontage for '{}'",
                residential_asset_id
            );
            return;
        };
        let Some(commercial_slot) =
            self.resolve_first_valid_slot_for_asset(&commercial_asset_id, zoning, graph)
        else {
            debug_log!(
                "economy",
                "founding bootstrap waiting: no valid commercial zoned frontage for '{}'",
                commercial_asset_id
            );
            return;
        };

        let residential_idx = self.commit_resolved_slot(residential_slot, zoning);
        let commercial_idx = self.commit_resolved_slot(commercial_slot, zoning);
        self.founding_bootstrap_consumed = true;

        debug_log!(
            "economy",
            "founding bootstrap placed residential_idx={} commercial_idx={} and is now consumed",
            residential_idx,
            commercial_idx
        );
    }

    fn preferred_founding_asset_id(&self, zone_class: ZoneClass) -> Option<String> {
        self.registry
            .buildings_for_zone(zone_class)
            .iter()
            .find(|qualified_id| {
                self.registry
                    .get(qualified_id.as_str())
                    .and_then(|entry| entry.manifest.building.as_ref())
                    .map(|building| building.level == 1)
                    .unwrap_or(false)
            })
            .cloned()
            .or_else(|| {
                self.registry
                    .buildings_for_zone(zone_class)
                    .first()
                    .cloned()
            })
    }

    fn resolve_first_valid_slot_for_asset(
        &self,
        asset_id: &str,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
    ) -> Option<ResolvedPlacement> {
        let params = self.asset_placement_params(asset_id)?;

        for edge_idx in 0..graph.edge_count() {
            let edge = graph.edge(edge_idx);
            if edge.deleted
                || edge.no_building_spawn
                || edge.physical_length < 0.1
                || edge.physical_geometry.len() < 2
            {
                continue;
            }

            let zone_cell_m = zoning.config.zone_cell_m;
            let cells_long = (edge.physical_length / zone_cell_m).floor() as usize;
            if cells_long == 0 || params.width_cells > cells_long {
                continue;
            }
            let max_leading = cells_long.saturating_sub(params.width_cells);

            for side in [1_i8, -1_i8] {
                for cell_x in 0..=max_leading {
                    if let Some(resolved) =
                        self.resolve_slot(asset_id, &params, edge_idx, side, cell_x, zoning, graph)
                    {
                        return Some(resolved);
                    }
                }
            }
        }

        None
    }

    fn asset_placement_params(&self, asset_id: &str) -> Option<AssetPlacementParams> {
        let entry = self.registry.get(asset_id)?;
        let building = entry.manifest.building.as_ref()?;
        Some(AssetPlacementParams {
            zone_type: zone_class_to_zone_type(building.zone_type),
            width_cells: building.lot_width_cells as usize,
            depth_cells: building.lot_depth_cells as usize,
            initial_level: building.level,
        })
    }

    fn resolve_slot(
        &self,
        asset_id: &str,
        params: &AssetPlacementParams,
        edge_idx: usize,
        side: i8,
        cell_x: usize,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
    ) -> Option<ResolvedPlacement> {
        let edge = graph.edge(edge_idx);
        let edge_len = edge.physical_length;
        let edge_width = edge.width;
        let zone_cell_m = zoning.config.zone_cell_m;
        let cells_long = (edge_len / zone_cell_m).floor() as usize;
        if cells_long == 0 || cell_x + params.width_cells > cells_long {
            return None;
        }

        if let Some(occ) = self.edge_occupancy.get(&edge_idx) {
            let slot = if side > 0 { &occ.left } else { &occ.right };
            if cell_x < slot.len() && slot[cell_x] {
                return None;
            }
        }

        let curb_dist = edge_width * 0.5 + crate::config::SIDEWALK_WIDTH;
        let t_col = (cell_x as f32 + 0.5) * zone_cell_m / edge_len;
        let frontage_pos = self.get_pos_on_edge(graph, edge_idx, t_col);
        let frontage_tangent = self.get_tangent_on_edge(graph, edge_idx, t_col);
        let frontage_normal = Vector2::new(frontage_tangent.y, -frontage_tangent.x) * side as f32;
        let frontage_center = frontage_pos + frontage_normal * (curb_dist + zone_cell_m * 0.5);
        if zoning.get_zone_world(frontage_center.x, frontage_center.y) != params.zone_type {
            return None;
        }

        let t_center = (cell_x as f32 + params.width_cells as f32 * 0.5) * zone_cell_m / edge_len;
        let world_pos_on_edge = Self::sample_pos_on_edge(graph, edge_idx, t_center);
        let tangent_c = Self::sample_tangent_on_edge(graph, edge_idx, t_center);
        let normal_c = Vector2::new(tangent_c.y, -tangent_c.x) * side as f32;
        let depth_offset =
            crate::config::SIDEWALK_WIDTH + (params.depth_cells as f32 * 0.5) * zone_cell_m;
        let center_2d = world_pos_on_edge + normal_c * (edge_width * 0.5 + depth_offset);

        for dx in 0..params.width_cells {
            let t_dx = (cell_x as f32 + dx as f32 + 0.5) * zone_cell_m / edge_len;
            let wp = Self::sample_pos_on_edge(graph, edge_idx, t_dx);
            let td = Self::sample_tangent_on_edge(graph, edge_idx, t_dx);
            let nd = Vector2::new(td.y, -td.x) * side as f32;
            for dy in 0..params.depth_cells {
                let cell_center = wp + nd * (curb_dist + (dy as f32 + 0.5) * zone_cell_m);
                if zoning.get_zone_world(cell_center.x, cell_center.y) != params.zone_type {
                    return None;
                }
            }
        }

        let width_m = params.width_cells as f32 * zone_cell_m;
        let depth_m = params.depth_cells as f32 * zone_cell_m;
        if zoning.is_rect_occupied(center_2d.x, center_2d.y, tangent_c, width_m, depth_m) {
            return None;
        }

        let half_depth = depth_m * 0.5;
        let road_dist = zoning.distance_to_road_world(center_2d.x, center_2d.y) as f32;
        if road_dist < half_depth {
            return None;
        }

        Some(ResolvedPlacement {
            asset_id: asset_id.to_owned(),
            zone_type: params.zone_type,
            initial_level: params.initial_level,
            edge_idx,
            side,
            cell_x,
            cells_long,
            width_cells: params.width_cells,
            depth_cells: params.depth_cells,
            center_2d,
            facing_dir: normal_c,
            frontage_t: t_center,
            edge_width,
        })
    }

    fn commit_resolved_slot(
        &mut self,
        placement: ResolvedPlacement,
        zoning: &mut ZoningSystem,
    ) -> usize {
        let zone_cell_m = zoning.config.zone_cell_m;
        let tangent = Vector2::new(-placement.facing_dir.y, placement.facing_dir.x);
        let width_m = placement.width_cells as f32 * zone_cell_m;
        let depth_m = placement.depth_cells as f32 * zone_cell_m;
        zoning.mark_occupied_rect(
            placement.center_2d.x,
            placement.center_2d.y,
            tangent,
            width_m,
            depth_m,
            true,
        );

        let occ = self
            .edge_occupancy
            .entry(placement.edge_idx)
            .or_insert_with(|| EdgeOccupancy {
                cells_long: placement.cells_long,
                left: vec![false; placement.cells_long],
                right: vec![false; placement.cells_long],
            });
        let required_cells = placement.cell_x + placement.width_cells;
        if occ.cells_long < required_cells {
            occ.left.resize(required_cells, false);
            occ.right.resize(required_cells, false);
            occ.cells_long = required_cells;
        }
        let slot = if placement.side > 0 {
            &mut occ.left
        } else {
            &mut occ.right
        };
        if placement.cell_x < slot.len() {
            slot[placement.cell_x] = true;
        }

        let building_idx = self.place_building_instance(placement);
        self.dirty = true;
        self.dirty_index = true;
        self.dirty_zones[self.buildings[building_idx].zone_type as usize] = true;
        debug_log!(
            "economy",
            "founding bootstrap placed building idx={} asset_id={} zone={:?} edge={} cell=({}, {}) center=({:.1}, {:.1})",
            building_idx,
            self.buildings[building_idx].asset_id,
            self.buildings[building_idx].zone_type,
            self.buildings[building_idx].edge_idx,
            self.buildings[building_idx].cell_x,
            self.buildings[building_idx].cell_y,
            self.buildings[building_idx].center_x,
            self.buildings[building_idx].center_y
        );
        building_idx
    }

    fn place_building_instance(&mut self, placement: ResolvedPlacement) -> usize {
        self.buildings.push(Building {
            zone_type: placement.zone_type,
            facing_dir: placement.facing_dir,
            frontage_t: placement.frontage_t,
            side_offset: placement.edge_width * 0.5 + crate::config::SIDEWALK_WIDTH,
            center_x: placement.center_2d.x,
            center_y: placement.center_2d.y,
            edge_idx: placement.edge_idx,
            side: placement.side,
            cell_x: placement.cell_x,
            cell_y: 0,
            width_cells: placement.width_cells as u16,
            depth_cells: placement.depth_cells as u16,
            occupancy: 0,
            worker_count: 0,
            asset_id: placement.asset_id,
            level: placement.initial_level,
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

fn has_connected_border_node(graph: &RegionGraph) -> bool {
    graph.nodes().iter().enumerate().any(|(i, node)| {
        node.node_type == NodeType::Border
            && graph
                .node_adjacency(i as u32)
                .iter()
                .any(|&edge_idx| !graph.edge(edge_idx).deleted)
    })
}

struct AssetPlacementParams {
    zone_type: ZoneType,
    width_cells: usize,
    depth_cells: usize,
    initial_level: u8,
}

struct ResolvedPlacement {
    asset_id: String,
    zone_type: ZoneType,
    initial_level: u8,
    edge_idx: usize,
    side: i8,
    cell_x: usize,
    cells_long: usize,
    width_cells: usize,
    depth_cells: usize,
    center_2d: Vector2,
    facing_dir: Vector2,
    frontage_t: f32,
    edge_width: f32,
}
