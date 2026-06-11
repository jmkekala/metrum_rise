//! Demand-driven building placement candidate discovery and frontage-slot resolution.

use crate::debug_log;
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, baseline_private_zone_slot,
    resolve_building_economy_profile_binding_with_catalog, zone_class_to_zone_type,
    zone_type_to_zone_class,
};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::demand::{
    DemandSpawnAction, DemandSpawnCandidate, DemandSpawnCandidatesByUse,
};
use crate::simulation::economy::fiscal::tax_amount;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::{ZoneType, ZoningParcel, ZoningSystem};
use godot::prelude::Vector2;
use rayon::prelude::*;
use std::collections::BTreeMap;

impl BuildingAllocator {
    pub(crate) fn collect_demand_spawn_candidates_by_use(
        &self,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
    ) -> DemandSpawnCandidatesByUse {
        let asset_candidates_by_profile =
            self.collect_spawn_asset_candidates_by_profile(zoning, catalog);
        let candidates = zoning
            .parcels()
            .par_iter()
            .fold(
                DemandSpawnCandidateSortBuckets::default,
                |mut candidates, parcel| {
                    let edge_idx = parcel.edge_idx();
                    if edge_idx >= graph.edge_count() {
                        return candidates;
                    }
                    let edge = graph.edge(edge_idx);
                    if edge.deleted
                        || edge.no_building_spawn
                        || edge.physical_length < 0.1
                        || edge.physical_geometry.len() < 2
                        || !parcel.is_available()
                    {
                        return candidates;
                    }

                    let profile_runtime_id = parcel.zone_profile_runtime_id();
                    let Some(profile) = zoning.profiles.profile_by_runtime_id(profile_runtime_id)
                    else {
                        return candidates;
                    };
                    let zone_type = profile.zone_type;

                    let Some(profile_candidates) =
                        asset_candidates_by_profile.get(&profile_runtime_id)
                    else {
                        return candidates;
                    };

                    let Some(resolved) = self.select_deterministic_fresh_spawn_asset(
                        profile_candidates,
                        profile_runtime_id,
                        parcel,
                        zoning,
                        graph,
                    ) else {
                        return candidates;
                    };

                    let sort_key = DemandSpawnCandidateSortKey::from_resolved(&resolved);
                    candidates.push_zone_type(
                        zone_type,
                        sort_key,
                        DemandSpawnCandidate {
                            action: DemandSpawnAction {
                                parcel_id: parcel.id().raw(),
                                asset_id: resolved.asset_id,
                            },
                            density: profile.density.as_str().to_owned(),
                        },
                    );
                    candidates
                },
            )
            .reduce(
                DemandSpawnCandidateSortBuckets::default,
                |mut left, right| {
                    left.extend(right);
                    left
                },
            );

        let candidates = candidates.finish();
        debug_log!(
            "spawn",
            "collect_candidates_by_use: residential={} commercial={} industrial={}",
            candidates.residential.len(),
            candidates.commercial.len(),
            candidates.industrial.len(),
        );
        candidates
    }

    fn collect_spawn_asset_candidates_by_profile(
        &self,
        zoning: &ZoningSystem,
        catalog: &RuntimeEconomyCatalog,
    ) -> BTreeMap<u16, SpawnProfileAssetCandidates> {
        let mut by_profile = BTreeMap::new();
        for profile in zoning.profiles.profiles() {
            let Some(zone_class) = zone_type_to_zone_class(profile.zone_type) else {
                continue;
            };
            let mut candidates = Vec::new();
            for qualified_id in self
                .registry
                .buildings_for_zone_density(zone_class, profile.density.as_str())
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
                let Some(params) = self.asset_placement_params(qualified_id, catalog) else {
                    continue;
                };
                if !zoning.profiles.asset_is_legal(
                    profile.runtime_id,
                    params.zone_type,
                    &params.density,
                    &params.tags,
                ) {
                    continue;
                }
                let family_key = entry
                    .manifest
                    .asset_set
                    .clone()
                    .unwrap_or_else(|| qualified_id.clone());
                candidates.push(SpawnAssetCandidate {
                    family_key,
                    qualified_id: qualified_id.clone(),
                    params,
                });
            }
            candidates.sort_by(|left, right| {
                left.family_key
                    .cmp(&right.family_key)
                    .then(left.qualified_id.cmp(&right.qualified_id))
            });
            by_profile.insert(
                profile.runtime_id,
                SpawnProfileAssetCandidates { candidates },
            );
        }
        by_profile
    }

    fn select_deterministic_fresh_spawn_asset(
        &self,
        profile_candidates: &SpawnProfileAssetCandidates,
        profile_runtime_id: u16,
        parcel: &ZoningParcel,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
    ) -> Option<ResolvedPlacement> {
        if profile_candidates.candidates.is_empty() {
            return None;
        }

        let mut best = None;
        for candidate in &profile_candidates.candidates {
            if let Some(resolved) = self.resolve_slot(
                &candidate.qualified_id,
                &candidate.params,
                parcel,
                zoning,
                graph,
            ) {
                let selection_key = (
                    stable_strip_family_hash(
                        profile_runtime_id,
                        parcel.id().raw(),
                        &candidate.family_key,
                    ),
                    candidate.family_key.as_str(),
                    stable_site_variant_hash(
                        profile_runtime_id,
                        parcel.id().raw(),
                        &candidate.qualified_id,
                    ),
                    candidate.qualified_id.as_str(),
                );
                if best
                    .as_ref()
                    .map(|(best_key, _)| selection_key < *best_key)
                    .unwrap_or(true)
                {
                    best = Some((selection_key, resolved));
                }
            }
        }

        best.map(|(_, resolved)| resolved)
    }

    fn asset_placement_params(
        &self,
        asset_id: &str,
        catalog: &RuntimeEconomyCatalog,
    ) -> Option<AssetPlacementParams> {
        let entry = self.registry.get(asset_id)?;
        let building = entry.manifest.building.as_ref()?;
        if !building.is_zoned_private() {
            return None;
        }
        let zone_type = zone_class_to_zone_type(building.zone_type?);
        let economy_binding = resolve_building_economy_profile_binding_with_catalog(
            &self.registry,
            catalog,
            asset_id,
        );
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
        parcel: &ZoningParcel,
        zoning: &ZoningSystem,
        graph: &RegionGraph,
    ) -> Option<ResolvedPlacement> {
        let edge_idx = parcel.edge_idx();
        let edge = graph.edge(edge_idx);
        let edge_width = edge.width;
        let zone_cell_m = zoning.config.zone_cell_m;
        if parcel.occupied_building().is_some() || edge.deleted || edge.no_building_spawn {
            return None;
        }

        let frontage_profile_runtime_id = parcel.zone_profile_runtime_id();
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

        let width_m = params.width_cells as f32 * zone_cell_m;
        let depth_m = params.depth_cells as f32 * zone_cell_m;
        if width_m > parcel.frontage_m() + f32::EPSILON || depth_m > parcel.depth_m() + f32::EPSILON
        {
            return None;
        }

        let center_2d = parcel.front_center() + parcel.normal() * (depth_m * 0.5);

        Some(ResolvedPlacement {
            asset_id: asset_id.to_owned(),
            zone_profile_runtime_id: frontage_profile_runtime_id,
            zone_type: params.zone_type,
            initial_level: params.initial_level,
            parcel_id: parcel.id().raw(),
            edge_idx,
            side: parcel.side(),
            cell_x: 0,
            width_cells: params.width_cells,
            depth_cells: params.depth_cells,
            center_2d,
            facing_dir: parcel.normal() * -1.0,
            frontage_t: parcel.frontage_center_t(),
            edge_width,
        })
    }

    fn commit_resolved_slot(
        &mut self,
        placement: ResolvedPlacement,
        zoning: &mut ZoningSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> usize {
        let parcel_id = placement.parcel_id;
        let building_idx = self.place_building_instance(placement, catalog, tuning);
        zoning.occupy_parcel(parcel_id, building_idx);
        self.bump_building_ref_revision();
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
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> Option<usize> {
        let Some(params) = self.asset_placement_params(&action.asset_id, catalog) else {
            return None;
        };
        let Some(parcel) = zoning.parcel_by_raw_id(action.parcel_id) else {
            return None;
        };
        let Some(resolved) = self.resolve_slot(&action.asset_id, &params, parcel, zoning, graph)
        else {
            return None;
        };
        Some(self.commit_resolved_slot(resolved, zoning, catalog, tuning))
    }

    fn place_building_instance(
        &mut self,
        placement: ResolvedPlacement,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> usize {
        let economy_binding = resolve_building_economy_profile_binding_with_catalog(
            &self.registry,
            catalog,
            &placement.asset_id,
        );
        let resource_count = catalog.resource_count();

        // Seed starting inventory for output ports when the profile specifies it.
        // This lets stores open with stock already on shelves before the first freight
        // delivery arrives, which is critical during the startup phase.
        let mut resource_inventory = vec![0.0f32; resource_count];
        if let Some(profile) = catalog.profile_by_runtime_id(economy_binding.runtime_id)
            && profile.starting_inventory_days > 0.0
        {
            for output in &profile.outputs {
                let cap = profile.output_buffer_capacity_units_for(output);
                let seed = (output.units_per_day * profile.starting_inventory_days).min(cap);
                // resource_inventory is 0-indexed; runtime_id is 1-based.
                let slot = output.resource_runtime_id as usize;
                if slot > 0 && slot <= resource_count {
                    resource_inventory[slot - 1] = seed;
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
                if economy_binding.economy_broken {
                    0.0
                } else {
                    let profile = catalog
                        .profile_by_runtime_id(economy_binding.runtime_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "asset '{}' references missing runtime economy profile id {}",
                                placement.asset_id, economy_binding.runtime_id
                            )
                        });
                    let daily_wage = profile.average_daily_wage();
                    let wage_runway =
                        profile.worker_capacity as f32 * daily_wage * STARTUP_RUNWAY_DAYS;

                    // Add expected cost of the first full OWA input import so the building can
                    // absorb it without going into distress on its opening day.
                    let owa_import_multiplier = tuning.owa_import_price_multiplier;
                    let first_import_base_cost = profile
                        .inputs
                        .iter()
                        .map(|port| {
                            let unit_price = catalog
                                .unit_price_for_resource(port.resource_runtime_id)
                                .unwrap_or_else(|| {
                                    let resource_id = catalog
                                        .resource_id_for_runtime_id(port.resource_runtime_id)
                                        .unwrap_or("<unknown>");
                                    panic!(
                                        "resource '{resource_id}' used by profile '{}' has no catalog price",
                                        profile.id
                                    )
                                });
                            profile.inventory_target_units_for(port)
                                * unit_price
                                * owa_import_multiplier
                        })
                        .sum::<f32>();
                    let first_import_cost = first_import_base_cost
                        + tax_amount(
                            first_import_base_cost,
                            tuning.fiscal.business_purchase_tax_rate,
                        );

                    (wage_runway + first_import_cost).max(STARTUP_MIN_BUDGET)
                }
            }
            _ => 0.0,
        };

        let construction_duration_hours =
            construction_duration_hours(placement.zone_type, placement.initial_level, tuning);

        self.buildings.push(Building {
            zone_profile_runtime_id: placement.zone_profile_runtime_id,
            parcel_id: placement.parcel_id,
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
            construction_total_hours: construction_duration_hours,
            construction_remaining_hours: construction_duration_hours,
            broken: false,
            economy_profile_runtime_id: economy_binding.runtime_id,
            economy_broken: economy_binding.economy_broken,
            resource_inventory,
            revenue: 0.0,
            operating_budget: startup_budget,
            profit_tax_budget_baseline: startup_budget,
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

fn construction_duration_hours(
    zone_type: ZoneType,
    level: u8,
    tuning: &RuntimeEconomyTuning,
) -> u16 {
    let levels = match zone_type {
        ZoneType::Residential => &tuning.construction.residential_hours_by_level,
        ZoneType::Commercial => &tuning.construction.commercial_hours_by_level,
        ZoneType::Industrial => &tuning.construction.industrial_hours_by_level,
        ZoneType::None | ZoneType::Office | ZoneType::Mixed => return 0,
    };
    let idx = usize::from(level.saturating_sub(1)).min(levels.len().saturating_sub(1));
    levels.get(idx).copied().unwrap_or(0)
}

fn stable_strip_family_hash(profile_runtime_id: u16, parcel_id: u64, family_key: &str) -> u64 {
    let mut hasher = StableHasher::new();
    hasher.write_u16(profile_runtime_id);
    hasher.write_u64(parcel_id);
    hasher.write_str(family_key);
    hasher.finish()
}

fn stable_site_variant_hash(
    profile_runtime_id: u16,
    parcel_id: u64,
    qualified_asset_id: &str,
) -> u64 {
    let mut hasher = StableHasher::new();
    hasher.write_u16(profile_runtime_id);
    hasher.write_u64(parcel_id);
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

    fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
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

struct SpawnAssetCandidate {
    family_key: String,
    qualified_id: String,
    params: AssetPlacementParams,
}

struct SpawnProfileAssetCandidates {
    candidates: Vec<SpawnAssetCandidate>,
}

struct ResolvedPlacement {
    asset_id: String,
    zone_profile_runtime_id: u16,
    zone_type: ZoneType,
    initial_level: u8,
    parcel_id: u64,
    edge_idx: usize,
    side: i8,
    cell_x: usize,
    width_cells: usize,
    depth_cells: usize,
    center_2d: Vector2,
    facing_dir: Vector2,
    frontage_t: f32,
    edge_width: f32,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct DemandSpawnCandidateSortKey {
    edge_idx: usize,
    side_order: u8,
    cell_x: usize,
    width_cells: usize,
    depth_cells: usize,
    zone_profile_runtime_id: u16,
    parcel_id: u64,
}

impl DemandSpawnCandidateSortKey {
    fn from_resolved(placement: &ResolvedPlacement) -> Self {
        Self {
            edge_idx: placement.edge_idx,
            side_order: spawn_side_order(placement.side),
            cell_x: placement.cell_x,
            width_cells: placement.width_cells,
            depth_cells: placement.depth_cells,
            zone_profile_runtime_id: placement.zone_profile_runtime_id,
            parcel_id: placement.parcel_id,
        }
    }
}

#[derive(Default)]
struct DemandSpawnCandidateSortBuckets {
    residential: Vec<(DemandSpawnCandidateSortKey, DemandSpawnCandidate)>,
    commercial: Vec<(DemandSpawnCandidateSortKey, DemandSpawnCandidate)>,
    industrial: Vec<(DemandSpawnCandidateSortKey, DemandSpawnCandidate)>,
}

impl DemandSpawnCandidateSortBuckets {
    fn push_zone_type(
        &mut self,
        zone_type: ZoneType,
        sort_key: DemandSpawnCandidateSortKey,
        candidate: DemandSpawnCandidate,
    ) {
        match zone_type {
            ZoneType::Residential => self.residential.push((sort_key, candidate)),
            ZoneType::Commercial => self.commercial.push((sort_key, candidate)),
            ZoneType::Industrial => self.industrial.push((sort_key, candidate)),
            _ => {}
        }
    }

    fn extend(&mut self, other: Self) {
        self.residential.extend(other.residential);
        self.commercial.extend(other.commercial);
        self.industrial.extend(other.industrial);
    }

    fn finish(mut self) -> DemandSpawnCandidatesByUse {
        sort_demand_spawn_candidates(&mut self.residential);
        sort_demand_spawn_candidates(&mut self.commercial);
        sort_demand_spawn_candidates(&mut self.industrial);

        let mut candidates = DemandSpawnCandidatesByUse::default();
        for (_, candidate) in self.residential {
            candidates.push_zone_type(ZoneType::Residential, candidate);
        }
        for (_, candidate) in self.commercial {
            candidates.push_zone_type(ZoneType::Commercial, candidate);
        }
        for (_, candidate) in self.industrial {
            candidates.push_zone_type(ZoneType::Industrial, candidate);
        }
        candidates
    }
}

fn sort_demand_spawn_candidates(
    candidates: &mut [(DemandSpawnCandidateSortKey, DemandSpawnCandidate)],
) {
    candidates.sort_unstable_by(|left, right| left.0.cmp(&right.0));
}

fn spawn_side_order(side: i8) -> u8 {
    if side > 0 { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_candidate(parcel_id: u64) -> DemandSpawnCandidate {
        DemandSpawnCandidate {
            action: DemandSpawnAction {
                parcel_id,
                asset_id: "test:asset".to_owned(),
            },
            density: "low".to_owned(),
        }
    }

    #[test]
    fn demand_spawn_candidates_sort_by_build_site_order() {
        let mut candidates = vec![
            (
                DemandSpawnCandidateSortKey {
                    edge_idx: 3,
                    side_order: 0,
                    cell_x: 0,
                    width_cells: 2,
                    depth_cells: 2,
                    zone_profile_runtime_id: 1,
                    parcel_id: 10,
                },
                spawn_candidate(10),
            ),
            (
                DemandSpawnCandidateSortKey {
                    edge_idx: 2,
                    side_order: 1,
                    cell_x: 0,
                    width_cells: 2,
                    depth_cells: 2,
                    zone_profile_runtime_id: 1,
                    parcel_id: 20,
                },
                spawn_candidate(20),
            ),
            (
                DemandSpawnCandidateSortKey {
                    edge_idx: 2,
                    side_order: 0,
                    cell_x: 1,
                    width_cells: 2,
                    depth_cells: 2,
                    zone_profile_runtime_id: 1,
                    parcel_id: 30,
                },
                spawn_candidate(30),
            ),
            (
                DemandSpawnCandidateSortKey {
                    edge_idx: 2,
                    side_order: 0,
                    cell_x: 0,
                    width_cells: 2,
                    depth_cells: 2,
                    zone_profile_runtime_id: 1,
                    parcel_id: 40,
                },
                spawn_candidate(40),
            ),
        ];

        sort_demand_spawn_candidates(&mut candidates);
        let parcel_ids: Vec<_> = candidates
            .iter()
            .map(|(_, candidate)| candidate.action.parcel_id)
            .collect();

        assert_eq!(parcel_ids, vec![40, 30, 20, 10]);
    }
}
