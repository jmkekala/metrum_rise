// SPDX-License-Identifier: GPL-2.0-only

//! Populated mixed-market funding across independent freight and household catchments.

use super::*;
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::hiring::JobFunding;

const HOMES: usize = 50;
const FIRMS: usize = 5;
const DISTRICT: usize = HOMES + FIRMS;

struct MarketFixture {
    catalog: std::sync::Arc<RuntimeEconomyCatalog>,
    graph: RegionGraph,
    network: TransitNetwork,
    allocator: BuildingAllocator,
    families: Vec<Household>,
    funding: JobFunding,
    logistics: ShipmentSystem,
}

type DistrictRow = (u32, u32, Vec<(usize, u16, u64)>);

impl MarketFixture {
    fn new(regimes: &[u8]) -> Self {
        let catalog = load_runtime_economy_catalog().unwrap();
        let mut graph = RegionGraph::new();
        let mut allocator = BuildingAllocator::new();
        let home = register_test_asset(
            &mut allocator,
            "test",
            "market_home",
            ZoneClass::Residential,
        );
        let commercial =
            register_test_asset(&mut allocator, "test", "market_shop", ZoneClass::Commercial);
        let industrial = register_test_asset(
            &mut allocator,
            "test",
            "market_factory",
            ZoneClass::Industrial,
        );
        let farm =
            register_test_utility_asset(&mut allocator, "test", "market_farm", "grain_farm_basic");
        let mut families = Vec::new();
        for (district, &regime) in regimes.iter().enumerate() {
            let x = district as f32 * 1024.0;
            let a = graph.add_node(Vector3::new(x - 100.0, 0.0, 0.0), NodeType::Border);
            let b = graph.add_node(Vector3::new(x + 400.0, 0.0, 0.0), NodeType::Junction);
            let edge = graph.add_edge(make_road_edge(a, b, x - 100.0, x + 400.0));
            for household in 0..HOMES {
                let mut building = make_building(
                    x + household as f32 * 4.0,
                    ZoneType::Residential,
                    &home,
                    0.0,
                );
                building.edge_idx = edge;
                let mut family = make_household(allocator.buildings.len(), 2, 0.0, 0.0);
                family.gross_daily_wages = if regime == 3 {
                    0.0
                } else if regime == 1 {
                    60.0
                } else {
                    240.0
                };
                family.employed_adults = u16::from(regime != 3);
                family.shopping_has_car = household % 2 == 0;
                allocator.buildings.push(building);
                families.push(family);
            }
            for (firm, id) in [
                "grain_farm_basic",
                "food_processor_basic",
                "grocery_basic",
                "personal_service_small",
                "machinery_factory_basic",
            ]
            .into_iter()
            .enumerate()
            {
                let profile = catalog.profile_for_id(id).unwrap();
                let (zone, asset) = match firm {
                    0 => (ZoneType::None, &farm),
                    2 | 3 => (ZoneType::Commercial, &commercial),
                    _ => (ZoneType::Industrial, &industrial),
                };
                let mut building = make_building(x + 220.0 + firm as f32 * 20.0, zone, asset, 0.0);
                building.edge_idx = edge;
                building.economy_profile_runtime_id = profile.runtime_id;
                building.work_area_scale = if firm == 0 { 10.0 } else { 1.0 };
                building.worker_count = physical_worker_capacity_for_profile(&building, profile);
                building.operating_budget = if regime == 2 && firm == 1 {
                    0.0
                } else {
                    100_000.0
                };
                allocator.buildings.push(building);
            }
        }
        graph.rebuild_adjacency_list();
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = CchGraph::build(&graph);
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        Self {
            catalog,
            graph,
            network,
            allocator,
            families,
            funding: JobFunding::default(),
            logistics: ShipmentSystem::new(),
        }
    }

    fn refresh(&mut self, tuning: &RuntimeEconomyTuning) {
        self.funding.refresh(
            &self.allocator,
            &self.families,
            &mut self.logistics,
            &self.network,
            &self.graph,
            &self.catalog,
            tuning,
            &CityFiscalPolicy::from_runtime_tuning(tuning),
            &[],
            0.0,
            420,
        );
    }

    // `refresh` run in slices of `budget` rows, restarting after `restart_after` slices.
    fn refresh_in_slices(
        &mut self,
        tuning: &RuntimeEconomyTuning,
        budget: usize,
        restart_after: Option<usize>,
    ) -> usize {
        self.funding.refresh_in_slices_for_test(
            &self.allocator,
            &self.families,
            &mut self.logistics,
            &self.network,
            &self.graph,
            &self.catalog,
            tuning,
            &CityFiscalPolicy::from_runtime_tuning(tuning),
            &[],
            0.0,
            420,
            budget,
            restart_after,
        )
    }

    // Every census output and book, bit for bit.
    fn assert_same_census(&self, other: &Self, label: &str) {
        self.funding.assert_same_operations_for_test(&other.funding);
        assert_eq!(
            self.funding.service_capacities, other.funding.service_capacities,
            "{label}"
        );
        assert_eq!(
            self.funding.freight_capital, other.funding.freight_capital,
            "{label}"
        );
        assert_eq!(
            self.funding.closure_passes, other.funding.closure_passes,
            "{label}"
        );
        // Each fixture builds its own network, whose revisions differ.
        assert_eq!(
            self.funding.census_book_fingerprints_for_test(),
            other.funding.census_book_fingerprints_for_test(),
            "{label}"
        );
    }

    fn district(&self, district: usize) -> Vec<DistrictRow> {
        let first = district * DISTRICT;
        (first + HOMES..first + DISTRICT)
            .map(|source| {
                let building = &self.allocator.buildings[source];
                let profile = self
                    .catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .unwrap();
                let activity = self
                    .funding
                    .input_demand_scale(source, building, profile, false);
                let mut deliveries = Vec::new();
                self.funding
                    .input_supply
                    .visit_dependents(source, |buyer, resource, units| {
                        assert!(
                            (first..first + DISTRICT).contains(&buyer),
                            "a disconnected buyer cannot reserve this district's output"
                        );
                        deliveries.push((buyer - first, resource, units.to_bits()));
                    });
                deliveries.sort_unstable();
                (
                    self.funding.capacities[source],
                    activity.to_bits(),
                    deliveries,
                )
            })
            .collect()
    }

    fn assert_same_funding(&self, other: &Self) {
        assert_eq!(self.funding.capacities, other.funding.capacities);
        self.funding
            .customers
            .assert_same_accounts_for_test(&other.funding.customers);
        for district in 0..self.allocator.buildings.len() / DISTRICT {
            assert_eq!(self.district(district), other.district(district));
            for owner in district * DISTRICT + HOMES..(district + 1) * DISTRICT {
                assert_eq!(
                    self.funding.business.financing(owner).map(f64::to_bits),
                    other.funding.business.financing(owner).map(f64::to_bits)
                );
            }
        }
    }
}

#[test]
fn production_market_chunks_match_full_census_after_input_changes() {
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut cached = MarketFixture::new(&[0, 1, 2, 3, 0, 1]);
    let mut full = MarketFixture::new(&[0, 1, 2, 3, 0, 1]);
    let count = cached.allocator.buildings.len();
    for market in [&mut cached, &mut full] {
        market.allocator.prepare_production_changes();
    }
    for step in 0..9 {
        for market in [&mut cached, &mut full] {
            match step {
                2 => market.allocator.set_worker_count(HOMES + 1, 0),
                3 => market.allocator.set_worker_count(HOMES + 1, 3),
                4 => market.allocator.set_work_area_scale(HOMES, 2.0),
                5 => market.allocator.invalidate_production_changes(),
                6 => market.catalog = std::sync::Arc::new((*market.catalog).clone()),
                7 => market.allocator.bump_building_ref_revision(),
                8 => {
                    let authored = include_str!("../../../../../../../economy/profiles.toml");
                    market.catalog = std::sync::Arc::new(
                        crate::simulation::economy::definitions::compile_test_catalog(authored)
                            .unwrap(),
                    );
                }
                _ => {}
            }
        }
        full.funding.invalidate_market_for_test();
        cached.refresh(&tuning);
        full.refresh(&tuning);
        cached.assert_same_funding(&full);
        cached.funding.assert_same_market_for_test(&full.funding);
        let expected = match step {
            1 | 6 => 0,
            2..=4 => 256,
            _ => count,
        };
        assert_eq!(
            cached.funding.market_rows_for_test(),
            expected,
            "step {step}"
        );
        assert_eq!(full.funding.market_rows_for_test(), count);
    }
}

#[test]
fn import_plan_handoff_matches_cold_funding_after_input_changes() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    let mut cached = MarketFixture::new(&[0, 1, 2, 3]);
    let mut cold = MarketFixture::new(&[0, 1, 2, 3]);
    cached.refresh(&tuning);
    let source = std::mem::take(&mut cached.funding);
    let mut hits = 0;
    for step in 0..8 {
        if step == 4 {
            tuning.owa_import_price_multiplier = 1.25;
        }
        for market in [&mut cached, &mut cold] {
            match step {
                1 => market.allocator.buildings[HOMES + 1].operating_budget += 10_000.0,
                2 => market.allocator.buildings[HOMES + 1].operating_budget = 1.0,
                3 => market.families[0].gross_daily_wages = 0.0,
                5 => market.allocator.set_work_area_scale(HOMES, 2.0),
                6 => market.catalog = std::sync::Arc::new((*market.catalog).clone()),
                7 => {
                    let authored = include_str!("../../../../../../../economy/profiles.toml");
                    market.catalog = std::sync::Arc::new(
                        crate::simulation::economy::definitions::compile_test_catalog(authored)
                            .unwrap(),
                    );
                }
                _ => {}
            }
        }
        cached.funding.refresh_with_service_capacity(
            &cached.allocator,
            &cached.families,
            &mut cached.logistics,
            &cached.network,
            &cached.graph,
            &cached.catalog,
            &tuning,
            &CityFiscalPolicy::from_runtime_tuning(&tuning),
            0.0,
            |_, _, _| 0,
            Some(&source),
            420,
        );
        cold.refresh(&tuning);
        cached.assert_same_funding(&cold);
        cached
            .funding
            .assert_same_operations_for_test(&cold.funding);
        hits += cached.funding.import_plan_replays();
        if step == 7 {
            assert_eq!(cached.funding.import_plan_replays(), 0);
        }
    }
    assert!(hits > 0, "fixture must exercise the import plan handoff");
}

#[test]
fn import_quote_retries_preserve_funding_and_reset_after_input_changes() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_export_price_multiplier = 0.0;
    let mut cached = MarketFixture::new(&[0, 1, 2, 3]);
    let mut ordinary = MarketFixture::new(&[0, 1, 2, 3]);
    ordinary.funding.disable_import_quote_replay();
    let mut reused = 0;
    for (income, imports, capital) in [
        (240.0, 1.0, 100_000.0),
        (0.0, 10.0, 0.0),
        (60.0, 1.25, 2_000.0),
        (240.0, 1.0, 100_000.0),
    ] {
        tuning.owa_import_price_multiplier = imports;
        for market in [&mut cached, &mut ordinary] {
            market.families[0].gross_daily_wages = income;
            market.allocator.buildings[HOMES + 1].operating_budget = capital;
            market.refresh(&tuning);
        }
        cached.assert_same_funding(&ordinary);
        reused += cached.funding.import_quote_replays();
        assert_eq!(ordinary.funding.import_quote_replays(), 0);
    }
    assert!(reused > 0, "fixture must exercise import quotation reuse");
}

#[test]
fn disconnected_populated_markets_keep_local_financing_and_customer_claims() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_export_price_multiplier = 0.0;
    let expected: Vec<_> = (0..4)
        .map(|regime| {
            let mut market = MarketFixture::new(&[regime]);
            market.refresh(&tuning);
            let result = market.district(0);
            market.refresh(&tuning);
            assert_eq!(market.district(0), result, "warm replay, regime {regime}");
            result
        })
        .collect();
    assert!(
        expected[0][2].0 > 0,
        "funded families must support a grocery"
    );
    assert_eq!(
        expected[2][1].0, 0,
        "staffing cannot replace processor financing"
    );
    assert!(
        expected[3].iter().all(|firm| firm.0 == 0),
        "unfunded demand cannot sustain a chain"
    );
    for regimes in [
        vec![0, 1, 2, 3],
        vec![3, 2, 1, 0],
        (0..32).map(|i| (i % 4) as u8).collect(),
    ] {
        let mut market = MarketFixture::new(&regimes);
        for _ in 0..2 {
            market.refresh(&tuning);
            for (district, &regime) in regimes.iter().enumerate() {
                assert_eq!(
                    market.district(district),
                    expected[regime as usize],
                    "district {district}, regime {regime}, {} disconnected markets",
                    regimes.len()
                );
            }
        }
    }
}

#[test]
fn rejected_direct_household_quotes_restage_before_acceptance() {
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut cached = MarketFixture::new(&[0]);
    let mut ordinary = MarketFixture::new(&[0]);
    ordinary.funding.disable_household_quote_replay();
    let mut policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    policy.commercial_property_tax_per_building_per_day = 100.0;
    policy.industrial_property_tax_per_building_per_day = 100.0;
    for market in [&mut cached, &mut ordinary] {
        let profile = market
            .catalog
            .profile_for_id("personal_service_small")
            .unwrap();
        for index in HOMES + 1..DISTRICT {
            let building = &mut market.allocator.buildings[index];
            building.economy_profile_runtime_id = profile.runtime_id;
            building.zone_type = ZoneType::Commercial;
            building.worker_count = profile.worker_capacity;
            building.operating_budget = if index == HOMES + 3 { 100_000.0 } else { 100.0 };
        }
        market.allocator.bump_building_ref_revision();
        market.allocator.rebuild_zone_index();
        market.funding.refresh(
            &market.allocator,
            &market.families,
            &mut market.logistics,
            &market.network,
            &market.graph,
            &market.catalog,
            &tuning,
            &policy,
            &[],
            0.0,
            420,
        );
    }
    cached.assert_same_funding(&ordinary);
    cached
        .funding
        .assert_same_operations_for_test(&ordinary.funding);
    let (reused, accepted) = cached.funding.household_rejected_quote_counts();
    assert!(reused > 0, "fixture must reuse a rejected spatial question");
    assert!(
        accepted > 0,
        "a later solvent shop must materialize the reused quote"
    );
    assert_eq!(ordinary.funding.household_rejected_quote_counts(), (0, 0));
}

#[test]
fn household_quote_prefix_reuse_matches_ordinary_requotation_through_market_changes() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_export_price_multiplier = 0.0;
    let regimes: Vec<_> = (0..16).map(|index| (index % 4) as u8).collect();
    let mut cached = MarketFixture::new(&regimes);
    let mut ordinary = MarketFixture::new(&regimes);
    ordinary.funding.disable_household_quote_replay();
    let mut reused = 0;
    for (cash, income, delay) in [
        (100_000.0, 1.0, 0.0),
        (0.0, 1.0, 0.0),
        (100_000.0, 0.25, 40.0),
        (100_000.0, 0.0, 0.0),
        (100_000.0, 1.0, 0.0),
    ] {
        for market in [&mut cached, &mut ordinary] {
            for (district, &regime) in regimes.iter().enumerate() {
                market.allocator.buildings[district * DISTRICT + HOMES + 1].operating_budget =
                    if regime == 2 { 0.0 } else { cash };
                for household in &mut market.families[district * HOMES..(district + 1) * HOMES] {
                    household.gross_daily_wages =
                        [240.0, 60.0, 240.0, 0.0][regime as usize] * income;
                }
            }
            for lane in &mut market.network.lane_system.lanes {
                lane.frontage_delay_penalty_s = delay;
            }
            market.network.frontage_delay_revision += 1;
            market.refresh(&tuning);
        }
        reused += cached.funding.household_quote_replay_counts().0;
        assert_eq!(ordinary.funding.household_quote_replay_counts().0, 0);
        for district in 0..regimes.len() {
            assert_eq!(cached.district(district), ordinary.district(district));
            for owner in district * DISTRICT + HOMES..(district + 1) * DISTRICT {
                assert_eq!(
                    cached.funding.business.financing(owner).map(f64::to_bits),
                    ordinary.funding.business.financing(owner).map(f64::to_bits)
                );
                let profile = cached
                    .catalog
                    .profile_by_runtime_id(
                        cached.allocator.buildings[owner].economy_profile_runtime_id,
                    )
                    .unwrap();
                for output in &profile.outputs {
                    assert_eq!(
                        cached
                            .funding
                            .customers
                            .reserved_units(owner, output.resource_runtime_id)
                            .to_bits(),
                        ordinary
                            .funding
                            .customers
                            .reserved_units(owner, output.resource_runtime_id)
                            .to_bits()
                    );
                }
            }
        }
    }
    assert!(
        reused > 0,
        "the comparison must exercise unchanged quote prefixes"
    );
}

#[test]
fn household_cross_census_reuse_matches_fresh_claims_after_each_input_changes() {
    use crate::simulation::economy::definitions::compile_test_catalog;

    let authored = include_str!("../../../../../../../economy/profiles.toml");
    let start = authored
        .find("[[profiles]]\nid = \"grocery_basic\"")
        .unwrap();
    let end = start + authored[start + 1..].find("\n[[profiles]]").unwrap() + 1;
    let alternate = authored[start..end]
        .replace("grocery_basic", "expensive_grocery")
        .replace("unit_price_currency = 25.0", "unit_price_currency = 37.0");
    let alternate_catalog =
        std::sync::Arc::new(compile_test_catalog(&format!("{authored}\n{alternate}")).unwrap());
    let mut replayed = 0;
    for change in [
        "unchanged",
        "income",
        "cash",
        "members",
        "unhoused",
        "empty",
        "home",
        "car",
        "adults",
        "price",
        "quantity",
        "basket",
        "vat",
        "tax",
        "treasury",
        "frontage",
        "frontage_revision",
        "entrance_revision",
        "seller_position",
        "home_position",
        "home_local_position",
        "unrelated_building",
        "topology",
        "door",
        "removed_family",
        "new_family",
        "clear",
    ] {
        let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
        tuning.owa_export_price_multiplier = 0.0;
        let mut policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
        let mut treasury = 0.0;
        let mut cached = MarketFixture::new(&[0, 1]);
        let mut ordinary = MarketFixture::new(&[0, 1]);
        ordinary.funding.disable_household_quote_replay();
        for market in [&mut cached, &mut ordinary] {
            market.catalog = alternate_catalog.clone();
            market.families[1].child_count = 1;
            market.families[1].member_count += 1;
            market.refresh(&tuning);
            market.refresh(&tuning);
        }
        cached.assert_same_funding(&ordinary);
        replayed += cached.funding.household_cross_census_replays();
        let mut fork = cached.funding.customers.clone();
        fork.prepare_project_owners(1_024, 1_024);
        let retained_capacity = fork.receipt_capacity_for_test();
        for market in [&mut cached, &mut ordinary] {
            match change {
                "income" => market.families[0].gross_daily_wages = 0.0,
                "cash" => market.allocator.buildings[HOMES + 2].operating_budget = 0.0,
                "members" => market.families[0].member_count = 1,
                "unhoused" => market.families[0].home_building_id = usize::MAX,
                "empty" => market.families[0].member_count = 0,
                "home" => market.families[0].home_building_id = DISTRICT,
                "car" => market.families[0].shopping_has_car = !market.families[0].shopping_has_car,
                "adults" => {
                    market.families[0].adult_count = 0;
                    market.families[0].elder_count = 0;
                }
                "price" => {
                    market.allocator.buildings[HOMES + 2].economy_profile_runtime_id = market
                        .catalog
                        .profile_for_id("expensive_grocery")
                        .unwrap()
                        .runtime_id
                }
                "quantity" => {
                    market.catalog = std::sync::Arc::new(
                        compile_test_catalog(&format!(
                            "{}\n{alternate}",
                            authored.replace(
                                &authored[start..end],
                                &authored[start..end]
                                    .replace("units_per_day = 200.0", "units_per_day = 100.0")
                            )
                        ))
                        .unwrap(),
                    )
                }
                "basket" => {
                    market.catalog = std::sync::Arc::new(
                        compile_test_catalog(&format!("{authored}\n{alternate}").replace(
                            "consumption_rate_per_resident = 1.0",
                            "consumption_rate_per_resident = 0.5",
                        ))
                        .unwrap(),
                    )
                }
                "frontage" => {
                    for lane in &mut market.network.lane_system.lanes {
                        lane.frontage_delay_penalty_s = 1_000_000.0;
                    }
                    market.network.frontage_delay_revision += 1;
                }
                "frontage_revision" => market.network.frontage_delay_revision += 1,
                "entrance_revision" => market.allocator.bump_entrance_ref_revision(),
                "seller_position" => {
                    market.allocator.entrances[HOMES + 2].door_pos.x += 512.0;
                    market.allocator.bump_entrance_ref_revision();
                }
                "home_position" | "home_local_position" => {
                    let distance = if change == "home_position" {
                        512.0
                    } else {
                        0.25
                    };
                    market.allocator.buildings[0].center_x += distance;
                    market.allocator.entrances[0].door_pos.x += distance;
                    market.allocator.bump_entrance_ref_revision();
                }
                "unrelated_building" => {
                    // An unoccupied home adds no recipe output or family budget. Existing
                    // seller entrances and every household's indexed membership stay exact.
                    market
                        .allocator
                        .buildings
                        .push(market.allocator.buildings[0].clone());
                    market.allocator.rebuild_zone_index();
                    market
                        .allocator
                        .rebuild_entrance_cache(&market.graph, &market.network.lane_system);
                }
                "topology" => {
                    market.graph.edge_mut(0).allowed_types = TransitFlags::CAR;
                    market.graph.rebuild_adjacency_list();
                    market.network.lane_system.rebuild(&mut market.graph);
                    market.network.rebuild_cch_and_check(&market.graph);
                    market
                        .allocator
                        .rebuild_entrance_cache(&market.graph, &market.network.lane_system);
                }
                "door" => {
                    market.allocator.buildings[HOMES + 2].edge_idx = usize::MAX;
                    market
                        .allocator
                        .rebuild_entrance_cache(&market.graph, &market.network.lane_system);
                }
                "removed_family" => {
                    market.families.pop();
                }
                "new_family" => market.families.push(market.families[0].clone()),
                "clear" => market.funding.clear(),
                "unchanged" | "vat" | "tax" | "treasury" => {}
                _ => unreachable!(),
            }
        }
        match change {
            "vat" => policy.household_vat_rate = 0.5,
            "tax" => policy.income_tax_rate = 0.5,
            "treasury" => treasury = 10_000.0,
            _ => {}
        }
        for round in 0..3 {
            let resamples_before = cached.funding.customers.access_resamples_for_test();
            // The oracle rebuilds membership too; shared index reuse cannot conceal a stale
            // reservation position after a home, component, or entrance change.
            ordinary.funding.customers.clear();
            for market in [&mut cached, &mut ordinary] {
                // A prefix can have changed after the prior census. Preserve dependencies
                // of rejected quotes too when traffic publishes another revision.
                if round == 2 {
                    market.network.frontage_delay_revision += 1;
                }
                market.funding.refresh(
                    &market.allocator,
                    &market.families,
                    &mut market.logistics,
                    &market.network,
                    &market.graph,
                    &market.catalog,
                    &tuning,
                    &policy,
                    &[],
                    treasury,
                    420,
                );
            }
            cached.assert_same_funding(&ordinary);
            // Forking after each changed input must match an independent deep copy,
            // including candidate order, exact balances, accepted receipts and rollback.
            fork.copy_reservations_from(&cached.funding.customers);
            assert!(fork.receipt_capacity_for_test() >= retained_capacity);
            let mut fresh = cached.funding.customers.clone();
            for book in [&mut fork, &mut fresh] {
                book.prepare_project_owners(1_024, 1_024);
            }
            fork.assert_same_accounts_for_test(&fresh);
            let seller = HOMES + 2;
            let profile = cached
                .catalog
                .profile_by_runtime_id(
                    cached.allocator.buildings[seller].economy_profile_runtime_id,
                )
                .unwrap();
            for requested in [1.0, 40.0, 10_000.0] {
                for book in [&mut fork, &mut fresh] {
                    book.begin_revalidation();
                    book.release_owner(seller);
                }
                for output in &profile.outputs {
                    let quote =
                        |book: &mut crate::simulation::economy::customers::ConsumerMarket| {
                            book.claim_reachable(
                                &cached.allocator.entrances[seller],
                                output.resource_runtime_id,
                                requested,
                                profile.unit_price_currency * (1.0 + policy.household_vat_rate),
                                &cached.allocator,
                                &cached.network,
                                &cached.graph,
                            )
                        };
                    assert_eq!(
                        quote(&mut fork).to_bits(),
                        quote(&mut fresh).to_bits(),
                        "fork quote differs: {change}, round={round}"
                    );
                }
                for book in [&mut fork, &mut fresh] {
                    book.accept_for(seller);
                }
                fork.assert_same_accounts_for_test(&fresh);
                for book in [&mut fork, &mut fresh] {
                    book.reject_revalidation();
                }
                fork.assert_same_accounts_for_test(&fresh);
            }
            if matches!(
                change,
                "entrance_revision"
                    | "seller_position"
                    | "unrelated_building"
                    | "home_position"
                    | "home_local_position"
                    | "car"
            ) {
                let expected = u64::from(
                    round == 0 && matches!(change, "home_position" | "home_local_position"),
                );
                assert_eq!(
                    cached.funding.customers.access_resamples_for_test() - resamples_before,
                    expected,
                    "only changed household geometry needs reconstruction: {change}, round={round}"
                );
            }
            assert_eq!(ordinary.funding.household_cross_census_replays(), 0);
            if matches!(
                change,
                "frontage_revision"
                    | "entrance_revision"
                    | "unrelated_building"
                    | "home_local_position"
            ) {
                assert!(
                    cached.funding.household_cross_census_replays() > 0,
                    "unchanged walking-proven claims survive unrelated revisions: {change}"
                );
            }
            if round == 0 && change == "seller_position" {
                assert!(
                    cached.funding.household_quote_replay_counts().1 > 0,
                    "a changed seller entrance must reconstruct its claim prefix"
                );
            }
        }
    }
    assert!(
        replayed > 0,
        "unchanged censuses must actually exercise retained claims"
    );
}

#[test]
fn freight_connectivity_and_frontage_prices_revalidate_populated_markets() {
    use crate::simulation::economy::definitions::compile_test_catalog;
    use crate::simulation::economy::logistics::freight_goods_multiplier;

    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 1.2;
    tuning.owa_export_price_multiplier = 0.0;
    let authored = include_str!("../../../../../../../economy/profiles.toml");
    let start = authored
        .find("[[profiles]]\nid = \"food_processor_basic\"")
        .unwrap();
    let end = start + authored[start + 1..].find("\n[[profiles]]").unwrap() + 1;
    let regional = authored[start..end]
        .replace("food_processor_basic", "regional_food")
        .replace("unit_price_currency = 12.0", "unit_price_currency = 13.0");
    let authored = authored.replace(
        "id = \"packaged_food\"\nquantity_unit = \"kg\"",
        "id = \"packaged_food\"\nquantity_unit = \"kg\"\nimport_unit_price_currency = 12.0",
    );
    let mut market = MarketFixture::new(&[0, 2]);
    market.catalog =
        std::sync::Arc::new(compile_test_catalog(&format!("{authored}\n{regional}")).unwrap());
    let factory = HOMES + 1;
    let distant_shop = DISTRICT + HOMES + 2;
    let regional = market.catalog.profile_for_id("regional_food").unwrap();
    let food = market
        .catalog
        .resource_runtime_id_for_id("packaged_food")
        .unwrap();
    assert_ne!(
        regional.unit_price_currency,
        market.catalog.unit_price_for_resource(food).unwrap()
    );
    market.allocator.buildings[factory].economy_profile_runtime_id = regional.runtime_id;
    // This bridge changes freight reachability; pedestrian catchments retain their own demand.
    for family in &mut market.families {
        family.shopping_has_car = false;
    }
    market.refresh(&tuning);
    let isolated = [market.district(0), market.district(1)];
    assert!(market.funding.capacities[factory] > 0);

    let mut bridge = make_road_edge(1, 2, 400.0, 924.0);
    bridge.allowed_types = TransitFlags::CAR;
    let bridge = market.graph.add_edge(bridge);
    let rebuild = |market: &mut MarketFixture| {
        market.graph.rebuild_adjacency_list();
        market.network.lane_system.rebuild(&mut market.graph);
        market.network.rebuild_cch_and_check(&market.graph);
        market
            .allocator
            .rebuild_entrance_cache(&market.graph, &market.network.lane_system);
    };
    rebuild(&mut market);
    market.refresh(&tuning);
    let carrier = market.logistics.customer_carrier_quote(
        &market.allocator.entrances[factory],
        &market.allocator.entrances[distant_shop],
        Some(factory),
        Some(distant_shop),
        &market.allocator,
        &market.network,
        &market.graph,
        &tuning,
    );
    assert!(
        carrier.is_some(),
        "the connected fixture must have a legal directed freight route"
    );
    assert!(
        market.funding.capacities[factory] > isolated[0][1].0,
        "reachable financed buyers must support additional output: isolated={:?}, connected={:?}/{:?}, sales={:?}, carrier={carrier:?}",
        isolated,
        &market.funding.capacities[HOMES..DISTRICT],
        &market.funding.capacities[DISTRICT + HOMES..2 * DISTRICT],
        market.funding.business.sales(factory, food, 200.0)
    );
    let mut cross_district = 0.0;
    market
        .funding
        .input_supply
        .visit_dependents(factory, |buyer, resource, units| {
            if buyer == distant_shop && resource == food {
                cross_district += units;
            }
        });
    assert_eq!(cross_district, 100.0);
    let grocery = market.catalog.profile_for_id("grocery_basic").unwrap();
    let receiving = freight_goods_multiplier(grocery, ZoneType::Commercial, &tuning, 420);
    let sales = market.funding.business.sales(factory, food, 200.0);
    assert_eq!(sales.units, 200.0);
    assert_eq!(
        sales.revenue,
        200.0 * f64::from(13.0 * receiving),
        "seller receipts use actual goods prices and exclude carrier charges"
    );
    let capacities = market.funding.capacities.clone();
    let finance = market.funding.business.financing(distant_shop);
    for delay in [40.0, 1_000_000.0, 0.0] {
        for lane in &mut market.network.lane_system.lanes {
            lane.frontage_delay_penalty_s = delay;
        }
        market.network.frontage_delay_revision += 1;
        market.refresh(&tuning);
        if delay == 40.0 {
            assert!(
                market.funding.business.financing(distant_shop)[0] < finance[0],
                "unchanged geometry must not preserve an obsolete freight quote"
            );
        } else if delay > 0.0 {
            assert_eq!(market.funding.capacities[factory], 0);
            assert_eq!(market.funding.capacities[distant_shop], 0);
            assert_eq!(
                market.funding.business.sales(factory, food, 200.0).units,
                0.0
            );
        } else {
            assert_eq!(market.funding.capacities, capacities);
            assert_eq!(
                market
                    .funding
                    .business
                    .financing(distant_shop)
                    .map(f64::to_bits),
                finance.map(f64::to_bits)
            );
            assert_eq!(market.funding.business.sales(factory, food, 200.0), sales);
        }
    }
    // A weak component remains connected when its vehicle direction is reversed.
    market.graph.edge_mut(bridge).fwd_lanes = 0;
    rebuild(&mut market);
    market.refresh(&tuning);
    assert_eq!(market.funding.capacities[factory], isolated[0][1].0);
    assert_eq!(
        market.funding.business.sales(factory, food, 200.0).units,
        100.0,
        "weak connectivity cannot retain a now-unreachable customer"
    );
    // Isolated subsystem topology mutation, followed by all dependent cache rebuilds.
    market.graph.edge_mut(bridge).deleted = true;
    rebuild(&mut market);
    market.refresh(&tuning);
    assert_eq!(
        [market.district(0), market.district(1)],
        isolated,
        "removing the freight connection must withdraw its customer and source commitments"
    );
}

#[test]
#[ignore = "manual release scaling of populated mixed-market funding; preparation measured separately"]
fn benchmark_populated_mixed_market_funding() {
    use std::time::Instant;
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_export_price_multiplier = 0.0;
    let mut reference = MarketFixture::new(&[0, 1, 2, 3]);
    reference.refresh(&tuning);
    let expected: Vec<_> = (0..4)
        .map(|district| reference.district(district))
        .collect();
    for districts in [4, 16, 64, 256, 1_024] {
        let regimes: Vec<_> = (0..districts).map(|index| (index % 4) as u8).collect();
        let setup = Instant::now();
        let mut market = MarketFixture::new(&regimes);
        let setup_ms = setup.elapsed().as_secs_f64() * 1_000.0;
        let first = Instant::now();
        market.refresh(&tuning);
        let first_ms = first.elapsed().as_secs_f64() * 1_000.0;
        let mut samples = [0.0_f64; 15];
        let mut passes = [0; 15];
        for round in 0..18 {
            let start = Instant::now();
            market.refresh(&tuning);
            let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
            if round >= 3 {
                samples[round - 3] = elapsed;
                passes[round - 3] = market.funding.closure_passes;
            }
        }
        // Validate every populated district outside the measurement, including exact deliveries.
        for (district, &regime) in regimes.iter().enumerate() {
            assert_eq!(market.district(district), expected[regime as usize]);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "mixed_market_funding districts={districts} households={} buildings={} setup_ms={setup_ms:.3} first_refresh_ms={first_ms:.3} warm_median_ms={:.3} warm_max_ms={:.3} closure_passes={passes:?}",
            market.families.len(),
            market.allocator.buildings.len(),
            samples[7],
            samples[14]
        );
    }
}

fn financing_chain(length: usize, background: usize) -> MarketFixture {
    financing_links(length, false, 200.0, None, background)
}

// Link i buys `units` of link i-1's goods and imports the rest of its 200 kg as feed. A cyclic
// market closes the loop with business goods only, so every link sells only to its successor;
// `units` below 200 gives each link a gain below one. Prices rise along the links unless one
// uniform price makes every link profitable at full activity.
fn financing_links(
    length: usize,
    cyclic: bool,
    units: f32,
    price: Option<u32>,
    background: usize,
) -> MarketFixture {
    use crate::simulation::economy::definitions::compile_test_catalog;
    use std::fmt::Write;
    assert!(length >= 2);
    let authored = include_str!("../../../../../../../economy/profiles.toml");
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
    let mut catalog = authored.to_owned();
    catalog.push_str("\n[[resources]]\nid = \"chain_feed\"\nquantity_unit = \"kg\"\nshipment_batch_kg = 50.0\nruntime_id = 10\nimport_unit_price_currency = 0.01\n");
    for index in 0..length - usize::from(!cyclic) {
        writeln!(catalog, "\n[[resources]]\nid = \"chain_goods_{index}\"\nquantity_unit = \"kg\"\nshipment_batch_kg = 50.0\nruntime_id = {}\nimport_unit_price_currency = {}.0", index + 11, 2 * (index + 1)).unwrap();
    }
    for index in 0..length {
        let input = if index > 0 {
            format!("chain_goods_{}", index - 1)
        } else if cyclic {
            format!("chain_goods_{}", length - 1)
        } else {
            "chain_feed".to_owned()
        };
        let output = if index + 1 == length && !cyclic {
            "household_supplies".to_owned()
        } else {
            format!("chain_goods_{index}")
        };
        let mut link = recipe
            .replace("food_processor_basic", &format!("chain_{index}"))
            .replace("\"grain\"", &format!("\"{input}\""))
            .replace("\"packaged_food\"", &format!("\"{output}\""));
        if cyclic {
            link = link.replace(&format!("resource = \"{input}\"\nunits_per_day = 200.0"), &format!(
                "resource = \"{input}\"\nunits_per_day = {units:?}\n\n[[profiles.inputs]]\nresource = \"chain_feed\"\nunits_per_day = {:?}",
                200.0 - units));
        }
        catalog.push_str(&link.replace(
            "unit_price_currency = 12.0",
            &format!(
                "unit_price_currency = {}.0",
                price.unwrap_or(2 * (index as u32 + 1))
            ),
        ));
    }
    let catalog = std::sync::Arc::new(compile_test_catalog(&catalog).unwrap());
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::ZERO, NodeType::Border);
    let b = graph.add_node(Vector3::new(1_000.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(make_road_edge(a, b, 0.0, 1_000.0));
    let mut allocator = BuildingAllocator::new();
    let factory = register_test_asset(
        &mut allocator,
        "test",
        "chain_factory",
        ZoneClass::Industrial,
    );
    let home = register_test_asset(&mut allocator, "test", "chain_home", ZoneClass::Residential);
    for index in 0..length {
        let mut building = make_building(
            20.0 + index as f32 * 4.0,
            ZoneType::Industrial,
            &factory,
            0.0,
        );
        building.economy_profile_runtime_id = catalog
            .profile_for_id(&format!("chain_{index}"))
            .unwrap()
            .runtime_id;
        building.worker_count = 1;
        building.operating_budget = 1_000_000.0;
        allocator.buildings.push(building);
    }
    for _ in 0..=background {
        allocator
            .buildings
            .push(make_building(10.0, ZoneType::Residential, &home, 0.0));
    }
    let mut family = make_household(length, 200, 0.0, 0.0);
    family.employed_adults = 200;
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    MarketFixture {
        catalog,
        graph,
        network,
        allocator,
        families: vec![family],
        funding: JobFunding::default(),
        logistics: ShipmentSystem::new(),
    }
}

#[test]
fn removed_buildings_invalidate_the_possible_buyer_layout_until_reseeded() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    let mut market = financing_chain(4, 1);
    market.families[0].gross_daily_wages = 1_000_000.0;
    market.families[0].budget = 1_000_000.0;
    market.refresh(&tuning);
    assert!(
        market
            .funding
            .business
            .layout_matches(&market.allocator, &market.network)
    );
    // A swap-removed last building leaves the census's layout naming a missing owner, which
    // crashed customer propagation in a 100k city when a pass ended before reseeding.
    market.allocator.buildings.pop();
    market.allocator.entrances.pop();
    market.allocator.bump_entrance_ref_revision();
    assert!(
        !market
            .funding
            .business
            .layout_matches(&market.allocator, &market.network)
    );
    market.families[0].gross_daily_wages = 0.0;
    market.families[0].budget = 0.0;
    market.refresh(&tuning);
    assert!(
        market
            .funding
            .business
            .layout_matches(&market.allocator, &market.network)
    );
    assert_eq!(&market.funding.capacities[..4], &[0; 4]);
}

#[test]
fn downstream_income_withdrawal_and_recovery_revalidate_the_complete_supplier_chain() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    for length in [2, 8, 32] {
        let mut market = financing_chain(length, 0);
        let mut replay = financing_chain(length, 0);
        replay.funding.disable_customer_cascade();
        for income in [0.0, 1_000_000.0, 0.0, 1_000_000.0] {
            market.families[0].gross_daily_wages = income;
            market.families[0].budget = income;
            replay.families[0] = market.families[0].clone();
            market.refresh(&tuning);
            replay.refresh(&tuning);
            assert!(
                market.funding.closure_passes <= 2,
                "zero-customer propagation must not replay the census for each supplier link"
            );
            assert_eq!(market.funding.capacities, replay.funding.capacities);
            market
                .funding
                .customers
                .assert_same_accounts_for_test(&replay.funding.customers);
            assert_eq!(
                &market.funding.capacities[..length],
                vec![u32::from(income > 0.0); length],
                "length={length}, income={income}"
            );
            for source in 0..length {
                assert_eq!(
                    market.funding.business.financing(source).map(f64::to_bits),
                    replay.funding.business.financing(source).map(f64::to_bits)
                );
                let mut deliveries = Vec::new();
                market
                    .funding
                    .input_supply
                    .visit_dependents(source, |buyer, _, units| deliveries.push((buyer, units)));
                assert_eq!(
                    deliveries,
                    if income > 0.0 && source + 1 < length {
                        vec![(source + 1, 200.0)]
                    } else {
                        vec![]
                    },
                    "source={source}, length={length}, income={income}"
                );
            }
        }
    }
}

#[test]
fn partial_downstream_income_caps_the_supplier_chain_without_a_census_per_link() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    // Incomes at which the end operation sells part of its output, then fails after its
    // partial sales have already capped the chain (end price scales with chain length).
    for (length, partial, failing) in [
        (2, 7_750.0, 7_500.0),
        (8, 11_250.0, 10_500.0),
        (32, 25_000.0, 24_500.0),
    ] {
        let mut market = financing_chain(length, 0);
        let mut replay = financing_chain(length, 0);
        replay.funding.disable_customer_cascade();
        for income in [1_000_000.0, partial, failing, partial, 1_000_000.0] {
            market.families[0].gross_daily_wages = income;
            market.families[0].budget = income;
            replay.families[0] = market.families[0].clone();
            market.refresh(&tuning);
            replay.refresh(&tuning);
            assert!(
                market.funding.closure_passes <= 3,
                "positive customer limits must not replay the census for each supplier link"
            );
            if income == partial {
                assert!(
                    replay.funding.closure_passes >= length,
                    "fixture must exercise the cascade"
                );
            }
            assert_eq!(market.funding.capacities, replay.funding.capacities);
            market
                .funding
                .customers
                .assert_same_accounts_for_test(&replay.funding.customers);
            for source in 0..length {
                let building = &market.allocator.buildings[source];
                let profile = market
                    .catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .unwrap();
                assert_eq!(
                    market
                        .funding
                        .input_demand_scale(source, building, profile, false)
                        .to_bits(),
                    replay
                        .funding
                        .input_demand_scale(source, building, profile, false)
                        .to_bits()
                );
                assert_eq!(
                    market.funding.business.financing(source).map(f64::to_bits),
                    replay.funding.business.financing(source).map(f64::to_bits)
                );
                let [mut deliveries, mut expected] = [Vec::new(), Vec::new()];
                market
                    .funding
                    .input_supply
                    .visit_dependents(source, |buyer, _, units| {
                        deliveries.push((buyer, units.to_bits()))
                    });
                replay
                    .funding
                    .input_supply
                    .visit_dependents(source, |buyer, _, units| {
                        expected.push((buyer, units.to_bits()))
                    });
                assert_eq!(
                    deliveries, expected,
                    "source={source}, length={length}, income={income}"
                );
                if income == partial {
                    let activity = market
                        .funding
                        .input_demand_scale(source, building, profile, false);
                    assert!(
                        activity > 0.0 && activity < 1.0,
                        "length={length}, activity={activity}"
                    );
                }
            }
        }
    }
}

fn with_disconnected_customer_chain(mut market: MarketFixture, length: usize) -> MarketFixture {
    let first = market.allocator.buildings.len();
    let a = market
        .graph
        .add_node(Vector3::new(2_000.0, 0.0, 0.0), NodeType::Border);
    let b = market
        .graph
        .add_node(Vector3::new(3_000.0, 0.0, 0.0), NodeType::Junction);
    let edge = market
        .graph
        .add_edge(make_road_edge(a, b, 2_000.0, 3_000.0));
    for index in 0..=length {
        let mut building = market.allocator.buildings[index].clone();
        building.center_x += 2_000.0;
        building.edge_idx = edge;
        market.allocator.buildings.push(building);
    }
    let mut family = market.families[0].clone();
    family.home_building_id = first + length;
    family.gross_daily_wages = 1_000_000.0;
    family.budget = 1_000_000.0;
    market.families.push(family);
    market.graph.rebuild_adjacency_list();
    market.network.lane_system.rebuild(&mut market.graph);
    market.network.rebuild_cch_and_check(&market.graph);
    market.allocator.rebuild_zone_index();
    market
        .allocator
        .rebuild_entrance_cache(&market.graph, &market.network.lane_system);
    market
}

#[test]
fn disconnected_healthy_customers_do_not_support_a_failed_supplier_chain() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    for length in [2, 8, 32] {
        let mut market = with_disconnected_customer_chain(financing_chain(length, 0), length);
        let mut replay = with_disconnected_customer_chain(financing_chain(length, 0), length);
        replay.funding.disable_customer_cascade();
        for income in [0.0, 1_000_000.0, 0.0, 1_000_000.0] {
            market.families[0].gross_daily_wages = income;
            market.families[0].budget = income;
            replay.families[0] = market.families[0].clone();
            market.refresh(&tuning);
            replay.refresh(&tuning);
            assert_eq!(market.funding.capacities, replay.funding.capacities);
            market
                .funding
                .customers
                .assert_same_accounts_for_test(&replay.funding.customers);
            for (first, funded) in [(0, income > 0.0), (length + 1, true)] {
                assert!(
                    market.funding.closure_passes <= 2,
                    "disconnected customers must not force a census for each failed supplier link"
                );
                assert_eq!(
                    &market.funding.capacities[first..first + length],
                    vec![u32::from(funded); length]
                );
                for source in first..first + length {
                    assert_eq!(
                        market.funding.business.financing(source).map(f64::to_bits),
                        replay.funding.business.financing(source).map(f64::to_bits)
                    );
                    let mut deliveries = Vec::new();
                    market
                        .funding
                        .input_supply
                        .visit_dependents(source, |buyer, _, units| {
                            deliveries.push((buyer, units))
                        });
                    assert_eq!(
                        deliveries,
                        if funded && source + 1 < first + length {
                            vec![(source + 1, 200.0)]
                        } else {
                            vec![]
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn zero_customer_propagation_matches_full_replay_in_varied_markets() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_export_price_multiplier = 0.0;
    let mut market = MarketFixture::new(&[0, 1, 2, 3]);
    let mut replay = MarketFixture::new(&[0, 1, 2, 3]);
    replay.funding.disable_customer_cascade();
    for cash in [100_000.0, 0.0, 100_000.0] {
        market.allocator.buildings[HOMES + 1].operating_budget = cash;
        replay.allocator.buildings[HOMES + 1].operating_budget = cash;
        market.refresh(&tuning);
        replay.refresh(&tuning);
        market.assert_same_funding(&replay);
    }
}

#[test]
fn census_receipt_commits_grow_exactly_from_empty_arenas() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_export_price_multiplier = 0.0;
    let mut market = MarketFixture::new(&[0, 1, 2, 3]);
    let mut reference = MarketFixture::new(&[0, 1, 2, 3]);
    market.funding.input_supply.discard_receipt_estimate = true;
    market.funding.customers.discard_receipt_estimate_for_test();
    // Released expansions and withdrawn/recovered suppliers fragment both receipt arenas.
    for cash in [100_000.0, 0.0, 100_000.0, 0.0] {
        for fixture in [&mut market, &mut reference] {
            fixture.allocator.buildings[HOMES + 1].operating_budget = cash;
            fixture.refresh(&tuning);
        }
        market.assert_same_funding(&reference);
        assert!(market.funding.input_supply.receipt_storage().0 > 0);
        assert!(market.funding.customers.receipt_capacity_for_test() > 0);
    }
}

#[test]
#[ignore = "manual release financing-cascade depth/background scaling; setup excluded"]
fn benchmark_financing_cascade_depth() {
    benchmark_chain_cases(false);
}

#[test]
#[ignore = "manual release disconnected financing-cascade depth/background scaling; setup excluded"]
fn benchmark_disconnected_financing_cascade_depth() {
    benchmark_chain_cases(true);
}

fn benchmark_chain_cases(disconnected: bool) {
    use std::time::Instant;
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    for (length, background) in [
        (4, 0),
        (16, 0),
        (64, 0),
        (4, 16_384),
        (16, 16_384),
        (64, 16_384),
    ] {
        let setup = Instant::now();
        let mut market = financing_chain(length, background);
        if disconnected {
            market = with_disconnected_customer_chain(market, length);
        }
        let setup_ms = setup.elapsed().as_secs_f64() * 1_000.0;
        let cold = Instant::now();
        market.refresh(&tuning);
        let cold_ms = cold.elapsed().as_secs_f64() * 1_000.0;
        let mut samples = [0.0_f64; 15];
        let mut passes = [0; 15];
        for round in 0..18 {
            let start = Instant::now();
            market.refresh(&tuning);
            let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
            if round >= 3 {
                samples[round - 3] = elapsed;
                passes[round - 3] = market.funding.closure_passes;
            }
            assert!(
                market.funding.capacities[..length + background + 1]
                    .iter()
                    .all(|&workers| workers == 0)
            );
            if disconnected {
                assert!(
                    market.funding.capacities[length + background + 1..2 * length + background + 1]
                        .iter()
                        .all(|&workers| workers == 1)
                );
            }
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "financing_cascade length={length} background={background} disconnected={disconnected} setup_ms={setup_ms:.3} cold_ms={cold_ms:.3} warm_median_ms={:.3} warm_max_ms={:.3} passes={passes:?}",
            samples[7], samples[14]
        );
    }
}

#[test]
#[ignore = "manual release partial financing-cascade depth/background scaling; setup excluded"]
fn benchmark_partial_financing_cascade_depth() {
    use std::time::Instant;
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    // Partial end-customer incomes from the replay regression; every link stays partly active.
    for (length, income, background) in [
        (8, 11_250.0, 0),
        (32, 25_000.0, 0),
        (8, 11_250.0, 16_384),
        (32, 25_000.0, 16_384),
    ] {
        let setup = Instant::now();
        let mut market = financing_chain(length, background);
        market.families[0].gross_daily_wages = income;
        market.families[0].budget = income;
        let setup_ms = setup.elapsed().as_secs_f64() * 1_000.0;
        let cold = Instant::now();
        market.refresh(&tuning);
        let cold_ms = cold.elapsed().as_secs_f64() * 1_000.0;
        let mut samples = [0.0_f64; 15];
        let mut passes = [0; 15];
        for round in 0..18 {
            let start = Instant::now();
            market.refresh(&tuning);
            let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
            if round >= 3 {
                samples[round - 3] = elapsed;
                passes[round - 3] = market.funding.closure_passes;
            }
            assert!(
                market.funding.capacities[..length]
                    .iter()
                    .all(|&workers| workers == 1)
            );
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "partial_financing_cascade length={length} background={background} setup_ms={setup_ms:.3} cold_ms={cold_ms:.3} warm_median_ms={:.3} warm_max_ms={:.3} passes={passes:?}",
            samples[7], samples[14]
        );
    }
}

// Adds a cashless rival beside every link of the first chain, on the same road.
fn with_cashless_rivals(mut market: MarketFixture, length: usize) -> MarketFixture {
    for index in 0..length {
        let mut rival = market.allocator.buildings[index].clone();
        rival.center_x += 2.0;
        rival.operating_budget = 0.0;
        market.allocator.buildings.push(rival);
    }
    market.allocator.rebuild_zone_index();
    market
        .allocator
        .rebuild_entrance_cache(&market.graph, &market.network.lane_system);
    market
}

// Compares every published link quantity with full replay: workers, recipe activity,
// financing and exact deliveries.
fn assert_links_match_replay(
    market: &MarketFixture,
    replay: &MarketFixture,
    links: impl Iterator<Item = usize>,
) {
    assert_eq!(market.funding.capacities, replay.funding.capacities);
    market
        .funding
        .customers
        .assert_same_accounts_for_test(&replay.funding.customers);
    for source in links {
        let building = &market.allocator.buildings[source];
        let profile = market
            .catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .unwrap();
        assert_eq!(
            market
                .funding
                .input_demand_scale(source, building, profile, false)
                .to_bits(),
            replay
                .funding
                .input_demand_scale(source, building, profile, false)
                .to_bits()
        );
        assert_eq!(
            market.funding.business.financing(source).map(f64::to_bits),
            replay.funding.business.financing(source).map(f64::to_bits)
        );
        let [mut deliveries, mut expected] = [Vec::new(), Vec::new()];
        market
            .funding
            .input_supply
            .visit_dependents(source, |buyer, _, units| {
                deliveries.push((buyer, units.to_bits()))
            });
        replay
            .funding
            .input_supply
            .visit_dependents(source, |buyer, _, units| {
                expected.push((buyer, units.to_bits()))
            });
        assert_eq!(deliveries, expected, "source={source}");
    }
}

fn rival_cascade(length: usize, background: usize) -> MarketFixture {
    with_cashless_rivals(
        with_disconnected_customer_chain(financing_chain(length, background), length),
        length,
    )
}

#[test]
fn cashless_rivals_do_not_hold_zero_or_partial_cascades_open() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    // A staffed rival without cash or stock stays a counted buyer unless its financing
    // impossibility is proven; otherwise every link waits for one more census.
    for (length, partial) in [(2, 7_750.0), (8, 11_250.0), (32, 25_000.0)] {
        let mut market = rival_cascade(length, 0);
        let mut replay = rival_cascade(length, 0);
        replay.funding.disable_customer_cascade();
        let links = market.allocator.buildings.len();
        for income in [1_000_000.0, partial, 0.0, partial, 1_000_000.0] {
            market.families[0].gross_daily_wages = income;
            market.families[0].budget = income;
            replay.families[0] = market.families[0].clone();
            market.refresh(&tuning);
            replay.refresh(&tuning);
            assert!(
                market.funding.closure_passes <= 2,
                "length={length}, income={income}"
            );
            if income < 1_000_000.0 && length > 2 {
                assert!(
                    replay.funding.closure_passes >= length,
                    "fixture must exercise the cascade"
                );
            }
            // Both chains and every rival; the homes between them carry no recipe.
            assert_links_match_replay(
                &market,
                &replay,
                (0..length)
                    .chain(length + 1..2 * length + 1)
                    .chain(links - length..links),
            );
            assert!(
                market.funding.capacities[links - length..]
                    .iter()
                    .all(|&workers| workers == 0)
            );
            assert!(
                market.funding.capacities[length + 1..2 * length + 1]
                    .iter()
                    .all(|&workers| workers == 1),
                "the disconnected healthy chain stays funded"
            );
        }
    }
}

#[test]
fn unprofitable_supplier_cycles_match_replay_within_their_depth() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    // Link 0 buys dearer goods than it sells. Its current profit could recover in a later
    // pass, so no ceiling may assume that; but even its cheapest possible inputs cost more
    // than its best possible sales, which proves a zero ceiling up front. Its suppliers then
    // lose their only possible buyer and the withdrawal crosses the whole loop in one
    // propagation instead of one census per link (`ECON-13-C`).
    for length in [2, 8, 64] {
        for units in [160.0, 199.8] {
            let mut market = financing_links(length, true, units, None, 0);
            let mut replay = financing_links(length, true, units, None, 0);
            replay.funding.disable_customer_cascade();
            for _ in 0..2 {
                market.refresh(&tuning);
                replay.refresh(&tuning);
                assert_links_match_replay(&market, &replay, 0..length);
                assert_eq!(&market.funding.capacities[..length], vec![0; length]);
                assert!(market.funding.closure_passes <= 2, "length={length}");
            }
        }
    }
    // With one high price every link is profitable at full activity, yet each link's sales
    // decay by `units / 200` around the loop. The joint proof rejects the loop before positive
    // limits can step that decay towards zero one round at a time.
    for (length, units) in [(2, 199.8), (2, 199.98), (8, 199.8)] {
        let mut market = financing_links(length, true, units, Some(1_000), 0);
        let mut replay = financing_links(length, true, units, Some(1_000), 0);
        replay.funding.disable_customer_cascade();
        for _ in 0..2 {
            market.refresh(&tuning);
            replay.refresh(&tuning);
            assert_links_match_replay(&market, &replay, 0..length);
            assert!(
                market.funding.closure_passes <= 2 && market.funding.propagation_rounds <= 1,
                "length={length}, units={units}"
            );
        }
    }
}

#[test]
#[ignore = "manual release cascade combinations: cashless rivals and unprofitable cycles; setup excluded"]
fn benchmark_combined_financing_cascades() {
    use std::time::Instant;
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    let incomes = |length| match length {
        8 => 11_250.0,
        32 => 25_000.0,
        _ => 0.0,
    };
    for (case, length, background) in [
        ("rivals", 8, 0),
        ("rivals", 32, 0),
        ("rivals", 64, 0),
        ("rivals", 8, 16_384),
        ("rivals", 32, 16_384),
        ("rivals", 64, 16_384),
        ("cycle", 4, 0),
        ("cycle", 16, 0),
        ("cycle", 64, 0),
        ("cycle", 4, 16_384),
        ("cycle", 16, 16_384),
        ("cycle", 64, 16_384),
    ] {
        let setup = Instant::now();
        let mut market = if case == "rivals" {
            rival_cascade(length, background)
        } else {
            financing_links(length, true, 199.8, None, background)
        };
        // Rival chains alternate partial or zero end income; 64 links use the zero cascade.
        market.families[0].gross_daily_wages = incomes(length);
        market.families[0].budget = incomes(length);
        let setup_ms = setup.elapsed().as_secs_f64() * 1_000.0;
        let cold = Instant::now();
        market.refresh(&tuning);
        let cold_ms = cold.elapsed().as_secs_f64() * 1_000.0;
        let cold_passes = market.funding.closure_passes;
        let mut samples = [0.0_f64; 15];
        let mut passes = [0; 15];
        for round in 0..18 {
            let start = Instant::now();
            market.refresh(&tuning);
            let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
            if round >= 3 {
                samples[round - 3] = elapsed;
                passes[round - 3] = market.funding.closure_passes;
            }
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "combined_financing_cascade case={case} length={length} background={background} setup_ms={setup_ms:.3} cold_ms={cold_ms:.3} cold_passes={cold_passes} warm_median_ms={:.3} warm_max_ms={:.3} passes={passes:?}",
            samples[7], samples[14]
        );
    }
}

#[test]
#[ignore = "short PERF-01 locality acceptance; setup excluded from timings"]
fn perf_contract_financing_locality() {
    use std::time::Instant;

    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    let mut results = Vec::new();
    for length in [4, 16, 64] {
        let setup = Instant::now();
        let mut market = financing_links(length, true, 199.8, None, 0);
        let setup_ms = setup.elapsed().as_secs_f64() * 1_000.0;

        let cold = Instant::now();
        market.refresh(&tuning);
        let cold_ms = cold.elapsed().as_secs_f64() * 1_000.0;
        let cold_passes = market.funding.closure_passes;
        assert!(
            cold_passes <= 3,
            "cold financing closure exceeded PERF-01 bound: length={length} passes={cold_passes}"
        );

        let warm_start = Instant::now();
        let mut warm_passes = 0;
        for _ in 0..8 {
            market.refresh(&tuning);
            warm_passes = warm_passes.max(market.funding.closure_passes);
        }
        let warm_ms = warm_start.elapsed().as_secs_f64() * 1_000.0;
        assert!(
            warm_passes <= 3,
            "warm financing closure exceeded PERF-01 bound: length={length} passes={warm_passes}"
        );
        results.push(serde_json::json!({
            "length": length,
            "setup_ms": setup_ms,
            "cold_ms": cold_ms,
            "cold_closure_passes": cold_passes,
            "warm_batch_ms": warm_ms,
            "warm_closure_passes": warm_passes,
        }));
    }
    eprintln!(
        "PERF_LOCALITY {}",
        serde_json::json!({ "kind": "financing", "results": results })
    );
}

// A labelled fixture and the incomes its census runs at in turn.
// `SIM-02-K`: a census restarted after any slice, inside the consumer-market refresh's household
// phases too, equals one run bit for bit. A restart there leaves the customer book torn, and the
// restarted refresh withdraws it, so the census is the one a loaded game computes. On a cold
// book the census also rewrites its route quotes, builds its supplier index and lays out its
// business book, and restarts land inside each of those slices and inside the business claims'
// serial sweep.
#[test]
fn a_census_restarted_after_any_slice_equals_one_run_bit_for_bit() {
    use crate::simulation::economy::hiring::RESTARTED_AT;

    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    RESTARTED_AT.with_borrow_mut(Vec::clear);
    // The second census's income: unchanged (the reference retains its book) or changed; or
    // the census on a cold book, the first of its fixture.
    for (second, cold) in [(1_000_000.0, false), (7_750.0, false), (7_750.0, true)] {
        let census = |restart: Option<usize>| {
            let mut market = financing_chain(2, 3);
            if !cold {
                market.families[0].gross_daily_wages = 1_000_000.0;
                market.families[0].budget = 1_000_000.0;
                market.refresh(&tuning);
            }
            market.families[0].gross_daily_wages = second;
            market.families[0].budget = second;
            let slices = match restart {
                None => {
                    market.refresh(&tuning);
                    0
                }
                Some(_) => market.refresh_in_slices(&tuning, 1, restart),
            };
            (market, slices)
        };
        let (reference, _) = census(None);
        let (_, slices) = census(Some(usize::MAX));
        let mut torn = 0;
        for restart in 1..slices {
            let (market, _) = census(Some(restart));
            torn += market.funding.customers.torn_refreshes_for_test();
            reference.assert_same_census(
                &market,
                &format!("income {second}, cold {cold}, restart after slice {restart} of {slices}"),
            );
        }
        if !cold {
            assert!(torn > 0, "some restart must land inside the refresh");
        }
    }
    // The fixture has one family, so its claims' accounts reset in one slice; restarts land
    // inside the availability reset after them.
    let restarted = RESTARTED_AT.with_borrow_mut(std::mem::take);
    for step in [
        "route_access",
        "route_quotes",
        "customer_index_build",
        "customer_claim_availability",
        "sources_layout",
        "sources_index",
        "sources_rates",
        "business_seed_rows",
        "business_layout_access",
        "business_layout_ports",
        "business_layout_indices",
        "business_claims",
    ] {
        assert!(
            restarted
                .iter()
                .any(|&(name, done)| name == step && done > 0),
            "some restart must land inside {step}"
        );
    }
}

type MarketCase = (String, Box<dyn Fn() -> MarketFixture>, Vec<f32>);

// The census in slices of any size, and restarted part way as a network change restarts it,
// equals the census run at once bit for bit (`SIM-02-I`), through closures of several passes.
#[test]
fn a_census_in_slices_or_restarted_equals_one_run_bit_for_bit() {
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 10.0;
    tuning.owa_export_price_multiplier = 0.0;
    // Budgets in rows, and the slice after which the census starts again.
    let slicings = [
        (1, None),
        (5, Some(3)),
        (37, None),
        (37, Some(1)),
        (1_000, None),
    ];
    let mut most_passes = 0;
    let mut cases: Vec<MarketCase> = vec![(
        "mixed districts".into(),
        Box::new(|| MarketFixture::new(&[0, 1, 2, 3])),
        vec![0.0, 0.0],
    )];
    for (length, partial, failing) in [(2, 7_750.0, 7_500.0), (8, 11_250.0, 10_500.0)] {
        for cascade in [true, false] {
            cases.push((
                format!("chain {length} cascade {cascade}"),
                Box::new(move || {
                    let mut market = financing_chain(length, 3);
                    if !cascade {
                        market.funding.disable_customer_cascade();
                    }
                    market
                }),
                vec![1_000_000.0, partial, failing, partial, 1_000_000.0],
            ));
        }
    }
    for (label, build, incomes) in &cases {
        let mut reference = build();
        let mut sliced: Vec<_> = slicings.iter().map(|_| build()).collect();
        for &income in incomes {
            for market in std::iter::once(&mut reference).chain(&mut sliced) {
                if income > 0.0 {
                    market.families[0].gross_daily_wages = income;
                    market.families[0].budget = income;
                }
            }
            reference.refresh(&tuning);
            most_passes = most_passes.max(reference.funding.closure_passes);
            for (market, &(budget, restart)) in sliced.iter_mut().zip(&slicings) {
                let slices = market.refresh_in_slices(&tuning, budget, restart);
                assert!(
                    budget >= 1_000 || slices > 1,
                    "{label}: {budget} rows a slice"
                );
                reference.assert_same_census(
                    market,
                    &format!("{label}, income {income}, {budget} rows, restart {restart:?}"),
                );
            }
        }
    }
    assert!(
        most_passes > 2,
        "the fixtures must close over several passes"
    );
}
