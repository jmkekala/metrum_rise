// SPDX-License-Identifier: GPL-2.0-only

//! Explicitly enabled census-input diagnostics, excluded from gameplay and acceptance timing.

use super::*;
use crate::debug_log;

/// Requires an explicit diagnostic request as well as the economy phase log channel.
pub(super) fn enabled() -> bool {
    crate::debug::Channel::EconomyPhases.enabled()
        && std::env::var_os("METRUM_TRACE_FUNDING_INPUTS").is_some()
}

// Selected dependencies locate invalidations; equality is not a production reuse proof.
// Strings keep exact float bits and named fields readable when a row changes. This scan
// deliberately allocates only in opt-in diagnostic tests, never in gameplay builds.
/// Selected dependency rows for locating changes, never authorizing census reuse.
pub(super) struct FundingInputs {
    production_revision: u64,
    production: Vec<(u16, u32, u32, bool, bool, bool, bool, usize)>,
    buildings: Vec<String>,
    households: Vec<String>,
    budgets: Vec<(usize, u16, u64)>,
    freight: Vec<u64>,
    dependencies: String,
    treasury: u64,
}

impl FundingInputs {
    /// Captures exact scalar bits and ordered consumer budgets outside acceptance timing.
    pub(super) fn capture(
        allocator: &BuildingAllocator,
        households: &[Household],
        logistics: &ShipmentSystem,
        network: &TransitNetwork,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        treasury: f64,
        minute: u16,
    ) -> Self {
        let buildings = allocator.buildings.iter().map(|b| format!(
            "profile={} level={} workers={} occupancy={} inactive={:?} construction={} budget={} inventory={:?} area={} activity={} sales={} geometry={:?}",
            b.economy_profile_runtime_id, b.level, b.worker_count, b.occupancy,
            (b.broken, b.economy_broken, b.is_deserted), b.construction_remaining_hours,
            b.operating_budget.to_bits(),
            b.resource_inventory.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            b.work_area_scale.to_bits(), b.commercial_activity_floor_scale.to_bits(),
            b.recent_household_sales_value.to_bits(),
            (b.center_x.to_bits(), b.center_y.to_bits(), b.width_cells, b.depth_cells,
             b.edge_idx, b.frontage_t.to_bits(), b.side_offset.to_bits()),
        )).collect();
        let household_rows = households.iter().map(|h| format!(
            "home={} members={} children={} adults={} employed={} elders={} wages={} car={} unemployment_days={}",
            h.home_building_id, h.member_count, h.child_count, h.adult_count,
            h.employed_adults, h.elder_count, h.gross_daily_wages.to_bits(),
            h.shopping_has_car, h.unemployment_days_elapsed,
        )).collect();
        let mut budgets = Vec::new();
        // Use the actual ordered benefit calculation, including treasury saturation.
        super::super::households::visit_consumer_budgets(
            households,
            allocator,
            catalog,
            tuning,
            fiscal,
            treasury,
            |index, members, budget| budgets.push((index, members, budget.to_bits())),
        );
        let mut freight = vec![0.0; allocator.buildings.len()];
        logistics.write_freight_working_capital(&mut freight);
        Self {
            production_revision: allocator.production_revision(),
            production: allocator
                .buildings
                .iter()
                .map(|building| {
                    (
                        building.economy_profile_runtime_id,
                        building.worker_count,
                        building.work_area_scale.to_bits(),
                        building.broken,
                        building.economy_broken,
                        building.is_deserted,
                        building.is_under_construction(),
                        building.edge_idx,
                    )
                })
                .collect(),
            buildings,
            households: household_rows,
            budgets,
            freight: freight.into_iter().map(f64::to_bits).collect(),
            dependencies: format!(
                "{:?}",
                (
                    allocator.building_ref_revision(),
                    allocator.entrance_ref_revision(),
                    allocator.registry.revision(),
                    network.cch_graph.build_generation,
                    network.cch_graph.metric_revision,
                    network.frontage_delay_revision,
                    network.lane_system.revision(),
                    freight_price_phase(tuning, minute),
                )
            ),
            treasury: treasury.to_bits(),
        }
    }

    fn compare(&self, current: &Self, minute: u16, allocator: &BuildingAllocator) {
        let mut changed_owners = Vec::new();
        let complete =
            allocator.production_changes_since(self.production_revision, &mut changed_owners);
        // This expensive oracle runs only under the explicit diagnostic switch. Road
        // attachment changes are guarded separately by the complete dependency tuple.
        if complete && self.dependencies == current.dependencies {
            assert_eq!(self.production.len(), current.production.len());
            for (owner, (before, after)) in
                self.production.iter().zip(&current.production).enumerate()
            {
                assert!(
                    before == after || changed_owners.binary_search(&owner).is_ok(),
                    "production journal missed owner {owner} at minute {minute}"
                );
            }
        }
        debug_log!(
            "economy_phases",
            "funding_production_changes minute={} complete={} owners={}",
            minute,
            complete,
            changed_owners.len()
        );
        fn changed<T: PartialEq>(before: &[T], after: &[T]) -> usize {
            before.iter().zip(after).filter(|(a, b)| a != b).count()
                + before.len().abs_diff(after.len())
        }
        debug_log!(
            "economy_phases",
            "funding_input_detail minute={} buildings={} households={} budgets={} freight={} dependencies_equal={} treasury_equal={}",
            minute,
            changed(&self.buildings, &current.buildings),
            changed(&self.households, &current.households),
            changed(&self.budgets, &current.budgets),
            changed(&self.freight, &current.freight),
            self.dependencies == current.dependencies,
            self.treasury == current.treasury,
        );
        for (index, (before, after)) in self
            .buildings
            .iter()
            .zip(&current.buildings)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(3)
        {
            debug_log!(
                "economy_phases",
                "funding_input_building minute={} index={} before=[{}] after=[{}]",
                minute,
                index,
                before,
                after
            );
        }
    }
}

impl JobFunding {
    /// Compares selected live-census inputs with execution inputs in opt-in diagnostic tests.
    pub(crate) fn trace_execution_inputs(
        &self,
        allocator: &BuildingAllocator,
        households: &[Household],
        logistics: &ShipmentSystem,
        network: &TransitNetwork,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        fiscal: &CityFiscalPolicy,
        treasury: f64,
        minute: u16,
    ) {
        if enabled()
            && let Some(before) = &self.input_trace
        {
            before.compare(
                &FundingInputs::capture(
                    allocator, households, logistics, network, catalog, tuning, fiscal, treasury,
                    minute,
                ),
                minute,
                allocator,
            );
        }
    }
}
