// SPDX-License-Identifier: GPL-2.0-only

//! Housing eligibility, relocation, eviction, and removal tests.

use super::support::*;
use super::*;
use crate::simulation::economy::demand::DemandSystem;

fn exit_policy() -> super::super::HouseholdExitPolicy {
    DemandSystem::new().household_exit_policy()
}

#[test]
fn household_building_remaps_preserve_simultaneous_home_and_store_moves() {
    use std::collections::HashMap;
    for mapping in [
        HashMap::new(),
        HashMap::from([(3, 5)]),
        HashMap::from([(3, 5), (5, 3)]),
    ] {
        let mut households = HouseholdSystem::new();
        for home in [3, 5, 9, usize::MAX] {
            for store in [3, 5, 9, usize::MAX] {
                let mut family = make_household(home, 1, 1.0, 1.0);
                family.budget = 37.0;
                family.reserved_store_building_id = store;
                households.households.push(family);
            }
        }
        let expected: Vec<_> = households
            .households
            .iter()
            .map(|family| {
                [family.home_building_id, family.reserved_store_building_id]
                    .map(|id| mapping.get(&id).copied().unwrap_or(id))
            })
            .collect();
        households.remap_building_indices(&mapping);
        for (family, expected) in households.households.iter().zip(expected) {
            assert_eq!(
                [family.home_building_id, family.reserved_store_building_id],
                expected
            );
            assert_eq!(family.budget, 37.0);
        }
    }
}

#[test]
fn hourly_rehousing_claims_affordable_homes_without_advancing_daily_failures() {
    let mut allocator = BuildingAllocator::new();
    let asset =
        register_test_residential_asset_with_capacity(&mut allocator, "test", "rehousing", 1);
    for (index, level) in [3, 1, 1].into_iter().enumerate() {
        let mut home = make_building(index as f32 * 20.0, ZoneType::Residential, &asset, 0.0);
        home.level = level;
        home.occupancy = u32::from(index == 2);
        allocator.buildings.push(home);
    }
    allocator.rebuild_zone_index();
    let mut households = HouseholdSystem::new();
    let mut agents = AgentSystem::new();
    for (hid, (home, reserves)) in [(usize::MAX, 1.0), (usize::MAX, 0.0), (2, 0.0)]
        .into_iter()
        .enumerate()
    {
        let mut household = make_household(home, 1, reserves, 1.0);
        household.unhoused_days_elapsed = 3;
        household.stay_failure_days = 1;
        households.households.push(household);
        let agent = agents.spawn_housed_agent(home, 0.0, 0.0);
        agents.assign_household_id(agent, hid);
    }
    households.recount_every_household(&agents, &allocator);
    households.rehouse_unhoused_before_growth(&mut agents, &mut allocator);
    assert_eq!(households.households[0].home_building_id, 1);
    assert_eq!(agents.home_building[0], 1);
    assert_eq!(allocator.buildings[1].occupancy, 1);
    assert_eq!(households.households[0].unhoused_days_elapsed, 0);
    assert_eq!(households.households[1].home_building_id, usize::MAX);
    assert_eq!(households.households[1].unhoused_days_elapsed, 3);
    assert_eq!(households.households[2].home_building_id, 2);
    assert_eq!(households.households[2].stay_failure_days, 1);
    assert_eq!(
        allocator.first_rehousing_vacancy(|_| true),
        Some(0),
        "unaffordable luxury space stays available to other households"
    );

    households.rehouse_unhoused_before_growth(&mut agents, &mut allocator);
    assert_eq!(
        allocator.buildings[1].occupancy, 1,
        "one household cannot claim twice"
    );
    assert_eq!(households.households[1].unhoused_days_elapsed, 3);
    let rich = make_household(usize::MAX, 1, 20.0, 1.0);
    households.households.push(rich);
    let agent = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
    agents.assign_household_id(agent, 3);
    households.recount_every_household(&agents, &allocator);
    households.rehouse_unhoused_before_growth(&mut agents, &mut allocator);
    assert_eq!(households.households[3].home_building_id, 0);
    assert_eq!(allocator.first_rehousing_vacancy(|_| true), None);
}

#[test]
#[ignore = "manual release scaling of existing-resident housing priority"]
fn benchmark_hourly_rehousing_priority() {
    use std::time::Instant;
    const DISPLACED: usize = 128;
    for background in [1_024, 8_192, 65_536] {
        let mut allocator = BuildingAllocator::new();
        let asset = register_test_residential_asset_with_capacity(
            &mut allocator,
            "test",
            "priority_bench",
            1,
        );
        for index in 0..background + DISPLACED {
            let mut home = make_building(index as f32 * 20.0, ZoneType::Residential, &asset, 0.0);
            home.level = if index < background { 3 } else { 1 };
            allocator.buildings.push(home);
        }
        allocator.rebuild_zone_index();
        let mut households = HouseholdSystem::new();
        let mut agents = AgentSystem::new();
        for hid in 0..DISPLACED {
            households
                .households
                .push(make_household(usize::MAX, 1, 1.0, 1.0));
            let agent = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
            agents.assign_household_id(agent, hid);
        }
        let mut census_samples = Vec::new();
        let mut housing_samples = Vec::new();
        for round in 0..11 {
            for hid in 0..DISPLACED {
                let home = households.households[hid].home_building_id;
                if home != usize::MAX {
                    allocator.release_vacancy(home);
                    agents.evict_household_member_home(hid, home);
                    households.households[hid].home_building_id = usize::MAX;
                }
            }
            let start = Instant::now();
            households.recount_every_household(&agents, &allocator);
            let census_ms = start.elapsed().as_secs_f64() * 1_000.0;
            let start = Instant::now();
            households.rehouse_unhoused_before_growth(&mut agents, &mut allocator);
            let housing_ms = start.elapsed().as_secs_f64() * 1_000.0;
            if round >= 2 {
                census_samples.push(census_ms);
                housing_samples.push(housing_ms);
            }
            for (hid, household) in households.households.iter().enumerate() {
                assert_eq!(household.home_building_id, background + hid);
                assert_eq!(agents.home_building[hid], background + hid);
                assert_eq!(allocator.buildings[background + hid].occupancy, 1);
            }
            assert_eq!(allocator.first_rehousing_vacancy(|level| level < 3), None);
            assert_eq!(allocator.first_rehousing_vacancy(|_| true), Some(0));
        }
        census_samples.sort_by(f64::total_cmp);
        housing_samples.sort_by(f64::total_cmp);
        eprintln!(
            "rehousing_priority background={background} displaced={DISPLACED} census_ms={:.3} rehousing_ms={:.3}",
            census_samples[4], housing_samples[4]
        );
    }
}

#[test]
fn rehousing_from_a_broken_home_releases_its_occupied_place() {
    for hourly in [false, true] {
        let mut allocator = BuildingAllocator::new();
        let asset = register_test_residential_asset_with_capacity(
            &mut allocator,
            "test",
            "broken_rehousing",
            1,
        );
        for index in 0..2 {
            let mut home = make_building(index as f32 * 20.0, ZoneType::Residential, &asset, 0.0);
            home.broken = index == 0;
            home.occupancy = u32::from(index == 0);
            allocator.buildings.push(home);
        }
        allocator.rebuild_zone_index();
        let mut households = HouseholdSystem::new();
        households.households.push(make_household(0, 1, 2.0, 1.0));
        let mut agents = AgentSystem::new();
        let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.assign_household_id(agent, 0);
        if hourly {
            households.recount_every_household(&agents, &allocator);
            households.rehouse_unhoused_before_growth(&mut agents, &mut allocator);
        } else {
            households.resolve_household_housing(&mut agents, &mut allocator);
        }
        assert_eq!(households.households[0].home_building_id, 1);
        assert_eq!(agents.home_building[agent], 1);
        assert_eq!(
            allocator.buildings[0].occupancy, 0,
            "old home must not retain a phantom household"
        );
        assert_eq!(allocator.buildings[1].occupancy, 1);
    }
}

#[test]
fn removing_an_agent_preserves_a_moved_shoppers_active_trip() {
    for remove_freight in [false, true] {
        let mut allocator = BuildingAllocator::new();
        allocator
            .buildings
            .push(make_building(0.0, ZoneType::Residential, "test:home", 0.0));
        let mut agents = AgentSystem::new();
        let removed = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        let shopper = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.assign_household_id(shopper, 0);
        let mut households = HouseholdSystem::new();
        let mut household = make_household(0, 1, 100.0, 0.0);
        household.replenishment_state = REPLENISHMENT_SHOPPING_RETURNING;
        household.shopping_agent = agents.agent_handle(shopper);
        household.reserved_amount = 10.0;
        household.shopping_timeout_hours_remaining = 24;
        households.households.push(household);

        if remove_freight {
            agents.freight_shipment_id[removed] = 1;
            agents.vehicle_type[removed] = VEHICLE_FREIGHT_DELIVERY;
            agents.remove_freight_carrier(removed, 1);
        } else {
            agents.kill_agent(removed, &mut allocator);
        }

        assert_eq!(
            agents.resolve_agent(households.households[0].shopping_agent),
            Some(0)
        );
        households.run_household_replenishment(
            &mut agents,
            &mut allocator,
            &TransitNetwork::new(),
            &RegionGraph::new(),
            0,
        );
        assert_eq!(
            households.households[0].replenishment_state,
            REPLENISHMENT_FULFILLED
        );
        assert_eq!(households.households[0].stock, 10.0);
    }
}

#[test]
#[ignore = "manual matched release timing of batched household removal"]
fn benchmark_household_removal() {
    use std::{hint::black_box, time::Instant};
    let policy = exit_policy();
    for count in [1_024, 8_192, 65_536] {
        let mut samples = Vec::new();
        for _ in 0..7 {
            let mut households = HouseholdSystem::new();
            let mut agents = AgentSystem::new();
            let mut allocator = BuildingAllocator::new();
            for idx in 0..count {
                households
                    .households
                    .push(make_household(usize::MAX, 1, idx as f32, 0.0));
                let agent = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
                agents.assign_household_id(agent, idx);
            }
            for household in &mut households.households {
                household.unhoused_days_elapsed = policy.max_unhoused_days;
            }
            let start = Instant::now();
            let removed = black_box(households.execute_demand_household_removal(
                256,
                policy,
                &mut agents,
                &mut allocator,
            ));
            samples.push(start.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(removed, 256);
            assert_eq!(households.households.len(), count - 256);
            assert_eq!(agents.len(), count - 256);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "household_removal households={count} removed=256 median_ms={:.3}",
            samples[3]
        );
    }
}

#[test]
fn batched_removal_preserves_surviving_members_and_ledgers() {
    let mut households = HouseholdSystem::new();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();
    let removed = [1, 3, 8, 9];
    let mut original_household_by_seed = std::collections::HashMap::new();
    for idx in 0..12 {
        let budget = if removed.contains(&idx) {
            0.0
        } else {
            100.0 + idx as f32
        };
        let mut household = make_household(usize::MAX, 2, budget, 0.0);
        household.stock = idx as f32;
        household.unhoused_days_elapsed = exit_policy().max_unhoused_days;
        households.households.push(household);
        for _ in 0..2 {
            let agent = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
            agents.assign_household_id(agent, idx);
            original_household_by_seed.insert(agents.schedule_seed[agent], idx);
        }
    }
    households.ensure_daily_ledger_len();
    for (idx, ledger) in households.daily_ledgers.iter_mut().enumerate() {
        ledger.wage_income = idx as f32;
    }
    assert_eq!(
        households.execute_demand_household_removal(4, exit_policy(), &mut agents, &mut allocator,),
        4
    );
    assert_eq!(households.households.len(), 8);
    assert_eq!(agents.len(), 16);
    for idx in 0..agents.len() {
        let original = original_household_by_seed[&agents.schedule_seed[idx]];
        assert!(!removed.contains(&original));
        let current = agents.household_id[idx];
        assert_eq!(households.households[current].stock, original as f32);
        assert_eq!(
            households.daily_ledgers[current].wage_income,
            original as f32
        );
    }
}

#[test]
fn child_only_household_cannot_keep_housing() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 1, 12.0, 3.0);
    household.child_count = 1;
    household.adult_count = 0;
    household.elder_count = 0;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "child_only_home",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let child = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(child, 0);
    agents.age_group[child] = AGE_CHILD;
    agents.transit[child] = TRANSIT_IN_BUILDING;
    agents.current_building[child] = 0;
    allocator.claim_vacancy(0);
    assert_eq!(allocator.buildings[0].occupancy, 1);

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, usize::MAX);
    assert_eq!(households.households[0].unhoused_days_elapsed, 1);
    assert_eq!(agents.home_building[child], usize::MAX);
    assert_eq!(allocator.buildings[0].occupancy, 0);
}

#[test]
fn under_construction_home_does_not_count_as_housed() {
    let household = make_household(0, 2, 3.0, 3.0);
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "under_construction_home",
        ZoneClass::Residential,
    );
    let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    home.construction_total_hours = 6;
    home.set_construction_hours_left(6);
    allocator.buildings.push(home);

    assert!(!household_is_housed(&household, &allocator));
}

#[test]
fn demand_household_removal_prioritizes_unhoused_households() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.5, 1.0));
    households
        .households
        .push(make_household(usize::MAX, 1, 5.0, 5.0));
    households.households.push(make_household(1, 1, 2.0, 2.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "removal_res_a",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let housed_a = agents.spawn_housed_agent(0, 0.0, 0.0);
    let unhoused = agents.spawn_housed_agent(0, 0.0, 0.0);
    let housed_b = agents.spawn_housed_agent(1, 0.0, 0.0);
    agents.assign_household_id(housed_a, 0);
    agents.assign_household_id(unhoused, 1);
    agents.home_building[unhoused] = usize::MAX;
    agents.target_building[unhoused] = usize::MAX;
    agents.assign_household_id(housed_b, 2);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(1);

    households.households[1].unhoused_days_elapsed = exit_policy().max_unhoused_days;
    households.execute_demand_household_removal(1, exit_policy(), &mut agents, &mut allocator);

    assert_eq!(households.households.len(), 2);
    assert_eq!(agents.len(), 2);
    assert!(
        agents
            .household_id
            .iter()
            .all(|&household_id| household_id < households.households.len())
    );
    assert!(agents.home_building.iter().all(|&home| home != usize::MAX));
}

#[test]
fn departure_execution_protects_recently_displaced_and_recovered_households() {
    let mut households = HouseholdSystem::new();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();
    // The poorest household has just lost its home. The next has recovered its resources
    // during the rehousing period. Only the destitute/expired households may leave.
    for (days, reserve_days, stock_days) in [
        (0, 0.0, 0.0),
        (2, 10.0, 10.0),
        (2, 0.0, 0.0),
        (7, 10.0, 10.0),
    ] {
        let id = households.households.len();
        let mut household = make_household(usize::MAX, 1, reserve_days, stock_days);
        household.unhoused_days_elapsed = days;
        households.households.push(household);
        let agent = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        agents.assign_household_id(agent, id);
    }
    let recovered_budget = households.households[1].budget;
    assert_eq!(
        households.execute_demand_household_removal(4, exit_policy(), &mut agents, &mut allocator,),
        2
    );
    assert_eq!(households.households.len(), 2);
    assert_eq!(agents.len(), 2);
    assert_eq!(households.households[0].unhoused_days_elapsed, 0);
    assert_eq!(households.households[1].budget, recovered_budget);
    assert_eq!(agents.household_id, vec![0, 1]);
}

#[test]
fn demand_household_removal_remaps_moved_freight_carrier() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "removal_freight_res",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let resident = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(resident, 0);
    let freight = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(freight, usize::MAX);
    agents.home_building[freight] = usize::MAX;
    agents.current_building[freight] = usize::MAX;
    agents.target_building[freight] = usize::MAX;
    agents.freight_shipment_id[freight] = 77;
    agents.vehicle_type[freight] = VEHICLE_FREIGHT_DELIVERY;
    allocator.claim_vacancy(0);
    let mut logistics = ShipmentSystem::new();
    logistics.shipments.push(Shipment {
        id: 77,
        resource_runtime_id: 0,
        amount: 1.0,
        source: ShipmentEndpoint::Building(0),
        destination: ShipmentEndpoint::OwaBorder(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier: agents.agent_handle(freight),
        total_cost: 0.0,
        freight_cost: 0.0,
        eta_hours: 1,
        leg_started_hour: 0,
        trip_event_slot: 0,
        cargo_loaded: true,
    });

    // The home has failed before departure; retain the agent and shipment remapping fixture.
    allocator.buildings[0].is_deserted = true;
    households.households[0].unhoused_days_elapsed = exit_policy().max_unhoused_days;
    households.execute_demand_household_removal(1, exit_policy(), &mut agents, &mut allocator);

    assert_eq!(agents.len(), 1);
    assert_eq!(agents.freight_shipment_id[0], 77);
    assert_eq!(
        agents.resolve_agent(logistics.shipments[0].carrier),
        Some(0)
    );
}

#[test]
fn demand_household_removal_never_fills_unused_slots_with_housed_households() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.5, 0.5));
    households.households.push(make_household(1, 1, 5.0, 5.0));
    households
        .households
        .push(make_household(usize::MAX, 1, 4.0, 4.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "removal_res_b",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let weak_housed = agents.spawn_housed_agent(0, 0.0, 0.0);
    let strong_housed = agents.spawn_housed_agent(1, 0.0, 0.0);
    let unhoused = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(weak_housed, 0);
    agents.assign_household_id(strong_housed, 1);
    agents.assign_household_id(unhoused, 2);
    agents.home_building[unhoused] = usize::MAX;
    agents.target_building[unhoused] = usize::MAX;
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(1);

    households.households[2].unhoused_days_elapsed = exit_policy().max_unhoused_days;
    households.execute_demand_household_removal(2, exit_policy(), &mut agents, &mut allocator);

    assert_eq!(households.households.len(), 2);
    assert_eq!(agents.len(), 2);
    for id in 0..2 {
        assert_eq!(households.households[id].home_building_id, id);
        assert_eq!(agents.household_id[id], id);
        assert_eq!(agents.home_building[id], id);
        assert_eq!(allocator.buildings[id].occupancy, 1);
    }
}

#[test]
fn unhoused_household_rehouses_into_affordable_vacant_home() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(usize::MAX, 2, 12.0, 3.0);
    household.unhoused_days_elapsed = 4;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "rehouse_res",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for a in [a0, a1] {
        agents.assign_household_id(a, 0);
        agents.home_building[a] = usize::MAX;
        agents.current_building[a] = usize::MAX;
        agents.target_building[a] = usize::MAX;
        agents.planned_target_building[a] = usize::MAX;
        agents.transit[a] = TRANSIT_ACCESS_INGRESS;
    }

    // The movement tick has already settled one resident's invalid ingress while unhoused.
    agents.transit[a1] = TRANSIT_IN_BUILDING;

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, 0);
    assert_eq!(households.households[0].unhoused_days_elapsed, 0);
    assert_eq!(allocator.buildings[0].occupancy, 1);
    assert_eq!(households.households.len(), 1);
    assert_eq!(agents.home_building[a1], 0);
    assert_eq!(agents.target_building[a0], 0);
    assert_eq!(agents.target_building[a1], 0);
    assert_eq!(agents.transit[a0], TRANSIT_ACCESS_INGRESS);
    assert_eq!(agents.transit[a1], TRANSIT_ACCESS_INGRESS);
}

#[test]
fn upgrade_search_does_not_consume_same_or_lower_level_vacancy() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 30.0, 5.0));
    households
        .households
        .push(make_household(usize::MAX, 1, 30.0, 5.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_residential_asset_with_capacity(
        &mut allocator,
        "test",
        "upgrade_no_burn_res",
        1,
    );
    let mut current_home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    current_home.level = 2;
    current_home.occupancy = 1;
    allocator.buildings.push(current_home);
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let housed_agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(housed_agent, 0);
    let unhoused_agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(unhoused_agent, 1);
    agents.home_building[unhoused_agent] = usize::MAX;
    agents.current_building[unhoused_agent] = usize::MAX;
    agents.target_building[unhoused_agent] = usize::MAX;
    agents.planned_target_building[unhoused_agent] = usize::MAX;

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, 0);
    assert_eq!(households.households[1].home_building_id, 1);
    assert_eq!(allocator.buildings[0].occupancy, 1);
    assert_eq!(allocator.buildings[1].occupancy, 1);
    assert_eq!(agents.home_building[unhoused_agent], 1);
}

#[test]
fn same_day_relocation_frees_home_for_later_household() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 30.0, 5.0));
    households
        .households
        .push(make_household(usize::MAX, 1, 30.0, 5.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_residential_asset_with_capacity(
        &mut allocator,
        "test",
        "same_day_free_res",
        1,
    );
    let mut old_home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    old_home.level = 1;
    old_home.occupancy = 1;
    let mut upgrade_home = make_building(20.0, ZoneType::Residential, &residential_asset, 0.0);
    upgrade_home.level = 2;
    allocator.buildings.push(old_home);
    allocator.buildings.push(upgrade_home);
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let upgrading_agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(upgrading_agent, 0);
    let unhoused_agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(unhoused_agent, 1);
    agents.home_building[unhoused_agent] = usize::MAX;
    agents.current_building[unhoused_agent] = usize::MAX;
    agents.target_building[unhoused_agent] = usize::MAX;
    agents.planned_target_building[unhoused_agent] = usize::MAX;

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, 1);
    assert_eq!(households.households[1].home_building_id, 0);
    assert_eq!(allocator.buildings[0].occupancy, 1);
    assert_eq!(allocator.buildings[1].occupancy, 1);
    assert_eq!(agents.home_building[upgrading_agent], 1);
    assert_eq!(agents.home_building[unhoused_agent], 0);
}

#[test]
fn unrehouseable_unhoused_household_accumulates_unhoused_days() {
    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(usize::MAX, 2, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();

    households.resolve_household_housing(&mut agents, &mut allocator);
    assert_eq!(households.households[0].unhoused_days_elapsed, 1);

    households.resolve_household_housing(&mut agents, &mut allocator);
    assert_eq!(households.households[0].unhoused_days_elapsed, 2);
}

#[test]
fn failed_stay_rule_evicts_household_when_no_affordable_home_exists() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 2, 0.5, 1.0);
    household.stay_failure_days = 1;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "test", "evict_res", ZoneClass::Residential);
    let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    home.level = 2;
    home.occupancy = 1;
    allocator.buildings.push(home);
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for a in [a0, a1] {
        agents.assign_household_id(a, 0);
        agents.home_building[a] = 0;
        agents.current_building[a] = 0;
        agents.target_building[a] = usize::MAX;
        agents.planned_target_building[a] = usize::MAX;
        agents.transit[a] = TRANSIT_IN_BUILDING;
    }

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households.len(), 1);
    assert_eq!(households.households[0].home_building_id, usize::MAX);
    assert_eq!(households.households[0].stay_failure_days, 0);
    assert_eq!(allocator.buildings[0].occupancy, 0);
    assert_eq!(agents.home_building[a0], usize::MAX);
    assert_eq!(agents.home_building[a1], usize::MAX);
    assert_eq!(agents.current_building[a0], usize::MAX);
    assert_eq!(agents.current_building[a1], usize::MAX);
    assert_eq!(agents.transit[a0], TRANSIT_ACCESS_INGRESS);
    assert_eq!(agents.transit[a1], TRANSIT_ACCESS_INGRESS);
}

#[test]
fn evicting_a_household_hands_its_shopper_at_a_store_back_to_the_schedule() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 1, 0.5, 1.0);
    household.stay_failure_days = 1;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "evict_shop_res",
        ZoneClass::Residential,
    );
    let store_asset = register_test_asset(
        &mut allocator,
        "test",
        "evict_shop_store",
        ZoneClass::Commercial,
    );
    let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    home.level = 2;
    home.occupancy = 1;
    allocator.buildings.push(home);
    allocator
        .buildings
        .push(make_building(50.0, ZoneType::Commercial, &store_asset, 0.0));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let shopper = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(shopper, 0);
    agents.current_building[shopper] = 1;
    agents.transit[shopper] = TRANSIT_IN_BUILDING;
    agents.activity[shopper] = crate::simulation::economy::agents::ACTIVITY_SHOPPING;
    let household = &mut households.households[0];
    household.replenishment_state = REPLENISHMENT_SHOPPING_TO_STORE;
    household.shopping_agent = agents.agent_handle(shopper);
    household.reserved_store_building_id = 1;
    household.reserved_amount = 2.0;

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, usize::MAX);
    assert_eq!(allocator.buildings[1].inventory_units(supplies), 2.0);
    // Shopping released the agent: it used to stay in the store with the shopping activity and
    // no trip, and its household never shopped again.
    assert_eq!(agents.current_building[shopper], 1);
    assert_ne!(
        agents.activity[shopper],
        crate::simulation::economy::agents::ACTIVITY_SHOPPING
    );
    // The schedule moves only agents at home or at work, so rehousing sends it home.
    agents.relocate_household_member_home(shopper, usize::MAX, 0, false);
    assert_eq!(agents.planned_target_building[shopper], 0);
    assert_eq!(
        agents.planned_activity[shopper],
        crate::simulation::economy::agents::ACTIVITY_HOME
    );
}

#[test]
fn evicted_unhoused_household_keeps_membership_until_demand_removal() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 2, 0.5, 1.0);
    household.stay_failure_days = 1;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "tracked_unhoused_res",
        ZoneClass::Residential,
    );
    let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    home.level = 2;
    home.occupancy = 1;
    allocator.buildings.push(home);
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for a in [a0, a1] {
        agents.assign_household_id(a, 0);
        agents.home_building[a] = 0;
        agents.current_building[a] = 0;
        agents.target_building[a] = usize::MAX;
        agents.planned_target_building[a] = usize::MAX;
        agents.transit[a] = TRANSIT_IN_BUILDING;
    }

    households.resolve_household_housing(&mut agents, &mut allocator);
    households.recount_every_household(&agents, &allocator);

    assert_eq!(households.households.len(), 1);
    assert_eq!(households.households[0].home_building_id, usize::MAX);
    assert_eq!(households.households[0].member_count, 2);
    assert_eq!(agents.household_id[a0], 0);
    assert_eq!(agents.household_id[a1], 0);
    households.households[0].unhoused_days_elapsed = exit_policy().max_unhoused_days;
    households.execute_demand_household_removal(1, exit_policy(), &mut agents, &mut allocator);

    assert_eq!(households.households.len(), 0);
    assert_eq!(agents.len(), 0);
}

#[test]
fn eviction_settles_active_shopping_reservations_once() {
    let catalog = load_runtime_economy_catalog().expect("catalog");
    let supplies = household_supply_resource_runtime_id(&catalog);
    for picked_up in [false, true] {
        let household = make_household(0, 1, 30.0, 0.0);
        let (mut households, mut allocator, mut agents, network, graph) =
            setup_replenishment_world(household, "eviction_home", "eviction_store", 50.0, 20.0);
        allocator.buildings[0].level = 2;
        allocator.claim_vacancy(0);
        households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);
        assert_eq!(
            households.households[0].replenishment_state,
            REPLENISHMENT_SHOPPING_TO_STORE
        );
        let reserved_cost = households.households[0].reserved_total_cost;
        let reserved_amount = households.households[0].reserved_amount;
        if picked_up {
            arrive_agent_at_building(&mut agents, 0, 1, 2);
            households.run_household_replenishment(
                &mut agents,
                &mut allocator,
                &network,
                &graph,
                0,
            );
            assert_eq!(
                households.households[0].replenishment_state,
                REPLENISHMENT_SHOPPING_RETURNING
            );
        }
        let store_revenue = allocator.buildings[1].revenue;
        households.households[0].budget = 0.0;
        households.households[0].stay_failure_days = 1;

        households.resolve_household_housing(&mut agents, &mut allocator);

        assert_eq!(households.households[0].home_building_id, usize::MAX);
        assert_eq!(households.households[0].reserved_amount, 0.0);
        assert_eq!(
            households.households[0].shopping_agent,
            crate::simulation::economy::agents::AgentHandle::NONE
        );
        assert_eq!(
            households.households[0].budget,
            if picked_up { 0.0 } else { reserved_cost }
        );
        assert_eq!(
            allocator.buildings[1].inventory_units(supplies),
            if picked_up {
                50.0 - reserved_amount
            } else {
                50.0
            }
        );
        assert_eq!(allocator.buildings[1].revenue, store_revenue);
        assert_eq!(
            households.daily_ledgers()[0].shopping_spend,
            if picked_up { reserved_cost } else { 0.0 }
        );
        assert_eq!(households.daily_ledgers()[0].shopper_trips_failed, 1);

        // A later hourly pass cannot refund the cancelled reservation again.
        households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 1);
        assert_eq!(
            households.households[0].budget,
            if picked_up { 0.0 } else { reserved_cost }
        );
        assert_eq!(households.daily_ledgers()[0].shopper_trips_failed, 1);
    }
}

#[test]
fn housing_distance_ties_use_building_order_across_chunk_boundaries() {
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "housing_tie",
        ZoneClass::Residential,
    );
    for (x, level) in [(512.0, 2), (0.0, 2), (256.0, 1)] {
        let mut building = make_building(x, ZoneType::Residential, &asset, 0.0);
        building.center_y = 256.0;
        building.level = level;
        allocator.buildings.push(building);
    }
    allocator.rebuild_zone_index();
    allocator.claim_vacancy(2);
    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(2, 1, 100.0, 30.0));
    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(2, 256.0, 256.0);
    agents.assign_household_id(agent, 0);

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, 0);
    assert_eq!(agents.home_building[agent], 0);
}

#[test]
fn failed_stay_rule_does_not_relocate_zero_reserve_household_to_level_one() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 2, 0.0, 1.0);
    household.stay_failure_days = 1;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "zero_reserve_relocate_res",
        ZoneClass::Residential,
    );
    let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    home.occupancy = 1;
    allocator.buildings.push(home);
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for a in [a0, a1] {
        agents.assign_household_id(a, 0);
        agents.home_building[a] = 0;
        agents.current_building[a] = 0;
        agents.target_building[a] = usize::MAX;
        agents.planned_target_building[a] = usize::MAX;
        agents.transit[a] = TRANSIT_IN_BUILDING;
    }

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, usize::MAX);
    assert_eq!(allocator.buildings[0].occupancy, 0);
    assert_eq!(allocator.buildings[1].occupancy, 0);
    assert_eq!(agents.home_building[a0], usize::MAX);
    assert_eq!(agents.home_building[a1], usize::MAX);
}

#[test]
fn housing_requests_leave_oldest_first_across_rehousings_and_reviews() {
    let mut allocator = BuildingAllocator::new();
    let asset =
        register_test_residential_asset_with_capacity(&mut allocator, "test", "oldest_first", 2);
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &asset, 0.0));
    allocator.rebuild_zone_index();
    let mut households = HouseholdSystem::new();
    let mut agents = AgentSystem::new();
    for hid in 0..2 {
        households.households.push(make_household(0, 1, 5.0, 5.0));
        let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.assign_household_id(agent, hid);
    }
    households.handles.register_appended(2);
    let (first, second) = (households.handles.handle(0), households.handles.handle(1));
    households.rehousing.push(9, second);
    households.housing_reviews.push(8, first);
    assert_eq!(
        households.serve_housing_requests(1, &mut agents, &mut allocator),
        1
    );
    assert!(
        households.housing_reviews.is_empty(),
        "the older review first"
    );
    assert_eq!(households.rehousing.len(), 1);
    households.housing_reviews.push(9, second);
    households.serve_housing_requests(1, &mut agents, &mut allocator);
    assert!(
        households.rehousing.is_empty() && households.housing_reviews.len() == 1,
        "a household's rehousing goes before its review of the same slot"
    );
}
