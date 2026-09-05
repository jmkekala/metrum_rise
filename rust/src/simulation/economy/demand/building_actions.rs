// SPDX-License-Identifier: GPL-2.0-only

//! Private building growth, level-change, and despawn planning.

use super::actions::{
    DemandBuildingActionKey, DemandLevelChangeAction, demand_building_action_key,
};
use super::credits::{
    advance_building_action_credit, advance_spawn_need_credit, normalized_negative_pressure,
    normalized_positive_pressure,
};
use super::diagnostics::BuildingActionDiagnostics;
use super::snapshot::{DailyDemandSnapshot, ResidentialOccupantSnapshot};
use super::spawn_need::{nonresidential_passes_absorption_gate, spawn_need_buildings_for_use};
use super::system::DemandSystem;
use super::types::{DemandUse, EPSILON};
use super::viability::{
    building_is_viable_for_downgrade, building_is_viable_for_upgrade, level_change_is_compatible,
};
use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use rayon::prelude::*;

#[derive(Clone, Debug)]
pub(super) struct WeightedLevelChangeCandidate {
    action: DemandLevelChangeAction,
    normalized_action_pressure: f32,
    building_idx: usize,
}

#[derive(Clone, Debug)]
pub(super) struct WeightedDespawnCandidate {
    pub(super) action: DemandBuildingActionKey,
    pub(super) normalized_action_pressure: f32,
    pub(super) deserted: bool,
    building_idx: usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ExistingBuildingCandidates {
    pub(super) despawns: Vec<WeightedDespawnCandidate>,
    pub(super) downgrades: Vec<WeightedLevelChangeCandidate>,
    pub(super) upgrades: Vec<WeightedLevelChangeCandidate>,
}

enum CollectedExistingBuildingCandidate {
    Despawn(WeightedDespawnCandidate),
    Downgrade(WeightedLevelChangeCandidate),
    Upgrade(WeightedLevelChangeCandidate),
}

impl ExistingBuildingCandidates {
    fn push(&mut self, candidate: CollectedExistingBuildingCandidate) {
        match candidate {
            CollectedExistingBuildingCandidate::Despawn(candidate) => {
                self.despawns.push(candidate);
            }
            CollectedExistingBuildingCandidate::Downgrade(candidate) => {
                self.downgrades.push(candidate);
            }
            CollectedExistingBuildingCandidate::Upgrade(candidate) => {
                self.upgrades.push(candidate);
            }
        }
    }

    fn extend(&mut self, other: Self) {
        self.despawns.extend(other.despawns);
        self.downgrades.extend(other.downgrades);
        self.upgrades.extend(other.upgrades);
    }

    fn sort_by_attachment_order(&mut self) {
        self.downgrades
            .sort_unstable_by(compare_level_change_candidates);
        self.upgrades
            .sort_unstable_by(compare_level_change_candidates);
    }
}

impl DemandSystem {
    pub(super) fn plan_private_building_actions(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        snapshot: &DailyDemandSnapshot,
        catalog: &RuntimeEconomyCatalog,
        economy_tuning: &RuntimeEconomyTuning,
        residential_occupants: &ResidentialOccupantSnapshot,
        cadence_fraction: f32,
        log_label: &str,
    ) {
        let mut commercial_spawn_resource_priorities = snapshot
            .committed_unmet_commercial_consumer_demand_by_resource
            .clone();
        commercial_spawn_resource_priorities.retain(|(_, unmet_units)| *unmet_units > EPSILON);
        commercial_spawn_resource_priorities.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        let mut spawn_candidates_by_use = allocator.collect_demand_spawn_candidates_by_use(
            zoning,
            graph,
            catalog,
            &commercial_spawn_resource_priorities,
        );
        let absorption_context = &snapshot.output_absorption;
        for use_kind in [
            DemandUse::Residential,
            DemandUse::Commercial,
            DemandUse::Industrial,
        ] {
            let zone_type = use_kind.zone_type();
            let growth_pressure = self.pressure_for_use(use_kind);
            let spawn_hysteresis_active = self.spawn_hysteresis_active.get(use_kind);
            let spawn_candidates = spawn_candidates_by_use.take_zone_type(zone_type);
            let existing_candidates = self.collect_existing_building_candidates(
                allocator,
                households,
                catalog,
                economy_tuning,
                &residential_occupants,
                zone_type,
                growth_pressure,
            );
            let spawn_candidate_count = spawn_candidates.len();
            let upgrade_candidate_count = existing_candidates.upgrades.len();
            let downgrade_candidate_count = existing_candidates.downgrades.len();
            let despawn_candidate_count = existing_candidates.despawns.len();

            let mut spawn_profile_missing = 0_usize;
            let normalized_spawn_pressure = spawn_candidates
                .iter()
                .map(|candidate| {
                    if let Some(profile) = self
                        .config
                        .profile_for_zone_density(zone_type, &candidate.density)
                    {
                        normalized_positive_profile_pressure(
                            growth_pressure,
                            profile.spawn_threshold,
                            profile.hysteresis_margin,
                            spawn_hysteresis_active,
                        )
                    } else {
                        spawn_profile_missing += 1;
                        0.0
                    }
                })
                .sum::<f32>();
            let normalized_spawn_pressure = if spawn_candidate_count == 0 {
                0.0
            } else {
                normalized_spawn_pressure / spawn_candidate_count as f32
            };
            self.spawn_hysteresis_active.set(
                use_kind,
                spawn_candidate_count > 0 && normalized_spawn_pressure > EPSILON,
            );
            let raw_spawn_need_buildings = spawn_need_buildings_for_use(
                use_kind,
                allocator,
                catalog,
                snapshot,
                &spawn_candidates,
            );
            let raw_spawn_need_buildings =
                if self.cheat_max_demands_enabled && spawn_candidate_count > 0 {
                    raw_spawn_need_buildings.max(1.0)
                } else {
                    raw_spawn_need_buildings
                };
            let spawn_need_buildings = raw_spawn_need_buildings * normalized_spawn_pressure;
            let spawn_credit_before = self.spawn_action_credit.get(use_kind);
            let spawns_today = advance_spawn_need_credit(
                self.spawn_action_credit.get_mut(use_kind),
                spawn_need_buildings,
                spawn_candidate_count,
            );
            let spawn_credit_after = self.spawn_action_credit.get(use_kind);
            debug_log!(
                "spawn",
                "{} zone={:?}: pressure={:.3} \
                 candidates={} profile_missing={} norm_pressure={:.3} raw_need={:.3} \
                 spawn_need={:.3} credit={:.3}->{:.3} spawns_today={}",
                log_label,
                zone_type,
                growth_pressure,
                spawn_candidate_count,
                spawn_profile_missing,
                normalized_spawn_pressure,
                raw_spawn_need_buildings,
                spawn_need_buildings,
                spawn_credit_before,
                spawn_credit_after,
                spawns_today,
            );
            let spawn_rejected_labour = 0_usize;
            let mut spawn_rejected_absorption = 0_usize;
            let mut spawn_skipped_budget = 0_usize;
            let selected_spawns: Vec<_> = if zone_type == ZoneType::Residential {
                spawn_skipped_budget = spawn_candidate_count.saturating_sub(spawns_today);
                spawn_candidates
                    .into_iter()
                    .take(spawns_today)
                    .map(|candidate| candidate.action)
                    .collect()
            } else if spawns_today == 0 {
                spawn_skipped_budget = spawn_candidate_count;
                Vec::new()
            } else {
                // Non-residential: output absorption is the final ordinary hard gate. Cheat mode
                // bypasses it explicitly, while staffing remains an operational outcome after spawn.
                let mut passed = 0;
                let mut selected = Vec::new();
                for candidate in spawn_candidates {
                    if passed >= spawns_today {
                        spawn_skipped_budget += 1;
                        continue;
                    }
                    if !self.cheat_max_demands_enabled
                        && !nonresidential_passes_absorption_gate(
                            allocator,
                            catalog,
                            absorption_context,
                            &candidate.action.asset_id,
                        )
                    {
                        spawn_rejected_absorption += 1;
                        continue;
                    }
                    passed += 1;
                    selected.push(candidate.action);
                }
                selected
            };
            let spawn_selected = selected_spawns.len();

            let normalized_upgrade_pressure = existing_candidates
                .upgrades
                .iter()
                .map(|candidate| candidate.normalized_action_pressure)
                .sum::<f32>();
            self.upgrade_hysteresis_active.set(
                use_kind,
                upgrade_candidate_count > 0 && normalized_upgrade_pressure > EPSILON,
            );
            let upgrade_budget_units = normalized_upgrade_pressure
                * self
                    .config
                    .action_budget
                    .upgrade_batch_fraction_by_use
                    .get(use_kind);
            let upgrade_credit_before = self.upgrade_action_credit.get(use_kind);
            let upgrades_today = advance_building_action_credit(
                self.upgrade_action_credit.get_mut(use_kind),
                upgrade_budget_units,
                upgrade_candidate_count,
                cadence_fraction,
            );
            let upgrade_credit_after = self.upgrade_action_credit.get(use_kind);
            let selected_upgrades: Vec<_> = existing_candidates
                .upgrades
                .iter()
                .take(upgrades_today)
                .map(|candidate| candidate.action.clone())
                .collect();
            let upgrade_selected = selected_upgrades.len();

            let normalized_downgrade_pressure = existing_candidates
                .downgrades
                .iter()
                .map(|candidate| candidate.normalized_action_pressure)
                .sum::<f32>();
            self.downgrade_hysteresis_active.set(
                use_kind,
                downgrade_candidate_count > 0 && normalized_downgrade_pressure > EPSILON,
            );
            let downgrade_budget_units = normalized_downgrade_pressure
                * self
                    .config
                    .action_budget
                    .downgrade_batch_fraction_by_use
                    .get(use_kind);
            let downgrade_credit_before = self.downgrade_action_credit.get(use_kind);
            let downgrades_today = advance_building_action_credit(
                self.downgrade_action_credit.get_mut(use_kind),
                downgrade_budget_units,
                downgrade_candidate_count,
                cadence_fraction,
            );
            let downgrade_credit_after = self.downgrade_action_credit.get(use_kind);
            let selected_downgrades: Vec<_> = existing_candidates
                .downgrades
                .iter()
                .take(downgrades_today)
                .map(|candidate| candidate.action.clone())
                .collect();
            let downgrade_selected = selected_downgrades.len();

            let normalized_despawn_pressure = existing_candidates
                .despawns
                .iter()
                .map(|candidate| candidate.normalized_action_pressure)
                .sum::<f32>();
            self.despawn_hysteresis_active.set(
                use_kind,
                despawn_candidate_count > 0 && normalized_despawn_pressure > EPSILON,
            );
            let despawn_budget_units = normalized_despawn_pressure
                * self
                    .config
                    .action_budget
                    .despawn_batch_fraction_by_use
                    .get(use_kind);
            let despawn_credit_before = self.despawn_action_credit.get(use_kind);
            let despawns_today = advance_building_action_credit(
                self.despawn_action_credit.get_mut(use_kind),
                despawn_budget_units,
                despawn_candidate_count,
                cadence_fraction,
            );
            let despawn_credit_after = self.despawn_action_credit.get(use_kind);
            let selected_despawns: Vec<_> = existing_candidates
                .despawns
                .iter()
                .take(despawns_today)
                .map(|candidate| candidate.action.clone())
                .collect();
            let despawn_selected = selected_despawns.len();

            *self.last_building_action_diagnostics.use_mut(use_kind) = BuildingActionDiagnostics {
                pressure: growth_pressure,
                spawn_candidates: spawn_candidate_count,
                spawn_profile_missing,
                spawn_normalized_pressure: normalized_spawn_pressure,
                spawn_need_buildings,
                spawn_credit_before,
                spawn_credit_after,
                spawn_planned: spawns_today,
                spawn_selected,
                spawn_rejected_labour,
                spawn_rejected_absorption,
                spawn_skipped_budget,
                upgrade_candidates: upgrade_candidate_count,
                upgrade_normalized_pressure: normalized_upgrade_pressure,
                upgrade_budget_units,
                upgrade_credit_before,
                upgrade_credit_after,
                upgrade_planned: upgrades_today,
                upgrade_selected,
                downgrade_candidates: downgrade_candidate_count,
                downgrade_normalized_pressure: normalized_downgrade_pressure,
                downgrade_budget_units,
                downgrade_credit_before,
                downgrade_credit_after,
                downgrade_planned: downgrades_today,
                downgrade_selected,
                despawn_candidates: despawn_candidate_count,
                despawn_normalized_pressure: normalized_despawn_pressure,
                despawn_budget_units,
                despawn_credit_before,
                despawn_credit_after,
                despawn_planned: despawns_today,
                despawn_selected,
            };

            let plan = self.building_actions.use_plan_mut(use_kind);
            plan.spawns.extend(selected_spawns);
            plan.upgrades.extend(selected_upgrades);
            plan.downgrades.extend(selected_downgrades);
            plan.despawns.extend(selected_despawns);
        }
    }

    pub(super) fn collect_existing_building_candidates(
        &self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        catalog: &RuntimeEconomyCatalog,
        economy_tuning: &RuntimeEconomyTuning,
        residential_occupants: &ResidentialOccupantSnapshot,
        zone_type: ZoneType,
        growth_pressure: f32,
    ) -> ExistingBuildingCandidates {
        let mut candidates = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter_map(|(building_idx, building)| {
                if building.zone_type != zone_type {
                    return None;
                }
                if building.broken
                    || building.economy_broken
                    || building.pending_redevelopment
                    || building.is_under_construction()
                {
                    return None;
                }
                let Some(entry) = allocator.registry.get(&building.asset_id) else {
                    return None;
                };
                let Some(asset_building) = entry.manifest.building.as_ref() else {
                    return None;
                };
                if !asset_building.is_zoned_private() {
                    return None;
                }
                let Some(density) = asset_building.density_key() else {
                    return None;
                };
                let Some(profile) = self.config.profile_for_zone_density(zone_type, density) else {
                    return None;
                };

                let Some(use_kind) = demand_use_for_zone_type(zone_type) else {
                    return None;
                };
                let despawn_pressure = normalized_negative_profile_pressure(
                    growth_pressure,
                    profile.despawn_threshold,
                    profile.hysteresis_margin,
                    self.despawn_hysteresis_active.get(use_kind),
                );
                let downgrade_pressure = normalized_negative_profile_pressure(
                    growth_pressure,
                    profile.downgrade_threshold,
                    profile.hysteresis_margin,
                    self.downgrade_hysteresis_active.get(use_kind),
                );
                let upgrade_pressure = normalized_positive_profile_pressure(
                    growth_pressure,
                    profile.upgrade_threshold,
                    profile.hysteresis_margin,
                    self.upgrade_hysteresis_active.get(use_kind),
                );

                if building.is_deserted {
                    if building.occupancy == 0
                        && building.worker_count == 0
                        && despawn_pressure > 0.0
                    {
                        return Some(CollectedExistingBuildingCandidate::Despawn(
                            WeightedDespawnCandidate {
                                action: demand_building_action_key(building),
                                normalized_action_pressure: despawn_pressure,
                                deserted: true,
                                building_idx,
                            },
                        ));
                    }
                    return None;
                }

                if building.occupancy == 0 && building.worker_count == 0 && despawn_pressure > 0.0 {
                    return Some(CollectedExistingBuildingCandidate::Despawn(
                        WeightedDespawnCandidate {
                            action: demand_building_action_key(building),
                            normalized_action_pressure: despawn_pressure,
                            deserted: false,
                            building_idx,
                        },
                    ));
                }

                if downgrade_pressure > 0.0
                    && let Some(target_asset_id) = allocator.registry.prev_level(&building.asset_id)
                    && level_change_is_compatible(allocator, catalog, building_idx, target_asset_id)
                    && building_is_viable_for_downgrade(
                        allocator,
                        households,
                        catalog,
                        economy_tuning,
                        residential_occupants,
                        building_idx,
                        target_asset_id,
                    )
                {
                    return Some(CollectedExistingBuildingCandidate::Downgrade(
                        WeightedLevelChangeCandidate {
                            action: DemandLevelChangeAction {
                                building: demand_building_action_key(building),
                                target_asset_id: target_asset_id.to_owned(),
                            },
                            normalized_action_pressure: downgrade_pressure,
                            building_idx,
                        },
                    ));
                }

                if upgrade_pressure > 0.0
                    && let Some(target_asset_id) = allocator.registry.next_level(&building.asset_id)
                    && level_change_is_compatible(allocator, catalog, building_idx, target_asset_id)
                    && building_is_viable_for_upgrade(
                        allocator,
                        households,
                        catalog,
                        economy_tuning,
                        residential_occupants,
                        building_idx,
                        target_asset_id,
                    )
                {
                    return Some(CollectedExistingBuildingCandidate::Upgrade(
                        WeightedLevelChangeCandidate {
                            action: DemandLevelChangeAction {
                                building: demand_building_action_key(building),
                                target_asset_id: target_asset_id.to_owned(),
                            },
                            normalized_action_pressure: upgrade_pressure,
                            building_idx,
                        },
                    ));
                }

                None
            })
            .fold(
                ExistingBuildingCandidates::default,
                |mut candidates, candidate| {
                    candidates.push(candidate);
                    candidates
                },
            )
            .reduce(ExistingBuildingCandidates::default, |mut left, right| {
                left.extend(right);
                left
            });

        candidates.sort_by_attachment_order();
        candidates.despawns.sort_unstable_by(|left, right| {
            right
                .deserted
                .cmp(&left.deserted)
                .then_with(|| {
                    action_attachment_sort_key(&left.action)
                        .cmp(&action_attachment_sort_key(&right.action))
                })
                .then(left.action.parcel_id.cmp(&right.action.parcel_id))
                .then(left.building_idx.cmp(&right.building_idx))
        });
        candidates
    }
}

fn action_attachment_sort_key(
    action: &DemandBuildingActionKey,
) -> (usize, u8, usize, u16, u16, u8, &str) {
    (
        action.edge_idx,
        if action.side > 0 { 0 } else { 1 },
        action.cell_x,
        action.width_cells,
        action.depth_cells,
        action.level,
        action.asset_id.as_str(),
    )
}

fn compare_level_change_candidates(
    left: &WeightedLevelChangeCandidate,
    right: &WeightedLevelChangeCandidate,
) -> std::cmp::Ordering {
    action_attachment_sort_key(&left.action.building)
        .cmp(&action_attachment_sort_key(&right.action.building))
        .then(
            left.action
                .building
                .parcel_id
                .cmp(&right.action.building.parcel_id),
        )
        .then(left.building_idx.cmp(&right.building_idx))
}

fn normalized_positive_profile_pressure(
    pressure: f32,
    threshold: f32,
    hysteresis_margin: f32,
    hysteresis_active: bool,
) -> f32 {
    let threshold = if hysteresis_active {
        (threshold - hysteresis_margin).max(0.0)
    } else {
        threshold
    };
    normalized_positive_pressure(pressure, threshold)
}

fn normalized_negative_profile_pressure(
    pressure: f32,
    threshold: f32,
    hysteresis_margin: f32,
    hysteresis_active: bool,
) -> f32 {
    let threshold = if hysteresis_active {
        (threshold + hysteresis_margin).min(1.0)
    } else {
        threshold
    };
    normalized_negative_pressure(pressure, threshold)
}

fn demand_use_for_zone_type(zone_type: ZoneType) -> Option<DemandUse> {
    match zone_type {
        ZoneType::Residential => Some(DemandUse::Residential),
        ZoneType::Commercial => Some(DemandUse::Commercial),
        ZoneType::Industrial => Some(DemandUse::Industrial),
        _ => None,
    }
}
