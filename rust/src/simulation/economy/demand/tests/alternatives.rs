// SPDX-License-Identifier: GPL-2.0-only

//! Viable business alternatives and shared reservations across lots and categories.

use super::*;
use crate::simulation::buildings::allocator::BuildingSiteEnvironment;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::terrain::TerrainSystem;

#[test]
fn snapshot_uses_funded_recipe_activity_instead_of_rounded_jobs_or_owner_assets() {
    use crate::simulation::economy::logistics::ShipmentSystem;
    use crate::simulation::economy::operation::InputRequirement;
    use crate::simulation::network::TransitNetwork;
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 1.0;
    let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    let config = load_builtin_demand_config().unwrap();
    let mut graph = graph_with_connected_border();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(&mut allocator, "funded_buyer_home", ZoneType::Residential);
    let shop_asset = register_test_asset(&mut allocator, "funded_buyer_shop", ZoneType::Commercial);
    let mut home = building(ZoneType::Residential, 0.0, 1, 0, home_asset);
    home.center_x = 10.0;
    home.center_y = 20.0;
    home.frontage_t = 0.2;
    allocator.buildings.push(home);
    let mut shop = building(ZoneType::Commercial, 0.0, 0, 3, shop_asset);
    shop.center_x = 40.0;
    shop.center_y = -20.0;
    shop.frontage_t = 0.8;
    shop.operating_budget = 100_000.0;
    allocator.buildings.push(shop);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut households = HouseholdSystem::new();
    let mut family = housed_household(0, 30, 100_000.0, 10.0);
    family.employed_adults = 30;
    family.gross_daily_wages = 10_000.0;
    households.households.push(family);
    let food = catalog.resource_runtime_id_for_id("packaged_food").unwrap();
    let supplies = catalog.resource_runtime_id_for_id("household_supplies").unwrap();
    let mut logistics = ShipmentSystem::new();
    for (workers, cash, income, stock, expected) in [
        (3, 100_000.0, 10_000.0, 0.0, 30.0),
        (0, 100_000.0, 10_000.0, 0.0, 0.0),
        (3, 0.0, 10_000.0, 0.0, 0.0),
        (3, 100_000.0, 0.0, 100_000.0, 0.0),
    ] {
        allocator.buildings[1].worker_count = workers;
        allocator.buildings[1].operating_budget = cash;
        allocator.buildings[1].set_inventory_units(food, stock);
        households.households[0].gross_daily_wages = income;
        households.job_funding.refresh(&allocator, &households.households, &mut logistics,
            &network, &graph, &catalog, &tuning, &policy, &[], 100_000.0, 420);
        if income > 0.0 && cash > 0.0 {
            assert_eq!(households.job_funding.capacities[1], 3);
        } else {
            assert_eq!(households.job_funding.capacities[1], 0);
        }
        let snapshot = DailyDemandSnapshot::from_runtime_with_catalog(
            &allocator, &households, &graph, &config, &catalog, &tuning, &policy, 100_000.0);
        assert!((snapshot.output_absorption.unmet_units(food) - expected).abs() < 0.001,
            "workers={workers} cash={cash} income={income} stock={stock}");
        assert_eq!(snapshot.output_absorption.unmet_units(supplies), 0.0,
            "the shop retains its full output commitment through its staffing search");
        if expected > 0.0 {
            for (units, fits) in [(30.0, true), (31.0, false)] {
                let costs = logistics.quote_recurring_input(&mut households.job_funding.input_supply,
                    InputRequirement { resource: supplies, daily_units: units, buffer_units: units * 2.0,
                        replenishment_batch_units: Some(40.0) },
                    Some(0), &allocator.entrances[0], None, &allocator, &catalog, &tuning, &network, &graph, 1.0);
                assert_eq!(costs.goods.is_finite(), fits,
                    "three rounded jobs cannot publish 40 units when only 30 are funded");
                households.job_funding.input_supply.reject(&allocator, &graph);
            }
        } else {
            assert!(!households.job_funding.input_supply.has_offers());
        }
    }
    tuning.operational_clock.freight_profiles.iter_mut()
        .find(|profile| profile.id == "daytime_receive").unwrap()
        .outside_window_cost_multiplier = 10.0;
    allocator.buildings[1].worker_count = 3;
    allocator.buildings[1].operating_budget = 100_000.0;
    allocator.buildings[1].set_inventory_units(food, 0.0);
    households.households[0].gross_daily_wages = 10_000.0;
    for (minute, jobs) in [(420, 3), (419, 0), (1080, 0), (420, 3)] {
        households.job_funding.refresh(&allocator, &households.households, &mut logistics,
            &network, &graph, &catalog, &tuning, &policy, &[], 100_000.0, minute);
        assert_eq!(households.job_funding.capacities[1], jobs,
            "live jobs must pay the receiving window's actual goods price at {minute}");
    }
    let mut revalidated = crate::simulation::economy::hiring::JobFunding::default();
    revalidated.refresh(&allocator, &households.households, &mut logistics,
        &network, &graph, &catalog, &tuning, &policy, &[], 100_000.0, 419);
    let current = DailyDemandSnapshot::from_runtime_with_funding(&allocator, &households,
        &graph, &config, &catalog, &tuning, &policy, 100_000.0, &revalidated);
    assert_eq!(current.output_absorption.unmet_units(food), 0.0,
        "a refreshed market cannot retain yesterday's funded input customers");
    assert_eq!(households.job_funding.capacities[1], 3,
        "the independent employment view still represents the earlier price interval");
}

#[test]
fn construction_shares_live_queued_and_placed_local_input_claims() {
    use crate::simulation::economy::logistics::ShipmentSystem;
    use crate::simulation::network::TransitNetwork;
    use std::sync::Arc;

    for commitment in ["new", "live", "queued"] {
        let mut demand = DemandSystem::new();
        Arc::make_mut(&mut demand.runtime_tuning).owa_import_price_multiplier = 2.0;
        let catalog = Arc::clone(&demand.runtime_catalog);
        let tuning = Arc::clone(&demand.runtime_tuning);
        let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
        let mut graph = graph_with_connected_border();
        let mut zoning = two_lots(&graph, ZoneType::Industrial);
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.rebuild_cch_and_check(&graph);
        let mut allocator = BuildingAllocator::new();
        let farm_asset =
            register_explicit_profile_asset(&mut allocator, "project_grain", "grain_farm_basic");
        let home_asset = register_test_asset(&mut allocator, "project_home", ZoneType::Residential);
        let factory_asset =
            register_test_asset(&mut allocator, "project_factory", ZoneType::Industrial);
        let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
        let mut farm = building(ZoneType::None, 0.0, 0, 1, farm_asset);
        farm.economy_profile_runtime_id = farm_profile.runtime_id;
        farm.work_area_scale = 10.0;
        let farm_workers =
            crate::simulation::economy::households::physical_worker_capacity_for_profile(
                &farm,
                farm_profile,
            );
        farm.worker_count = farm_workers;
        farm.operating_budget = 100_000.0;
        farm.center_x = 45.0;
        farm.center_y = 20.0;
        farm.frontage_t = 0.9;
        farm.parcel_id = 10_000;
        farm.cell_x = 100;
        allocator.buildings.push(farm);
        let mut home = building(ZoneType::Residential, 0.0, 1, 0, home_asset);
        home.center_x = 5.0;
        home.center_y = 20.0;
        home.frontage_t = 0.1;
        home.parcel_id = 10_001;
        home.cell_x = 102;
        allocator.buildings.push(home);
        let mut households = HouseholdSystem::new();
        households
            .households
            .push(housed_household(1, 30, 100_000.0, 10.0));
        if commitment == "live" {
            let mut incumbent = building(ZoneType::Industrial, 0.0, 0, 0, factory_asset.clone());
            incumbent.center_x = 25.0;
            incumbent.center_y = 45.0;
            incumbent.parcel_id = 10_002;
            incumbent.cell_x = 104;
            incumbent.operating_budget = 100_000.0;
            allocator.buildings.push(incumbent);
        }
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        allocator.prepare_building_site_query_index(5.0);
        let mut logistics = ShipmentSystem::new();
        households.job_funding.refresh(
            &allocator,
            &households.households,
            &mut logistics,
            &network,
            &graph,
            &catalog,
            &tuning,
            &policy,
            &[],
            100_000.0, 420,
        );
        assert_eq!(households.job_funding.capacities[0], farm_workers);
        if commitment == "live" {
            assert_eq!(households.job_funding.capacities[2], 10);
        }
        let actions: Vec<_> = zoning
            .parcels()
            .iter()
            .map(|parcel| DemandSpawnAction {
                parcel_id: parcel.id().raw(),
                asset_id: factory_asset.clone(),
            })
            .collect();
        if commitment == "queued" {
            demand.set_queued_construction_with_entrances(
                &allocator,
                &zoning,
                0,
                [(1, ZoneType::Industrial, &actions[0])],
                |action| {
                    allocator.demand_site_entrance(
                        action.parcel_id,
                        &action.asset_id,
                        &zoning,
                        &graph,
                        &network.lane_system,
                    )
                },
            );
        }
        let terrain = TerrainSystem::new(4, 4);
        let mut surface = RoadSurfaceSystem::new(512.0);
        surface.compile_dirty(&graph, &terrain);
        demand.run_hourly_pass_with_policy(
            &allocator,
            &households,
            &graph,
            &zoning,
            100_000.0,
            &policy,
            &network,
            &mut logistics,
            BuildingSiteEnvironment {
                road_surface: &surface,
                terrain: &terrain,
            }, 420,
        );
        if commitment != "new" {
            assert!(
                demand.building_actions.industrial.spawns.is_empty(),
                "{commitment} operation already owns the farm's 200 daily grain units"
            );
            continue;
        }
        assert_eq!(demand.building_actions.industrial.spawns.len(), 1);
        let chosen = demand.building_actions.industrial.spawns[0].clone();
        let other = actions
            .iter()
            .find(|action| action.parcel_id != chosen.parcel_id)
            .unwrap();
        // Dropped permits model a failed physical placement. Neither probe may keep inputs.
        for action in [&chosen, other, &chosen] {
            assert!(
                demand
                    .prepare_queued_spawn(
                        action,
                        ZoneType::Industrial,
                        &allocator,
                        &households,
                        &graph,
                        1,
                        &network,
                        &mut logistics,
                        &zoning,
                        100_000.0,
                        &policy
                    )
                    .is_ok()
            );
        }
        if commitment == "new" {
            let window = Arc::make_mut(&mut demand.runtime_tuning)
                .operational_clock.freight_profiles.iter_mut()
                .find(|profile| profile.id == "always_open").unwrap();
            let original = window.clone();
            window.preferred_windows[0].start_minute = 240;
            window.preferred_windows[0].end_minute = 480;
            window.outside_window_cost_multiplier = 10.0;
            for (minute, accepted) in [(479, true), (480, false), (479, true)] {
                let result = demand.prepare_queued_spawn(&chosen, ZoneType::Industrial,
                    &allocator, &households, &graph, minute, &network, &mut logistics,
                    &zoning, 100_000.0, &policy);
                assert_eq!(result.is_ok(), accepted,
                    "queued execution must reprice at a window boundary within the same execution hour");
            }
            *Arc::make_mut(&mut demand.runtime_tuning).operational_clock.freight_profiles.iter_mut()
                .find(|profile| profile.id == "always_open").unwrap() = original;
            demand.invalidate_construction_execution();
        }
        let permit = demand
            .prepare_queued_spawn(
                &chosen,
                ZoneType::Industrial,
                &allocator,
                &households,
                &graph,
                1,
                &network,
                &mut logistics,
                &zoning,
                100_000.0,
                &policy,
            )
            .unwrap();
        let placed = allocator.execute_single_demand_spawn_action(
            ZoneType::Industrial,
            &chosen,
            &mut zoning,
            &graph,
            &network.lane_system,
            &surface,
            &terrain,
            &catalog,
            &tuning,
        );
        assert_eq!(
            placed.industrial.spawn_executed, 1,
            "the viable project must physically place"
        );
        demand.record_queued_spawn_placement(permit, &chosen, &allocator);
        let funded = allocator.buildings.last().unwrap().operating_budget;
        assert!(funded > 0.0);
        assert!(allocator.buildings.last().unwrap().is_under_construction());
        for now in [2, 61] {
            assert_eq!(
                demand
                    .prepare_queued_spawn(
                        other,
                        ZoneType::Industrial,
                        &allocator,
                        &households,
                        &graph,
                        now,
                        &network,
                        &mut logistics,
                        &zoning,
                        100_000.0,
                        &policy
                    )
                    .err(),
                Some("market-cannot-fund-operation"),
                "placed input claims survive execution epoch changes"
            );
        }
        assert_eq!(allocator.buildings.len(), 3);
        assert_eq!(
            allocator.buildings.last().unwrap().operating_budget,
            funded,
            "rejected entry cannot spend additional owner money"
        );
    }
}

#[test]
fn live_local_inputs_can_fund_only_their_incumbents_upgrade() {
    use super::super::supply::SiteSupply;
    use crate::simulation::economy::logistics::{
        RecurringSupply, ShipmentSystem, collect_connected_border_nodes,
    };
    use crate::simulation::network::TransitNetwork;

    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 2.0;
    let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    let mut graph = graph_with_connected_border();
    let mut zoning = two_lots(&graph, ZoneType::Industrial);
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let terrain = TerrainSystem::new(4, 4);
    let mut surface = RoadSurfaceSystem::new(512.0);
    surface.compile_dirty(&graph, &terrain);
    let mut allocator = BuildingAllocator::new();
    let farm_asset =
        register_explicit_profile_asset(&mut allocator, "upgrade_grain", "grain_farm_basic");
    let old = register_family_asset(
        &mut allocator,
        "input_incumbent",
        ZoneType::Industrial,
        Some("input_family"),
        1,
    );
    let target = register_family_asset(
        &mut allocator,
        "input_replacement",
        ZoneType::Industrial,
        Some("input_family"),
        2,
    );
    let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
    let mut farm = building(ZoneType::None, 0.0, 0, 0, farm_asset);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 10.0;
    farm.worker_count =
        crate::simulation::economy::households::physical_worker_capacity_for_profile(
            &farm,
            farm_profile,
        );
    farm.operating_budget = 100_000.0;
    farm.center_x = 45.0;
    farm.center_y = 20.0;
    farm.frontage_t = 0.9;
    farm.parcel_id = 10_000;
    farm.cell_x = 100;
    allocator.buildings.push(farm);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.prepare_building_site_query_index(5.0);
    let actions: Vec<_> = zoning
        .parcels()
        .iter()
        .map(|p| DemandSpawnAction {
            parcel_id: p.id().raw(),
            asset_id: old.clone(),
        })
        .collect();
    for action in &actions {
        let placed = allocator.execute_single_demand_spawn_action(
            ZoneType::Industrial,
            action,
            &mut zoning,
            &graph,
            &network.lane_system,
            &surface,
            &terrain,
            &catalog,
            &tuning,
        );
        assert_eq!(placed.industrial.spawn_executed, 1);
        let factory = allocator.buildings.last_mut().unwrap();
        factory.construction_remaining_hours = 0;
        factory.worker_count = 10;
        factory.operating_budget = 100_000.0;
        for input in &catalog
            .profile_for_id("food_processor_basic")
            .unwrap()
            .inputs
        {
            factory.set_inventory_units(input.resource_runtime_id, 10_000.0);
        }
    }
    let households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    funding.refresh(
        &allocator,
        &[],
        &mut logistics,
        &network,
        &graph,
        &catalog,
        &tuning,
        &policy,
        &[],
        100_000.0, 420,
    );
    assert_eq!(&funding.capacities[1..], &[10, 0]);
    let mut borders = Vec::new();
    collect_connected_border_nodes(&graph, &mut borders);
    let mut projected = RecurringSupply::default();
    for reject in [false, true] {
        projected.copy_reservations_from(&funding.input_supply, &allocator, &catalog, &graph);
        logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &borders);
        let supply = SiteSupply::new(
            &mut projected,
            &allocator,
            &zoning,
            &network,
            &graph,
            &mut logistics,
            &tuning,
            &borders, &catalog, 420,
        );
        let mut snapshot = vacant_admission_snapshot();
        snapshot.open_job_slots = 0;
        snapshot.existing_unemployed_member_count = 0;
        snapshot.output_absorption = OutputAbsorptionContext::from_resource_amounts(
            catalog.resource_count(),
            &[
                (farm_profile.outputs[0].resource_runtime_id, 200.0),
                (
                    catalog.resource_runtime_id_for_id("packaged_food").unwrap(),
                    400.0,
                ),
            ],
            &[],
            0,
            &[],
        );
        snapshot
            .output_absorption
            .set_export_expansion_available(true);
        let mut test_policy = policy.clone();
        if reject {
            test_policy.property_tax_level_multiplier = 1_000_000.0;
        }
        let mut demand = DemandSystem::new();
        demand.upgrade_action_credit.industrial = 2.0;
        demand.plan_private_building_actions(
            &allocator,
            &graph,
            &zoning,
            &snapshot,
            &funding,
            &catalog,
            &tuning,
            &test_policy,
            &ResidentialOccupantSnapshot::from_runtime(&allocator, &households),
            1.0 / 24.0,
            BuildingSiteEnvironment {
                road_surface: &surface,
                terrain: &terrain,
            },
            supply,
        );
        let upgrades = &demand.building_actions.industrial.upgrades;
        assert_eq!(upgrades.len(), usize::from(!reject));
        if !reject {
            assert_eq!(upgrades[0].target_asset_id, target);
            assert_eq!(upgrades[0].building.parcel_id, actions[0].parcel_id);
        }
        assert!(
            !projected.offers_inputs(catalog.profile_for_id("food_processor_basic").unwrap()),
            "accepted replacements retain inputs; failed ones restore incumbents"
        );
    }
}

#[test]
fn constructing_areas_and_services_share_finite_inputs_with_actual_capital() {
    use super::super::supply::SiteSupply;
    use crate::simulation::economy::hiring::JobFunding;
    use crate::simulation::economy::logistics::{RecurringSupply, ShipmentSystem, collect_connected_border_nodes};
    use crate::simulation::economy::operation::InputRequirement;
    use crate::simulation::network::TransitNetwork;

    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 2.0;
    let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    let mut graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let mut allocator = BuildingAllocator::new();
    for (index, (profile_id, area)) in [
        ("machinery_factory_basic", 1.0),
        ("grain_farm_basic", 10.0),
        ("coal_mine_basic", 1.0),
        ("water_plant_basic", 1.0),
    ].into_iter().enumerate() {
        let asset = if index == 3 {
            register_test_utility_asset(&mut allocator, "pending_water", profile_id)
        } else {
            register_explicit_profile_asset(&mut allocator, profile_id, profile_id)
        };
        let profile = catalog.profile_for_id(profile_id).unwrap();
        let mut site = building(if index == 0 { ZoneType::Industrial } else { ZoneType::None }, 0.0, 0, 0, asset);
        site.economy_profile_runtime_id = profile.runtime_id;
        site.work_area_scale = area;
        site.worker_count = if index == 0 { profile.worker_capacity } else { 0 };
        site.center_x = 10.0 + index as f32 * 10.0;
        site.center_y = if index == 0 { 20.0 } else { -20.0 };
        site.frontage_t = site.center_x / 50.0;
        site.parcel_id = 10_000 + index as u64;
        site.construction_remaining_hours = if index == 0 { 0 } else { 10 };
        site.construction_total_hours = site.construction_remaining_hours;
        allocator.buildings.push(site);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let coal = catalog.resource_runtime_id_for_id("coal").unwrap();
    let mut market = OutputAbsorptionContext::from_resource_amounts(
        catalog.resource_count(), &[(grain, 200.0), (coal, 9600.0)], &[], 1, &[],
    );
    market.set_export_expansion_available(true);
    let service_door = allocator.entrances[3].clone();
    let mut borders = Vec::new();
    collect_connected_border_nodes(&graph, &mut borders);
    for (cash, paid_cargo, reachable) in [(100_000.0, 0.0, true), (0.0, 0.0, true), (0.0, 100_000.0, true), (100_000.0, 0.0, false)] {
        allocator.buildings[1].operating_budget = cash;
        allocator.buildings[2].operating_budget = cash;
        allocator.buildings[3].operating_budget = 0.0;
        allocator.entrances[3] = service_door.clone();
        if !reachable {
            allocator.entrances[3].car_lane_fwd = usize::MAX;
            allocator.entrances[3].car_lane_bkw = usize::MAX;
        }
        let mut logistics = ShipmentSystem::new();
        let mut funding = JobFunding::default();
        funding.refresh(&allocator, &[], &mut logistics, &network, &graph, &catalog, &tuning, &policy, &[], 100_000.0, 420);
        assert_eq!(&funding.capacities[1..], &[0, 0, 0], "unfinished projects never advertise live jobs");
        assert_eq!(&funding.service_capacities[1..], &[0, 0, 3]);
        // Isolate project replay: a previously validated staffed source offers 40/day.
        let mut book = RecurringSupply::default();
        book.reset(&allocator, &graph, &network, &catalog, &[1.0, 0.0, 0.0, 0.0]);
        logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &borders);
        SiteSupply::new(&mut book, &allocator, &zoning, &network, &graph, &mut logistics, &tuning, &borders, &catalog, 420)
            .reserve_projects(std::iter::empty(), &catalog, &market, &mut Default::default(), &policy,
                &funding.service_capacities, &[0.0, paid_cargo, paid_cargo, 0.0]);
        // Ten hectares need 1 machinery/day; the one-hectare mine needs 4. The public
        // water plant needs 0.5 independently of private capital, if its door is reachable.
        let residual = 40.0 - if f64::from(cash) + paid_cargo > 0.0 { 5.0 } else { 0.0 }
            - if reachable { 0.5 } else { 0.0 };
        for (units, expected) in [(residual, true), (residual + 0.25, false)] {
            let costs = logistics.quote_recurring_input(&mut book, InputRequirement {
                resource: machinery, daily_units: units, buffer_units: 2.0 * units,
                replenishment_batch_units: Some(40.0),
            }, None, &service_door, None, &allocator, &catalog, &tuning, &network, &graph, 1.0);
            assert_eq!(costs.goods.is_finite(), expected, "cash={cash} cargo={paid_cargo} reachable={reachable} residual={residual}");
            book.reject(&allocator, &graph);
        }
        assert_eq!(allocator.buildings[1].operating_budget, cash);
        assert_eq!(allocator.buildings[3].operating_budget, 0.0);
    }
}

#[test]
fn constructing_shops_replay_inputs_in_admission_order() {
    use super::super::supply::SiteSupply;
    use crate::simulation::economy::logistics::{
        RecurringSupply, ShipmentSystem, collect_connected_border_nodes,
    };
    use crate::simulation::economy::operation::InputRequirement;
    use crate::simulation::network::TransitNetwork;
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 2.0;
    let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    let mut graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let mut allocator = BuildingAllocator::new();
    let factory = register_test_asset(&mut allocator, "replay_factory", ZoneType::Industrial);
    let grocery = register_test_asset(&mut allocator, "replay_grocery", ZoneType::Commercial);
    for index in 0..4 {
        let constructing = index >= 2;
        let mut site = building(
            if constructing {
                ZoneType::Commercial
            } else {
                ZoneType::Industrial
            },
            0.0,
            0,
            if constructing { 0 } else { 10 },
            if constructing {
                grocery.clone()
            } else {
                factory.clone()
            },
        );
        site.center_x = 10.0 + index as f32 * 10.0;
        site.center_y = if constructing { -20.0 } else { 20.0 };
        site.frontage_t = site.center_x / 50.0;
        site.parcel_id = 10_000 + index as u64;
        site.construction_remaining_hours = if constructing { 10 } else { 0 };
        site.construction_total_hours = site.construction_remaining_hours;
        allocator.buildings.push(site);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let food = catalog.resource_runtime_id_for_id("packaged_food").unwrap();
    let supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .unwrap();
    let market = OutputAbsorptionContext::from_resource_amounts(
        catalog.resource_count(),
        &[(food, 400.0), (supplies, 400.0)],
        &[(supplies, 300.0)],
        1,
        &[],
    );
    let mut book = RecurringSupply::default();
    // Isolate replay from upstream funding: two validated staffed suppliers offer 400/day.
    book.reset(&allocator, &graph, &network, &catalog, &[1.0, 1.0, 0.0, 0.0]);
    let mut logistics = ShipmentSystem::new();
    let mut borders = Vec::new();
    collect_connected_border_nodes(&graph, &mut borders);
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &borders);
    SiteSupply::new(
        &mut book,
        &allocator,
        &zoning,
        &network,
        &graph,
        &mut logistics,
        &tuning,
        &borders, &catalog, 420,
    )
    .reserve_projects(std::iter::empty(), &catalog, &market, &mut Default::default(), &policy, &[], &[]);
    // The first shop owns 200/day; the next owns the residual 100/day. Crediting only
    // each shop against the final 400-capacity market would incorrectly reserve 100+100.
    for (units, expected) in [(100.0, true), (101.0, false)] {
        let costs = logistics.quote_recurring_input(
            &mut book,
            InputRequirement {
                resource: food,
                daily_units: units,
                buffer_units: 2.0 * units,
                replenishment_batch_units: Some(40.0),
            },
            None,
            &allocator.entrances[2],
            None,
            &allocator,
            &catalog,
            &tuning,
            &network,
            &graph, 1.0,
        );
        assert_eq!(costs.goods.is_finite(), expected);
        book.reject(&allocator, &graph);
    }
    assert_eq!(
        market.unmet_units(supplies),
        0.0,
        "all full output commitments remain reserved"
    );
}

#[test]
fn capital_uses_quoted_input_values_without_double_counting_internal_output() {
    use crate::simulation::economy::operation::working_capital_assets_at_prices;
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let mut factory = building(ZoneType::Industrial, 0.0, 0, 4, "test:factory".to_owned());
    factory.operating_budget = 100.0;
    let steel = catalog.resource_runtime_id_for_id("steel").unwrap();
    let metals = catalog.resource_runtime_id_for_id("metals").unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    factory.set_inventory_units(steel, 10.0);
    factory.set_inventory_units(metals, 20.0);
    factory.set_inventory_units(machinery, 30.0);
    let assets = working_capital_assets_at_prices(
        &factory,
        profile,
        50.0,
        |resource| {
            if resource == steel {
                2.0
            } else if resource == metals {
                3.0
            } else {
                4.0
            }
        },
        |_| panic!("self-produced inputs must not also receive an output value"),
    );
    assert_eq!(assets, 100.0 + 50.0 + 10.0 * 2.0 + 20.0 * 3.0 + 30.0 * 4.0);
    assert_eq!(factory.operating_budget, 100.0);
}

#[test]
fn housing_selection_uses_actual_capacity_and_leaves_other_offers_for_workplaces() {
    for (capacities, need, selected_count) in [
        ([1, 1, 12], 3, 3),
        ([12, 1, 1], 3, 1),
        ([6, 1, 1], 7, 2),
        ([6, 1, 1], 0, 0),
    ] {
        let mut allocator = BuildingAllocator::new();
        let candidates: Vec<_> = capacities
            .into_iter()
            .enumerate()
            .map(|(i, capacity)| {
                let asset = register_test_asset(
                    &mut allocator,
                    &format!("home_{i}"),
                    ZoneType::Residential,
                );
                let mut manifest = allocator.registry.get(&asset).unwrap().manifest.clone();
                manifest.building.as_mut().unwrap().household_capacity = Some(capacity);
                allocator.registry.register("test", manifest, String::new());
                DemandSpawnCandidate {
                    action: DemandSpawnAction {
                        parcel_id: i as u64,
                        asset_id: asset,
                    },
                }
            })
            .collect();
        let mut optional = Vec::new();
        let selected = select_residential_spawns(&allocator, candidates, need, &mut optional);
        assert_eq!(selected.len(), selected_count);
        assert_eq!(optional.len(), capacities.len() - selected_count);
        assert!(
            selected
                .iter()
                .map(|home| allocator.registry.household_capacity(&home.asset_id))
                .sum::<u32>()
                >= need
        );
        for (i, home) in selected.iter().chain(&optional).enumerate() {
            assert_eq!(
                home.parcel_id, i as u64,
                "attachment order must remain stable"
            );
        }
    }
}

fn two_lots(graph: &RegionGraph, zone: ZoneType) -> ZoningSystem {
    let zoning = zoning_run(graph, zone);
    assert_eq!(zoning.parcels().len(), 2);
    zoning
}

fn plan(
    allocator: &BuildingAllocator,
    graph: &RegionGraph,
    zoning: &ZoningSystem,
    workers: u32,
    sales: &[(&str, f32)],
) -> DemandSystem {
    let tuning = load_runtime_economy_tuning().unwrap();
    plan_with_quote(allocator, graph, zoning, workers, sales, |_, _| {
        Some(tuning.logistics.dispatch_fee_currency)
    })
}

fn plan_with_quote(
    allocator: &BuildingAllocator,
    graph: &RegionGraph,
    zoning: &ZoningSystem,
    workers: u32,
    sales: &[(&str, f32)],
    quote: impl Fn(u64, &str) -> Option<f32>,
) -> DemandSystem {
    plan_with_supply(allocator, graph, zoning, workers, sales, quote)
}

fn plan_with_supply(
    allocator: &BuildingAllocator,
    graph: &RegionGraph,
    zoning: &ZoningSystem,
    workers: u32,
    sales: &[(&str, f32)],
    supply: impl super::super::supply::ProjectSupply,
) -> DemandSystem {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let demand_by_resource: Vec<_> = sales
        .iter()
        .map(|(resource, units)| {
            (
                catalog.resource_runtime_id_for_id(resource).unwrap(),
                *units,
            )
        })
        .collect();
    let mut snapshot = vacant_admission_snapshot();
    snapshot.open_job_slots = 0;
    snapshot.existing_unemployed_member_count = workers;
    snapshot.output_absorption = OutputAbsorptionContext::from_resource_amounts(
        catalog.resource_count(),
        &[],
        &demand_by_resource,
        1,
        &[],
    );
    snapshot
        .output_absorption
        .set_export_expansion_available(true);
    snapshot.committed_unmet_commercial_consumer_demand_by_resource = demand_by_resource;
    let terrain = TerrainSystem::new(4, 4);
    let mut surface = RoadSurfaceSystem::new(512.0);
    surface.compile_dirty(graph, &terrain);
    let mut demand = DemandSystem::new();
    demand.plan_private_building_actions(
        allocator,
        graph,
        zoning,
        &snapshot,
        &Default::default(),
        &catalog,
        &tuning,
        &CityFiscalPolicy::default(),
        &ResidentialOccupantSnapshot::from_runtime(allocator, &HouseholdSystem::new()),
        1.0 / 24.0,
        BuildingSiteEnvironment {
            road_surface: &surface,
            terrain: &terrain,
        },
        supply,
    );
    demand
}

#[test]
fn alternative_reservations_commit_the_winner_after_later_rejected_probes() {
    use super::super::supply::ProjectSupply;
    use crate::simulation::economy::operation::{InputRequirement, InputSupplyCosts};
    use std::{cell::RefCell, rc::Rc};

    struct Probe {
        viable: String,
        staged: String,
        committed: Rc<RefCell<Vec<String>>>,
    }
    impl ProjectSupply for Probe {
        fn begin(&mut self, _: u64, asset: &str) -> Option<f32> {
            self.staged = asset.to_owned();
            (asset == self.viable).then_some(4.0)
        }
        fn quote(
            &mut self,
            input: InputRequirement,
            catalog: &crate::simulation::economy::definitions::RuntimeEconomyCatalog,
            prices: OperationPrices,
        ) -> InputSupplyCosts {
            input.imported(catalog, prices)
        }
        fn accept(&mut self) {
            self.committed
                .borrow_mut()
                .push(std::mem::take(&mut self.staged));
        }
        fn reject(&mut self) {
            self.staged.clear();
        }
    }

    let mut allocator = BuildingAllocator::new();
    for id in ["probe_a", "probe_b", "probe_c", "probe_d"] {
        register_test_asset(&mut allocator, id, ZoneType::Industrial);
    }
    let graph = graph_with_connected_border();
    let zoning = two_lots(&graph, ZoneType::Industrial);
    let catalog = load_runtime_economy_catalog().unwrap();
    let terrain = TerrainSystem::new(4, 4);
    let mut surface = RoadSurfaceSystem::new(512.0);
    surface.compile_dirty(&graph, &terrain);
    let environment = BuildingSiteEnvironment {
        road_surface: &surface,
        terrain: &terrain,
    };
    let candidates = allocator.collect_demand_spawn_candidates_by_use(
        &zoning,
        &graph,
        &catalog,
        &[],
        environment,
    );
    let preferred = &candidates.industrial[0].action;
    let assets = allocator.collect_spawn_asset_candidates_by_profile(&zoning, &catalog);
    let mut order = Vec::new();
    allocator.select_demand_spawn_alternative(
        preferred,
        &assets,
        &zoning,
        &graph,
        &catalog,
        &[],
        environment,
        |asset| {
            order.push(asset.to_owned());
            false
        },
    );
    assert!(order.len() >= 2);
    let viable = order[0].clone();
    let committed = Rc::new(RefCell::new(Vec::new()));
    let demand = plan_with_supply(
        &allocator,
        &graph,
        &zoning,
        10,
        &[],
        Probe {
            viable: viable.clone(),
            staged: String::new(),
            committed: Rc::clone(&committed),
        },
    );
    assert_eq!(demand.building_actions.industrial.spawns.len(), 1);
    assert_eq!(
        demand.building_actions.industrial.spawns[0].asset_id,
        viable
    );
    assert_eq!(committed.borrow().as_slice(), &[viable]);
}

#[test]
fn project_supply_must_fit_opening_equity_without_counting_equity_as_sales() {
    use super::super::spawn_need::market_covers_operation_with_input_quotes;
    use crate::simulation::economy::operation::InputSupplyCosts;
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let mut market = OutputAbsorptionContext::empty(catalog.resource_count());
    market.set_export_expansion_available(true);
    let prices = OperationPrices {
        utilities: [0.0; 3],
        property_tax: 0.0,
        imports: 1.0,
        carrier_quote: 4.0,
        truck_payload: 20_000.0,
    };
    let covers = |goods, buffer| {
        market_covers_operation_with_input_quotes(
            profile,
            &catalog,
            market.market(),
            prices,
            1.0,
            |_| InputSupplyCosts {
                goods,
                buffer,
                freight: 0.0,
            },
        )
    };
    assert!(covers(0.0, 0.0));
    assert!(
        !covers(0.0, 1_000_000.0),
        "profitable operation still needs its input buffers funded"
    );
    assert!(
        !covers(100_000.0, 0.0),
        "opening equity cannot subsidize recurring losses"
    );
}

#[test]
fn growth_records_distinguish_missing_land_assets_and_unusable_sites() {
    let mut allocator = BuildingAllocator::new();
    let graph = graph_with_connected_border();
    let zoning = two_lots(&graph, ZoneType::Commercial);
    let missing = plan(&allocator, &graph, &zoning, 8, &[]);
    let record = missing.last_building_action_diagnostics.commercial;
    assert!(record.assessed);
    assert!(!record.sites.has_assets);
    assert_eq!(record.sites.free_lots, 2);
    assert_eq!(record.sites.missing_asset_lots, 2);
    assert_eq!(record.spawn_candidates, 0);

    register_test_asset(&mut allocator, "grocery", ZoneType::Commercial);
    let no_land = plan(&allocator, &graph, &empty_zoning(), 8, &[]);
    let record = no_land.last_building_action_diagnostics.commercial;
    assert!(record.sites.has_assets);
    assert_eq!(record.sites.free_lots, 0);

    // Stale road references are unavailable sites, not missing assets or missing zoning.
    let no_road = plan(&allocator, &RegionGraph::new(), &zoning, 8, &[]);
    let record = no_road.last_building_action_diagnostics.commercial;
    assert_eq!(record.sites.free_lots, 2);
    assert_eq!(record.sites.unusable_lots, 2);
    assert_eq!(record.sites.missing_asset_lots, 0);
    assert_eq!(record.spawn_candidates, 0);

    let market = plan(&allocator, &graph, &zoning, 8, &[]);
    let record = market.last_building_action_diagnostics.commercial;
    assert_eq!(record.sites.unusable_lots, 0);
    assert_eq!(record.spawn_candidates, 2);
    assert_eq!(record.spawn_rejected_absorption, 2);
}

#[test]
fn losing_priority_category_does_not_hide_viable_service() {
    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "grocery", ZoneType::Commercial);
    let barber = register_family_asset_with_economy_profile(
        &mut allocator,
        "barber",
        ZoneType::Commercial,
        None,
        1,
        Some("personal_service_small"),
    );
    let graph = graph_with_connected_border();
    let zoning = two_lots(&graph, ZoneType::Commercial);
    // Groceries rank first by unmet sales value (150 > 100), but six daily
    // customers cannot cover that store's inputs and first employee's wages.
    let demand = plan(
        &allocator,
        &graph,
        &zoning,
        8,
        &[("household_supplies", 6.0), ("personal_services", 5.0)],
    );
    let selected = &demand.building_actions.commercial.spawns;
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].asset_id, barber);
    assert_eq!(
        demand
            .last_building_action_diagnostics
            .commercial
            .spawn_rejected_viability,
        1
    );
}

#[test]
fn selected_category_releases_other_lots_to_remaining_categories() {
    let mut allocator = BuildingAllocator::new();
    let barber = register_family_asset_with_economy_profile(
        &mut allocator,
        "barber",
        ZoneType::Commercial,
        None,
        1,
        Some("personal_service_small"),
    );
    let pharmacy = register_family_asset_with_economy_profile(
        &mut allocator,
        "pharmacy",
        ZoneType::Commercial,
        None,
        1,
        Some("health_essentials_small"),
    );
    let graph = graph_with_connected_border();
    let zoning = two_lots(&graph, ZoneType::Commercial);
    let demand = plan(
        &allocator,
        &graph,
        &zoning,
        9,
        &[("personal_services", 80.0), ("health_essentials", 100.0)],
    );
    let selected = &demand.building_actions.commercial.spawns;
    assert_eq!(selected.len(), 2);
    assert_eq!(selected[0].asset_id, barber);
    assert_eq!(selected[1].asset_id, pharmacy);
    assert_ne!(selected[0].parcel_id, selected[1].parcel_id);
}

#[test]
fn scarce_workforce_can_select_smaller_factory_on_the_same_lot() {
    let mut allocator = BuildingAllocator::new();
    let a = register_test_asset(&mut allocator, "factory_a", ZoneType::Industrial);
    let b = register_test_asset(&mut allocator, "factory_b", ZoneType::Industrial);
    let graph = graph_with_connected_border();
    let zoning = industrial_zoning_run(&graph);
    let catalog = load_runtime_economy_catalog().unwrap();
    let terrain = TerrainSystem::new(4, 4);
    let mut surface = RoadSurfaceSystem::new(512.0);
    surface.compile_dirty(&graph, &terrain);
    let candidates = allocator.collect_demand_spawn_candidates_by_use(
        &zoning,
        &graph,
        &catalog,
        &[],
        BuildingSiteEnvironment {
            road_surface: &surface,
            terrain: &terrain,
        },
    );
    let preferred = &candidates.industrial[0].action;
    // Keep the hash-preferred ten-worker factory, and give its alternative the
    // ordinary four-worker recipe. Family/variant hashes and geometry are unchanged.
    let alternative_id = if preferred.asset_id == a { &b } else { &a };
    register_family_asset_with_economy_profile(
        &mut allocator,
        alternative_id.strip_prefix("test:").unwrap(),
        ZoneType::Industrial,
        None,
        1,
        Some("machinery_factory_basic"),
    );
    let demand = plan(&allocator, &graph, &zoning, 4, &[]);
    let selected = &demand.building_actions.industrial.spawns;
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].asset_id, *alternative_id);
    assert_eq!(selected[0].parcel_id, preferred.parcel_id);
    assert_eq!(
        plan(&allocator, &graph, &zoning, 3, &[])
            .building_actions
            .industrial
            .spawns
            .len(),
        0
    );
}

#[test]
fn export_openings_use_shared_labor_instead_of_one_project_quota() {
    let mut allocator = BuildingAllocator::new();
    register_family_asset_with_economy_profile(
        &mut allocator,
        "machinery",
        ZoneType::Industrial,
        None,
        1,
        Some("machinery_factory_basic"),
    );
    let graph = graph_with_connected_border();
    let zoning = two_lots(&graph, ZoneType::Industrial);
    for (workers, projects) in [(3, 0), (4, 1), (7, 1), (8, 2)] {
        let demand = plan(&allocator, &graph, &zoning, workers, &[]);
        assert_eq!(
            demand.building_actions.industrial.spawns.len(),
            projects,
            "each selected project must reserve four workers from {workers}"
        );
    }
}

#[test]
fn upgrades_share_ready_workers_and_preserve_incumbent_job_reservations() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    // Existing funded vacancies have already claimed workers. Do not claim them twice.
    // Conversely, a future home cannot supply an upgrade applied immediately this hour.
    for (previous_id, assigned, funded, unemployed, open_jobs, future_home, selected) in [
        ("machinery_factory_basic", 4, 4, 6, 0, false, 1),
        ("food_processor_basic", 9, 10, 2, 2, false, 2),
        ("food_processor_basic", 9, 0, 2, 0, false, 2),
        ("machinery_factory_basic", 4, 4, 0, 0, true, 0),
    ] {
        let mut allocator = BuildingAllocator::new();
        let previous = catalog.profile_for_id(previous_id).unwrap();
        let old_asset = register_family_asset_with_economy_profile(
            &mut allocator,
            "old_factory",
            ZoneType::Industrial,
            Some("upgrade_factory"),
            1,
            Some(previous_id),
        );
        let target = register_family_asset_with_economy_profile(
            &mut allocator,
            "larger_factory",
            ZoneType::Industrial,
            Some("upgrade_factory"),
            2,
            Some("food_processor_basic"),
        );
        for index in 0..2 {
            let mut factory = building(ZoneType::Industrial, 0.0, 0, assigned, old_asset.clone());
            factory.economy_profile_runtime_id = previous.runtime_id;
            factory.operating_budget = 100_000.0;
            if previous_id == "food_processor_basic" && funded > 0 {
                use crate::simulation::economy::operation::{OperatingCosts, OperationPlan};
                let prices = OperationPrices {
                    utilities: crate::simulation::economy::households::local_utility_service_costs(
                        &catalog,
                    )
                    .map(|p| p * tuning.owa_import_price_multiplier),
                    property_tax: crate::simulation::economy::fiscal::daily_property_tax(
                        ZoneType::Industrial,
                        2,
                        &CityFiscalPolicy::default(),
                    ),
                    imports: tuning.owa_import_price_multiplier,
                    carrier_quote: tuning.logistics.dispatch_fee_currency,
                    truck_payload: tuning.logistics.truck_payload_kg,
                };
                let costs = OperatingCosts::for_plan(
                    previous,
                    &catalog,
                    OperationPlan {
                        workers: previous.worker_capacity,
                        activity: 1.0,
                        area: 1.0,
                    },
                    prices,
                    |_, units| units,
                );
                // Inputs below are already paid. Keep cash for one operating cycle, not an
                // additional arbitrary four-day reserve on top of the funded stock.
                factory.operating_budget = costs.payroll + costs.overhead + costs.freight + 1.0;
            }
            factory.parcel_id = index;
            factory.cell_x = index as usize * 2;
            for input in &previous.inputs {
                factory.set_inventory_units(
                    input.resource_runtime_id,
                    previous.input_inventory_targets(input, 1.0).0,
                );
            }
            allocator.buildings.push(factory);
        }
        if future_home {
            let home = register_test_asset(&mut allocator, "future_home", ZoneType::Residential);
            let mut home = building(ZoneType::Residential, 0.0, 0, 0, home);
            home.construction_remaining_hours = 1;
            home.construction_total_hours = 1;
            allocator.buildings.push(home);
        }
        let mut snapshot = vacant_admission_snapshot();
        snapshot.existing_unemployed_member_count = unemployed;
        snapshot.open_job_slots = open_jobs;
        snapshot.output_absorption = OutputAbsorptionContext::from_resource_amounts(
            catalog.resource_count(),
            &previous
                .outputs
                .iter()
                .map(|p| (p.resource_runtime_id, p.units_per_day * 2.0))
                .collect::<Vec<_>>(),
            &[],
            0,
            &[],
        );
        snapshot
            .output_absorption
            .set_export_expansion_available(true);
        let graph = graph_with_connected_border();
        let terrain = TerrainSystem::new(4, 4);
        let mut surface = RoadSurfaceSystem::new(512.0);
        surface.compile_dirty(&graph, &terrain);
        let mut funding = crate::simulation::economy::hiring::JobFunding::default();
        funding.capacities = vec![funded, funded];
        let mut demand = DemandSystem::new();

        demand.upgrade_action_credit.industrial = 2.0;
        demand.plan_private_building_actions(
            &allocator,
            &graph,
            &empty_zoning(),
            &snapshot,
            &funding,
            &catalog,
            &tuning,
            &CityFiscalPolicy::default(),
            &ResidentialOccupantSnapshot::from_runtime(&allocator, &HouseholdSystem::new()),
            1.0 / 24.0,
            BuildingSiteEnvironment {
                road_surface: &surface,
                terrain: &terrain,
            },
            |_: u64, _: &str| Some(tuning.logistics.dispatch_fee_currency),
        );
        let upgrades = &demand.building_actions.industrial.upgrades;
        assert_eq!(
            upgrades.len(),
            selected,
            "{previous_id}: {assigned}/{funded}, unemployed {unemployed}, future home {future_home}"
        );
        assert!(
            upgrades
                .iter()
                .all(|action| action.target_asset_id == target)
        );
        assert!(
            demand.upgrade_action_credit.industrial >= (2 - selected) as f32,
            "rejected proposals cannot consume pacing credit"
        );
    }
}

#[test]
fn upgrade_requires_target_working_capital_but_counts_stock_and_paid_cargo() {
    use super::super::spawn_need::{
        industrial_market_covers_operation, replacement_market_covers_operation,
    };
    use crate::simulation::economy::fiscal::daily_property_tax;
    use crate::simulation::economy::households::local_utility_service_costs;
    use crate::simulation::economy::operation::{OperatingCosts, OperationPlan};

    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let mut absorption = OutputAbsorptionContext::empty(catalog.resource_count());
    absorption.set_export_expansion_available(true);
    let prices = OperationPrices {
        utilities: local_utility_service_costs(&catalog)
            .map(|p| p * tuning.owa_import_price_multiplier),
        property_tax: daily_property_tax(ZoneType::Industrial, 2, &CityFiscalPolicy::default()),
        imports: tuning.owa_import_price_multiplier,
        carrier_quote: tuning.logistics.dispatch_fee_currency,
        truck_payload: tuning.logistics.truck_payload_kg,
    };
    let costs = OperatingCosts::for_plan(
        profile,
        &catalog,
        OperationPlan {
            workers: profile.worker_capacity,
            activity: 1.0,
            area: 1.0,
        },
        prices,
        |_, units| units,
    );
    assert!(industrial_market_covers_operation(
        profile,
        &catalog,
        &absorption,
        prices,
        tuning.owa_export_price_multiplier
    ));
    let mut factory = building(
        ZoneType::Industrial,
        0.0,
        0,
        profile.worker_capacity,
        String::new(),
    );
    let funded = |factory: &Building, paid_cargo| {
        replacement_market_covers_operation(
            profile,
            &catalog,
            absorption.replacement_market(profile, 1.0),
            prices,
            tuning.owa_export_price_multiplier,
            factory,
            paid_cargo,
            &mut |_: u64, _: &str| Some(prices.carrier_quote),
        )
    };
    factory.operating_budget = 0.0;
    assert!(
        !funded(&factory, 0.0),
        "profitable sales do not buy the target's initial input buffer"
    );
    factory.operating_budget = costs.working_capital + 1.0;
    assert!(funded(&factory, 0.0));
    factory.operating_budget = costs.payroll + costs.overhead + costs.freight + 1.0;
    for input in &profile.inputs {
        factory.set_inventory_units(
            input.resource_runtime_id,
            profile.input_inventory_targets(input, 1.0).0,
        );
    }
    assert!(
        funded(&factory, 0.0),
        "owned usable inputs cannot require a second cash payment"
    );
    factory.resource_inventory.fill(0.0);
    assert!(!funded(&factory, 0.0));
    let paid_inputs = f64::from(costs.working_capital - factory.operating_budget + 1.0);
    assert!(
        funded(&factory, paid_inputs),
        "same-recipe paid cargo remains working capital"
    );

    absorption.set_export_expansion_available(false);
    factory.operating_budget = 1_000_000.0;
    assert!(
        !replacement_market_covers_operation(
            profile,
            &catalog,
            absorption.replacement_market(profile, 1.0),
            prices,
            tuning.owa_export_price_multiplier,
            &factory,
            0.0,
            &mut |_: u64, _: &str| Some(prices.carrier_quote),
        ),
        "owner cash cannot substitute for recurring customers"
    );
}

#[test]
fn business_downgrades_require_recovery_and_share_workers_and_customers() {
    use crate::simulation::economy::fiscal::daily_property_tax;
    use crate::simulation::economy::households::local_utility_service_costs;
    use crate::simulation::economy::operation::{OperatingCosts, OperationPlan};

    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let target = catalog.profile_for_id("machinery_factory_basic").unwrap();
    // All sites satisfy the old low-staffing/low-input downgrade trigger. Selection must
    // preserve a supportable incumbent and reserve a viable replacement's actual opportunity.
    for (
        name,
        previous_id,
        cash,
        stocked,
        expensive_tier,
        exports,
        workers,
        future_home,
        expected,
    ) in [
        (
            "healthy",
            "machinery_factory_basic",
            100_000.0,
            false,
            false,
            true,
            8,
            false,
            0,
        ),
        (
            "paid inputs",
            "machinery_factory_basic",
            0.0,
            true,
            false,
            true,
            8,
            false,
            0,
        ),
        (
            "recover at lower tax",
            "machinery_factory_basic",
            100_000.0,
            false,
            true,
            true,
            8,
            false,
            2,
        ),
        (
            "shared workers",
            "machinery_factory_basic",
            100_000.0,
            false,
            true,
            true,
            4,
            false,
            1,
        ),
        (
            "future workers",
            "machinery_factory_basic",
            100_000.0,
            false,
            true,
            true,
            0,
            true,
            0,
        ),
        (
            "no working capital",
            "machinery_factory_basic",
            0.0,
            false,
            true,
            true,
            8,
            false,
            0,
        ),
        (
            "no customers",
            "machinery_factory_basic",
            100_000.0,
            false,
            true,
            false,
            8,
            false,
            0,
        ),
        (
            "shared customers",
            "food_processor_basic",
            100_000.0,
            false,
            false,
            false,
            8,
            false,
            1,
        ),
    ] {
        let previous = catalog.profile_for_id(previous_id).unwrap();
        let mut allocator = BuildingAllocator::new();
        let target_asset = register_family_asset_with_economy_profile(
            &mut allocator,
            "smaller_factory",
            ZoneType::Industrial,
            Some("recover_factory"),
            1,
            Some("machinery_factory_basic"),
        );
        let old_asset = register_family_asset_with_economy_profile(
            &mut allocator,
            "large_factory",
            ZoneType::Industrial,
            Some("recover_factory"),
            2,
            Some(previous_id),
        );
        let mut policy = CityFiscalPolicy::default();
        let base_prices = OperationPrices {
            utilities: local_utility_service_costs(&catalog)
                .map(|price| price * tuning.owa_import_price_multiplier),
            property_tax: 0.0,
            imports: tuning.owa_import_price_multiplier,
            carrier_quote: tuning.logistics.dispatch_fee_currency,
            truck_payload: tuning.logistics.truck_payload_kg,
        };
        let plan = OperationPlan {
            workers: target.worker_capacity,
            activity: 1.0,
            area: 1.0,
        };
        if expensive_tier {
            let costs =
                OperatingCosts::for_plan(target, &catalog, plan, base_prices, |_, units| units);
            let sales: f32 = target
                .outputs
                .iter()
                .map(|output| {
                    target.net_output_units_per_day(output)
                        * catalog
                            .unit_price_for_resource(output.resource_runtime_id)
                            .unwrap()
                        * tuning.owa_export_price_multiplier
                })
                .sum();
            let margin = sales - costs.total();
            assert!(margin > 0.0);
            policy.industrial_property_tax_per_building_per_day = margin / 2.0;
            policy.property_tax_level_multiplier = 3.0;
        }
        for index in 0..2 {
            let mut factory = building(ZoneType::Industrial, 0.0, 0, 0, old_asset.clone());
            factory.level = 2;
            factory.economy_profile_runtime_id = previous.runtime_id;
            factory.operating_budget = cash;
            factory.parcel_id = index;
            factory.cell_x = index as usize * 2;
            if stocked {
                let costs = OperatingCosts::for_plan(
                    previous,
                    &catalog,
                    plan,
                    OperationPrices {
                        property_tax: daily_property_tax(ZoneType::Industrial, 2, &policy),
                        ..base_prices
                    },
                    |_, units| units,
                );
                factory.operating_budget = costs.payroll + costs.overhead + costs.freight + 1.0;
                for input in &previous.inputs {
                    factory.set_inventory_units(
                        input.resource_runtime_id,
                        previous.input_inventory_targets(input, 1.0).0,
                    );
                }
            }
            allocator.buildings.push(factory);
        }
        if future_home {
            let asset = register_test_asset(&mut allocator, "future_home", ZoneType::Residential);
            let mut home = building(ZoneType::Residential, 0.0, 0, 0, asset);
            home.construction_remaining_hours = 1;
            home.construction_total_hours = 1;
            allocator.buildings.push(home);
        }
        let mut snapshot = vacant_admission_snapshot();
        snapshot.existing_unemployed_member_count = workers;
        snapshot.open_job_slots = 0;
        let customers: Vec<_> = if previous_id != "machinery_factory_basic" {
            target
                .outputs
                .iter()
                .map(|output| {
                    (
                        output.resource_runtime_id,
                        target.net_output_units_per_day(output),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        snapshot.output_absorption = OutputAbsorptionContext::from_resource_amounts(
            catalog.resource_count(),
            &[],
            &[],
            0,
            &customers,
        );
        snapshot
            .output_absorption
            .set_export_expansion_available(exports);
        let graph = graph_with_connected_border();
        let terrain = TerrainSystem::new(4, 4);
        let mut surface = RoadSurfaceSystem::new(512.0);
        surface.compile_dirty(&graph, &terrain);
        let mut demand = DemandSystem::new();

        demand.downgrade_action_credit.industrial = 2.0;
        demand.plan_private_building_actions(
            &allocator,
            &graph,
            &empty_zoning(),
            &snapshot,
            &Default::default(),
            &catalog,
            &tuning,
            &policy,
            &ResidentialOccupantSnapshot::from_runtime(&allocator, &HouseholdSystem::new()),
            1.0 / 24.0,
            BuildingSiteEnvironment {
                road_surface: &surface,
                terrain: &terrain,
            },
            |_: u64, _: &str| Some(tuning.logistics.dispatch_fee_currency),
        );
        let selected = &demand.building_actions.industrial.downgrades;
        assert_eq!(selected.len(), expected, "{name}");
        assert!(
            selected
                .iter()
                .all(|action| action.target_asset_id == target_asset),
            "{name}"
        );
        assert!(
            demand.downgrade_action_credit.industrial >= (2 - expected) as f32,
            "{name}: rejection cannot spend pacing credit"
        );
    }
}

#[test]
fn residential_downgrades_preserve_worker_housing_and_share_surplus() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let factory_profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    for (kind, jobs, unhoused, connected, expected) in [
        ("surplus", 0, 0, true, 2),
        ("live", 4, 0, true, 1),
        ("live", 8, 0, true, 0),
        ("construction", 8, 0, true, 0),
        ("queued", 8, 0, true, 0),
        ("new", 8, 0, true, 0),
        ("unhoused", 0, 8, true, 0),
        ("disconnected", 0, 0, false, 2),
    ] {
        let mut allocator = BuildingAllocator::new();
        let mut home_assets = Vec::new();
        for (level, capacity) in [(1, 2), (2, 6)] {
            let asset = register_family_asset(
                &mut allocator,
                &format!("home_{level}"),
                ZoneType::Residential,
                Some("downsizing_home"),
                level,
            );
            let mut manifest = allocator.registry.get(&asset).unwrap().manifest.clone();
            manifest.building.as_mut().unwrap().household_capacity = Some(capacity);
            allocator.registry.register("test", manifest, String::new());
            home_assets.push(asset);
        }
        for index in 0..2 {
            let mut home = building(ZoneType::Residential, 0.0, 0, 0, home_assets[1].clone());
            home.level = 2;
            home.parcel_id = 1000 + index;
            home.cell_x = 100 + index as usize * 2;
            allocator.buildings.push(home);
        }
        let factory = register_family_asset_with_economy_profile(
            &mut allocator,
            "factory",
            ZoneType::Industrial,
            None,
            1,
            Some("machinery_factory_basic"),
        );
        if kind == "construction" {
            for index in 0..2 {
                let mut plant = building(ZoneType::Industrial, 0.0, 0, 0, factory.clone());
                plant.economy_profile_runtime_id = factory_profile.runtime_id;
                plant.parcel_id = 2000 + index;
                plant.cell_x = 120 + index as usize * 2;
                plant.construction_remaining_hours = 5;
                plant.construction_total_hours = 5;
                allocator.buildings.push(plant);
            }
        }
        let graph = graph_with_connected_border();
        let zoning = if matches!(kind, "queued" | "new") {
            two_lots(&graph, ZoneType::Industrial)
        } else {
            empty_zoning()
        };
        let terrain = TerrainSystem::new(4, 4);
        let mut surface = RoadSurfaceSystem::new(512.0);
        surface.compile_dirty(&graph, &terrain);
        let mut snapshot = vacant_admission_snapshot();
        snapshot.vacant_household_slots = 12;
        snapshot.open_job_slots = if kind == "live" { jobs } else { 0 };
        snapshot.connected_border_count = u32::from(connected);
        snapshot.unhoused_household_count = unhoused;
        snapshot.output_absorption = OutputAbsorptionContext::empty(catalog.resource_count());
        snapshot
            .output_absorption
            .set_export_expansion_available(connected);
        let mut demand = DemandSystem::new();

        demand.downgrade_action_credit.residential = 2.0;
        if kind == "queued" {
            let queued: Vec<_> = zoning
                .parcels()
                .iter()
                .map(|parcel| DemandSpawnAction {
                    parcel_id: parcel.id().raw(),
                    asset_id: factory.clone(),
                })
                .collect();
            demand.set_queued_construction(
                &allocator,
                &zoning,
                0,
                queued
                    .iter()
                    .map(|action| (1, ZoneType::Industrial, action)),
            );
            assert_eq!(demand.queued_construction.len(), 2);
            snapshot.reserve_queued_construction(&catalog, &demand.queued_construction);
        }
        demand.plan_private_building_actions(
            &allocator,
            &graph,
            &zoning,
            &snapshot,
            &Default::default(),
            &catalog,
            &tuning,
            &CityFiscalPolicy::default(),
            &ResidentialOccupantSnapshot::from_runtime(&allocator, &HouseholdSystem::new()),
            1.0 / 24.0,
            BuildingSiteEnvironment {
                road_surface: &surface,
                terrain: &terrain,
            },
            |_: u64, _: &str| Some(tuning.logistics.dispatch_fee_currency),
        );
        let selected = &demand.building_actions.residential.downgrades;
        assert_eq!(selected.len(), expected, "{kind}, {jobs}");
        assert!(
            selected
                .iter()
                .all(|action| action.target_asset_id == home_assets[0])
        );
        assert!(demand.downgrade_action_credit.residential >= (2 - expected) as f32);
        if kind == "new" {
            assert_eq!(
                demand.building_actions.industrial.spawns.len(),
                2,
                "downsizing cannot preempt a viable workplace's housing"
            );
        }
        if connected {
            assert!(
                demand.workforce_reservations.unreserved_ready_homes() >= 1,
                "retain the same one-place vacancy reserve used by housing construction"
            );
        }
    }
}

#[test]
fn an_available_lower_tier_cannot_hide_a_viable_upgrade() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let mut allocator = BuildingAllocator::new();
    let assets: Vec<_> = (1..=3)
        .map(|level| {
            register_family_asset_with_economy_profile(
                &mut allocator,
                &format!("factory_{level}"),
                ZoneType::Industrial,
                Some("both_directions"),
                level,
                Some("machinery_factory_basic"),
            )
        })
        .collect();
    let mut factory = building(ZoneType::Industrial, 0.0, 0, 4, assets[1].clone());
    factory.level = 2;
    factory.economy_profile_runtime_id = profile.runtime_id;
    factory.operating_budget = 100_000.0;
    for input in &profile.inputs {
        factory.set_inventory_units(
            input.resource_runtime_id,
            profile.input_inventory_targets(input, 1.0).0,
        );
    }
    allocator.buildings.push(factory);
    let mut snapshot = vacant_admission_snapshot();
    snapshot.open_job_slots = 0;
    snapshot.output_absorption = OutputAbsorptionContext::empty(catalog.resource_count());
    snapshot
        .output_absorption
        .set_export_expansion_available(true);
    let graph = graph_with_connected_border();
    let terrain = TerrainSystem::new(4, 4);
    let mut surface = RoadSurfaceSystem::new(512.0);
    surface.compile_dirty(&graph, &terrain);
    let mut demand = DemandSystem::new();
    demand.upgrade_action_credit.industrial = 1.0;
    demand.downgrade_action_credit.industrial = 1.0;
    demand.plan_private_building_actions(
        &allocator,
        &graph,
        &empty_zoning(),
        &snapshot,
        &Default::default(),
        &catalog,
        &tuning,
        &CityFiscalPolicy::default(),
        &ResidentialOccupantSnapshot::from_runtime(&allocator, &HouseholdSystem::new()),
        1.0 / 24.0,
        BuildingSiteEnvironment {
            road_surface: &surface,
            terrain: &terrain,
        },
        |_: u64, _: &str| Some(tuning.logistics.dispatch_fee_currency),
    );
    assert_eq!(demand.building_actions.industrial.upgrades.len(), 1);
    assert_eq!(
        demand.building_actions.industrial.upgrades[0].target_asset_id,
        assets[2]
    );
    assert!(demand.building_actions.industrial.downgrades.is_empty());
    assert!(demand.downgrade_action_credit.industrial >= 1.0);
    // Cash cannot subsidize a replacement whose proposed route has no recurring margin.
    demand.building_actions = Default::default();
    demand.upgrade_action_credit.industrial = 1.0;
    demand.plan_private_building_actions(
        &allocator,
        &graph,
        &empty_zoning(),
        &snapshot,
        &Default::default(),
        &catalog,
        &tuning,
        &CityFiscalPolicy::default(),
        &ResidentialOccupantSnapshot::from_runtime(&allocator, &HouseholdSystem::new()),
        1.0 / 24.0,
        BuildingSiteEnvironment {
            road_surface: &surface,
            terrain: &terrain,
        },
        |_: u64, _: &str| Some(1_000_000.0),
    );
    assert!(demand.building_actions.industrial.upgrades.is_empty());
    assert!(demand.building_actions.industrial.downgrades.is_empty());
    assert!(demand.upgrade_action_credit.industrial >= 1.0);
}

#[test]
#[ignore = "manual release scaling of active business and residential level-change planning"]
fn benchmark_active_level_change_planning() {
    use crate::simulation::economy::households::local_utility_service_costs;
    use crate::simulation::economy::operation::{OperatingCosts, OperationPlan};
    use std::hint::black_box;
    use std::time::Instant;

    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let prices = OperationPrices {
        utilities: local_utility_service_costs(&catalog)
            .map(|price| price * tuning.owa_import_price_multiplier),
        property_tax: 0.0,
        imports: tuning.owa_import_price_multiplier,
        carrier_quote: tuning.logistics.dispatch_fee_currency,
        truck_payload: tuning.logistics.truck_payload_kg,
    };
    let costs = OperatingCosts::for_plan(
        profile,
        &catalog,
        OperationPlan {
            workers: 4,
            activity: 1.0,
            area: 1.0,
        },
        prices,
        |_, units| units,
    );
    let sales: f32 = profile
        .outputs
        .iter()
        .map(|output| {
            profile.net_output_units_per_day(output)
                * catalog
                    .unit_price_for_resource(output.resource_runtime_id)
                    .unwrap()
                * tuning.owa_export_price_multiplier
        })
        .sum();
    let policy = CityFiscalPolicy {
        industrial_property_tax_per_building_per_day: (sales - costs.total()) / 2.0,
        property_tax_level_multiplier: 3.0,
        ..Default::default()
    };
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let terrain = TerrainSystem::new(4, 4);
    let mut surface = RoadSurfaceSystem::new(512.0);
    surface.compile_dirty(&graph, &terrain);
    for pairs in [1_024_usize, 8_192, 65_536] {
        let mut allocator = BuildingAllocator::new();
        let mut home_assets = Vec::new();
        let mut factory_assets = Vec::new();
        for (level, homes) in [(1, 8), (2, 16)] {
            let asset = register_family_asset(
                &mut allocator,
                &format!("home_{level}"),
                ZoneType::Residential,
                Some("bench_home"),
                level,
            );
            let mut manifest = allocator.registry.get(&asset).unwrap().manifest.clone();
            manifest.building.as_mut().unwrap().household_capacity = Some(homes);
            allocator.registry.register("test", manifest, String::new());
            home_assets.push(asset);
            factory_assets.push(register_family_asset_with_economy_profile(
                &mut allocator,
                &format!("factory_{level}"),
                ZoneType::Industrial,
                Some("bench_factory"),
                level,
                Some("machinery_factory_basic"),
            ));
        }
        let mut home = building(ZoneType::Residential, 0.0, 0, 0, home_assets[1].clone());
        home.level = 2;
        let mut factory = building(ZoneType::Industrial, 0.0, 0, 0, factory_assets[1].clone());
        factory.level = 2;
        factory.economy_profile_runtime_id = profile.runtime_id;
        factory.operating_budget = 100_000.0;
        allocator.buildings.reserve(pairs * 2);
        for index in 0..pairs * 2 {
            let mut building = if index % 2 == 0 {
                home.clone()
            } else {
                factory.clone()
            };
            building.parcel_id = index as u64 + 1;
            building.cell_x = index * 2;
            allocator.buildings.push(building);
        }
        let mut snapshot = vacant_admission_snapshot();
        snapshot.vacant_household_slots = pairs as u32 * 16;
        snapshot.open_job_slots = 0;
        let placed_output: Vec<_> = profile
            .outputs
            .iter()
            .map(|port| (port.resource_runtime_id, port.units_per_day * pairs as f32))
            .collect();
        snapshot.output_absorption = OutputAbsorptionContext::from_resource_amounts(
            catalog.resource_count(),
            &placed_output,
            &[],
            0,
            &[],
        );
        snapshot
            .output_absorption
            .set_export_expansion_available(true);
        let occupants =
            ResidentialOccupantSnapshot::from_runtime(&allocator, &HouseholdSystem::new());
        let mut demand = DemandSystem::new();
        let mut plan_once = || {
            demand.building_actions = Default::default();
            demand.downgrade_action_credit.residential = pairs as f32;
            demand.downgrade_action_credit.industrial = pairs as f32;
            demand.plan_private_building_actions(
                black_box(&allocator),
                &graph,
                &zoning,
                &snapshot,
                &Default::default(),
                &catalog,
                &tuning,
                &policy,
                &occupants,
                1.0 / 24.0,
                BuildingSiteEnvironment {
                    road_surface: &surface,
                    terrain: &terrain,
                },
                |_: u64, _: &str| Some(tuning.logistics.dispatch_fee_currency),
            );
            assert_eq!(demand.building_actions.residential.downgrades.len(), pairs);
            assert_eq!(demand.building_actions.industrial.downgrades.len(), pairs);
            assert_eq!(
                demand.workforce_reservations.unreserved_ready_homes(),
                pairs as u32 * 4
            );
        };
        for _ in 0..3 {
            plan_once();
        }
        let mut samples = [0.0; 9];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..3 {
                plan_once();
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 3.0;
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "active_level_change_planning buildings={} selected={} median_ms={:.3}",
            pairs * 2,
            pairs * 2,
            samples[4]
        );
    }
}

#[test]
fn an_unreachable_factory_does_not_block_other_sites() {
    let mut allocator = BuildingAllocator::new();
    register_family_asset_with_economy_profile(
        &mut allocator,
        "site_factory",
        ZoneType::Industrial,
        None,
        1,
        Some("machinery_factory_basic"),
    );
    let graph = graph_with_connected_border();
    let zoning = two_lots(&graph, ZoneType::Industrial);
    let first = zoning.parcels().iter().map(|p| p.id().raw()).min().unwrap();
    let demand = plan_with_quote(&allocator, &graph, &zoning, 4, &[], |parcel, _| {
        (parcel != first).then_some(4.0)
    });
    assert_eq!(demand.building_actions.industrial.spawns.len(), 1);
    assert_ne!(
        demand.building_actions.industrial.spawns[0].parcel_id,
        first
    );
    assert_eq!(
        demand
            .last_building_action_diagnostics
            .industrial
            .spawn_rejected_viability,
        1
    );
}

#[test]
#[ignore = "manual release measurement of proposed entrance and warm freight quotation"]
fn benchmark_demand_site_freight_quotes() {
    use crate::simulation::economy::logistics::{ShipmentSystem, collect_connected_border_nodes};
    use crate::simulation::network::TransitNetwork;
    use std::hint::black_box;
    use std::time::Instant;

    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "site_quote", ZoneType::Industrial);
    let mut graph = graph_with_connected_border();
    let zoning = two_lots(&graph, ZoneType::Industrial);
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let mut borders = Vec::new();
    collect_connected_border_nodes(&graph, &mut borders);
    let mut logistics = ShipmentSystem::new();
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &borders);
    let tuning = load_runtime_economy_tuning().unwrap();
    let lots: Vec<_> = zoning.parcels().iter().map(|p| p.id().raw()).collect();
    for count in [1_024, 8_192, 65_536] {
        let mut samples = [0.0; 9];
        for round in 0..12 {
            let start = Instant::now();
            for i in 0..count {
                let entrance = allocator
                    .demand_site_entrance(
                        black_box(lots[i % lots.len()]),
                        &asset,
                        &zoning,
                        &graph,
                        &network.lane_system,
                    )
                    .unwrap();
                assert!(
                    black_box(
                        logistics
                            .site_carrier_quote(&entrance, &network, &graph, &borders, &tuning,)
                    )
                    .is_some()
                );
            }
            if round >= 3 {
                samples[round - 3] = start.elapsed().as_secs_f64() * 1_000.0;
            }
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "demand_site_freight_quotes candidates={count} median_ms={:.3}",
            samples[4]
        );
    }
}

#[test]
#[ignore = "manual release scaling of active queue snapshots, reservations and warm claims"]
fn benchmark_active_construction_queue() {
    use crate::simulation::buildings::allocator::construction_duration_hours;
    use crate::simulation::zoning::parcels::ParcelGeometry;
    use std::hint::black_box;
    use std::time::Instant;

    let graph = graph_with_connected_border();
    let households = HouseholdSystem::new();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let config = load_builtin_demand_config().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let duration = construction_duration_hours(ZoneType::Industrial, 1, &tuning);
    // Vary background size independently of the active queue, then grow both. Geometry,
    // assets and the synthetic building census are setup, not timed gameplay placement.
    for (background, pairs) in [
        (2_048, 512),
        (16_384, 512),
        (131_072, 512),
        (16_384, 4_096),
        (131_072, 32_768),
    ] {
        let mut allocator = BuildingAllocator::new();
        let home = register_test_asset(&mut allocator, "queue_home", ZoneType::Residential);
        let factory = register_family_asset_with_economy_profile(
            &mut allocator,
            "queue_factory",
            ZoneType::Industrial,
            None,
            1,
            Some("machinery_factory_basic"),
        );
        let capacity = allocator.registry.household_capacity(&home);
        assert!(capacity >= profile.worker_capacity);
        let template = building(ZoneType::Residential, 0.0, 0, 0, home.clone());
        allocator.buildings.resize(background, template);
        let mut zoning = empty_zoning();
        let mut queue = Vec::with_capacity(pairs * 2);
        for index in 0..pairs * 2 {
            let origin = Vector2::new((index % 256) as f32 * 20.0, (index / 256) as f32 * 20.0);
            let parcel = zoning.parcels.insert_new(
                ParcelGeometry {
                    edge_idx: 0,
                    side: 1,
                    frontage_center_t: 0.5,
                    frontage_m: 10.0,
                    depth_m: 10.0,
                    front_center: origin + Vector2::new(5.0, 0.0),
                    center: origin + Vector2::new(5.0, 5.0),
                    tangent: Vector2::new(1.0, 0.0),
                    normal: Vector2::new(0.0, 1.0),
                    corners: [
                        origin,
                        origin + Vector2::new(10.0, 0.0),
                        origin + Vector2::new(10.0, 10.0),
                        origin + Vector2::new(0.0, 10.0),
                    ],
                    aabb_min: origin,
                    aabb_max: origin + Vector2::new(10.0, 10.0),
                },
                0,
            );
            let (zone, asset) = if index % 2 == 0 {
                (ZoneType::Residential, &home)
            } else {
                (ZoneType::Industrial, &factory)
            };
            queue.push((
                index as u64 + 1,
                zone,
                DemandSpawnAction {
                    parcel_id: parcel.raw(),
                    asset_id: asset.clone(),
                },
            ));
        }
        let mut demand = DemandSystem::new();
        let mut samples = [[0.0_f64; 3]; 9];
        for round in 0..12 {
            let start = Instant::now();
            let mut snapshot = DailyDemandSnapshot::from_runtime(
                black_box(&allocator),
                &households,
                &graph,
                &config,
                1_000_000.0,
            );
            let census_ms = start.elapsed().as_secs_f64() * 1_000.0;
            for output in &profile.outputs {
                snapshot.output_absorption.add_demand_units(
                    output.resource_runtime_id,
                    profile.net_output_units_per_day(output) * (pairs * 2 + 1) as f32,
                );
            }
            let start = Instant::now();
            demand.set_queued_construction(
                &allocator,
                &zoning,
                0,
                queue
                    .iter()
                    .map(|(due, zone, action)| (*due, *zone, action)),
            );
            snapshot.reserve_queued_construction(&catalog, &demand.queued_construction);
            demand.workforce_reservations.reset(
                &allocator,
                &catalog,
                &tuning,
                &snapshot,
                &demand.queued_construction,
                demand.next_construction_start_hour,
            );
            demand.workforce_reservations.reserve_existing();
            let reservations_ms = start.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(snapshot.queued_by_use, [pairs as u32, 0, pairs as u32]);
            assert_eq!(
                snapshot.under_construction_household_slots,
                pairs as u32 * capacity
            );
            assert_eq!(
                snapshot.move_in_job_slots, 0,
                "future jobs cannot fund arrivals"
            );
            let start = Instant::now();
            for _ in 0..pairs {
                assert!(black_box(
                    demand
                        .workforce_reservations
                        .claim(duration, profile.worker_capacity)
                ));
                snapshot.output_absorption.reserve_candidate(profile);
            }
            let claims_ms = start.elapsed().as_secs_f64() * 1_000.0;
            for output in &profile.outputs {
                assert_eq!(
                    snapshot
                        .output_absorption
                        .unmet_units(output.resource_runtime_id),
                    profile.net_output_units_per_day(output),
                    "queued and newly selected projects each reserve their customers once"
                );
            }
            // All remaining supply must be finite and usable exactly once after the queued
            // homes finish. Drain outside timing to verify that warm claims really consume it.
            let remaining =
                (background + pairs) as u32 * capacity - pairs as u32 * profile.worker_capacity * 2;
            assert!(demand.workforce_reservations.claim(duration, remaining));
            assert!(!demand.workforce_reservations.claim(duration, 1));
            if round >= 3 {
                samples[round - 3] = [census_ms, reservations_ms, claims_ms];
            }
        }
        let median = |column| {
            let mut values = samples.map(|sample| sample[column]);
            values.sort_by(f64::total_cmp);
            values[4]
        };
        eprintln!(
            "active_queue background={background} queued={} claims={pairs} census_ms={:.3} reservations_ms={:.3} claims_ms={:.3}",
            pairs * 2,
            median(0),
            median(1),
            median(2)
        );
    }
}
