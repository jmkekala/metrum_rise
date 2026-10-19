// SPDX-License-Identifier: GPL-2.0-only

//! Economic permits around allocator-owned physical level changes.

use super::super::supply::ProjectSupply;
use super::*;
#[cfg(test)]
use crate::simulation::buildings::allocator::DemandBuildingActionExecution;
use crate::simulation::core::handles::Handle;
#[cfg(test)]
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
    #[allow(clippy::too_many_arguments)]
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
    /// Ends the proofs after the last level change, before any despawn removes a building.
    fn close(&mut self, households: &mut HouseholdSystem, allocator: &BuildingAllocator);
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
    fn close(&mut self, _: &mut HouseholdSystem, _: &BuildingAllocator) {}
}

struct LevelChangeGuard<'a> {
    demand: &'a mut DemandSystem,
    graph: &'a RegionGraph,
    network: &'a TransitNetwork,
    policy: &'a CityFiscalPolicy,
    minute: u16,
}

enum LevelChangePermit {
    Unchecked,
    Business {
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
        _: f64,
    ) -> Option<LevelChangePermit> {
        let building = &allocator.buildings[owner];
        if building.zone_type == ZoneType::Residential || self.demand.cheat_max_demands_enabled {
            return Some(LevelChangePermit::Unchecked);
        }
        // Business level changes are proved in the planning slot's epoch only.
        if !self.demand.construction_execution.open {
            return None;
        }
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
            households.job_funding.paid_cargo_capital(owner)
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
            &mut execution.book,
            &mut execution.reservations,
            allocator,
            zoning,
            self.network,
            self.graph,
            logistics,
            tuning,
            &execution.borders,
            catalog,
            self.minute,
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
            workers,
            previous: previous.runtime_id,
            area,
            target: target.runtime_id,
        })
    }

    fn finish(&mut self, permit: LevelChangePermit, applied: bool, allocator: &BuildingAllocator) {
        let execution = &mut self.demand.construction_execution;
        let LevelChangePermit::Business {
            workers,
            previous,
            area,
            target,
        } = permit
        else {
            return;
        };
        if !applied {
            execution
                .reservations
                .reject(&mut execution.book, allocator, self.graph);
            return;
        }
        execution.changed = true;
        execution.reservations.accept(&mut execution.book);
        let claimed = execution.workforce.claim_ready(workers);
        debug_assert!(claimed, "placement does not consume the checked workforce");
        let catalog = &self.demand.runtime_catalog;
        execution.market.as_mut().unwrap().reserve_replacement(
            catalog.profile_by_runtime_id(previous).unwrap(),
            area,
            catalog.profile_by_runtime_id(target).unwrap(),
        );
    }

    fn close(&mut self, households: &mut HouseholdSystem, allocator: &BuildingAllocator) {
        self.demand
            .close_construction_epoch(households, allocator, self.graph);
    }
}

/// A level change the hour's actions slot proved, waiting for the `level changes` queue to
/// apply it (`SIM-02-J`). Saved in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QueuedLevelChange {
    /// Slot that queued it.
    pub(crate) slot: u64,
    /// The building it replaces in place.
    pub(crate) building: Handle<Building>,
    /// The asset one level up or down.
    pub(crate) target_asset_id: String,
    /// Digest of the building state the change was proved for ([`level_change_permit`]).
    pub(crate) permit: u64,
}

/// Why a queued level change was not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LevelChangeRefusal {
    /// The building left.
    Gone,
    /// The building no longer has the door, profile, level or asset the change was proved for.
    StalePermit,
}

impl DemandSystem {
    /// Proves the hour plan's level changes in the actions slot's construction epoch and
    /// queues those proved, with their permits, for the `level changes` queue (`SIM-02-J`).
    /// Business changes need the epoch open; residential ones need no proof. The epoch closes
    /// before it returns. O(changes × proof).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn queue_level_changes(
        &mut self,
        plan: &DemandBuildingActionPlan,
        slot: u64,
        allocator: &mut BuildingAllocator,
        zoning: &ZoningSystem,
        households: &mut HouseholdSystem,
        logistics: &mut ShipmentSystem,
        treasury: f64,
        graph: &RegionGraph,
        network: &TransitNetwork,
        terrain: &TerrainSystem,
        now: u64,
        policy: &CityFiscalPolicy,
    ) {
        let catalog = Arc::clone(&self.runtime_catalog);
        debug_assert!(
            self.construction_execution.open
                || [&plan.commercial, &plan.industrial]
                    .iter()
                    .all(|plan| plan.downgrades.is_empty() && plan.upgrades.is_empty()),
            "business level changes are proved in the planning slot's construction epoch"
        );
        let mut proved = Vec::new();
        let mut guard = LevelChangeGuard {
            demand: self,
            graph,
            network,
            policy,
            minute: (now % 1440) as u16,
        };
        allocator.prove_demand_level_changes(
            plan,
            zoning,
            households,
            logistics,
            treasury,
            graph,
            network,
            terrain,
            &catalog,
            &mut guard,
            &mut proved,
        );
        allocator
            .building_handles
            .register_appended(allocator.buildings.len());
        for (building_idx, action) in proved {
            self.level_change_requests.push_back(QueuedLevelChange {
                slot,
                building: allocator.building_handles.handle(building_idx),
                target_asset_id: action.target_asset_id.clone(),
                permit: level_change_permit(
                    &allocator.buildings[building_idx],
                    Some(&allocator.derived_entrance(building_idx, graph, &network.lane_system)),
                    graph,
                    &network.lane_system,
                ),
            });
        }
    }

    /// The building queued level change `request` may replace now: it still stands with the
    /// door, profile, level and asset the change was proved for (a home's door does not
    /// count), its door derived now. Nothing is proved again. O(door derivation + asset id).
    pub(crate) fn check_level_change_permit(
        &self,
        request: &QueuedLevelChange,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        lanes: &LaneSystem,
    ) -> Result<usize, LevelChangeRefusal> {
        let building_idx = allocator
            .building_handles
            .resolve(request.building)
            .ok_or(LevelChangeRefusal::Gone)?;
        let permit = level_change_permit(
            &allocator.buildings[building_idx],
            Some(&allocator.derived_entrance(building_idx, graph, lanes)),
            graph,
            lanes,
        );
        if permit != request.permit && !self.cheat_max_demands_enabled {
            return Err(LevelChangeRefusal::StalePermit);
        }
        Ok(building_idx)
    }

    /// Test driver: applies `plan`'s level changes and despawns at once, as the actions slot
    /// did before the `level changes` and `building removals` queues (`SIM-02-J`). Business
    /// level changes are proved in the planning slot's construction epoch
    /// ([`Self::open_construction_epoch`]), which every level change shares and which closes
    /// before the despawns. Failed placement consumes no claims. A business level change
    /// without an open epoch is rejected.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
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
        debug_assert!(
            self.construction_execution.open
                || [&plan.commercial, &plan.industrial]
                    .iter()
                    .all(|plan| plan.downgrades.is_empty() && plan.upgrades.is_empty()),
            "business level changes are proved in the planning slot's construction epoch"
        );
        let mut guard = LevelChangeGuard {
            demand: self,
            graph,
            network,
            policy,
            minute: (now % 1440) as u16,
        };
        allocator.execute_demand_building_actions(
            plan, zoning, agents, households, logistics, treasury, graph, network, terrain,
            &catalog, &tuning, &mut guard,
        )
    }

    /// Test driver of a planning slot's immediate actions on the census `households`
    /// publishes: counts the workforce, opens the construction epoch and runs `plan` in it, as
    /// the hour's first slot does after its permits (`SIM-02-I`).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_building_actions_for_test(
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
        self.workforce_view.count_live(
            allocator,
            households,
            &self.runtime_catalog,
            &self.runtime_tuning,
            policy.income_tax_rate,
        );
        // Fixtures build their census with `JobFunding::refresh` at a minute of their own.
        self.lend_construction_epoch(
            allocator, households, zoning, graph, network, logistics, policy, *treasury, now,
        );
        let execution = self.execute_building_actions(
            plan, allocator, zoning, agents, households, logistics, treasury, graph, network,
            terrain, now, policy,
        );
        self.close_construction_epoch(households, allocator, graph);
        execution
    }
}
