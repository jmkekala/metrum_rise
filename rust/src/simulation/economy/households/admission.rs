//! Household admission and arrived carrier materialization.

use super::data::{Household, HouseholdSystem};
use super::metrics::household_demand_profile;
use super::replenishment::{REPLENISHMENT_STABLE, stable_replenishment_offset_hours};
use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{AgentSystem, TRANSIT_IN_BUILDING};
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, RuntimeEconomyTuning, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};

impl HouseholdSystem {
    pub(crate) fn admit_immigrant_household(
        &mut self,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        home_building_id: usize,
        member_count: u16,
    ) -> usize {
        let profile = household_demand_profile(catalog);
        let consumption_rate = profile.consumption_rate_per_resident;
        let target_days = profile.stock_target_days;
        let starting_stock_days = target_days.min(tuning.households.immigrant_starting_stock_days);

        let member_count = member_count.max(1);
        self.households.push(Household {
            home_building_id,
            budget: tuning.households.immigrant_starting_budget_per_member * member_count as f32,
            stock: member_count as f32 * consumption_rate * starting_stock_days,
            member_count,
            consumption_rate,
            stock_days: starting_stock_days,
            replenishment_state: REPLENISHMENT_STABLE,
            cooldown_hours: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            pickup_eta_hours: 0,
            stay_failure_days: 0,
            unhoused_days_elapsed: 0,
            unemployment_days_elapsed: 0,
            replenishment_offset_hours: stable_replenishment_offset_hours(
                home_building_id,
                self.households.len() as u32,
            ),
        });
        self.households.len() - 1
    }

    pub(super) fn materialize_arrived_household_carriers(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &BuildingAllocator,
    ) {
        let mut catalog = None;
        let mut tuning = None;
        let mut i = 0;
        while i < agents.len() {
            let pending_size = agents.pending_household_size[i];
            if pending_size == 0 {
                i += 1;
                continue;
            }
            let home = agents.home_building[i];
            if home == usize::MAX
                || home >= allocator.buildings.len()
                || agents.transit[i] != TRANSIT_IN_BUILDING
                || agents.current_building[i] != home
            {
                i += 1;
                continue;
            }
            if catalog.is_none() {
                catalog = Some(load_runtime_economy_catalog().unwrap_or_else(|err| {
                    panic!("could not load built-in economy catalog during carrier arrival: {err}")
                }));
            }
            if tuning.is_none() {
                tuning = Some(load_runtime_economy_tuning().unwrap_or_else(|err| {
                    panic!("could not load built-in economy tuning during carrier arrival: {err}")
                }));
            }
            let catalog_ref = catalog.as_ref().expect("catalog loaded above");
            let tuning_ref = tuning.as_ref().expect("tuning loaded above");
            let household_id =
                self.admit_immigrant_household(catalog_ref, tuning_ref, home, pending_size);
            let home_door = allocator
                .entrances
                .get(home)
                .map(|entrance| entrance.door_pos);

            agents.materialize_household_carrier(i, household_id, home_door);

            for _ in 1..pending_size {
                let (x, y) = home_door
                    .map(|door| (door.x, door.y))
                    .unwrap_or((agents.pos_x[i], agents.pos_y[i]));
                let resident_idx = agents.spawn_housed_agent(home, x, y);
                agents.assign_household_id(resident_idx, household_id);
            }

            debug_log!(
                "economy",
                "household arrival carrier materialized household_id={} size={} home_building={} carrier_agent={}",
                household_id,
                pending_size,
                home,
                i,
            );
            i += 1;
        }
    }
}
