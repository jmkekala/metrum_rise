// SPDX-License-Identifier: GPL-2.0-only

//! One-household farms, resident employment, and shared household lifecycle regressions.

use super::support::*;
use super::*;
use crate::assets::asset::BuildingFieldData;
use crate::simulation::buildings::allocator::baseline_private_zone_slot;
use crate::simulation::economy::agents::AGE_ADULT;
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use std::sync::atomic::Ordering;

fn farm_allocator() -> BuildingAllocator {
    let mut allocator = BuildingAllocator::new();
    let id = register_test_commercial_asset_with_profile(
        &mut allocator,
        "test",
        "farm_home",
        "grain_farm_basic",
    );
    let mut manifest = allocator.registry.get(&id).unwrap().manifest.clone();
    let authored = manifest.building.as_mut().unwrap();
    authored.placement_mode = PlacementMode::Explicit;
    authored.zone_type = None;
    authored.density = None;
    authored.field = Some(BuildingFieldData {
        resource: "grain".into(),
        area_mode: "player_polygon".into(),
    });
    allocator.registry.register("test", manifest, String::new());
    let mut farm = make_building(10.0, ZoneType::None, &id, 0.0);
    farm.economy_profile_runtime_id = load_runtime_economy_catalog()
        .unwrap()
        .profile_for_id("grain_farm_basic")
        .unwrap()
        .runtime_id;
    farm.operating_budget = 10_000.0;
    farm.commercial_activity_floor_scale = 1.0;
    allocator.buildings.push(farm);
    allocator.rebuild_zone_index();
    allocator
}

#[test]
fn farm_has_one_household_slot_independent_of_field_jobs() {
    let mut allocator = farm_allocator();
    let residential = baseline_private_zone_slot(ZoneType::Residential).unwrap();
    assert!(allocator.zone_index[residential].is_empty());
    assert_eq!(allocator.vacancy_index[residential], vec![0]);
    assert_eq!(allocator.flat_size_m2(0), 120.0);
    assert!(allocator.next_household_admission_candidate().is_some());
    let catalog = load_runtime_economy_catalog().unwrap();
    for (hectares, jobs) in [(0.01, 2), (20.0, 2), (30.0, 3), (100.0, 10)] {
        allocator.buildings[0].set_work_area_scale(hectares);
        assert_eq!(allocator.household_capacity(0), 1);
        assert_eq!(allocator.worker_capacity_with_catalog(0, &catalog), jobs);
    }
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    assert_eq!(allocator.buildings[0].occupancy, 1);
    assert!(allocator.next_household_admission_candidate().is_none());
    allocator.rebuild_zone_index();
    assert!(allocator.vacancy_index[residential].is_empty());
    allocator.release_vacancy(0);
    assert_eq!(allocator.vacancy_index[residential], vec![0]);
    let mut second = allocator.buildings[0].clone();
    second.center_x = 50.0;
    allocator.buildings.push(second);
    assert!(allocator.index_appended_building(1));
    assert_eq!(allocator.vacancy_index[residential], vec![0, 1]);
    assert!(allocator.zone_index[residential].is_empty());
}

#[test]
fn bankrupt_farm_withdraws_vacancy_before_rehousing() {
    let mut allocator = farm_allocator();
    let mut healthy = allocator.buildings[0].clone();
    healthy.center_x = 50.0;
    allocator.buildings.push(healthy);
    assert!(allocator.index_appended_building(1));
    allocator.buildings[0].operating_budget = -1.0;
    allocator.buildings[0].budget_distress = true;
    let mut households = HouseholdSystem::new();
    households.run_bankruptcy_check(&mut allocator);
    let residential = baseline_private_zone_slot(ZoneType::Residential).unwrap();
    assert_eq!(allocator.household_capacity(0), 0);
    assert_eq!(allocator.vacancy_index[residential], vec![1]);
    assert_eq!(allocator.vacancy_pos, vec![usize::MAX, 0]);
    assert!(allocator.preview_vacancy_claims_except(1, 1).is_empty());
    assert_eq!(
        allocator.preview_vacancy_claims_except(usize::MAX, 2),
        vec![1]
    );
    allocator.release_vacancy(0);
    allocator.claim_vacancy(1);
    assert!(allocator.vacancy_index[residential].is_empty());
    assert!(allocator.next_household_admission_candidate().is_none());
}

#[test]
fn farm_carrier_materializes_one_normal_household_once() {
    let mut allocator = farm_allocator();
    let mut households = HouseholdSystem::new();
    let mut agents = AgentSystem::new();
    allocator.claim_vacancy(0);
    let carrier = agents.spawn_household_arrival_carrier(0, 6, 0, 10.0, 0.0);
    agents.current_building[carrier] = 0;
    agents.transit[carrier] = TRANSIT_IN_BUILDING;
    let arrival = households.materialize_arrived_household_carriers(&mut agents, &allocator);
    assert_eq!((arrival.households, arrival.residents), (1, 6));
    assert_eq!(
        households
            .materialize_arrived_household_carriers(&mut agents, &allocator)
            .households,
        0
    );
    households.rebuild_household_and_worker_counts(&agents, &mut allocator);
    assert_eq!(households.households.len(), 1);
    assert_eq!(allocator.buildings[0].occupancy, 1);
    assert!(agents.home_building.iter().all(|&home| home == 0));
    assert!(agents.household_id.iter().all(|&id| id == 0));
    assert!(household_is_housed(&households.households[0], &allocator));
    let stock_before = households.households[0].stock;
    let consumption = households.households[0].consumption_rate;
    households.consume_household_stock(&mut agents);
    assert!(
        (households.households[0].stock - (stock_before - 6.0 * consumption / 24.0)).abs() < 0.001
    );
    assert!(households.households[0].adult_count > 0);
    assert!(households.households[0].child_count + households.households[0].elder_count > 0);
}

#[test]
fn unhoused_family_can_claim_farm_but_not_a_second_household() {
    let mut allocator = farm_allocator();
    let mut households = HouseholdSystem::new();
    let mut agents = AgentSystem::new();
    for id in 0..2 {
        households
            .households
            .push(make_household(usize::MAX, 2, 10_000.0, 5.0));
        for age in [AGE_ADULT, AGE_CHILD] {
            let agent = agents.spawn_housed_agent_with_age_group(usize::MAX, 0.0, 0.0, age);
            agents.assign_household_id(agent, id);
        }
    }
    households.rebuild_household_and_worker_counts(&agents, &mut allocator);
    households.resolve_household_housing(&mut agents, &mut allocator);
    assert_eq!(households.households[0].home_building_id, 0);
    assert_eq!(households.households[1].home_building_id, usize::MAX);
    assert_eq!(allocator.buildings[0].occupancy, 1);
    assert_eq!(agents.home_building[0], 0);
    assert_eq!(agents.home_building[1], 0);
    allocator.buildings[0].set_work_area_scale(0.01);
    households.resolve_household_housing(&mut agents, &mut allocator);
    assert_eq!(households.households[0].home_building_id, 0);
    allocator.buildings[0].is_deserted = true;
    allocator.rebuild_zone_index();
    assert!(allocator.next_household_admission_candidate().is_none());
}

#[test]
fn farm_family_works_on_site_and_larger_fields_hire_commuters() {
    let mut allocator = farm_allocator();
    let (mut graph, mut network) = simple_work_graph();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "outside_home",
        ZoneClass::Residential,
    );
    allocator
        .buildings
        .push(make_building(30.0, ZoneType::Residential, &home, 0.0));
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(1, 1, 0.0, 0.0));
    households.households.push(make_household(0, 4, 0.0, 0.0));
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(1);
    let mut agents = AgentSystem::new();
    // A commuter with a lower agent ID must not consume the resident adults' available slots.
    let commuter = agents.spawn_housed_agent(1, 30.0, 0.0);
    agents.assign_household_id(commuter, 0);
    for age in [AGE_ADULT, AGE_ADULT, AGE_CHILD, AGE_ELDER] {
        let id = agents.spawn_housed_agent_with_age_group(0, 10.0, 0.0, age);
        agents.assign_household_id(id, 1);
        agents.schedule_seed[id] = 0;
    }
    households.rebuild_household_and_worker_counts(&agents, &mut allocator);
    let policy = CityFiscalPolicy::default();
    let mut treasury = 100_000.0;
    households.pay_household_transfers(&agents, &allocator, &mut treasury, &policy);
    assert_eq!(
        households.households[1].budget,
        2.0 * policy.unemployment_benefit_per_adult_per_day
            + policy.pension_per_elder_per_day
            + policy.child_support_per_child_per_day
    );
    households.households[1].budget = 0.0;
    households.households[0].budget = 0.0;
    households.assign_agent_workplaces_with_service_funding(
        &mut agents,
        &mut allocator,
        &network,
        &graph,
        &[],
        true,
    );
    assert_eq!(allocator.buildings[0].worker_count, 2);
    assert_eq!(agents.work_building[1], 0);
    assert_eq!(agents.work_building[2], 0);
    assert_eq!(agents.work_building[commuter], usize::MAX);
    assert_eq!(agents.work_building[3], usize::MAX);
    assert_eq!(agents.work_building[4], usize::MAX);

    let routes = agents.pathfind_count.load(Ordering::Relaxed);
    for (minute, activity) in [(12 * 60, 1), (20 * 60, 0), (12 * 60, 1)] {
        agents.tick(&allocator, &mut network, &mut graph, 0.1, 0, minute);
        for id in [1, 2] {
            assert_eq!(agents.activity[id], activity);
            assert_eq!(agents.transit[id], TRANSIT_IN_BUILDING);
            assert_eq!(agents.current_building[id], 0);
            assert_eq!(agents.planned_target_building[id], usize::MAX);
        }
        assert_eq!(agents.pathfind_count.load(Ordering::Relaxed), routes);
    }
    allocator.buildings[0].set_work_area_scale(30.0);
    households.assign_agent_workplaces_with_service_funding(
        &mut agents,
        &mut allocator,
        &network,
        &graph,
        &[],
        true,
    );
    assert_eq!(agents.work_building[commuter], 0);
    assert_eq!(allocator.buildings[0].worker_count, 3);
    assert_eq!(agents.home_building[commuter], 1);
    allocator.buildings[0].set_work_area_scale(0.01);
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury);
    assert_eq!(allocator.buildings[0].worker_count, 2);
    assert_eq!(allocator.buildings[0].occupancy, 1);
    assert_eq!(households.households[1].home_building_id, 0);
    assert!(agents.home_building[1..].iter().all(|&home| home == 0));
    for id in [1, 2] {
        if agents.work_building[id] == usize::MAX {
            assert_eq!(
                agents.activity[id], 0,
                "An unemployed resident is available for home activities"
            );
        }
    }
}

#[test]
fn leaving_farm_job_preserves_the_trip_home() {
    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 10.0, 0.0);
    agents.assign_work_building(agent, 0, 0);
    agents.transit[agent] = TRANSIT_ACCESS_INGRESS;
    agents.current_building[agent] = usize::MAX;
    agents.target_building[agent] = 0;
    agents.planned_target_building[agent] = 0;
    agents.activity[agent] = 0;
    agents.planned_activity[agent] = 0;
    agents.current_path[agent] = vec![1, 2];
    agents.assign_work_building(agent, usize::MAX, 0);
    assert_eq!(agents.home_building[agent], 0);
    assert_eq!(agents.target_building[agent], 0);
    assert_eq!(agents.planned_target_building[agent], 0);
    assert_eq!(agents.current_path[agent], vec![1, 2]);
}

#[test]
#[ignore = "manual release locality timing for farmhouse vacancy claims"]
fn benchmark_farm_vacancy_claims() {
    use std::hint::black_box;
    use std::time::Instant;

    for count in [1, 1_000, 100_000] {
        let mut allocator = farm_allocator();
        let farm = allocator.buildings[0].clone();
        allocator.buildings.resize(count, farm);
        allocator.rebuild_zone_index();
        // Keep one home's claim/release workload fixed as background housing increases.
        // Setup, index construction, and capacity reservations are outside the timed loop.
        let mut samples = [0.0; 21];
        for iteration in 0..24 {
            let start = Instant::now();
            for _ in 0..10_000 {
                allocator.claim_vacancy(black_box(0));
                allocator.release_vacancy(black_box(0));
            }
            if iteration >= 3 {
                samples[iteration - 3] = start.elapsed().as_secs_f64();
            }
        }
        let residential = baseline_private_zone_slot(ZoneType::Residential).unwrap();
        assert_eq!(allocator.buildings[0].occupancy, 0);
        assert_eq!(allocator.vacancy_index[residential].len(), count);
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "farm_vacancies buildings={count} pairs=10000 median_ms={:.3}",
            samples[10] * 1000.0
        );
    }
}

#[test]
#[ignore = "manual matched release timing for idle versus resident-worker ticks"]
fn benchmark_farm_household_tick() {
    use std::hint::black_box;
    use std::time::Instant;

    // Synthetic in-building agents isolate movement cost; no placement, admission, or economy setup.
    for count in [1_000, 10_000, 100_000] {
        for working in [false, true] {
            let mut allocator = farm_allocator();
            let (mut graph, mut network) = simple_work_graph();
            allocator.rebuild_entrance_cache(&graph, &network.lane_system);
            let mut agents = AgentSystem::new();
            for _ in 0..count {
                let agent = agents.spawn_housed_agent(0, 10.0, 0.0);
                agents.schedule_seed[agent] = 0;
                if working {
                    agents.assign_work_building(agent, 0, 0);
                }
            }
            for _ in 0..3 {
                agents.tick(&allocator, &mut network, &mut graph, 0.1, 0, 720);
            }
            let mut samples = [0.0; 21];
            for sample in &mut samples {
                let start = Instant::now();
                for _ in 0..10 {
                    agents.tick(&allocator, &mut network, &mut graph, 0.1, 0, black_box(720));
                }
                *sample = start.elapsed().as_secs_f64() / 10.0;
            }
            assert!(
                agents
                    .transit
                    .iter()
                    .all(|&state| state == TRANSIT_IN_BUILDING)
            );
            assert_eq!(agents.pathfind_count.load(Ordering::Relaxed), 0);
            assert!(
                agents
                    .activity
                    .iter()
                    .all(|&activity| activity == u8::from(working))
            );
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "farm_household_tick agents={count} working={working} median_ms={:.3} ns_per_agent={:.2}",
                samples[10] * 1000.0,
                samples[10] * 1e9 / count as f64
            );
        }
    }
}
