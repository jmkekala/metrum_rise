// SPDX-License-Identifier: GPL-2.0-only

//! Demand system state and pass orchestration.

use super::UseTuningF32;
use super::actions::DemandBuildingActionPlan;
use super::admission::HouseholdAdmissionBudget;
use super::config::{DemandConfig, load_builtin_demand_config};
#[cfg(test)]
use super::credits::advance_persistent_exit_credit;
use super::credits::clamp01;
use super::diagnostics::{
    BuildingActionDiagnosticsByUse, HouseholdAdmissionDiagnostics, HouseholdRemovalDiagnostics,
};
use super::snapshot::{
    DailyDemandSnapshot, DemandSnapshotSweep, SavedDemandSnapshot, SweepInputs,
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
use crate::simulation::economy::households::{HouseholdExitPolicy, HouseholdSystem};
use crate::simulation::economy::logistics::{ShipmentSystem, collect_connected_border_nodes};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::zoning::ZoningSystem;
use std::sync::Arc;

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
    pub(crate) building_actions: DemandBuildingActionPlan,
    pub(super) last_admission_diagnostics: HouseholdAdmissionDiagnostics,
    pub(super) last_removal_diagnostics: HouseholdRemovalDiagnostics,
    pub(super) last_building_action_diagnostics: BuildingActionDiagnosticsByUse,
    /// The demand snapshot folded over the previous hour (`SIM-02-I`); saved.
    pub(super) snapshot_sweep: DemandSnapshotSweep,
    pub(super) workforce_reservations: super::workforce::WorkforceReservations,
    pub(super) construction_execution: super::execution::ConstructionExecution,
    pub(super) queued_construction: Vec<super::queued::QueuedConstruction>,
    pub(super) queued_supply_order: Vec<usize>,
    /// Operational hour relative to which queued completion offsets were prepared.
    pub(super) queued_reference_hour: u64,
    pub(super) next_construction_start_hour: u64,
    pub(super) admission_budget: HouseholdAdmissionBudget,
    freight_border_nodes: Vec<u32>,
    /// Current outside applicant seed, derived from simulation time rather than accepted arrivals.
    pub(crate) admission_candidate_seed: usize,
}

/// Checks after every test-build planning pass that the live books return bit for bit.
/// Long populated-city measurements switch it off; it costs a full fingerprint per pass.
#[cfg(test)]
pub(crate) static VERIFY_LIVE_PASSES: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

// Exchanges the live economic books with the planner's in O(1); a second call returns both.
fn exchange_live_books(
    funding: &mut crate::simulation::economy::hiring::JobFunding,
    reservations: &mut super::supply::ProjectReservations,
    live: &mut crate::simulation::economy::hiring::JobFunding,
) {
    std::mem::swap(&mut funding.input_supply, &mut live.input_supply);
    std::mem::swap(&mut reservations.customers, &mut live.customers);
    std::mem::swap(&mut reservations.business, &mut live.business);
}

#[cfg(test)]
fn live_fingerprint(
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
            last_admission_diagnostics: HouseholdAdmissionDiagnostics::default(),
            last_removal_diagnostics: HouseholdRemovalDiagnostics::default(),
            last_building_action_diagnostics: BuildingActionDiagnosticsByUse::default(),
            snapshot_sweep: DemandSnapshotSweep::default(),
            workforce_reservations: super::workforce::WorkforceReservations::default(),
            construction_execution: super::execution::ConstructionExecution::default(),
            queued_construction: Vec::new(),
            queued_supply_order: Vec::new(),
            queued_reference_hour: 0,
            next_construction_start_hour: 0,
            admission_budget: HouseholdAdmissionBudget::default(),
            freight_border_nodes: Vec::new(),
            admission_candidate_seed: 0,
        }
    }

    /// Returns the cached compiled economy catalog used by demand planning and execution.
    pub(crate) fn runtime_catalog(&self) -> &RuntimeEconomyCatalog {
        self.runtime_catalog.as_ref()
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
        // A whole hour without the slot clock plans on a snapshot of the current state.
        self.discard_snapshot_front();
        self.run_hourly_pass_with_policy(
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

    /// Advances hourly demand using the current fiscal policy and prepared live hiring.
    /// With a current live input census, the pass plans directly on the live customer,
    /// business and input books inside one first-touch transaction and restores them before
    /// returning, so its cost follows the rows it touches rather than a copy of every book.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_hourly_pass_with_policy(
        &mut self,
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
    ) {
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
        self.invalidate_construction_execution();
        self.construction_execution
            .discard_project_proof_for_reset();
        self.building_actions = DemandBuildingActionPlan::default();
        let catalog = Arc::clone(&self.runtime_catalog);
        let economy_tuning = Arc::clone(&self.runtime_tuning);
        // Move reusable gateway scratch out while the planner mutates the other demand state.
        let mut borders = std::mem::take(&mut self.freight_border_nodes);
        collect_connected_border_nodes(graph, &mut borders);
        logistics.prepare_site_carrier_quotes(allocator, network, graph, &borders);
        lap(0);
        let mut funding = std::mem::take(&mut self.construction_execution.funding);
        let mut reservations = std::mem::take(&mut self.construction_execution.reservations);
        let reuse_funding = live_households.job_funding.has_current_input_supply(
            allocator,
            network,
            &borders,
            crate::simulation::economy::logistics::freight_price_phase(
                &economy_tuning,
                minute_of_day,
            ),
        );
        #[cfg(test)]
        let before = (reuse_funding
            && VERIFY_LIVE_PASSES.load(std::sync::atomic::Ordering::Relaxed))
        .then(|| live_fingerprint(&live_households.job_funding));
        if reuse_funding {
            // Lend the live books to the planner; the execution books wait in their place.
            let live = &mut live_households.job_funding;
            exchange_live_books(&mut funding, &mut reservations, live);
            funding
                .service_capacities
                .clone_from(&live.service_capacities);
            funding.freight_capital.clone_from(&live.freight_capital);
        }
        let households = &*live_households;
        if !reuse_funding {
            // Placement/removal or frontage prices can change after the employment pass.
            // Rebuild once after such edits before reusing its funded sources.
            crate::debug::work::add(crate::debug::work::Work::PlannerCensuses, 1);
            funding.refresh_with_service_capacity(
                allocator,
                &households.households,
                logistics,
                network,
                graph,
                &catalog,
                &economy_tuning,
                fiscal_policy,
                treasury_balance,
                |index, physical, _| {
                    households
                        .job_funding
                        .service_capacities
                        .get(index)
                        .copied()
                        .unwrap_or(0)
                        .min(physical)
                },
                Some(&households.job_funding),
                minute_of_day,
            );
        }
        lap(1);
        // Move only the mutable reservation book; the planner reads the matching
        // funding columns without copying them or borrowing the whole owner mutably.
        let mut inputs = std::mem::take(&mut funding.input_supply);
        let funded_jobs = if reuse_funding {
            &households.job_funding
        } else {
            &funding
        };
        if reuse_funding {
            reservations.begin_live_pass(
                funded_jobs,
                &mut inputs,
                allocator,
                &catalog,
                &economy_tuning,
                fiscal_policy,
                minute_of_day,
            );
        } else {
            reservations.reset(
                funded_jobs,
                allocator,
                &catalog,
                &economy_tuning,
                fiscal_policy,
                minute_of_day,
            );
        }
        lap(2);
        // The snapshot swept over the previous hour, with the entities created since
        // (`SIM-02-I`); a world without one builds it now from the census this pass plans with.
        let front = self.snapshot_sweep.take_front(&SweepInputs {
            allocator,
            households,
            job_funding: funded_jobs,
            config: &self.config,
            catalog: catalog.as_ref(),
            tuning: economy_tuning.as_ref(),
            policy: fiscal_policy,
            treasury: treasury_balance,
        });
        let mut snapshot = DailyDemandSnapshot::compose(
            &front.totals,
            households,
            connected_road_border_count(graph, &borders),
            catalog.as_ref(),
            economy_tuning.as_ref(),
            fiscal_policy,
            treasury_balance,
        );
        snapshot.reserve_queued_construction(catalog.as_ref(), &self.queued_construction);

        self.admission_budget = HouseholdAdmissionBudget::from_snapshot(&snapshot, &self.config);
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
        self.last_admission_diagnostics = admission_diagnostics;
        lap(3);
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
            &funding.service_capacities,
            &funding.freight_capital,
        );
        lap(5);
        self.plan_private_building_actions(
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
            supply,
        );
        self.snapshot_sweep.restore_front(front);
        lap(6);
        if reuse_funding {
            reservations.end_live_pass(&mut inputs, allocator, graph);
        }
        crate::debug_log!(
            "economy_phases",
            "demand_preparation_detail reuse_funding={} carrier_ms={:.3} funding_ms={:.3} reservations_ms={:.3} snapshots_ms={:.3} site_preparation_ms={:.3} queued_ms={:.3} planning_ms={:.3}",
            reuse_funding,
            times[0],
            times[1],
            times[2],
            times[3],
            times[4],
            times[5],
            times[6]
        );
        funding.input_supply = inputs;
        if reuse_funding {
            exchange_live_books(
                &mut funding,
                &mut reservations,
                &mut live_households.job_funding,
            );
        }
        #[cfg(test)]
        if let Some(before) = before {
            assert_eq!(
                live_fingerprint(&live_households.job_funding),
                before,
                "a planning pass must return the live books exactly"
            );
        }
        self.construction_execution.reservations = reservations;
        self.construction_execution.funding = funding;
        self.freight_border_nodes = borders;
        self.admission_budget
            .reserve_worker_homes(self.workforce_reservations.unreserved_ready_homes());
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
                job_funding: &households.job_funding,
                config: &self.config,
                catalog: &self.runtime_catalog,
                tuning: &self.runtime_tuning,
                policy: fiscal_policy,
                treasury: treasury_balance,
            },
        );
    }

    /// Drops the snapshot front, so the next hourly pass builds one from the current state: for
    /// callers that step a whole hour without the slot clock (`SIM-02-I`).
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
        self.building_actions = DemandBuildingActionPlan::default();
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
