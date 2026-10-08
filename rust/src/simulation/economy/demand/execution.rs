// SPDX-License-Identifier: GPL-2.0-only

//! Revalidates queued entry against placed workforce, customers, and finite input commitments.

use super::actions::DemandSpawnAction;
use super::snapshot::DailyDemandSnapshot;
use super::spawn_need::{
    OutputAbsorptionContext, candidate_economy_profile, market_covers_operation_with_input_quotes,
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

type Epoch = (u64, u64, u64, u32, u64, u64, u16);

fn epoch(
    allocator: &BuildingAllocator,
    hour: u64,
    routing: u32,
    metric: u64,
    frontage: u64,
    price_phase: u16,
) -> Epoch {
    (
        hour,
        allocator.building_ref_revision(),
        allocator.entrance_ref_revision(),
        routing,
        metric,
        frontage,
        price_phase,
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
}

/// A checked project consumes its claims only after allocator placement succeeds.
pub(crate) struct QueuedSpawnPermit {
    epoch: Epoch,
    duration: u16,
    workers: u32,
    profile: u16,
    residential: bool,
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
        );
        let mut permit = QueuedSpawnPermit {
            epoch: current,
            duration: 0,
            workers: 0,
            profile: 0,
            residential: zone == ZoneType::Residential,
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
        let execution = &mut self.construction_execution;
        if execution.epoch != Some(current) {
            collect_connected_border_nodes(graph, &mut execution.borders);
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
            let snapshot = DailyDemandSnapshot::from_runtime_with_funding(
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
            execution.workforce.reset(
                allocator,
                &self.runtime_catalog,
                &self.runtime_tuning,
                &snapshot,
                &[],
                0,
            );
            execution.workforce.reserve_existing();
            execution.market = Some(snapshot.output_absorption);
            logistics.prepare_site_carrier_quotes(allocator, network, graph, &execution.borders);
            let mut supply = SiteSupply::new(
                &mut execution.funding.input_supply,
                allocator,
                zoning,
                network,
                graph,
                logistics,
                &self.runtime_tuning,
                &execution.borders, &self.runtime_catalog, (now % 1440) as u16,
            );
            supply.reserve_projects(
                std::iter::empty(),
                &self.runtime_catalog,
                execution.market.as_ref().unwrap(),
                &mut execution.replay_market,
                policy,
                &execution.funding.service_capacities,
                &execution.funding.freight_capital,
            );
            execution.epoch = Some(current);
        }
        let market = execution
            .market
            .as_ref()
            .expect("execution snapshot prepared above");
        logistics.prepare_site_carrier_quotes(allocator, network, graph, &execution.borders);
        let mut supply = SiteSupply::new(
            &mut execution.funding.input_supply,
            allocator,
            zoning,
            network,
            graph,
            logistics,
            &self.runtime_tuning,
            &execution.borders, &self.runtime_catalog, (now % 1440) as u16,
        );
        let carrier_quote = supply.begin(action.parcel_id, &action.asset_id);
        let prices = supply.prices(
            &self.runtime_catalog,
            zone,
            authored.level,
            policy,
            carrier_quote,
        );
        if !market_covers_operation_with_input_quotes(
            profile,
            &self.runtime_catalog,
            market.market(),
            prices,
            self.runtime_tuning.owa_export_price_multiplier,
            |input| supply.quote(input, &self.runtime_catalog, prices),
        ) {
            supply.reject();
            return Err("market-cannot-fund-operation");
        }
        permit.duration = construction_duration_hours(zone, authored.level, &self.runtime_tuning);
        permit.workers = market.candidate_workers(profile);
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

    /// Commits successful placement to the reusable calendar; failed placement consumes nothing.
    pub(crate) fn record_queued_spawn_placement(
        &mut self,
        permit: QueuedSpawnPermit,
        action: &DemandSpawnAction,
        allocator: &BuildingAllocator,
    ) {
        let execution = &mut self.construction_execution;
        if execution.epoch != Some(permit.epoch) || self.cheat_max_demands_enabled {
            execution.epoch = None;
            return;
        }
        if permit.residential {
            // A new home can change which displaced household claims an earlier/later
            // completion. Rebuild lazily at the next business check, once for a home batch.
            if !execution
                .workforce
                .placed_home(allocator, &self.runtime_tuning, action)
            {
                execution.epoch = None;
                return;
            }
        } else {
            execution.funding.input_supply.accept();
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
        ));
    }
}
