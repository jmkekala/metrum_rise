//! Demand-driven building placement candidate discovery and frontage-slot resolution.

use crate::assets::ZoneClass;
use crate::debug_log;
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, EdgeOccupancy, zone_class_to_zone_type, zone_type_to_zone_class,
};
use crate::simulation::economy::demand::{DemandSpawnAction, DemandSpawnCandidate};
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector2;
use std::collections::BTreeMap;

impl BuildingAllocator {
    pub(crate) fn collect_demand_spawn_candidates(
        &self,
        zone_type: ZoneType,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
    ) -> Vec<DemandSpawnCandidate> {
        let Some(zone_class) = zone_type_to_zone_class(zone_type) else {
            return Vec::new();
        };
        let mut reserved_frontage: BTreeMap<(usize, i8), Vec<bool>> = BTreeMap::new();
        let mut candidates = Vec::new();

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
            if cells_long == 0 {
                continue;
            }

            for side in [1_i8, -1_i8] {
                for cell_x in 0..cells_long {
                    let reserved = reserved_frontage
                        .entry((edge_idx, side))
                        .or_insert_with(|| vec![false; cells_long]);
                    if reserved.get(cell_x).copied().unwrap_or(false) {
                        continue;
                    }

                    let Some(profile_runtime_id) = self.frontage_profile_runtime_id_for_site(
                        edge_idx, side, cell_x, zoning, graph,
                    ) else {
                        continue;
                    };
                    let Some(profile) = zoning.profiles.profile_by_runtime_id(profile_runtime_id)
                    else {
                        continue;
                    };
                    if profile.zone_type != zone_type {
                        continue;
                    }

                    let Some(resolved) = self.select_deterministic_fresh_spawn_asset(
                        zone_class,
                        profile.density.as_str(),
                        profile_runtime_id,
                        edge_idx,
                        side,
                        cell_x,
                        zoning,
                        graph,
                    ) else {
                        continue;
                    };

                    let required_cells = cell_x + resolved.width_cells;
                    if required_cells > reserved.len() {
                        reserved.resize(required_cells, false);
                    }
                    if reserved
                        .iter()
                        .skip(cell_x)
                        .take(resolved.width_cells)
                        .any(|occupied| *occupied)
                    {
                        continue;
                    }
                    for occupied in reserved.iter_mut().skip(cell_x).take(resolved.width_cells) {
                        *occupied = true;
                    }

                    candidates.push(DemandSpawnCandidate {
                        action: DemandSpawnAction {
                            edge_idx,
                            side,
                            cell_x,
                            asset_id: resolved.asset_id,
                        },
                        density: profile.density.as_str().to_owned(),
                    });
                }
            }
        }

        candidates
    }

    fn frontage_profile_runtime_id_for_site(
        &self,
        edge_idx: usize,
        side: i8,
        cell_x: usize,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
    ) -> Option<u16> {
        let edge = graph.edge(edge_idx);
        let edge_len = edge.physical_length;
        let zone_cell_m = zoning.config.zone_cell_m;
        if edge_len < zone_cell_m * 0.5 {
            return None;
        }
        let t_col = (cell_x as f32 + 0.5) * zone_cell_m / edge_len;
        let frontage_pos = self.get_pos_on_edge(graph, edge_idx, t_col);
        let frontage_tangent = self.get_tangent_on_edge(graph, edge_idx, t_col);
        let frontage_normal = Vector2::new(frontage_tangent.y, -frontage_tangent.x) * side as f32;
        let curb_dist = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH;
        let frontage_center = frontage_pos + frontage_normal * (curb_dist + zone_cell_m * 0.5);
        let profile_runtime_id =
            zoning.get_zone_profile_runtime_id_world(frontage_center.x, frontage_center.y);
        if profile_runtime_id == 0 {
            None
        } else {
            Some(profile_runtime_id)
        }
    }

    fn select_deterministic_fresh_spawn_asset(
        &self,
        zone_class: ZoneClass,
        density: &str,
        profile_runtime_id: u16,
        edge_idx: usize,
        side: i8,
        cell_x: usize,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
    ) -> Option<ResolvedPlacement> {
        let mut families: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for qualified_id in self
            .registry
            .buildings_for_zone_density(zone_class, density)
        {
            let Some(entry) = self.registry.get(qualified_id) else {
                continue;
            };
            let Some(building) = entry.manifest.building.as_ref() else {
                continue;
            };
            if !building.is_zoned_private() || building.level != 1 {
                continue;
            }
            let Some(asset_zone_class) = building.zone_type else {
                continue;
            };
            let Some(asset_density) = building.density_key() else {
                continue;
            };
            if !zoning.profiles.asset_is_legal(
                profile_runtime_id,
                zone_class_to_zone_type(asset_zone_class),
                asset_density,
                &entry.manifest.tags,
            ) {
                continue;
            }
            let family_key = entry
                .manifest
                .asset_set
                .clone()
                .unwrap_or_else(|| qualified_id.clone());
            families
                .entry(family_key)
                .or_default()
                .push(qualified_id.clone());
        }

        let mut ordered_families: Vec<(u64, String, Vec<String>)> = families
            .into_iter()
            .map(|(family_key, mut candidate_ids)| {
                candidate_ids.sort();
                (
                    stable_strip_family_hash(profile_runtime_id, edge_idx, side, &family_key),
                    family_key,
                    candidate_ids,
                )
            })
            .collect();
        ordered_families.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

        for (_, _, candidate_ids) in ordered_families {
            let mut resolved_variants: Vec<(u64, String, ResolvedPlacement)> = Vec::new();
            for qualified_id in candidate_ids {
                let Some(params) = self.asset_placement_params(&qualified_id) else {
                    continue;
                };
                if let Some(resolved) = self.resolve_slot(
                    &qualified_id,
                    &params,
                    edge_idx,
                    side,
                    cell_x,
                    zoning,
                    graph,
                ) {
                    resolved_variants.push((
                        stable_site_variant_hash(
                            profile_runtime_id,
                            edge_idx,
                            side,
                            cell_x,
                            &qualified_id,
                        ),
                        qualified_id,
                        resolved,
                    ));
                }
            }
            resolved_variants.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            if let Some((_, _, resolved)) = resolved_variants.into_iter().next() {
                return Some(resolved);
            }
        }

        None
    }

    fn asset_placement_params(&self, asset_id: &str) -> Option<AssetPlacementParams> {
        let entry = self.registry.get(asset_id)?;
        let building = entry.manifest.building.as_ref()?;
        if !building.is_zoned_private() {
            return None;
        }
        Some(AssetPlacementParams {
            zone_type: zone_class_to_zone_type(building.zone_type?),
            density: building.density_key()?.to_owned(),
            tags: entry.manifest.tags.clone(),
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
        let frontage_profile_runtime_id =
            zoning.get_zone_profile_runtime_id_world(frontage_center.x, frontage_center.y);
        if frontage_profile_runtime_id == 0 {
            return None;
        }
        if !zoning.profiles.asset_is_legal(
            frontage_profile_runtime_id,
            params.zone_type,
            &params.density,
            &params.tags,
        ) {
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
                if zoning.get_zone_profile_runtime_id_world(cell_center.x, cell_center.y)
                    != frontage_profile_runtime_id
                {
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
        self.entrances_dirty = true;
        self.dirty_zones[self.buildings[building_idx].zone_type as usize] = true;
        debug_log!(
            "economy",
            "demand placed building idx={} asset_id={} zone={:?} edge={} cell=({}, {}) center=({:.1}, {:.1})",
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

    pub(crate) fn execute_demand_spawn_action(
        &mut self,
        action: &DemandSpawnAction,
        zoning: &mut ZoningSystem,
        graph: &RegionGraph,
    ) -> bool {
        let Some(params) = self.asset_placement_params(&action.asset_id) else {
            return false;
        };
        let Some(resolved) = self.resolve_slot(
            &action.asset_id,
            &params,
            action.edge_idx,
            action.side,
            action.cell_x,
            zoning,
            graph,
        ) else {
            return false;
        };
        self.commit_resolved_slot(resolved, zoning);
        true
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
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
            abandoned_timer: 0,
        });
        self.buildings.len() - 1
    }
}

fn stable_strip_family_hash(
    profile_runtime_id: u16,
    edge_idx: usize,
    side: i8,
    family_key: &str,
) -> u64 {
    let mut hasher = StableHasher::new();
    hasher.write_u16(profile_runtime_id);
    hasher.write_usize(edge_idx);
    hasher.write_i8(side);
    hasher.write_str(family_key);
    hasher.finish()
}

fn stable_site_variant_hash(
    profile_runtime_id: u16,
    edge_idx: usize,
    side: i8,
    cell_x: usize,
    qualified_asset_id: &str,
) -> u64 {
    let mut hasher = StableHasher::new();
    hasher.write_u16(profile_runtime_id);
    hasher.write_usize(edge_idx);
    hasher.write_i8(side);
    hasher.write_usize(cell_x);
    hasher.write_str(qualified_asset_id);
    hasher.finish()
}

struct StableHasher {
    state: u64,
}

impl StableHasher {
    fn new() -> Self {
        Self {
            state: 0xcbf29ce484222325,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= byte as u64;
            self.state = self.state.wrapping_mul(0x100000001b3);
        }
        self.state ^= 0xff;
        self.state = self.state.wrapping_mul(0x100000001b3);
    }

    fn write_u16(&mut self, value: u16) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.write_bytes(&(value as u64).to_le_bytes());
    }

    fn write_i8(&mut self, value: i8) {
        self.write_bytes(&[value as u8]);
    }

    fn write_str(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }

    fn finish(self) -> u64 {
        self.state
    }
}

struct AssetPlacementParams {
    zone_type: ZoneType,
    density: String,
    tags: Vec<String>,
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
