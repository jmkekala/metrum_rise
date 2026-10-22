// SPDX-License-Identifier: GPL-2.0-only

//! Demand system state and pass orchestration.

use super::UseTuningF32;
use super::actions::{DemandBuildingActionPlan, DemandSpawnAction, PlanningSlot};
use super::admission::HouseholdAdmissionBudget;
use super::config::{DemandConfig, load_builtin_demand_config};
#[cfg(test)]
use super::credits::advance_persistent_exit_credit;
use super::credits::clamp01;
use super::diagnostics::{
    BuildingActionDiagnosticsByUse, HouseholdAdmissionDiagnostics, HouseholdRemovalDiagnostics,
};
use super::snapshot::{
    DailyDemandSnapshot, DemandSnapshotSweep, SavedDemandSnapshot, SweepInputs, WorkforceView,
    connected_road_border_count,
};
use super::supply::SiteSupply;
use super::types::DEMAND_HOURLY_CADENCE_FRACTION;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingSiteEnvironment};
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, RuntimeEconomyTuning, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::hiring::{CensusKey, JobFunding};
use crate::simulation::economy::households::{HouseholdExitPolicy, HouseholdSystem};
use crate::simulation::economy::logistics::{ShipmentSystem, collect_connected_border_nodes};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::zoning::ZoningSystem;
use std::sync::Arc;

/// What one lending of a census book prepares for planning (`SIM-02-K`): the minute-58 and
/// minute-59 preparation splits the work over window slots of its own, and loading prepares
/// the whole at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanningPreparation {
    /// The pass rows and the supplier rows, without their layout.
    SupplierRows,
    /// The supplier rows, unless current, and their layout.
    Suppliers,
    /// Everything; the book is then prepared. With `masks` the business book's finance masks
    /// are refreshed too, else a later slot refreshes them.
    Whole {
        /// Whether the business book's finance masks are refreshed in this lending.
        masks: bool,
    },
}

/// Demand-owned daily growth state derived from the settled economy snapshot.
pub struct DemandSystem {
    pub(super) config: Arc<DemandConfig>,
    pub(super) runtime_catalog: Arc<RuntimeEconomyCatalog>,
    pub(super) runtime_tuning: Arc<RuntimeEconomyTuning>,
    pub(crate) cheat_max_demands_enabled: bool,
    pub(crate) households_to_admit_today: u32,
    /// Departures released so far in the calendar day (`SIM-02-G`).
    pub(crate) households_to_remove_today: u32,
    pub(crate) persistent_exit_action_credit: f32,
    /// Whether any household was eligible to leave at its daily phase in the calendar day.
    pub(crate) exit_eligible_today: bool,
    pub(crate) upgrade_action_credit: UseTuningF32,
    pub(crate) downgrade_action_credit: UseTuningF32,
    pub(crate) despawn_action_credit: UseTuningF32,
    /// The hour's plan (`SIM-02-I`): each zone slot's spawns, upgrades and despawns, with the
    /// actions slot's downgrades and claimed optional homes, until the actions slot queues the
    /// spawns and executes the rest. Saved between the slots.
    pub(crate) building_actions: DemandBuildingActionPlan,
    /// Homes the residential slot offers employers without selecting them (`DEM-02-C`):
    /// later slots count them as optional housing and the actions slot queues those the
    /// workforce claims. Saved between the slots.
    pub(crate) optional_homes: Vec<DemandSpawnAction>,
    pub(super) last_admission_diagnostics: HouseholdAdmissionDiagnostics,
    pub(super) last_removal_diagnostics: HouseholdRemovalDiagnostics,
    pub(super) last_building_action_diagnostics: BuildingActionDiagnosticsByUse,
    /// The demand snapshot folded over the previous hour (`SIM-02-I`); saved.
    pub(super) snapshot_sweep: DemandSnapshotSweep,
    // The live workforce each pass composes its snapshot with; storage only, not saved.
    pub(super) workforce_view: WorkforceView,
    // What a planning slot's setup slot counted and scanned for its plan (`SIM-02-K`); saved
    // between the two.
    pub(super) planning_setup: Option<super::setup::PlanningSetup>,
    pub(super) workforce_reservations: super::workforce::WorkforceReservations,
    pub(super) construction_execution: super::execution::ConstructionExecution,
    pub(super) queued_construction: Vec<super::queued::QueuedConstruction>,
    pub(super) queued_supply_order: Vec<usize>,
    /// Operational hour relative to which queued completion offsets were prepared.
    pub(super) queued_reference_hour: u64,
    pub(super) next_construction_start_hour: u64,
    pub(super) admission_budget: HouseholdAdmissionBudget,
    pub(super) freight_border_nodes: Vec<u32>,
    /// Current outside applicant seed, derived from simulation time rather than accepted arrivals.
    pub(crate) admission_candidate_seed: usize,
    /// Level changes the hour's actions slot proved, oldest first, until the `level changes`
    /// queue applies them (`SIM-02-J`). Saved.
    pub(crate) level_change_requests:
        std::collections::VecDeque<super::execution::QueuedLevelChange>,
    // The `level changes` queue's service this hour, logged at its last slot.
    pub(crate) level_change_stats: crate::simulation::core::request_queue::QueueStats,
    /// Earlier slots' spawns and upgrades this system's later slots dropped on a failed
    /// re-proof (`SIM-02-I`); [`crate::debug::work::Work::ReproofDrops`] counts them across
    /// the process.
    #[cfg(test)]
    pub(crate) reproof_drops: u64,
}

/// Checks after every test-build planning pass that the live books return bit for bit.
/// Long populated-city measurements switch it off; it costs a full fingerprint per pass.
#[cfg(test)]
pub(crate) static VERIFY_LIVE_PASSES: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

#[cfg(test)]
pub(super) fn live_fingerprint(
    funding: &crate::simulation::economy::hiring::JobFunding,
) -> [Vec<(&'static str, u64)>; 3] {
    [
        funding.customers.live_fingerprint_for_test(),
        funding.business.live_fingerprint_for_test(),
        funding.input_supply.live_fingerprint_for_test(),
    ]
}

impl Default for DemandSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl DemandSystem {
    /// Creates a new demand system using the shipped demand tuning file.
    pub fn new() -> Self {
        let config = load_builtin_demand_config()
            .unwrap_or_else(|err| panic!("could not load built-in demand tuning: {err}"));
        let runtime_catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let runtime_tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        Self {
            config,
            runtime_catalog,
            runtime_tuning,
            cheat_max_demands_enabled: false,
            households_to_admit_today: 0,
            households_to_remove_today: 0,
            persistent_exit_action_credit: 0.0,
            exit_eligible_today: false,
            upgrade_action_credit: UseTuningF32::default(),
            downgrade_action_credit: UseTuningF32::default(),
            despawn_action_credit: UseTuningF32::default(),
            building_actions: DemandBuildingActionPlan::default(),
            optional_homes: Vec::new(),
            last_admission_diagnostics: HouseholdAdmissionDiagnostics::default(),
            last_removal_diagnostics: HouseholdRemovalDiagnostics::default(),
            last_building_action_diagnostics: BuildingActionDiagnosticsByUse::default(),
            snapshot_sweep: DemandSnapshotSweep::default(),
            workforce_view: WorkforceView::default(),
            planning_setup: None,
            workforce_reservations: super::workforce::WorkforceReservations::default(),
            construction_execution: super::execution::ConstructionExecution::default(),
            queued_construction: Vec::new(),
            queued_supply_order: Vec::new(),
            queued_reference_hour: 0,
            next_construction_start_hour: 0,
            admission_budget: HouseholdAdmissionBudget::default(),
            freight_border_nodes: Vec::new(),
            admission_candidate_seed: 0,
            level_change_requests: std::collections::VecDeque::new(),
            level_change_stats: Default::default(),
            #[cfg(test)]
            reproof_drops: 0,
        }
    }

    /// Returns the cached compiled economy catalog used by demand planning and execution.
    pub(crate) fn runtime_catalog(&self) -> &RuntimeEconomyCatalog {
        self.runtime_catalog.as_ref()
    }

    /// The runtime economy catalog, shared, for callers that mutate this system while reading
    /// it. O(1).
    pub(crate) fn shared_runtime_catalog(&self) -> Arc<RuntimeEconomyCatalog> {
        Arc::clone(&self.runtime_catalog)
    }

    /// Returns the cached economy runtime tuning used by demand planning and execution.
    pub(crate) fn runtime_tuning(&self) -> &RuntimeEconomyTuning {
        self.runtime_tuning.as_ref()
    }

    /// Advances the deterministic outside applicant stream even when a household is rejected.
    pub(crate) fn set_admission_candidate_hour(&mut self, day_index: u32, minute_of_day: u16) {
        self.admission_candidate_seed = day_index as usize * 24 + usize::from(minute_of_day / 60);
    }

    /// Consumes a batch's remaining recurring-income capacity for one actual arrival candidate.
    pub(crate) fn claim_household_admission(
        &mut self,
        composition: crate::simulation::economy::agents::HouseholdAgeComposition,
        members: u16,
        home_level: u8,
        fiscal_policy: &CityFiscalPolicy,
    ) -> bool {
        self.last_admission_diagnostics.examined_candidates += 1;
        if !self.admission_budget.can_house(composition) {
            self.last_admission_diagnostics.worker_housing_rejections += 1;
            return false;
        }
        let accepted = self
            .admission_budget
            .claim(composition, members, home_level, fiscal_policy);
        if !accepted {
            self.last_admission_diagnostics.income_rejections += 1;
        }
        accepted
    }

    /// Advances hourly household and building demand outputs.
    #[cfg(test)]
    pub(crate) fn run_hourly_pass(
        &mut self,
        allocator: &BuildingAllocator,
        households: &mut HouseholdSystem,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        treasury_balance: f64,
    ) {
        let fiscal_policy = CityFiscalPolicy::from_runtime_tuning(self.runtime_tuning.as_ref());
        let terrain = crate::simulation::terrain::TerrainSystem::new(4, 4);
        let mut road_surface = crate::simulation::network::surface::RoadSurfaceSystem::new(512.0);
        road_surface.compile_dirty(graph, &terrain);
        let mut routing_graph = graph.clone();
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut routing_graph);
        let graph = &routing_graph;
        network.rebuild_cch_and_check(graph);
        // Planning without the slot clock reads a snapshot of the current state.
        self.discard_snapshot_front();
        self.run_planning_hour_for_test(
            allocator,
            households,
            graph,
            zoning,
            treasury_balance,
            &fiscal_policy,
            &network,
            &mut ShipmentSystem::new(),
            BuildingSiteEnvironment {
                road_surface: &road_surface,
                terrain: &terrain,
            },
            420,
        );
    }

    /// Test driver of an hour's demand planning without the slot clock or execution: publishes
    /// a census when the published one is not current, then runs the hour's four planning
    /// slots in turn (`SIM-02-I`). The whole hour's plan stays in the plan for the test to read.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_planning_hour_for_test(
        &mut self,
        allocator: &BuildingAllocator,
        households: &mut HouseholdSystem,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        treasury_balance: f64,
        fiscal_policy: &CityFiscalPolicy,
        network: &TransitNetwork,
        logistics: &mut ShipmentSystem,
        environment: BuildingSiteEnvironment<'_>,
        minute_of_day: u16,
    ) {
        self.publish_census_if_stale_for_test(
            allocator,
            households,
            network,
            graph,
            logistics,
            fiscal_policy,
            treasury_balance,
            minute_of_day,
        );
        for slot in PlanningSlot::ALL {
            self.run_planning_slot(
                slot,
                allocator,
                households,
                graph,
                zoning,
                treasury_balance,
                fiscal_policy,
                network,
                logistics,
                environment,
                minute_of_day,
            );
        }
    }

    /// Ends the hour's plan after the actions slot executed it (`SIM-02-I`).
    pub(crate) fn clear_hour_plan(&mut self) {
        self.building_actions = DemandBuildingActionPlan::default();
        self.optional_homes.clear();
    }

    /// Runs planning slot `slot` of the hour (`SIM-02-I`) on the census published at the hour's
    /// first slot: the residential, commercial or industrial slot plans its zone's spawns,
    /// upgrades and despawns; the actions slot plans the downgrades and the hour's admission
    /// and selects the optional homes the workforce claims. A slot plans directly on the live
    /// customer, business and input books inside one first-touch transaction and restores them
    /// before returning, so its cost follows the rows it touches rather than a copy of every
    /// book. Books not valid for the live city are never lent: the slot is skipped and
    /// counted, and returns `false`. The plan decides on the live workforce and the scans its
    /// setup slot held ([`Self::prepare_planning_setup`], `SIM-02-K`); without a held setup it
    /// counts and scans itself.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_planning_slot(
        &mut self,
        slot: PlanningSlot,
        allocator: &BuildingAllocator,
        live_households: &mut HouseholdSystem,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        treasury_balance: f64,
        fiscal_policy: &CityFiscalPolicy,
        network: &TransitNetwork,
        logistics: &mut ShipmentSystem,
        environment: BuildingSiteEnvironment<'_>,
        minute_of_day: u16,
    ) -> bool {
        #[cfg(test)]
        let _contract_phase = crate::debug::work::phases::enter("planning");
        let mut phase = crate::debug::Channel::EconomyPhases
            .enabled()
            .then(std::time::Instant::now);
        let mut times = [0.0; 7];
        let mut lap = |slot: usize| {
            if let Some(previous) = phase {
                times[slot] = previous.elapsed().as_secs_f64() * 1_000.0;
                phase = Some(std::time::Instant::now());
            }
        };
        crate::debug::work::add(crate::debug::work::Work::PlanningPasses, 1);
        // What this slot's setup slot counted and scanned, if it is held for this plan
        // (`SIM-02-K`); any other held setup is dropped.
        let setup = self.take_planning_setup(slot, allocator.now_slot);
        // The hour's plan starts at its first slot.
        if slot == PlanningSlot::Residential {
            self.building_actions = DemandBuildingActionPlan::default();
            self.optional_homes.clear();
        }
        let catalog = Arc::clone(&self.runtime_catalog);
        let economy_tuning = Arc::clone(&self.runtime_tuning);
        // Move reusable gateway scratch out while the planner mutates the other demand state.
        let mut borders = std::mem::take(&mut self.freight_border_nodes);
        collect_connected_border_nodes(graph, &mut borders);
        logistics.prepare_site_carrier_quotes(allocator, network, graph, &borders);
        lap(0);
        // The census published at this slot, before the operational hour, on the live city's
        // key: appends since its copy are covered by the lent books (`SIM-02-I`). Books that are
        // not valid for the live key are never lent: the pass is skipped and counted, and
        // planning waits for the next publication.
        if !self.lent_books_current(live_households, network, &borders, minute_of_day) {
            crate::debug::work::add(slot.skip_work(), 1);
            crate::debug_log!(
                "economy",
                "planning slot {:?} skipped: the published census is not valid for the live city: minute={}",
                slot,
                minute_of_day
            );
            if slot == PlanningSlot::Actions {
                self.households_to_admit_today = 0;
            }
            self.freight_border_nodes = borders;
            return false;
        }
        live_households.job_funding.planning_prepared = true;
        let mut reservations = std::mem::take(&mut self.construction_execution.reservations);
        #[cfg(test)]
        let before = VERIFY_LIVE_PASSES
            .load(std::sync::atomic::Ordering::Relaxed)
            .then(|| live_fingerprint(&live_households.job_funding));
        // Lend the live books to the planner, which reads the matching funding columns in
        // place.
        let mut inputs =
            super::execution::lend_live_books(&mut reservations, &mut live_households.job_funding);
        let households = &*live_households;
        lap(1);
        let funded_jobs = &households.job_funding;
        reservations.begin_live_pass(
            funded_jobs,
            &mut inputs,
            allocator,
            graph,
            &catalog,
            &economy_tuning,
            fiscal_policy,
            minute_of_day,
        );
        lap(2);
        // The snapshot swept over the previous hour, with the entities created since
        // (`SIM-02-I`); a world without one builds it now from the census this pass plans with.
        let front = self.snapshot_sweep.take_front(&SweepInputs {
            allocator,
            households,
            config: &self.config,
            catalog: catalog.as_ref(),
            tuning: economy_tuning.as_ref(),
            policy: fiscal_policy,
            treasury: treasury_balance,
        });
        // The workforce as the hour's job search left it, not as the sweep found each entity:
        // counted at this slot's setup slot, or now without one (`SIM-02-K`).
        let scans = match setup {
            Some((workforce, scans)) => {
                self.workforce_view = workforce;
                Some(scans)
            }
            None => {
                self.workforce_view.count_live(
                    allocator,
                    households,
                    catalog.as_ref(),
                    economy_tuning.as_ref(),
                    fiscal_policy.income_tax_rate,
                );
                None
            }
        };
        let mut snapshot = DailyDemandSnapshot::compose(
            &front.totals,
            &self.workforce_view,
            households,
            connected_road_border_count(graph, &borders),
            catalog.as_ref(),
            economy_tuning.as_ref(),
            fiscal_policy,
            treasury_balance,
        );
        snapshot.reserve_queued_construction(catalog.as_ref(), &self.queued_construction);

        self.admission_budget = HouseholdAdmissionBudget::from_snapshot(&snapshot, &self.config);
        // Admission runs at the actions slot, after the hour's plans and before its actions,
        // as it ran between the single pass and the actions.
        if slot == PlanningSlot::Actions {
            // This is execution capacity, not approval of the preview household. Each actual
            // applicant claims its own income below. A jobless town can consider pension-funded
            // applicants too; temporary benefits cannot pay recurring essentials.
            self.households_to_admit_today = if snapshot.connected_border_count == 0 {
                0
            } else {
                snapshot
                    .vacant_household_slots
                    .min(snapshot.move_in_job_slots.div_ceil(24).max(1))
            };
            let mut admission_diagnostics = HouseholdAdmissionDiagnostics::from_snapshot(&snapshot);
            admission_diagnostics.planned_households = self.households_to_admit_today;
            admission_diagnostics.launched_households = 0;
            admission_diagnostics.incoming_household_need =
                self.last_admission_diagnostics.incoming_household_need;
            self.last_admission_diagnostics = admission_diagnostics;
        }
        lap(3);
        // The workforce reset's pass over the buildings' free places reads neither the books
        // nor the queue, so it runs beside the site preparation and the queue's replay
        // (`SIM-02-K`); each gives the result it gives alone.
        let (mut supply, free_places) = rayon::join(
            || {
                let mut supply = SiteSupply::new(
                    &mut inputs,
                    &mut reservations,
                    allocator,
                    zoning,
                    network,
                    graph,
                    logistics,
                    &economy_tuning,
                    &borders,
                    &catalog,
                    minute_of_day,
                    fiscal_policy.household_vat_rate,
                );
                lap(4);
                supply.reserve_projects(
                    self.queued_supply_order
                        .iter()
                        .map(|&index| &self.queued_construction[index]),
                    &catalog,
                    &snapshot.output_absorption,
                    &mut self.construction_execution.replay_market,
                    fiscal_policy,
                    &funded_jobs.service_capacities,
                    &funded_jobs.freight_capital,
                    |index| funded_jobs.saw_operating(index),
                );
                lap(5);
                supply
            },
            || {
                super::workforce::WorkforceReservations::scan_free_places(
                    allocator,
                    &economy_tuning,
                    &snapshot,
                )
            },
        );
        self.plan_private_building_actions(
            slot,
            allocator,
            graph,
            zoning,
            &snapshot,
            funded_jobs,
            catalog.as_ref(),
            economy_tuning.as_ref(),
            fiscal_policy,
            &front.occupants,
            DEMAND_HOURLY_CADENCE_FRACTION,
            environment,
            &mut supply,
            scans,
            Some(free_places),
        );
        self.snapshot_sweep.restore_front(front);
        lap(6);
        reservations.end_live_pass(&mut inputs, allocator, graph);
        crate::debug_log!(
            "economy_phases",
            "demand_preparation_detail carrier_ms={:.3} funding_ms={:.3} reservations_ms={:.3} snapshots_ms={:.3} site_preparation_ms={:.3} queued_ms={:.3} planning_ms={:.3}",
            times[0],
            times[1],
            times[2],
            times[3],
            times[4],
            times[5],
            times[6]
        );
        super::execution::return_live_books(
            inputs,
            &mut reservations,
            &mut live_households.job_funding,
        );
        #[cfg(test)]
        if let Some(before) = before {
            assert_eq!(
                live_fingerprint(&live_households.job_funding),
                before,
                "a planning pass must return the live books exactly"
            );
        }
        self.construction_execution.reservations = reservations;
        self.freight_border_nodes = borders;
        if slot == PlanningSlot::Actions {
            self.admission_budget
                .reserve_worker_homes(self.workforce_reservations.unreserved_ready_homes());
        }
        true
    }

    /// Prepares on `funding`'s books what a planning slot prepares before it plans
    /// (`SIM-02-K`): the border carrier tables, the pass rows, the supplier and buyer site rows
    /// and their layouts, the pass's owner rows and the books' planning storage. Saves keep none
    /// of it, so the first slot to lend each book after loading built it all in one frame; the
    /// live book is prepared at load and a fresh census book once it is finished, in window
    /// slots of its own. The books are lent and restored exactly as a planning slot does, and
    /// what is prepared is derived from the city and keyed by its revisions, so no result
    /// changes. Books not valid for the live `key` at `minute_of_day` (the minute they serve)
    /// are left alone. `part` names what this lending prepares ([`PlanningPreparation`]); a
    /// book stays unprepared for a later call until the whole is. O(B + P) and the books'
    /// storage.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_planning_books(
        &mut self,
        funding: &mut JobFunding,
        key: CensusKey,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        fiscal_policy: &CityFiscalPolicy,
        network: &TransitNetwork,
        logistics: &mut ShipmentSystem,
        minute_of_day: u16,
        part: PlanningPreparation,
    ) {
        let suppliers_only = !matches!(part, PlanningPreparation::Whole { .. });
        // Attempted once per book: an invalid one is left to its first planning slot.
        funding.planning_prepared = !suppliers_only;
        let catalog = Arc::clone(&self.runtime_catalog);
        let economy_tuning = Arc::clone(&self.runtime_tuning);
        let mut borders = std::mem::take(&mut self.freight_border_nodes);
        collect_connected_border_nodes(graph, &mut borders);
        let phase = crate::simulation::economy::logistics::freight_price_phase(
            &economy_tuning,
            minute_of_day,
        );
        if funding.has_current_input_supply(key, &borders, phase) {
            logistics.prepare_site_carrier_quotes(allocator, network, graph, &borders);
            #[cfg(test)]
            let before = VERIFY_LIVE_PASSES
                .load(std::sync::atomic::Ordering::Relaxed)
                .then(|| live_fingerprint(funding));
            let mut reservations = std::mem::take(&mut self.construction_execution.reservations);
            let mut inputs = super::execution::lend_live_books(&mut reservations, funding);
            reservations.begin_live_pass(
                funding,
                &mut inputs,
                allocator,
                graph,
                &catalog,
                &economy_tuning,
                fiscal_policy,
                minute_of_day,
            );
            // Preparing the rows and storage is all a new supply does before its first quote.
            reservations.suppliers_only = suppliers_only;
            reservations.supplier_rows_only = part == PlanningPreparation::SupplierRows;
            SiteSupply::new(
                &mut inputs,
                &mut reservations,
                allocator,
                zoning,
                network,
                graph,
                logistics,
                &economy_tuning,
                &borders,
                &catalog,
                minute_of_day,
                fiscal_policy.household_vat_rate,
            );
            reservations.end_live_pass(&mut inputs, allocator, graph);
            super::execution::return_live_books(inputs, &mut reservations, funding);
            #[cfg(test)]
            if let Some(before) = before {
                assert_eq!(
                    live_fingerprint(funding),
                    before,
                    "preparing the planning books must return them exactly"
                );
            }
            self.construction_execution.reservations = reservations;
            // The census published every buyer's financing, which the hour's first masked
            // business claim would refresh, one buyer at a time, in the residential slot
            // (0.8 ms at 100k). The same refresh here leaves that claim only the buyers changed
            // since. Masks only prune unclaimable buyers, so no result changes (`SIM-02-K`).
            // O(changed buyers' tranches and their bucket paths).
            if part == (PlanningPreparation::Whole { masks: true }) {
                funding.business.refresh_finance_masks();
            }
        }
        self.freight_border_nodes = borders;
    }

    /// Counts the live workforce as a planning pass does, for epochs opened without one.
    #[cfg(test)]
    pub(crate) fn count_workforce_for_test(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        fiscal_policy: &CityFiscalPolicy,
    ) {
        self.workforce_view.count_live(
            allocator,
            households,
            &self.runtime_catalog,
            &self.runtime_tuning,
            fiscal_policy.income_tax_rate,
        );
    }

    // Whether the published census's books are valid for the live city, for a pass or epoch at
    // `minute_of_day` with the connected `borders`: the same building indices and network, the
    // hour's freight price phase and gateways (`SIM-02-I`). O(borders).
    pub(super) fn lent_books_current(
        &self,
        households: &HouseholdSystem,
        network: &TransitNetwork,
        borders: &[u32],
        minute_of_day: u16,
    ) -> bool {
        households.job_funding.has_current_input_supply(
            households.census_key(network),
            borders,
            crate::simulation::economy::logistics::freight_price_phase(
                &self.runtime_tuning,
                minute_of_day,
            ),
        )
    }

    /// Test driver for passes planned without the slot clock, which publishes a census at
    /// every pass's slot (`SIM-02-I`): publishes a census of the live city when the published
    /// one is not current, with the old one's service ceilings.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn publish_census_if_stale_for_test(
        &self,
        allocator: &BuildingAllocator,
        households: &mut HouseholdSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        logistics: &mut ShipmentSystem,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
        minute_of_day: u16,
    ) {
        let mut borders = Vec::new();
        collect_connected_border_nodes(graph, &mut borders);
        if !self.lent_books_current(households, network, &borders, minute_of_day) {
            self.publish_census_for_pass(
                allocator,
                households,
                network,
                graph,
                logistics,
                fiscal_policy,
                treasury_balance,
                minute_of_day,
            );
        }
    }

    // A census of the live city, with the old one's service ceilings.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn publish_census_for_pass(
        &self,
        allocator: &BuildingAllocator,
        households: &mut HouseholdSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        logistics: &mut ShipmentSystem,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
        minute_of_day: u16,
    ) {
        let key = households.census_key(network);
        let previous = std::mem::take(&mut households.job_funding);
        let funding = &mut households.job_funding;
        funding.refresh_with_service_capacity(
            allocator,
            &households.households,
            logistics,
            network,
            graph,
            &self.runtime_catalog,
            &self.runtime_tuning,
            fiscal_policy,
            treasury_balance,
            |index, physical, _| {
                previous
                    .service_capacities
                    .get(index)
                    .copied()
                    .unwrap_or(0)
                    .min(physical)
            },
            Some(&previous),
            minute_of_day,
        );
        funding.set_published(Some(key));
    }

    /// Slot `slot`'s chunk of the hour-long demand snapshot sweep (`SIM-02-I`): a share of the
    /// households and buildings, read with the live census. The caller registers appended
    /// entities' handles first. O(households/960 + buildings/960).
    pub(crate) fn advance_snapshot_sweep(
        &mut self,
        slot: u64,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
    ) {
        self.snapshot_sweep.advance(
            slot,
            &SweepInputs {
                allocator,
                households,
                config: &self.config,
                catalog: &self.runtime_catalog,
                tuning: &self.runtime_tuning,
                policy: fiscal_policy,
                treasury: treasury_balance,
            },
        );
    }

    /// Folds the entities created since the snapshot front's sweep began into it, before the
    /// hour's job search can hire their adults (`SIM-02-I`). O(those entities).
    pub(crate) fn fold_created_snapshot_entities(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
    ) {
        self.snapshot_sweep.top_up_front(&SweepInputs {
            allocator,
            households,
            config: &self.config,
            catalog: &self.runtime_catalog,
            tuning: &self.runtime_tuning,
            policy: fiscal_policy,
            treasury: treasury_balance,
        });
    }

    /// The saved front [`Self::fold_created_snapshot_entities`] would leave now.
    #[cfg(test)]
    pub(crate) fn topped_up_snapshot_front_for_test(
        &self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        fiscal_policy: &CityFiscalPolicy,
    ) -> Option<Vec<u8>> {
        self.snapshot_sweep.topped_up_front_bits(&SweepInputs {
            allocator,
            households,
            config: &self.config,
            catalog: &self.runtime_catalog,
            tuning: &self.runtime_tuning,
            policy: fiscal_policy,
            treasury: 0.0,
        })
    }

    /// Drops the snapshot front, so the next planning slot builds one from the current state:
    /// for subsystem tests that plan on hand-built state without the slot clock (`SIM-02-I`).
    #[cfg(test)]
    pub(crate) fn discard_snapshot_front(&mut self) {
        self.snapshot_sweep.discard_front();
    }

    /// The snapshot sweep's saved form.
    pub(crate) fn saved_snapshot_sweep(&self) -> SavedDemandSnapshot {
        self.snapshot_sweep.save()
    }

    /// Restores a saved snapshot sweep; a malformed buffer is an error.
    pub(crate) fn restore_snapshot_sweep(
        &mut self,
        saved: &SavedDemandSnapshot,
    ) -> Result<(), String> {
        self.snapshot_sweep = DemandSnapshotSweep::restore(saved)?;
        Ok(())
    }

    /// Rebuilds household diagnostics from runtime state without advancing action budgets.
    pub(crate) fn refresh_household_diagnostics(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        treasury_balance: f64,
        fiscal_policy: &CityFiscalPolicy,
    ) {
        // The hour's plan stands: a load or a policy change between its slots keeps it.
        let catalog = Arc::clone(&self.runtime_catalog);
        let economy_tuning = Arc::clone(&self.runtime_tuning);
        let snapshot = DailyDemandSnapshot::from_runtime_with_catalog(
            allocator,
            households,
            graph,
            &self.config,
            catalog.as_ref(),
            economy_tuning.as_ref(),
            fiscal_policy,
            treasury_balance,
        );

        let mut admission_diagnostics = HouseholdAdmissionDiagnostics::from_snapshot(&snapshot);
        admission_diagnostics.planned_households = self.households_to_admit_today;
        admission_diagnostics.incoming_household_need =
            self.last_admission_diagnostics.incoming_household_need;
        self.last_admission_diagnostics = admission_diagnostics;

        let mut removal_diagnostics = HouseholdRemovalDiagnostics::from_snapshot(&snapshot);
        removal_diagnostics.persistent_exit_credit_before = self.persistent_exit_action_credit;
        removal_diagnostics.persistent_exit_credit_after = self.persistent_exit_action_credit;
        removal_diagnostics.max_actionable_households =
            snapshot.persistent_exit_eligible_household_count;
        removal_diagnostics.planned_households = self.households_to_remove_today;
        self.last_removal_diagnostics = removal_diagnostics;
    }

    pub(crate) fn with_persisted_state(
        households_to_admit_today: u32,
        households_to_remove_today: u32,
        persistent_exit_action_credit: f32,
        upgrade_action_credit: [f32; 3],
        downgrade_action_credit: [f32; 3],
        despawn_action_credit: [f32; 3],
        cheat_max_demands_enabled: bool,
    ) -> Self {
        let mut system = Self::new();
        system.households_to_admit_today = households_to_admit_today;
        system.households_to_remove_today = households_to_remove_today;
        system.persistent_exit_action_credit = persistent_exit_action_credit.max(0.0);
        system.upgrade_action_credit = UseTuningF32 {
            residential: upgrade_action_credit[0],
            commercial: upgrade_action_credit[1],
            industrial: upgrade_action_credit[2],
        };
        system.downgrade_action_credit = UseTuningF32 {
            residential: downgrade_action_credit[0],
            commercial: downgrade_action_credit[1],
            industrial: downgrade_action_credit[2],
        };
        system.despawn_action_credit = UseTuningF32 {
            residential: despawn_action_credit[0],
            commercial: despawn_action_credit[1],
            industrial: despawn_action_credit[2],
        };
        if cheat_max_demands_enabled {
            system.enable_max_demand_cheat();
        }
        system
    }

    /// Enables the persistent cheat override that bypasses private construction viability checks.
    pub(crate) fn enable_max_demand_cheat(&mut self) {
        self.cheat_max_demands_enabled = true;
    }

    /// Records how many planned household arrivals actually launched as carriers.
    pub(crate) fn record_household_admission_execution(&mut self, launched_households: u32) {
        self.last_admission_diagnostics.launched_households = launched_households;
    }

    /// Shares the exact departure conditions with execution's live-state recheck.
    pub(crate) fn household_exit_policy(&self) -> HouseholdExitPolicy {
        self.config.household_action.exit_policy
    }

    /// Records how many planned household removals actually left the city.
    pub(crate) fn record_household_removal_execution(&mut self, removed_households: u32) {
        self.last_removal_diagnostics.removed_households = removed_households;
        let removed_household_ratio = if self.last_removal_diagnostics.total_household_count == 0 {
            0.0
        } else {
            clamp01(
                removed_households as f32
                    / self.last_removal_diagnostics.total_household_count as f32,
            )
        };
        self.last_removal_diagnostics.removed_household_ratio = removed_household_ratio;
    }

    /// Accrues one eligible household's share of the daily exit budget at its daily phase and
    /// reports whether it leaves now (`SIM-02-G`): each eligible household adds
    /// `persistent_exit_daily_fraction`, every whole unit of credit releases one departure, and
    /// at most `max_household_removals_per_day` leave per calendar day. O(1).
    pub(crate) fn claim_household_exit(&mut self) -> bool {
        self.exit_eligible_today = true;
        self.persistent_exit_action_credit += self
            .config
            .household_action
            .persistent_exit_daily_fraction
            .max(0.0);
        if self.persistent_exit_action_credit < 1.0
            || self.households_to_remove_today
                >= self.config.action_budget.max_household_removals_per_day
        {
            return false;
        }
        self.persistent_exit_action_credit -= 1.0;
        self.households_to_remove_today += 1;
        true
    }

    /// Ends the calendar day's exit budget: credit lapses when no household was eligible, and
    /// the daily departure limit starts again. O(1).
    pub(crate) fn close_exit_day(&mut self) {
        if !self.exit_eligible_today {
            self.persistent_exit_action_credit = 0.0;
        }
        self.exit_eligible_today = false;
        self.households_to_remove_today = 0;
    }

    /// Plans household-specific departures without rebuilding the business market.
    /// O(households), using the same parallel census and exit policy as hourly diagnostics.
    #[cfg(test)]
    pub(crate) fn run_daily_pass(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
    ) {
        self.building_actions = DemandBuildingActionPlan::default();
        let mut removal_diagnostics = super::snapshot::collect_household_removal_diagnostics(
            allocator,
            households,
            &self.runtime_catalog,
            &self.runtime_tuning,
            &self.config,
        );
        self.households_to_admit_today = 0;
        // Eligibility is local to each household; the shared daily budget only paces exits.
        let persistent_exit_credit_before = self.persistent_exit_action_credit;
        let persistent_exit_removals = advance_persistent_exit_credit(
            &mut self.persistent_exit_action_credit,
            removal_diagnostics.persistent_exit_eligible_household_count,
            self.config.household_action.persistent_exit_daily_fraction,
            self.config.action_budget.max_household_removals_per_day,
        );
        self.households_to_remove_today = persistent_exit_removals;
        removal_diagnostics.persistent_exit_credit_before = persistent_exit_credit_before;
        removal_diagnostics.persistent_exit_credit_after = self.persistent_exit_action_credit;
        removal_diagnostics.persistent_exit_planned_households = persistent_exit_removals;
        removal_diagnostics.max_actionable_households =
            removal_diagnostics.persistent_exit_eligible_household_count;
        removal_diagnostics.planned_households = self.households_to_remove_today;
        removal_diagnostics.removed_households = 0;
        removal_diagnostics.removed_household_ratio = 0.0;
        self.last_removal_diagnostics = removal_diagnostics;
    }
}
