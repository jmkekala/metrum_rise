// SPDX-License-Identifier: GPL-2.0-only

//! Private building growth, level-change, and despawn planning.

use super::actions::{
    DemandBuildingActionKey, DemandLevelChangeAction, demand_building_action_key,
};
use super::credits::advance_building_action_credit;
use super::diagnostics::BuildingActionDiagnostics;
use super::snapshot::{DailyDemandSnapshot, ResidentialOccupantSnapshot};
use super::spawn_need::{
    OutputAbsorptionContext, candidate_economy_profile, incoming_household_need,
    project_covers_operation, replacement_market_covers_operation,
    residential_missing_household_slots, residential_vacancy_reserve, select_residential_spawns,
};
use super::system::DemandSystem;
use super::supply::ProjectSupply;
use super::types::{DemandUse, EPSILON};
use super::viability::{
    building_is_viable_for_downgrade, building_is_viable_for_upgrade, level_change_is_compatible,
};
use super::workforce::WorkforceReservations;
use crate::simulation::buildings::allocator::{
    BuildingAllocator, BuildingSiteEnvironment, construction_duration_hours,
};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::fiscal::{CityFiscalPolicy, daily_property_tax};
use crate::simulation::economy::hiring::JobFunding;
use crate::simulation::economy::households::{
    applicant_places_by_adults, local_utility_service_costs, output_area_scale_for_building,
};
use crate::simulation::economy::operation::OperationPrices;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use rayon::prelude::*;

#[derive(Clone, Debug)]
pub(super) struct LevelChangeCandidate {
    action: DemandLevelChangeAction,
    building_idx: usize,
}

/// A deserted site that is already empty and safe to clear.
#[derive(Clone, Debug)]
pub(super) struct DespawnCandidate {
    pub(super) action: DemandBuildingActionKey,
    building_idx: usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ExistingBuildingCandidates {
    pub(super) despawns: Vec<DespawnCandidate>,
    pub(super) downgrades: Vec<LevelChangeCandidate>,
    pub(super) upgrades: Vec<LevelChangeCandidate>,
}

enum CollectedExistingBuildingCandidate {
    Despawn(DespawnCandidate),
    Downgrade(LevelChangeCandidate),
    Upgrade(LevelChangeCandidate),
}

enum SpawnRejection {
    Customers,
    OperatingIncome,
    Workforce,
}

fn nonresidential_spawn_rejection(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    absorption: &OutputAbsorptionContext,
    workforce: &WorkforceReservations,
    asset_id: &str,
    completion_hour: u16,
    prices: OperationPrices,
    exports: f32,
    supply: &mut impl ProjectSupply,
) -> Option<SpawnRejection> {
    let Some(profile) = candidate_economy_profile(allocator, catalog, asset_id) else {
        supply.reject();
        return Some(SpawnRejection::Customers);
    };
    if !project_covers_operation(profile, catalog, absorption.market(), prices, exports, supply) {
        let no_customers = supply.supported_workers(absorption.candidate_workers(profile)) == 0;
        supply.reject();
        return Some(if no_customers { SpawnRejection::Customers } else { SpawnRejection::OperatingIncome });
    }
    if !workforce.can_claim(completion_hour, supply.supported_workers(absorption.candidate_workers(profile))) {
        supply.reject();
        return Some(SpawnRejection::Workforce);
    }
    None
}

// A replacement must operate from its own sales and working capital. Downsizing must also
// repair an unsupported current operation, rather than react to a transient cash/stock dip.
// Shared claims change only after all checks pass. Work is O(P² + log H + C log M + S):
// recipe ports P, homes H, owner/source claims C, source bucket size M and route-query work S.
// Receipt release never scans unrelated buildings; quote buffers are reused.
fn reserve_business_level_change(
    candidate: &LevelChangeCandidate,
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    fiscal_policy: &CityFiscalPolicy,
    funded_jobs: &JobFunding,
    utility_prices: [f32; 3],
    supply: &mut impl ProjectSupply,
    absorption: &mut OutputAbsorptionContext,
    workforce: &mut WorkforceReservations,
) -> bool {
    let building = &allocator.buildings[candidate.building_idx];
    let Some(previous) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id) else {
        return false;
    };
    let Some(target) =
        candidate_economy_profile(allocator, catalog, &candidate.action.target_asset_id)
    else {
        return false;
    };
    let target_level = allocator
        .registry
        .get(&candidate.action.target_asset_id)
        .and_then(|entry| entry.manifest.building.as_ref())
        .unwrap()
        .level;
    let scale = output_area_scale_for_building(building, previous);
    let market = absorption.replacement_market(previous, scale);
    let prices = OperationPrices {
        utilities: utility_prices,
        property_tax: daily_property_tax(building.zone_type, target_level, fiscal_policy),
        imports: tuning.owa_import_price_multiplier,
        carrier_quote: f32::INFINITY,
        truck_payload: tuning.logistics.truck_payload_kg,
    };
    let paid_cargo = funded_jobs.paid_cargo_capital(candidate.building_idx);
    if target_level < building.level
        && replacement_market_covers_operation(
            previous,
            catalog,
            market,
            OperationPrices {
                property_tax: daily_property_tax(building.zone_type, building.level, fiscal_policy),
                carrier_quote: supply
                    .begin_replacement(
                        candidate.building_idx,
                        building.parcel_id,
                        &building.asset_id,
                    )
                    .unwrap_or(f32::INFINITY),
                ..prices
            },
            tuning.owa_export_price_multiplier,
            building,
            paid_cargo,
            supply,
        )
    {
        supply.reject();
        return false;
    }
    let reserved_workers = building.worker_count.max(
        funded_jobs
            .capacities
            .get(candidate.building_idx)
            .copied()
            .unwrap_or(0),
    );
    if !replacement_market_covers_operation(
        target,
        catalog,
        market,
        OperationPrices {
            carrier_quote: supply
                .begin_replacement(
                    candidate.building_idx,
                    building.parcel_id,
                    &candidate.action.target_asset_id,
                )
                .unwrap_or(f32::INFINITY),
            ..prices
        },
        tuning.owa_export_price_multiplier,
        building,
        // Changed recipes cannot count old in-flight goods until they settle as usable stock/cash.
        if previous.runtime_id == target.runtime_id {
            paid_cargo
        } else {
            0.0
        },
        supply,
    ) || !workforce.claim_ready(supply.supported_workers(market.candidate_workers(target)).saturating_sub(reserved_workers))
    {
        supply.reject();
        return false;
    }
    supply.accept();
    absorption.reserve_replacement(previous, scale, target);
    true
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
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        snapshot: &DailyDemandSnapshot,
        funded_jobs: &JobFunding,
        catalog: &RuntimeEconomyCatalog,
        economy_tuning: &RuntimeEconomyTuning,
        fiscal_policy: &CityFiscalPolicy,
        residential_occupants: &ResidentialOccupantSnapshot,
        cadence_fraction: f32,
        environment: BuildingSiteEnvironment<'_>,
        mut supply: impl ProjectSupply,
    ) {
        let timing = crate::debug::category_enabled("economy_phases");
        let begin = timing.then(std::time::Instant::now);
        let mut phase = begin;
        let mut times = [0.0; 4];
        let mut lap = |slot: usize| {
            if let Some(previous) = phase {
                times[slot] += previous.elapsed().as_secs_f64() * 1_000.0;
                phase = Some(std::time::Instant::now());
            }
        };
        let utility_prices = local_utility_service_costs(catalog)
            .map(|price| price * economy_tuning.owa_import_price_multiplier);
        let mut commercial_spawn_resource_priorities = snapshot
            .committed_unmet_commercial_consumer_demand_by_resource
            .clone();
        // Pantry replenishment can exceed a shop's daily output temporarily. Select a new
        // category from recurring unreserved sales, the same market used by entry itself.
        // Currency rather than raw units makes ordering invariant to a resource's unit size.
        for (resource, priority) in &mut commercial_spawn_resource_priorities {
            *priority = snapshot.output_absorption.unmet_units(*resource)
                * catalog.unit_price_for_resource(*resource).unwrap_or(0.0);
        }
        commercial_spawn_resource_priorities.retain(|(_, unmet_units)| *unmet_units > EPSILON);
        commercial_spawn_resource_priorities.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        let spawn_assets = allocator.collect_spawn_asset_candidates_by_profile(zoning, catalog);
        lap(0);
        let mut spawn_candidates_by_use = allocator.collect_demand_spawn_candidates_from_assets(
            zoning,
            graph,
            catalog,
            &commercial_spawn_resource_priorities,
            environment,
            &spawn_assets,
        );
        lap(1);
        let mut absorption_context = snapshot.output_absorption.clone();
        self.workforce_reservations.reset(
            allocator,
            catalog,
            economy_tuning,
            snapshot,
            &self.queued_construction,
            self.next_construction_start_hour,
            supply.reservations(),
        );
        lap(2);
        // Existing-candidate collection is interleaved with dependent zone decisions.
        let mut existing_ms = 0.0;
        let mut optional_homes = Vec::new();
        let mut downgrade_batches = Vec::with_capacity(3);
        for use_kind in [
            DemandUse::Residential,
            DemandUse::Commercial,
            DemandUse::Industrial,
        ] {
            let zone_type = use_kind.zone_type();
            let mut spawn_candidates = spawn_candidates_by_use.take_zone_type(zone_type);
            spawn_candidates.retain(|candidate| {
                self.queued_construction
                    .binary_search_by_key(&candidate.action.parcel_id, |project| project.parcel_id)
                    .map_or(true, |index| !self.queued_construction[index].active)
            });
            let existing_begin = timing.then(std::time::Instant::now);
            let existing_candidates = self.collect_existing_building_candidates(
                allocator,
                catalog,
                economy_tuning,
                &residential_occupants,
                zone_type,
            );
            if let Some(begin) = existing_begin { existing_ms += begin.elapsed().as_secs_f64() * 1_000.0; }
            let spawn_candidate_count = spawn_candidates.len();
            let upgrade_candidate_count = existing_candidates.upgrades.len();
            let downgrade_candidate_count = existing_candidates.downgrades.len();
            let despawn_candidate_count = existing_candidates.despawns.len();
            // Only households need a capacity target; businesses claim their own shared
            // customers and workforce below. The minute queue paces physical placement.
            let housing_needed_slots = if use_kind == DemandUse::Residential {
                let incoming = incoming_household_need(
                    snapshot,
                    allocator,
                    &self.admission_budget,
                    fiscal_policy,
                    self.admission_candidate_seed,
                    &spawn_candidates,
                );
                self.last_admission_diagnostics.incoming_household_need = incoming as f32;
                let slots = residential_missing_household_slots(snapshot, incoming);
                if self.cheat_max_demands_enabled {
                    slots.max(1)
                } else {
                    slots
                }
            } else {
                0
            };
            let mut spawn_rejected_absorption = 0_usize;
            let mut spawn_rejected_viability = 0_usize;
            let mut spawn_rejected_workforce = 0_usize;
            let mut spawn_not_needed = 0_usize;
            let selected_spawns: Vec<_> = if zone_type == ZoneType::Residential {
                let selected = select_residential_spawns(
                    allocator,
                    spawn_candidates,
                    housing_needed_slots,
                    &mut optional_homes,
                );
                spawn_not_needed = spawn_candidate_count.saturating_sub(selected.len());
                for action in &selected {
                    self.workforce_reservations.add_home(
                        allocator,
                        economy_tuning,
                        action,
                        false,
                    );
                }
                for action in &optional_homes {
                    self.workforce_reservations.add_home(
                        allocator,
                        economy_tuning,
                        action,
                        true,
                    );
                }
                self.workforce_reservations.reserve_existing();
                selected
            } else {
                // Non-residential openings need customers and a possible source of labor.
                // Cheat mode bypasses these ordinary viability gates explicitly.
                let mut selected = Vec::new();
                // Fresh zoned assets are level one; preparation excludes higher levels.
                let completion_hour = construction_duration_hours(zone_type, 1, economy_tuning);
                let prices = OperationPrices {
                    utilities: utility_prices,
                    property_tax: daily_property_tax(zone_type, 1, fiscal_policy),
                    imports: economy_tuning.owa_import_price_multiplier,
                    carrier_quote: f32::INFINITY, // Replaced by each candidate door's quote below.
                    truck_payload: economy_tuning.logistics.truck_payload_kg,
                };
                for mut candidate in spawn_candidates {
                    if !self.cheat_max_demands_enabled {
                        let parcel = candidate.action.parcel_id;
                        let workforce = &self.workforce_reservations;
                        let mut rejection = |asset: &str| {
                            let mut probe = |crew_cap: Option<u32>| {
                                let carrier_quote = supply.begin(parcel, asset);
                                if let Some(workers) = crew_cap {
                                    supply.set_crew_cap(workers);
                                }
                                nonresidential_spawn_rejection(
                                    allocator,
                                    catalog,
                                    &absorption_context,
                                    workforce,
                                    asset,
                                    completion_hour,
                                    OperationPrices {
                                        carrier_quote: carrier_quote.unwrap_or(f32::INFINITY),
                                        ..prices
                                    },
                                    economy_tuning.owa_export_price_multiplier,
                                    &mut supply,
                                )
                            };
                            let full = probe(None);
                            // A store short only of workers may open with the crew the
                            // calendar can house, serving that crew's share (`DEM-02-C`).
                            if zone_type == ZoneType::Commercial
                                && matches!(full, Some(SpawnRejection::Workforce))
                            {
                                let available = workforce.available(completion_hour);
                                if available > 0 {
                                    return probe(Some(available));
                                }
                            }
                            full
                        };
                        if let Some(reason) = rejection(&candidate.action.asset_id) {
                            let alternative = allocator.select_demand_spawn_alternative(
                                &candidate.action,
                                &spawn_assets,
                                zoning,
                                graph,
                                catalog,
                                &commercial_spawn_resource_priorities,
                                environment,
                                |asset| rejection(asset).is_none(),
                            );
                            let Some(alternative) = alternative else {
                                supply.reject();
                                match reason {
                                    SpawnRejection::Customers => spawn_rejected_absorption += 1,
                                    SpawnRejection::OperatingIncome => {
                                        spawn_rejected_viability += 1
                                    }
                                    SpawnRejection::Workforce => spawn_rejected_workforce += 1,
                                }
                                continue;
                            };
                            candidate.action = alternative;
                            // Selection can inspect another asset after finding its winner.
                            // Only the chosen asset may retain staged supplier claims.
                            if rejection(&candidate.action.asset_id).is_some() {
                                spawn_rejected_viability += 1;
                                continue;
                            }
                        }
                    }
                    if let Some(profile) =
                        candidate_economy_profile(allocator, catalog, &candidate.action.asset_id)
                    {
                        if !self.cheat_max_demands_enabled
                            && !self.workforce_reservations.claim(
                                completion_hour,
                                supply.supported_workers(absorption_context.candidate_workers(profile)),
                            )
                        {
                            spawn_rejected_workforce += 1;
                            supply.reject();
                            continue;
                        }
                        let crew_cap = supply.crew_cap();
                        if crew_cap < profile.worker_capacity {
                            absorption_context.reserve_scaled_candidate(
                                profile,
                                crew_cap as f32 / profile.worker_capacity as f32,
                            );
                        } else {
                            absorption_context.reserve_candidate(profile);
                        }
                    }
                    supply.accept();
                    selected.push(candidate.action);
                }
                selected
            };
            let spawn_selected = selected_spawns.len();

            // Local viability establishes the opportunity; the configured fraction only paces it.
            let upgrade_budget_units = upgrade_candidate_count as f32
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
            let mut selected_upgrades = Vec::with_capacity(upgrades_today);
            for candidate in &existing_candidates.upgrades {
                if selected_upgrades.len() == upgrades_today {
                    break;
                }
                if zone_type != ZoneType::Residential
                    && !self.cheat_max_demands_enabled
                    && !reserve_business_level_change(
                        candidate,
                        allocator,
                        catalog,
                        economy_tuning,
                        fiscal_policy,
                        funded_jobs,
                        utility_prices,
                        &mut supply,
                        &mut absorption_context,
                        &mut self.workforce_reservations,
                    )
                {
                    continue;
                }
                selected_upgrades.push(candidate.action.clone());
            }
            // Economic rejection must not spend pacing credit or hide later viable proposals.
            *self.upgrade_action_credit.get_mut(use_kind) +=
                (upgrades_today - selected_upgrades.len()) as f32;
            let upgrade_credit_after = self.upgrade_action_credit.get(use_kind);
            let upgrade_selected = selected_upgrades.len();

            let downgrade_budget_units = if self.cheat_max_demands_enabled {
                0.0
            } else {
                downgrade_candidate_count as f32
                    * self
                        .config
                        .action_budget
                        .downgrade_batch_fraction_by_use
                        .get(use_kind)
            };
            let downgrade_credit_before = self.downgrade_action_credit.get(use_kind);
            let downgrades_today = advance_building_action_credit(
                self.downgrade_action_credit.get_mut(use_kind),
                downgrade_budget_units,
                downgrade_candidate_count,
                cadence_fraction,
            );
            // Clearance is paced by eligible failed sites, never a citywide market signal.
            let despawn_budget_units = despawn_candidate_count as f32
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
                spawn_candidates: spawn_candidate_count,
                assessed: true,
                sites: spawn_candidates_by_use.availability[use_kind as usize],
                constructing: snapshot.constructing_by_use[use_kind as usize],
                queued: snapshot.queued_by_use[use_kind as usize],
                housing_needed_slots,
                spawn_planned: spawn_selected,
                spawn_selected,
                spawn_rejected_absorption,
                spawn_rejected_viability,
                spawn_rejected_workforce,
                spawn_not_needed,
                upgrade_candidates: upgrade_candidate_count,
                upgrade_budget_units,
                upgrade_credit_before,
                upgrade_credit_after,
                upgrade_planned: upgrades_today,
                upgrade_selected,
                downgrade_candidates: downgrade_candidate_count,
                downgrade_budget_units,
                downgrade_credit_before,
                downgrade_credit_after: self.downgrade_action_credit.get(use_kind),
                downgrade_planned: downgrades_today,
                downgrade_selected: 0,
                despawn_candidates: despawn_candidate_count,
                despawn_budget_units,
                despawn_credit_before,
                despawn_credit_after,
                despawn_planned: despawns_today,
                despawn_selected,
            };

            let plan = self.building_actions.use_plan_mut(use_kind);
            plan.spawns.extend(selected_spawns);
            plan.upgrades.extend(selected_upgrades);
            plan.despawns.extend(selected_despawns);
            downgrade_batches.push((use_kind, existing_candidates.downgrades, downgrades_today));
        }
        // Growth claims first: a smaller home cannot erase housing promised to a new employer.
        let arrivals_possible = snapshot.connected_border_count > 0;
        let protected_homes = residential_vacancy_reserve(snapshot.total_household_count)
            .saturating_add(snapshot.unhoused_household_count)
            .max(u32::from(
                arrivals_possible && snapshot.total_household_count == 0,
            ));
        let mut retirable_homes = snapshot
            .vacant_household_slots
            .saturating_sub(protected_homes);
        // Business replacements also claim ready homes before residential capacity can shrink.
        downgrade_batches.rotate_left(1);
        for (use_kind, candidates, planned) in downgrade_batches {
            if use_kind == DemandUse::Residential && arrivals_possible {
                retirable_homes = self
                    .workforce_reservations
                    .unreserved_ready_homes()
                    .saturating_sub(protected_homes);
            }
            let mut selected = Vec::with_capacity(planned);
            for candidate in candidates {
                if selected.len() == planned {
                    break;
                }
                // A family may offer both directions; a selected upgrade wins this pass.
                if self
                    .building_actions
                    .use_plan_mut(use_kind)
                    .upgrades
                    .binary_search_by(|action| {
                        compare_level_change_actions(action, &candidate.action)
                    })
                    .is_ok()
                {
                    continue;
                }
                if use_kind == DemandUse::Residential {
                    let lost = allocator
                        .household_capacity(candidate.building_idx)
                        .saturating_sub(
                            allocator
                                .registry
                                .household_capacity(&candidate.action.target_asset_id),
                        );
                    // Free places above the smaller capacity, by their applicants' adults.
                    let capacity = allocator.household_capacity(candidate.building_idx);
                    let retired = applicant_places_by_adults(
                        allocator.flat_size_m2(candidate.building_idx),
                        candidate.building_idx,
                        (capacity - lost)
                            .max(allocator.buildings[candidate.building_idx].occupancy)
                            ..capacity,
                    );
                    if lost > retirable_homes
                        || (arrivals_possible
                            && !self.workforce_reservations.retire_ready_homes(retired))
                    {
                        continue;
                    }
                    retirable_homes -= lost;
                } else if !reserve_business_level_change(
                    &candidate,
                    allocator,
                    catalog,
                    economy_tuning,
                    fiscal_policy,
                    funded_jobs,
                    utility_prices,
                    &mut supply,
                    &mut absorption_context,
                    &mut self.workforce_reservations,
                ) {
                    continue;
                }
                selected.push(candidate.action);
            }
            *self.downgrade_action_credit.get_mut(use_kind) += (planned - selected.len()) as f32;
            let diagnostics = self.last_building_action_diagnostics.use_mut(use_kind);
            diagnostics.downgrade_selected = selected.len();
            diagnostics.downgrade_credit_after = self.downgrade_action_credit.get(use_kind);
            self.building_actions
                .use_plan_mut(use_kind)
                .downgrades
                .extend(selected);
        }
        // Only materialize optional housing actually claimed by incumbent or newly selected
        // workplaces. These are the same legal candidates already collected for this pass.
        self.workforce_reservations.select_optional_homes();
        for (index, action) in optional_homes.into_iter().enumerate() {
            if self.workforce_reservations.needs_home(index) {
                self.building_actions.residential.spawns.push(action);
                let diagnostics = &mut self.last_building_action_diagnostics.residential;
                diagnostics.spawn_selected += 1;
                diagnostics.spawn_planned += 1;
                diagnostics.spawn_not_needed = diagnostics.spawn_not_needed.saturating_sub(1);
            }
        }
        supply.reject();
        lap(3);
        crate::debug_log!("economy_phases", "demand_planning_detail assets_ms={:.3} spawn_sites_ms={:.3} workforce_ms={:.3} existing_ms={:.3} decisions_ms={:.3}",
            times[0], times[1], times[2], existing_ms, times[3] - existing_ms);
    }

    pub(super) fn collect_existing_building_candidates(
        &self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        economy_tuning: &RuntimeEconomyTuning,
        residential_occupants: &ResidentialOccupantSnapshot,
        zone_type: ZoneType,
    ) -> ExistingBuildingCandidates {
        let mut candidates = allocator
            .buildings
            .par_iter()
            .enumerate()
            .flat_map_iter(|(building_idx, building)| {
                let mut result = [None, None];
                if building.zone_type != zone_type
                    || building.broken
                    || building.economy_broken
                    || building.pending_redevelopment
                    || building.is_under_construction()
                {
                    return result;
                }
                let Some(asset_building) = allocator
                    .registry
                    .get(&building.asset_id)
                    .and_then(|entry| entry.manifest.building.as_ref())
                else {
                    return result;
                };
                if !asset_building.is_zoned_private() {
                    return result;
                }
                if building.is_deserted {
                    if building.occupancy == 0 && building.worker_count == 0 {
                        result[0] = Some(CollectedExistingBuildingCandidate::Despawn(
                            DespawnCandidate {
                                action: demand_building_action_key(building),
                                building_idx,
                            },
                        ));
                    }
                    return result;
                }
                if let Some(target_asset_id) = allocator.downgrade_target(building)
                    && level_change_is_compatible(allocator, catalog, building_idx, target_asset_id)
                    && building_is_viable_for_downgrade(
                        allocator,
                        economy_tuning,
                        residential_occupants,
                        building_idx,
                    )
                {
                    result[0] = Some(CollectedExistingBuildingCandidate::Downgrade(
                        LevelChangeCandidate {
                            action: DemandLevelChangeAction {
                                building: demand_building_action_key(building),
                                target_asset_id: target_asset_id.to_owned(),
                            },
                            building_idx,
                        },
                    ));
                }
                if let Some(target_asset_id) = allocator.upgrade_target(building)
                    && level_change_is_compatible(allocator, catalog, building_idx, target_asset_id)
                    && building_is_viable_for_upgrade(
                        allocator,
                        catalog,
                        economy_tuning,
                        residential_occupants,
                        building_idx,
                        target_asset_id,
                    )
                {
                    result[1] = Some(CollectedExistingBuildingCandidate::Upgrade(
                        LevelChangeCandidate {
                            action: DemandLevelChangeAction {
                                building: demand_building_action_key(building),
                                target_asset_id: target_asset_id.to_owned(),
                            },
                            building_idx,
                        },
                    ));
                }
                result
            })
            .filter_map(|candidate| candidate)
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
            action_attachment_sort_key(&left.action)
                .cmp(&action_attachment_sort_key(&right.action))
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
    left: &LevelChangeCandidate,
    right: &LevelChangeCandidate,
) -> std::cmp::Ordering {
    compare_level_change_actions(&left.action, &right.action)
        .then(left.building_idx.cmp(&right.building_idx))
}

fn compare_level_change_actions(
    left: &DemandLevelChangeAction,
    right: &DemandLevelChangeAction,
) -> std::cmp::Ordering {
    action_attachment_sort_key(&left.building)
        .cmp(&action_attachment_sort_key(&right.building))
        .then(left.building.parcel_id.cmp(&right.building.parcel_id))
}
