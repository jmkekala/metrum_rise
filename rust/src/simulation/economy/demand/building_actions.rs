// SPDX-License-Identifier: GPL-2.0-only

//! Private building growth, level-change, and despawn planning.

use super::actions::{
    DemandBuildingActionKey, DemandLevelChangeAction, DemandSpawnAction, DemandSpawnCandidate,
    PlanningSlot, SpawnSiteAvailability, demand_building_action_key,
};
use super::credits::advance_building_action_credit;
use super::diagnostics::BuildingActionDiagnostics;
use super::snapshot::{DailyDemandSnapshot, ResidentialOccupantSnapshot};
use super::spawn_need::{
    OutputAbsorptionContext, candidate_economy_profile, incoming_household_need,
    project_covers_operation, replacement_market_covers_operation,
    residential_missing_household_slots, residential_vacancy_reserve, select_residential_spawns,
};
use super::supply::ProjectSupply;
use super::system::DemandSystem;
use super::types::{DemandUse, EPSILON};
use super::viability::{
    building_is_viable_for_downgrade, building_is_viable_for_upgrade, level_change_is_compatible,
};
use super::workforce::WorkforceReservations;
use crate::simulation::buildings::allocator::{
    BuildingAllocator, BuildingSiteEnvironment, SpawnProfileAssetCandidates,
    construction_duration_hours,
};
use crate::simulation::economy::definitions::{
    ResourceRuntimeId, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use crate::simulation::economy::fiscal::{CityFiscalPolicy, daily_property_tax};
use crate::simulation::economy::hiring::JobFunding;
use crate::simulation::economy::households::{
    applicant_places_by_adults, local_utility_service_costs, output_area_scale_for_building,
};
use crate::simulation::economy::operation::OperationPrices;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::{ParcelId, ZoneType, ZoningSystem};
use rayon::prelude::*;
use std::collections::BTreeMap;

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

// Which existing-building actions a planning slot collects (`SIM-02-I`): a zone slot plans its
// zone's upgrades and despawns, the actions slot every zone's downgrades.
#[derive(Clone, Copy)]
struct ExistingActions {
    upgrades: bool,
    downgrades: bool,
    despawns: bool,
}

impl ExistingActions {
    #[cfg(test)]
    const ALL: Self = Self {
        upgrades: true,
        downgrades: true,
        despawns: true,
    };
    const ZONE_SLOT: Self = Self {
        upgrades: true,
        downgrades: false,
        despawns: true,
    };
    const ACTIONS_SLOT: Self = Self {
        upgrades: false,
        downgrades: true,
        despawns: false,
    };
}

/// What a planning slot's scans of the live city found (`SIM-02-K`): a zone slot's legal spawn
/// sites on free lots, their site availability, and its existing buildings' upgrade and despawn
/// candidates; the actions slot's downgrade candidates of every zone, indexed by `DemandUse`.
/// The planning slot's setup slot scans and holds them for its plan
/// (`PLANNING_SETUP_LEAD` slots later); a plan without a setup scans them itself.
#[derive(Debug, Default)]
pub(super) struct PlanningScans {
    spawns: Vec<DemandSpawnCandidate>,
    availability: SpawnSiteAvailability,
    existing: ExistingBuildingCandidates,
    downgrades: [Vec<LevelChangeCandidate>; 3],
}

impl PlanningScans {
    // Appends the saved form of a planning setup's scans (`SIM-02-K`): the spawn sites by parcel
    // and asset, the zone's site availability, and the building candidates by dense index with
    // their targets, each list in its order.
    pub(super) fn write_saved(&self, out: &mut Vec<u8>) {
        use super::setup::{put_len, put_str, put_u64};
        put_len(out, self.spawns.len());
        for spawn in &self.spawns {
            put_u64(out, spawn.action.parcel_id);
            put_str(out, &spawn.action.asset_id);
        }
        let availability = &self.availability;
        put_u64(out, u64::from(availability.has_assets));
        put_len(out, availability.free_lots);
        put_len(out, availability.missing_asset_lots);
        put_len(out, availability.unusable_lots);
        let level_changes = |out: &mut Vec<u8>, candidates: &[LevelChangeCandidate]| {
            put_len(out, candidates.len());
            for candidate in candidates {
                put_len(out, candidate.building_idx);
                put_str(out, &candidate.action.target_asset_id);
            }
        };
        level_changes(out, &self.existing.upgrades);
        level_changes(out, &self.existing.downgrades);
        put_len(out, self.existing.despawns.len());
        for despawn in &self.existing.despawns {
            put_len(out, despawn.building_idx);
        }
        for downgrades in &self.downgrades {
            level_changes(out, downgrades);
        }
    }

    // Reads what [`Self::write_saved`] wrote on the loaded `allocator`: each building
    // candidate's key is its building's, which the setup took between the slots that change
    // no building's key. `None` on a malformed buffer or a building the city does not hold.
    pub(super) fn read_saved(
        reader: &mut super::setup::SavedReader<'_>,
        allocator: &BuildingAllocator,
    ) -> Option<Self> {
        let mut scans = Self::default();
        for _ in 0..reader.count()? {
            scans.spawns.push(DemandSpawnCandidate {
                action: DemandSpawnAction {
                    parcel_id: reader.u64()?,
                    asset_id: reader.string()?,
                },
            });
        }
        scans.availability = SpawnSiteAvailability {
            has_assets: reader.u64()? != 0,
            free_lots: usize::try_from(reader.u64()?).ok()?,
            missing_asset_lots: usize::try_from(reader.u64()?).ok()?,
            unusable_lots: usize::try_from(reader.u64()?).ok()?,
        };
        let key = |building_idx: usize| {
            allocator
                .buildings
                .get(building_idx)
                .map(demand_building_action_key)
        };
        let level_changes = |reader: &mut super::setup::SavedReader<'_>| {
            let count = reader.count()?;
            let mut candidates = Vec::with_capacity(count);
            for _ in 0..count {
                let building_idx = usize::try_from(reader.u64()?).ok()?;
                candidates.push(LevelChangeCandidate {
                    action: DemandLevelChangeAction {
                        building: key(building_idx)?,
                        target_asset_id: reader.string()?,
                    },
                    building_idx,
                });
            }
            Some(candidates)
        };
        scans.existing.upgrades = level_changes(reader)?;
        scans.existing.downgrades = level_changes(reader)?;
        let despawns = reader.count()?;
        for _ in 0..despawns {
            let building_idx = usize::try_from(reader.u64()?).ok()?;
            scans.existing.despawns.push(DespawnCandidate {
                action: key(building_idx)?,
                building_idx,
            });
        }
        for downgrades in &mut scans.downgrades {
            *downgrades = level_changes(reader)?;
        }
        Some(scans)
    }
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

#[allow(clippy::too_many_arguments)]
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
    if !project_covers_operation(
        profile,
        catalog,
        absorption.market(),
        prices,
        exports,
        supply,
    ) {
        let no_customers = supply.supported_workers(absorption.candidate_workers(profile)) == 0;
        supply.reject();
        return Some(if no_customers {
            SpawnRejection::Customers
        } else {
            SpawnRejection::OperatingIncome
        });
    }
    if !workforce.can_claim(
        completion_hour,
        supply.supported_workers(absorption.candidate_workers(profile)),
    ) {
        supply.reject();
        return Some(SpawnRejection::Workforce);
    }
    None
}

// A business spawn's proof on `asset` at `parcel`: the full crew, then, for a store short only
// of workers, the crew the calendar can house, serving that crew's share (`DEM-02-C`). A
// passing proof leaves its claims staged in `supply`.
#[allow(clippy::too_many_arguments)]
fn business_spawn_rejection(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    absorption: &OutputAbsorptionContext,
    workforce: &WorkforceReservations,
    zone_type: ZoneType,
    parcel: u64,
    asset: &str,
    completion_hour: u16,
    prices: OperationPrices,
    exports: f32,
    supply: &mut impl ProjectSupply,
) -> Option<SpawnRejection> {
    let mut probe = |crew_cap: Option<u32>| {
        let carrier_quote = supply.begin(parcel, asset);
        if let Some(workers) = crew_cap {
            supply.set_crew_cap(workers);
        }
        nonresidential_spawn_rejection(
            allocator,
            catalog,
            absorption,
            workforce,
            asset,
            completion_hour,
            OperationPrices {
                carrier_quote: carrier_quote.unwrap_or(f32::INFINITY),
                ..prices
            },
            exports,
            supply,
        )
    };
    let full = probe(None);
    if zone_type == ZoneType::Commercial && matches!(full, Some(SpawnRejection::Workforce)) {
        let available = workforce.available(completion_hour);
        if available > 0 {
            return probe(Some(available));
        }
    }
    full
}

// Commits a proved business spawn on `asset`: its workers by `completion_hour`, its customers
// (a store's crew share of them) and its staged supply. Without the workers it commits nothing
// and returns false.
#[allow(clippy::too_many_arguments)]
fn commit_business_spawn(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    asset: &str,
    completion_hour: u16,
    cheat: bool,
    supply: &mut impl ProjectSupply,
    absorption: &mut OutputAbsorptionContext,
    workforce: &mut WorkforceReservations,
) -> bool {
    if let Some(profile) = candidate_economy_profile(allocator, catalog, asset) {
        if !cheat
            && !workforce.claim(
                completion_hour,
                supply.supported_workers(absorption.candidate_workers(profile)),
            )
        {
            supply.reject();
            return false;
        }
        let crew_cap = supply.crew_cap();
        if crew_cap < profile.worker_capacity {
            absorption.reserve_scaled_candidate(
                profile,
                crew_cap as f32 / profile.worker_capacity as f32,
            );
        } else {
            absorption.reserve_candidate(profile);
        }
    }
    supply.accept();
    true
}

// The prices a fresh level-one business of `zone_type` proves its operation with; each
// candidate door replaces the carrier quote.
fn new_business_prices(
    zone_type: ZoneType,
    utility_prices: [f32; 3],
    tuning: &RuntimeEconomyTuning,
    fiscal_policy: &CityFiscalPolicy,
) -> OperationPrices {
    OperationPrices {
        utilities: utility_prices,
        property_tax: daily_property_tax(zone_type, 1, fiscal_policy),
        imports: tuning.owa_import_price_multiplier,
        carrier_quote: f32::INFINITY,
        truck_payload: tuning.logistics.truck_payload_kg,
    }
}

// A replacement must operate from its own sales and working capital. Downsizing must also
// repair an unsupported current operation, rather than react to a transient cash/stock dip.
// Shared claims change only after all checks pass. Work is O(P² + log H + C log M + S):
// recipe ports P, homes H, owner/source claims C, source bucket size M and route-query work S.
// Receipt release never scans unrelated buildings; quote buffers are reused.
#[allow(clippy::too_many_arguments)]
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
    ) || !workforce.claim_ready(
        supply
            .supported_workers(market.candidate_workers(target))
            .saturating_sub(reserved_workers),
    ) {
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
    // Counts a later slot's failed re-proofs (`SIM-02-I`).
    fn count_reproof_drops(&mut self, dropped: usize) {
        crate::debug::work::add(crate::debug::work::Work::ReproofDrops, dropped as u64);
        #[cfg(test)]
        {
            self.reproof_drops += dropped as u64;
        }
    }

    /// Plans the hour's planning slot `slot` (`SIM-02-I`). A zone slot plans its zone's spawns,
    /// upgrades and despawns into the hour's plan, as the single pass planned each zone in
    /// turn; the actions slot plans the downgrades and the optional homes the workforce claims.
    /// Nothing is queued before the actions slot, so a later slot first offers the residential
    /// slot's homes and optional homes to the calendar, then reserves the earlier slots'
    /// business spawns and upgrades again, zone by zone, as the single pass held their claims.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_private_building_actions(
        &mut self,
        slot: PlanningSlot,
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
        supply: &mut impl ProjectSupply,
        scans: Option<PlanningScans>,
        free_places: Option<super::workforce::FreePlaceScan>,
    ) {
        let timing = crate::debug::Channel::EconomyPhases.enabled();
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
        let commercial_spawn_resource_priorities = commercial_spawn_priorities(snapshot, catalog);
        let zone = slot.zone();
        let spawn_assets = if zone.is_some() {
            allocator.collect_spawn_asset_candidates_by_profile(zoning, catalog)
        } else {
            Default::default()
        };
        lap(0);
        // The setup slot's scans, or this slot's own (`SIM-02-K`).
        let scan_begin = timing.then(std::time::Instant::now);
        let PlanningScans {
            spawns: zone_spawn_candidates,
            availability: zone_availability,
            existing: zone_existing_candidates,
            downgrades: scanned_downgrades,
        } = scans.unwrap_or_else(|| {
            self.scan_with(
                slot,
                allocator,
                graph,
                zoning,
                catalog,
                economy_tuning,
                residential_occupants,
                environment,
                &commercial_spawn_resource_priorities,
                &spawn_assets,
            )
        });
        let scan_ms = scan_begin.map_or(0.0, |begin| begin.elapsed().as_secs_f64() * 1_000.0);
        lap(1);
        let mut absorption_context = snapshot.output_absorption.clone();
        // The buildings' free places, scanned beside the slot's site preparation, or now.
        let free_places = free_places.unwrap_or_else(|| {
            super::workforce::WorkforceReservations::scan_free_places(
                allocator,
                economy_tuning,
                snapshot,
            )
        });
        self.workforce_reservations.reset_scanned(
            free_places,
            allocator,
            catalog,
            economy_tuning,
            snapshot,
            &self.queued_construction,
            self.next_construction_start_hour,
            supply.reservations(),
            |index| funded_jobs.saw_operating(index),
        );
        if slot != PlanningSlot::Residential {
            // The residential slot's homes and optional homes, which this slot's employers may
            // claim, as the single pass offered them (`SIM-02-I`). A home whose lot went since
            // supplies nobody: the queue drops it.
            let available = |action: &DemandSpawnAction| {
                zoning
                    .parcels
                    .get(ParcelId::from_raw(action.parcel_id))
                    .is_some_and(|parcel| parcel.is_available())
            };
            self.optional_homes.retain(available);
            for (action, optional) in self
                .building_actions
                .residential
                .spawns
                .iter()
                .filter(|action| available(action))
                .map(|action| (action, false))
                .chain(self.optional_homes.iter().map(|action| (action, true)))
            {
                self.workforce_reservations
                    .add_home(allocator, economy_tuning, action, optional);
            }
            self.workforce_reservations.reserve_existing();
            // Business spawns, then upgrades, of earlier slots claim their workers, customers
            // and inputs again before this slot's decisions, in the single pass's order. One
            // whose proof now fails would get no permit on these books: it leaves the hour's
            // plan, so its claims are not freed while it stays queued, and a dropped upgrade
            // gets its pacing credit back, as a rejected one does.
            for use_kind in [DemandUse::Commercial, DemandUse::Industrial] {
                if slot.zone().is_some_and(|zone| zone as u8 <= use_kind as u8) {
                    continue;
                }
                let zone_type = use_kind.zone_type();
                let completion_hour = construction_duration_hours(zone_type, 1, economy_tuning);
                let prices =
                    new_business_prices(zone_type, utility_prices, economy_tuning, fiscal_policy);
                let cheat = self.cheat_max_demands_enabled;
                let mut spawns =
                    std::mem::take(&mut self.building_actions.use_plan_mut(use_kind).spawns);
                let planned = spawns.len();
                spawns.retain(|action| {
                    // Cheat mode planned them without proofs.
                    (cheat
                        || business_spawn_rejection(
                            allocator,
                            catalog,
                            &absorption_context,
                            &self.workforce_reservations,
                            zone_type,
                            action.parcel_id,
                            &action.asset_id,
                            completion_hour,
                            prices,
                            economy_tuning.owa_export_price_multiplier,
                            supply,
                        )
                        .is_none())
                        && commit_business_spawn(
                            allocator,
                            catalog,
                            &action.asset_id,
                            completion_hour,
                            cheat,
                            supply,
                            &mut absorption_context,
                            &mut self.workforce_reservations,
                        )
                });
                self.count_reproof_drops(planned - spawns.len());
                self.building_actions.use_plan_mut(use_kind).spawns = spawns;
                if cheat {
                    continue;
                }
                let mut upgrades =
                    std::mem::take(&mut self.building_actions.use_plan_mut(use_kind).upgrades);
                let planned = upgrades.len();
                upgrades.retain(|action| {
                    // A building no longer standing as planned holds no claims, and slot 3
                    // skips it as it does now.
                    let Some(building_idx) =
                        allocator.resolve_demand_building_action(&action.building, zoning)
                    else {
                        return true;
                    };
                    reserve_business_level_change(
                        &LevelChangeCandidate {
                            action: action.clone(),
                            building_idx,
                        },
                        allocator,
                        catalog,
                        economy_tuning,
                        fiscal_policy,
                        funded_jobs,
                        utility_prices,
                        supply,
                        &mut absorption_context,
                        &mut self.workforce_reservations,
                    )
                });
                self.count_reproof_drops(planned - upgrades.len());
                *self.upgrade_action_credit.get_mut(use_kind) += (planned - upgrades.len()) as f32;
                self.building_actions.use_plan_mut(use_kind).upgrades = upgrades;
            }
        }
        lap(2);
        if let Some(use_kind) = zone {
            let mut optional_homes = Vec::new();
            let zone_type = use_kind.zone_type();
            let mut spawn_candidates = zone_spawn_candidates;
            spawn_candidates.retain(|candidate| {
                self.queued_construction
                    .binary_search_by_key(&candidate.action.parcel_id, |project| project.parcel_id)
                    .map_or(true, |index| !self.queued_construction[index].active)
            });
            let existing_candidates = zone_existing_candidates;
            let spawn_candidate_count = spawn_candidates.len();
            let upgrade_candidate_count = existing_candidates.upgrades.len();
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
                    self.workforce_reservations
                        .add_home(allocator, economy_tuning, action, false);
                }
                for action in &optional_homes {
                    self.workforce_reservations
                        .add_home(allocator, economy_tuning, action, true);
                }
                self.workforce_reservations.reserve_existing();
                // Offered to every later slot's employers (`SIM-02-I`).
                self.optional_homes = std::mem::take(&mut optional_homes);
                selected
            } else {
                // Non-residential openings need customers and a possible source of labor.
                // Cheat mode bypasses these ordinary viability gates explicitly.
                let mut selected = Vec::new();
                // Fresh zoned assets are level one; preparation excludes higher levels.
                let completion_hour = construction_duration_hours(zone_type, 1, economy_tuning);
                let prices =
                    new_business_prices(zone_type, utility_prices, economy_tuning, fiscal_policy);
                for mut candidate in spawn_candidates {
                    if !self.cheat_max_demands_enabled {
                        let parcel = candidate.action.parcel_id;
                        let workforce = &self.workforce_reservations;
                        let mut rejection = |asset: &str| {
                            business_spawn_rejection(
                                allocator,
                                catalog,
                                &absorption_context,
                                workforce,
                                zone_type,
                                parcel,
                                asset,
                                completion_hour,
                                prices,
                                economy_tuning.owa_export_price_multiplier,
                                supply,
                            )
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
                    if !commit_business_spawn(
                        allocator,
                        catalog,
                        &candidate.action.asset_id,
                        completion_hour,
                        self.cheat_max_demands_enabled,
                        supply,
                        &mut absorption_context,
                        &mut self.workforce_reservations,
                    ) {
                        spawn_rejected_workforce += 1;
                        continue;
                    }
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
                        supply,
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
                sites: zone_availability,
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
                // The actions slot plans the downgrades and fills these in.
                downgrade_candidates: 0,
                downgrade_budget_units: 0.0,
                downgrade_credit_before: self.downgrade_action_credit.get(use_kind),
                downgrade_credit_after: self.downgrade_action_credit.get(use_kind),
                downgrade_planned: 0,
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
        } else {
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
            let mut downgrades_by_use = scanned_downgrades;
            // Business replacements also claim ready homes before residential capacity can shrink.
            for use_kind in [
                DemandUse::Commercial,
                DemandUse::Industrial,
                DemandUse::Residential,
            ] {
                let candidates = std::mem::take(&mut downgrades_by_use[use_kind as usize]);
                let downgrade_candidate_count = candidates.len();
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
                let planned = advance_building_action_credit(
                    self.downgrade_action_credit.get_mut(use_kind),
                    downgrade_budget_units,
                    downgrade_candidate_count,
                    cadence_fraction,
                );
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
                            allocator.applicant_seed(candidate.building_idx),
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
                        supply,
                        &mut absorption_context,
                        &mut self.workforce_reservations,
                    ) {
                        continue;
                    }
                    selected.push(candidate.action);
                }
                *self.downgrade_action_credit.get_mut(use_kind) +=
                    (planned - selected.len()) as f32;
                let diagnostics = self.last_building_action_diagnostics.use_mut(use_kind);
                diagnostics.downgrade_candidates = downgrade_candidate_count;
                diagnostics.downgrade_budget_units = downgrade_budget_units;
                diagnostics.downgrade_credit_before = downgrade_credit_before;
                diagnostics.downgrade_planned = planned;
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
            for (index, action) in self.optional_homes.iter().enumerate() {
                if self.workforce_reservations.needs_home(index) {
                    self.building_actions
                        .residential
                        .spawns
                        .push(action.clone());
                    let diagnostics = &mut self.last_building_action_diagnostics.residential;
                    diagnostics.spawn_selected += 1;
                    diagnostics.spawn_planned += 1;
                    diagnostics.spawn_not_needed = diagnostics.spawn_not_needed.saturating_sub(1);
                }
            }
        }
        supply.reject();
        lap(3);
        crate::debug_log!(
            "economy_phases",
            "demand_planning_detail assets_ms={:.3} scans_ms={:.3} workforce_ms={:.3} decisions_ms={:.3}",
            times[0],
            scan_ms,
            times[2],
            times[3]
        );
    }

    /// Scans the live city for planning slot `slot` (`SIM-02-K`): a zone slot's legal spawn sites
    /// on free lots, ranked by the commercial priorities `snapshot`'s committed demand sets, and
    /// its existing buildings' upgrade and despawn candidates; the actions slot's downgrade
    /// candidates of every zone. It reads only the city, the catalog and those priorities, so the
    /// setup slot's scan of a city its plan finds unchanged equals the plan's own.
    /// O(P/p + B/p) in parallel, plus the zone's free lots' assets.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn scan_for_planning(
        &self,
        slot: PlanningSlot,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        snapshot: &DailyDemandSnapshot,
        catalog: &RuntimeEconomyCatalog,
        economy_tuning: &RuntimeEconomyTuning,
        residential_occupants: &ResidentialOccupantSnapshot,
        environment: BuildingSiteEnvironment<'_>,
    ) -> PlanningScans {
        let priorities = commercial_spawn_priorities(snapshot, catalog);
        let spawn_assets = if slot.zone().is_some() {
            allocator.collect_spawn_asset_candidates_by_profile(zoning, catalog)
        } else {
            Default::default()
        };
        self.scan_with(
            slot,
            allocator,
            graph,
            zoning,
            catalog,
            economy_tuning,
            residential_occupants,
            environment,
            &priorities,
            &spawn_assets,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_with(
        &self,
        slot: PlanningSlot,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        catalog: &RuntimeEconomyCatalog,
        economy_tuning: &RuntimeEconomyTuning,
        residential_occupants: &ResidentialOccupantSnapshot,
        environment: BuildingSiteEnvironment<'_>,
        priorities: &[(ResourceRuntimeId, f32)],
        spawn_assets: &BTreeMap<u16, SpawnProfileAssetCandidates>,
    ) -> PlanningScans {
        let mut scans = PlanningScans::default();
        match slot.zone() {
            Some(use_kind) => {
                let zone_type = use_kind.zone_type();
                // Independent read-only scans of the lots and the buildings, side by side.
                let (mut by_use, existing) = rayon::join(
                    || {
                        allocator.collect_demand_spawn_candidates_from_assets(
                            zoning,
                            graph,
                            catalog,
                            priorities,
                            environment,
                            spawn_assets,
                            Some(zone_type),
                        )
                    },
                    || {
                        self.collect_existing_actions(
                            allocator,
                            catalog,
                            economy_tuning,
                            residential_occupants,
                            Some(zone_type),
                            ExistingActions::ZONE_SLOT,
                        )
                    },
                );
                scans.spawns = by_use.take_zone_type(zone_type);
                scans.availability = by_use.availability[use_kind as usize];
                scans.existing = existing;
            }
            None => {
                scans.downgrades = self.collect_downgrades_by_use(
                    allocator,
                    catalog,
                    economy_tuning,
                    residential_occupants,
                );
            }
        }
        scans
    }

    /// Test driver of the hour's four planning slots on one `supply` without the slot clock or
    /// execution (`SIM-02-I`): the whole hour's plan stays in the plan for the test to read.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_hour_for_test(
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
        for slot in PlanningSlot::ALL {
            self.plan_private_building_actions(
                slot,
                allocator,
                graph,
                zoning,
                snapshot,
                funded_jobs,
                catalog,
                economy_tuning,
                fiscal_policy,
                residential_occupants,
                cadence_fraction,
                environment,
                &mut supply,
                None,
                None,
            );
        }
    }

    // Every existing-building action of `zone_type`'s buildings, for tests.
    #[cfg(test)]
    pub(super) fn collect_existing_building_candidates(
        &self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        economy_tuning: &RuntimeEconomyTuning,
        residential_occupants: &ResidentialOccupantSnapshot,
        zone_type: ZoneType,
    ) -> ExistingBuildingCandidates {
        self.collect_existing_actions(
            allocator,
            catalog,
            economy_tuning,
            residential_occupants,
            Some(zone_type),
            ExistingActions::ALL,
        )
    }

    // Slot 3's downgrade candidates of every zone from one scan, indexed by `DemandUse`
    // (`SIM-02-K`). A building's candidacy reads only the city the pass holds immutably, and the
    // attachment order is total, so each zone's list equals its own scan's. O(B / p), parallel,
    // once instead of once per zone.
    fn collect_downgrades_by_use(
        &self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        economy_tuning: &RuntimeEconomyTuning,
        residential_occupants: &ResidentialOccupantSnapshot,
    ) -> [Vec<LevelChangeCandidate>; 3] {
        let mut by_use: [Vec<LevelChangeCandidate>; 3] = Default::default();
        for candidate in self
            .collect_existing_actions(
                allocator,
                catalog,
                economy_tuning,
                residential_occupants,
                None,
                ExistingActions::ACTIONS_SLOT,
            )
            .downgrades
        {
            let use_kind = match allocator.buildings[candidate.building_idx].zone_type {
                ZoneType::Residential => DemandUse::Residential,
                ZoneType::Commercial => DemandUse::Commercial,
                ZoneType::Industrial => DemandUse::Industrial,
                _ => continue,
            };
            by_use[use_kind as usize].push(candidate);
        }
        // Test builds: each zone's list equals that zone's own scan, as slot 3 ran it.
        #[cfg(test)]
        if crate::debug::work::city_checks() {
            for use_kind in [
                DemandUse::Residential,
                DemandUse::Commercial,
                DemandUse::Industrial,
            ] {
                let own = self
                    .collect_existing_actions(
                        allocator,
                        catalog,
                        economy_tuning,
                        residential_occupants,
                        Some(use_kind.zone_type()),
                        ExistingActions::ACTIONS_SLOT,
                    )
                    .downgrades;
                let key = |list: &[LevelChangeCandidate]| {
                    list.iter()
                        .map(|candidate| (candidate.action.clone(), candidate.building_idx))
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    key(&by_use[use_kind as usize]),
                    key(&own),
                    "one downgrade scan must equal {use_kind:?}'s own"
                );
            }
        }
        by_use
    }

    // The `kinds` of existing-building actions of `zone_type`'s buildings, or of every zone's
    // with `None`. O(B / p), parallel.
    fn collect_existing_actions(
        &self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        economy_tuning: &RuntimeEconomyTuning,
        residential_occupants: &ResidentialOccupantSnapshot,
        zone_type: Option<ZoneType>,
        kinds: ExistingActions,
    ) -> ExistingBuildingCandidates {
        let mut candidates = allocator
            .buildings
            .par_iter()
            .enumerate()
            .flat_map_iter(|(building_idx, building)| {
                let mut result = [None, None];
                if zone_type.is_some_and(|zone_type| building.zone_type != zone_type)
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
                    if kinds.despawns && building.occupancy == 0 && building.worker_count == 0 {
                        result[0] = Some(CollectedExistingBuildingCandidate::Despawn(
                            DespawnCandidate {
                                action: demand_building_action_key(building),
                                building_idx,
                            },
                        ));
                    }
                    return result;
                }
                if kinds.downgrades
                    && let Some(target_asset_id) = allocator.downgrade_target(building)
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
                if kinds.upgrades
                    && let Some(target_asset_id) = allocator.upgrade_target(building)
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

// The resources a new shop may sell, by the currency of the recurring unreserved sales
// `snapshot` leaves for them, largest first. Pantry replenishment can exceed a shop's daily
// output temporarily, so a new category is selected from recurring unreserved sales, the same
// market entry itself uses; currency rather than raw units makes the order invariant to a
// resource's unit size.
pub(super) fn commercial_spawn_priorities(
    snapshot: &DailyDemandSnapshot,
    catalog: &RuntimeEconomyCatalog,
) -> Vec<(ResourceRuntimeId, f32)> {
    let mut priorities = snapshot
        .committed_unmet_commercial_consumer_demand_by_resource
        .clone();
    for (resource, priority) in &mut priorities {
        *priority = snapshot.output_absorption.unmet_units(*resource)
            * catalog.unit_price_for_resource(*resource).unwrap_or(0.0);
    }
    priorities.retain(|(_, unmet_units)| *unmet_units > EPSILON);
    priorities.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    priorities
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
