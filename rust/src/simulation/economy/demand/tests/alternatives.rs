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
    let home_asset =
        register_test_asset(&mut allocator, "funded_buyer_home", ZoneType::Residential);
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
    let supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .unwrap();
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
            100_000.0,
            420,
        );
        if income > 0.0 && cash > 0.0 {
            assert_eq!(households.job_funding.capacities[1], 3);
        } else {
            assert_eq!(households.job_funding.capacities[1], 0);
        }
        let snapshot = DailyDemandSnapshot::from_runtime_with_catalog(
            &allocator,
            &households,
            &graph,
            &config,
            &catalog,
            &tuning,
            &policy,
            100_000.0,
        );
        assert!(
            (snapshot.output_absorption.unmet_units(food) - expected).abs() < 0.001,
            "workers={workers} cash={cash} income={income} stock={stock}"
        );
        assert_eq!(
            snapshot.output_absorption.unmet_units(supplies),
            0.0,
            "the shop retains its full output commitment through its staffing search"
        );
        if expected > 0.0 {
            for units in [1.0, 30.0, 31.0] {
                let costs = logistics.quote_recurring_input(
                    &mut households.job_funding.input_supply,
                    InputRequirement {
                        resource: supplies,
                        daily_units: units,
                        buffer_units: units * 2.0,
                        replenishment_batch_units: Some(40.0),
                    },
                    Some(0),
                    &allocator.entrances[0],
                    None,
                    &allocator,
                    &catalog,
                    &tuning,
                    &network,
                    &graph,
                    1.0,
                );
                assert!(
                    costs.goods.is_infinite(),
                    "the 30 funded units already belong to households, not additional business buyers"
                );
                households
                    .job_funding
                    .input_supply
                    .reject(&allocator, &graph);
            }
        } else {
            assert!(!households.job_funding.input_supply.has_offers());
        }
    }
    tuning
        .operational_clock
        .freight_profiles
        .iter_mut()
        .find(|profile| profile.id == "daytime_receive")
        .unwrap()
        .outside_window_cost_multiplier = 10.0;
    allocator.buildings[1].worker_count = 3;
    allocator.buildings[1].operating_budget = 100_000.0;
    allocator.buildings[1].set_inventory_units(food, 0.0);
    households.households[0].gross_daily_wages = 10_000.0;
    for (minute, jobs) in [(420, 3), (419, 0), (1080, 0), (420, 3)] {
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
            100_000.0,
            minute,
        );
        assert_eq!(
            households.job_funding.capacities[1], jobs,
            "live jobs must pay the receiving window's actual goods price at {minute}"
        );
    }
    let mut revalidated = crate::simulation::economy::hiring::JobFunding::default();
    revalidated.refresh(
        &allocator,
        &households.households,
        &mut logistics,
        &network,
        &graph,
        &catalog,
        &tuning,
        &policy,
        &[],
        100_000.0,
        419,
    );
    let current = DailyDemandSnapshot::from_runtime_with_funding(
        &allocator,
        &households,
        &graph,
        &config,
        &catalog,
        &tuning,
        &policy,
        100_000.0,
        &revalidated,
    );
    assert_eq!(
        current.output_absorption.unmet_units(food),
        0.0,
        "a refreshed market cannot retain yesterday's funded input customers"
    );
    assert_eq!(
        households.job_funding.capacities[1], 3,
        "the independent employment view still represents the earlier price interval"
    );
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
            100_000.0,
            420,
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
                [(0, 1, ZoneType::Industrial, &actions[0])],
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
        demand.publish_census_if_stale_for_test(
            &allocator,
            &mut households,
            &network,
            &graph,
            &mut logistics,
            &policy,
            100_000.0,
            420,
        );
        demand.run_planning_hour_for_test(
            &allocator,
            &mut households,
            &graph,
            &zoning,
            100_000.0,
            &policy,
            &network,
            &mut logistics,
            BuildingSiteEnvironment {
                road_surface: &surface,
                terrain: &terrain,
            },
            420,
        );
        // The planning slot's permits for a queue, proved on the census the town publishes
        // (`SIM-02-I`); the epoch stays open so its books can be read, then closes.
        let permits = |demand: &mut DemandSystem,
                       queue: &[&DemandSpawnAction],
                       allocator: &BuildingAllocator,
                       zoning: &ZoningSystem,
                       households: &mut HouseholdSystem,
                       logistics: &mut ShipmentSystem,
                       now: u64| {
            let queue: Vec<_> = queue
                .iter()
                .map(|&action| (ZoneType::Industrial, action))
                .collect();
            let permits = demand.open_permits_for_test(
                &queue, allocator, households, zoning, &graph, &network, logistics, &policy,
                100_000.0, now,
            );
            let purchases = demand.construction_execution.reservations.business.sales(
                0,
                farm_profile.outputs[0].resource_runtime_id,
                f64::INFINITY,
            );
            demand.close_construction_epoch(households, allocator, &graph);
            (permits, purchases)
        };
        if commitment != "new" {
            assert!(
                demand.building_actions.industrial.spawns.is_empty(),
                "{commitment} operation already owns the farm's 200 daily grain units"
            );
            if commitment == "queued" {
                for _ in 0..3 {
                    let (proved, purchases) = permits(
                        &mut demand,
                        &[&actions[0], &actions[1]],
                        &allocator,
                        &zoning,
                        &mut households,
                        &mut logistics,
                        1,
                    );
                    assert!(
                        proved[0].is_some(),
                        "a queued operation retains its financed claims without reserving twice"
                    );
                    assert_eq!(purchases.units, 200.0);
                    assert_eq!(
                        purchases.revenue,
                        200.0 * f64::from(farm_profile.unit_price_currency),
                        "queued input demand retains the actual local seller price, not the import tariff"
                    );
                    assert!(
                        proved[1].is_none(),
                        "another project cannot consume the queued buyer's material reservation"
                    );
                }
                let (proved, _) = permits(
                    &mut demand,
                    &[&actions[1]],
                    &allocator,
                    &zoning,
                    &mut households,
                    &mut logistics,
                    1,
                );
                assert!(
                    proved[0].is_some(),
                    "a queue without the first customer returns its material and labour"
                );
            }
            continue;
        }
        assert_eq!(demand.building_actions.industrial.spawns.len(), 1);
        let chosen = demand.building_actions.industrial.spawns[0].clone();
        let other = actions
            .iter()
            .find(|action| action.parcel_id != chosen.parcel_id)
            .unwrap();
        // Each slot proves its own queue: neither project keeps inputs into the next.
        for queue in [[&chosen], [other], [&chosen]] {
            let (proved, _) = permits(
                &mut demand,
                &queue,
                &allocator,
                &zoning,
                &mut households,
                &mut logistics,
                1,
            );
            assert!(proved[0].is_some());
        }
        let (proved, _) = permits(
            &mut demand,
            &[&chosen, other],
            &allocator,
            &zoning,
            &mut households,
            &mut logistics,
            1,
        );
        assert!(
            proved[0].is_some() && proved[1].is_none(),
            "the second project cannot take the first one's grain"
        );
        if commitment == "new" {
            let window = Arc::make_mut(&mut demand.runtime_tuning)
                .operational_clock
                .freight_profiles
                .iter_mut()
                .find(|profile| profile.id == "always_open")
                .unwrap();
            let original = window.clone();
            window.preferred_windows[0].start_minute = 240;
            window.preferred_windows[0].end_minute = 480;
            window.outside_window_cost_multiplier = 10.0;
            for (minute, accepted) in [(479, true), (480, false), (479, true)] {
                let (proved, _) = permits(
                    &mut demand,
                    &[&chosen],
                    &allocator,
                    &zoning,
                    &mut households,
                    &mut logistics,
                    minute,
                );
                assert_eq!(
                    proved[0].is_some(),
                    accepted,
                    "each planning slot prices its own freight window"
                );
            }
            *Arc::make_mut(&mut demand.runtime_tuning)
                .operational_clock
                .freight_profiles
                .iter_mut()
                .find(|profile| profile.id == "always_open")
                .unwrap() = original;
        }
        let (proved, _) = permits(
            &mut demand,
            &[&chosen],
            &allocator,
            &zoning,
            &mut households,
            &mut logistics,
            1,
        );
        assert!(proved[0].is_some());
        let placed = allocator.execute_single_demand_spawn_action(
            ZoneType::Industrial,
            &chosen,
            None,
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
        demand.finish_queued_spawn(0, &chosen);
        let funded = allocator.buildings.last().unwrap().operating_budget;
        assert!(funded > 0.0);
        assert!(allocator.buildings.last().unwrap().is_under_construction());
        for now in [2, 61] {
            let (proved, _) = permits(
                &mut demand,
                &[other],
                &allocator,
                &zoning,
                &mut households,
                &mut logistics,
                now,
            );
            assert!(
                proved[0].is_none(),
                "the placed site's input claims are replayed as construction"
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
fn replacement_prices_revalidate_downstream_buyers_and_restore_failed_joint_proofs() {
    use super::super::spawn_need::replacement_market_covers_operation;
    use super::super::supply::{ProjectReservations, ProjectSupply, SiteSupply};
    use crate::simulation::economy::definitions::compile_test_catalog;
    use crate::simulation::economy::logistics::{
        RecurringSupply, ShipmentSystem, collect_connected_border_nodes,
    };
    use crate::simulation::economy::operation::InputRequirement;
    use crate::simulation::network::TransitNetwork;
    let authored = include_str!("../../../../../../economy/profiles.toml");
    let start = authored
        .find("[[profiles]]\nid = \"food_processor_basic\"")
        .unwrap();
    let end = start + authored[start + 1..].find("\n[[profiles]]").unwrap() + 1;
    for (price, accepted, output, changed_inputs, extra_buyer, commitment) in [
        (14.0, true, None, false, false, "live"),
        (30.0, false, None, false, false, "live"),
        (14.0, true, Some("replacement_output"), false, false, "live"),
        (14.0, false, None, true, false, "live"),
        (14.0, true, Some("household_supplies"), false, false, "live"),
        (14.0, true, None, false, true, "live"),
        (14.0, true, None, false, false, "constructing"),
        (30.0, false, None, false, false, "constructing"),
        (
            14.0,
            true,
            Some("replacement_output"),
            false,
            false,
            "constructing",
        ),
        (
            14.0,
            true,
            Some("household_supplies"),
            false,
            false,
            "constructing",
        ),
        (14.0, true, None, false, true, "constructing"),
        (14.0, true, None, false, false, "queued"),
        (30.0, false, None, false, false, "queued"),
        (
            14.0,
            true,
            Some("replacement_output"),
            false,
            false,
            "queued",
        ),
        (
            14.0,
            true,
            Some("household_supplies"),
            false,
            false,
            "queued",
        ),
        (14.0, false, None, true, false, "queued"),
    ] {
        let constructing = commitment == "constructing";
        let queued = commitment == "queued";
        let changed_recipe = output.is_some();
        let household_output = output == Some("household_supplies");
        let mut recipe = authored[start..end]
            .replace("food_processor_basic", "replacement_food")
            .replace(
                "unit_price_currency = 12.0",
                &format!("unit_price_currency = {price:.1}"),
            );
        if let Some(output) = output {
            recipe = recipe.replace("\"packaged_food\"", &format!("\"{output}\""));
        }
        if extra_buyer {
            // Twice the customers require twice the material throughput, at unchanged staffing.
            recipe = recipe.replace("units_per_day = 200.0", "units_per_day = 400.0");
        }
        if changed_inputs {
            recipe = recipe.replace("\"grain\"", "\"steel\"");
        }
        let resource = "[[resources]]\nid = \"replacement_output\"\nquantity_unit = \"kg\"\nshipment_batch_kg = 50.0\nruntime_id = 10\nimport_unit_price_currency = 20.0\n";
        let feed = "[[resources]]\nid = \"replacement_feed\"\nquantity_unit = \"kg\"\nshipment_batch_kg = 50.0\nruntime_id = 11\nimport_unit_price_currency = 0.01\n";
        let upstream = authored[start..end]
            .replace("food_processor_basic", "dependent_grain")
            .replace("\"grain\"", "\"replacement_feed\"")
            .replace("\"packaged_food\"", "\"grain\"")
            .replace("unit_price_currency = 12.0", "unit_price_currency = 4.8")
            .replace(
                "[[profiles.inputs]]\nresource = \"machinery\"\nunits_per_day = 2.0",
                "",
            );
        let catalog = compile_test_catalog(&format!(
            "{authored}\n{resource}\n{feed}\n{recipe}\n{upstream}"
        ))
        .unwrap();
        let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
        // Future buyers must actually choose the incumbent's local inputs so withdrawal
        // exercises their dependency. Equal goods prices can favor the outside route.
        tuning.owa_import_price_multiplier = if changed_inputs || constructing || queued {
            1.1
        } else {
            1.0
        };
        if changed_inputs && !queued {
            tuning.owa_export_price_multiplier = 0.5;
        }
        let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
        let mut graph = graph_with_connected_border();
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.rebuild_cch_and_check(&graph);
        let mut zoning = two_lots(&graph, ZoneType::Industrial);
        let terrain = TerrainSystem::new(4, 4);
        let mut surface = RoadSurfaceSystem::new(512.0);
        surface.compile_dirty(&graph, &terrain);
        let mut allocator = BuildingAllocator::new();
        let old = register_family_asset_with_economy_profile(
            &mut allocator,
            "joint_old",
            ZoneType::Industrial,
            Some("joint_food"),
            1,
            Some("food_processor_basic"),
        );
        let target = register_family_asset_with_economy_profile(
            &mut allocator,
            "joint_target",
            ZoneType::Industrial,
            Some("joint_food"),
            2,
            Some("replacement_food"),
        );
        let action = DemandSpawnAction {
            parcel_id: zoning.parcels()[0].id().raw(),
            asset_id: old,
        };
        let placed = allocator.execute_single_demand_spawn_action(
            ZoneType::Industrial,
            &action,
            None,
            &mut zoning,
            &graph,
            &network.lane_system,
            &surface,
            &terrain,
            &catalog,
            &tuning,
        );
        assert_eq!(placed.industrial.spawn_executed, 1);
        allocator.buildings[0].construction_complete_slot = 0;
        allocator.buildings[0].worker_count = 10;
        allocator.buildings[0].operating_budget = 100_000.0;
        let shop_asset = register_family_asset_with_economy_profile(
            &mut allocator,
            "joint_buyer",
            ZoneType::Commercial,
            None,
            1,
            Some("grocery_basic"),
        );
        let home_asset = register_test_asset(&mut allocator, "joint_home", ZoneType::Residential);
        for (index, zone, workers, asset) in [
            (1, ZoneType::Commercial, 15, shop_asset.clone()),
            (2, ZoneType::Residential, 0, home_asset),
        ] {
            if index == 1 && queued {
                continue;
            }
            let mut site = building(zone, 0.0, 0, workers, asset);
            site.center_x = 35.0 + index as f32 * 5.0;
            site.center_y = if index == 1 { -20.0 } else { 20.0 };
            site.frontage_t = site.center_x / 50.0;
            site.operating_budget = 100_000.0;
            if index == 1 {
                site.economy_profile_runtime_id =
                    catalog.profile_for_id("grocery_basic").unwrap().runtime_id;
                if constructing {
                    site.worker_count = 0;
                    site.set_construction_hours_left(12);
                    site.construction_total_hours = 12;
                }
            }
            allocator.buildings.push(site);
        }
        if changed_inputs {
            // Before the queued grocery exists the incumbent needs export revenue. Its
            // upstream processor serves only local buyers in this variant, so changing the
            // incumbent's input still destroys that supplier's sole financed customer.
            let zone = if queued {
                ZoneType::Commercial
            } else {
                ZoneType::Industrial
            };
            let asset = register_family_asset_with_economy_profile(
                &mut allocator,
                "dependent_grain",
                zone,
                None,
                1,
                Some("dependent_grain"),
            );
            let mut source = building(zone, 0.0, 0, 10, asset);
            source.economy_profile_runtime_id = catalog
                .profile_for_id("dependent_grain")
                .unwrap()
                .runtime_id;
            source.center_x = 5.0;
            source.center_y = 20.0;
            source.frontage_t = 0.1;
            source.operating_budget = 100_000.0;
            allocator.buildings.push(source);
        }
        if extra_buyer {
            let mut buyer = allocator.buildings[1].clone();
            buyer.center_x = 15.0;
            buyer.center_y = -25.0;
            buyer.frontage_t = 0.3;
            allocator.buildings.push(buyer);
        }
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let mut borders = Vec::new();
        collect_connected_border_nodes(&graph, &mut borders);
        let mut family = housed_household(
            if queued { 1 } else { 2 },
            if household_output || extra_buyer {
                400
            } else {
                200
            },
            100_000.0,
            0.0,
        );
        family.employed_adults = if queued { 185 } else { 200 };
        family.gross_daily_wages = 100_000.0;
        let mut logistics = ShipmentSystem::new();
        let mut funding = crate::simulation::economy::hiring::JobFunding::default();
        funding.refresh(
            &allocator,
            &[family.clone()],
            &mut logistics,
            &network,
            &graph,
            &catalog,
            &tuning,
            &policy,
            &[],
            0.0,
            420,
        );
        assert_eq!(
            &funding.capacities[..2],
            &[10, if constructing || queued { 0 } else { 15 }]
        );
        if changed_inputs {
            assert_eq!(funding.capacities[allocator.buildings.len() - 1], 10);
            let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
            assert_eq!(
                funding
                    .business
                    .sales(allocator.buildings.len() - 1, grain, 200.0)
                    .units,
                200.0
            );
        }
        let food = catalog.resource_runtime_id_for_id("packaged_food").unwrap();
        assert_eq!(
            funding.business.sales(0, food, 200.0).units,
            if constructing || queued { 0.0 } else { 200.0 }
        );
        let mut demand = DemandSystem::new();
        demand.runtime_catalog = std::sync::Arc::new(catalog.clone());
        demand.runtime_tuning = std::sync::Arc::new(tuning.clone());
        let queued_action = DemandSpawnAction {
            parcel_id: zoning.parcels()[1].id().raw(),
            asset_id: shop_asset,
        };
        let buyer = if queued {
            let parcel = zoning.parcels()[1].id();
            let zone = zoning
                .profiles
                .default_runtime_id_for_zone_type(ZoneType::Commercial)
                .unwrap();
            zoning.parcels.set_zone_profile_runtime_id(parcel, zone);
            demand.set_queued_construction_with_entrances(
                &allocator,
                &zoning,
                420,
                [(0, 421, ZoneType::Commercial, &queued_action)],
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
            assert!(demand.queued_construction[0].entrance.is_some());
            allocator.buildings.len() + zoning.parcels.storage_index(parcel).unwrap()
        } else {
            1
        };
        let mut projected = RecurringSupply::default();
        projected.copy_reservations_from(&funding.input_supply, &allocator, &catalog, &graph);
        let mut reservations = ProjectReservations::default();
        reservations.reset(&funding, &allocator, &catalog, &tuning, &policy, 420);
        if constructing || queued {
            let market = OutputAbsorptionContext::empty(catalog.resource_count());
            SiteSupply::new(
                &mut projected,
                &mut reservations,
                &allocator,
                &zoning,
                &network,
                &graph,
                &mut logistics,
                &tuning,
                &borders,
                &catalog,
                420,
                policy.household_vat_rate,
            )
            .reserve_projects(
                demand.queued_construction.iter(),
                &catalog,
                &market,
                &mut Default::default(),
                &policy,
                &funding.service_capacities,
                &funding.freight_capital,
                |_| true,
            );
            if queued {
                assert_eq!(reservations.queued_workers, [(queued_action.parcel_id, 15)]);
                assert!(
                    reservations.operations[1].is_none(),
                    "no unplaced buyer may appear as a live operation"
                );
            } else {
                assert_eq!(reservations.building_workers[1], 15);
            }
            assert_eq!(
                reservations.business.sales(0, food, 200.0).units,
                200.0,
                "only replay of the financed future operation may publish its input purchases"
            );
        }
        if changed_inputs {
            let operation = reservations.operations[allocator.buildings.len() - 1].unwrap();
            let supplier = catalog.profile_for_id("dependent_grain").unwrap();
            let exported = crate::simulation::economy::operation::OperatingCosts::for_plan(
                supplier,
                &catalog,
                operation.plan,
                operation.prices,
                |_, units| units,
            );
            let exports =
                if crate::simulation::economy::logistics::building_outputs_can_export_to_owa(
                    allocator.buildings.last().unwrap(),
                    supplier,
                ) {
                    tuning.owa_export_price_multiplier
                } else {
                    0.0
                };
            assert!(
                200.0 * f64::from(supplier.unit_price_currency * exports) < exported.total(),
                "outside buyers cannot fund the abandoned supplier"
            );
        }
        let before = reservations.business.financing(buyer);
        let original = reservations.business.sales(0, food, 200.0);
        let outside = reservations.operations[buyer].unwrap().prices.carrier_quote;
        let imported = catalog.unit_price_for_resource(food).unwrap();
        let original_sources = |book: &RecurringSupply| {
            let mut sources = Vec::new();
            assert!(book.visit_input_sources(
                buyer,
                InputRequirement {
                    resource: food,
                    daily_units: 200.0,
                    buffer_units: 600.0,
                    replenishment_batch_units: Some(200.0)
                },
                imported,
                Some(outside),
                |source, units, goods, carrier| sources.push((source, units, goods, carrier))
            ));
            sources
        };
        let inputs = original_sources(&projected);
        let all_finance = |reservations: &ProjectReservations| {
            reservations
                .operations
                .iter()
                .enumerate()
                .filter_map(|(owner, operation)| {
                    operation.map(|_| {
                        (
                            owner,
                            reservations.business.financing(owner).map(f64::to_bits),
                        )
                    })
                })
                .collect::<Vec<_>>()
        };
        let all_inputs = |reservations: &ProjectReservations, book: &RecurringSupply| {
            let mut sources = Vec::new();
            for (owner, operation) in reservations.operations.iter().enumerate() {
                let Some(operation) = operation else {
                    continue;
                };
                let profile = catalog.profile_by_runtime_id(operation.profile).unwrap();
                crate::simulation::economy::operation::OperatingCosts::for_plan_with_input_quotes(
                    profile,
                    operation.plan,
                    operation.prices,
                    |_, _| 0.0,
                    |input| {
                        assert!(book.visit_input_sources(
                            owner,
                            input,
                            catalog.unit_price_for_resource(input.resource).unwrap()
                                * operation.prices.imports,
                            Some(operation.prices.carrier_quote),
                            |source, units, goods, carrier| sources.push((
                                owner,
                                input.resource,
                                source,
                                units.to_bits(),
                                goods.to_bits(),
                                carrier.to_bits()
                            ))
                        ));
                        Default::default()
                    },
                );
            }
            sources
        };
        let financing_before = all_finance(&reservations);
        let sources_before = all_inputs(&reservations, &projected);
        let customers_before = reservations.customers.clone();
        let assert_rollback = |reservations: &ProjectReservations, book: &RecurringSupply| {
            assert_eq!(all_finance(reservations), financing_before);
            assert_eq!(all_inputs(reservations, book), sources_before);
            reservations
                .customers
                .assert_same_accounts_for_test(&customers_before);
        };
        let profile = catalog.profile_for_id("replacement_food").unwrap();
        let market = OutputAbsorptionContext::empty(catalog.resource_count());
        let mut accepted_finance = None;
        for attempt in 0..5 {
            let mut forced_capacity = None;
            if attempt == 2 {
                reservations.business.discard_repricing_scratch_for_test();
                if household_output {
                    reservations.customers.shrink_receipt_storage_for_test();
                    forced_capacity = Some(reservations.customers.receipt_capacity_for_test());
                }
                if extra_buyer {
                    projected.shrink_receipt_storage_for_test();
                    forced_capacity = Some(projected.receipt_storage().1);
                }
            }
            let mut supply = SiteSupply::new(
                &mut projected,
                &mut reservations,
                &allocator,
                &zoning,
                &network,
                &graph,
                &mut logistics,
                &tuning,
                &borders,
                &catalog,
                420,
                policy.household_vat_rate,
            );
            let carrier = supply.begin_replacement(0, action.parcel_id, &target);
            let prices = supply.prices(&catalog, ZoneType::Industrial, 2, &policy, carrier);
            if changed_inputs {
                let costs = crate::simulation::economy::operation::OperatingCosts::for_plan(
                    profile,
                    &catalog,
                    crate::simulation::economy::operation::OperationPlan {
                        workers: 10,
                        activity: 1.0,
                        area: 1.0,
                    },
                    prices,
                    |_, _| 0.0,
                );
                assert!(
                    200.0 * f64::from(price) > costs.total() && costs.working_capital < 100_000.0,
                    "the replacement's own operating proof passes; its supplier loses viable sales"
                );
            }
            assert_eq!(
                replacement_market_covers_operation(
                    profile,
                    &catalog,
                    market.market(),
                    prices,
                    tuning.owa_export_price_multiplier,
                    &allocator.buildings[0],
                    0.0,
                    &mut supply
                ),
                accepted,
                "seller price {price} must preserve the {commitment} grocery"
            );
            if accepted && attempt >= 3 {
                supply.accept();
            } else {
                supply.reject();
            }
            if let Some(capacity) = forced_capacity {
                assert!(
                    if household_output {
                        reservations.customers.receipt_capacity_for_test()
                    } else {
                        projected.receipt_storage().1
                    } > capacity,
                    "the complete runtime proof must grow and replay the exhausted arena"
                );
            }
            if accepted && attempt >= 3 {
                assert_eq!(
                    reservations.business.sales(0, food, 200.0).revenue,
                    if changed_recipe {
                        0.0
                    } else {
                        200.0 * f64::from(price)
                    }
                );
                assert!(reservations.business.financing(buyer)[0] > 0.0);
                if !changed_recipe {
                    assert!(reservations.business.financing(buyer)[0] < before[0]);
                }
                let finance = reservations.business.financing(buyer).map(f64::to_bits);
                if let Some(previous) = accepted_finance {
                    assert_eq!(finance, previous);
                }
                accepted_finance = Some(finance);
                let current = original_sources(&projected);
                assert_eq!(current.len(), 1);
                assert_eq!(
                    (current[0].0, current[0].1, current[0].2),
                    if changed_recipe {
                        (None, 200.0, imported)
                    } else {
                        (Some(0), 200.0, price)
                    }
                );
                projected.visit_dependents(0, |_, _, _| panic!("old output was withdrawn"));
                assert_eq!(
                    allocator.buildings[0].economy_profile_runtime_id,
                    catalog
                        .profile_for_id("food_processor_basic")
                        .unwrap()
                        .runtime_id
                );
            } else {
                assert_eq!(
                    reservations.business.financing(buyer).map(f64::to_bits),
                    before.map(f64::to_bits)
                );
                assert_eq!(reservations.business.sales(0, food, 200.0), original);
                assert_eq!(original_sources(&projected), inputs);
                assert_rollback(&reservations, &projected);
            }
        }

        // The gameplay executor runs the same proof in the planning slot's construction epoch
        // on the census the town publishes, and keeps only a placed change; the epoch returns
        // the census's books exactly after a rejection (`SIM-02-I`), which test builds assert.
        // A terrain failure comes after financing and must preserve every buyer commitment.
        let mut households = HouseholdSystem::new();
        households.households.push(family);
        households.job_funding = funding;
        let mut agents = crate::simulation::economy::agents::AgentSystem::new();
        let mut plan = crate::simulation::economy::demand::DemandBuildingActionPlan::default();
        plan.industrial.upgrades.push(
            crate::simulation::economy::demand::DemandLevelChangeAction {
                building: crate::simulation::economy::demand::demand_building_action_key(
                    &allocator.buildings[0],
                ),
                target_asset_id: target.clone(),
            },
        );
        network.road_surface.compile_dirty(&graph, &terrain);
        let mut blocked = terrain.clone();
        for z in 0..blocked.height {
            for x in 0..blocked.width {
                blocked.set_height(x, z, 20.0);
            }
        }
        let budgets: Vec<_> = allocator
            .buildings
            .iter()
            .map(|building| building.operating_budget.to_bits())
            .collect();
        for delay in [1_000_000.0, 0.0] {
            for lane in &mut network.lane_system.lanes {
                lane.frontage_delay_penalty_s = delay;
            }
            network.frontage_delay_revision += 1;
            demand.execute_building_actions_for_test(
                &plan,
                &mut allocator,
                &mut zoning,
                &mut agents,
                &mut households,
                &mut logistics,
                &mut 0.0,
                &graph,
                &network,
                if delay > 0.0 { &terrain } else { &blocked },
                420,
                &policy,
            );
            assert_eq!(
                allocator.buildings[0].asset_id, action.asset_id,
                "current freight prices reject the operation before a physically valid replacement"
            );
            assert_eq!(
                allocator
                    .buildings
                    .iter()
                    .map(|building| building.operating_budget.to_bits())
                    .collect::<Vec<_>>(),
                budgets
            );
        }
        for _ in 0..3 {
            demand.execute_building_actions_for_test(
                &plan,
                &mut allocator,
                &mut zoning,
                &mut agents,
                &mut households,
                &mut logistics,
                &mut 0.0,
                &graph,
                &network,
                &blocked,
                420,
                &policy,
            );
            assert_eq!(allocator.buildings[0].asset_id, action.asset_id);
        }
        demand.execute_building_actions_for_test(
            &plan,
            &mut allocator,
            &mut zoning,
            &mut agents,
            &mut households,
            &mut logistics,
            &mut 0.0,
            &graph,
            &network,
            &terrain,
            420,
            &policy,
        );
        assert_eq!(
            allocator.buildings[0].asset_id,
            if accepted { target } else { action.asset_id }
        );
        if queued {
            assert_eq!(
                demand.construction_execution.reservations.queued_workers,
                [(queued_action.parcel_id, 15)],
                "replacement must retain the supported queue workforce"
            );
            // The next planning slot proves the queued buyer on the census of the replaced city.
            let members = std::mem::take(&mut households.households);
            households.job_funding.refresh(
                &allocator,
                &members,
                &mut logistics,
                &network,
                &graph,
                &catalog,
                &tuning,
                &policy,
                &[],
                0.0,
                421,
            );
            households.households = members;
            let permit = demand.permits_for_test(
                &[(ZoneType::Commercial, &queued_action)],
                &allocator,
                &mut households,
                &zoning,
                &graph,
                &network,
                &mut logistics,
                &policy,
                0.0,
                421,
            )[0];
            assert!(
                permit.is_some(),
                "the preserved downstream commitment must remain executable"
            );
            let count = allocator.buildings.len();
            let placed = allocator.execute_single_demand_spawn_action(
                ZoneType::Commercial,
                &queued_action,
                None,
                &mut zoning,
                &graph,
                &network.lane_system,
                &surface,
                &terrain,
                &demand.runtime_catalog,
                &demand.runtime_tuning,
            );
            assert_eq!(placed.commercial.spawn_executed, 1);
            demand.finish_queued_spawn(0, &queued_action);
            assert_eq!(allocator.buildings.len(), count + 1);
            assert_eq!(allocator.buildings[count].worker_count, 0);
            assert!(allocator.buildings[count].is_under_construction());
            assert!(
                demand
                    .queued_construction
                    .iter()
                    .all(|project| !project.active)
            );
        }
    }
}

#[test]
fn cyclic_replacement_preserves_both_financing_proofs_and_exact_rejection() {
    use super::super::spawn_need::replacement_market_covers_operation;
    use super::super::supply::{ProjectReservations, ProjectSupply, SiteSupply};
    use crate::simulation::economy::definitions::compile_test_catalog;
    use crate::simulation::economy::logistics::{
        RecurringSupply, ShipmentSystem, collect_connected_border_nodes,
    };
    use crate::simulation::economy::operation::OperatingCosts;
    use crate::simulation::network::TransitNetwork;

    let authored = include_str!("../../../../../../economy/profiles.toml");
    let start = authored
        .find("[[profiles]]\nid = \"food_processor_basic\"")
        .unwrap();
    let end = start + authored[start + 1..].find("\n[[profiles]]").unwrap() + 1;
    let recipe = authored[start..end]
        .replace("worker_capacity = 10", "worker_capacity = 1")
        .replace(
            "[[profiles.inputs]]\nresource = \"machinery\"\nunits_per_day = 2.0",
            "",
        );
    let a = recipe
        .replace("food_processor_basic", "cycle_a")
        .replace("\"grain\"", "\"cycle_swap\"")
        .replace("\"packaged_food\"", "\"grain\"")
        .replace("\"cycle_swap\"", "\"packaged_food\"")
        .replace(
            "[[profiles.inputs]]\nresource = \"packaged_food\"\nunits_per_day = 200.0",
            "[[profiles.inputs]]\nresource = \"packaged_food\"\nunits_per_day = 20.0",
        );
    let a = format!("{a}\n[[profiles.inputs]]\nresource = \"cycle_feed\"\nunits_per_day = 180.0\n");
    let b = recipe
        .replace("food_processor_basic", "cycle_b")
        .replace("unit_price_currency = 12.0", "unit_price_currency = 30.0");
    // Imported feed balances B's exports. Neither full recipe can finance imported cycle
    // inputs; the accepted live book must prove the two local purchases together.
    let feed = "[[resources]]\nid = \"cycle_feed\"\nquantity_unit = \"kg\"\nshipment_batch_kg = 10.0\nruntime_id = 10\nimport_unit_price_currency = 0.01\n";
    for reverse in [false, true] {
        for (price, accepted) in [(12.5, true), (14.0, false)] {
            let target_recipe = a.replace("cycle_a", "cycle_a_target").replace(
                "unit_price_currency = 12.0",
                &format!("unit_price_currency = {price:.1}"),
            );
            let catalog =
                compile_test_catalog(&format!("{authored}\n{feed}\n{a}\n{b}\n{target_recipe}"))
                    .unwrap();
            let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
            tuning.owa_import_price_multiplier = 10.0;
            tuning.owa_export_price_multiplier = 1.0;
            let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
            let mut graph = graph_with_connected_border();
            let mut network = TransitNetwork::new();
            network.lane_system.rebuild(&mut graph);
            network.rebuild_cch_and_check(&graph);
            let mut zoning = two_lots(&graph, ZoneType::Industrial);
            let terrain = TerrainSystem::new(4, 4);
            network.road_surface.compile_dirty(&graph, &terrain);
            let mut allocator = BuildingAllocator::new();
            let old = register_family_asset_with_economy_profile(
                &mut allocator,
                "cycle_old",
                ZoneType::Industrial,
                Some("cycle_family"),
                1,
                Some("cycle_a"),
            );
            let target = register_family_asset_with_economy_profile(
                &mut allocator,
                "cycle_target",
                ZoneType::Industrial,
                Some("cycle_family"),
                2,
                Some("cycle_a_target"),
            );
            let other = register_family_asset_with_economy_profile(
                &mut allocator,
                "cycle_other",
                ZoneType::Industrial,
                None,
                1,
                Some("cycle_b"),
            );
            let owner = usize::from(reverse);
            let actions: Vec<_> = zoning
                .parcels()
                .iter()
                .enumerate()
                .map(|(index, parcel)| DemandSpawnAction {
                    parcel_id: parcel.id().raw(),
                    asset_id: if index == owner {
                        old.clone()
                    } else {
                        other.clone()
                    },
                })
                .collect();
            for action in &actions {
                let execution = allocator.execute_single_demand_spawn_action(
                    ZoneType::Industrial,
                    action,
                    None,
                    &mut zoning,
                    &graph,
                    &network.lane_system,
                    &network.road_surface,
                    &terrain,
                    &catalog,
                    &tuning,
                );
                assert_eq!(execution.industrial.spawn_executed, 1);
                let building = allocator.buildings.last_mut().unwrap();
                building.construction_complete_slot = 0;
                building.worker_count = 1;
                building.operating_budget = 1_000_000.0;
            }
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
                0.0,
                420,
            );
            assert_eq!(funding.capacities, [1, 1]);
            let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
            let food = catalog.resource_runtime_id_for_id("packaged_food").unwrap();
            assert_eq!(funding.business.sales(owner, grain, 200.0).units, 200.0);
            assert_eq!(funding.business.sales(1 - owner, food, 200.0).units, 20.0);
            let mut book = RecurringSupply::default();
            book.copy_reservations_from(&funding.input_supply, &allocator, &catalog, &graph);
            let mut reservations = ProjectReservations::default();
            reservations.reset(&funding, &allocator, &catalog, &tuning, &policy, 420);
            let operations = [
                reservations.operations[0].unwrap(),
                reservations.operations[1].unwrap(),
            ];
            let finance = |reservations: &ProjectReservations| {
                [
                    reservations.business.financing(0).map(f64::to_bits),
                    reservations.business.financing(1).map(f64::to_bits),
                ]
            };
            let input_sources = |book: &RecurringSupply| {
                let mut sources = Vec::new();
                for (buyer, operation) in operations.iter().enumerate() {
                    let profile = catalog.profile_by_runtime_id(operation.profile).unwrap();
                    OperatingCosts::for_plan_with_input_quotes(
                        profile,
                        operation.plan,
                        operation.prices,
                        |_, _| 0.0,
                        |input| {
                            assert!(book.visit_input_sources(
                                buyer,
                                input,
                                catalog.unit_price_for_resource(input.resource).unwrap()
                                    * tuning.owa_import_price_multiplier,
                                Some(operation.prices.carrier_quote),
                                |source, units, goods, carrier| sources.push((
                                    buyer,
                                    input.resource,
                                    source,
                                    units.to_bits(),
                                    goods.to_bits(),
                                    carrier.to_bits()
                                ))
                            ));
                            Default::default()
                        },
                    );
                }
                sources
            };
            let before = finance(&reservations);
            let receipts = input_sources(&book);
            assert!(
                receipts
                    .iter()
                    .any(|row| row.0 == owner && row.2 == Some(1 - owner))
            );
            assert!(
                receipts
                    .iter()
                    .any(|row| row.0 == 1 - owner && row.2 == Some(owner))
            );
            let mut borders = Vec::new();
            collect_connected_border_nodes(&graph, &mut borders);
            let market = OutputAbsorptionContext::empty(catalog.resource_count());
            let profile = catalog.profile_for_id("cycle_a_target").unwrap();
            let target_prices = OperationPrices {
                property_tax: crate::simulation::economy::fiscal::daily_property_tax(
                    ZoneType::Industrial,
                    2,
                    &policy,
                ),
                ..operations[owner].prices
            };
            let own_costs = OperatingCosts::for_plan_with_input_quotes(
                profile,
                operations[owner].plan,
                target_prices,
                |_, _| 0.0,
                |input| {
                    book.owner_input_costs(
                        owner,
                        input,
                        catalog.unit_price_for_resource(input.resource).unwrap()
                            * tuning.owa_import_price_multiplier,
                        Some(target_prices.carrier_quote),
                    )
                },
            );
            assert!(
                200.0 * price > own_costs.total() && own_costs.working_capital < 1_000_000.0,
                "both candidate prices must finance the replacement itself"
            );
            let buyer = 1 - owner;
            let buyer_profile = catalog.profile_for_id("cycle_b").unwrap();
            let local_carrier = f32::from_bits(
                receipts
                    .iter()
                    .find(|row| row.0 == buyer && row.1 == grain && row.2 == Some(owner))
                    .unwrap()
                    .5,
            );
            let buyer_costs = OperatingCosts::for_plan_with_input_quotes(
                buyer_profile,
                operations[buyer].plan,
                operations[buyer].prices,
                |_, units| (units - 20.0).max(0.0),
                |input| input.at_prices(price as f32, local_carrier),
            );
            let buyer_revenue = 20.0 * f64::from(buyer_profile.unit_price_currency)
                + 180.0
                    * f64::from(
                        catalog.unit_price_for_resource(food).unwrap()
                            * tuning.owa_export_price_multiplier,
                    );
            assert_eq!(
                buyer_revenue > buyer_costs.total(),
                accepted,
                "the rejected price must fail the other recipe's recurring margin"
            );
            assert!(buyer_costs.working_capital < 1_000_000.0);
            for _ in 0..3 {
                let mut supply = SiteSupply::new(
                    &mut book,
                    &mut reservations,
                    &allocator,
                    &zoning,
                    &network,
                    &graph,
                    &mut logistics,
                    &tuning,
                    &borders,
                    &catalog,
                    420,
                    policy.household_vat_rate,
                );
                let carrier = supply.begin_replacement(owner, actions[owner].parcel_id, &target);
                let prices = supply.prices(&catalog, ZoneType::Industrial, 2, &policy, carrier);
                assert_eq!(
                    replacement_market_covers_operation(
                        profile,
                        &catalog,
                        market.market(),
                        prices,
                        tuning.owa_export_price_multiplier,
                        &allocator.buildings[owner],
                        0.0,
                        &mut supply
                    ),
                    accepted,
                    "cycle price={price}, reverse={reverse}"
                );
                supply.reject();
                assert_eq!(finance(&reservations), before);
                assert_eq!(input_sources(&book), receipts);
                assert_eq!(
                    reservations.business.sales(owner, grain, 200.0).revenue,
                    2_400.0
                );
                assert_eq!(
                    reservations.business.sales(1 - owner, food, 200.0).revenue,
                    600.0
                );
            }
            let mut demand = DemandSystem::new();
            demand.runtime_catalog = std::sync::Arc::new(catalog);
            demand.runtime_tuning = std::sync::Arc::new(tuning);
            let demand_catalog = demand.runtime_catalog.clone();
            let demand_tuning = demand.runtime_tuning.clone();
            let mut plan = DemandBuildingActionPlan::default();
            plan.industrial.upgrades.push(DemandLevelChangeAction {
                building: demand_building_action_key(&allocator.buildings[owner]),
                target_asset_id: target.clone(),
            });
            let mut blocked = terrain.clone();
            for z in 0..blocked.height {
                for x in 0..blocked.width {
                    blocked.set_height(x, z, 20.0);
                }
            }
            let cash: Vec<_> = allocator
                .buildings
                .iter()
                .map(|building| building.operating_budget.to_bits())
                .collect();
            let mut agents = crate::simulation::economy::agents::AgentSystem::new();
            // The gameplay executor proves the replacement in a construction epoch on the census
            // the town publishes; the next census sees what it placed (`SIM-02-I`).
            let census = |allocator: &BuildingAllocator, logistics: &mut ShipmentSystem| {
                let mut households = HouseholdSystem::new();
                households.job_funding.refresh(
                    allocator,
                    &[],
                    logistics,
                    &network,
                    &graph,
                    &demand_catalog,
                    &demand_tuning,
                    &policy,
                    &[],
                    0.0,
                    420,
                );
                households
            };
            let books = |households: &HouseholdSystem, allocator: &BuildingAllocator| {
                let funding = &households.job_funding;
                (
                    allocator
                        .buildings
                        .iter()
                        .map(|building| {
                            (
                                building.asset_id.clone(),
                                building.operating_budget.to_bits(),
                                building.construction_complete_slot,
                            )
                        })
                        .collect::<Vec<_>>(),
                    [0, 1].map(|index| funding.business.financing(index).map(f64::to_bits)),
                    [
                        funding.business.sales(owner, grain, 200.0),
                        funding.business.sales(1 - owner, food, 200.0),
                    ]
                    .map(|sales| (sales.units.to_bits(), sales.revenue.to_bits())),
                )
            };
            // Persisted inputs as a save would hold them before either execution attempt.
            let mut cold = (allocator.clone(), zoning.clone(), logistics.clone());
            for ground in [&blocked, &terrain] {
                let mut households = census(&allocator, &mut logistics);
                demand.execute_building_actions_for_test(
                    &plan,
                    &mut allocator,
                    &mut zoning,
                    &mut agents,
                    &mut households,
                    &mut logistics,
                    &mut 0.0,
                    &graph,
                    &network,
                    ground,
                    420,
                    &policy,
                );
                let placed = accepted && std::ptr::eq(ground, &terrain);
                assert_eq!(
                    allocator.buildings[owner].asset_id,
                    if placed { &target } else { &old }.as_str()
                );
                assert_eq!(
                    allocator
                        .buildings
                        .iter()
                        .map(|building| building.operating_budget.to_bits())
                        .collect::<Vec<_>>(),
                    cash
                );
                let next = census(&allocator, &mut logistics);
                assert_eq!(
                    next.job_funding.business.sales(owner, grain, 200.0).revenue,
                    if placed { 200.0 * price } else { 2_400.0 }
                );
                assert_eq!(
                    next.job_funding
                        .business
                        .sales(1 - owner, food, 200.0)
                        .revenue,
                    600.0
                );
            }
            // Reload rebuilds every derived book cold. The rejected attempt restored the saved
            // state exactly, so a cold replay of the cyclic replacement must match bit for bit.
            let mut reloaded = DemandSystem::new();
            reloaded.runtime_catalog = demand.runtime_catalog.clone();
            reloaded.runtime_tuning = demand.runtime_tuning.clone();
            let mut cold_households = census(&cold.0, &mut cold.2);
            reloaded.execute_building_actions_for_test(
                &plan,
                &mut cold.0,
                &mut cold.1,
                &mut crate::simulation::economy::agents::AgentSystem::new(),
                &mut cold_households,
                &mut cold.2,
                &mut 0.0,
                &graph,
                &network,
                &terrain,
                420,
                &policy,
            );
            assert_eq!(
                books(&census(&cold.0, &mut cold.2), &cold.0),
                books(&census(&allocator, &mut logistics), &allocator),
                "cold cyclic replacement must match warm execution, price={price}, reverse={reverse}"
            );
        }
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
            None,
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
        factory.construction_complete_slot = 0;
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
        100_000.0,
        420,
    );
    assert_eq!(&funding.capacities[1..], &[10, 0]);
    let mut borders = Vec::new();
    collect_connected_border_nodes(&graph, &mut borders);
    let mut projected = RecurringSupply::default();
    for reject in [false, true] {
        projected.copy_reservations_from(&funding.input_supply, &allocator, &catalog, &graph);
        logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &borders);
        let mut reservations = super::super::supply::ProjectReservations::default();
        reservations.reset(&funding, &allocator, &catalog, &tuning, &policy, 420);
        let supply = SiteSupply::new(
            &mut projected,
            &mut reservations,
            &allocator,
            &zoning,
            &network,
            &graph,
            &mut logistics,
            &tuning,
            &borders,
            &catalog,
            420,
            policy.household_vat_rate,
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
        let mut test_policy = policy;
        if reject {
            test_policy.property_tax_level_multiplier = 1_000_000.0;
        }
        let mut demand = DemandSystem::new();
        demand.upgrade_action_credit.industrial = 2.0;
        demand.plan_hour_for_test(
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
    for (index, (profile_id, area)) in [
        ("machinery_factory_basic", 1.0),
        ("grain_farm_basic", 10.0),
        ("coal_mine_basic", 1.0),
        ("water_plant_basic", 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let asset = if index == 3 {
            register_test_utility_asset(&mut allocator, "pending_water", profile_id)
        } else {
            register_explicit_profile_asset(&mut allocator, profile_id, profile_id)
        };
        let profile = catalog.profile_for_id(profile_id).unwrap();
        let mut site = building(
            if index == 0 {
                ZoneType::Industrial
            } else {
                ZoneType::None
            },
            0.0,
            0,
            0,
            asset,
        );
        site.economy_profile_runtime_id = profile.runtime_id;
        site.work_area_scale = area;
        site.worker_count = if index == 0 {
            profile.worker_capacity
        } else {
            0
        };
        site.center_x = 10.0 + index as f32 * 10.0;
        site.center_y = if index == 0 { 20.0 } else { -20.0 };
        site.frontage_t = site.center_x / 50.0;
        site.parcel_id = 10_000 + index as u64;
        site.construction_total_hours = if index == 0 { 0 } else { 10 };
        site.set_construction_hours_left(site.construction_total_hours);
        allocator.buildings.push(site);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let coal = catalog.resource_runtime_id_for_id("coal").unwrap();
    let mut market = OutputAbsorptionContext::from_resource_amounts(
        catalog.resource_count(),
        &[(grain, 200.0), (coal, 9600.0)],
        &[],
        1,
        &[],
    );
    market.set_export_expansion_available(true);
    let service_door = allocator.entrances[3].clone();
    let mut borders = Vec::new();
    collect_connected_border_nodes(&graph, &mut borders);
    for (cash, paid_cargo, reachable) in [
        (100_000.0, 0.0, true),
        (0.0, 0.0, true),
        (0.0, 100_000.0, true),
        (100_000.0, 0.0, false),
    ] {
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
            100_000.0,
            420,
        );
        assert_eq!(
            &funding.capacities[1..],
            &[0, 0, 0],
            "unfinished projects never advertise live jobs"
        );
        assert_eq!(&funding.service_capacities[1..], &[0, 0, 3]);
        // Isolate project replay: a previously validated staffed source offers 40/day.
        let mut book = RecurringSupply::default();
        book.reset(
            &allocator,
            &graph,
            &network,
            &catalog,
            &[1.0, 0.0, 0.0, 0.0],
        );
        let mut reservations = super::super::supply::ProjectReservations::default();
        reservations.business.reset(
            &allocator,
            &network,
            &graph,
            book.freight_components().unwrap(),
            &catalog,
            0,
            0,
            |_| {},
        );
        logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &borders);
        SiteSupply::new(
            &mut book,
            &mut reservations,
            &allocator,
            &zoning,
            &network,
            &graph,
            &mut logistics,
            &tuning,
            &borders,
            &catalog,
            420,
            policy.household_vat_rate,
        )
        .reserve_projects(
            std::iter::empty(),
            &catalog,
            &market,
            &mut Default::default(),
            &policy,
            &funding.service_capacities,
            &[0.0, paid_cargo, paid_cargo, 0.0],
            |_| true,
        );
        // Ten hectares need 1 machinery/day; the one-hectare mine needs 4. The public
        // water plant needs 0.5 independently of private capital, if its door is reachable.
        let residual =
            40.0 - if f64::from(cash) + paid_cargo > 0.0 {
                5.0
            } else {
                0.0
            } - if reachable { 0.5 } else { 0.0 };
        for (units, expected) in [(residual, true), (residual + 0.25, false)] {
            let costs = logistics.quote_recurring_input(
                &mut book,
                InputRequirement {
                    resource: machinery,
                    daily_units: units,
                    buffer_units: 2.0 * units,
                    replenishment_batch_units: Some(40.0),
                },
                None,
                &service_door,
                None,
                &allocator,
                &catalog,
                &tuning,
                &network,
                &graph,
                1.0,
            );
            assert_eq!(
                costs.goods.is_finite(),
                expected,
                "cash={cash} cargo={paid_cargo} reachable={reachable} residual={residual}"
            );
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
        site.construction_total_hours = if constructing { 10 } else { 0 };
        site.set_construction_hours_left(site.construction_total_hours);
        site.operating_budget = 100_000.0;
        allocator.buildings.push(site);
    }
    let home_asset = register_test_asset(
        &mut allocator,
        "replay_customer_home",
        ZoneType::Residential,
    );
    let mut home = building(ZoneType::Residential, 0.0, 1, 0, home_asset);
    home.center_x = 10.0;
    home.center_y = 20.0;
    home.frontage_t = 0.2;
    allocator.buildings.push(home);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut family = housed_household(4, 300, 0.0, 0.0);
    family.employed_adults = 300;
    family.gross_daily_wages = 100_000.0;
    let mut reservations = super::super::supply::ProjectReservations::default();
    reservations.customers.reset(
        &[family],
        &allocator,
        &catalog,
        &tuning,
        &policy,
        0.0,
        &network,
        &graph,
    );
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
    book.reset(
        &allocator,
        &graph,
        &network,
        &catalog,
        &[1.0, 1.0, 0.0, 0.0],
    );
    reservations.business.reset(
        &allocator,
        &network,
        &graph,
        book.freight_components().unwrap(),
        &catalog,
        0,
        0,
        |_| {},
    );
    let mut logistics = ShipmentSystem::new();
    let mut borders = Vec::new();
    collect_connected_border_nodes(&graph, &mut borders);
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &borders);
    SiteSupply::new(
        &mut book,
        &mut reservations,
        &allocator,
        &zoning,
        &network,
        &graph,
        &mut logistics,
        &tuning,
        &borders,
        &catalog,
        420,
        policy.household_vat_rate,
    )
    .reserve_projects(
        std::iter::empty(),
        &catalog,
        &market,
        &mut Default::default(),
        &policy,
        &[],
        &[],
        |_| true,
    );
    assert_eq!(&reservations.building_workers[2..4], &[15, 8]);
    let purchased: f64 = (0..2)
        .map(|owner| {
            reservations
                .business
                .sales(owner, food, f64::INFINITY)
                .units
        })
        .sum();
    assert_eq!(
        purchased, 300.0,
        "constructing buyers publish their supported input quantities"
    );
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
            &graph,
            1.0,
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
fn business_projects_share_financed_customers_through_replay_and_rollback() {
    use super::super::spawn_need::project_covers_operation;
    use super::super::supply::{ProjectReservations, ProjectSupply, SiteSupply};
    use crate::simulation::economy::hiring::JobFunding;
    use crate::simulation::economy::logistics::{
        RecurringSupply, ShipmentSystem, collect_connected_border_nodes,
    };
    use crate::simulation::network::TransitNetwork;
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    let receiving = tuning
        .operational_clock
        .freight_profiles
        .iter()
        .find(|profile| profile.id == "early_morning_preferred")
        .unwrap()
        .clone();
    // Give the factory its own receiving boundary; the shipped recipe is always open.
    let factory_freight = tuning
        .operational_clock
        .freight_profiles
        .iter_mut()
        .find(|profile| profile.id == "always_open")
        .unwrap();
    factory_freight.preferred_windows = receiving.preferred_windows;
    factory_freight.outside_window_cost_multiplier = receiving.outside_window_cost_multiplier;
    let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    let mut graph = graph_with_connected_border();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let zoning = two_lots(&graph, ZoneType::Industrial);
    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(
        &mut allocator,
        "business_customer_home",
        ZoneType::Residential,
    );
    let shop_asset = register_test_asset(
        &mut allocator,
        "business_customer_shop",
        ZoneType::Commercial,
    );
    let factory_asset =
        register_test_asset(&mut allocator, "business_project", ZoneType::Industrial);
    for (index, (zone, asset, workers)) in [
        (ZoneType::Residential, home_asset, 0),
        (ZoneType::Commercial, shop_asset, 15),
    ]
    .into_iter()
    .enumerate()
    {
        let mut site = building(zone, 0.0, 0, workers, asset);
        site.center_x = 10.0 + index as f32 * 20.0;
        site.center_y = 20.0;
        site.frontage_t = site.center_x / 50.0;
        site.operating_budget = 100_000.0;
        allocator.buildings.push(site);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut family = housed_household(0, 200, 0.0, 0.0);
    family.employed_adults = 200;
    family.gross_daily_wages = 100_000.0;
    let mut funding = JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    let mut book = RecurringSupply::default();
    let mut reservations = ProjectReservations::default();
    let mut borders = Vec::new();
    collect_connected_border_nodes(&graph, &mut borders);
    let action = DemandSpawnAction {
        parcel_id: zoning.parcels()[0].id().raw(),
        asset_id: factory_asset.clone(),
    };
    let food = catalog.resource_runtime_id_for_id("packaged_food").unwrap();
    // Deliberately wrong aggregate demand cannot create a customer or erase a real one.
    let market = OutputAbsorptionContext::from_resource_amounts(
        catalog.resource_count(),
        &[],
        &[(food, 100_000.0)],
        0,
        &[],
    );
    let mut profile = catalog
        .profile_for_id("food_processor_basic")
        .unwrap()
        .clone();
    profile.unit_price_currency = 14.0;
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let mut supplier = catalog.profile_for_id("grain_farm_basic").unwrap().clone();
    supplier.inputs.clear();
    let supplier_parcel = zoning.parcels()[1].id().raw();
    for (funded, minute, residents) in [
        (true, 420, 200),
        (true, 419, 200),
        (true, 480, 200),
        (true, 420, 100),
        (false, 420, 200),
    ] {
        family.member_count = residents;
        family.employed_adults = residents;
        allocator.buildings[1].operating_budget = if funded { 100_000.0 } else { 0.0 };
        funding.refresh(
            &allocator,
            std::slice::from_ref(&family),
            &mut logistics,
            &network,
            &graph,
            &catalog,
            &tuning,
            &policy,
            &[],
            0.0,
            minute,
        );
        assert_eq!(
            funding.capacities[1],
            if funded {
                (15 * u32::from(residents)).div_ceil(200)
            } else {
                0
            }
        );
        reservations.reset(&funding, &allocator, &catalog, &tuning, &policy, minute);
        assert!(
            reservations.operations[allocator.buildings.len()..]
                .iter()
                .all(Option::is_none),
            "accepted projects from the preceding pass must release their retained owner slots"
        );
        book.copy_reservations_from(&funding.input_supply, &allocator, &catalog, &graph);
        logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &borders);
        let mut supply = SiteSupply::new(
            &mut book,
            &mut reservations,
            &allocator,
            &zoning,
            &network,
            &graph,
            &mut logistics,
            &tuning,
            &borders,
            &catalog,
            minute,
            policy.household_vat_rate,
        );
        let before = supply
            .reservations()
            .unwrap()
            .business
            .financing(1)
            .map(f64::to_bits);
        for _ in 0..2 {
            let carrier = supply.begin(action.parcel_id, &action.asset_id);
            let prices = supply.prices(&catalog, ZoneType::Industrial, 1, &policy, carrier);
            assert_eq!(
                project_covers_operation(
                    &profile,
                    &catalog,
                    market.market(),
                    prices,
                    0.0,
                    &mut supply
                ),
                funded
            );
            if funded {
                assert_eq!(supply.supported_workers(0), u32::from(residents) / 20);
                let multiplier = crate::simulation::economy::logistics::freight_goods_multiplier(
                    catalog.profile_for_id("grocery_basic").unwrap(),
                    ZoneType::Commercial,
                    &tuning,
                    minute,
                );
                assert_eq!(
                    supply.output_price(food),
                    Some(f64::from(14.0 * multiplier) * f64::from(residents) / 200.0)
                );
            }
            supply.reject();
            assert_eq!(
                supply
                    .reservations()
                    .unwrap()
                    .business
                    .financing(1)
                    .map(f64::to_bits),
                before
            );
            supply.begin(supplier_parcel, &action.asset_id);
            let quote = supply.customers(&supplier, 100.0, prices, 0.0).unwrap();
            assert_eq!(
                quote.revenue, 0.0,
                "a rejected project must not advertise future purchases"
            );
            supply.reject();
        }
        if funded {
            let carrier = supply.begin(action.parcel_id, &action.asset_id);
            let prices = supply.prices(&catalog, ZoneType::Industrial, 1, &policy, carrier);
            let planner_prices = OperationPrices {
                imports: tuning.owa_import_price_multiplier,
                ..prices
            };
            assert!(project_covers_operation(
                &profile,
                &catalog,
                market.market(),
                planner_prices,
                0.0,
                &mut supply
            ));
            supply.accept();
            let operation =
                supply.reservations().unwrap().operations[allocator.buildings.len()].unwrap();
            assert_eq!(operation.plan.workers, u32::from(residents) / 20);
            assert_eq!(
                operation.prices.imports, prices.imports,
                "retained outside prices include the site's current receiving-window charge"
            );
            let capital = f64::from(
                profile.zoned_startup_budget(&catalog, tuning.owa_import_price_multiplier),
            );
            let receiving = crate::simulation::economy::logistics::freight_goods_multiplier(
                &profile,
                ZoneType::Industrial,
                &tuning,
                minute,
            );
            let costs =
                crate::simulation::economy::operation::OperatingCosts::for_plan_with_input_quotes(
                    &profile,
                    operation.plan,
                    operation.prices,
                    |_, _| 0.0,
                    |input| {
                        input.at_prices(
                            catalog.unit_price_for_resource(input.resource).unwrap()
                                * tuning.owa_import_price_multiplier
                                * receiving,
                            operation.prices.carrier_quote,
                        )
                    },
                );
            assert_eq!(
                operation.headroom[1],
                capital - costs.working_capital,
                "off-window quotes cannot enlarge the equity actually supplied by placement"
            );
            if minute == 480 {
                assert!(operation.prices.imports > planner_prices.imports);
            }
            let mut grain_units = 0.0;
            crate::simulation::economy::operation::OperatingCosts::for_plan_with_input_quotes(
                &profile,
                operation.plan,
                operation.prices,
                |_, _| 0.0,
                |input| {
                    if input.resource == grain {
                        grain_units = f64::from(input.daily_units);
                    }
                    Default::default()
                },
            );
            assert_eq!(grain_units, f64::from(residents));
            let multiplier = crate::simulation::economy::logistics::freight_goods_multiplier(
                &profile,
                ZoneType::Industrial,
                &tuning,
                minute,
            );
            let future_finance = supply
                .reservations()
                .unwrap()
                .business
                .financing(allocator.buildings.len())
                .map(f64::to_bits);
            let storage = supply.reservations().unwrap().business.account_storage();
            for _ in 0..16 {
                supply.begin(supplier_parcel, &action.asset_id);
                let quote = supply.customers(&supplier, 100.0, prices, 0.0).unwrap();
                assert!(
                    (quote.revenue
                        - grain_units * f64::from(supplier.unit_price_currency * multiplier))
                    .abs()
                        < 0.001,
                    "accepted project purchases follow supported activity and the buyer's receiving price"
                );
                supply.reject();
                assert_eq!(
                    supply
                        .reservations()
                        .unwrap()
                        .business
                        .financing(allocator.buildings.len())
                        .map(f64::to_bits),
                    future_finance
                );
                assert_eq!(
                    supply.reservations().unwrap().business.account_storage(),
                    storage
                );
            }
            supply.begin(action.parcel_id, &action.asset_id);
            assert!(
                !project_covers_operation(
                    &profile,
                    &catalog,
                    market.market(),
                    prices,
                    0.0,
                    &mut supply
                ),
                "the next project cannot reuse the shop's purchases"
            );
            supply.reject();
        }
    }
    allocator.buildings[1].operating_budget = 100_000.0;
    let mut constructing = building(ZoneType::Industrial, 0.0, 0, 0, factory_asset);
    constructing.center_x = 45.0;
    constructing.center_y = 20.0;
    constructing.frontage_t = 0.9;
    constructing.operating_budget = 100_000.0;
    constructing.set_construction_hours_left(12);
    constructing.construction_total_hours = 12;
    allocator.buildings.push(constructing);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut demand = DemandSystem::new();
    demand.set_queued_construction_with_entrances(
        &allocator,
        &zoning,
        0,
        [(0, 1, ZoneType::Industrial, &action)],
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
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &borders);
    // Disable exports so replay must carry the one shop's finite business account.
    let mut local_tuning = tuning.clone();
    local_tuning.owa_export_price_multiplier = 0.0;
    for (cash, constructing_workers, queued_workers) in [(100_000.0, 10, 0), (0.0, 0, 10)] {
        allocator.buildings[2].operating_budget = cash;
        funding.refresh(
            &allocator,
            std::slice::from_ref(&family),
            &mut logistics,
            &network,
            &graph,
            &catalog,
            &tuning,
            &policy,
            &[],
            0.0,
            420,
        );
        reservations.reset(&funding, &allocator, &catalog, &tuning, &policy, 420);
        book.copy_reservations_from(&funding.input_supply, &allocator, &catalog, &graph);
        let mut supply = SiteSupply::new(
            &mut book,
            &mut reservations,
            &allocator,
            &zoning,
            &network,
            &graph,
            &mut logistics,
            &local_tuning,
            &borders,
            &catalog,
            420,
            policy.household_vat_rate,
        );
        supply.reserve_projects(
            demand.queued_construction.iter(),
            &catalog,
            &market,
            &mut Default::default(),
            &policy,
            &[],
            &[],
            |_| true,
        );
        assert_eq!(
            supply.reservations().unwrap().building_workers[2],
            constructing_workers
        );
        assert_eq!(
            supply.reservations().unwrap().queued_workers,
            [(action.parcel_id, queued_workers)]
        );
        supply.begin(supplier_parcel, &action.asset_id);
        let prices = supply.prices(&catalog, ZoneType::Industrial, 1, &policy, None);
        let quote = supply.customers(&supplier, 100.0, prices, 0.0).unwrap();
        assert_eq!(
            quote.revenue,
            200.0 * f64::from(supplier.unit_price_currency),
            "only the financed constructing or queued buyer advertises its grain purchases"
        );
        supply.reject();
    }
}

#[test]
fn kept_pass_rows_equal_a_full_recomputation_across_lendings() {
    use super::super::supply::ProjectReservations;
    use crate::simulation::economy::hiring::JobFunding;
    use crate::simulation::economy::logistics::ShipmentSystem;
    use crate::simulation::network::TransitNetwork;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    let mut graph = graph_with_connected_border();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(&mut allocator, "kept_rows_home", ZoneType::Residential);
    let shop_asset = register_test_asset(&mut allocator, "kept_rows_shop", ZoneType::Commercial);
    for (index, (zone, asset, workers)) in [
        (ZoneType::Residential, home_asset, 0),
        (ZoneType::Commercial, shop_asset.clone(), 15),
    ]
    .into_iter()
    .enumerate()
    {
        let mut site = building(zone, 0.0, 0, workers, asset);
        site.center_x = 10.0 + index as f32 * 20.0;
        site.center_y = 20.0;
        site.frontage_t = site.center_x / 50.0;
        site.operating_budget = 100_000.0;
        allocator.buildings.push(site);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut family = housed_household(0, 200, 0.0, 0.0);
    family.employed_adults = 200;
    family.gross_daily_wages = 100_000.0;
    let mut funding = JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    funding.refresh(
        &allocator,
        std::slice::from_ref(&family),
        &mut logistics,
        &network,
        &graph,
        &catalog,
        &tuning,
        &policy,
        &[],
        0.0,
        420,
    );
    assert!(
        funding.capacities[1] > 0,
        "the shop must hold a funded operation row"
    );
    // Test builds compare the rows of every pass start with a full recomputation.
    let mut reservations = ProjectReservations::default();
    reservations.reset(&funding, &allocator, &catalog, &tuning, &policy, 420);
    assert_eq!(
        reservations.kept_row_starts, 0,
        "a first lending derives every row"
    );
    // The same book, phase and policy, with a recorded edit and an appended building.
    allocator.set_service_funding_override(1, 0.5);
    let mut appended = building(ZoneType::Commercial, 0.0, 0, 15, shop_asset);
    appended.center_x = 50.0;
    appended.center_y = 20.0;
    allocator.buildings.push(appended);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    reservations.reset(&funding, &allocator, &catalog, &tuning, &policy, 421);
    assert_eq!(
        reservations.kept_row_starts, 1,
        "a later lending keeps the unchanged rows"
    );
    // Another price phase, policy or book derives every row again.
    let mut taxed = policy;
    taxed.income_tax_rate += 0.01;
    reservations.reset(&funding, &allocator, &catalog, &tuning, &taxed, 421);
    assert_eq!(reservations.kept_row_starts, 1);
    funding.refresh(
        &allocator,
        std::slice::from_ref(&family),
        &mut logistics,
        &network,
        &graph,
        &catalog,
        &tuning,
        &taxed,
        &[],
        0.0,
        421,
    );
    reservations.reset(&funding, &allocator, &catalog, &tuning, &taxed, 421);
    assert_eq!(reservations.kept_row_starts, 1);
    reservations.reset(&funding, &allocator, &catalog, &tuning, &taxed, 422);
    assert_eq!(reservations.kept_row_starts, 2);
}

#[test]
fn service_projects_share_live_customers_and_replacement_rollback() {
    use super::super::spawn_need::replacement_market_covers_operation;
    use super::super::supply::{ProjectReservations, ProjectSupply, SiteSupply};
    use crate::simulation::economy::hiring::JobFunding;
    use crate::simulation::economy::logistics::{ShipmentSystem, collect_connected_border_nodes};
    use crate::simulation::network::TransitNetwork;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    let profile = catalog.profile_for_id("personal_service_small").unwrap();
    assert!(profile.inputs.is_empty());
    let mut graph = graph_with_connected_border();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let zoning = two_lots(&graph, ZoneType::Commercial);
    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(
        &mut allocator,
        "service_customer_home",
        ZoneType::Residential,
    );
    let shop_asset = register_family_asset_with_economy_profile(
        &mut allocator,
        "service_customer_shop",
        ZoneType::Commercial,
        None,
        1,
        Some("personal_service_small"),
    );
    for index in 0..3 {
        let mut site = building(
            if index == 0 {
                ZoneType::Residential
            } else {
                ZoneType::Commercial
            },
            0.0,
            0,
            if index == 1 { 4 } else { 0 },
            if index == 0 {
                home_asset.clone()
            } else {
                shop_asset.clone()
            },
        );
        site.center_x = 10.0 + index as f32 * 10.0;
        site.center_y = if index == 0 { 20.0 } else { -20.0 };
        site.frontage_t = site.center_x / 50.0;
        site.operating_budget = 100_000.0;
        if index > 0 {
            site.economy_profile_runtime_id = profile.runtime_id;
        }
        if index == 2 {
            site.set_construction_hours_left(12);
            site.construction_total_hours = 12;
        }
        allocator.buildings.push(site);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    // 100 units/day: live shop owns 80, construction gets 20, the queue gets none.
    let mut family = housed_household(0, 1000, 0.0, 0.0);
    family.employed_adults = 1000;
    family.gross_daily_wages = 100_000.0;
    let mut logistics = ShipmentSystem::new();
    let mut funding = JobFunding::default();
    funding.refresh(
        &allocator,
        &[family],
        &mut logistics,
        &network,
        &graph,
        &catalog,
        &tuning,
        &policy,
        &[],
        0.0,
        420,
    );
    assert_eq!(funding.capacities[1], 4);
    let mut reservations = ProjectReservations::default();
    reservations.reset(&funding, &allocator, &catalog, &tuning, &policy, 420);
    let action = DemandSpawnAction {
        parcel_id: zoning.parcels()[0].id().raw(),
        asset_id: shop_asset.clone(),
    };
    let mut demand = DemandSystem::new();
    demand.set_queued_construction_with_entrances(
        &allocator,
        &zoning,
        0,
        [(0, 1, ZoneType::Commercial, &action)],
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
    let mut borders = Vec::new();
    collect_connected_border_nodes(&graph, &mut borders);
    logistics.prepare_site_carrier_quotes(&allocator, &network, &graph, &borders);
    let market = OutputAbsorptionContext::empty(catalog.resource_count());
    let mut supply = SiteSupply::new(
        &mut funding.input_supply,
        &mut reservations,
        &allocator,
        &zoning,
        &network,
        &graph,
        &mut logistics,
        &tuning,
        &borders,
        &catalog,
        420,
        policy.household_vat_rate,
    );
    supply.reserve_projects(
        demand.queued_construction.iter(),
        &catalog,
        &market,
        &mut Default::default(),
        &policy,
        &[],
        &[],
        |_| true,
    );
    assert_eq!(supply.reservations().unwrap().building_workers[2], 1);
    assert_eq!(
        supply.reservations().unwrap().queued_workers,
        [(action.parcel_id, 0)]
    );
    let operation_bits = |operation: crate::simulation::economy::operation::FundedOperation| {
        (
            operation.profile,
            operation.plan.workers,
            [
                operation.plan.activity,
                operation.plan.area,
                operation.prices.utilities[0],
                operation.prices.utilities[1],
                operation.prices.utilities[2],
                operation.prices.property_tax,
                operation.prices.imports,
                operation.prices.carrier_quote,
                operation.prices.truck_payload,
            ]
            .map(f32::to_bits),
            operation.headroom.map(f64::to_bits),
        )
    };
    let live = supply.reservations().unwrap().operations[1].unwrap();
    let construction = supply.reservations().unwrap().operations[2].unwrap();
    assert_eq!(
        (live.profile, live.plan.workers, live.plan.activity),
        (profile.runtime_id, 4, 1.0)
    );
    assert_eq!(
        (
            construction.profile,
            construction.plan.workers,
            construction.plan.activity
        ),
        (profile.runtime_id, 1, 0.25)
    );
    assert!(construction.headroom[0] > 0.0 && construction.headroom[1] >= 0.0);
    let mut snapshot = vacant_admission_snapshot();
    snapshot.existing_unemployed_member_count = 2;
    snapshot.open_job_slots = 0;
    snapshot.connected_border_count = 0;
    let mut calendar = super::super::workforce::WorkforceReservations::default();
    calendar.reset(
        &allocator,
        &catalog,
        &tuning,
        &snapshot,
        &demand.queued_construction,
        0,
        supply.reservations(),
        |_| true,
    );
    calendar.reserve_existing();
    assert!(calendar.claim(12, 1));
    assert!(
        !calendar.claim(12, 1),
        "construction reserved its one customer-supported worker"
    );
    let prices = supply.prices(&catalog, ZoneType::Commercial, 1, &policy, None);
    for _ in 0..3 {
        supply.begin_replacement(1, action.parcel_id, &shop_asset);
        assert!(!replacement_market_covers_operation(
            profile,
            &catalog,
            market.market(),
            OperationPrices {
                property_tax: 1_000_000.0,
                ..prices
            },
            0.0,
            &allocator.buildings[1],
            0.0,
            &mut supply
        ));
        supply.reject();
        assert_eq!(
            operation_bits(supply.reservations().unwrap().operations[1].unwrap()),
            operation_bits(live)
        );
        assert_eq!(
            operation_bits(supply.reservations().unwrap().operations[2].unwrap()),
            operation_bits(construction)
        );
        supply.begin(action.parcel_id, &shop_asset);
        assert_eq!(
            supply.customers(profile, 1.0, prices, 0.0).unwrap(),
            Default::default(),
            "failed replacement cannot free either live or constructing customers"
        );
        supply.reject();
    }
    supply.begin_replacement(1, action.parcel_id, &shop_asset);
    assert!(replacement_market_covers_operation(
        profile,
        &catalog,
        market.market(),
        prices,
        0.0,
        &allocator.buildings[1],
        0.0,
        &mut supply
    ));
    assert_eq!(supply.supported_workers(0), 4);
    supply.accept();
    let replacement = supply.reservations().unwrap().operations[1].unwrap();
    assert_eq!(
        (
            replacement.profile,
            replacement.plan.workers,
            replacement.plan.activity
        ),
        (profile.runtime_id, 4, 1.0)
    );
    assert!(replacement.headroom[0] > 0.0 && replacement.headroom[1] >= 0.0);
    assert_eq!(
        operation_bits(supply.reservations().unwrap().operations[2].unwrap()),
        operation_bits(construction)
    );
    supply.begin(action.parcel_id, &shop_asset);
    assert_eq!(
        supply.customers(profile, 1.0, prices, 0.0).unwrap(),
        Default::default()
    );
    supply.reject();
}

#[test]
fn replacement_payroll_caps_retained_workers_at_target_slots() {
    use super::super::spawn_need::replacement_market_covers_operation;
    use super::super::supply::{ProjectCustomers, ProjectSupply};
    use crate::simulation::economy::definitions::{EconomyProfileRuntime, RuntimeEconomyCatalog};
    use crate::simulation::economy::operation::{
        InputRequirement, InputSupplyCosts, OperationPlan,
    };
    struct Quote {
        plan: Option<OperationPlan>,
    }
    impl ProjectSupply for Quote {
        fn customers(
            &mut self,
            profile: &EconomyProfileRuntime,
            _: f32,
            _: OperationPrices,
            _: f32,
        ) -> Option<ProjectCustomers> {
            Some(ProjectCustomers {
                activity: 0.25,
                revenue: f64::from(profile.average_daily_wage())
                    * (f64::from(profile.worker_capacity) + 0.5),
                export_freight: 0.0,
            })
        }
        fn set_supported_plan(&mut self, plan: OperationPlan) {
            self.plan = Some(plan);
        }
        fn begin(&mut self, _: u64, _: &str) -> Option<f32> {
            None
        }
        fn quote(
            &mut self,
            _: InputRequirement,
            _: &RuntimeEconomyCatalog,
            _: OperationPrices,
        ) -> InputSupplyCosts {
            panic!("input-free service")
        }
        fn accept(&mut self) {}
        fn reject(&mut self) {}
    }
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("personal_service_small").unwrap();
    let market = OutputAbsorptionContext::empty(catalog.resource_count());
    let prices = OperationPrices {
        utilities: [0.0; 3],
        property_tax: 0.0,
        imports: 1.0,
        carrier_quote: f32::INFINITY,
        truck_payload: 1_000.0,
    };
    let mut incumbent = building(ZoneType::Commercial, 0.0, 0, 15, "test:larger_shop".into());
    incumbent.operating_budget = 100_000.0;
    assert!(incumbent.worker_count > profile.worker_capacity);
    let mut quote = Quote { plan: None };
    assert!(
        replacement_market_covers_operation(
            profile,
            &catalog,
            market.market(),
            prices,
            0.0,
            &incumbent,
            0.0,
            &mut quote
        ),
        "sales cover the target's retained payroll"
    );
    let plan = quote.plan.unwrap();
    assert_eq!(
        (plan.workers, plan.activity),
        (profile.worker_capacity, 0.25)
    );
    incumbent.operating_budget = 0.0;
    assert!(
        !replacement_market_covers_operation(
            profile,
            &catalog,
            market.market(),
            prices,
            0.0,
            &incumbent,
            0.0,
            &mut quote
        ),
        "positive recurring margin still requires working capital"
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
    let snapshot = planning_snapshot(workers, sales);
    let terrain = TerrainSystem::new(4, 4);
    let mut surface = RoadSurfaceSystem::new(512.0);
    surface.compile_dirty(graph, &terrain);
    let mut demand = DemandSystem::new();
    demand.plan_hour_for_test(
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

// A planning snapshot with `workers` unemployed members, `sales` of unmet consumer demand and
// export expansion open.
fn planning_snapshot(workers: u32, sales: &[(&str, f32)]) -> DailyDemandSnapshot {
    let catalog = load_runtime_economy_catalog().unwrap();
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
    snapshot
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
    // The industrial slot commits the winner; the actions slot proves the hour's spawn again
    // on the driver's one supply (`SIM-02-I`). No rejected probe commits.
    assert_eq!(committed.borrow().as_slice(), &[viable.clone(), viable]);
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
                factory.operating_budget =
                    (costs.payroll + costs.overhead + costs.freight + 1.0) as f32;
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
            home.set_construction_hours_left(1);
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
        funding.see_operating_for_test(&allocator.buildings);
        let mut demand = DemandSystem::new();

        demand.upgrade_action_credit.industrial = 2.0;
        demand.plan_hour_for_test(
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
    factory.operating_budget = (costs.working_capital + 1.0) as f32;
    assert!(funded(&factory, 0.0));
    factory.operating_budget = (costs.payroll + costs.overhead + costs.freight + 1.0) as f32;
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
    let paid_inputs = costs.working_capital - f64::from(factory.operating_budget) + 1.0;
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
            let margin = f64::from(sales) - costs.total();
            assert!(margin > 0.0);
            policy.industrial_property_tax_per_building_per_day = (margin / 2.0) as f32;
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
                factory.operating_budget =
                    (costs.payroll + costs.overhead + costs.freight + 1.0) as f32;
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
            home.set_construction_hours_left(1);
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
        let mut funding = crate::simulation::economy::hiring::JobFunding::default();
        funding.see_operating_for_test(&allocator.buildings);
        let mut demand = DemandSystem::new();

        demand.downgrade_action_credit.industrial = 2.0;
        demand.plan_hour_for_test(
            &allocator,
            &graph,
            &empty_zoning(),
            &snapshot,
            &funding,
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
    // The two six-place homes offer applicants with 15 adults in 12 places, less the store
    // workers their shopping needs (`DEM-02-C`): four jobs leave room for one smaller home,
    // eight for none.
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
                plant.set_construction_hours_left(5);
                plant.construction_total_hours = 5;
                allocator.buildings.push(plant);
            }
        }
        allocator.seed_applicants_by_index_for_test();
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
        demand.plan_hour_for_test(
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
    demand.plan_hour_for_test(
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
    demand.plan_hour_for_test(
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
    use crate::simulation::economy::demand::workforce::resident_store_labor;
    use crate::simulation::economy::households::{
        applicant_places_and_members, local_utility_service_costs,
    };
    use crate::simulation::economy::operation::{OperatingCosts, OperationPlan};
    use std::hint::black_box;
    use std::time::Instant;

    // Workers each downgraded level-1 factory claims from ready arrivals.
    const CREW: u32 = 4;
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
            workers: CREW,
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
        industrial_property_tax_per_building_per_day: ((f64::from(sales) - costs.total()) / 2.0)
            as f32,
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
        // Each home keeps places 0..8 after retiring 8..16. Places count by their applicants'
        // adults; claims take the factories' crews plus the store labor of every applicant
        // (`DEM-02-C`) from the fewest-adult worker places first.
        let mut kept = [0_u32; 3];
        let mut members = 0;
        for idx in (0..pairs * 2).step_by(2) {
            let flat = allocator.flat_size_m2(idx);
            let (places, _) = applicant_places_and_members(flat, idx, 0..8);
            members += applicant_places_and_members(flat, idx, 0..16).1;
            for (kept, places) in kept.iter_mut().zip(places) {
                *kept += places;
            }
        }
        let [idle, singles, couples] = kept;
        let store = (f64::from(members) * resident_store_labor(&catalog)).ceil() as u32;
        let claimed = (pairs as u32 * CREW + store).min(singles + 2 * couples);
        let needed = if claimed <= singles {
            claimed
        } else {
            singles + (claimed - singles).div_ceil(2)
        };
        let ready_homes = idle + singles + couples - needed;
        let mut demand = DemandSystem::new();
        let mut plan_once = || {
            demand.building_actions = Default::default();
            demand.downgrade_action_credit.residential = pairs as f32;
            demand.downgrade_action_credit.industrial = pairs as f32;
            demand.plan_hour_for_test(
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
                ready_homes
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
fn prepared_demand_entrances_match_fresh_geometry_across_input_changes() {
    use crate::simulation::buildings::allocator::BuildingEntranceKey;
    use crate::simulation::network::TransitNetwork;

    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "retained_door", ZoneType::Industrial);
    let mut graph = graph_with_connected_border();
    let mut zoning = two_lots(&graph, ZoneType::Industrial);
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.rebuild_cch_and_check(&graph);
    let lots: Vec<_> = zoning.parcels().iter().map(|p| p.id().raw()).collect();
    let mut previous = None;
    for phase in 0..10 {
        let mut parcel = lots[0];
        match phase {
            2 => parcel = lots[1],
            3 => {
                let mut manifest = allocator.registry.get(&asset).unwrap().manifest.clone();
                manifest.anchors[0].position[0] += 3.0;
                allocator.registry.register("test", manifest, String::new());
            }
            4 => {
                allocator
                    .buildings
                    .push(building(ZoneType::Industrial, 0.0, 0, 1, asset.clone()));
                allocator.buildings[0].center_x = 31.0;
                allocator.buildings[0].center_y = 25.0;
                assert!(zoning.occupy_parcel(lots[0], 0));
            }
            5 => allocator.buildings[0].facing_dir = Vector2::new(1.0, 0.0),
            6 => network.lane_system.rebuild(&mut graph),
            7 => network.rebuild_cch_and_check(&graph),
            8 => {
                let mut manifest = allocator.registry.get(&asset).unwrap().manifest.clone();
                manifest.anchors.clear();
                allocator.registry.register("test", manifest, String::new());
            }
            _ => {}
        }
        let fresh = allocator
            .demand_site_entrance_reusing(
                parcel,
                &asset,
                &zoning,
                &graph,
                &network.lane_system,
                network.cch_graph.build_generation,
                None,
            )
            .unwrap();
        let reused = allocator
            .demand_site_entrance_reusing(
                parcel,
                &asset,
                &zoning,
                &graph,
                &network.lane_system,
                network.cch_graph.build_generation,
                previous.as_ref().map(|(key, entrance)| (*key, entrance)),
            )
            .unwrap();
        assert_eq!(fresh.0, reused.0);
        assert_eq!(
            BuildingEntranceKey::new(&fresh.1),
            BuildingEntranceKey::new(&reused.1),
            "phase {phase}"
        );
        if let Some((old_key, _)) = &previous {
            assert_eq!(*old_key == fresh.0, matches!(phase, 1 | 9), "phase {phase}");
        }
        // A marked value proves that matching keys actually take the reuse branch.
        let mut marked = fresh.1.clone();
        marked.entrance_s_m = -1234.0;
        let hit = allocator
            .demand_site_entrance_reusing(
                parcel,
                &asset,
                &zoning,
                &graph,
                &network.lane_system,
                network.cch_graph.build_generation,
                Some((fresh.0, &marked)),
            )
            .unwrap();
        assert_eq!(hit.1.entrance_s_m, -1234.0);
        previous = Some(fresh);
    }
    assert!(
        allocator
            .demand_site_entrance_reusing(
                u64::MAX,
                &asset,
                &zoning,
                &graph,
                &network.lane_system,
                network.cch_graph.build_generation,
                previous.as_ref().map(|(key, entrance)| (*key, entrance))
            )
            .is_none()
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
                None,
                |_| true,
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
            let supply = demand.workforce_reservations.available(duration);
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
            // Supply counts arrivals' adults net of their store labor (`DEM-02-C`), not places.
            let remaining = supply - pairs as u32 * profile.worker_capacity;
            assert_eq!(demand.workforce_reservations.available(duration), remaining);
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

// Runs planning slot `slot` of `demand` on a snapshot with `workers` unemployed members and
// `sales` of unmet consumer demand, as the hour's slots run it on the published census.
fn plan_slot(
    demand: &mut DemandSystem,
    slot: crate::simulation::economy::demand::PlanningSlot,
    world: (&BuildingAllocator, &RegionGraph, &ZoningSystem),
    workers: u32,
    sales: &[(&str, f32)],
) {
    let (allocator, graph, zoning) = world;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let terrain = TerrainSystem::new(4, 4);
    let mut surface = RoadSurfaceSystem::new(512.0);
    surface.compile_dirty(graph, &terrain);
    let fee = tuning.logistics.dispatch_fee_currency;
    demand.plan_private_building_actions(
        slot,
        allocator,
        graph,
        zoning,
        &planning_snapshot(workers, sales),
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
        &mut |_: u64, _: &str| Some(fee),
        None,
        None,
    );
}

// A later slot proves the earlier slots' business spawns and upgrades again (`SIM-02-I`). The
// commercial slot plans a barber on the free lot and the upgrade of the staffed one, with the
// town's eight workers; when the market they were proved on is gone by the industrial slot,
// both fail its re-proof: they leave the hour's plan, the upgrade's pacing credit is refunded,
// and the workers they held go to the industrial slot's factories, which the held claims left
// no room for while the market stood.
#[test]
fn failed_reproofs_leave_the_plan_and_free_their_workers_for_later_slots() {
    use crate::simulation::economy::demand::PlanningSlot;
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut allocator = BuildingAllocator::new();
    let barber = |allocator: &mut BuildingAllocator, name: &str, level| {
        register_family_asset_with_economy_profile(
            allocator,
            name,
            ZoneType::Commercial,
            Some("barber"),
            level,
            Some("personal_service_small"),
        )
    };
    let level_one = barber(&mut allocator, "barber_l1", 1);
    let level_two = barber(&mut allocator, "barber_l2", 2);
    register_family_asset_with_economy_profile(
        &mut allocator,
        "machinery",
        ZoneType::Industrial,
        None,
        1,
        Some("machinery_factory_basic"),
    );
    let graph = graph_with_connected_border();
    let mut zoning = two_lots(&graph, ZoneType::Commercial);
    let industrial = zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Industrial)
        .unwrap();
    zoning
        .place_parcel_run_at(10.0, 20.0, 40.0, 20.0, industrial, 20.0, 30.0, 0.0, &graph)
        .unwrap();
    // A fully staffed level-one barber with service stock on the first commercial lot.
    let lot = zoning.parcels()[0].id().raw();
    let mut shop = building(ZoneType::Commercial, 0.0, 0, 4, level_one);
    shop.parcel_id = lot;
    shop.operating_budget = 100_000.0;
    shop.economy_profile_runtime_id = catalog
        .profile_for_id("personal_service_small")
        .unwrap()
        .runtime_id;
    let services = catalog
        .resource_runtime_id_for_id("personal_services")
        .unwrap();
    shop.resource_inventory[services as usize - 1] = 10.0;
    allocator.buildings.push(shop);
    assert!(zoning.occupy_parcel(lot, 0));
    let sales = [("personal_services", 600.0)];
    let world = (&allocator, &graph, &zoning);
    let planned = || {
        let mut demand = DemandSystem::new();
        demand.upgrade_action_credit.commercial = 1.0;
        for slot in [PlanningSlot::Residential, PlanningSlot::Commercial] {
            plan_slot(&mut demand, slot, world, 8, &sales);
        }
        let plan = &demand.building_actions.commercial;
        assert_eq!(plan.spawns.len(), 1, "the commercial slot plans a barber");
        assert_ne!(plan.spawns[0].parcel_id, lot);
        assert_eq!(plan.upgrades.len(), 1, "and the staffed barber's upgrade");
        assert_eq!(plan.upgrades[0].target_asset_id, level_two);
        let credit = demand.upgrade_action_credit.commercial;
        assert!(credit < 1.0, "the upgrade spent its pacing credit");
        (demand, credit)
    };
    // While the market stands, both are proved again and keep their claims.
    let (mut kept, credit) = planned();
    plan_slot(&mut kept, PlanningSlot::Industrial, world, 8, &sales);
    assert_eq!(kept.reproof_drops, 0);
    assert_eq!(kept.building_actions.commercial.spawns.len(), 1);
    assert_eq!(kept.building_actions.commercial.upgrades.len(), 1);
    assert_eq!(kept.upgrade_action_credit.commercial, credit);
    assert!(
        kept.building_actions.industrial.spawns.is_empty(),
        "the commercial projects hold every worker"
    );
    // Without it, both leave the plan and the factories take the freed workers.
    let (mut dropped, credit) = planned();
    plan_slot(&mut dropped, PlanningSlot::Industrial, world, 8, &[]);
    assert_eq!(dropped.reproof_drops, 2);
    assert!(dropped.building_actions.commercial.spawns.is_empty());
    assert!(dropped.building_actions.commercial.upgrades.is_empty());
    assert_eq!(dropped.upgrade_action_credit.commercial, credit + 1.0);
    assert_eq!(dropped.building_actions.industrial.spawns.len(), 2);
}
