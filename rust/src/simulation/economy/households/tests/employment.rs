// SPDX-License-Identifier: GPL-2.0-only

//! Employment search, claims, wages, and founding assignment tests.

use super::support::*;
use super::*;

#[test]
#[ignore = "matched workplace cache publication/lookup timing; excludes setup"]
fn benchmark_workplace_route_cache_hashing() {
    use super::super::employment::WorkplaceRouteCache;
    use std::{collections::HashMap, hash::BuildHasher, hint::black_box, time::Instant};
    type Entry = ((usize, usize, bool), Option<u16>);
    fn sample<S: BuildHasher>(
        cache: &mut HashMap<(usize, usize, bool), Option<u16>, S>,
        entries: &[Entry],
    ) -> (f64, f64, u64) {
        cache.clear();
        let begin = Instant::now();
        for &(key, value) in entries {
            cache.insert(black_box(key), value);
        }
        let insert_ns = begin.elapsed().as_nanos() as f64 / entries.len() as f64;
        let begin = Instant::now();
        let mut checksum = 0;
        for _ in 0..8 {
            for &(key, _) in entries {
                let value = cache.get(black_box(&key)).unwrap();
                checksum += value.map_or(1, |value| u64::from(value) + 2);
            }
        }
        let lookup_ns = begin.elapsed().as_nanos() as f64 / (entries.len() * 8) as f64;
        for &(key, expected) in entries {
            assert_eq!(cache.get(&key), Some(&expected));
        }
        (insert_ns, lookup_ns, black_box(checksum))
    }
    for homes in [1000, 10_000] {
        let entries: Vec<_> = (0..homes)
            .flat_map(|home| {
                (0..16).map(move |work| {
                    let value = (home + work) % 7 != 0;
                    (
                        (home, work * 491, home % 2 == 0),
                        value.then_some(((home * 5 + work) % 65536) as u16),
                    )
                })
            })
            .collect();
        let mut ordinary = HashMap::with_capacity(entries.len());
        let mut fast = WorkplaceRouteCache::default();
        fast.reserve(entries.len());
        let ordinary_capacity = ordinary.capacity();
        let fast_capacity = fast.capacity();
        let expected: u64 = entries
            .iter()
            .map(|&(_, value)| value.map_or(1, |value| u64::from(value) + 2))
            .sum::<u64>()
            * 8;
        for pair in 0..11 {
            for fixed in if pair % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let (insert_ns, lookup_ns, checksum) = if fixed {
                    sample(&mut fast, &entries)
                } else {
                    sample(&mut ordinary, &entries)
                };
                assert_eq!(checksum, expected);
                assert_eq!(ordinary.capacity(), ordinary_capacity);
                assert_eq!(fast.capacity(), fast_capacity);
                println!(
                    "WORKPLACE_HASH {{\"entries\":{},\"fixed\":{fixed},\"pair\":{pair},\"insert_ns\":{insert_ns},\"lookup_ns\":{lookup_ns}}}",
                    entries.len()
                );
            }
        }
    }
}

#[test]
fn failed_expansion_retains_the_input_backing_of_a_local_funded_operation() {
    use crate::simulation::economy::definitions::compile_test_catalog;
    use crate::simulation::economy::operation::{OperatingCosts, working_capital_assets_at_prices};
    let authored = include_str!("../../../../../../economy/profiles.toml");
    let start = authored
        .find("[[profiles]]\nid = \"grain_farm_basic\"")
        .unwrap();
    let end = start + authored[start + 1..].find("\n[[profiles]]").unwrap() + 1;
    let cheap = authored[start..end]
        .replace("grain_farm_basic", "cheap_grain")
        .replace(
            "wage_min_currency_per_day = 80.0",
            "wage_min_currency_per_day = 1.0",
        )
        .replace(
            "wage_max_currency_per_day = 100.0",
            "wage_max_currency_per_day = 1.0",
        )
        .replace("unit_price_currency = 4.8", "unit_price_currency = 0.2")
        .replace(
            "idle_kwh_per_day = 2.0, operating_kwh_per_day = 3.0",
            "idle_kwh_per_day = 0.0, operating_kwh_per_day = 0.0",
        )
        .replace(
            "area_utility_units_per_hectare = [0.555556, 1.0, 0.0]",
            "area_utility_units_per_hectare = [0.0, 0.0, 0.0]",
        )
        .replace(
            "[[profiles.inputs]]\nresource = \"machinery\"\nunits_per_day = 0.1",
            "",
        );
    let start = authored
        .find("[[profiles]]\nid = \"food_processor_basic\"")
        .unwrap();
    let end = start + authored[start + 1..].find("\n[[profiles]]").unwrap() + 1;
    let recipe = authored[start..end].replace(
        "[[profiles.inputs]]\nresource = \"machinery\"\nunits_per_day = 2.0",
        "",
    );
    let premium = recipe
        .replace("food_processor_basic", "premium_processor")
        .replace("unit_price_currency = 12.0", "unit_price_currency = 30.0")
        .replace("\"packaged_food\"", "\"premium_food\"");
    let ordinary = recipe.replace("food_processor_basic", "ordinary_processor");
    let resource = "[[resources]]\nid = \"premium_food\"\nquantity_unit = \"kg\"\nshipment_batch_kg = 50.0\nruntime_id = 10\nimport_unit_price_currency = 30.0\n";
    let catalog = compile_test_catalog(&format!(
        "{authored}\n{resource}\n{cheap}\n{premium}\n{ordinary}"
    ))
    .unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 2.0;
    tuning.owa_export_price_multiplier = 1.0;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let farm_asset =
        register_test_utility_asset(&mut allocator, "test", "fallback_grain", "grain_farm_basic");
    let factory_asset = register_test_asset(
        &mut allocator,
        "test",
        "fallback_food",
        ZoneClass::Industrial,
    );
    for (id, x) in [
        ("cheap_grain", 20.0),
        ("grain_farm_basic", 10.0),
        ("premium_processor", 40.0),
        ("ordinary_processor", 60.0),
    ] {
        let farm = id.ends_with("grain") || id == "grain_farm_basic";
        let profile = catalog.profile_for_id(id).unwrap();
        let mut building = make_building(
            x,
            if farm {
                ZoneType::None
            } else {
                ZoneType::Industrial
            },
            if farm { &farm_asset } else { &factory_asset },
            0.0,
        );
        building.economy_profile_runtime_id = profile.runtime_id;
        building.work_area_scale = if farm { 5.0 } else { 1.0 };
        building.worker_count = if farm {
            physical_worker_capacity_for_profile(&building, profile)
        } else {
            5
        };
        building.operating_budget = if farm { 100_000.0 } else { 0.0 };
        allocator.buildings.push(building);
    }
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    allocator.buildings[2].set_inventory_units(grain, 900.0);
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    let mut carriers = [None; 4];
    logistics.write_border_carrier_quotes(
        &allocator,
        &network,
        &graph,
        &[0],
        &catalog,
        &tuning,
        &mut carriers,
    );
    let utilities = local_utility_service_costs(&catalog)
        .map(|price| price * tuning.owa_import_price_multiplier);
    let mut supported = 0;
    // The first processor can fund imports but reject cheap inventory repricing during
    // expansion. The second processor's current operation needs the other local supplier.
    // Released cheap supply must not turn its rejected expansion into unbacked import jobs.
    for cash in (400..=600).step_by(25) {
        allocator.buildings[2].operating_budget = cash as f32;
        for stock in (500..=800).step_by(5) {
            allocator.buildings[3].set_inventory_units(grain, stock as f32);
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
            let building = &allocator.buildings[3];
            let profile = catalog.profile_for_id("ordinary_processor").unwrap();
            let Some(operation) =
                funding.funded_operation(3, building, profile, utilities, &tuning, &policy, 420)
            else {
                continue;
            };
            supported += 1;
            let mut input_prices = Vec::new();
            let costs = OperatingCosts::for_plan_with_input_quotes(
                profile,
                operation.plan,
                operation.prices,
                |_, units| units,
                |input| {
                    let price = catalog.unit_price_for_resource(input.resource).unwrap()
                        * operation.prices.imports;
                    let quote =
                        funding
                            .input_supply
                            .owner_input_costs(3, input, price, carriers[3]);
                    input_prices.push((input.resource, quote.goods / f64::from(input.daily_units)));
                    quote
                },
            );
            let capital = working_capital_assets_at_prices(
                building,
                profile,
                0.0,
                |resource| {
                    input_prices
                        .iter()
                        .find(|&&(id, _)| id == resource)
                        .unwrap()
                        .1
                },
                |resource| f64::from(catalog.unit_price_for_resource(resource).unwrap()),
            );
            let revenue = profile
                .outputs
                .iter()
                .map(|output| {
                    f64::from(
                        profile.net_output_units_per_day(output)
                            * operation.plan.area
                            * operation.plan.activity,
                    ) * f64::from(
                        catalog
                            .unit_price_for_resource(output.resource_runtime_id)
                            .unwrap(),
                    )
                })
                .sum::<f64>();
            assert!(
                revenue > costs.total() && capital >= costs.working_capital,
                "cash={cash} stock={stock} workers={} activity={} revenue={revenue} costs={costs:?} capital={capital} capacities={:?}",
                operation.plan.workers,
                operation.plan.activity,
                funding.capacities
            );
        }
    }
    assert!(
        supported > 0,
        "the fixture must retain locally financed operations"
    );
}

#[test]
fn local_stock_repricing_preserves_the_funded_import_alternative() {
    use crate::simulation::economy::operation::{
        InputRequirement, OperatingCosts, OperationPlan, OperationPrices,
        working_capital_assets_at_prices,
    };
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 1.2;
    tuning.owa_export_price_multiplier = 1.0;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let farm_asset =
        register_test_utility_asset(&mut allocator, "test", "stock_grain", "grain_farm_basic");
    let factory_asset =
        register_test_asset(&mut allocator, "test", "stock_food", ZoneClass::Industrial);
    let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
    let factory_profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let mut farm = make_building(20.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 10.0;
    farm.operating_budget = 100_000.0;
    let farm_workers = physical_worker_capacity_for_profile(&farm, farm_profile);
    allocator.buildings.push(farm);
    let mut factory = make_building(40.0, ZoneType::Industrial, &factory_asset, 0.0);
    factory.operating_budget = 0.0;
    factory.worker_count = factory_profile.worker_capacity;
    allocator.buildings.push(factory);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    let mut carriers = [None; 2];
    logistics.write_border_carrier_quotes(
        &allocator,
        &network,
        &graph,
        &[0],
        &catalog,
        &tuning,
        &mut carriers,
    );
    let mut retained_import = false;
    // Sweep the narrow capital boundary: excess grain finances payroll, so cheaper
    // replacement prices can lower available assets more than opening requirements.
    for stock in (800..=1_400).step_by(5) {
        allocator.buildings[1].set_inventory_units(grain, stock as f32);
        allocator.buildings[0].worker_count = 0;
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
        let imported = funding.capacities[1];
        allocator.buildings[0].worker_count = farm_workers;
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
        assert!(
            funding.capacities[1] >= imported,
            "optional local supply erased funded jobs at {stock} grain units"
        );
        if imported == factory_profile.worker_capacity {
            let mut local_units = 0.0;
            funding
                .input_supply
                .visit_dependents(0, |buyer, resource, units| {
                    if buyer == 1 && resource == grain {
                        local_units += units;
                    }
                });
            retained_import |= local_units < 200.0;
            // A financed partial substitution may use part of the released source. Verify
            // its actual mixed quote independently, including repricing retained inventory.
            let prices = OperationPrices {
                utilities: local_utility_service_costs(&catalog)
                    .map(|price| price * tuning.owa_import_price_multiplier),
                property_tax: crate::simulation::economy::fiscal::daily_property_tax(
                    ZoneType::Industrial,
                    1,
                    &policy,
                ),
                imports: tuning.owa_import_price_multiplier,
                carrier_quote: carriers[1].unwrap(),
                truck_payload: tuning.logistics.truck_payload_kg,
            };
            let mut input_prices = Vec::new();
            let costs = OperatingCosts::for_plan_with_input_quotes(
                factory_profile,
                OperationPlan {
                    workers: factory_profile.worker_capacity,
                    activity: 1.0,
                    area: 1.0,
                },
                prices,
                |_, units| units,
                |input| {
                    let costs = funding.input_supply.owner_input_costs(
                        1,
                        input,
                        catalog
                            .unit_price_for_resource(input.resource)
                            .unwrap_or(0.0)
                            * prices.imports,
                        carriers[1],
                    );
                    input_prices.push((input.resource, costs.goods / f64::from(input.daily_units)));
                    costs
                },
            );
            let capital = working_capital_assets_at_prices(
                &allocator.buildings[1],
                factory_profile,
                0.0,
                |resource| {
                    input_prices
                        .iter()
                        .find(|&&(id, _)| id == resource)
                        .unwrap()
                        .1
                },
                |resource| f64::from(catalog.unit_price_for_resource(resource).unwrap_or(0.0)),
            );
            assert!(
                capital >= costs.working_capital,
                "stock={stock} local={local_units}: capital={capital}, required={}",
                costs.working_capital
            );
            funding.input_supply.release_owner(1, &allocator, &graph);
            let quote = logistics.quote_recurring_input(
                &mut funding.input_supply,
                InputRequirement {
                    resource: grain,
                    daily_units: 200.0,
                    buffer_units: 800.0,
                    replenishment_batch_units: Some(400.0),
                },
                Some(1),
                &allocator.entrances[1],
                None,
                &allocator,
                &catalog,
                &tuning,
                &network,
                &graph,
                1.0,
            );
            assert!(
                quote.goods.is_finite(),
                "every new local receipt must remain owned and releasable"
            );
            funding.input_supply.reject(&allocator, &graph);
        }
    }
    assert!(
        retained_import,
        "a rejected full local substitution must preserve a funded import remainder"
    );
}

#[test]
fn unfinished_service_keeps_fiscal_ceiling_without_advertising_live_jobs() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (graph, mut network) = simple_work_graph();
    let mut allocator = BuildingAllocator::new();
    let asset =
        register_test_utility_asset(&mut allocator, "test", "pending_power", "power_plant_basic");
    let profile = catalog.profile_for_id("power_plant_basic").unwrap();
    let mut plant = make_building(40.0, ZoneType::None, &asset, 0.0);
    plant.economy_profile_runtime_id = profile.runtime_id;
    plant.set_construction_hours_left(24);
    plant.construction_total_hours = 24;
    allocator.buildings.push(plant);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    for fraction in [0.0, 0.5, 1.0] {
        funding.refresh(
            &allocator,
            &[],
            &mut logistics,
            &network,
            &graph,
            &catalog,
            &tuning,
            &policy,
            &[fraction],
            100_000.0,
            420,
        );
        assert_eq!(funding.capacities, [0]);
        assert_eq!(
            funding.service_capacities,
            [service_funded_worker_capacity(
                profile.worker_capacity,
                profile,
                0,
                &[fraction],
            )]
        );
    }
    allocator.buildings[0].construction_complete_slot = 0;
    funding.refresh(
        &allocator,
        &[],
        &mut logistics,
        &network,
        &graph,
        &catalog,
        &tuning,
        &policy,
        &[0.5],
        100_000.0,
        420,
    );
    assert_eq!(funding.capacities, funding.service_capacities);
    assert!(funding.capacities[0] > 0);
    let mut borders = Vec::new();
    crate::simulation::economy::logistics::collect_connected_border_nodes(&graph, &mut borders);
    // The census's owner stamps it with the live city's key (`SIM-02-I`).
    use crate::simulation::economy::hiring::{CensusKey, StructureRevisions};
    funding.published = Some(CensusKey::live(Default::default(), 0, &network));
    let key = |network: &TransitNetwork| CensusKey::live(Default::default(), 0, network);
    let shifted = |buildings, households| {
        CensusKey::live(
            StructureRevisions {
                buildings,
                households,
            },
            0,
            &network,
        )
    };
    assert!(funding.has_current_input_supply(key(&network), &borders, 420));
    assert!(
        !funding.has_current_input_supply(
            key(&network),
            &borders,
            crate::simulation::economy::logistics::freight_price_phase(&tuning, 480)
        ),
        "a new receiving-price interval cannot reuse earlier profitability"
    );
    assert!(
        !funding.has_current_input_supply(shifted(1, 0), &borders, 420),
        "a building index shift invalidates the books"
    );
    assert!(
        funding.has_current_input_supply(shifted(0, 1), &borders, 420),
        "a departed household stays in the books until the next census"
    );
    network.frontage_delay_revision += 1;
    assert!(
        !funding.has_current_input_supply(key(&network), &borders, 420),
        "unchanged road geometry cannot validate stale recurring freight prices"
    );
}

#[test]
fn fiscal_service_inputs_share_live_supply_without_turning_payroll_into_private_sales() {
    use crate::simulation::economy::operation::InputRequirement;

    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    for reachable in [true, false] {
        let mut allocator = BuildingAllocator::new();
        let source_asset = register_test_utility_asset(
            &mut allocator,
            "test",
            "service_machinery",
            "machinery_factory_basic",
        );
        let service_asset = register_test_utility_asset(
            &mut allocator,
            "test",
            "service_water",
            "water_plant_basic",
        );
        let source_profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
        let service_profile = catalog.profile_for_id("water_plant_basic").unwrap();
        let mut source = make_building(20.0, ZoneType::Industrial, &source_asset, 0.0);
        source.economy_profile_runtime_id = source_profile.runtime_id;
        source.worker_count = source_profile.worker_capacity;
        source.operating_budget = 100_000.0;
        allocator.buildings.push(source);
        let mut service = make_building(40.0, ZoneType::None, &service_asset, 0.0);
        service.economy_profile_runtime_id = service_profile.runtime_id;
        service.worker_count = service_profile.worker_capacity;
        service.operating_budget = 0.0;
        allocator.buildings.push(service);
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let proposed_door = allocator.entrances[1].clone();
        if !reachable {
            allocator.entrances[1].car_lane_fwd = usize::MAX;
            allocator.entrances[1].car_lane_bkw = usize::MAX;
        }
        let mut households = HouseholdSystem::new();
        let mut logistics = ShipmentSystem::new();
        households.job_funding.refresh(
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
            households.job_funding.capacities,
            [4, 3],
            "public payroll follows fiscal funding, not a private-sales profitability test"
        );
        let resource = catalog.resource_runtime_id_for_id("machinery").unwrap();
        for (units, expected) in [(39.5, true), (40.0, !reachable)] {
            let book = &mut households.job_funding.input_supply;
            let costs = logistics.quote_recurring_input(
                book,
                InputRequirement {
                    resource,
                    daily_units: units,
                    buffer_units: units * 2.0,
                    replenishment_batch_units: Some(10.0),
                },
                None,
                &proposed_door,
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
                "only a reachable water plant reserves its 0.5 daily machinery input"
            );
            book.reject(&allocator, &graph);
        }
        assert_eq!(allocator.buildings[0].operating_budget, 100_000.0);
        assert_eq!(allocator.buildings[1].operating_budget, 0.0);
    }
}

#[test]
fn household_recount_follows_job_assignments() {
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(0, 3, 1_000.0, 5.0));
    households
        .households
        .push(make_household(usize::MAX, 1, 1_000.0, 5.0));
    for hid in [0, 0, 0, 1] {
        let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.assign_household_id(agent, hid);
    }
    let allocator = BuildingAllocator::new();
    agents.assign_work_building(0, 1, 0);
    agents.assign_work_building(3, 1, 0);
    households.recount_every_household(&agents, &allocator);
    assert_eq!(households.households[0].employed_adults, 1);
    assert_eq!(households.households[1].employed_adults, 1);
    agents.assign_work_building(0, usize::MAX, 0);
    agents.assign_work_building(1, 2, 0);
    agents.assign_work_building(2, 2, 0);
    households.recount_every_household(&agents, &allocator);
    assert_eq!(households.households[0].employed_adults, 2);
}

#[test]
fn live_hiring_shares_finite_local_grain_and_releases_rejected_capital_claims() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 2.0;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let farm_asset =
        register_test_utility_asset(&mut allocator, "test", "finite_grain", "grain_farm_basic");
    let factory_asset =
        register_test_asset(&mut allocator, "test", "finite_food", ZoneClass::Industrial);
    let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
    let factory_profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let mut farm = make_building(20.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 10.0;
    farm.operating_budget = 100_000.0;
    farm.worker_count = physical_worker_capacity_for_profile(&farm, farm_profile);
    allocator.buildings.push(farm);
    for x in [40.0, 60.0] {
        let mut factory = make_building(x, ZoneType::Industrial, &factory_asset, 0.0);
        factory.operating_budget = 100_000.0;
        allocator.buildings.push(factory);
    }
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    let refresh = |households: &mut HouseholdSystem,
                   logistics: &mut ShipmentSystem,
                   allocator: &BuildingAllocator| {
        households.job_funding.refresh(
            allocator,
            &[],
            logistics,
            &network,
            &graph,
            &catalog,
            &tuning,
            &policy,
            &[],
            100_000.0,
            420,
        );
    };
    refresh(&mut households, &mut logistics, &allocator);
    assert_eq!(
        households.job_funding.capacities[1],
        factory_profile.worker_capacity
    );
    assert_eq!(
        households.job_funding.capacities[2], 0,
        "a second factory cannot reuse the farm's 200 daily grain units"
    );
    allocator.buildings[1].operating_budget = 0.0;
    refresh(&mut households, &mut logistics, &allocator);
    assert_eq!(households.job_funding.capacities[1], 0);
    assert_eq!(
        households.job_funding.capacities[2], factory_profile.worker_capacity,
        "capital rejection must release every staged local input claim"
    );
    assert_eq!(allocator.buildings[2].operating_budget, 100_000.0);
    allocator.buildings[0].worker_count = 0;
    allocator.buildings[0]
        .set_inventory_units(farm_profile.outputs[0].resource_runtime_id, 1_000_000.0);
    refresh(&mut households, &mut logistics, &allocator);
    assert_eq!(
        households.job_funding.capacities[2], 0,
        "stock without active production cannot promise recurring staffing"
    );
}

#[test]
fn locally_funded_staffed_processor_supplies_the_next_funded_shop() {
    for source_before_shop in [true, false] {
        let catalog = load_runtime_economy_catalog().unwrap();
        let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
        tuning.owa_import_price_multiplier = 2.0;
        let policy =
            crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
        let (mut graph, network) = simple_work_graph();
        graph.set_node_type(0, NodeType::Border);
        let mut allocator = BuildingAllocator::new();
        let farm_asset =
            register_test_utility_asset(&mut allocator, "test", "chain_grain", "grain_farm_basic");
        let factory_asset =
            register_test_asset(&mut allocator, "test", "chain_food", ZoneClass::Industrial);
        let shop_asset =
            register_test_asset(&mut allocator, "test", "chain_shop", ZoneClass::Commercial);
        let home_asset =
            register_test_asset(&mut allocator, "test", "chain_home", ZoneClass::Residential);
        let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
        let factory_profile = catalog.profile_for_id("food_processor_basic").unwrap();
        let shop_profile = catalog.profile_for_id("grocery_basic").unwrap();
        let mut farm = make_building(20.0, ZoneType::None, &farm_asset, 0.0);
        farm.economy_profile_runtime_id = farm_profile.runtime_id;
        farm.work_area_scale = 10.0;
        farm.worker_count = physical_worker_capacity_for_profile(&farm, farm_profile);
        farm.operating_budget = 100_000.0;
        allocator.buildings.push(farm);
        for (x, zone, asset, workers) in [
            (
                40.0,
                ZoneType::Industrial,
                &factory_asset,
                factory_profile.worker_capacity,
            ),
            (
                60.0,
                ZoneType::Commercial,
                &shop_asset,
                shop_profile.worker_capacity,
            ),
        ] {
            let mut building = make_building(x, zone, asset, 0.0);
            building.worker_count = workers;
            building.operating_budget = 100_000.0;
            allocator.buildings.push(building);
        }
        allocator
            .buildings
            .push(make_building(80.0, ZoneType::Residential, &home_asset, 0.0));
        let (factory, shop) = if source_before_shop {
            (1, 2)
        } else {
            allocator.buildings.swap(1, 2);
            (2, 1)
        };
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let mut households = HouseholdSystem::new();
        let mut customer = make_household(3, 200, 100.0, 1.0);
        customer.gross_daily_wages = 100_000.0;
        households.households.push(customer);
        let mut logistics = ShipmentSystem::new();
        let refresh = |households: &mut HouseholdSystem,
                       logistics: &mut ShipmentSystem,
                       allocator: &BuildingAllocator| {
            households.job_funding.refresh(
                allocator,
                &households.households,
                logistics,
                &network,
                &graph,
                &catalog,
                &tuning,
                &policy,
                &[],
                100_000.0,
                420,
            );
        };
        refresh(&mut households, &mut logistics, &allocator);
        assert_eq!(
            households.job_funding.capacities[factory],
            factory_profile.worker_capacity
        );
        assert_eq!(
            households.job_funding.capacities[shop], shop_profile.worker_capacity,
            "the locally funded processor's staffed production must reach the shop"
        );
        assert!(
            !households
                .job_funding
                .input_supply
                .offers_inputs(shop_profile),
            "the shop must reserve the processor's entire finite output"
        );
        allocator.buildings[factory].worker_count = 0;
        allocator.buildings[factory]
            .set_inventory_units(factory_profile.outputs[0].resource_runtime_id, 1_000_000.0);
        refresh(&mut households, &mut logistics, &allocator);
        assert_eq!(
            households.job_funding.capacities[factory],
            factory_profile.worker_capacity
        );
        assert_eq!(
            households.job_funding.capacities[shop], 0,
            "unfilled funded jobs and existing stock cannot publish recurring production"
        );
        allocator.buildings[factory].worker_count = factory_profile.worker_capacity;
        allocator.buildings[factory]
            .set_inventory_units(factory_profile.outputs[0].resource_runtime_id, 0.0);
        allocator.buildings[factory].operating_budget = 0.0;
        refresh(&mut households, &mut logistics, &allocator);
        assert_eq!(households.job_funding.capacities[factory], 0);
        assert_eq!(
            households.job_funding.capacities[shop], 0,
            "a rejected producer cannot publish stock or leave a funded downstream operation"
        );
        allocator.buildings[factory].operating_budget = 100_000.0;
        refresh(&mut households, &mut logistics, &allocator);
        assert_eq!(
            households.job_funding.capacities[shop],
            shop_profile.worker_capacity
        );
    }
}

#[test]
fn staffed_buyer_without_financing_cannot_fund_its_supplier() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_export_price_multiplier = 0.0;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let farm_asset =
        register_test_utility_asset(&mut allocator, "test", "unpaid_grain", "grain_farm_basic");
    let factory_asset =
        register_test_asset(&mut allocator, "test", "unpaid_food", ZoneClass::Industrial);
    let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
    let factory_profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let mut farm = make_building(20.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 10.0;
    farm.operating_budget = 100_000.0;
    farm.worker_count = physical_worker_capacity_for_profile(&farm, farm_profile);
    let mut factory = make_building(40.0, ZoneType::Industrial, &factory_asset, 0.0);
    factory.worker_count = factory_profile.worker_capacity;
    factory.operating_budget = 0.0;
    allocator.buildings.extend([farm, factory]);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    funding.refresh(
        &allocator,
        &[],
        &mut ShipmentSystem::new(),
        &network,
        &graph,
        &catalog,
        &tuning,
        &policy,
        &[],
        100_000.0,
        420,
    );
    assert_eq!(funding.capacities[1], 0);
    assert_eq!(
        funding.capacities[0], 0,
        "raw staffed grain requirements are not financed purchases"
    );
}

#[test]
fn household_sales_withdraw_staffed_output_from_business_input_offers() {
    use crate::simulation::economy::operation::InputRequirement;

    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let shop_asset = register_test_asset(
        &mut allocator,
        "test",
        "shared_output_shop",
        ZoneClass::Commercial,
    );
    let home_asset = register_test_asset(
        &mut allocator,
        "test",
        "shared_output_home",
        ZoneClass::Residential,
    );
    let profile = catalog.profile_for_id("grocery_basic").unwrap();
    let resource = profile.outputs[0].resource_runtime_id;
    let mut shop = make_building(20.0, ZoneType::Commercial, &shop_asset, 0.0);
    shop.operating_budget = 100_000.0;
    allocator.buildings.push(shop);
    allocator
        .buildings
        .push(make_building(40.0, ZoneType::Residential, &home_asset, 0.0));
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut household = make_household(1, 200, 100_000.0, 1.0);
    household.gross_daily_wages = 100_000.0;
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    for workers in [profile.worker_capacity, 5, 0, profile.worker_capacity] {
        allocator.buildings[0].worker_count = workers;
        funding.refresh(
            &allocator,
            std::slice::from_ref(&household),
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
        assert_eq!(funding.capacities[0], profile.worker_capacity);
        for _ in 0..2 {
            let quote = logistics.quote_recurring_input(
                &mut funding.input_supply,
                InputRequirement {
                    resource,
                    daily_units: 1.0,
                    buffer_units: 4.0,
                    replenishment_batch_units: Some(40.0),
                },
                None,
                &allocator.entrances[1],
                None,
                &allocator,
                &catalog,
                &tuning,
                &network,
                &graph,
                1.0,
            );
            assert!(
                quote.goods.is_infinite(),
                "{workers} staffed workers cannot promise their household output to a business too"
            );
            funding.input_supply.reject(&allocator, &graph);
        }
    }
}

#[test]
fn household_and_business_customers_share_output_and_exact_recipe_activity() {
    use crate::simulation::economy::definitions::compile_test_catalog;
    let authored = include_str!("../../../../../../economy/profiles.toml");
    let first = authored
        .find("[[profiles]]\nid = \"food_processor_basic\"")
        .unwrap();
    let end = first + authored[first + 1..].find("\n[[profiles]]").unwrap() + 1;
    let recipe = authored[first..end]
        .replace("food_processor_basic", "mixed_customer_processor")
        .replace(
            "[[profiles.inputs]]\nresource = \"grain\"\nunits_per_day = 200.0",
            "[[profiles.inputs]]\nresource = \"household_supplies\"\nunits_per_day = 20.0",
        );
    // The extra outside feed preserves the main recipe's mass balance while allowing a
    // business customer to buy the same resource that households consume.
    let feed = "[[resources]]\nid = \"mixed_feed\"\nquantity_unit = \"kg\"\nshipment_batch_kg = 10.0\nruntime_id = 10\nimport_unit_price_currency = 0.01\n";
    let catalog = compile_test_catalog(&format!("{authored}\n{feed}\n{recipe}\n[[profiles.inputs]]\nresource = \"mixed_feed\"\nunits_per_day = 180.0\n")).unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let resource = catalog
        .resource_runtime_id_for_id("household_supplies")
        .unwrap();
    let shop_profile = catalog.profile_for_id("grocery_basic").unwrap();
    let factory_profile = catalog.profile_for_id("mixed_customer_processor").unwrap();
    for shop_first in [true, false] {
        let mut allocator = BuildingAllocator::new();
        let home_asset =
            register_test_asset(&mut allocator, "test", "mixed_home", ZoneClass::Residential);
        let shop_asset =
            register_test_asset(&mut allocator, "test", "mixed_shop", ZoneClass::Commercial);
        let factory_asset = register_test_asset(
            &mut allocator,
            "test",
            "mixed_factory",
            ZoneClass::Industrial,
        );
        allocator
            .buildings
            .push(make_building(20.0, ZoneType::Residential, &home_asset, 0.0));
        let shop = if shop_first { 1 } else { 2 };
        let factory = 3 - shop;
        for index in 1..=2 {
            let is_shop = index == shop;
            let profile = if is_shop {
                shop_profile
            } else {
                factory_profile
            };
            let mut building = make_building(
                20.0 + index as f32 * 20.0,
                if is_shop {
                    ZoneType::Commercial
                } else {
                    ZoneType::Industrial
                },
                if is_shop { &shop_asset } else { &factory_asset },
                0.0,
            );
            building.economy_profile_runtime_id = profile.runtime_id;
            building.worker_count = profile.worker_capacity;
            building.operating_budget = 100_000.0;
            allocator.buildings.push(building);
        }
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let mut family = make_household(0, 180, 0.0, 1.0);
        family.gross_daily_wages = 100_000.0;
        let mut funding = crate::simulation::economy::hiring::JobFunding::default();
        let mut logistics = ShipmentSystem::new();
        for financed in [true, false, true] {
            allocator.buildings[factory].operating_budget = if financed { 100_000.0 } else { 0.0 };
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
            assert_eq!(funding.capacities[factory], if financed { 10 } else { 0 });
            assert_eq!(
                funding.capacities[shop],
                if financed { 15 } else { 14 },
                "180 household units and 20 financed business units share one 200-unit operation"
            );
            assert_eq!(
                funding.input_demand_scale(shop, &allocator.buildings[shop], shop_profile, false),
                if financed { 1.0 } else { 0.9 },
                "rounded jobs must not overstate recipe activity"
            );
            let mut deliveries = 0.0;
            funding
                .input_supply
                .visit_dependents(shop, |buyer, output, units| {
                    if buyer == factory && output == resource {
                        deliveries += units;
                    }
                });
            assert_eq!(deliveries, if financed { 20.0 } else { 0.0 });
        }
    }
}

#[test]
fn withdrawn_business_sales_return_household_customers_to_a_viable_competitor() {
    use crate::simulation::economy::definitions::compile_test_catalog;
    let authored = include_str!("../../../../../../economy/profiles.toml");
    let first = authored
        .find("[[profiles]]\nid = \"grocery_basic\"")
        .unwrap();
    let end = first + authored[first + 1..].find("\n[[profiles]]").unwrap() + 1;
    // This shop needs both output markets to cover its unchanged 200-unit input recipe.
    // A staffed but cashless processor supplies only an unproven demand for its grain.
    let recipe = authored[first..end].replace("grocery_basic", "conditional_shop")
        .replace("resource = \"household_supplies\"\nunits_per_day = 200.0",
            "resource = \"household_supplies\"\nunits_per_day = 100.0\n\n[[profiles.outputs]]\nresource = \"grain\"\nunits_per_day = 100.0");
    let catalog = compile_test_catalog(&format!("{authored}\n{recipe}")).unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(
        &mut allocator,
        "test",
        "returned_home",
        ZoneClass::Residential,
    );
    let shop_asset = register_test_asset(
        &mut allocator,
        "test",
        "returned_shop",
        ZoneClass::Commercial,
    );
    let factory_asset = register_test_asset(
        &mut allocator,
        "test",
        "returned_factory",
        ZoneClass::Industrial,
    );
    allocator
        .buildings
        .push(make_building(20.0, ZoneType::Residential, &home_asset, 0.0));
    for (index, id) in ["conditional_shop", "grocery_basic", "food_processor_basic"]
        .into_iter()
        .enumerate()
    {
        let industrial = index == 2;
        let profile = catalog.profile_for_id(id).unwrap();
        let mut building = make_building(
            40.0 + index as f32 * 20.0,
            if industrial {
                ZoneType::Industrial
            } else {
                ZoneType::Commercial
            },
            if industrial {
                &factory_asset
            } else {
                &shop_asset
            },
            0.0,
        );
        building.economy_profile_runtime_id = profile.runtime_id;
        building.worker_count = profile.worker_capacity;
        building.operating_budget = if industrial { 0.0 } else { 100_000.0 };
        allocator.buildings.push(building);
    }
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut family = make_household(0, 100, 0.0, 1.0);
    family.gross_daily_wages = 100_000.0;
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    let mut rebuilt = false;
    for members in [100, 150, 250] {
        family.member_count = members;
        for conditional_cash in [0.0, 100_000.0, 0.0, 100_000.0] {
            allocator.buildings[1].operating_budget = conditional_cash;
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
            rebuilt |= funding.household_quote_replay_counts().1 > 0;
            assert_eq!(
                funding.capacities[1], 0,
                "households alone cannot finance both outputs"
            );
            assert_eq!(funding.capacities[3], 0);
            let activity = (f32::from(members) / 200.0).min(1.0);
            assert_eq!(
                funding.capacities[2],
                (15.0 * activity).ceil() as u32,
                "withdrawn conditional sales must return household units in this census: {members} members"
            );
            assert_eq!(
                funding.input_demand_scale(
                    2,
                    &allocator.buildings[2],
                    catalog.profile_for_id("grocery_basic").unwrap(),
                    false
                ),
                activity
            );
        }
    }
    assert!(
        rebuilt,
        "withdrawing a conditional seller must reconstruct the changed household prefix"
    );
}

#[test]
fn financed_business_sales_withdraw_output_from_later_project_inputs() {
    use crate::simulation::economy::operation::InputRequirement;

    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    // The processor initially quotes the cheaper outside input. Its proven margin can
    // also support the farm's more expensive local delivery when customers are matched.
    tuning.owa_import_price_multiplier = 0.5;
    tuning.owa_export_price_multiplier = 1.0;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let farm_asset =
        register_test_utility_asset(&mut allocator, "test", "promised_grain", "grain_farm_basic");
    let factory_asset = register_test_asset(
        &mut allocator,
        "test",
        "promised_food",
        ZoneClass::Industrial,
    );
    let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
    let factory_profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let mut farm = make_building(20.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 10.0;
    farm.operating_budget = 100_000.0;
    farm.worker_count = physical_worker_capacity_for_profile(&farm, farm_profile);
    let mut factory = make_building(40.0, ZoneType::Industrial, &factory_asset, 0.0);
    factory.worker_count = factory_profile.worker_capacity;
    factory.operating_budget = 100_000.0;
    allocator.buildings.extend([farm, factory]);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
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
    assert_eq!(funding.capacities[0], allocator.buildings[0].worker_count);
    assert_eq!(funding.capacities[1], factory_profile.worker_capacity);
    let mut fork = crate::simulation::economy::logistics::RecurringSupply::default();
    fork.copy_reservations_from(&funding.input_supply, &allocator, &catalog, &graph);
    fork.copy_reservations_from(&funding.input_supply, &allocator, &catalog, &graph);
    for book in [&mut funding.input_supply, &mut fork] {
        let mut dependents = Vec::new();
        book.visit_dependents(0, |buyer, resource, units| {
            dependents.push((buyer, resource, units))
        });
        assert_eq!(dependents, [(1, grain, 200.0)]);
        let input = InputRequirement {
            resource: grain,
            daily_units: 200.0,
            buffer_units: 800.0,
            replenishment_batch_units: Some(50.0),
        };
        let expected_goods = 200.0 * f64::from(farm_profile.unit_price_currency);
        let input_costs = book.owner_input_costs(1, input, 0.0, None);
        assert_eq!(input_costs.goods, expected_goods);
        for _ in 0..2 {
            let quote = logistics.quote_recurring_input(
                book,
                InputRequirement {
                    daily_units: 1.0,
                    ..input
                },
                None,
                &allocator.entrances[1],
                None,
                &allocator,
                &catalog,
                &tuning,
                &network,
                &graph,
                1.0,
            );
            assert!(
                quote.goods.is_infinite(),
                "grain already promised to the financed processor cannot fund another project"
            );
            book.reject(&allocator, &graph);
            book.release_owner(1, &allocator, &graph);
            let quote = logistics.quote_recurring_input(
                book,
                input,
                Some(1),
                &allocator.entrances[1],
                None,
                &allocator,
                &catalog,
                &tuning,
                &network,
                &graph,
                1.0,
            );
            assert_eq!(
                quote.goods, expected_goods,
                "the customer owns its new input receipt"
            );
            book.reject(&allocator, &graph);
            let restored = book.owner_input_costs(1, input, 0.0, None);
            assert_eq!(restored.goods.to_bits(), input_costs.goods.to_bits());
            assert_eq!(restored.freight.to_bits(), input_costs.freight.to_bits());
            assert_eq!(restored.buffer.to_bits(), input_costs.buffer.to_bits());
        }
    }
}

#[test]
fn staffed_buyers_keep_inputs_before_other_firms_reserve_expansion() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 1.2;
    tuning.owa_export_price_multiplier = 1.0;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let farm_asset =
        register_test_utility_asset(&mut allocator, "test", "staffed_grain", "grain_farm_basic");
    let factory_asset = register_test_asset(
        &mut allocator,
        "test",
        "staffed_food",
        ZoneClass::Industrial,
    );
    let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
    let factory_profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let mut farm = make_building(20.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 10.0;
    farm.operating_budget = 100_000.0;
    farm.worker_count = physical_worker_capacity_for_profile(&farm, farm_profile);
    allocator.buildings.push(farm);
    for x in [40.0, 60.0] {
        let mut factory = make_building(x, ZoneType::Industrial, &factory_asset, 0.0);
        factory.worker_count = factory_profile.worker_capacity / 2;
        factory.operating_budget = 100_000.0;
        allocator.buildings.push(factory);
    }
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    for cash in [100_000.0, 0.0, 100_000.0] {
        allocator.buildings[1].operating_budget = cash;
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
            funding.capacities[1],
            if cash > 0.0 {
                factory_profile.worker_capacity
            } else {
                0
            }
        );
        assert_eq!(funding.capacities[2], factory_profile.worker_capacity);
        let mut claims = [0.0; 3];
        funding
            .input_supply
            .visit_dependents(0, |buyer, resource, units| {
                if resource == grain {
                    claims[buyer] += units;
                }
            });
        if cash > 0.0 {
            assert_eq!(
                claims,
                [0.0, 100.0, 100.0],
                "each current operation precedes another buyer's prospective workers"
            );
        } else {
            assert_eq!(
                claims[1], 0.0,
                "an unfunded current operation cannot retain inputs"
            );
            assert_eq!(claims[2], 200.0);
        }
    }
    // A stock-rich buyer can fund imports but reject a complete cheap local substitution:
    // repricing its inventory would remove the assets that bridge settlement. The first
    // processor then reserves the farm for expansion; additional business matches must
    // not sell that same prospective input commitment to the second processor again.
    allocator.buildings[2].worker_count = factory_profile.worker_capacity;
    allocator.buildings[2].operating_budget = 0.0;
    let mut protected_expansion = false;
    for stock in (1_300..=1_400).step_by(5) {
        allocator.buildings[2].set_inventory_units(grain, stock as f32);
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
        let mut claims = [0.0; 3];
        funding
            .input_supply
            .visit_dependents(0, |buyer, resource, units| {
                if resource == grain {
                    claims[buyer] += units;
                }
            });
        if claims[1] == 200.0 {
            protected_expansion = true;
            let mut extra = 0.0;
            funding
                .business
                .visit_additional_sales(|seller, _, resource, units, _| {
                    if seller == 0 && resource == grain {
                        extra += units;
                    }
                });
            assert_eq!(
                extra, 0.0,
                "all farm output already belongs to an accepted input plan"
            );
        }
    }
    assert!(
        protected_expansion,
        "the fixture must reach the import-backed alternative"
    );
}

#[test]
fn mutually_financed_local_recipes_close_without_an_import_funded_seed() {
    use crate::simulation::economy::definitions::compile_test_catalog;
    let authored = include_str!("../../../../../../economy/profiles.toml");
    let first = authored
        .find("[[profiles]]\nid = \"food_processor_basic\"")
        .unwrap();
    let end = first + authored[first + 1..].find("\n[[profiles]]").unwrap() + 1;
    let recipe = authored[first..end]
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
    let b = recipe
        .replace("food_processor_basic", "cycle_b")
        .replace("unit_price_currency = 12.0", "unit_price_currency = 30.0");
    // The outside feed balances the 180 kg exported from the cycle; no recipe creates mass.
    let feed = "[[resources]]\nid = \"cycle_feed\"\nquantity_unit = \"kg\"\nshipment_batch_kg = 10.0\nruntime_id = 10\nimport_unit_price_currency = 0.01\n";
    let a = format!("{a}\n[[profiles.inputs]]\nresource = \"cycle_feed\"\nunits_per_day = 180.0\n");
    let catalog = compile_test_catalog(&format!("{authored}\n{feed}\n{a}\n{b}")).unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 1.0;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    for order in [["cycle_a", "cycle_b"], ["cycle_b", "cycle_a"]] {
        let mut allocator = BuildingAllocator::new();
        let asset = register_test_asset(&mut allocator, "test", "cycle", ZoneClass::Industrial);
        for (index, id) in order.iter().enumerate() {
            let profile = catalog.profile_for_id(id).unwrap();
            let mut building = make_building(
                20.0 + index as f32 * 20.0,
                ZoneType::Industrial,
                &asset,
                0.0,
            );
            building.economy_profile_runtime_id = profile.runtime_id;
            building.worker_count = profile.worker_capacity;
            building.operating_budget = 1_000_000.0;
            allocator.buildings.push(building);
        }
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let mut funding = crate::simulation::economy::hiring::JobFunding::default();
        let mut logistics = ShipmentSystem::new();
        // Independently prove that both complete operations can pay actual local quotes.
        // This provisional book is fixture evidence, not publication of unproved live supply.
        let mut supply = crate::simulation::economy::logistics::RecurringSupply::default();
        supply.reset(&allocator, &graph, &network, &catalog, &[1.0; 2]);
        let mut carriers = [None; 2];
        logistics.write_border_carrier_quotes(
            &allocator,
            &network,
            &graph,
            &[0],
            &catalog,
            &tuning,
            &mut carriers,
        );
        for (index, id) in order.iter().enumerate() {
            use crate::simulation::economy::operation::{
                OperatingCosts, OperationPlan, OperationPrices,
            };
            let profile = catalog.profile_for_id(id).unwrap();
            let costs = OperatingCosts::for_plan_with_input_quotes(
                profile,
                OperationPlan {
                    workers: 1,
                    activity: 1.0,
                    area: 1.0,
                },
                OperationPrices {
                    utilities: local_utility_service_costs(&catalog)
                        .map(|price| price * tuning.owa_import_price_multiplier),
                    property_tax: crate::simulation::economy::fiscal::daily_property_tax(
                        ZoneType::Industrial,
                        1,
                        &policy,
                    ),
                    imports: tuning.owa_import_price_multiplier,
                    carrier_quote: carriers[index].unwrap(),
                    truck_payload: tuning.logistics.truck_payload_kg,
                },
                |_, units| {
                    if *id == "cycle_b" {
                        (units - 20.0).max(0.0)
                    } else {
                        0.0
                    }
                },
                |input| {
                    logistics.quote_recurring_input(
                        &mut supply,
                        input,
                        Some(index),
                        &allocator.entrances[index],
                        carriers[index],
                        &allocator,
                        &catalog,
                        &tuning,
                        &network,
                        &graph,
                        1.0,
                    )
                },
            );
            let revenue = if *id == "cycle_a" { 2_400.0 } else { 2_760.0 };
            assert!(
                revenue > costs.total(),
                "{id}: revenue {revenue}, costs {costs:?}"
            );
            assert!(costs.working_capital < f64::from(allocator.buildings[index].operating_budget));
            supply.accept_for(index);
        }
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
            funding.capacities,
            [1, 1],
            "joint local financing, order {order:?}"
        );
        allocator.buildings[0].operating_budget = 0.0;
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
            funding.capacities,
            [0, 0],
            "a broken financing cycle cannot publish supply"
        );
    }
}

// Commute/claim tests need funded jobs independent of a tiny fixture's retail market.
// City-funded workplaces keep the real job search and service-funding path under test.
fn fund_route_test_workplaces(allocator: &mut BuildingAllocator) {
    let asset = register_test_utility_asset(
        allocator,
        "test",
        "route_test_employer",
        "power_plant_basic",
    );
    let profile = load_runtime_economy_catalog()
        .unwrap()
        .profile_for_id("power_plant_basic")
        .unwrap()
        .runtime_id;
    for building in &mut allocator.buildings {
        if building.zone_type == ZoneType::Commercial {
            building.zone_type = ZoneType::None;
            building.asset_id = asset.clone();
            building.economy_profile_runtime_id = profile;
        }
    }
}

#[test]
fn live_hiring_and_admission_keep_jobs_after_paid_stock_and_payroll() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "funded_factory",
        ZoneClass::Industrial,
    );
    let mut factory = make_building(50.0, ZoneType::Industrial, &asset, 0.0);
    let capital = profile.zoned_startup_budget(&catalog, tuning.owa_import_price_multiplier);
    factory.operating_budget = capital;
    allocator.buildings.push(factory);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    let mut check =
        |allocator: &mut BuildingAllocator, logistics: &mut ShipmentSystem, expected| {
            households.job_funding.refresh(
                allocator,
                &households.households,
                logistics,
                &network,
                &graph,
                &catalog,
                &tuning,
                &policy,
                &[],
                100_000.0,
                420,
            );
            assert_eq!(households.job_funding.capacities[0], expected);
            // The demand snapshot reads the copies a published census leaves (`SIM-02-I`).
            households.publish_funded_job_capacities(allocator);
            let mut demand = crate::simulation::economy::demand::DemandSystem::new();
            let zoning = crate::simulation::zoning::ZoningSystem::new(
                &crate::simulation::core::config::WorldConfig::default(),
            );
            let terrain = crate::simulation::terrain::TerrainSystem::new(4, 4);
            let mut surface = crate::simulation::network::surface::RoadSurfaceSystem::new(512.0);
            surface.compile_dirty(&graph, &terrain);
            demand.publish_census_if_stale_for_test(
                allocator,
                &mut households,
                &network,
                &graph,
                logistics,
                &policy,
                100_000.0,
                420,
            );
            demand.run_planning_hour_for_test(
                allocator,
                &mut households,
                &graph,
                &zoning,
                100_000.0,
                &policy,
                &network,
                logistics,
                crate::simulation::buildings::allocator::BuildingSiteEnvironment {
                    road_surface: &surface,
                    terrain: &terrain,
                },
                420,
            );
            assert_eq!(
                demand.scenario_diagnostics().0.move_in_job_slots,
                expected,
                "admission and workplace assignment must consume the same funded jobs"
            );
        };
    check(&mut allocator, &mut logistics, profile.worker_capacity);
    let payroll = profile.worker_capacity as f32 * profile.average_daily_wage();
    let paid_goods = capital - payroll - 8.0;
    allocator.buildings[0].operating_budget = payroll;
    logistics.shipments.push(Shipment {
        id: 1,
        resource_runtime_id: profile.inputs[0].resource_runtime_id,
        amount: paid_goods
            / (catalog
                .unit_price_for_resource(profile.inputs[0].resource_runtime_id)
                .unwrap()
                * tuning.owa_import_price_multiplier),
        source: ShipmentEndpoint::OwaBorder(0),
        destination: ShipmentEndpoint::Building(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier: crate::simulation::economy::agents::AgentHandle::from_parts(7, 0),
        total_cost: paid_goods + 8.0,
        freight_cost: 8.0,
        eta_hours: 1,
        leg_started_hour: 0,
        trip_event_slot: 0,
        cargo_loaded: true,
    });
    check(&mut allocator, &mut logistics, profile.worker_capacity);
    allocator.buildings[0].operating_budget -= payroll;
    check(&mut allocator, &mut logistics, profile.worker_capacity);
    allocator.buildings[0].set_inventory_units(
        profile.inputs[0].resource_runtime_id,
        logistics.shipments[0].amount,
    );
    logistics.shipments[0].status = ShipmentStatus::Fulfilled;
    check(&mut allocator, &mut logistics, profile.worker_capacity);
}

#[test]
fn owner_money_and_unprofitable_exports_do_not_create_live_jobs() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "unfunded_factory",
        ZoneClass::Industrial,
    );
    let mut factory = make_building(50.0, ZoneType::Industrial, &asset, 0.0);
    factory.operating_budget = 1_000_000.0;
    allocator.buildings.push(factory);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    for outside_bid in [0.0, 0.01] {
        tuning.owa_export_price_multiplier = outside_bid;
        households.job_funding.refresh(
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
            households.job_funding.capacities[0], 0,
            "startup money cannot substitute for a market that covers recurring costs"
        );
    }
    tuning.owa_export_price_multiplier = 1.0;
    allocator.buildings[0].operating_budget = 0.0;
    households.job_funding.refresh(
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
        households.job_funding.capacities[0], 0,
        "profitable sales still need working capital"
    );
    allocator.buildings[0].operating_budget = 1_000_000.0;
    allocator.buildings[0].set_construction_hours_left(1);
    households.job_funding.refresh(
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
        households.job_funding.capacities[0], 0,
        "future production cannot pay arrivals"
    );
    allocator.buildings[0].construction_complete_slot = 0;
    for (role, expected) in [
        (NodeType::Border, 10),
        (NodeType::Junction, 0),
        (NodeType::Border, 10),
    ] {
        graph.set_node_type(0, role);
        households.job_funding.refresh(
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
            households.job_funding.capacities[0], expected,
            "gateway role changes must invalidate outside quotes even without a CCH rebuild"
        );
    }
}

#[test]
fn profitable_local_staffing_survives_an_unprofitable_full_export_plan() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_export_price_multiplier = 0.01;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let factory_asset = register_test_asset(
        &mut allocator,
        "test",
        "local_factory",
        ZoneClass::Industrial,
    );
    let shop_asset =
        register_test_asset(&mut allocator, "test", "local_buyer", ZoneClass::Commercial);
    let home_asset = register_test_asset(
        &mut allocator,
        "test",
        "local_customers",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Industrial,
        &factory_asset,
        0.0,
    ));
    allocator
        .buildings
        .push(make_building(40.0, ZoneType::Commercial, &shop_asset, 0.0));
    allocator
        .buildings
        .push(make_building(60.0, ZoneType::Residential, &home_asset, 0.0));
    allocator.buildings[0].operating_budget = 100_000.0;
    allocator.buildings[1].operating_budget = 100_000.0;
    allocator.buildings[1].worker_count = 8;
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(2, 100, 30.0, 30.0));
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
    assert_eq!(
        households.job_funding.capacities[1], 0,
        "customer savings and owner capital cannot promise recurring shop payroll"
    );
    households.households[0].gross_daily_wages = 10_000.0;
    households.households[0].employed_adults = 100;
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
    assert!(households.job_funding.capacities[1] > 0);
    let jobs = households.job_funding.capacities[0];
    assert!(
        jobs > 0
            && jobs
                < catalog
                    .profile_for_id("food_processor_basic")
                    .unwrap()
                    .worker_capacity,
        "local buyers support a smaller operation even though full export production loses money: {jobs}"
    );
}

#[test]
#[ignore = "manual matched release timing of payroll plus final money publication"]
fn benchmark_payroll_phase() {
    use std::{hint::black_box, time::Instant};
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    for count in [1_024_usize, 8_192, 65_536] {
        let mut allocator = BuildingAllocator::new();
        allocator
            .buildings
            .push(make_building(0.0, ZoneType::Residential, "test:home", 0.0));
        let capacity = profile.worker_capacity as usize;
        for _ in 0..count.div_ceil(capacity) {
            let mut factory = make_building(30.0, ZoneType::Industrial, "test:factory", 0.0);
            factory.operating_budget = 1_000_000.0;
            for input in &profile.inputs {
                factory.set_inventory_units(input.resource_runtime_id, 1_000.0);
            }
            allocator.buildings.push(factory);
        }
        let mut agents = AgentSystem::new();
        let mut households = HouseholdSystem::new();
        for idx in 0..count {
            households
                .households
                .push(make_household(0, 1, 1_000.0, 5.0));
            let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
            agents.assign_household_id(agent, idx);
            let work = 1 + idx / capacity;
            agents.assign_work_building(agent, work, 0);
            allocator.buildings[work].worker_count += 1;
        }
        households.job_funding.capacities = allocator
            .buildings
            .iter()
            .map(|building| building.worker_count)
            .collect();
        let logistics = ShipmentSystem::new();
        let mut treasury = 0.0;
        let mut phase = || {
            black_box(households.pay_funded_daily_wages(
                &mut agents,
                &mut allocator,
                0.12,
                &mut treasury,
                &logistics,
            ));
            households.recount_every_household(&agents, &allocator);
        };
        phase();
        let mut samples = [0.0; 11];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..4 {
                phase();
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 4.0;
        }
        assert!(agents.work_building.iter().all(|&work| work != usize::MAX));
        samples.sort_by(f64::total_cmp);
        eprintln!("payroll_phase workers={count} median_ms={:.3}", samples[5]);
    }
}

#[test]
fn no_car_unemployed_adult_can_take_reachable_job_even_with_household_savings() {
    for (budget, stock_days) in [(0.0, 0.0), (100_000.0, 7.0)] {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(make_foot_only_edge(n0, n1));
        graph.rebuild_adjacency_list();

        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = CchGraph::build(&graph);

        let mut households = HouseholdSystem::new();
        households.households.push(make_household(0, 1, 0.0, 0.0));
        households.households[0].budget = budget;
        households.households[0].stock = stock_days;
        households.households[0].stock_days = stock_days;

        let mut allocator = BuildingAllocator::new();
        let residential_asset = register_test_asset(
            &mut allocator,
            "test",
            "walk_job_res",
            ZoneClass::Residential,
        );
        let commercial_asset = register_test_asset(
            &mut allocator,
            "test",
            "walk_job_com",
            ZoneClass::Commercial,
        );
        allocator.buildings.push(make_building(
            0.0,
            ZoneType::Residential,
            &residential_asset,
            0.0,
        ));
        allocator.buildings.push(make_building(
            20.0,
            ZoneType::Commercial,
            &commercial_asset,
            0.0,
        ));
        fund_route_test_workplaces(&mut allocator);
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);

        let mut agents = AgentSystem::new();
        let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.assign_household_id(agent, 0);
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.has_car[agent] = false;

        households.recount_every_household(&agents, &allocator);
        households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

        assert_eq!(agents.work_building[agent], 1);
        assert_eq!(allocator.buildings[1].worker_count, 1);
    }
}

#[test]
fn children_and_elders_do_not_take_jobs() {
    let (graph, network) = simple_work_graph();
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 2, 0.0, 0.0));
    households.households[0].child_count = 1;
    households.households[0].adult_count = 0;
    households.households[0].elder_count = 1;
    households.households[0].budget = 0.0;
    households.households[0].stock = 0.0;
    households.households[0].stock_days = 0.0;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "age_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset =
        register_test_asset(&mut allocator, "test", "age_job_com", ZoneClass::Commercial);
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    fund_route_test_workplaces(&mut allocator);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let child = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(child, 0);
    agents.age_group[child] = AGE_CHILD;
    agents.transit[child] = TRANSIT_IN_BUILDING;
    agents.current_building[child] = 0;
    let elder = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(elder, 0);
    agents.age_group[elder] = AGE_ELDER;
    agents.transit[elder] = TRANSIT_IN_BUILDING;
    agents.current_building[elder] = 0;

    households.recount_every_household(&agents, &allocator);
    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[child], usize::MAX);
    assert_eq!(agents.work_building[elder], usize::MAX);
    assert_eq!(allocator.buildings[1].worker_count, 0);
}

#[test]
fn worker_can_take_far_reachable_job() {
    let (graph, network) = work_graph_to(6_500.0);
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));
    households.households[0].budget = 0.0;
    households.households[0].stock = 0.0;
    households.households[0].stock_days = 0.0;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "far_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset =
        register_test_asset(&mut allocator, "test", "far_job_com", ZoneClass::Commercial);
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        6_000.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    fund_route_test_workplaces(&mut allocator);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(agent, 0);
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.has_car[agent] = true;

    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[agent], 1);
    assert_eq!(allocator.buildings[1].worker_count, 1);
    assert_eq!(
        households.households[0].employed_adults, 1,
        "the hire counts at once, before the household's next phase"
    );
}

#[test]
fn workplace_claim_falls_back_to_next_ranked_job_when_best_fills() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let worker_capacity = catalog
        .profile_for_id("power_plant_basic")
        .expect("funded service profile")
        .worker_capacity;

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 2, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "fallback_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "fallback_job_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        30.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    fund_route_test_workplaces(&mut allocator);
    allocator.rebuild_zone_index();
    let (graph, network) = simple_work_graph();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for agent in [a0, a1] {
        agents.assign_household_id(agent, 0);
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.has_car[agent] = true;
    }

    households.assign_agent_workplaces_with_service_funding(
        &mut agents,
        &mut allocator,
        &network,
        &graph,
        &[
            0.0,
            1.0 / worker_capacity as f32,
            1.0 / worker_capacity as f32,
        ],
        &mut ShipmentSystem::new(),
        &crate::simulation::economy::fiscal::CityFiscalPolicy::default(),
        100_000.0,
        420,
    );

    assert_eq!(agents.work_building[a0], 1);
    assert_eq!(agents.work_building[a1], 2);
    assert_eq!(allocator.buildings[1].worker_count, 1);
    assert_eq!(allocator.buildings[2].worker_count, 1);
}

#[test]
fn missing_entrance_cache_does_not_use_straight_line_work_fallback() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));
    households.households[0].budget = 0.0;
    households.households[0].stock = 0.0;
    households.households[0].stock_days = 0.0;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "missing_cache_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "missing_cache_job_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    fund_route_test_workplaces(&mut allocator);
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(agent, 0);
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.has_car[agent] = true;

    let (graph, network) = simple_work_graph();
    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[agent], usize::MAX);
    assert_eq!(allocator.buildings[1].worker_count, 0);
}

#[test]
fn deserted_employer_is_ejected_before_wages() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let daily_wage = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile")
        .average_daily_wage();

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_wage_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_wage_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings[1].operating_budget = daily_wage;
    allocator.buildings[1].worker_count = 1;
    allocator.buildings[1].is_deserted = true;

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(agent, 0);
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);

    let mut treasury_balance = 0.0;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(agents.work_building[agent], usize::MAX);
    assert_eq!(allocator.buildings[1].worker_count, 0);
    assert_eq!(households.households[0].budget, 0.0);
    assert_eq!(allocator.buildings[1].operating_budget, daily_wage);
}

#[test]
fn payroll_keeps_funded_store_workers_without_city_sales_activity() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grocery = catalog.profile_for_id("grocery_basic").unwrap();
    let staff = grocery.worker_capacity;
    let daily_payroll = grocery.average_daily_wage() * staff as f32;
    assert!(staff > 1);

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(0, staff as u16, 0.0, 0.0));
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "funded_store_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "funded_store_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings[1].worker_count = staff;
    allocator.buildings[1].operating_budget = daily_payroll * 3.0;

    let mut agents = AgentSystem::new();
    for _ in 0..staff {
        let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.assign_household_id(agent, 0);
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.assign_work_building(agent, 1, 0);
    }
    // Hiring funded the whole store. No household has bought anything, so the citywide
    // sales activity alone would justify one worker.
    households.job_funding.capacities = vec![0, staff];

    let mut treasury_balance = 0.0;
    for day in 1..=3 {
        households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);
        assert_eq!(allocator.buildings[1].worker_count, staff);
        assert!(
            (0..agents.len()).all(|agent| agents.work_building[agent] == 1
                && agents.consecutive_unpaid_days[agent] == 0)
        );
        assert!((households.households[0].budget - daily_payroll * day as f32).abs() < 0.01);
    }
    assert!(allocator.buildings[1].operating_budget.abs() < 0.01);
}

#[test]
fn insolvent_self_fire_decrements_worker_count() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "insolvent_wage_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "insolvent_wage_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings[1].operating_budget = 0.0;
    allocator.buildings[1].worker_count = 1;

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(agent, 0);
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);
    agents.consecutive_unpaid_days[agent] = 1;

    households.job_funding.capacities = vec![0, 1];
    allocator.prepare_production_changes();
    let production_revision = allocator.production_revision();
    let mut treasury_balance = 0.0;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(agents.work_building[agent], usize::MAX);
    assert_eq!(allocator.buildings[1].worker_count, 0);
    assert_eq!(households.households[0].budget, 0.0);
    assert_eq!(allocator.buildings[1].operating_budget, 0.0);
    let mut changed = Vec::new();
    assert!(allocator.production_changes_since(production_revision, &mut changed));
    assert_eq!(changed, [1]);
}

#[test]
fn wage_payment_liquidates_output_inventory_before_unpaid() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let processor = catalog
        .profile_for_id("food_processor_basic")
        .expect("processor starter profile");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let daily_wage = processor.average_daily_wage();

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "inventory_wage_res",
        ZoneClass::Residential,
    );
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "inventory_wage_ind",
        ZoneClass::Industrial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Industrial,
        &industrial_asset,
        0.0,
    ));
    allocator.buildings[1].operating_budget = 0.0;
    allocator.buildings[1].worker_count = 1;
    allocator.buildings[1].set_inventory_units(packaged_food, 100.0);

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(agent, 0);
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);

    households.job_funding.capacities = vec![0, 1];
    let mut treasury_balance = 0.0;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(agents.work_building[agent], 1);
    assert_eq!(agents.consecutive_unpaid_days[agent], 0);
    assert!((households.households[0].budget - daily_wage).abs() <= 0.001);
    assert!(allocator.buildings[1].inventory_units(packaged_food) < 100.0);
    assert!(allocator.buildings[1].operating_budget.abs() <= 0.001);
}

#[test]
fn full_current_workplace_is_scored_before_switching() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let employer = catalog
        .profile_for_id("power_plant_basic")
        .expect("funded service profile");
    let worker_capacity = employer.worker_capacity;

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));
    households.households[0].budget = 0.0;
    households.households[0].stock = 0.0;
    households.households[0].stock_days = 0.0;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "stay_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "stay_job_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        200.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings[1].worker_count = worker_capacity;
    fund_route_test_workplaces(&mut allocator);
    let (graph, network) = simple_work_graph();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(agent, 0);
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.has_car[agent] = true;
    agents.assign_work_building(agent, 1, 0);
    agents.job_lock_until_day[agent] = 0;

    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[agent], 1);
    assert_eq!(allocator.buildings[1].worker_count, worker_capacity);
    assert_eq!(allocator.buildings[2].worker_count, 0);
}

#[test]
fn immigrant_household_assigns_nearby_work_during_founding() {
    let mut households = HouseholdSystem::new();
    let catalog = load_runtime_economy_catalog().expect("catalog");
    let tuning = load_runtime_economy_tuning().expect("tuning");
    let hid = households.admit_immigrant_household(
        &catalog,
        &tuning,
        0,
        2,
        crate::simulation::economy::agents::household_age_composition(0, 0, 2),
    );
    households.households[hid].budget = 0.0;
    households.households[hid].stock = 1.0;
    households.households[hid].stock_days = 0.5;

    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "test", "res_house", ZoneClass::Residential);
    let industrial_asset =
        register_test_asset(&mut allocator, "test", "ind_shop", ZoneClass::Industrial);
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Industrial,
        &industrial_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();
    allocator.buildings[1].operating_budget = catalog
        .profile_for_id("food_processor_basic")
        .unwrap()
        .zoned_startup_budget(&catalog, tuning.owa_import_price_multiplier);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for a in [a0, a1] {
        agents.assign_household_id(a, hid);
        agents.transit[a] = TRANSIT_IN_BUILDING;
        agents.current_building[a] = 0;
        agents.target_building[a] = usize::MAX;
        agents.current_node[a] = 0;
        agents.has_car[a] = true;
    }

    households.consume_household_stock(&mut agents);
    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[a0], 1);
    assert_eq!(agents.planned_activity[a0], 0);
    assert_eq!(agents.planned_target_building[a0], usize::MAX);
}

#[test]
fn household_counts_and_wages_update_at_its_own_hourly_phase() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let wage = catalog
        .profile_for_id("food_processor_basic")
        .unwrap()
        .average_daily_wage();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "test", "phase_home", ZoneClass::Residential);
    let factory = register_test_asset(
        &mut allocator,
        "test",
        "phase_factory",
        ZoneClass::Industrial,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    allocator
        .buildings
        .push(make_building(20.0, ZoneType::Industrial, &factory, 0.0));
    let mut households = HouseholdSystem::new();
    let mut agents = AgentSystem::new();
    for household_id in 0..2 {
        households.households.push(make_household(0, 0, 0.0, 0.0));
        for _ in 0..2 {
            let member = agents.spawn_housed_agent(0, 0.0, 0.0);
            agents.assign_household_id(member, household_id);
        }
    }
    for member in [0, 2, 3] {
        allocator.set_worker_count(1, allocator.buildings[1].worker_count + 1);
        agents.assign_work_building(member, 1, 0);
    }
    super::super::membership::assert_worker_counts_match_assignments(&agents, &allocator);
    households
        .handles
        .register_appended(households.households.len());
    let first = households.handles.handle(0);
    households.run_household_phase(&[first], &mut agents, &mut allocator);
    let counted = &households.households[0];
    assert_eq!((counted.member_count, counted.adult_count), (2, 2));
    assert_eq!(counted.employed_adults, 1);
    assert_eq!(counted.gross_daily_wages, wage);
    let waiting = &households.households[1];
    assert_eq!(
        (waiting.member_count, waiting.employed_adults),
        (0, 0),
        "the second household keeps its counts until its own phase"
    );
    let second = households.handles.handle(1);
    households.run_household_phase(&[second], &mut agents, &mut allocator);
    let counted = &households.households[1];
    assert_eq!((counted.member_count, counted.adult_count), (2, 2));
    assert_eq!(counted.employed_adults, 2);
    assert_eq!(counted.gross_daily_wages, 2.0 * wage);
}

#[test]
fn consumer_market_uses_each_familys_income_instead_of_wallet_balance() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "consumer_home",
        ZoneClass::Residential,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    let mut families = vec![
        make_household(0, 1, 0.0, 0.0),
        make_household(0, 1, 0.0, 0.0),
    ];
    families[0].gross_daily_wages = 100_000.0;
    families[0].employed_adults = 1;
    families[1].budget = 1_000_000.0;
    let buyers = |families: &[Household],
                  policy: &crate::simulation::economy::fiscal::CityFiscalPolicy| {
        affordable_consumer_residents(families, &allocator, &catalog, &tuning, policy, 100_000.0)
    };
    assert_eq!(
        buyers(&families, &policy),
        1.0,
        "one rich family cannot finance another's recurring purchases; savings are finite"
    );
    families[0].budget = 0.0;
    assert_eq!(
        buyers(&families, &policy),
        1.0,
        "paying a bill cannot erase recurring wages"
    );
    families[1].gross_daily_wages = 30.0;
    families[1].employed_adults = 1;
    let partial = buyers(&families, &policy);
    assert!(partial > 1.0 && partial < 2.0, "{partial}");
    policy.household_vat_rate = 1.0;
    assert!(
        buyers(&families, &policy) < partial,
        "VAT must not become store revenue"
    );
}

#[test]
fn consumer_market_reserves_transfers_once_and_excludes_temporary_income() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "pension_home",
        ZoneClass::Residential,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    let mut pensioner = make_household(0, 1, 0.0, 0.0);
    pensioner.adult_count = 0;
    pensioner.elder_count = 1;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy {
        pension_per_elder_per_day: 100.0,
        unemployment_benefit_per_adult_per_day: 100.0,
        ..Default::default()
    };
    let buyers = |families: &[Household], treasury| {
        affordable_consumer_residents(families, &allocator, &catalog, &tuning, &policy, treasury)
    };
    assert_eq!(buyers(&[pensioner.clone(), pensioner.clone()], 100.0), 1.0);
    assert_eq!(buyers(&[pensioner.clone(), pensioner.clone()], 0.0), 0.0);
    let unemployed = make_household(0, 1, 100.0, 0.0);
    assert_eq!(
        buyers(std::slice::from_ref(&unemployed), 100.0),
        0.0,
        "time-limited transfers cannot promise ongoing customer revenue"
    );
    assert_eq!(
        buyers(&[unemployed, pensioner], 100.0),
        0.0,
        "earlier temporary claims still consume city cash before later permanent claims"
    );
}

#[test]
fn consumer_reservations_keep_family_money_quantities_and_exact_rollback_separate() {
    use crate::simulation::economy::customers::ConsumerBudgets;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "consumer_claim_home",
        ZoneClass::Residential,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    let mut families = vec![
        make_household(0, 1, 0.0, 0.0),
        make_household(0, 1, 100_000.0, 0.0),
    ];
    families[0].gross_daily_wages = 100.0;
    families[0].employed_adults = 1;
    let resource = catalog
        .resource_runtime_id_for_id("household_supplies")
        .unwrap();
    let mut customers = ConsumerBudgets::default();
    customers.reset(&families, &allocator, &catalog, &tuning, &policy, 100_000.0);
    assert_eq!(
        customers.affordable_residents(),
        affordable_consumer_residents(&families, &allocator, &catalog, &tuning, &policy, 100_000.0)
    );
    assert_eq!(
        customers.available(1, resource, 1.0),
        0.0,
        "savings and a neighbour's wages cannot promise recurring purchases"
    );
    assert_eq!(
        customers.available(0, resource, 1.0),
        1.0,
        "cheap goods cannot create more than the family's actual daily need"
    );
    let expensive = customers.available(0, resource, 500.0);
    assert!(expensive > 0.0 && expensive < 1.0);
    assert_eq!(
        customers.claim(0, resource, expensive * 0.25, 500.0),
        expensive * 0.25
    );
    assert_eq!(
        customers.claim(0, resource, expensive * 0.25, 500.0),
        expensive * 0.25
    );
    customers.reject();
    assert_eq!(
        customers.available(0, resource, 500.0),
        expensive,
        "multiple staged claims must restore the original spending bits"
    );
    assert_eq!(customers.claim(0, resource, 10.0, 1.0), 1.0);
    customers.accept();
    assert_eq!(customers.available(0, resource, 1.0), 0.0);
    customers.reject();
    assert_eq!(
        customers.available(0, resource, 500.0),
        0.0,
        "rejecting a later quote cannot restore an accepted household's quantity"
    );
    customers.reset(&families, &allocator, &catalog, &tuning, &policy, 100_000.0);
    assert_eq!(customers.available(0, resource, 500.0), expensive);
    customers.clear();
    assert_eq!(customers.available(0, resource, 1.0), 0.0);
}

#[test]
fn routed_consumer_claims_restore_eligibility_and_exclude_disconnected_money() {
    use crate::simulation::economy::customers::ConsumerMarket;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let (mut graph, mut network) = simple_work_graph();
    let a = graph.add_node(Vector3::new(900.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(1300.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(make_road_edge(a, b, 900.0, 1300.0));
    graph.rebuild_adjacency_list();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "routed_customer_home",
        ZoneClass::Residential,
    );
    let shop = register_test_asset(
        &mut allocator,
        "test",
        "routed_customer_shop",
        ZoneClass::Commercial,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    allocator
        .buildings
        .push(make_building(40.0, ZoneType::Commercial, &shop, 0.0));
    let mut remote = make_building(1000.0, ZoneType::Residential, &home, 0.0);
    remote.edge_idx = 1;
    allocator.buildings.push(remote);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut families = vec![
        make_household(0, 1, 0.0, 0.0),
        make_household(2, 100, 0.0, 0.0),
    ];
    for family in &mut families {
        family.gross_daily_wages = 100_000.0;
        family.employed_adults = 1;
        family.shopping_has_car = true;
    }
    let resource = catalog
        .resource_runtime_id_for_id("household_supplies")
        .unwrap();
    let mut customers = ConsumerMarket::default();
    customers.reset(
        &families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph,
    );
    let quote = |customers: &mut ConsumerMarket| {
        customers.claim_reachable(
            &allocator.entrances[1],
            resource,
            200.0,
            25.0,
            &allocator,
            &network,
            &graph,
        )
    };
    assert_eq!(
        quote(&mut customers),
        1.0,
        "neither the disconnected 100 residents nor duplicate foot/car entries add sales"
    );
    assert_eq!(quote(&mut customers), 0.0);
    customers.reject();
    assert_eq!(
        quote(&mut customers),
        1.0,
        "rollback must restore bucket eligibility too"
    );
    customers.accept_for(1);
    customers.reject();
    assert_eq!(quote(&mut customers), 0.0);
    let mut construction = ConsumerMarket::default();
    construction.copy_reservations_from(&customers);
    assert_eq!(quote(&mut construction), 0.0);
    construction.release_owner(1);
    assert_eq!(quote(&mut construction), 1.0);
    construction.reject();
    assert_eq!(
        quote(&mut construction),
        0.0,
        "failed replacement restores the incumbent's exact existing claim"
    );
    construction.release_owner(1);
    assert_eq!(
        construction.claim_reachable(
            &allocator.entrances[1],
            resource,
            0.5,
            25.0,
            &allocator,
            &network,
            &graph
        ),
        0.5
    );
    construction.accept_for(1);
    assert_eq!(
        quote(&mut construction),
        0.5,
        "accepted replacement retires the old receipt and retains its actual new purchases"
    );
    construction.reject();
    construction.copy_reservations_from(&customers);
    assert_eq!(
        quote(&mut construction),
        0.0,
        "a warm fork must restore the live book"
    );
    assert_eq!(
        quote(&mut customers),
        0.0,
        "construction cannot mutate the live accounts"
    );
    construction.prepare_project_owners(4, 4);
    for round in 0..32 {
        construction.begin_revalidation();
        construction.release_owner(1);
        assert_eq!(
            construction.claim_reachable(
                &allocator.entrances[1],
                resource,
                0.25,
                50.0,
                &allocator,
                &network,
                &graph
            ),
            0.25
        );
        construction.accept_for(1);
        assert_eq!(quote(&mut construction), 0.75);
        construction.accept_for(3);
        assert_eq!(construction.reserved_units(1, resource), 0.25);
        assert_eq!(construction.reserved_units(3, resource), 0.75);
        if round % 2 == 0 {
            construction.release_owner(3);
            assert_eq!(quote(&mut construction), 0.75);
        }
        construction.reject_revalidation();
        assert_eq!(construction.reserved_units(1, resource), 1.0);
        assert_eq!(construction.reserved_units(3, resource), 0.0);
        assert_eq!(quote(&mut construction), 0.0);
        construction.release_owner(1);
        assert_eq!(
            quote(&mut construction),
            1.0,
            "outer rejection must restore receipt ownership and foot/car bucket eligibility"
        );
        construction.reject();
    }
    customers.reset(
        &families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph,
    );
    assert_eq!(
        quote(&mut customers),
        1.0,
        "warm census refills exhausted trees"
    );
    customers.reject();
    families[0].adult_count = 0;
    families[0].elder_count = 0;
    families[0].child_count = 1;
    customers.reset(
        &families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph,
    );
    assert_eq!(
        quote(&mut customers),
        0.0,
        "a child-only family has no eligible shopper"
    );
    customers.clear();
    assert_eq!(quote(&mut customers), 0.0);
}

#[test]
fn routed_consumer_claims_require_the_return_trip_and_the_households_mode() {
    use crate::simulation::economy::customers::ConsumerMarket;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let (mut graph, mut network) = simple_work_graph();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "consumer_mode_home",
        ZoneClass::Residential,
    );
    let shop = register_test_asset(
        &mut allocator,
        "test",
        "consumer_mode_shop",
        ZoneClass::Commercial,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    allocator
        .buildings
        .push(make_building(40.0, ZoneType::Commercial, &shop, 0.0));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    for entrance in &mut allocator.entrances {
        entrance.foot_lane_fwd = usize::MAX;
        entrance.foot_lane_bkw = usize::MAX;
    }
    let mut families = vec![make_household(0, 1, 0.0, 0.0)];
    families[0].gross_daily_wages = 1000.0;
    families[0].employed_adults = 1;
    let resource = catalog
        .resource_runtime_id_for_id("household_supplies")
        .unwrap();
    let mut customers = ConsumerMarket::default();
    customers.reset(
        &families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph,
    );
    assert_eq!(
        customers.claim_reachable(
            &allocator.entrances[1],
            resource,
            1.0,
            25.0,
            &allocator,
            &network,
            &graph
        ),
        0.0,
        "car-only frontage cannot serve a family without a car"
    );
    let builds = customers.index_builds_for_test();
    families[0].shopping_has_car = true;
    customers.reset(
        &families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph,
    );
    assert_eq!(
        customers.claim_reachable(
            &allocator.entrances[1],
            resource,
            1.0,
            25.0,
            &allocator,
            &network,
            &graph
        ),
        1.0
    );
    customers.reject();
    assert_eq!(customers.index_builds_for_test(), builds);
    for has_car in [false, true, false, true] {
        families[0].shopping_has_car = has_car;
        customers.reset(
            &families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph,
        );
        let mut fork = ConsumerMarket::default();
        fork.copy_reservations_from(&customers);
        for market in [&mut customers, &mut fork] {
            assert_eq!(
                market.claim_reachable(
                    &allocator.entrances[1],
                    resource,
                    1.0,
                    25.0,
                    &allocator,
                    &network,
                    &graph
                ),
                f64::from(u8::from(has_car))
            );
            market.reject();
        }
        assert_eq!(
            customers.index_builds_for_test(),
            builds,
            "a changed car flag must not rebuild immutable component membership"
        );
    }
    // A proposed forward-only door is reachable outward along the shared frontage,
    // but cannot return to the home's forward-only door through the dead-end road.
    graph.edge_mut(0).bkw_lanes = 0;
    graph.edge_mut(0).allowed_types = TransitFlags::CAR;
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    customers.reset(
        &families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph,
    );
    let mut proposed = allocator.entrances[1].clone();
    proposed.car_lane_bkw = usize::MAX;
    let paths = std::sync::atomic::AtomicU32::new(0);
    let home_origin = crate::simulation::economy::agents::tick::BuildingTripEnds::origin(
        0, &allocator, &network, &graph,
    );
    let destination = crate::simulation::economy::agents::tick::BuildingTripEnds::at_entrance(
        &proposed, false, &network, &graph,
    );
    assert!(matches!(
        crate::simulation::economy::agents::tick::estimate_trip_seconds_between(
            &home_origin,
            &destination,
            true,
            &network,
            &graph,
            &paths,
            f32::INFINITY,
            f32::INFINITY,
            None
        ),
        crate::simulation::economy::agents::tick::TripEstimate::Seconds(Some(_))
    ));
    assert_eq!(
        customers.claim_reachable(&proposed, resource, 1.0, 25.0, &allocator, &network, &graph),
        0.0,
        "one outward trip cannot promise a shopping round trip"
    );
}

#[test]
fn walking_component_proofs_follow_directed_lane_connectors() {
    use crate::simulation::economy::agents::tick::{
        BuildingTripEnds, BuildingWalkingAccess, PedestrianReachability,
        building_origin_trip_is_feasible,
    };
    use crate::simulation::network::lanes::LaneType;
    let (mut graph, mut network) = simple_work_graph();
    let end = graph.add_node(Vector3::new(700.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(make_road_edge(1, end, 300.0, 700.0));
    graph.rebuild_adjacency_list();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "test", "walk_proof", ZoneClass::Residential);
    for (x, edge) in [(0.0, 0), (100.0, 0), (400.0, 1), (500.0, 1)] {
        let mut building = make_building(x, ZoneType::Residential, &asset, 0.0);
        building.edge_idx = edge;
        allocator.buildings.push(building);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut reachability = PedestrianReachability::build(&network, &graph);
    let queries = std::sync::atomic::AtomicU32::new(0);
    let mut query = crate::simulation::pathing::cch::CchQueryScratch::default();
    query.prepare(&network.cch_graph, &graph);
    let mut proven = 0;
    for from in 0..4 {
        for to in 0..4 {
            if from == to {
                continue;
            }
            let origin = BuildingTripEnds::origin(from, &allocator, &network, &graph);
            let destination = BuildingTripEnds::destination(to, &allocator, &network, &graph);
            let [walking_origin, _] =
                BuildingTripEnds::walking_pair(from, &allocator, &network, &graph);
            let [_, walking_destination] =
                BuildingTripEnds::walking_pair(to, &allocator, &network, &graph);
            assert_eq!(
                walking_origin.walking_groups(&reachability),
                origin.walking_groups(&reachability)
            );
            assert_eq!(
                walking_destination.walking_groups(&reachability),
                destination.walking_groups(&reachability)
            );
            assert_eq!(
                walking_origin.has_proven_walk_to(&walking_destination, &reachability),
                origin.has_proven_walk_to(&destination, &reachability)
            );
            let compact_origin = BuildingWalkingAccess::at_entrance(
                &allocator.entrances[from],
                &network,
                &graph,
                &reachability,
            );
            let compact_destination = BuildingWalkingAccess::at_entrance(
                &allocator.entrances[to],
                &network,
                &graph,
                &reachability,
            );
            assert_eq!(
                compact_origin.proves_to(compact_destination),
                origin.has_proven_walk_to(&destination, &reachability)
            );
            assert_eq!(
                walking_origin.can_reach(
                    &walking_destination,
                    false,
                    &network,
                    &graph,
                    &mut reachability,
                    &mut query
                ),
                origin.can_reach(
                    &destination,
                    false,
                    &network,
                    &graph,
                    &mut reachability,
                    &mut query
                )
            );
            for has_car in [false, true] {
                assert_eq!(
                    origin.can_reach(
                        &destination,
                        has_car,
                        &network,
                        &graph,
                        &mut reachability,
                        &mut query
                    ),
                    building_origin_trip_is_feasible(
                        from, to, 0, has_car, &allocator, &network, &graph, &queries
                    )
                );
            }
            if origin.has_proven_walk_to(&destination, &reachability) {
                proven += 1;
                assert!(
                    building_origin_trip_is_feasible(
                        from, to, 0, false, &allocator, &network, &graph, &queries
                    ),
                    "false lane proof for {from} -> {to}"
                );
            }
        }
    }
    assert!(
        proven > 0,
        "connected pedestrian lanes must avoid repeated path searches"
    );
    allocator.entrances[0].foot_lane_bkw = usize::MAX;
    allocator.entrances[3].foot_lane_bkw = usize::MAX;
    let origin = BuildingTripEnds::origin(0, &allocator, &network, &graph);
    let destination = BuildingTripEnds::destination(3, &allocator, &network, &graph);
    assert!(
        !origin.has_proven_walk_to(&destination, &reachability),
        "shared-node ends retain the ordinary direct-connector check"
    );
    assert!(
        !BuildingWalkingAccess::at_entrance(
            &allocator.entrances[0],
            &network,
            &graph,
            &reachability
        )
        .proves_to(BuildingWalkingAccess::at_entrance(
            &allocator.entrances[3],
            &network,
            &graph,
            &reachability
        ))
    );
    assert!(building_origin_trip_is_feasible(
        0, 3, 0, false, &allocator, &network, &graph, &queries
    ));
    for lane in &mut network.lane_system.lanes {
        if lane.lane_type == LaneType::Foot && lane.edge_id == usize::MAX {
            lane.next_lanes.clear();
        }
    }
    let mut disconnected = PedestrianReachability::build(&network, &graph);
    let origin = BuildingTripEnds::origin(0, &allocator, &network, &graph);
    let destination = BuildingTripEnds::destination(3, &allocator, &network, &graph);
    assert!(
        !origin.has_proven_walk_to(&destination, &disconnected),
        "weak road connectivity is insufficient after pedestrian connectors disappear"
    );
    assert!(!origin.can_reach(
        &destination,
        false,
        &network,
        &graph,
        &mut disconnected,
        &mut query
    ));
    assert!(!building_origin_trip_is_feasible(
        0, 3, 0, false, &allocator, &network, &graph, &queries
    ));
}

#[test]
fn live_consumer_funding_rejects_unreachable_sales_and_releases_failed_capital_claims() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let (mut graph, mut network) = simple_work_graph();
    let a = graph.add_node(Vector3::new(900.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(1300.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(make_road_edge(a, b, 900.0, 1300.0));
    graph.rebuild_adjacency_list();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "live_customer_home",
        ZoneClass::Residential,
    );
    let shop = register_test_asset(
        &mut allocator,
        "test",
        "live_customer_shop",
        ZoneClass::Commercial,
    );
    let profile = catalog.profile_for_id("personal_service_small").unwrap();
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    for x in [40.0, 80.0] {
        let mut business = make_building(x, ZoneType::Commercial, &shop, 0.0);
        business.economy_profile_runtime_id = profile.runtime_id;
        business.operating_budget = 100_000.0;
        allocator.buildings.push(business);
    }
    let mut remote = make_building(1000.0, ZoneType::Residential, &home, 0.0);
    remote.edge_idx = 1;
    allocator.buildings.push(remote);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut families = vec![make_household(3, 200, 0.0, 0.0)];
    families[0].gross_daily_wages = 100_000.0;
    families[0].employed_adults = 1;
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    funding.refresh(
        &allocator,
        &families,
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
        &funding.capacities[1..3],
        &[0, 0],
        "a rich disconnected community cannot finance either local shop"
    );
    families[0].home_building_id = 0;
    allocator.buildings[1].operating_budget = 0.0;
    funding.refresh(
        &allocator,
        &families,
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
    assert_eq!(funding.capacities[1], 0);
    assert!(
        funding.capacities[2] > 0,
        "a failed capital check must release reached family claims"
    );
    allocator.buildings[1].operating_budget = 100_000.0;
    funding.refresh(
        &allocator,
        &families,
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
    assert!(funding.capacities[1] > 0);
    assert_eq!(
        funding.capacities[2], 0,
        "the same household need cannot fund both shops after the first commits its capacity"
    );
}

#[test]
fn household_wage_snapshot_counts_only_active_paid_adult_assignments() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "test", "wage_home", ZoneClass::Residential);
    let factory = register_test_asset(
        &mut allocator,
        "test",
        "wage_factory",
        ZoneClass::Industrial,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    allocator
        .buildings
        .push(make_building(20.0, ZoneType::Industrial, &factory, 0.0));
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 2, 0.0, 0.0));
    let mut agents = AgentSystem::new();
    for _ in 0..2 {
        let member = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.assign_household_id(member, 0);
        agents.assign_work_building(member, 1, 0);
    }
    households.recount_every_household(&agents, &allocator);
    assert_eq!(households.households[0].employed_adults, 2);
    assert_eq!(
        households.households[0].gross_daily_wages,
        2.0 * profile.average_daily_wage()
    );
    agents.consecutive_unpaid_days[0] = 1;
    households.recount_every_household(&agents, &allocator);
    assert_eq!(
        households.households[0].employed_adults, 2,
        "an unpaid worker is still assigned"
    );
    assert_eq!(
        households.households[0].gross_daily_wages,
        profile.average_daily_wage()
    );
    allocator.buildings[1].set_construction_hours_left(1);
    households.recount_every_household(&agents, &allocator);
    assert_eq!(households.households[0].gross_daily_wages, 0.0);
    assert_eq!(households.households[0].employed_adults, 2);
}

#[test]
fn shopping_car_census_uses_eligible_residents_and_resets_removed_access() {
    use crate::simulation::economy::agents::{AGE_ADULT, AGE_CHILD, AGE_ELDER};
    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(usize::MAX, 2, 0.0, 0.0));
    let allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();
    for age in [AGE_ADULT, AGE_CHILD] {
        let index = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        agents.assign_household_id(index, 0);
        agents.age_group[index] = age;
        agents.has_car[index] = age == AGE_CHILD;
    }
    households.recount_every_household(&agents, &allocator);
    assert!(
        !households.households[0].shopping_has_car,
        "a child's car cannot supply an eligible household shopper"
    );
    agents.has_car[0] = true;
    households.recount_every_household(&agents, &allocator);
    assert!(households.households[0].shopping_has_car);
    agents.age_group[0] = AGE_ELDER;
    households.households[0].shopping_has_car = false;
    households.rebuild_household_shopping_access(&agents);
    assert!(
        households.households[0].shopping_has_car,
        "the load census must include elders without relying on saved household flags"
    );
    agents.assign_household_id(0, usize::MAX);
    households.rebuild_household_shopping_access(&agents);
    assert!(!households.households[0].shopping_has_car);
    households.households[0].shopping_has_car = true;
    households.recount_every_household(&agents, &allocator);
    assert!(
        !households.households[0].shopping_has_car,
        "a removed resident cannot leave cached car access behind"
    );
}

#[test]
#[ignore = "manual release scaling of recurring household income and purchasing forecasts"]
fn benchmark_consumer_income_forecast() {
    use std::hint::black_box;
    use std::time::Instant;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "income_bench_home",
        ZoneClass::Residential,
    );
    let factory = register_test_asset(
        &mut allocator,
        "test",
        "income_bench_work",
        ZoneClass::Industrial,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    allocator
        .buildings
        .push(make_building(20.0, ZoneType::Industrial, &factory, 0.0));
    // Isolate indexed aggregation: repeated references avoid gameplay placement/routing setup.
    for count in [1_024, 8_192, 65_536] {
        let mut households = HouseholdSystem::new();
        households.households = vec![make_household(0, 2, 0.0, 0.0); count];
        let mut agents = AgentSystem::new();
        for hid in 0..count {
            for _ in 0..2 {
                let index = agents.spawn_housed_agent(0, 0.0, 0.0);
                agents.assign_household_id(index, hid);
                agents.assign_work_building(index, 1, 0);
            }
        }
        let mut samples = [0.0; 9];
        for round in 0..12 {
            let start = Instant::now();
            households.recount_every_household(black_box(&agents), &allocator);
            let buyers = affordable_consumer_residents(
                &households.households,
                &allocator,
                &catalog,
                &tuning,
                &policy,
                100_000.0,
            );
            assert_eq!(black_box(buyers), (count * 2) as f64);
            if round >= 3 {
                samples[round - 3] = start.elapsed().as_secs_f64() * 1_000.0;
            }
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "consumer_income_forecast households={count} agents={} median_ms={:.3}",
            count * 2,
            samples[4]
        );
    }
}

mod markets;

#[test]
fn workplace_quotes_expire_a_day_of_passes_after_they_were_added() {
    let mut households = HouseholdSystem::new();
    households.restore_workplace_route_cache(
        0,
        0,
        0,
        vec![((0, 1, false), Some(60), 0), ((1, 0, true), None, 3)],
        3,
    );
    for _ in 0..20 {
        households.expire_workplace_route_cache();
    }
    assert_eq!(households.workplace_route_cache_entries().count(), 2);
    households.expire_workplace_route_cache();
    let left: Vec<_> = households
        .workplace_route_cache_entries()
        .map(|entry| entry.0)
        .collect();
    assert_eq!(left, vec![(1, 0, true)]);
    for _ in 0..3 {
        households.expire_workplace_route_cache();
    }
    assert_eq!(households.workplace_route_cache_entries().count(), 0);
}

#[test]
fn employed_searchers_are_locked_again_when_no_job_is_open() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 5.0, 5.0));
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "test", "locked_res", ZoneClass::Residential);
    let commercial_asset =
        register_test_asset(&mut allocator, "test", "locked_com", ZoneClass::Commercial);
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    let (graph, network) = simple_work_graph();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(agent, 0);
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);
    allocator.buildings[1].worker_count = 1;
    assert_eq!(agents.job_lock_until_day[agent], agents.settlement_days);
    // No building has a funded job open, so the board is empty.
    households.assign_agent_workplaces_from_census(
        &mut agents,
        &mut allocator,
        &network,
        &graph,
        420,
    );
    assert_eq!(agents.work_building[agent], 1);
    assert!(agents.job_lock_until_day[agent] > agents.settlement_days);
}

#[test]
fn a_job_board_swept_over_an_hour_equals_one_built_at_once() {
    use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
    use crate::simulation::economy::households::JobBoardRoll;
    let catalog = load_runtime_economy_catalog().expect("catalog");
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "test", "board_com", ZoneClass::Commercial);
    for (index, (funded, workers)) in [(2, 0), (1, 1), (3, 1)].into_iter().enumerate() {
        let mut building = make_building(index as f32 * 30.0, ZoneType::Commercial, &asset, 0.0);
        building.funded_job_capacity = funded;
        building.worker_count = workers;
        allocator.buildings.push(building);
    }
    allocator
        .building_handles
        .register_appended(allocator.buildings.len());
    let mut swept = JobBoardRoll::default();
    for slot in SLOTS_PER_HOUR..2 * SLOTS_PER_HOUR {
        swept.advance(slot, &allocator, &catalog);
    }
    // A reader with no board for its hour builds one at once.
    let mut built = JobBoardRoll::default();
    built.members_for(2, &allocator, &catalog);
    let handles = &allocator.building_handles;
    assert_eq!(
        swept.members_for(2, &allocator, &catalog),
        [handles.handle(0), handles.handle(2)],
        "the full building has no open job"
    );
    assert_eq!(swept.saved(), built.saved());
    // A sweep that misses a slot is abandoned; its hour then builds a board at once.
    let mut missed = JobBoardRoll::default();
    for slot in (2 * SLOTS_PER_HOUR..3 * SLOTS_PER_HOUR).filter(|slot| slot % 100 != 7) {
        missed.advance(slot, &allocator, &catalog);
    }
    assert!(missed.saved().front.is_none() && missed.saved().sweep.is_none());
}
