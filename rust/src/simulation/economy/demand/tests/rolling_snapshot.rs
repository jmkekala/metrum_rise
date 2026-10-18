// SPDX-License-Identifier: GPL-2.0-only

//! The demand snapshot swept over one hour of slots (`SIM-02-I`).

use std::ops::Range;

use super::super::snapshot::{
    DemandSnapshotSweep, SnapshotBuffer, SweepInputs, WorkforceView, connected_road_border_count,
};
use super::*;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::households::{
    REPLENISHMENT_SHOPPING_TO_STORE, affordable_consumer_residents,
};
use crate::simulation::economy::logistics::collect_connected_border_nodes;

// A thousand households over six homes, two shops, a factory and two construction sites,
// with registered handles: each chunk of the hour folds one or two households. Some households
// are displaced, families with elders and children claim transfers, and shoppers carry stock.
fn city() -> (BuildingAllocator, HouseholdSystem) {
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "rolling_home", ZoneType::Residential);
    let shop = register_test_asset(&mut allocator, "rolling_shop", ZoneType::Commercial);
    let factory = register_test_asset(&mut allocator, "rolling_factory", ZoneType::Industrial);
    for index in 0..6 {
        let mut home = building(ZoneType::Residential, 0.0, 1 + index % 3, 0, home.clone());
        home.is_deserted = index == 5;
        allocator.buildings.push(home);
    }
    allocator
        .buildings
        .push(building(ZoneType::Commercial, 40.0, 0, 1, shop.clone()));
    allocator
        .buildings
        .push(building(ZoneType::Commercial, 15.0, 0, 0, shop.clone()));
    allocator
        .buildings
        .push(building(ZoneType::Industrial, 0.0, 0, 3, factory));
    for (zone_type, asset_id) in [(ZoneType::Residential, home), (ZoneType::Commercial, shop)] {
        let mut site = building(zone_type, 0.0, 0, 0, asset_id);
        site.set_construction_hours_left(4);
        allocator.buildings.push(site);
    }
    let mut households = HouseholdSystem::new();
    for index in 0..1_000_usize {
        let home = if index % 9 == 4 {
            usize::MAX
        } else {
            index % 6
        };
        let members = 1 + (index % 4) as u16;
        let mut household =
            housed_household(home, members, (index % 5) as f32 * 40.0, (index % 3) as f32);
        if index % 5 == 0 && members > 1 {
            household.adult_count -= 1;
            household.elder_count = 1;
        }
        if index % 7 == 0 && household.adult_count > 1 {
            household.adult_count -= 1;
            household.child_count = 1;
        }
        household.employed_adults = household.adult_count.min((index % 3) as u16);
        household.gross_daily_wages = f32::from(household.employed_adults) * 90.0;
        household.unemployment_days_elapsed = (index % 4) as u32;
        if index % 11 == 0 {
            household.replenishment_state = REPLENISHMENT_SHOPPING_TO_STORE;
            household.reserved_amount = 1.5;
        }
        households.households.push(household);
    }
    households.job_funding.capacities = vec![3; allocator.buildings.len()];
    households.job_funding.set_activity(6, 0.75);
    households.job_funding.set_activity(8, 0.5);
    publish(&mut allocator, &households);
    allocator
        .building_handles
        .register_appended(allocator.buildings.len());
    households
        .handles
        .register_appended(households.households.len());
    (allocator, households)
}

// Copies the census onto the buildings, as its publication does (`SIM-02-I`).
fn publish(allocator: &mut BuildingAllocator, households: &HouseholdSystem) {
    let funding = &households.job_funding;
    for (index, building) in allocator.buildings.iter_mut().enumerate() {
        building.funded_job_capacity = funding.capacities.get(index).copied().unwrap_or(0);
        building.funded_activity = funding.activities().get(index).copied().unwrap_or(0.0);
    }
}

// The transfer claims of the first few hundred households exhaust it.
const TREASURY: f64 = 5_000.0;

fn front_bits(sweep: &DemandSnapshotSweep) -> Option<Vec<u8>> {
    sweep.save().front
}

#[test]
fn an_hour_long_sweep_equals_the_whole_city_snapshot() {
    let (allocator, households) = city();
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().unwrap();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(tuning.as_ref());
    let inputs = SweepInputs {
        allocator: &allocator,
        households: &households,
        config: &config,
        catalog: &catalog,
        tuning: &tuning,
        policy: &policy,
        treasury: TREASURY,
    };
    let hour = 7 * SLOTS_PER_HOUR;
    let mut sweep = DemandSnapshotSweep::default();
    // A sweep that starts mid-hour publishes nothing; the next hour's sweep finishes.
    for slot in hour - SLOTS_PER_HOUR / 3..hour + SLOTS_PER_HOUR - 1 {
        sweep.advance(slot, &inputs);
    }
    assert_eq!(front_bits(&sweep), None);
    sweep.advance(hour + SLOTS_PER_HOUR - 1, &inputs);
    let swept = front_bits(&sweep).expect("a finished sweep publishes its front");

    let front = sweep.take_front(&inputs);
    let composed = composed(&front, &households, &graph, &policy);
    let whole = DailyDemandSnapshot::from_runtime_with_funding(
        &allocator,
        &households,
        &graph,
        &config,
        &catalog,
        &tuning,
        &policy,
        TREASURY,
        &households.job_funding,
    );
    // Handle order is dense order here, and each fold of the whole city is one chunk, so
    // every sum is added in the same order.
    assert_eq!(format!("{composed:?}"), format!("{whole:?}"));
    assert!(!whole.unhoused_workforce.is_empty());
    assert!(whole.open_job_slots > 0 && whole.under_construction_household_slots > 0);
    let unlimited = affordable_consumer_residents(
        &households.households,
        &allocator,
        &catalog,
        &tuning,
        &policy,
        f64::MAX,
    );
    let limited = affordable_consumer_residents(
        &households.households,
        &allocator,
        &catalog,
        &tuning,
        &policy,
        TREASURY,
    );
    assert!(limited < unlimited, "the treasury runs out during the walk");
    assert_same_occupants(&front, &allocator, &households);
    assert!(front.occupants.household_count(&allocator, 0) > 0);
    // Without new entities, taking the front leaves it as it was.
    sweep.restore_front(front);
    assert_eq!(front_bits(&sweep), Some(swept.clone()));
    // A world without a front builds the same one at once.
    let mut built = DemandSnapshotSweep::default();
    let front = built.take_front(&inputs);
    built.restore_front(front);
    assert_eq!(front_bits(&built), Some(swept.clone()));

    // A sweep that misses a slot is abandoned: the pass keeps the last finished front.
    let changed = changed_households(&households, |index, household| {
        household.budget += index as f32;
    });
    let inputs = SweepInputs {
        households: &changed,
        ..inputs
    };
    let next = hour + SLOTS_PER_HOUR;
    for slot in (next..next + SLOTS_PER_HOUR).filter(|&slot| slot != next + 400) {
        sweep.advance(slot, &inputs);
    }
    assert_eq!(front_bits(&sweep), Some(swept));
}

// Planning composes its snapshot with the workforce of the live city: a hire after the sweep
// took the household and the workplace shows at once, as in a snapshot of the whole city built
// now, while the swept workforce still counts the job open and the adult unemployed. The other
// folds stay as the sweep found each entity.
#[test]
fn the_planned_workforce_follows_a_hire_after_the_sweep() {
    let (mut allocator, mut households) = city();
    // The census published before the hour saw the city as it is.
    households
        .job_funding
        .see_operating_for_test(&allocator.buildings);
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().unwrap();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(tuning.as_ref());
    let mut sweep = DemandSnapshotSweep::default();
    let inputs = SweepInputs {
        allocator: &allocator,
        households: &households,
        config: &config,
        catalog: &catalog,
        tuning: &tuning,
        policy: &policy,
        treasury: TREASURY,
    };
    for slot in 0..SLOTS_PER_HOUR {
        sweep.advance(slot, &inputs);
    }
    let worker = households
        .households
        .iter()
        .position(|household| {
            crate::simulation::economy::households::household_is_housed(household, &allocator)
                && household.adult_count > household.employed_adults
        })
        .unwrap();
    let workplace = allocator
        .buildings
        .iter()
        .position(|building| {
            building.zone_type == ZoneType::Commercial
                && !building.is_under_construction()
                && building.funded_job_capacity > building.worker_count
        })
        .unwrap();
    households.households[worker].employed_adults += 1;
    allocator.buildings[workplace].worker_count += 1;

    let inputs = SweepInputs {
        allocator: &allocator,
        households: &households,
        config: &config,
        catalog: &catalog,
        tuning: &tuning,
        policy: &policy,
        treasury: TREASURY,
    };
    let front = sweep.take_front(&inputs);
    let whole = DailyDemandSnapshot::from_runtime_with_funding(
        &allocator,
        &households,
        &graph,
        &config,
        &catalog,
        &tuning,
        &policy,
        TREASURY,
        &households.job_funding,
    );
    let mut borders = Vec::new();
    collect_connected_border_nodes(&graph, &mut borders);
    let workforce = |snapshot: &DailyDemandSnapshot| {
        format!(
            "{} {} {} {} {} {} {} {} {:?}",
            snapshot.existing_unemployed_member_count,
            snapshot.open_job_slots,
            snapshot.move_in_job_slots,
            snapshot.move_in_job_equivalent_slots,
            snapshot.average_move_in_job_wage_per_day,
            snapshot.physical_worker_capacity,
            snapshot.funded_worker_capacity,
            snapshot.open_jobs_unfunded,
            snapshot.unhoused_workforce,
        )
    };
    let compose = |view: &WorkforceView| {
        workforce(&DailyDemandSnapshot::compose(
            &front.totals,
            view,
            &households,
            connected_road_border_count(&graph, &borders),
            &catalog,
            &tuning,
            &policy,
            TREASURY,
        ))
    };
    let mut live = WorkforceView::default();
    live.count_live(
        &allocator,
        &households,
        &catalog,
        &tuning,
        policy.income_tax_rate,
    );
    assert_eq!(compose(&live), workforce(&whole));
    assert_ne!(
        compose(&WorkforceView::of_totals(&front.totals)),
        workforce(&whole)
    );
}

fn composed(
    front: &SnapshotBuffer,
    households: &HouseholdSystem,
    graph: &RegionGraph,
    policy: &CityFiscalPolicy,
) -> DailyDemandSnapshot {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut borders = Vec::new();
    collect_connected_border_nodes(graph, &mut borders);
    DailyDemandSnapshot::compose(
        &front.totals,
        &WorkforceView::of_totals(&front.totals),
        households,
        connected_road_border_count(graph, &borders),
        &catalog,
        &tuning,
        policy,
        TREASURY,
    )
}

fn assert_same_occupants(
    front: &SnapshotBuffer,
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
) {
    let occupants = ResidentialOccupantSnapshot::from_runtime(allocator, households);
    for building_idx in 0..allocator.buildings.len() {
        assert_eq!(
            front.occupants.household_count(allocator, building_idx),
            occupants.household_count(allocator, building_idx)
        );
        assert_eq!(
            front
                .occupants
                .min_reserve_days(allocator, building_idx)
                .to_bits(),
            occupants
                .min_reserve_days(allocator, building_idx)
                .to_bits()
        );
    }
}

#[test]
fn entities_created_after_the_freeze_count_at_the_next_pass() {
    let (mut allocator, mut households) = city();
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().unwrap();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(tuning.as_ref());
    let mut sweep = DemandSnapshotSweep::default();
    let advance = |sweep: &mut DemandSnapshotSweep,
                   slots: Range<u64>,
                   allocator: &BuildingAllocator,
                   households: &HouseholdSystem| {
        let inputs = SweepInputs {
            allocator,
            households,
            config: &config,
            catalog: &catalog,
            tuning: &tuning,
            policy: &policy,
            treasury: TREASURY,
        };
        for slot in slots {
            sweep.advance(slot, &inputs);
        }
    };
    advance(&mut sweep, 0..500, &allocator, &households);
    // A shop opens and a family arrives in new handle slots during the sweep.
    let shop = allocator.buildings[6].clone();
    allocator.buildings.push(shop);
    allocator
        .building_handles
        .register_appended(allocator.buildings.len());
    households
        .households
        .push(housed_household(2, 3, 80.0, 1.0));
    households
        .handles
        .register_appended(households.households.len());
    households.job_funding.capacities.push(3);
    publish(&mut allocator, &households);
    advance(&mut sweep, 500..SLOTS_PER_HOUR, &allocator, &households);

    let inputs = SweepInputs {
        allocator: &allocator,
        households: &households,
        config: &config,
        catalog: &catalog,
        tuning: &tuning,
        policy: &policy,
        treasury: TREASURY,
    };
    let front = sweep.take_front(&inputs);
    let whole = DailyDemandSnapshot::from_runtime_with_funding(
        &allocator,
        &households,
        &graph,
        &config,
        &catalog,
        &tuning,
        &policy,
        TREASURY,
        &households.job_funding,
    );
    assert_eq!(
        format!("{:?}", composed(&front, &households, &graph, &policy)),
        format!("{whole:?}")
    );
    assert_same_occupants(&front, &allocator, &households);
    // The new entities are folded once.
    sweep.restore_front(front);
    let topped_up = sweep.save().front;
    let front = sweep.take_front(&inputs);
    sweep.restore_front(front);
    assert_eq!(sweep.save().front, topped_up);
}

// A copy of `households`' records and handles with `change` applied; the sweeps keep reading
// the original census.
fn changed_households(
    households: &HouseholdSystem,
    mut change: impl FnMut(usize, &mut Household),
) -> HouseholdSystem {
    let mut changed = HouseholdSystem::new();
    changed.households = households.households.clone();
    for (index, household) in changed.households.iter_mut().enumerate() {
        change(index, household);
    }
    changed.handles = households.handles.clone();
    changed
}

#[test]
fn a_saved_sweep_continues_identically() {
    let (allocator, households) = city();
    let config = load_builtin_demand_config().unwrap();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(tuning.as_ref());
    let inputs = SweepInputs {
        allocator: &allocator,
        households: &households,
        config: &config,
        catalog: &catalog,
        tuning: &tuning,
        policy: &policy,
        treasury: TREASURY,
    };
    let mut sweep = DemandSnapshotSweep::default();
    // A finished sweep, then part of the next with the treasury already drawn down.
    for slot in 0..SLOTS_PER_HOUR + 523 {
        sweep.advance(slot, &inputs);
    }
    let saved = sweep.save();
    assert!(saved.front.is_some() && saved.sweep.is_some());
    let mut restored = DemandSnapshotSweep::restore(&saved).unwrap();
    assert_eq!(restored.save(), saved);
    // The rest of the hour reads a changed city and treasury in both.
    let changed = changed_households(&households, |index, household| {
        if index % 3 == 0 {
            household.gross_daily_wages += 25.0;
            household.home_building_id = 4;
        }
    });
    let inputs = SweepInputs {
        households: &changed,
        treasury: 0.0,
        ..inputs
    };
    for slot in SLOTS_PER_HOUR + 523..2 * SLOTS_PER_HOUR {
        sweep.advance(slot, &inputs);
        restored.advance(slot, &inputs);
    }
    let finished = sweep.save();
    assert_ne!(finished.front, saved.front);
    assert_eq!(finished.sweep, None);
    assert_eq!(restored.save(), finished);

    let mut corrupt = saved.clone();
    corrupt.front.as_mut().unwrap().pop();
    assert!(DemandSnapshotSweep::restore(&corrupt).is_err());
}

// A census rebuilt mid-hour, by a load or a policy change, replaces the live operation book but
// not the copies the published census left on the buildings, which the sweep folds (`SIM-02-I`).
#[test]
fn a_census_rebuilt_mid_hour_leaves_the_sweep_unchanged() {
    let (allocator, households) = city();
    let config = load_builtin_demand_config().unwrap();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(tuning.as_ref());
    let inputs = SweepInputs {
        allocator: &allocator,
        households: &households,
        config: &config,
        catalog: &catalog,
        tuning: &tuning,
        policy: &policy,
        treasury: TREASURY,
    };
    // The same households with an empty live census.
    let rebuilt = changed_households(&households, |_, _| {});
    assert!(rebuilt.job_funding.capacities.is_empty());
    let rebuilt_inputs = SweepInputs {
        households: &rebuilt,
        ..inputs
    };
    let (mut sweep, mut other) = (
        DemandSnapshotSweep::default(),
        DemandSnapshotSweep::default(),
    );
    for slot in 0..SLOTS_PER_HOUR {
        sweep.advance(slot, &inputs);
        other.advance(
            slot,
            if slot < SLOTS_PER_HOUR / 2 {
                &inputs
            } else {
                &rebuilt_inputs
            },
        );
    }
    assert!(sweep.save().front.is_some());
    assert_eq!(sweep.save(), other.save());
}

#[test]
fn a_whole_hour_pass_plans_on_the_current_city_instead_of_an_old_front() {
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "rolling_vacant_home", ZoneType::Residential);
    let mut households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();
    let policy = CityFiscalPolicy::from_runtime_tuning(demand.runtime_tuning());
    // A front swept over an hour of a city without homes.
    for slot in 0..SLOTS_PER_HOUR {
        demand.advance_snapshot_sweep(slot, &allocator, &households, &policy, 1_000.0);
    }
    assert!(demand.saved_snapshot_sweep().front.is_some());
    for _ in 0..2 {
        allocator
            .buildings
            .push(building(ZoneType::Residential, 0.0, 0, 0, home.clone()));
    }
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 1_000.0);
    let vacant = demand.last_admission_diagnostics.vacant_household_slots;
    assert!(vacant > 0);
    let config = load_builtin_demand_config().unwrap();
    assert_eq!(
        vacant,
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0)
            .vacant_household_slots
    );
}
