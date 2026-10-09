// SPDX-License-Identifier: GPL-2.0-only

//! Economic permits around allocator-owned physical level changes.

use super::*;
use crate::simulation::buildings::allocator::DemandBuildingActionExecution;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::demand::{DemandBuildingActionPlan, DemandLevelChangeAction};
use crate::simulation::economy::households::output_area_scale_for_building;
use crate::simulation::terrain::TerrainSystem;
use std::sync::Arc;

/// Transaction boundary used by physical execution without transferring allocator ownership.
pub(crate) trait DemandLevelChangeGuard {
    /// A provisional economic proof retained until the allocator reports placement success.
    type Permit;
    /// Checks one resolved incumbent before any live building mutation.
    fn prepare(
        &mut self,
        owner: usize,
        action: &DemandLevelChangeAction,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        logistics: &mut ShipmentSystem,
        zoning: &ZoningSystem,
        treasury: f64,
    ) -> Option<Self::Permit>;
    /// Commits a placed operation or restores the entire rejected proposal.
    fn finish(&mut self, permit: Self::Permit, applied: bool, allocator: &BuildingAllocator);
}

// Mechanical allocator fixtures and populated benchmark setup deliberately test placement
// independently of market admission. Gameplay uses the demand-owned guard below.
impl DemandLevelChangeGuard for () {
    type Permit = ();
    fn prepare(
        &mut self,
        _: usize,
        _: &DemandLevelChangeAction,
        _: &BuildingAllocator,
        _: &HouseholdSystem,
        _: &mut ShipmentSystem,
        _: &ZoningSystem,
        _: f64,
    ) -> Option<()> {
        Some(())
    }
    fn finish(&mut self, _: (), _: bool, _: &BuildingAllocator) {}
}

struct LevelChangeGuard<'a> {
    demand: &'a mut DemandSystem,
    graph: &'a RegionGraph,
    network: &'a TransitNetwork,
    policy: &'a CityFiscalPolicy,
    now: u64,
}

enum LevelChangePermit {
    Unchecked,
    Business {
        epoch: Epoch,
        workers: u32,
        previous: u16,
        area: f32,
        target: u16,
    },
}

impl DemandLevelChangeGuard for LevelChangeGuard<'_> {
    type Permit = LevelChangePermit;

    fn prepare(
        &mut self,
        owner: usize,
        action: &DemandLevelChangeAction,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        logistics: &mut ShipmentSystem,
        zoning: &ZoningSystem,
        treasury: f64,
    ) -> Option<LevelChangePermit> {
        let building = &allocator.buildings[owner];
        if building.zone_type == ZoneType::Residential || self.demand.cheat_max_demands_enabled {
            return Some(LevelChangePermit::Unchecked);
        }
        let minute = (self.now % 1440) as u16;
        let current = epoch(
            allocator,
            self.now / 60,
            self.network.cch_graph.build_generation,
            self.network.cch_graph.metric_revision,
            self.network.frontage_delay_revision,
            crate::simulation::economy::logistics::freight_price_phase(
                &self.demand.runtime_tuning,
                minute,
            ),
            zoning.overlay_revision(),
        );
        self.demand.prepare_execution_market(
            current,
            allocator,
            households,
            self.graph,
            self.now,
            self.network,
            logistics,
            zoning,
            treasury,
            self.policy,
        );
        let catalog = &self.demand.runtime_catalog;
        let tuning = &self.demand.runtime_tuning;
        let previous = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)?;
        let target = candidate_economy_profile(allocator, catalog, &action.target_asset_id)?;
        let level = allocator
            .registry
            .get(&action.target_asset_id)?
            .manifest
            .building
            .as_ref()?
            .level;
        let execution = &mut self.demand.construction_execution;
        let area = output_area_scale_for_building(building, previous);
        let market = execution
            .market
            .as_ref()
            .unwrap()
            .replacement_market(previous, area);
        let reserved = execution.reservations.operations[owner]
            .map_or(building.worker_count, |operation| {
                operation.plan.workers.max(building.worker_count)
            });
        let paid_cargo = if previous.runtime_id == target.runtime_id {
            execution.funding.paid_cargo_capital(owner)
        } else {
            0.0
        };
        logistics.prepare_site_carrier_quotes(
            allocator,
            self.network,
            self.graph,
            &execution.borders,
        );
        let mut supply = SiteSupply::new(
            &mut execution.funding.input_supply,
            &mut execution.reservations,
            allocator,
            zoning,
            self.network,
            self.graph,
            logistics,
            tuning,
            &execution.borders,
            catalog,
            minute,
            self.policy.household_vat_rate,
        );
        let carrier = supply.begin_replacement(owner, building.parcel_id, &action.target_asset_id);
        let prices = supply.prices(catalog, building.zone_type, level, self.policy, carrier);
        if !super::super::spawn_need::replacement_market_covers_operation(
            target,
            catalog,
            market,
            prices,
            tuning.owa_export_price_multiplier,
            building,
            paid_cargo,
            &mut supply,
        ) {
            supply.reject();
            return None;
        }
        let workers = supply
            .supported_workers(market.candidate_workers(target))
            .saturating_sub(reserved);
        if !execution.workforce.can_claim_ready(workers) {
            supply.reject();
            return None;
        }
        Some(LevelChangePermit::Business {
            epoch: current,
            workers,
            previous: previous.runtime_id,
            area,
            target: target.runtime_id,
        })
    }

    fn finish(&mut self, permit: LevelChangePermit, applied: bool, allocator: &BuildingAllocator) {
        let execution = &mut self.demand.construction_execution;
        let LevelChangePermit::Business {
            epoch: checked,
            workers,
            previous,
            area,
            target,
        } = permit
        else {
            if applied {
                execution.epoch = None;
            }
            return;
        };
        if !applied {
            execution.reservations.reject(
                &mut execution.funding.input_supply,
                allocator,
                self.graph,
            );
            return;
        }
        execution
            .reservations
            .accept(&mut execution.funding.input_supply);
        let claimed = execution.workforce.claim_ready(workers);
        debug_assert!(claimed, "placement does not consume the checked workforce");
        let catalog = &self.demand.runtime_catalog;
        execution.market.as_mut().unwrap().reserve_replacement(
            catalog.profile_by_runtime_id(previous).unwrap(),
            area,
            catalog.profile_by_runtime_id(target).unwrap(),
        );
        execution.epoch = Some(epoch(
            allocator, checked.0, checked.3, checked.4, checked.5, checked.6, checked.7,
        ));
    }
}

impl DemandSystem {
    /// Applies immediate actions with a shared economic proof around each business replacement.
    /// A batch prepares live/constructing/queued commitments once; subsequent proposals follow
    /// affected receipt chains and existing route indices. Failed placement consumes no claims.
    pub(crate) fn execute_building_actions(
        &mut self,
        plan: &DemandBuildingActionPlan,
        allocator: &mut BuildingAllocator,
        zoning: &mut ZoningSystem,
        agents: &mut AgentSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury: &mut f64,
        graph: &RegionGraph,
        network: &TransitNetwork,
        terrain: &TerrainSystem,
        now: u64,
        policy: &CityFiscalPolicy,
    ) -> DemandBuildingActionExecution {
        let catalog = Arc::clone(&self.runtime_catalog);
        let tuning = Arc::clone(&self.runtime_tuning);
        let mut guard = LevelChangeGuard {
            demand: self,
            graph,
            network,
            policy,
            now,
        };
        allocator.execute_demand_building_actions(
            plan, zoning, agents, households, logistics, treasury, graph, network, terrain,
            &catalog, &tuning, &mut guard,
        )
    }
}
