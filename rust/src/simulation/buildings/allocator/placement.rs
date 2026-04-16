//! Demand-driven building placement candidate discovery and frontage-slot resolution.

use crate::assets::ZoneClass;
use crate::debug_log;
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, EdgeOccupancy, baseline_private_zone_slot,
    resolve_building_economy_profile_binding, zone_class_to_zone_type, zone_type_to_zone_class,
};
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
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
        let mut dbg_edges_no_spawn = 0_u32;
        let mut dbg_edges_active = 0_u32;
        let mut dbg_cells_no_profile = 0_u32;
        let mut dbg_cells_wrong_zone = 0_u32;
        let mut dbg_cells_no_asset = 0_u32;

        for edge_idx in 0..graph.edge_count() {
            let edge = graph.edge(edge_idx);
            if edge.deleted
                || edge.no_building_spawn
                || edge.physical_length < 0.1
                || edge.physical_geometry.len() < 2
            {
                if !edge.deleted && edge.no_building_spawn {
                    dbg_edges_no_spawn += 1;
                }
                continue;
            }
            dbg_edges_active += 1;

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
                        dbg_cells_no_profile += 1;
                        continue;
                    };
                    let Some(profile) = zoning.profiles.profile_by_runtime_id(profile_runtime_id)
                    else {
                        dbg_cells_no_profile += 1;
                        continue;
                    };
                    if profile.zone_type != zone_type {
                        dbg_cells_wrong_zone += 1;
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
                        dbg_cells_no_asset += 1;
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

        debug_log!(
            "spawn",
            "collect_candidates zone={:?}: active_edges={} no_spawn_flag={} \
             cells_no_profile={} cells_wrong_zone={} cells_no_asset={} candidates={}",
            zone_type,
            dbg_edges_active,
            dbg_edges_no_spawn,
            dbg_cells_no_profile,
            dbg_cells_wrong_zone,
            dbg_cells_no_asset,
            candidates.len(),
        );
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
        let zone_density_assets = self.registry.buildings_for_zone_density(zone_class, density);
        if zone_density_assets.is_empty() {
            debug_log!(
                "spawn",
                "select_spawn_asset: registry has 0 assets for zone={:?} density={}",
                zone_class,
                density,
            );
            return None;
        }
        let mut families: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for qualified_id in zone_density_assets
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
        let zone_type = zone_class_to_zone_type(building.zone_type?);
        let economy_binding = resolve_building_economy_profile_binding(&self.registry, asset_id);
        if matches!(zone_type, ZoneType::Commercial | ZoneType::Industrial)
            && (economy_binding.economy_broken || economy_binding.runtime_id == 0)
        {
            debug_log!(
                "spawn",
                "asset_params: {} rejected — economy_broken={} runtime_id={}",
                asset_id,
                economy_binding.economy_broken,
                economy_binding.runtime_id,
            );
            return None;
        }
        Some(AssetPlacementParams {
            zone_type,
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
            zone_profile_runtime_id: frontage_profile_runtime_id,
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
        if let Some(zone_idx) = baseline_private_zone_slot(self.buildings[building_idx].zone_type) {
            self.dirty_zones[zone_idx] = true;
        }
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
        let economy_binding =
            resolve_building_economy_profile_binding(&self.registry, &placement.asset_id);
        let catalog = load_runtime_economy_catalog();
        let resource_count = catalog.as_ref().map(|c| c.resource_count()).unwrap_or(0);

        // Seed starting inventory for output ports when the profile specifies it.
        // This lets stores open with stock already on shelves before the first freight
        // delivery arrives, which is critical during the startup phase.
        let mut resource_inventory = vec![0.0f32; resource_count];
        if let Ok(catalog) = catalog.as_ref() {
            if let Some(profile) = catalog.profile_by_runtime_id(economy_binding.runtime_id) {
                if profile.starting_inventory_days > 0.0 {
                    for output in &profile.outputs {
                        let cap = profile.output_buffer_capacity_units_for(output);
                        let seed =
                            (output.units_per_day * profile.starting_inventory_days).min(cap);
                        // resource_inventory is 0-indexed; runtime_id is 1-based.
                        let slot = output.resource_runtime_id as usize;
                        if slot > 0 && slot <= resource_count {
                            resource_inventory[slot - 1] = seed;
                        }
                    }
                }
            }
        }

        // Seed startup operating capital so commercial and industrial buildings can pay
        // workers for STARTUP_RUNWAY_DAYS before first revenue arrives, AND cover the cost
        // of the first OWA input import (which fires immediately when local supply is absent).
        // Computed from profile data so it scales with the building.
        const STARTUP_RUNWAY_DAYS: f32 = 7.0;
        const STARTUP_MIN_BUDGET: f32 = 500.0;
        let startup_budget = match placement.zone_type {
            ZoneType::Commercial | ZoneType::Industrial => {
                let worker_cap = self.registry.worker_capacity(&placement.asset_id);
                let catalog_ref = catalog.as_ref().ok();
                let profile = catalog_ref
                    .and_then(|c| c.profile_by_runtime_id(economy_binding.runtime_id));
                let daily_wage = profile.map(|p| p.average_daily_wage()).unwrap_or(0.0);
                let wage_runway = worker_cap as f32 * daily_wage * STARTUP_RUNWAY_DAYS;

                // Add expected cost of the first full OWA input import so the building can
                // absorb it without going into distress on its opening day.
                let owa_import_multiplier = load_runtime_economy_tuning()
                    .map(|t| t.owa_import_price_multiplier.max(1.0))
                    .unwrap_or(1.5);
                let first_import_cost = profile
                    .map(|p| {
                        p.inputs.iter().map(|port| {
                            let unit_price = catalog_ref
                                .and_then(|c| {
                                    c.unit_price_for_resource(port.resource_runtime_id)
                                })
                                .unwrap_or(p.unit_price_currency);
                            p.inventory_target_units_for(port)
                                * unit_price
                                * owa_import_multiplier
                        }).sum::<f32>()
                    })
                    .unwrap_or(0.0);

                (wage_runway + first_import_cost).max(STARTUP_MIN_BUDGET)
            }
            _ => 0.0,
        };

        self.buildings.push(Building {
            zone_profile_runtime_id: placement.zone_profile_runtime_id,
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
            economy_profile_runtime_id: economy_binding.runtime_id,
            economy_broken: economy_binding.economy_broken,
            resource_inventory,
            revenue: 0.0,
            operating_budget: startup_budget,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
            is_deserted: false,
            budget_distress: false,
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
    zone_profile_runtime_id: u16,
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
