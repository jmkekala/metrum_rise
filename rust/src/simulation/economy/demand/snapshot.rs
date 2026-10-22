// SPDX-License-Identifier: GPL-2.0-only

//! Economy snapshots consumed by project planning and household diagnostics.
//!
//! The hourly demand pass composes its snapshot from the front of a sweep folded over the
//! previous hour ([`rolling`], `SIM-02-I`). The construction-execution epoch, diagnostics and
//! tests build one from the whole city at once.

mod rolling;

pub(crate) use rolling::SavedDemandSnapshot;
#[cfg(test)]
pub(super) use rolling::SnapshotBuffer;
pub(super) use rolling::{DemandSnapshotSweep, SweepInputs};

use super::config::DemandConfig;
use super::credits::clamp01;
use super::spawn_need::OutputAbsorptionContext;
use super::types::EPSILON;
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::handles::HandleTable;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    RuntimeEconomyTuning,
};
#[cfg(test)]
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::{CityFiscalPolicy, tax_amount};
use crate::simulation::economy::hiring::JobFunding;

/// Where a building snapshot reads each building's funded jobs and funded input scale.
#[derive(Clone, Copy)]
pub(in crate::simulation::economy::demand) enum FundedJobs<'a> {
    /// A census's operation book, by building index: construction execution's own census.
    Census(&'a JobFunding),
    /// The copies the published census left on each building (`funded_job_capacity`,
    /// `funded_activity`), which saves keep, so a census rebuilt by a load or a policy change
    /// mid-hour never changes what the rolling sweep folds (`SIM-02-I`).
    Published,
}
use crate::simulation::economy::households::{
    CommercialActivitySums, CommercialActivityTerms, Household, HouseholdSystem,
    active_worker_capacity_for_profile_with_floor_scale, affordable_consumer_residents,
    commercial_activity_sums_for_city, household_is_housed, household_reserve_days,
    output_area_scale_for_building,
};
use crate::simulation::economy::logistics::collect_connected_border_nodes;
use crate::simulation::economy::reduction::ordered_fold;
use crate::simulation::economy::resource_totals::{
    add_resource_amount, merge_resource_amounts, resource_amount,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{TransitFlags, TransitType};
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

/// Housed households per residential building and their smallest reserve days, keyed by the
/// building's handle slot and generation, so a snapshot folded over an hour stays valid while
/// buildings move in the dense store (`SIM-02-I`). Readers look a building up in O(1).
#[derive(Clone, Debug, Default)]
pub(super) struct ResidentialOccupantSnapshot {
    // Per building handle slot; an entry of another generation belongs to a building that left.
    entries: Vec<OccupantEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct OccupantEntry {
    generation: u32,
    households: u32,
    min_reserve_days: f32,
}

const NO_OCCUPANTS: OccupantEntry = OccupantEntry {
    generation: 0,
    households: 0,
    min_reserve_days: f32::INFINITY,
};

impl ResidentialOccupantSnapshot {
    #[cfg(test)]
    pub(super) fn from_runtime(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
    ) -> Self {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let mut occupants = Self::default();
        for household in &households.households {
            occupants.add_household(allocator, catalog.as_ref(), tuning.as_ref(), household);
        }
        occupants
    }

    /// Housed households of the building at dense index `building_idx`. O(1).
    pub(super) fn household_count(
        &self,
        allocator: &BuildingAllocator,
        building_idx: usize,
    ) -> u32 {
        self.entry(allocator, building_idx).households
    }

    /// Smallest reserve days among those households; infinite without any. O(1).
    pub(super) fn min_reserve_days(
        &self,
        allocator: &BuildingAllocator,
        building_idx: usize,
    ) -> f32 {
        self.entry(allocator, building_idx).min_reserve_days
    }

    fn entry(&self, allocator: &BuildingAllocator, building_idx: usize) -> OccupantEntry {
        let (slot, generation) = building_key(&allocator.building_handles, building_idx);
        self.entries
            .get(slot as usize)
            .copied()
            .filter(|entry| entry.generation == generation)
            .unwrap_or(NO_OCCUPANTS)
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    // Counts a housed household at its home. O(1) amortized: the entries grow to the largest
    // home handle slot and keep their storage when cleared.
    fn add_household(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        household: &Household,
    ) {
        if household.member_count == 0 || !household_is_housed(household, allocator) {
            return;
        }
        let (slot, generation) =
            building_key(&allocator.building_handles, household.home_building_id);
        let slot = slot as usize;
        if slot >= self.entries.len() {
            self.entries.resize(slot + 1, NO_OCCUPANTS);
        }
        let entry = &mut self.entries[slot];
        // The slot was reused since an earlier household of this fold counted its old building.
        if entry.generation != generation {
            *entry = OccupantEntry {
                generation,
                ..NO_OCCUPANTS
            };
        }
        entry.households += 1;
        let reserve_days = household_reserve_days(catalog, tuning, household);
        let reserve_days = if reserve_days.is_finite() {
            reserve_days.max(0.0)
        } else {
            f32::INFINITY
        };
        entry.min_reserve_days = entry.min_reserve_days.min(reserve_days);
    }
}

// Occupant key of the building at `dense`: its handle slot and generation, or for a building
// appended to the store directly and not yet registered (tests, tools), the slot
// `register_appended` gives it when no slot is free.
fn building_key(handles: &HandleTable<Building>, dense: usize) -> (u32, u32) {
    let handle = handles.handle(dense);
    if handle.is_none() {
        (
            (handles.capacity() + dense - handles.registered()) as u32,
            0,
        )
    } else {
        (handle.index(), handle.generation())
    }
}

#[derive(Clone, Copy, Debug)]
/// A displaced household's eligibility and available labour, in household priority order.
pub(super) struct UnhousedWorkforce {
    /// Current relocation affordability, evaluated by the ordinary household contract.
    pub(super) reserve_days: f32,
    /// Adults with no current workplace assignment; already employed adults are excluded.
    pub(super) workers: u32,
}

impl UnhousedWorkforce {
    // Only displaced households need completion-time matching; the ordinary housed case is
    // `None`.
    fn of(
        household: &Household,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> Option<Self> {
        (household.member_count > 0
            && (household.adult_count > 0 || household.elder_count > 0)
            && !household_is_housed(household, allocator))
        .then(|| Self {
            reserve_days: household_reserve_days(catalog, tuning, household),
            workers: u32::from(
                household
                    .adult_count
                    .saturating_sub(household.employed_adults),
            ),
        })
    }
}

#[derive(Debug)]
pub(super) struct DailyDemandSnapshot {
    /// Existing residents who must claim eligible future homes before prospective arrivals.
    pub(super) unhoused_workforce: Vec<UnhousedWorkforce>,
    pub(super) vacant_household_slots: u32,
    pub(super) total_household_count: u32,
    pub(super) housed_household_count: u32,
    pub(super) unhoused_household_count: u32,
    pub(super) zero_budget_household_count: u32,
    pub(super) persistent_exit_eligible_household_count: u32,
    pub(super) unhoused_household_ratio: f32,
    pub(super) zero_budget_household_ratio: f32,
    pub(super) under_construction_household_slots: u32,
    pub(super) constructing_by_use: [u32; 3],
    pub(super) queued_by_use: [u32; 3],
    pub(super) committed_unmet_commercial_consumer_demand_by_resource:
        Vec<(ResourceRuntimeId, f32)>,
    pub(super) connected_border_count: u32,
    pub(super) city_treasury_balance: f32,
    /// Recurring consumption and variable imported utility cost per resident.
    pub(super) daily_essential_cost_per_resident: f32,
    /// Full home base electricity allowance, charged once per prospective household.
    pub(super) daily_home_electricity_cost: f32,
    pub(super) unemployment_daily_benefit_per_adult: f32,
    pub(super) pension_daily_benefit_per_elder: f32,
    pub(super) child_support_daily_benefit_per_child: f32,
    pub(super) existing_unemployed_member_count: u32,
    pub(super) existing_child_count: u32,
    pub(super) existing_elder_count: u32,
    pub(super) open_job_slots: u32,
    pub(super) move_in_job_slots: u32,
    pub(super) move_in_job_equivalent_slots: f32,
    pub(super) average_move_in_job_wage_per_day: f32,
    pub(super) physical_worker_capacity: u32,
    pub(super) funded_worker_capacity: u32,
    pub(super) open_jobs_unfunded: u32,
    pub(super) output_absorption: OutputAbsorptionContext,
}

/// The city-wide folds a [`DailyDemandSnapshot`] is composed from: households, buildings, the
/// commercial activity sums, the residents whose consumer basket recurring income pays for
/// after the transfer claims on the treasury in household order, and the displaced households
/// in that order.
#[derive(Clone, Debug, Default)]
pub(super) struct SnapshotTotals {
    households: HouseholdSnapshotAccumulator,
    buildings: BuildingSnapshotAccumulator,
    activity: CommercialActivitySums,
    affordable_residents: f64,
    unhoused_workforce: Vec<UnhousedWorkforce>,
}

impl SnapshotTotals {
    // Empties the totals and keeps their storage.
    fn clear(&mut self) {
        self.households = HouseholdSnapshotAccumulator::default();
        self.buildings.clear();
        self.activity = CommercialActivitySums::default();
        self.affordable_residents = 0.0;
        self.unhoused_workforce.clear();
    }
}

/// The jobs of operating workplaces: open funded jobs and their net daily wage sum, and the
/// physical, funded and unfunded worker capacities.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct JobTotals {
    open_job_slots: u32,
    open_job_wage_sum: f32,
    physical_worker_capacity: u32,
    funded_worker_capacity: u32,
    open_jobs_unfunded: u32,
}

impl JobTotals {
    // Adds building `idx`'s jobs if it is a workplace that operates, with its funded jobs read
    // from `funded`; one the census did not see complete (`!complete`) is still under
    // construction. O(1).
    #[allow(clippy::too_many_arguments)]
    fn add_workplace(
        &mut self,
        catalog: &RuntimeEconomyCatalog,
        income_tax_rate: f32,
        funded: FundedJobs<'_>,
        idx: usize,
        building: &Building,
        complete: bool,
    ) {
        if building.broken
            || building.economy_broken
            || building.is_deserted
            || building.is_under_construction()
            || !complete
        {
            return;
        }
        let Some(profile) = catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .filter(|profile| profile_offers_work(building, profile))
        else {
            return;
        };
        // A funding change does not end an employment assignment. Releasing these
        // workers early lets another project reserve them while their employer still
        // reserves its customers (e.g. before the daily bankruptcy/layoff boundary).
        let filled_workers = building.worker_count;
        let physical_worker_capacity = active_worker_capacity_for_profile_with_floor_scale(
            catalog,
            building,
            profile,
            // Stores and explicit work areas cache their activity; a store's is its census.
            building.commercial_activity_floor_scale,
        );
        let funded_worker_capacity = physical_worker_capacity.min(match funded {
            FundedJobs::Census(job_funding) => {
                job_funding.capacities.get(idx).copied().unwrap_or(0)
            }
            FundedJobs::Published => building.funded_job_capacity,
        });
        if physical_worker_capacity > funded_worker_capacity {
            self.open_jobs_unfunded = self
                .open_jobs_unfunded
                .saturating_add(physical_worker_capacity.saturating_sub(funded_worker_capacity));
        }
        if physical_worker_capacity > 0 {
            self.physical_worker_capacity = self
                .physical_worker_capacity
                .saturating_add(physical_worker_capacity);
            self.funded_worker_capacity = self
                .funded_worker_capacity
                .saturating_add(funded_worker_capacity);
        }
        if funded_worker_capacity > 0 {
            let average_daily_wage = profile.average_daily_wage();
            if average_daily_wage > 0.1 {
                let open_slots = funded_worker_capacity.saturating_sub(filled_workers);
                let net_daily_wage =
                    average_daily_wage - tax_amount(average_daily_wage, income_tax_rate);
                self.open_job_slots = self.open_job_slots.saturating_add(open_slots);
                self.open_job_wage_sum += open_slots as f32 * net_daily_wage.max(0.0);
            }
        }
    }

    fn merge(&mut self, other: Self) {
        self.open_job_slots = self.open_job_slots.saturating_add(other.open_job_slots);
        self.open_job_wage_sum += other.open_job_wage_sum;
        self.physical_worker_capacity = self
            .physical_worker_capacity
            .saturating_add(other.physical_worker_capacity);
        self.funded_worker_capacity = self
            .funded_worker_capacity
            .saturating_add(other.funded_worker_capacity);
        self.open_jobs_unfunded = self
            .open_jobs_unfunded
            .saturating_add(other.open_jobs_unfunded);
    }
}

/// The workforce a snapshot is composed with: housed unemployed adults, displaced households'
/// labour in household priority order, and the workplaces' [`JobTotals`].
///
/// Planning counts it on the live city at its pass, after the hour's job search
/// ([`Self::count_live`], `SIM-02-I`): the rolling sweep holds each household and workplace as
/// it stood at its chunk, so a hire, quit or release since would leave the job open on one
/// side and its worker employed on the other, and the workforce calendar and admission would
/// count it twice or not at all.
#[derive(Clone, Debug, Default)]
pub(super) struct WorkforceView {
    housed_unemployed: u32,
    unhoused: Vec<UnhousedWorkforce>,
    jobs: JobTotals,
}

impl WorkforceView {
    // Appends the saved form of a planning setup's workforce (`SIM-02-K`).
    pub(super) fn write_saved(&self, out: &mut Vec<u8>) {
        use super::setup::{put_f32, put_len, put_u32};
        put_u32(out, self.housed_unemployed);
        put_len(out, self.unhoused.len());
        for unhoused in &self.unhoused {
            put_f32(out, unhoused.reserve_days);
            put_u32(out, unhoused.workers);
        }
        let jobs = &self.jobs;
        put_u32(out, jobs.open_job_slots);
        put_f32(out, jobs.open_job_wage_sum);
        put_u32(out, jobs.physical_worker_capacity);
        put_u32(out, jobs.funded_worker_capacity);
        put_u32(out, jobs.open_jobs_unfunded);
    }

    // Reads what [`Self::write_saved`] wrote; `None` on a malformed buffer.
    pub(super) fn read_saved(reader: &mut super::setup::SavedReader<'_>) -> Option<Self> {
        let housed_unemployed = reader.u32()?;
        let count = reader.count()?;
        let mut unhoused = Vec::with_capacity(count);
        for _ in 0..count {
            unhoused.push(UnhousedWorkforce {
                reserve_days: reader.f32()?,
                workers: reader.u32()?,
            });
        }
        let jobs = JobTotals {
            open_job_slots: reader.u32()?,
            open_job_wage_sum: reader.f32()?,
            physical_worker_capacity: reader.u32()?,
            funded_worker_capacity: reader.u32()?,
            open_jobs_unfunded: reader.u32()?,
        };
        Some(Self {
            housed_unemployed,
            unhoused,
            jobs,
        })
    }

    /// The workforce `totals` folded.
    pub(super) fn of_totals(totals: &SnapshotTotals) -> Self {
        Self {
            housed_unemployed: totals.households.housed_unemployed_count,
            unhoused: totals.unhoused_workforce.clone(),
            jobs: totals.buildings.jobs,
        }
    }

    /// Counts the workforce of the live city, with funded jobs from the copies the published
    /// census left on the buildings, keeping the displaced list's storage. A workplace that
    /// census did not see complete counts no jobs: planning claims its constructing workers
    /// instead. A parallel scan, O(H/p + B/p); `SIM-02-K` replaces it with totals the job,
    /// household and building mutators maintain.
    pub(super) fn count_live(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        income_tax_rate: f32,
    ) {
        let census = &households.job_funding;
        let unhoused = &mut self.unhoused;
        // Three independent read-only passes, side by side (`SIM-02-K`).
        let ((housed_unemployed, ()), jobs) = rayon::join(
            || {
                rayon::join(
                    || {
                        households
                            .households
                            .par_iter()
                            .filter(|household| {
                                household.member_count > 0
                                    && household_is_housed(household, allocator)
                            })
                            .map(|household| {
                                u32::from(
                                    household
                                        .adult_count
                                        .saturating_sub(household.employed_adults),
                                )
                            })
                            .reduce(|| 0, u32::saturating_add)
                    },
                    || {
                        unhoused.clear();
                        // Indexed parallel collection preserves household priority.
                        unhoused.par_extend(households.households.par_iter().filter_map(
                            |household| {
                                UnhousedWorkforce::of(household, allocator, catalog, tuning)
                            },
                        ));
                    },
                )
            },
            || {
                ordered_fold(
                    &allocator.buildings,
                    BUILDING_SNAPSHOT_CHUNK_SIZE,
                    JobTotals::default,
                    |jobs, idx, building| {
                        jobs.add_workplace(
                            catalog,
                            income_tax_rate,
                            FundedJobs::Published,
                            idx,
                            building,
                            census.saw_operating(idx),
                        );
                    },
                    |mut merged, jobs| {
                        merged.merge(jobs);
                        merged
                    },
                )
            },
        );
        self.housed_unemployed = housed_unemployed;
        self.jobs = jobs;
    }
}

impl DailyDemandSnapshot {
    #[cfg(test)]
    pub(super) fn from_runtime(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        config: &DemandConfig,
        treasury_balance: f64,
    ) -> Self {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let fiscal_policy = CityFiscalPolicy::from_runtime_tuning(tuning.as_ref());
        Self::from_runtime_with_catalog(
            allocator,
            households,
            graph,
            config,
            catalog.as_ref(),
            tuning.as_ref(),
            &fiscal_policy,
            treasury_balance,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_runtime_with_catalog(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        config: &DemandConfig,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
    ) -> Self {
        Self::from_runtime_with_funding(
            allocator,
            households,
            graph,
            config,
            catalog,
            tuning,
            fiscal_policy,
            treasury_balance,
            &households.job_funding,
        )
    }

    /// Builds the whole snapshot at once from `job_funding`'s operation book: fixed parallel
    /// chunks and ordered merges in dense order, O(households + buildings + nodes).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_runtime_with_funding(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        config: &DemandConfig,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
        job_funding: &JobFunding,
    ) -> Self {
        let demand_sink_rates_by_resource = demand_sink_rates_by_resource(catalog);
        let activity_terms = CommercialActivityTerms::new(catalog);
        // Main-thread policy changes reach here outside any pool (CODE-17).
        let totals = crate::thread_pools::install(|| SnapshotTotals {
            households: collect_household_snapshot_accumulator(
                allocator, households, catalog, tuning, config,
            ),
            buildings: collect_building_snapshot_accumulator(
                allocator,
                catalog,
                fiscal_policy.income_tax_rate,
                &demand_sink_rates_by_resource,
                job_funding,
            ),
            activity: commercial_activity_sums_for_city(
                catalog,
                activity_terms,
                &households.households,
                allocator,
            ),
            affordable_residents: affordable_consumer_residents(
                &households.households,
                allocator,
                catalog,
                tuning,
                fiscal_policy,
                treasury_balance,
            ),
            // Indexed parallel collection preserves household priority.
            unhoused_workforce: households
                .households
                .par_iter()
                .filter_map(|household| {
                    UnhousedWorkforce::of(household, allocator, catalog, tuning)
                })
                .collect(),
        });
        let mut borders = Vec::new();
        collect_connected_border_nodes(graph, &mut borders);
        Self::compose(
            &totals,
            &WorkforceView::of_totals(&totals),
            households,
            connected_road_border_count(graph, &borders),
            catalog,
            tuning,
            fiscal_policy,
            treasury_balance,
        )
    }

    /// Composes a snapshot from city-wide totals, a workforce view and the values read when
    /// it is used: pending arrivals, the connected borders, the fiscal policy's benefits and
    /// the treasury. O(catalog profile ports + resources + displaced households).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn compose(
        totals: &SnapshotTotals,
        workforce: &WorkforceView,
        households: &HouseholdSystem,
        connected_border_count: u32,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
    ) -> Self {
        let demand_sink_rates_by_resource = demand_sink_rates_by_resource(catalog);
        let mut daily_supply_cost_per_resident = 0.0_f32;
        for &(resource_runtime_id, consumption_rate_per_resident) in &demand_sink_rates_by_resource
        {
            let resource_price = catalog
                .unit_price_for_resource(resource_runtime_id)
                .unwrap_or_else(|| {
                    let resource_id = catalog
                        .resource_id_for_runtime_id(resource_runtime_id)
                        .unwrap_or("<unknown>");
                    panic!(
                        "resource '{resource_id}' used by household demand sink has no catalog price"
                    )
                });
            daily_supply_cost_per_resident +=
                consumption_rate_per_resident.max(0.0) * resource_price.max(0.0);
        }
        // Admission must remain affordable with imported utilities, before local coverage exists.
        let daily_home_electricity_cost =
            crate::simulation::economy::households::household_daily_utility_allowance(
                catalog, tuning, 0.0,
            ) * tuning.owa_import_price_multiplier;
        let daily_essential_cost_per_resident = daily_supply_cost_per_resident
            * (1.0 + fiscal_policy.household_vat_rate)
            + crate::simulation::economy::households::household_daily_utility_allowance(
                catalog, tuning, 1.0,
            ) * tuning.owa_import_price_multiplier
            - daily_home_electricity_cost;
        let commercial_activity_signal = totals
            .activity
            .signal(CommercialActivityTerms::new(catalog));
        let household_accumulator = &totals.households;
        let housed_resident_count = household_accumulator.housed_resident_count;
        let building_accumulator = &totals.buildings;
        let total_household_slots = building_accumulator.total_household_slots;
        let occupied_household_slots = building_accumulator.occupied_household_slots;
        let under_construction_household_slots =
            building_accumulator.under_construction_household_slots;
        let JobTotals {
            open_job_slots,
            open_job_wage_sum,
            physical_worker_capacity,
            funded_worker_capacity,
            open_jobs_unfunded,
        } = workforce.jobs;
        let committed_commercial_output_capacity_by_resource =
            &building_accumulator.committed_commercial_output_capacity_by_resource;
        let business_input_need_by_resource = &building_accumulator.business_input_need_by_resource;
        let committed_output_capacity_by_resource =
            &building_accumulator.committed_output_capacity_by_resource;
        let existing_unemployed_member_count = workforce
            .housed_unemployed
            .saturating_add(households.pending_arrivals.adults);

        let vacant_household_slots = total_household_slots.saturating_sub(occupied_household_slots);
        let live_child_count = household_accumulator
            .live_child_count
            .saturating_add(households.pending_arrivals.children);
        let live_elder_count = household_accumulator
            .live_elder_count
            .saturating_add(households.pending_arrivals.elders);
        let housed_household_count = household_accumulator.housed_household_count;
        let unhoused_household_count = household_accumulator.unhoused_household_count;
        let zero_budget_household_count = household_accumulator.zero_budget_household_count;
        let persistent_exit_eligible_household_count =
            household_accumulator.persistent_exit_eligible_household_count;
        let mut output_absorption = OutputAbsorptionContext::from_resource_amounts(
            catalog.resource_count(),
            committed_output_capacity_by_resource,
            &demand_sink_rates_by_resource,
            0,
            business_input_need_by_resource,
        );
        let affordable_residents = totals.affordable_residents;
        for &(resource, rate) in &demand_sink_rates_by_resource {
            output_absorption
                .add_demand_units(resource, (f64::from(rate) * affordable_residents) as f32);
        }

        let export_expansion_available =
            connected_border_count > 0 && tuning.owa_export_price_multiplier > EPSILON;
        output_absorption.set_export_expansion_available(export_expansion_available);

        let total_household_count = housed_household_count
            .saturating_add(unhoused_household_count)
            .saturating_add(households.pending_arrivals.households);
        let mut committed_unmet_commercial_consumer_demand_by_resource = Vec::new();
        for &(resource_runtime_id, consumption_rate_per_resident) in &demand_sink_rates_by_resource
        {
            let consumer_demand = if resource_runtime_id
                == commercial_activity_signal.household_supply_resource_runtime_id
            {
                commercial_activity_signal.demand_units_per_day
            } else {
                consumption_rate_per_resident * housed_resident_count as f32
            };
            if consumer_demand <= 0.0 {
                continue;
            }
            let committed_capacity = resource_amount(
                committed_commercial_output_capacity_by_resource,
                resource_runtime_id,
            );
            let committed_gap = (consumer_demand - committed_capacity).max(0.0);
            add_resource_amount(
                &mut committed_unmet_commercial_consumer_demand_by_resource,
                resource_runtime_id,
                committed_gap,
            );
        }
        let unhoused_household_ratio = if total_household_count == 0 {
            0.0
        } else {
            clamp01(unhoused_household_count as f32 / total_household_count as f32)
        };
        let zero_budget_household_ratio = if total_household_count == 0 {
            0.0
        } else {
            clamp01(zero_budget_household_count as f32 / total_household_count as f32)
        };
        let net_open_job_slots = open_job_slots.saturating_sub(existing_unemployed_member_count);
        let net_open_job_wage_sum = if open_job_slots == 0 {
            0.0
        } else {
            open_job_wage_sum * net_open_job_slots as f32 / open_job_slots as f32
        };
        // A household cannot finance its arrival with jobs its own future spending might
        // create. Only existing open jobs enter admission's finite recurring-income budget.
        // Housing need converts them into homes with admission's applicant stream.
        let move_in_job_slots = net_open_job_slots;
        let move_in_job_equivalent_slots = net_open_job_slots as f32;
        let average_move_in_job_wage_per_day = if move_in_job_equivalent_slots <= EPSILON {
            0.0
        } else {
            net_open_job_wage_sum / move_in_job_equivalent_slots
        };

        debug_log!(
            "spawn",
            "daily_snapshot: border_nodes={} homes={} \
             live_jobs={} available_jobs={} pending_homes={}",
            connected_border_count,
            vacant_household_slots,
            open_job_slots,
            move_in_job_slots,
            under_construction_household_slots,
        );

        Self {
            unhoused_workforce: workforce.unhoused.clone(),
            vacant_household_slots,
            total_household_count,
            housed_household_count,
            unhoused_household_count,
            zero_budget_household_count,
            persistent_exit_eligible_household_count,
            unhoused_household_ratio,
            zero_budget_household_ratio,
            committed_unmet_commercial_consumer_demand_by_resource,
            under_construction_household_slots,
            constructing_by_use: building_accumulator.constructing_by_use,
            queued_by_use: [0; 3],
            connected_border_count,
            city_treasury_balance: treasury_balance as f32,
            daily_essential_cost_per_resident,
            daily_home_electricity_cost,
            unemployment_daily_benefit_per_adult: fiscal_policy
                .unemployment_benefit_per_adult_per_day,
            pension_daily_benefit_per_elder: fiscal_policy.pension_per_elder_per_day,
            child_support_daily_benefit_per_child: fiscal_policy.child_support_per_child_per_day,
            existing_unemployed_member_count,
            existing_child_count: live_child_count,
            existing_elder_count: live_elder_count,
            open_job_slots,
            move_in_job_slots,
            move_in_job_equivalent_slots,
            average_move_in_job_wage_per_day,
            physical_worker_capacity,
            funded_worker_capacity,
            open_jobs_unfunded,
            output_absorption,
        }
    }
}

/// Connected borders among `borders` (as [`collect_connected_border_nodes`] lists them) that
/// have a live road open to cars. O(borders × degree).
pub(super) fn connected_road_border_count(graph: &RegionGraph, borders: &[u32]) -> u32 {
    borders
        .iter()
        .filter(|&&node| {
            graph.node_adjacency(node).iter().any(|&edge_idx| {
                let edge = graph.edge(edge_idx);
                !edge.deleted
                    && edge.primary_type == TransitType::Road
                    && (edge.allowed_types & TransitFlags::CAR) != 0
            })
        })
        .count() as u32
}

// Household demand-sink rates of the resources stores sell.
fn demand_sink_rates_by_resource(catalog: &RuntimeEconomyCatalog) -> Vec<(ResourceRuntimeId, f32)> {
    let mut rates = Vec::new();
    demand_sink_rates_into(catalog, &mut Vec::new(), &mut rates);
    rates
}

// `demand_sink_rates_by_resource` into reused storage; `store_outputs` is scratch.
// O(profile ports), allocation-free once warm.
fn demand_sink_rates_into(
    catalog: &RuntimeEconomyCatalog,
    store_outputs: &mut Vec<(ResourceRuntimeId, f32)>,
    rates: &mut Vec<(ResourceRuntimeId, f32)>,
) {
    store_outputs.clear();
    rates.clear();
    for profile in catalog.all_profiles() {
        if !matches!(
            profile.kind,
            EconomyProfileRuntimeKind::Store | EconomyProfileRuntimeKind::ServiceStore
        ) {
            continue;
        }
        for output_port in &profile.outputs {
            add_resource_amount(store_outputs, output_port.resource_runtime_id, 1.0);
        }
    }
    for profile in catalog.all_profiles() {
        if profile.kind != EconomyProfileRuntimeKind::DemandSink {
            continue;
        }
        for input_port in &profile.inputs {
            if resource_amount(store_outputs, input_port.resource_runtime_id) <= 0.0 {
                continue;
            }
            add_resource_amount(
                rates,
                input_port.resource_runtime_id,
                profile.consumption_rate_per_resident,
            );
        }
    }
}

const BUILDING_SNAPSHOT_CHUNK_SIZE: usize = 1024;
const HOUSEHOLD_SNAPSHOT_CHUNK_SIZE: usize = 2048;
#[derive(Clone, Debug, Default)]
struct BuildingSnapshotAccumulator {
    total_household_slots: u32,
    occupied_household_slots: u32,
    under_construction_household_slots: u32,
    constructing_by_use: [u32; 3],
    jobs: JobTotals,
    committed_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
    committed_commercial_output_capacity_by_resource: Vec<(ResourceRuntimeId, f32)>,
    business_input_need_by_resource: Vec<(ResourceRuntimeId, f32)>,
}

impl BuildingSnapshotAccumulator {
    // Zeroes the sums and keeps the resource lists' storage.
    fn clear(&mut self) {
        let mut cleared = Self {
            committed_output_capacity_by_resource: std::mem::take(
                &mut self.committed_output_capacity_by_resource,
            ),
            committed_commercial_output_capacity_by_resource: std::mem::take(
                &mut self.committed_commercial_output_capacity_by_resource,
            ),
            business_input_need_by_resource: std::mem::take(
                &mut self.business_input_need_by_resource,
            ),
            ..Self::default()
        };
        cleared.committed_output_capacity_by_resource.clear();
        cleared
            .committed_commercial_output_capacity_by_resource
            .clear();
        cleared.business_input_need_by_resource.clear();
        *self = cleared;
    }

    #[allow(clippy::too_many_arguments)]
    fn absorb_building(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        income_tax_rate: f32,
        demand_sink_rates_by_resource: &[(ResourceRuntimeId, f32)],
        funded: FundedJobs<'_>,
        idx: usize,
        building: &Building,
    ) {
        if building.broken || building.economy_broken || building.is_deserted {
            return;
        }

        let is_private_building = allocator
            .registry
            .get(&building.asset_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .map(|authored| authored.is_zoned_private())
            .unwrap_or(!matches!(building.zone_type, ZoneType::None));
        let active_profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);

        if building.is_under_construction() {
            if is_private_building
                && let Some(index) = super::actions::private_zone_index(building.zone_type)
            {
                self.constructing_by_use[index] += 1;
            }
            self.under_construction_household_slots = self
                .under_construction_household_slots
                .saturating_add(allocator.registry.household_capacity(&building.asset_id));
            if let Some(profile) = active_profile {
                let scale = output_area_scale_for_building(building, profile);
                for output_port in &profile.outputs {
                    let net_output = profile.net_output_units_per_day(output_port) * scale;
                    add_resource_amount(
                        &mut self.committed_output_capacity_by_resource,
                        output_port.resource_runtime_id,
                        net_output,
                    );
                    if building.zone_type == ZoneType::Commercial
                        && resource_amount(
                            demand_sink_rates_by_resource,
                            output_port.resource_runtime_id,
                        ) > 0.0
                    {
                        add_resource_amount(
                            &mut self.committed_commercial_output_capacity_by_resource,
                            output_port.resource_runtime_id,
                            net_output,
                        );
                    }
                }
            }
            return;
        }

        let household_capacity = allocator.household_capacity(idx);
        if household_capacity > 0 {
            self.total_household_slots = self
                .total_household_slots
                .saturating_add(household_capacity);
            let occupied = building.occupancy.min(household_capacity);
            self.occupied_household_slots = self.occupied_household_slots.saturating_add(occupied);
        }

        self.jobs
            .add_workplace(catalog, income_tax_rate, funded, idx, building, true);

        if let Some(profile) = active_profile {
            let commercial = building.zone_type == ZoneType::Commercial;
            for output_port in &profile.outputs {
                // Completion, staff searches and restocking do not release this firm's
                // customers to new construction. Hiring uses the same physical net capacity.
                let committed_units = profile.net_output_units_per_day(output_port)
                    * output_area_scale_for_building(building, profile);
                add_resource_amount(
                    &mut self.committed_output_capacity_by_resource,
                    output_port.resource_runtime_id,
                    committed_units,
                );
                if commercial
                    && resource_amount(
                        demand_sink_rates_by_resource,
                        output_port.resource_runtime_id,
                    ) > 0.0
                {
                    add_resource_amount(
                        &mut self.committed_commercial_output_capacity_by_resource,
                        output_port.resource_runtime_id,
                        committed_units,
                    );
                }
            }
            let service = allocator.is_city_service_building(building);
            let input_capacity_scale = match funded {
                FundedJobs::Census(job_funding) => {
                    job_funding.input_demand_scale(idx, building, profile, service)
                }
                FundedJobs::Published => {
                    JobFunding::published_input_demand_scale(building, profile, service)
                }
            };
            for input_port in &profile.inputs {
                // Maintenance supplied by this recipe is already excluded from net output.
                // It cannot also claim another supplier's customers.
                let internal = profile
                    .output_port(input_port.resource_runtime_id)
                    .map_or(0.0, |port| port.units_per_day);
                let input_units_per_day =
                    (input_port.units_per_day - internal).max(0.0) * input_capacity_scale;
                add_resource_amount(
                    &mut self.business_input_need_by_resource,
                    input_port.resource_runtime_id,
                    input_units_per_day,
                );
            }
        }
    }

    fn merge(&mut self, other: Self) {
        for (left, right) in self
            .constructing_by_use
            .iter_mut()
            .zip(other.constructing_by_use)
        {
            *left += right;
        }
        self.total_household_slots = self
            .total_household_slots
            .saturating_add(other.total_household_slots);
        self.occupied_household_slots = self
            .occupied_household_slots
            .saturating_add(other.occupied_household_slots);
        self.under_construction_household_slots = self
            .under_construction_household_slots
            .saturating_add(other.under_construction_household_slots);
        self.jobs.merge(other.jobs);
        merge_resource_amounts(
            &mut self.committed_output_capacity_by_resource,
            other.committed_output_capacity_by_resource,
        );
        merge_resource_amounts(
            &mut self.committed_commercial_output_capacity_by_resource,
            other.committed_commercial_output_capacity_by_resource,
        );
        merge_resource_amounts(
            &mut self.business_input_need_by_resource,
            other.business_input_need_by_resource,
        );
    }
}

fn profile_offers_work(building: &Building, profile: &EconomyProfileRuntime) -> bool {
    matches!(
        building.zone_type,
        ZoneType::Commercial | ZoneType::Industrial
    ) || matches!(
        profile.kind,
        EconomyProfileRuntimeKind::Extractor
            | EconomyProfileRuntimeKind::FieldProducer
            | EconomyProfileRuntimeKind::UtilityProducer
            | EconomyProfileRuntimeKind::UtilityProcessor
    )
}

fn collect_building_snapshot_accumulator(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    income_tax_rate: f32,
    demand_sink_rates_by_resource: &[(ResourceRuntimeId, f32)],
    job_funding: &JobFunding,
) -> BuildingSnapshotAccumulator {
    ordered_fold(
        &allocator.buildings,
        BUILDING_SNAPSHOT_CHUNK_SIZE,
        BuildingSnapshotAccumulator::default,
        |accumulator, idx, building| {
            accumulator.absorb_building(
                allocator,
                catalog,
                income_tax_rate,
                demand_sink_rates_by_resource,
                FundedJobs::Census(job_funding),
                idx,
                building,
            );
        },
        |mut merged, accumulator| {
            merged.merge(accumulator);
            merged
        },
    )
}

#[derive(Clone, Debug, Default)]
struct HouseholdSnapshotAccumulator {
    housed_resident_count: u32,
    housed_unemployed_count: u32,
    live_child_count: u32,
    live_elder_count: u32,
    housed_household_count: u32,
    unhoused_household_count: u32,
    zero_budget_household_count: u32,
    persistent_exit_eligible_household_count: u32,
}

impl HouseholdSnapshotAccumulator {
    fn absorb_household(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        config: &DemandConfig,
        household: &Household,
    ) {
        if household.member_count == 0 {
            return;
        }
        if household.budget <= EPSILON {
            self.zero_budget_household_count = self.zero_budget_household_count.saturating_add(1);
        }
        self.live_child_count = self
            .live_child_count
            .saturating_add(household.child_count as u32);
        self.live_elder_count = self
            .live_elder_count
            .saturating_add(household.elder_count as u32);
        if household_is_housed(household, allocator) {
            self.housed_household_count = self.housed_household_count.saturating_add(1);
            self.housed_resident_count = self
                .housed_resident_count
                .saturating_add(household.member_count as u32);
            self.housed_unemployed_count = self.housed_unemployed_count.saturating_add(u32::from(
                household
                    .adult_count
                    .saturating_sub(household.employed_adults),
            ));
        } else {
            self.unhoused_household_count = self.unhoused_household_count.saturating_add(1);
            if config
                .household_action
                .exit_policy
                .is_eligible(household, allocator, catalog, tuning)
            {
                self.persistent_exit_eligible_household_count = self
                    .persistent_exit_eligible_household_count
                    .saturating_add(1);
            }
        }
    }

    fn merge(&mut self, other: Self) {
        self.housed_resident_count = self
            .housed_resident_count
            .saturating_add(other.housed_resident_count);
        self.housed_unemployed_count = self
            .housed_unemployed_count
            .saturating_add(other.housed_unemployed_count);
        self.live_child_count = self.live_child_count.saturating_add(other.live_child_count);
        self.live_elder_count = self.live_elder_count.saturating_add(other.live_elder_count);
        self.housed_household_count = self
            .housed_household_count
            .saturating_add(other.housed_household_count);
        self.unhoused_household_count = self
            .unhoused_household_count
            .saturating_add(other.unhoused_household_count);
        self.zero_budget_household_count = self
            .zero_budget_household_count
            .saturating_add(other.zero_budget_household_count);
        self.persistent_exit_eligible_household_count = self
            .persistent_exit_eligible_household_count
            .saturating_add(other.persistent_exit_eligible_household_count);
    }
}

fn collect_household_snapshot_accumulator(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    config: &DemandConfig,
) -> HouseholdSnapshotAccumulator {
    ordered_fold(
        &households.households,
        HOUSEHOLD_SNAPSHOT_CHUNK_SIZE,
        HouseholdSnapshotAccumulator::default,
        |accumulator, _, household| {
            accumulator.absorb_household(allocator, catalog, tuning, config, household);
        },
        |mut merged, accumulator| {
            merged.merge(accumulator);
            merged
        },
    )
}

/// Reuses the household census for daily exits without evaluating unrelated business markets.
#[cfg(test)]
pub(super) fn collect_household_removal_diagnostics(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    config: &DemandConfig,
) -> super::diagnostics::HouseholdRemovalDiagnostics {
    let counts =
        collect_household_snapshot_accumulator(allocator, households, catalog, tuning, config);
    let total = counts
        .housed_household_count
        .saturating_add(counts.unhoused_household_count)
        .saturating_add(households.pending_arrivals.households);
    let ratio = |count| {
        if total == 0 {
            0.0
        } else {
            clamp01(count as f32 / total as f32)
        }
    };
    super::diagnostics::HouseholdRemovalDiagnostics {
        total_household_count: total,
        housed_household_count: counts.housed_household_count,
        unhoused_household_count: counts.unhoused_household_count,
        zero_budget_household_count: counts.zero_budget_household_count,
        persistent_exit_eligible_household_count: counts.persistent_exit_eligible_household_count,
        unhoused_household_ratio: ratio(counts.unhoused_household_count),
        zero_budget_household_ratio: ratio(counts.zero_budget_household_count),
        ..Default::default()
    }
}
