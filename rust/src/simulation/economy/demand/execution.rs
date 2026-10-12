// SPDX-License-Identifier: GPL-2.0-only

//! Revalidates physical demand execution against shared workforce, customer and input commitments.

use super::actions::DemandSpawnAction;
use super::snapshot::DailyDemandSnapshot;
use super::spawn_need::{
    OutputAbsorptionContext, candidate_economy_profile, project_covers_operation,
};
use super::supply::{ProjectSupply, SiteSupply};
use super::system::DemandSystem;
use super::workforce::WorkforceReservations;
use crate::simulation::buildings::allocator::{BuildingAllocator, construction_duration_hours};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::{ShipmentSystem, collect_connected_border_nodes};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::zoning::{ZoneType, ZoningSystem};

mod level_change;
pub(crate) use level_change::DemandLevelChangeGuard;

type Epoch = (u64, u64, u64, u32, u64, u64, u16, u64, u64);

fn epoch(
    allocator: &BuildingAllocator,
    hour: u64,
    routing: u32,
    metric: u64,
    frontage: u64,
    price_phase: u16,
    parcels: u64,
) -> Epoch {
    (
        hour,
        allocator.building_ref_revision(),
        allocator.entrance_ref_revision(),
        routing,
        metric,
        frontage,
        price_phase,
        parcels,
        allocator.registry.revision(),
    )
}

/// Derived execution scratch. It is rebuilt after load rather than saved.
#[derive(Default)]
pub(super) struct ConstructionExecution {
    epoch: Option<Epoch>,
    workforce: WorkforceReservations,
    market: Option<OutputAbsorptionContext>,
    pub(super) replay_market: OutputAbsorptionContext,
    borders: Vec<u32>,
    pub(super) funding: crate::simulation::economy::hiring::JobFunding,
    pub(super) reservations: super::supply::ProjectReservations,
    // The epoch census's customer and business books are lent to `reservations` inside a
    // pass instead of being copied; they return, restored, before the next census.
    books_lent: bool,
    #[cfg(test)]
    lent_fingerprint: Option<[Vec<(&'static str, u64)>; 2]>,
}

impl ConstructionExecution {
    /// Ends a dropped permit's provisional proof before a full live-market rebuild.
    /// Old input source indices can be invalid; customer rollback uses retained geometry.
    pub(super) fn discard_project_proof_for_reset(&mut self) {
        self.funding.input_supply.discard_project_proof_for_reset();
        self.reservations.business.reject_revalidation();
        self.reservations.customers.reject_revalidation();
        if std::mem::take(&mut self.books_lent) {
            self.reservations.end_book_pass();
            std::mem::swap(&mut self.funding.customers, &mut self.reservations.customers);
            std::mem::swap(&mut self.funding.business, &mut self.reservations.business);
            #[cfg(test)]
            if let Some(lent) = self.lent_fingerprint.take() {
                assert_eq!([self.funding.customers.live_fingerprint_for_test(),
                    self.funding.business.live_fingerprint_for_test()], lent,
                    "an execution epoch must return its census books exactly");
            }
        }
    }
}

/// A checked project consumes its claims only after allocator placement succeeds.
pub(crate) struct QueuedSpawnPermit {
    epoch: Epoch,
    duration: u16,
    workers: u32,
    profile: u16,
    residential: bool,
    reserved: bool,
}

impl DemandSystem {
    /// Invalidates workforce scratch for role/policy changes outside indexed building edits.
    pub(crate) fn invalidate_construction_execution(&mut self) {
        self.construction_execution.epoch = None;
    }

    /// Checks placed homes and unemployed adults before spending a queued project's equity.
    /// Prepares the snapshot/calendar and live input market once per active queue hour or
    /// external layout/metric/frontage-price revision. Successful appends update scratch incrementally.
    /// Warm checks cost O(ports² + log H + K log K + G + S), for K borders, local door/lane
    /// geometry G and source-query work S. There is no per-project city scan.
    pub(crate) fn prepare_queued_spawn(
        &mut self,
        action: &DemandSpawnAction,
        zone: ZoneType,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        now: u64,
        network: &TransitNetwork,
        logistics: &mut ShipmentSystem,
        zoning: &ZoningSystem,
        treasury: f64,
        policy: &CityFiscalPolicy,
    ) -> Result<QueuedSpawnPermit, &'static str> {
        // Execution at an exact boundary precedes the construction/hourly economy tick.
        let current = epoch(
            allocator,
            now.saturating_sub(1) / 60,
            network.cch_graph.build_generation,
            network.cch_graph.metric_revision,
            network.frontage_delay_revision,
            crate::simulation::economy::logistics::freight_price_phase(&self.runtime_tuning, (now % 1440) as u16),
            zoning.overlay_revision(),
        );
        let mut permit = QueuedSpawnPermit {
            epoch: current,
            duration: 0,
            workers: 0,
            profile: 0,
            residential: zone == ZoneType::Residential,
            reserved: false,
        };
        if permit.residential || self.cheat_max_demands_enabled {
            return Ok(permit);
        }
        let authored = allocator
            .registry
            .get(&action.asset_id)
            .and_then(|entry| entry.manifest.building.as_ref())
            .ok_or("missing-asset")?;
        let profile = candidate_economy_profile(allocator, &self.runtime_catalog, &action.asset_id)
            .ok_or("missing-economy-profile")?;
        let duration = construction_duration_hours(zone, authored.level, &self.runtime_tuning);
        let level = authored.level;
        let profile_id = profile.runtime_id;
        if let Ok(index) = self.queued_construction.binary_search_by_key(&action.parcel_id,
            |project| project.parcel_id) {
            let project = &mut self.queued_construction[index];
            if project.active {
                let entrance = allocator.demand_site_entrance(action.parcel_id, &action.asset_id,
                    zoning, graph, &network.lane_system);
                if project.entrance != entrance || project.profile_runtime_id != profile_id
                    || project.level != level {
                    project.entrance = entrance;
                    project.profile_runtime_id = profile_id;
                    project.level = level;
                    project.commercial = zone == ZoneType::Commercial;
                    project.completion_hour = current.0.saturating_add(u64::from(duration))
                        .saturating_sub(self.queued_reference_hour);
                    self.construction_execution.epoch = None;
                }
            }
        }
        self.prepare_execution_market(current, allocator, households, graph, now,
            network, logistics, zoning, treasury, policy);
        let profile = self.runtime_catalog.profile_by_runtime_id(profile_id).unwrap();
        let execution = &mut self.construction_execution;
        execution.reservations.reject(&mut execution.funding.input_supply, allocator, graph);
        if self.queued_construction.binary_search_by_key(&action.parcel_id,
            |project| project.parcel_id).is_ok_and(|index| self.queued_construction[index].active) {
            let operation = execution.reservations.project_operation(action.parcel_id, zoning)
                .filter(|operation| operation.profile == profile_id)
                .ok_or("queued-operation-not-supported")?;
            // This epoch has already proved the entire queue against placed housing and
            // reserved each supported workforce. Requoting as a new owner would duplicate
            // its customers and inputs or withdraw demand supporting another queued firm.
            permit.duration = duration;
            permit.workers = operation.plan.workers;
            permit.profile = profile_id;
            permit.reserved = true;
            return Ok(permit);
        }
        let market = execution
            .market
            .as_ref()
            .expect("execution snapshot prepared above");
        logistics.prepare_site_carrier_quotes(allocator, network, graph, &execution.borders);
        let mut supply = SiteSupply::new(
            &mut execution.funding.input_supply,
            &mut execution.reservations,
            allocator,
            zoning,
            network,
            graph,
            logistics,
            &self.runtime_tuning,
            &execution.borders, &self.runtime_catalog, (now % 1440) as u16, policy.household_vat_rate,
        );
        let carrier_quote = supply.begin(action.parcel_id, &action.asset_id);
        let prices = supply.prices(
            &self.runtime_catalog,
            zone,
            level,
            policy,
            carrier_quote,
        );
        if !project_covers_operation(
            profile,
            &self.runtime_catalog,
            market.market(),
            prices,
            self.runtime_tuning.owa_export_price_multiplier,
            &mut supply,
        ) {
            supply.reject();
            return Err("market-cannot-fund-operation");
        }
        permit.duration = duration;
        permit.workers = supply.supported_workers(market.candidate_workers(profile));
        permit.profile = profile.runtime_id;
        if !execution
            .workforce
            .can_claim(permit.duration, permit.workers)
        {
            supply.reject();
            return Err("workforce-not-ready-by-completion");
        }
        Ok(permit)
    }

    // One census per execution epoch; successful actions keep the books and owner namespace.
    fn prepare_execution_market(
        &mut self, current: Epoch, allocator: &BuildingAllocator,
        households: &HouseholdSystem, graph: &RegionGraph, now: u64, network: &TransitNetwork,
        logistics: &mut ShipmentSystem, zoning: &ZoningSystem, treasury: f64, policy: &CityFiscalPolicy,
    ) {
        let execution = &mut self.construction_execution;
        if execution.epoch != Some(current) {
            // A dropped placement permit can retain provisional inner commits. The new
            // epoch rebuilds inputs from live state; its old source IDs may no longer exist.
            execution.discard_project_proof_for_reset();
            collect_connected_border_nodes(graph, &mut execution.borders);
            crate::debug::work::add(crate::debug::work::Work::ExecutionCensuses, 1);
            execution.funding.refresh_with_service_capacity(
                allocator,
                &households.households,
                logistics,
                network,
                graph,
                &self.runtime_catalog,
                &self.runtime_tuning,
                policy,
                treasury,
                |index, physical, _| {
                    households
                        .job_funding
                        .service_capacities
                        .get(index)
                        .copied()
                        .unwrap_or(0)
                        .min(physical)
                }, (now % 1440) as u16,
            );
            let mut snapshot = DailyDemandSnapshot::from_runtime_with_funding(
                allocator,
                households,
                graph,
                &self.config,
                &self.runtime_catalog,
                &self.runtime_tuning,
                policy,
                treasury,
                &execution.funding,
            );
            snapshot.reserve_queued_construction(&self.runtime_catalog, &self.queued_construction);
            #[cfg(test)]
            if super::system::VERIFY_LIVE_PASSES.load(std::sync::atomic::Ordering::Relaxed) {
                execution.lent_fingerprint = Some([execution.funding.customers.live_fingerprint_for_test(),
                    execution.funding.business.live_fingerprint_for_test()]);
            }
            // Lend the fresh census books to the epoch instead of copying them.
            std::mem::swap(&mut execution.funding.customers, &mut execution.reservations.customers);
            std::mem::swap(&mut execution.funding.business, &mut execution.reservations.business);
            execution.reservations.begin_book_pass(&execution.funding, allocator, &self.runtime_catalog,
                &self.runtime_tuning, policy, (now % 1440) as u16);
            execution.books_lent = true;
            logistics.prepare_site_carrier_quotes(allocator, network, graph, &execution.borders);
            let mut supply = SiteSupply::new(
                &mut execution.funding.input_supply,
                &mut execution.reservations,
                allocator,
                zoning,
                network,
                graph,
                logistics,
                &self.runtime_tuning,
                &execution.borders, &self.runtime_catalog, (now % 1440) as u16, policy.household_vat_rate,
            );
            let queue_order = self.queued_supply_order.as_slice();
            supply.reserve_execution_projects(
                queue_order.iter().map(|&index| &self.queued_construction[index]),
                &self.runtime_catalog,
                &snapshot,
                &mut execution.replay_market,
                policy,
                &execution.funding.service_capacities,
                &execution.funding.freight_capital,
                &mut execution.workforce,
                current.0.saturating_sub(self.queued_reference_hour),
            );
            execution.market = Some(snapshot.output_absorption);
            execution.epoch = Some(current);
        }
    }

    /// Commits successful placement to the reusable calendar; failed placement consumes nothing.
    pub(crate) fn record_queued_spawn_placement(
        &mut self,
        permit: QueuedSpawnPermit,
        action: &DemandSpawnAction,
        allocator: &BuildingAllocator,
    ) {
        self.retire_queued_construction(action.parcel_id);
        let execution = &mut self.construction_execution;
        if execution.epoch != Some(permit.epoch) || self.cheat_max_demands_enabled {
            execution.epoch = None;
            return;
        }
        if permit.residential {
            // Placed housing can now finance previously unsupported queued workforces.
            // Rebuild lazily once after the home batch, preserving queue priority.
            execution.epoch = None;
            return;
        } else if !permit.reserved {
            execution.reservations.accept(&mut execution.funding.input_supply);
            let claimed = execution.workforce.claim(permit.duration, permit.workers);
            debug_assert!(claimed, "placement cannot change the prepared workforce");
            if let Some(profile) = self.runtime_catalog.profile_by_runtime_id(permit.profile) {
                execution
                    .market
                    .as_mut()
                    .expect("prepared market")
                    .reserve_candidate(profile);
            }
        }
        execution.epoch = Some(epoch(
            allocator,
            permit.epoch.0,
            permit.epoch.3,
            permit.epoch.4,
            permit.epoch.5,
            permit.epoch.6,
            permit.epoch.7,
        ));
    }

    /// Current execution-epoch proof of a queued or constructing project, if prepared.
    #[cfg(test)]
    pub(crate) fn project_proof_for_test(&self, parcel: u64, zoning: &ZoningSystem)
        -> Option<(u32, [u64; 2], Vec<(u16, Option<usize>, u64, u32, u32)>)> {
        let execution = &self.construction_execution;
        execution.reservations.project_proof_for_test(&execution.funding.input_supply, parcel,
            zoning, &self.runtime_catalog, &self.runtime_tuning)
    }

    /// Removes a failed queue entry and ends its provisional proof before later execution.
    /// The next check revalidates commitments which may have depended on this customer.
    pub(crate) fn reject_queued_spawn(&mut self, action: &DemandSpawnAction,
        allocator: &BuildingAllocator, graph: &RegionGraph) {
        let retired = self.retire_queued_construction(action.parcel_id);
        let execution = &mut self.construction_execution;
        execution.reservations.reject(&mut execution.funding.input_supply, allocator, graph);
        if let Some(profile) = retired.and_then(|id| self.runtime_catalog.profile_by_runtime_id(id))
            && let Some(market) = &mut execution.market {
            market.release_candidate(profile, 1.0);
        }
        if retired.is_some() && execution.reservations.queued_workers
            .binary_search_by_key(&action.parcel_id, |&(parcel, _)| parcel)
            .is_ok_and(|index| execution.reservations.queued_workers[index].1 > 0) {
            execution.epoch = None;
        }
    }
}
