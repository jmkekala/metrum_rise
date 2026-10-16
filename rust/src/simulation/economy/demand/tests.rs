// SPDX-License-Identifier: GPL-2.0-only

//! Demand tests.

mod alternatives;

use super::spawn_need::{
    OutputAbsorptionContext, commercial_market_covers_operation,
    nonresidential_passes_absorption_gate,
};
use super::types::DemandUse;
use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::economy::agents::{HouseholdAgeComposition, household_age_composition};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::{
    Household, HouseholdSystem, REPLENISHMENT_STABLE,
    candidate_immigrant_household_size_for_vacancy,
    candidate_immigrant_household_size_from_flat_size,
};
use crate::simulation::economy::operation::OperationPrices;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{
    EdgeClass, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::{Vector2, Vector3};

fn test_economy_runtime_id(zone_type: ZoneType) -> u16 {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    match zone_type {
        ZoneType::Commercial => {
            catalog
                .profile_for_id("grocery_basic")
                .expect("grocery starter profile")
                .runtime_id
        }
        ZoneType::Industrial => {
            catalog
                .profile_for_id("food_processor_basic")
                .expect("food processor starter profile")
                .runtime_id
        }
        _ => 0,
    }
}

fn register_test_asset(
    allocator: &mut BuildingAllocator,
    asset_id: &str,
    zone_type: ZoneType,
) -> String {
    register_family_asset(allocator, asset_id, zone_type, None, 1)
}

fn register_test_utility_asset(
    allocator: &mut BuildingAllocator,
    asset_id: &str,
    profile_id: &str,
) -> String {
    allocator.registry.register(
        "test",
        AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test Utility".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![],
            mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
                width_m: None,
                length_m: None,
                vehicle_class: None,
            }],
            site_surfaces: vec![],
            building: Some(BuildingData {
                window_brightness: 3.0,
                appearance: None,
                flat_size_m2: None,
                placement_mode: PlacementMode::Explicit,
                zone_type: None,
                density: None,
                lot_width_cells: 2,
                lot_depth_cells: 2,
                frontage_forward: None,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity: None,
                worker_capacity: Some(20),
                service_class: Some("power".to_owned()),
                economy_profile: Some(profile_id.to_owned()),
                extractor: None,
                field: None,
                yard_hedge: None,
                yard_planting: Vec::new(),
            }),
            prop: None,
            vehicle: None,
            character: None,
        },
        String::new(),
    );
    format!("test:{asset_id}")
}

fn register_explicit_profile_asset(
    allocator: &mut BuildingAllocator,
    asset_id: &str,
    profile_id: &str,
) -> String {
    allocator.registry.register(
        "test",
        AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test Explicit Work Area".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![],
            mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
                width_m: None,
                length_m: None,
                vehicle_class: None,
            }],
            site_surfaces: vec![],
            building: Some(BuildingData {
                window_brightness: 3.0,
                appearance: None,
                flat_size_m2: None,
                placement_mode: PlacementMode::Explicit,
                zone_type: None,
                density: None,
                lot_width_cells: 2,
                lot_depth_cells: 2,
                frontage_forward: None,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity: None,
                worker_capacity: Some(8),
                service_class: None,
                economy_profile: Some(profile_id.to_owned()),
                extractor: None,
                field: None,
                yard_hedge: None,
                yard_planting: Vec::new(),
            }),
            prop: None,
            vehicle: None,
            character: None,
        },
        String::new(),
    );
    format!("test:{asset_id}")
}

fn register_family_asset(
    allocator: &mut BuildingAllocator,
    asset_id: &str,
    zone_type: ZoneType,
    asset_set: Option<&str>,
    level: u8,
) -> String {
    let economy_profile = match zone_type {
        ZoneType::Commercial => Some("grocery_basic"),
        ZoneType::Industrial => Some("food_processor_basic"),
        _ => None,
    };
    register_family_asset_with_economy_profile(
        allocator,
        asset_id,
        zone_type,
        asset_set,
        level,
        economy_profile,
    )
}

fn register_family_asset_with_economy_profile(
    allocator: &mut BuildingAllocator,
    asset_id: &str,
    zone_type: ZoneType,
    asset_set: Option<&str>,
    level: u8,
    economy_profile: Option<&str>,
) -> String {
    register_family_asset_with_economy_profile_and_flat_size(
        allocator,
        asset_id,
        zone_type,
        asset_set,
        level,
        economy_profile,
        None,
    )
}

fn register_family_asset_with_economy_profile_and_flat_size(
    allocator: &mut BuildingAllocator,
    asset_id: &str,
    zone_type: ZoneType,
    asset_set: Option<&str>,
    level: u8,
    economy_profile: Option<&str>,
    flat_size_m2: Option<f32>,
) -> String {
    let (zone_class, household_capacity, worker_capacity) = match zone_type {
        ZoneType::Residential => (ZoneClass::Residential, Some(6), None),
        ZoneType::Commercial => (ZoneClass::Commercial, None, Some(4)),
        ZoneType::Industrial => (ZoneClass::Industrial, None, Some(4)),
        ZoneType::Office => (ZoneClass::Office, None, Some(4)),
        ZoneType::Mixed => (ZoneClass::Mixed, Some(4), Some(2)),
        ZoneType::None => panic!("test assets must use a real zone type"),
    };
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Test".to_owned(),
        asset_set: asset_set.map(str::to_owned),
        tags: vec![],
        thumbnail: None,
        lods: vec![],
        mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [0.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
            width_m: None,
            length_m: None,
            vehicle_class: None,
        }],
        site_surfaces: vec![],
        building: Some(BuildingData {
            window_brightness: 3.0,
            appearance: None,
            flat_size_m2: if matches!(zone_type, ZoneType::Residential | ZoneType::Mixed) {
                Some(flat_size_m2.unwrap_or(80.0))
            } else {
                None
            },
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(zone_class),
            density: Some("low".to_owned()),
            lot_width_cells: 2,
            lot_depth_cells: 2,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level,
            household_capacity,
            worker_capacity,
            service_class: None,
            economy_profile: economy_profile.map(str::to_owned),
            extractor: None,
            field: None,
            yard_hedge: None,
            yard_planting: Vec::new(),
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator.registry.register("test", manifest, String::new());
    format!("test:{asset_id}")
}

fn building(
    zone_type: ZoneType,
    stock: f32,
    occupancy: u32,
    worker_count: u32,
    asset_id: String,
) -> Building {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let runtime_id = test_economy_runtime_id(zone_type);
    let mut resource_inventory = vec![0.0; catalog.resource_count()];
    if stock > 0.0
        && let Some(profile) = catalog.profile_by_runtime_id(runtime_id)
        && let Some(output_port) = profile.outputs.first()
    {
        resource_inventory[output_port.resource_runtime_id as usize - 1] = stock;
    }
    Building {
        build_generation: 0,
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 2,
        depth_cells: 2,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy,
        worker_count,
        service_funding_override: -1.0,
        asset_id,
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: runtime_id,
        economy_broken: false,
        resource_inventory,
        revenue: 0.0,
        operating_budget: 500.0,
        profit_tax_budget_baseline: 500.0,
        last_day_profit: 0.0,
        shipment_cooldown_hours: 0,
        daily_owa_input_value: 0.0,
        daily_local_input_value: 0.0,
        daily_city_funded_input_cost: 0.0,
        daily_household_sales_value: 0.0,
        daily_power_service_units: 0.0,
        daily_electricity: Default::default(),
        recent_electricity: Default::default(),
        hourly_economy_activity: 0.0,
        daily_power_served_units: 0.0,
        recent_power_service_units: 0.0,
        recent_power_served_units: 0.0,
        recent_household_sales_value: 0.0,
        commercial_activity_floor_scale: 0.0,
        work_area_scale: 1.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    }
}

fn housed_household(
    home_building_id: usize,
    member_count: u16,
    budget: f32,
    stock_days: f32,
) -> Household {
    Household {
        home_building_id,
        power_arrears: Default::default(),
        budget,
        stock: stock_days * member_count as f32,
        member_count,
        child_count: 0,
        adult_count: member_count,
        employed_adults: 0,
        gross_daily_wages: 0.0,
        shopping_has_car: false,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days,
        replenishment_state: REPLENISHMENT_STABLE,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent: crate::simulation::economy::agents::AgentHandle::NONE,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    }
}

fn unhoused_household(member_count: u16, budget: f32, stock_days: f32) -> Household {
    housed_household(usize::MAX, member_count, budget, stock_days)
}

fn graph_with_connected_border() -> RegionGraph {
    let mut graph = RegionGraph::new();
    let border = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Border);
    let junction = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(Edge {
        start_node: border,
        end_node: junction,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 50.0,
        physical_length: 50.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
    graph
}

fn empty_zoning() -> ZoningSystem {
    ZoningSystem::new(&WorldConfig::default())
}

fn zoning_run(graph: &RegionGraph, zone_type: ZoneType) -> ZoningSystem {
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let profile = zoning
        .profiles
        .default_runtime_id_for_zone_type(zone_type)
        .expect("zoning profile");
    zoning
        .place_parcel_run_at(10.0, -20.0, 40.0, -20.0, profile, 20.0, 30.0, 0.0, graph)
        .expect("zoning run");
    zoning
}

fn residential_zoning_run(graph: &RegionGraph) -> ZoningSystem {
    zoning_run(graph, ZoneType::Residential)
}

fn commercial_zoning_run(graph: &RegionGraph) -> ZoningSystem {
    zoning_run(graph, ZoneType::Commercial)
}

fn industrial_zoning_run(graph: &RegionGraph) -> ZoningSystem {
    zoning_run(graph, ZoneType::Industrial)
}

fn vacant_admission_snapshot() -> DailyDemandSnapshot {
    DailyDemandSnapshot {
        unhoused_workforce: Vec::new(),
        vacant_household_slots: 10,
        under_construction_household_slots: 0,
        constructing_by_use: [0; 3],
        queued_by_use: [0; 3],
        total_household_count: 4,
        housed_household_count: 4,
        unhoused_household_count: 0,
        zero_budget_household_count: 0,
        persistent_exit_eligible_household_count: 0,
        unhoused_household_ratio: 0.0,
        zero_budget_household_ratio: 0.0,
        committed_unmet_commercial_consumer_demand_by_resource: Vec::new(),
        connected_border_count: 1,
        city_treasury_balance: 100_000.0,
        daily_essential_cost_per_resident: 27.0,
        daily_home_electricity_cost: 0.0,
        unemployment_daily_benefit_per_adult: 30.0,
        pension_daily_benefit_per_elder: 30.0,
        child_support_daily_benefit_per_child: 10.0,
        existing_unemployed_member_count: 0,
        existing_child_count: 0,
        existing_elder_count: 0,
        open_job_slots: 2,
        move_in_job_slots: 2,
        move_in_job_equivalent_slots: 2.0,
        average_move_in_job_wage_per_day: 79.2,
        physical_worker_capacity: 0,
        funded_worker_capacity: 0,
        open_jobs_unfunded: 0,
        output_absorption: OutputAbsorptionContext::empty(0),
    }
}

#[test]
fn customer_reservations_require_recurring_household_income() {
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "consumer_home", ZoneType::Residential);
    allocator
        .buildings
        .push(building(ZoneType::Residential, 0.0, 1, 0, home));
    let mut households = HouseholdSystem::new();
    households
        .households
        .push(housed_household(0, 5, 1_000_000.0, 3.0));
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().unwrap();
    let catalog = load_runtime_economy_catalog().unwrap();
    let supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .unwrap();
    let unfunded =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(unfunded.output_absorption.unmet_units(supplies), 0.0);
    households.households[0].gross_daily_wages = 1_000.0;
    households.households[0].employed_adults = 5;
    households.households[0].budget = 0.0;
    let funded =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(funded.output_absorption.unmet_units(supplies), 5.0);
}

#[test]
fn completing_or_unstaffed_shop_keeps_its_customer_reservation() {
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "capacity_home", ZoneType::Residential);
    let pharmacy = register_family_asset_with_economy_profile(
        &mut allocator,
        "capacity_pharmacy",
        ZoneType::Commercial,
        None,
        1,
        Some("health_essentials_small"),
    );
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("health_essentials_small").unwrap();
    let health = profile.outputs[0].resource_runtime_id;
    allocator
        .buildings
        .push(building(ZoneType::Residential, 0.0, 1, 0, home));
    let mut shop = building(ZoneType::Commercial, 0.0, 0, 0, pharmacy.clone());
    shop.economy_profile_runtime_id = profile.runtime_id;
    allocator.buildings.push(shop);
    let mut households = HouseholdSystem::new();
    let mut customers = housed_household(0, 80, 0.0, 3.0);
    customers.gross_daily_wages = 8_000.0;
    customers.employed_adults = 80;
    households.households.push(customers);
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().unwrap();
    for (remaining_hours, workers) in [(8, 0), (0, 0), (0, 1), (0, 6)] {
        allocator.buildings[1].construction_remaining_hours = remaining_hours;
        allocator.buildings[1].worker_count = workers;
        let snapshot =
            DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
        assert_eq!(
            snapshot.output_absorption.unmet_units(health),
            0.0,
            "completion/staffing must not release already committed customers"
        );
        assert!(!nonresidential_passes_absorption_gate(
            &allocator,
            &catalog,
            &snapshot.output_absorption,
            &pharmacy,
        ));
    }
    allocator.buildings[1].is_deserted = true;
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert!(snapshot.output_absorption.unmet_units(health) > 0.0);
    assert!(
        nonresidential_passes_absorption_gate(
            &allocator,
            &catalog,
            &snapshot.output_absorption,
            &pharmacy,
        ),
        "a failed shop releases its customers for replacement"
    );
}

#[test]
fn self_supplied_maintenance_does_not_consume_saleable_capacity_twice() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let port = &profile.outputs[0];
    let net = profile.net_output_units_per_day(port);
    let mut allocator = BuildingAllocator::new();
    let asset = register_family_asset_with_economy_profile(
        &mut allocator,
        "net_machinery",
        ZoneType::Industrial,
        None,
        1,
        Some("machinery_factory_basic"),
    );
    let mut factory = building(ZoneType::Industrial, 0.0, 0, profile.worker_capacity, asset);
    factory.economy_profile_runtime_id = profile.runtime_id;
    allocator.buildings.push(factory);
    let households = HouseholdSystem::new();
    let graph = RegionGraph::new();
    let config = load_builtin_demand_config().unwrap();
    for remaining in [8, 0] {
        allocator.buildings[0].construction_remaining_hours = remaining;
        let mut snapshot =
            DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
        snapshot
            .output_absorption
            .add_demand_units(port.resource_runtime_id, net);
        assert_eq!(
            snapshot
                .output_absorption
                .unmet_units(port.resource_runtime_id),
            0.0
        );
        snapshot
            .output_absorption
            .add_demand_units(port.resource_runtime_id, 1.0);
        assert_eq!(
            snapshot
                .output_absorption
                .unmet_units(port.resource_runtime_id),
            1.0
        );
    }
}

#[test]
fn replacement_releases_only_its_net_output_and_preserves_neighbors_claims() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let port = &profile.outputs[0];
    let net = profile.net_output_units_per_day(port);
    assert!(net < port.units_per_day);
    let mut market = OutputAbsorptionContext::from_resource_amounts(
        catalog.resource_count(),
        &[(port.resource_runtime_id, 2.0 * net)],
        &[(port.resource_runtime_id, 2.0 * net)],
        1,
        &[],
    );
    market.reserve_replacement(profile, 1.0, profile);
    assert_eq!(market.unmet_units(port.resource_runtime_id), 0.0);
}

#[test]
fn commercial_spawn_requires_operating_income_before_housing_pull() {
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(housed_household(0, 5, 1_000.0, 3.0));

    let graph = graph_with_connected_border();
    let zoning = commercial_zoning_run(&graph);
    let mut demand = DemandSystem::new();
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let required_workers = allocator
        .worker_capacity_for_asset_with_catalog(&commercial_asset, catalog.as_ref())
        .expect("commercial economy profile must resolve");

    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);

    assert!(
        required_workers > 5,
        "regression setup must cover a profile whose full staffing exceeds the starter household"
    );

    assert_eq!(
        demand.building_actions.commercial.spawns.len(),
        0,
        "five customers cannot fund this shop's first worker; future immigrants cannot finance its opening"
    );
}

#[test]
fn commercial_spawn_candidate_prefers_asset_for_unmet_consumer_resource() {
    let mut allocator = BuildingAllocator::new();
    register_family_asset_with_economy_profile(
        &mut allocator,
        "commercial_grocery",
        ZoneType::Commercial,
        None,
        1,
        Some("grocery_basic"),
    );
    let barber = register_family_asset_with_economy_profile(
        &mut allocator,
        "commercial_barber",
        ZoneType::Commercial,
        None,
        1,
        Some("personal_service_small"),
    );
    register_family_asset_with_economy_profile(
        &mut allocator,
        "commercial_pharmacy",
        ZoneType::Commercial,
        None,
        1,
        Some("health_essentials_small"),
    );
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let personal_services = catalog
        .resource_runtime_id_for_id("personal_services")
        .expect("personal_services resource");
    let graph = graph_with_connected_border();
    let zoning = commercial_zoning_run(&graph);

    let terrain = crate::simulation::terrain::TerrainSystem::new(4, 4);
    let mut road_surface = crate::simulation::network::surface::RoadSurfaceSystem::new(512.0);
    road_surface.compile_dirty(&graph, &terrain);
    let candidates = allocator.collect_demand_spawn_candidates_by_use(
        &zoning,
        &graph,
        catalog.as_ref(),
        &[(personal_services, 1.0)],
        crate::simulation::buildings::allocator::BuildingSiteEnvironment {
            road_surface: &road_surface,
            terrain: &terrain,
        },
    );

    assert!(!candidates.commercial.is_empty());
    assert!(
        candidates
            .commercial
            .iter()
            .all(|candidate| candidate.action.asset_id == barber),
        "unmet personal-service demand should expose barber candidates before generic commercial variants"
    );
}

#[test]
fn nonresidential_absorption_gate_fails_safe_without_candidate_profile() {
    let mut allocator = BuildingAllocator::new();
    let unbound_commercial = register_family_asset_with_economy_profile(
        &mut allocator,
        "commercial_without_profile",
        ZoneType::Commercial,
        None,
        1,
        None,
    );
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let absorption = OutputAbsorptionContext::empty(catalog.resource_count());

    assert!(!nonresidential_passes_absorption_gate(
        &allocator,
        catalog.as_ref(),
        &absorption,
        &unbound_commercial,
    ));
}

#[test]
fn nonresidential_absorption_gate_fails_safe_without_matching_demand() {
    let mut allocator = BuildingAllocator::new();
    let commercial = register_test_asset(
        &mut allocator,
        "commercial_without_consumers",
        ZoneType::Commercial,
    );
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let absorption = OutputAbsorptionContext::empty(catalog.resource_count());

    assert!(!nonresidential_passes_absorption_gate(
        &allocator,
        catalog.as_ref(),
        &absorption,
        &commercial,
    ));
}

#[test]
fn industrial_absorption_gate_uses_commercial_input_demand() {
    let mut allocator = BuildingAllocator::new();
    let industrial = register_test_asset(
        &mut allocator,
        "industrial_with_downstream_need",
        ZoneType::Industrial,
    );
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged_food resource");
    let absorption = OutputAbsorptionContext::from_resource_amounts(
        catalog.resource_count(),
        &[],
        &[],
        0,
        &[(packaged_food, 160.0)],
    );

    assert!(nonresidential_passes_absorption_gate(
        &allocator,
        catalog.as_ref(),
        &absorption,
        &industrial,
    ));
}

#[test]
fn residential_need_counts_live_pull_and_committed_household_places() {
    let mut snapshot = vacant_admission_snapshot();
    snapshot.total_household_count = 8;
    snapshot.vacant_household_slots = 1;
    assert_eq!(residential_missing_household_slots(&snapshot, 0), 0);
    assert_eq!(residential_missing_household_slots(&snapshot, 2), 2);
    snapshot.vacant_household_slots = 0;
    assert_eq!(residential_missing_household_slots(&snapshot, 2), 3);
    snapshot.under_construction_household_slots = 6;
    assert_eq!(residential_missing_household_slots(&snapshot, 2), 0);
}

#[test]
fn residential_need_includes_displaced_households_before_spare_capacity() {
    let mut snapshot = vacant_admission_snapshot();
    snapshot.total_household_count = 100;
    snapshot.unhoused_household_count = 40;
    snapshot.vacant_household_slots = 10;
    snapshot.under_construction_household_slots = 20;
    assert_eq!(residential_missing_household_slots(&snapshot, 5), 25);
    snapshot.under_construction_household_slots = 45;
    assert_eq!(residential_missing_household_slots(&snapshot, 5), 0);
}

#[test]
fn displaced_households_plan_replacement_homes_without_new_jobs_or_arrivals() {
    for connected in [false, true] {
        let mut allocator = BuildingAllocator::new();
        register_test_asset(&mut allocator, "replacement_home", ZoneType::Residential);
        let mut households = HouseholdSystem::new();
        for _ in 0..10 {
            households
                .households
                .push(unhoused_household(2, 1_000.0, 3.0));
        }
        let mut graph = graph_with_connected_border();
        if !connected {
            graph.set_node_type(0, NodeType::Junction);
        }
        let zoning = residential_zoning_run(&graph);
        let mut demand = DemandSystem::new();
        demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
        assert_eq!(
            demand.last_admission_diagnostics.incoming_household_need,
            0.0
        );
        assert_eq!(demand.households_to_admit_today, 0);
        assert_eq!(
            demand
                .last_building_action_diagnostics
                .residential
                .housing_needed_slots,
            11,
        );
        let homes = demand.building_actions.residential.spawns.clone();
        assert_eq!(
            homes.len(),
            2,
            "ten displaced households need both six-place homes"
        );

        demand.set_queued_construction(
            &allocator,
            &zoning,
            0,
            homes
                .iter()
                .enumerate()
                .map(|(i, home)| (i as u64 + 1, ZoneType::Residential, home)),
        );
        demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
        assert_eq!(
            demand
                .last_building_action_diagnostics
                .residential
                .housing_needed_slots,
            0
        );
        assert!(demand.building_actions.residential.spawns.is_empty());
    }
}

#[test]
fn pending_commercial_construction_is_committed_but_not_live_capacity() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    let commercial_asset = register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));
    let mut pending_store = building(ZoneType::Commercial, 0.0, 0, 0, commercial_asset.clone());
    pending_store.construction_total_hours = 6;
    pending_store.construction_remaining_hours = 6;
    allocator.buildings.push(pending_store);

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(housed_household(0, 5, 1_000.0, 3.0));

    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);

    assert_eq!(snapshot.constructing_by_use, [0, 1, 0]);
    assert_eq!(snapshot.queued_by_use, [0; 3]);

    let expected_uncommitted_service_demand = 5.0
        * ["personal_service_demand", "health_essentials_demand"]
            .iter()
            .map(|id| {
                catalog
                    .profile_for_id(id)
                    .unwrap()
                    .consumption_rate_per_resident
            })
            .sum::<f32>();
    assert!(
        (snapshot
            .committed_unmet_commercial_consumer_demand_by_resource
            .iter()
            .map(|(_, units)| *units)
            .sum::<f32>()
            - expected_uncommitted_service_demand)
            .abs()
            < 0.001,
        "pending grocery construction should not satisfy unrelated service-store demand"
    );

    assert!(!nonresidential_passes_absorption_gate(
        &allocator,
        catalog.as_ref(),
        &snapshot.output_absorption,
        &commercial_asset,
    ));
}

#[test]
fn pantry_recovery_does_not_create_recurring_grocery_customers() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    let commercial_asset = register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));
    allocator.buildings.push(building(
        ZoneType::Commercial,
        0.0,
        0,
        0,
        commercial_asset.clone(),
    ));

    let mut households = HouseholdSystem::new();
    for _ in 0..75 {
        households
            .households
            .push(housed_household(0, 2, 1_000.0, 0.0));
    }

    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);

    assert!(
        crate::simulation::economy::resource_totals::resource_amount(
            &snapshot.committed_unmet_commercial_consumer_demand_by_resource,
            catalog
                .resource_runtime_id_for_id("household_supplies")
                .unwrap(),
        ) > 90.0,
        "one 200/day grocery should not hide a 300/day pantry recovery demand"
    );

    // Recovery purchases are not recurring demand for a second grocery.
    assert!(!nonresidential_passes_absorption_gate(
        &allocator,
        catalog.as_ref(),
        &snapshot.output_absorption,
        &commercial_asset,
    ));
}

#[test]
fn pending_industrial_construction_is_committed_but_not_live_capacity() {
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    let industrial_asset = register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
    allocator
        .buildings
        .push(building(ZoneType::Commercial, 0.0, 0, 1, commercial_asset));
    let mut pending_industrial =
        building(ZoneType::Industrial, 0.0, 0, 0, industrial_asset.clone());
    pending_industrial.construction_total_hours = 6;
    pending_industrial.construction_remaining_hours = 6;
    allocator.buildings.push(pending_industrial);

    let households = HouseholdSystem::new();
    // Isolate local customers from the separately tested regional market.
    let graph = RegionGraph::new();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);

    assert_eq!(
        snapshot
            .output_absorption
            .unmet_units(catalog.resource_runtime_id_for_id("packaged_food").unwrap()),
        0.0
    );

    assert!(!nonresidential_passes_absorption_gate(
        &allocator,
        catalog.as_ref(),
        &snapshot.output_absorption,
        &industrial_asset,
    ));
}

#[test]
fn hourly_pass_does_not_admit_without_external_connection() {
    let allocator = BuildingAllocator::new();
    let mut households = HouseholdSystem::new();
    let graph = RegionGraph::new();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(
        &allocator,
        &mut households,
        &graph,
        &empty_zoning(),
        1_000.0,
    );

    assert_eq!(demand.last_admission_diagnostics.connected_border_count, 0);

    assert_eq!(demand.households_to_admit_today, 0);
}

#[test]
fn max_demand_cheat_survives_daily_and_hourly_recompute() {
    let allocator = BuildingAllocator::new();
    let mut households = HouseholdSystem::new();
    let graph = RegionGraph::new();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.enable_max_demand_cheat();
    assert!(demand.cheat_max_demands_enabled);

    demand.run_daily_pass(&allocator, &households);
    assert!(demand.cheat_max_demands_enabled);

    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, -100.0);
    assert!(demand.cheat_max_demands_enabled);
}

#[test]
fn max_demand_cheat_plans_commercial_spawn_without_consumer_need() {
    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    let mut households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = commercial_zoning_run(&graph);
    let mut demand = DemandSystem::new();

    demand.enable_max_demand_cheat();
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 1_000_000.0);

    assert_eq!(
        demand.building_actions.commercial.spawns.len(),
        demand
            .last_building_action_diagnostics
            .commercial
            .spawn_candidates,
    );
    assert_eq!(
        demand
            .last_building_action_diagnostics
            .commercial
            .spawn_rejected_absorption,
        0
    );
}

#[test]
fn industrial_opening_requires_enough_homes_ready_by_completion() {
    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "industrial_labor", ZoneType::Industrial);
    let mut households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = industrial_zoning_run(&graph);
    let mut demand = DemandSystem::new();
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
    assert!(demand.building_actions.industrial.spawns.is_empty());
    assert!(
        demand
            .last_building_action_diagnostics
            .industrial
            .spawn_rejected_workforce
            > 0
    );

    let home = register_test_asset(&mut allocator, "available_home", ZoneType::Residential);
    allocator
        .buildings
        .push(building(ZoneType::Residential, 0.0, 0, 0, home.clone()));
    allocator.rebuild_zone_index();
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
    assert!(
        demand.building_actions.industrial.spawns.is_empty(),
        "six homes cannot supply ten workers"
    );

    let mut second_home = building(ZoneType::Residential, 0.0, 0, 0, home);
    second_home.construction_total_hours = 18;
    second_home.construction_remaining_hours = 18;
    allocator.buildings.push(second_home);
    allocator.rebuild_zone_index();
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
    assert!(
        demand.building_actions.industrial.spawns.is_empty(),
        "homes completing after the factory cannot supply its workers"
    );

    allocator.buildings[1].construction_remaining_hours = 12;
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
    assert_eq!(demand.building_actions.industrial.spawns.len(), 1);
}

#[test]
fn constructing_workplace_plans_homes_without_admission_income() {
    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "future_homes", ZoneType::Residential);
    let asset = register_test_asset(&mut allocator, "future_jobs", ZoneType::Industrial);
    let mut factory = building(ZoneType::Industrial, 0.0, 0, 0, asset);
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    factory.operating_budget = catalog
        .profile_for_id("food_processor_basic")
        .unwrap()
        .zoned_startup_budget(&catalog, tuning.owa_import_price_multiplier);
    factory.construction_total_hours = 12;
    factory.construction_remaining_hours = 12;
    allocator.buildings.push(factory);
    allocator.rebuild_zone_index();
    let mut households = HouseholdSystem::new();
    let mut graph = graph_with_connected_border();
    let mut network = crate::simulation::network::TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let zoning = residential_zoning_run(&graph);
    let mut demand = DemandSystem::new();
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
    let slots: u32 = demand
        .building_actions
        .residential
        .spawns
        .iter()
        .map(|home| allocator.registry.household_capacity(&home.asset_id))
        .sum();
    assert!(
        slots >= 10,
        "the committed factory needs housing alongside its construction: {slots}"
    );
    assert_eq!(demand.last_admission_diagnostics.move_in_job_slots, 0);
    assert_eq!(demand.households_to_admit_today, 0);
}

#[test]
fn queued_workplace_reserves_homes_without_admitting_households() {
    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "queued_homes", ZoneType::Residential);
    let factory = register_test_asset(&mut allocator, "queued_factory", ZoneType::Industrial);
    let mut households = HouseholdSystem::new();
    let mut graph = graph_with_connected_border();
    let mut network = crate::simulation::network::TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut zoning = residential_zoning_run(&graph);
    let industrial_profile = zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Industrial)
        .unwrap();
    zoning
        .place_parcel_run_at(
            10.0,
            20.0,
            40.0,
            20.0,
            industrial_profile,
            20.0,
            30.0,
            0.0,
            &graph,
        )
        .unwrap();
    let action = DemandSpawnAction {
        parcel_id: zoning
            .parcels
            .parcels()
            .iter()
            .find(|parcel| parcel.zone_profile_runtime_id() == industrial_profile)
            .unwrap()
            .id()
            .raw(),
        asset_id: factory,
    };
    let mut demand = DemandSystem::new();
    demand.set_queued_construction_with_entrances(
        &allocator,
        &zoning,
        0,
        [
            (61, ZoneType::Industrial, &action),
            (62, ZoneType::Industrial, &action),
        ],
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
    assert_eq!(
        demand.queued_construction.len(),
        1,
        "a historical duplicate cannot promise two factories"
    );
    assert_eq!(demand.queued_construction[0].completion_hour, 13);
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
    let slots: u32 = demand
        .building_actions
        .residential
        .spawns
        .iter()
        .map(|home| allocator.registry.household_capacity(&home.asset_id))
        .sum();
    assert_eq!(
        slots, 12,
        "one queued ten-worker factory needs two six-family homes"
    );
    assert_eq!(demand.last_admission_diagnostics.move_in_job_slots, 0);
    assert_eq!(demand.households_to_admit_today, 0);
    assert!(demand.building_actions.industrial.spawns.is_empty());

    // Reserving those houses in the next pass must not promise them again.
    let homes = demand.building_actions.residential.spawns.clone();
    demand.set_queued_construction_with_entrances(
        &allocator,
        &zoning,
        0,
        [(61, ZoneType::Industrial, &action)].into_iter().chain(
            homes
                .iter()
                .enumerate()
                .map(|(index, home)| (62 + index as u64, ZoneType::Residential, home)),
        ),
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
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
    assert!(demand.building_actions.residential.spawns.is_empty());
}

#[test]
fn pension_arrivals_preserve_homes_for_constructing_workplaces() {
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "worker_home", ZoneType::Residential);
    let factory = register_test_asset(&mut allocator, "future_jobs", ZoneType::Industrial);
    for _ in 0..2 {
        allocator
            .buildings
            .push(building(ZoneType::Residential, 0.0, 0, 0, home.clone()));
    }
    let mut workplace = building(ZoneType::Industrial, 1_000_000.0, 0, 0, factory);
    workplace.construction_remaining_hours = 12;
    allocator.buildings.push(workplace);
    allocator.rebuild_zone_index();
    let mut households = HouseholdSystem::new();
    let mut graph = graph_with_connected_border();
    let mut network = crate::simulation::network::TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut demand = DemandSystem::new();
    demand.run_hourly_pass(
        &allocator,
        &mut households,
        &graph,
        &empty_zoning(),
        100_000.0,
    );
    assert_eq!(demand.last_admission_diagnostics.vacant_household_slots, 12);
    assert_eq!(demand.last_admission_diagnostics.move_in_job_slots, 0);
    let mut policy = CityFiscalPolicy::from_runtime_tuning(demand.runtime_tuning());
    policy.pension_per_elder_per_day = 100.0;
    let elder = crate::simulation::economy::agents::HouseholdAgeComposition {
        elder_count: 1,
        ..Default::default()
    };
    // The places' applicants bring 15 adults (`DEM-02-C`). The factory's workers and the
    // store workers the arrivals' shopping needs fill single-adult places first.
    let unreserved = demand.workforce_reservations.unreserved_ready_homes();
    assert_eq!(unreserved, 3);
    for _ in 0..unreserved {
        assert!(demand.claim_household_admission(elder, 1, 1, &policy));
    }
    assert!(!demand.claim_household_admission(elder, 1, 1, &policy));
    assert_eq!(
        demand.last_admission_diagnostics.worker_housing_rejections,
        1
    );
    assert!(
        !demand.claim_household_admission(
            crate::simulation::economy::agents::HouseholdAgeComposition {
                adult_count: 1,
                ..Default::default()
            },
            1,
            1,
            &policy,
        ),
        "reserved housing cannot turn unfinished jobs into admission wages"
    );
    assert_eq!(demand.last_admission_diagnostics.income_rejections, 1);
}

#[test]
fn queued_homes_obey_deadlines_and_are_claimed_only_once() {
    let mut allocator = BuildingAllocator::new();
    let home = register_family_asset(
        &mut allocator,
        "queued_late_home",
        ZoneType::Residential,
        None,
        3,
    );
    let graph = graph_with_connected_border();
    let zoning = residential_zoning_run(&graph);
    let action = DemandSpawnAction {
        parcel_id: zoning.parcels.parcels()[0].id().raw(),
        asset_id: home,
    };
    let mut demand = DemandSystem::new();
    demand.set_queued_construction(
        &allocator,
        &zoning,
        0,
        [(61, ZoneType::Residential, &action)],
    );
    let mut snapshot = vacant_admission_snapshot();
    snapshot.open_job_slots = 0;
    let mut workforce = super::workforce::WorkforceReservations::default();
    workforce.reset(
        &allocator,
        &demand.runtime_catalog,
        &demand.runtime_tuning,
        &snapshot,
        &demand.queued_construction,
        demand.next_construction_start_hour,
        None,
    );
    workforce.reserve_existing();
    assert!(
        !workforce.claim(12, 1),
        "the queued home cannot complete by this factory deadline"
    );
    // Each place supplies its applicant's adults net of the store workers their shopping
    // needs (`DEM-02-C`), not one worker.
    let project = &demand.queued_construction[0];
    let store_workers = (f64::from(project.household_members)
        * super::workforce::resident_store_labor(&demand.runtime_catalog))
    .ceil() as u32;
    let workers = project.household_workers - store_workers;
    assert!(workers > 0);
    assert!(workforce.claim(24, workers));
    assert!(
        !workforce.claim(24, 1),
        "another project cannot reuse the claimed housing"
    );

    // Distant queue timestamps must not create a dense multi-billion-hour calendar.
    demand.set_queued_construction(
        &allocator,
        &zoning,
        0,
        [(u64::MAX - 1, ZoneType::Residential, &action)],
    );
    workforce.reset(
        &allocator,
        &demand.runtime_catalog,
        &demand.runtime_tuning,
        &snapshot,
        &demand.queued_construction,
        demand.next_construction_start_hour,
        None,
    );
    assert!(!workforce.claim(12, 1));
    assert!(workforce.claim(24, workers));
}

#[test]
fn queued_shop_reserves_customers_but_never_admission_wages() {
    let mut allocator = BuildingAllocator::new();
    let shop = register_test_asset(&mut allocator, "queued_shop", ZoneType::Commercial);
    let graph = graph_with_connected_border();
    let zoning = commercial_zoning_run(&graph);
    let action = DemandSpawnAction {
        parcel_id: zoning.parcels.parcels()[0].id().raw(),
        asset_id: shop,
    };
    let mut demand = DemandSystem::new();
    demand.set_queued_construction(&allocator, &zoning, 0, [(1, ZoneType::Commercial, &action)]);
    let mut snapshot = vacant_admission_snapshot();
    let profile = demand
        .runtime_catalog
        .profile_for_id("grocery_basic")
        .unwrap();
    let output = &profile.outputs[0];
    let quantity = profile.net_output_units_per_day(output);
    snapshot.committed_unmet_commercial_consumer_demand_by_resource =
        vec![(output.resource_runtime_id, quantity)];
    snapshot.output_absorption = OutputAbsorptionContext::from_resource_amounts(
        demand.runtime_catalog.resource_count(),
        &[],
        &[(output.resource_runtime_id, quantity)],
        1,
        &[],
    );
    let live_jobs = snapshot.move_in_job_slots;
    let live_wages = snapshot.average_move_in_job_wage_per_day;
    snapshot.reserve_queued_construction(&demand.runtime_catalog, &demand.queued_construction);
    assert_eq!(
        snapshot
            .output_absorption
            .unmet_units(output.resource_runtime_id),
        0.0
    );
    assert_eq!(
        snapshot.committed_unmet_commercial_consumer_demand_by_resource[0].1,
        0.0
    );
    assert_eq!(snapshot.move_in_job_slots, live_jobs);
    assert_eq!(snapshot.average_move_in_job_wage_per_day, live_wages);
}

#[test]
fn max_demand_cheat_plans_industrial_spawn_without_input_need() {
    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
    let mut households = HouseholdSystem::new();
    let mut graph = graph_with_connected_border();
    graph.set_node_type(0, NodeType::Junction);
    let zoning = industrial_zoning_run(&graph);
    let mut demand = DemandSystem::new();

    demand.enable_max_demand_cheat();
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 1_000_000.0);

    assert_eq!(
        demand.building_actions.industrial.spawns.len(),
        demand
            .last_building_action_diagnostics
            .industrial
            .spawn_candidates,
    );
    assert_eq!(
        demand
            .last_building_action_diagnostics
            .industrial
            .spawn_rejected_absorption,
        0
    );
}

#[test]
fn residential_construction_bootstraps_before_vacant_homes_exist() {
    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    let employer = register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
    allocator
        .buildings
        .push(building(ZoneType::Industrial, 0.0, 0, 0, employer));

    let mut households = HouseholdSystem::new();
    // This snapshot fixture supplies the already evaluated employment funding result.
    households.job_funding.capacities = vec![10];
    let graph = graph_with_connected_border();
    let zoning = residential_zoning_run(&graph);
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);

    assert!(
        !demand.building_actions.residential.spawns.is_empty(),
        "healthy construction-side move-in viability should allow residential spawns"
    );
    assert!(demand.last_admission_diagnostics.incoming_household_need > 0.0);
    assert_eq!(demand.last_admission_diagnostics.vacant_household_slots, 0);
    assert_eq!(demand.last_admission_diagnostics.planned_households, 0);
}

#[test]
fn residential_construction_stops_when_existing_capacity_covers_live_opportunity() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));

    let mut households = HouseholdSystem::new();
    households.households.push(housed_household(0, 1, 0.0, 3.0));
    let graph = graph_with_connected_border();
    let zoning = residential_zoning_run(&graph);
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, -100.0);

    assert_eq!(demand.building_actions.residential.spawns.len(), 0);
    assert_eq!(demand.last_admission_diagnostics.move_in_job_slots, 0);
}

#[test]
fn hourly_pass_produces_startup_household_admission_when_capacity_jobs_and_border_exist() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    let commercial_asset = register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        0,
        0,
        residential_asset,
    ));
    allocator.buildings.push(building(
        ZoneType::Commercial,
        500.0,
        0,
        0,
        commercial_asset,
    ));
    allocator.rebuild_zone_index();

    let mut households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 1_000.0);

    assert!(demand.households_to_admit_today > 0);
}

#[test]
fn rejected_applicant_does_not_block_later_affordable_households() {
    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(&mut allocator, "applicant_home", ZoneType::Residential);
    let factory_asset = register_test_asset(&mut allocator, "applicant_jobs", ZoneType::Industrial);
    allocator
        .buildings
        .push(building(ZoneType::Residential, 0.0, 0, 0, home_asset));
    allocator
        .buildings
        .push(building(ZoneType::Industrial, 0.0, 0, 0, factory_asset));
    allocator.rebuild_zone_index();
    let mut households = HouseholdSystem::new();
    let mut graph = graph_with_connected_border();
    let mut network = crate::simulation::network::TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    allocator.buildings.last_mut().unwrap().operating_budget = 100_000.0;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();
    let policy = CityFiscalPolicy::from_runtime_tuning(demand.runtime_tuning());
    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
    assert!(demand.households_to_admit_today > 0);
    assert!(!demand.claim_household_admission(
        HouseholdAgeComposition {
            adult_count: 1,
            child_count: 4,
            elder_count: 0
        },
        5,
        1,
        &policy,
    ));
    assert!(demand.claim_household_admission(
        HouseholdAgeComposition {
            adult_count: 1,
            ..Default::default()
        },
        1,
        1,
        &policy,
    ));
    assert_eq!(demand.last_admission_diagnostics.examined_candidates, 2);
    assert_eq!(demand.last_admission_diagnostics.income_rejections, 1);
}

#[test]
fn job_funding_changes_do_not_release_employed_workers_to_new_projects() {
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "occupied_home", ZoneType::Residential);
    let employer = register_test_asset(&mut allocator, "assigned_employer", ZoneType::Industrial);
    allocator
        .buildings
        .push(building(ZoneType::Residential, 0.0, 1, 0, home));
    allocator
        .buildings
        .push(building(ZoneType::Industrial, 0.0, 0, 3, employer));
    allocator.rebuild_zone_index();
    let mut households = HouseholdSystem::new();
    households
        .households
        .push(housed_household(0, 5, 1_000.0, 3.0));
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().unwrap();
    for (funded, assigned, unemployed, open) in
        [(6, 3, 2, 3), (1, 3, 2, 0), (0, 3, 2, 0), (0, 0, 5, 0)]
    {
        households.job_funding.capacities = vec![0, funded];
        allocator.buildings[1].worker_count = assigned;
        households.households[0].employed_adults = assigned as u16;
        let snapshot =
            DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
        assert_eq!(snapshot.existing_unemployed_member_count, unemployed);
        assert_eq!(snapshot.open_job_slots, open);
    }
}

#[test]
fn unhoused_employees_do_not_consume_housed_unemployed_workers() {
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "unemployment_home", ZoneType::Residential);
    let employer =
        register_test_asset(&mut allocator, "unemployment_factory", ZoneType::Industrial);
    allocator
        .buildings
        .push(building(ZoneType::Residential, 0.0, 1, 0, home));
    allocator
        .buildings
        .push(building(ZoneType::Industrial, 0.0, 0, 3, employer));
    allocator.rebuild_zone_index();
    let mut households = HouseholdSystem::new();
    let mut housed = housed_household(0, 3, 1_000.0, 3.0);
    housed.employed_adults = 1;
    let mut displaced = housed_household(usize::MAX, 3, 1_000.0, 3.0);
    displaced.employed_adults = 2;
    households.households.extend([housed, displaced]);
    households.job_funding.capacities = vec![0, 3];
    let snapshot = DailyDemandSnapshot::from_runtime(
        &allocator,
        &households,
        &graph_with_connected_border(),
        &load_builtin_demand_config().unwrap(),
        100_000.0,
    );
    assert_eq!(snapshot.existing_unemployed_member_count, 2);
    assert_eq!(snapshot.unhoused_workforce.len(), 1);
    assert_eq!(snapshot.unhoused_workforce[0].workers, 1);
}

#[test]
fn runtime_snapshot_uses_existing_unemployed_before_new_household_job_pull() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "net_job_home", ZoneType::Residential);
    let industrial_asset =
        register_test_asset(&mut allocator, "net_job_factory", ZoneType::Industrial);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));
    let mut factory = building(ZoneType::Industrial, 0.0, 0, 1, industrial_asset);
    factory.operating_budget = 10_000.0;
    allocator.buildings.push(factory);
    allocator.rebuild_zone_index();

    let mut households = HouseholdSystem::new();
    // This snapshot fixture supplies the already evaluated employment funding result.
    households.job_funding.capacities = vec![0, 10];
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let before =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert!(before.move_in_job_equivalent_slots > 0.0);
    households.pending_arrivals.households = 5;
    households.pending_arrivals.adults = before.open_job_slots;
    households.pending_arrivals.elders = 2;
    households.pending_arrivals.children = 3;
    let pending =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(pending.move_in_job_slots, 0);
    assert_eq!(pending.existing_elder_count, 2);
    assert_eq!(pending.existing_child_count, 3);

    households.pending_arrivals = Default::default();
    households
        .households
        .push(housed_household(0, 20, 10_000.0, 3.0));
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);

    assert!(snapshot.open_job_slots > 0);
    assert!(snapshot.existing_unemployed_member_count >= snapshot.open_job_slots);
    assert_eq!(snapshot.move_in_job_slots, 0);
    assert_eq!(snapshot.move_in_job_slots, 0);
    assert!(
        snapshot.move_in_job_equivalent_slots <= 0.001,
        "raw open jobs should not pull immigrants while existing unemployed adults can fill them"
    );
}

#[test]
fn job_driven_admission_prefers_worker_candidate_over_workerless_front_candidate() {
    const TEST_FLAT_SIZE_M2: f32 = 65.5;

    let next_household_id = 0;
    let mut workerless_home_idx = None;
    let mut worker_home_idx = None;
    for building_idx in 0..256 {
        let Some(candidate_size) =
            candidate_immigrant_household_size_for_vacancy(TEST_FLAT_SIZE_M2, building_idx, 0)
        else {
            continue;
        };
        if candidate_size != 1 {
            continue;
        }
        let composition =
            household_age_composition(building_idx, next_household_id, candidate_size);
        if composition.adult_count == 0 {
            workerless_home_idx.get_or_insert(building_idx);
        } else if workerless_home_idx.is_some() {
            worker_home_idx = Some(building_idx);
            break;
        }
    }
    let workerless_home_idx =
        workerless_home_idx.expect("test seed should find a workerless front candidate");
    let worker_home_idx =
        worker_home_idx.expect("test seed should find a worker-capable later candidate");

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_family_asset_with_economy_profile_and_flat_size(
        &mut allocator,
        "worker_preferred_home",
        ZoneType::Residential,
        None,
        1,
        None,
        Some(TEST_FLAT_SIZE_M2),
    );
    let industrial_asset = register_test_asset(
        &mut allocator,
        "worker_preferred_jobs",
        ZoneType::Industrial,
    );

    for building_idx in 0..=worker_home_idx {
        let zone_type = if building_idx == workerless_home_idx || building_idx == worker_home_idx {
            ZoneType::Residential
        } else {
            ZoneType::None
        };
        allocator
            .buildings
            .push(building(zone_type, 0.0, 0, 0, residential_asset.clone()));
    }
    let mut factory = building(ZoneType::Industrial, 0.0, 0, 0, industrial_asset);
    factory.operating_budget = 100_000.0;
    allocator.buildings.push(factory);
    allocator.rebuild_zone_index();

    let normal_candidate = allocator
        .next_household_admission_candidate_for_household(next_household_id, false)
        .expect("normal candidate should exist");
    assert_eq!(normal_candidate.0, workerless_home_idx);
    assert_eq!(
        household_age_composition(normal_candidate.0, next_household_id, normal_candidate.1)
            .adult_count,
        0
    );

    let preferred_candidate = allocator
        .next_household_admission_candidate_for_household(next_household_id, true)
        .expect("preferred candidate should exist");
    assert_eq!(preferred_candidate.0, worker_home_idx);
    assert!(
        household_age_composition(
            preferred_candidate.0,
            next_household_id,
            preferred_candidate.1
        )
        .adult_count
            > 0
    );

    let mut households = HouseholdSystem::new();
    let mut graph = graph_with_connected_border();
    let mut network = crate::simulation::network::TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    allocator.buildings.last_mut().unwrap().operating_budget = 100_000.0;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, -20_000.0);

    let diagnostics = demand.last_admission_diagnostics;
    assert!(diagnostics.open_job_slots > 0);
    assert_eq!(diagnostics.existing_unemployed_member_count, 0);
    let policy = CityFiscalPolicy::from_runtime_tuning(demand.runtime_tuning());
    assert!(
        demand.claim_household_admission(
            household_age_composition(
                preferred_candidate.0,
                next_household_id,
                preferred_candidate.1
            ),
            preferred_candidate.1,
            1,
            &policy,
        ),
        "negative treasury must not block an affordable working household"
    );
    assert!(demand.households_to_admit_today > 0);
}

#[test]
fn runtime_snapshot_counts_cached_explicit_work_area_jobs_when_commercial_floor_is_zero() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "explicit_work_home", ZoneType::Residential);
    let farm_asset =
        register_explicit_profile_asset(&mut allocator, "explicit_work_farm", "grain_farm_basic");
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog must load");
    let farm_profile = catalog
        .profile_for_id("grain_farm_basic")
        .expect("grain farm profile");
    let mut farm = building(
        ZoneType::Industrial,
        0.0,
        0,
        farm_profile.worker_capacity,
        farm_asset,
    );
    farm.zone_type = ZoneType::None;
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.operating_budget = 100_000.0;
    farm.work_area_scale = 1.0;
    farm.commercial_activity_floor_scale = 1.0;
    allocator.buildings.push(farm);
    allocator.rebuild_zone_index();

    let mut households = HouseholdSystem::new();
    // This snapshot fixture supplies the already evaluated employment funding result.
    households.job_funding.capacities = vec![0, 2];
    households.households.push(housed_household(
        0,
        farm_profile.worker_capacity as u16,
        10_000.0,
        3.0,
    ));

    households.households[0].employed_adults = farm_profile.worker_capacity as u16;
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);

    assert_eq!(snapshot.physical_worker_capacity, 2);
    assert_eq!(snapshot.funded_worker_capacity, 2);
    assert_eq!(snapshot.open_job_slots, 1);
    assert_eq!(snapshot.existing_unemployed_member_count, 0);
}

#[test]
fn farm_snapshot_counts_one_home_alongside_area_scaled_jobs() {
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_utility_asset(&mut allocator, "farm_household", "grain_farm_basic");
    let mut manifest = allocator.registry.get(&asset).unwrap().manifest.clone();
    let authored = manifest.building.as_mut().unwrap();
    authored.service_class = None;
    authored.household_capacity = Some(9); // Farm housing is exactly one slot, even in an authored override.
    authored.field = Some(crate::assets::asset::BuildingFieldData {
        resource: "grain".into(),
        area_mode: "player_polygon".into(),
    });
    allocator.registry.register("test", manifest, String::new());
    let mut farm = building(ZoneType::None, 0.0, 0, 0, asset);
    farm.economy_profile_runtime_id = load_runtime_economy_catalog()
        .unwrap()
        .profile_for_id("grain_farm_basic")
        .unwrap()
        .runtime_id;
    farm.work_area_scale = 30.0;
    farm.commercial_activity_floor_scale = 1.0;
    farm.operating_budget = 10_000.0;
    allocator.buildings.push(farm);
    allocator.rebuild_zone_index();
    let graph = graph_with_connected_border();
    let mut households = HouseholdSystem::new();
    // This snapshot fixture supplies the already evaluated employment funding result.
    households.job_funding.capacities = vec![3];
    let config = load_builtin_demand_config().unwrap();
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(snapshot.vacant_household_slots, 1);
    assert_eq!(snapshot.open_job_slots, 3);
    allocator.claim_vacancy(0);
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(snapshot.vacant_household_slots, 0);
    assert_eq!(snapshot.open_job_slots, 3);
}

#[test]
fn temporary_benefits_cannot_admit_households_without_live_jobs() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(housed_household(0, 2, 1_000.0, 3.0));
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    for _ in 0..48 {
        demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
        if demand.households_to_admit_today > 0 {
            break;
        }
    }

    assert_eq!(demand.last_admission_diagnostics.move_in_job_slots, 0);
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(
        demand.runtime_tuning(),
    );
    assert!(
        !demand.claim_household_admission(
            crate::simulation::economy::agents::HouseholdAgeComposition {
                adult_count: 1,
                ..Default::default()
            },
            1,
            1,
            &policy,
        ),
        "temporary benefits cannot fund an actual worker admission"
    );
}

#[test]
fn admission_requires_external_connection() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(housed_household(0, 2, 1_000.0, 3.0));
    let graph = RegionGraph::new();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    for _ in 0..24 {
        demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
    }

    assert_eq!(demand.last_admission_diagnostics.connected_border_count, 0);
    assert_eq!(demand.households_to_admit_today, 0);
}

#[test]
fn hourly_pass_sizes_large_vacant_home_from_starter_mix_not_full_capacity() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_family_asset_with_economy_profile_and_flat_size(
        &mut allocator,
        "large_residential",
        ZoneType::Residential,
        None,
        1,
        None,
        Some(200.0),
    );
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        0,
        0,
        residential_asset,
    ));
    allocator.rebuild_zone_index();

    let mut households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);

    let candidate = allocator
        .next_household_admission_candidate_for_household(0, false)
        .unwrap();
    assert_eq!(
        candidate.1, 1,
        "actual applicants use the starter mix, not full flat capacity"
    );
    let policy = CityFiscalPolicy::from_runtime_tuning(demand.runtime_tuning());
    assert!(!demand.claim_household_admission(
        HouseholdAgeComposition {
            adult_count: 1,
            ..Default::default()
        },
        candidate.1,
        1,
        &policy,
    ));
}

#[test]
fn actual_admission_requires_income_when_household_economy_is_failing() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));

    let mut households = HouseholdSystem::new();
    households.households.push(housed_household(0, 1, 0.0, 0.0));
    for _ in 0..3 {
        households.households.push(unhoused_household(1, 0.0, 0.0));
    }

    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, -100.0);

    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(
        demand.runtime_tuning(),
    );
    assert!(!demand.claim_household_admission(
        crate::simulation::economy::agents::HouseholdAgeComposition {
            adult_count: 1,
            ..Default::default()
        },
        1,
        1,
        &policy,
    ));
    assert_eq!(demand.last_admission_diagnostics.income_rejections, 1);
}

#[test]
fn household_admission_diagnostics_record_actual_opportunity() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        0,
        0,
        residential_asset,
    ));

    let mut households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 1_000.0);
    let diagnostics = demand.last_admission_diagnostics;

    assert_eq!(diagnostics.total_household_count, 0);
    assert_eq!(
        diagnostics.vacant_household_slots,
        allocator.household_capacity(0)
    );
    assert_eq!(diagnostics.connected_border_count, 1);
    assert!(diagnostics.incoming_household_need > 0.0);
    assert_eq!(
        diagnostics.planned_households,
        demand.households_to_admit_today
    );

    demand.record_household_admission_execution(1);

    assert_eq!(demand.last_admission_diagnostics.launched_households, 1);
}

#[test]
fn newly_displaced_households_do_not_leave_even_when_most_of_the_city_is_unhoused() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(housed_household(0, 1, 200.0, 3.0));
    households.households.push(unhoused_household(1, 0.0, 0.0));
    households.households.push(unhoused_household(1, 0.0, 0.0));

    let mut demand = DemandSystem::new();

    demand.run_daily_pass(&allocator, &households);
    let diagnostics = demand.last_removal_diagnostics;

    assert_eq!(diagnostics.total_household_count, 3);
    assert_eq!(diagnostics.housed_household_count, 1);
    assert_eq!(diagnostics.unhoused_household_count, 2);
    assert_eq!(diagnostics.zero_budget_household_count, 2);
    assert_eq!(diagnostics.persistent_exit_eligible_household_count, 0);
    assert_eq!(diagnostics.planned_households, 0);
    assert_eq!(
        diagnostics.planned_households,
        demand.households_to_remove_today
    );

    demand.record_household_removal_execution(2);

    assert_eq!(demand.last_removal_diagnostics.removed_households, 2);
}

#[test]
fn household_exit_pacing_does_not_depend_on_citywide_homelessness() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        7,
        0,
        residential_asset,
    ));

    for housed_count in [0, 7, 100] {
        let mut households = HouseholdSystem::new();
        for _ in 0..housed_count {
            households
                .households
                .push(housed_household(0, 1, 200.0, 3.0));
        }
        for _ in 0..8 {
            let mut household = unhoused_household(1, 0.0, 0.0);
            household.unhoused_days_elapsed = 2;
            households.households.push(household);
        }

        let mut demand = DemandSystem::new();

        demand.run_daily_pass(&allocator, &households);
        let diagnostics = demand.last_removal_diagnostics;

        assert_eq!(diagnostics.unhoused_household_count, 8);
        assert_eq!(diagnostics.total_household_count, housed_count + 8);
        assert_eq!(diagnostics.persistent_exit_eligible_household_count, 8);
        assert_eq!(diagnostics.persistent_exit_planned_households, 2);
        assert_eq!(demand.households_to_remove_today, 2);
    }
}

#[test]
fn persistent_exit_counts_cash_that_cannot_fund_a_day_of_needs() {
    let allocator = BuildingAllocator::new();
    let graph = graph_with_connected_border();
    let mut households = HouseholdSystem::new();
    let mut household = unhoused_household(2, 20.0, 0.0);
    household.unhoused_days_elapsed = 2;
    households.households.push(household);
    let config = load_builtin_demand_config().unwrap();
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);
    assert_eq!(snapshot.zero_budget_household_count, 0);
    assert_eq!(snapshot.persistent_exit_eligible_household_count, 1);

    households.households[0].budget = 100.0;
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);
    assert_eq!(snapshot.persistent_exit_eligible_household_count, 0);
}

#[test]
fn household_failures_and_city_size_do_not_veto_affordable_admission() {
    let mut snapshot = vacant_admission_snapshot();
    let config = load_builtin_demand_config().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(&load_runtime_economy_tuning().unwrap());
    let worker = HouseholdAgeComposition {
        adult_count: 1,
        ..Default::default()
    };
    for total in [10, 600, 10_000] {
        snapshot.total_household_count = total;
        snapshot.zero_budget_household_ratio = 0.9;
        snapshot.unhoused_household_ratio = 0.9;
        snapshot.city_treasury_balance = 0.0;
        let mut budget =
            super::admission::HouseholdAdmissionBudget::from_snapshot(&snapshot, &config);
        assert!(
            budget.claim(worker, 1, 1, &policy),
            "affordable worker blocked at {total} households"
        );
    }
}

#[test]
fn actual_admission_accounts_for_transfer_treasury_coverage() {
    let mut snapshot = vacant_admission_snapshot();
    snapshot.existing_unemployed_member_count = 100;
    snapshot.move_in_job_equivalent_slots = 0.0;
    snapshot.daily_essential_cost_per_resident = 20.0;
    let config = load_builtin_demand_config().unwrap();
    let mut policy = CityFiscalPolicy::from_runtime_tuning(&load_runtime_economy_tuning().unwrap());
    policy.pension_per_elder_per_day = 30.0;
    let elder = HouseholdAgeComposition {
        elder_count: 1,
        ..Default::default()
    };
    for (treasury, expected) in [(100_000.0, true), (0.0, false)] {
        snapshot.city_treasury_balance = treasury;
        let mut budget =
            super::admission::HouseholdAdmissionBudget::from_snapshot(&snapshot, &config);
        assert_eq!(budget.claim(elder, 1, 1, &policy), expected);
    }
}

#[test]
fn runtime_snapshot_counts_unhoused_children_and_elders_as_transfer_claimants() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "transfer_claimant_candidate_home",
        ZoneType::Residential,
    );
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        0,
        0,
        residential_asset,
    ));
    let mut households = HouseholdSystem::new();
    let mut child_household = unhoused_household(1, 60.0, 1.0);
    child_household.child_count = 1;
    child_household.adult_count = 0;
    child_household.elder_count = 0;
    let mut elder_household = unhoused_household(1, 60.0, 1.0);
    elder_household.child_count = 0;
    elder_household.adult_count = 0;
    elder_household.elder_count = 1;
    households.households.push(child_household);
    households.households.push(elder_household);

    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);

    assert_eq!(snapshot.housed_household_count, 0);
    assert_eq!(snapshot.unhoused_household_count, 2);
    assert_eq!(snapshot.existing_child_count, 1);
    assert_eq!(snapshot.existing_elder_count, 1);

    // Existing dependent residents consume transfer headroom before an arriving pensioner.
    let mut snapshot = snapshot;
    snapshot.pension_daily_benefit_per_elder = 50.0;
    snapshot.child_support_daily_benefit_per_child = 50.0;
    snapshot.daily_essential_cost_per_resident = 40.0;
    snapshot.city_treasury_balance = 50.0
        * config
            .household_action
            .move_in_benefit_treasury_coverage_days;
    let mut policy = CityFiscalPolicy::from_runtime_tuning(&load_runtime_economy_tuning().unwrap());
    policy.pension_per_elder_per_day = 50.0;
    let elder = HouseholdAgeComposition {
        elder_count: 1,
        ..Default::default()
    };
    let mut claimed = super::admission::HouseholdAdmissionBudget::from_snapshot(&snapshot, &config);
    assert!(!claimed.claim(elder, 1, 1, &policy));
    snapshot.existing_child_count = 0;
    snapshot.existing_elder_count = 0;
    let mut empty = super::admission::HouseholdAdmissionBudget::from_snapshot(&snapshot, &config);
    assert!(empty.claim(elder, 1, 1, &policy));
}

#[test]
fn actual_elder_admission_uses_pension_not_unemployment() {
    let snapshot = vacant_admission_snapshot();
    let config = load_builtin_demand_config().unwrap();
    let mut policy = CityFiscalPolicy::from_runtime_tuning(&load_runtime_economy_tuning().unwrap());
    policy.unemployment_benefit_per_adult_per_day = 100.0;
    let elder = HouseholdAgeComposition {
        elder_count: 1,
        ..Default::default()
    };
    for (pension, expected) in [(50.0, true), (0.0, false)] {
        policy.pension_per_elder_per_day = pension;
        let mut budget =
            super::admission::HouseholdAdmissionBudget::from_snapshot(&snapshot, &config);
        assert_eq!(budget.claim(elder, 1, 1, &policy), expected);
    }
}

#[test]
fn actual_working_household_can_enter_without_benefits() {
    let mut snapshot = vacant_admission_snapshot();
    snapshot.city_treasury_balance = 0.0;
    let config = load_builtin_demand_config().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(&load_runtime_economy_tuning().unwrap());
    let mut budget = super::admission::HouseholdAdmissionBudget::from_snapshot(&snapshot, &config);
    assert!(budget.claim(
        HouseholdAgeComposition {
            adult_count: 2,
            ..Default::default()
        },
        2,
        1,
        &policy
    ));
}

#[test]
fn marginal_commercial_jobs_do_not_fund_arrivals_when_current_store_cap_is_filled() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let grocery = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile");
    let household_profile = catalog
        .profile_for_id("basic_household_demand")
        .expect("household demand profile");
    let household_supply_resource = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    let household_supply_output = grocery
        .output_port(household_supply_resource)
        .expect("grocery household-supply output")
        .units_per_day;
    let candidate_household_size =
        candidate_immigrant_household_size_from_flat_size(80.0).expect("80m2 starter flat");
    let target_active_workers = (grocery.worker_capacity / 3)
        .max(1)
        .min(grocery.worker_capacity.saturating_sub(1));
    let current_demand_threshold =
        target_active_workers as f32 * household_supply_output / grocery.worker_capacity as f32;
    let resident_count = (current_demand_threshold
        / household_profile
            .consumption_rate_per_resident
            .max(f32::EPSILON))
    .floor()
    .max(1.0) as u16;
    let current_demand_units =
        resident_count as f32 * household_profile.consumption_rate_per_resident.max(0.0);
    let candidate_demand_units =
        candidate_household_size as f32 * household_profile.consumption_rate_per_resident.max(0.0);
    let marginal_active_workers = (grocery.worker_capacity as f32
        * (current_demand_units + candidate_demand_units)
        / household_supply_output)
        .ceil() as u32;
    assert_eq!(
        (grocery.worker_capacity as f32 * current_demand_units / household_supply_output).ceil()
            as u32,
        target_active_workers
    );
    assert!(
        marginal_active_workers > target_active_workers,
        "the candidate household should lift the sales-scaled grocery staffing cap"
    );

    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "marginal_job_home", ZoneType::Residential);
    let commercial_asset =
        register_test_asset(&mut allocator, "marginal_job_shop", ZoneType::Commercial);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        1,
        0,
        residential_asset,
    ));
    let mut shop = building(
        ZoneType::Commercial,
        0.0,
        0,
        target_active_workers,
        commercial_asset,
    );
    shop.operating_budget = grocery.average_daily_wage() * (target_active_workers + 1) as f32;
    allocator.buildings.push(shop);

    let mut households = HouseholdSystem::new();
    // This snapshot fixture supplies the already evaluated employment funding result.
    households.job_funding.capacities = vec![0, target_active_workers];
    let mut demand_household = housed_household(
        0,
        resident_count,
        10_000.0,
        household_profile.stock_target_days,
    );
    demand_household.adult_count = target_active_workers as u16;
    demand_household.child_count = resident_count.saturating_sub(demand_household.adult_count);
    households.households.push(demand_household);

    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let fiscal_policy = CityFiscalPolicy::from_runtime_tuning(tuning.as_ref());
    let snapshot = DailyDemandSnapshot::from_runtime_with_catalog(
        &allocator,
        &households,
        &graph,
        config.as_ref(),
        catalog.as_ref(),
        tuning.as_ref(),
        &fiscal_policy,
        100_000.0,
    );

    assert_eq!(snapshot.open_job_slots, 0);
    assert_eq!(snapshot.move_in_job_slots, 0);

    assert_eq!(snapshot.open_job_slots, 0);
    assert_eq!(snapshot.move_in_job_slots, 0);
    let mut budget = super::admission::HouseholdAdmissionBudget::from_snapshot(&snapshot, &config);
    assert!(!budget.claim(
        HouseholdAgeComposition {
            adult_count: 1,
            ..Default::default()
        },
        1,
        1,
        &fiscal_policy
    ));
}

#[test]
fn service_funding_limits_open_jobs_in_demand_snapshot() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let mut allocator = BuildingAllocator::new();
    let power_asset =
        register_test_utility_asset(&mut allocator, "funded_power", "power_plant_basic");
    let power_profile = catalog
        .profile_for_id("power_plant_basic")
        .expect("power profile");
    let mut power = building(ZoneType::None, 0.0, 0, 2, power_asset);
    power.economy_profile_runtime_id = power_profile.runtime_id;
    allocator.buildings.push(power);

    let mut households = HouseholdSystem::new();
    // This snapshot fixture supplies the already evaluated employment funding result.
    households.job_funding.capacities = vec![power_profile.worker_capacity];
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let fiscal_policy = CityFiscalPolicy::from_runtime_tuning(tuning.as_ref());

    let fully_funded = DailyDemandSnapshot::from_runtime_with_catalog(
        &allocator,
        &households,
        &graph,
        &config,
        catalog.as_ref(),
        tuning.as_ref(),
        &fiscal_policy,
        100_000.0,
    );
    assert_eq!(
        fully_funded.open_job_slots,
        power_profile.worker_capacity - 2
    );
    assert_eq!(fully_funded.open_jobs_unfunded, 0);

    households.job_funding.capacities[0] = 2;
    let defunded = DailyDemandSnapshot::from_runtime_with_catalog(
        &allocator,
        &households,
        &graph,
        &config,
        catalog.as_ref(),
        tuning.as_ref(),
        &fiscal_policy,
        100_000.0,
    );

    assert_eq!(
        defunded.physical_worker_capacity,
        power_profile.worker_capacity
    );
    assert_eq!(defunded.funded_worker_capacity, 2);
    assert_eq!(
        defunded.open_jobs_unfunded,
        power_profile.worker_capacity - 2
    );
    assert_eq!(defunded.open_job_slots, 0);
    assert_eq!(defunded.move_in_job_slots, 0);
}

#[test]
fn load_diagnostics_refresh_does_not_advance_action_state() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "load_refresh_home", ZoneType::Residential);
    let employer_asset =
        register_test_asset(&mut allocator, "load_refresh_jobs", ZoneType::Industrial);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        0,
        0,
        residential_asset,
    ));
    allocator
        .buildings
        .push(building(ZoneType::Industrial, 100.0, 0, 0, employer_asset));

    let mut households = HouseholdSystem::new();
    // This snapshot fixture supplies the already evaluated employment funding result.
    households.job_funding.capacities = vec![0, 10];
    let graph = graph_with_connected_border();
    let mut demand = DemandSystem::new();
    demand.households_to_admit_today = 3;
    demand.upgrade_action_credit.residential = 1.75;

    demand.refresh_household_diagnostics(
        &allocator,
        &households,
        &graph,
        100_000.0,
        &CityFiscalPolicy::default(),
    );

    assert_eq!(demand.last_admission_diagnostics.move_in_job_slots, 10);
    assert_eq!(demand.last_admission_diagnostics.planned_households, 3);
    assert_eq!(demand.households_to_admit_today, 3);
    assert_eq!(demand.upgrade_action_credit.residential, 1.75);
    assert!(demand.building_actions.residential.spawns.is_empty());
    assert!(demand.building_actions.commercial.spawns.is_empty());
    assert!(demand.building_actions.industrial.spawns.is_empty());
}

#[test]
fn deserted_residential_buildings_do_not_count_as_capacity_or_housed_households() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    let mut home = building(ZoneType::Residential, 0.0, 1, 0, residential_asset);
    home.is_deserted = true;
    allocator.buildings.push(home);

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(housed_household(0, 2, 100.0, 3.0));

    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);
    let occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);

    assert_eq!(snapshot.vacant_household_slots, 0);
    assert_eq!(snapshot.housed_household_count, 0);
    assert_eq!(snapshot.unhoused_household_count, 1);
    assert_eq!(occupants.household_count_by_building[0], 0);
}

#[test]
fn residential_upgrade_requires_current_household_affordability_for_target_level() {
    let mut allocator = BuildingAllocator::new();
    let level_one = register_family_asset(
        &mut allocator,
        "res_level_1",
        ZoneType::Residential,
        Some("res_family"),
        1,
    );
    let _level_two = register_family_asset(
        &mut allocator,
        "res_level_2",
        ZoneType::Residential,
        Some("res_family"),
        2,
    );
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        6,
        0,
        level_one.clone(),
    ));

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(housed_household(0, 6, 200.0, 3.0));

    let demand = DemandSystem::new();
    let economy_tuning = load_runtime_economy_tuning().expect("runtime economy tuning must load");
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog must load");
    let residential_occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);
    let low_affordability = demand.collect_existing_building_candidates(
        &allocator,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Residential,
    );
    assert!(low_affordability.upgrades.is_empty());

    households.households[0].budget = 1_200.0;
    let residential_occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);
    let high_affordability = demand.collect_existing_building_candidates(
        &allocator,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Residential,
    );
    assert_eq!(high_affordability.upgrades.len(), 1);
}

#[test]
fn commercial_upgrade_requires_business_viability_not_only_pressure() {
    let mut allocator = BuildingAllocator::new();
    let level_one = register_family_asset(
        &mut allocator,
        "com_level_1",
        ZoneType::Commercial,
        Some("com_family"),
        1,
    );
    let _level_two = register_family_asset(
        &mut allocator,
        "com_level_2",
        ZoneType::Commercial,
        Some("com_family"),
        2,
    );
    let mut shop = building(ZoneType::Commercial, 50.0, 0, 1, level_one);
    shop.operating_budget = 20.0;
    allocator.buildings.push(shop);

    let households = HouseholdSystem::new();
    let demand = DemandSystem::new();
    let economy_tuning = load_runtime_economy_tuning().expect("runtime economy tuning must load");
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog must load");
    let residential_occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);

    let weak_viability = demand.collect_existing_building_candidates(
        &allocator,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
    );
    assert!(weak_viability.upgrades.is_empty());

    allocator.buildings[0].worker_count = 15;
    // Collection leaves finances to target-operation selection, which also values stock/cargo.
    allocator.buildings[0].operating_budget = 0.0;
    let strong_viability = demand.collect_existing_building_candidates(
        &allocator,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
    );
    assert_eq!(strong_viability.upgrades.len(), 1);
}

#[test]
fn viable_upgrade_does_not_depend_on_citywide_pressure() {
    let mut allocator = BuildingAllocator::new();
    let level_one = register_family_asset(
        &mut allocator,
        "com_hysteresis_level_1",
        ZoneType::Commercial,
        Some("com_hysteresis_family"),
        1,
    );
    let _level_two = register_family_asset(
        &mut allocator,
        "com_hysteresis_level_2",
        ZoneType::Commercial,
        Some("com_hysteresis_family"),
        2,
    );
    let mut shop = building(ZoneType::Commercial, 50.0, 0, 15, level_one);
    shop.operating_budget = 6_000.0;
    allocator.buildings.push(shop);

    let households = HouseholdSystem::new();
    let economy_tuning = load_runtime_economy_tuning().expect("runtime economy tuning must load");
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog must load");
    let residential_occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);

    let demand = DemandSystem::new();

    let candidates = demand.collect_existing_building_candidates(
        &allocator,
        &catalog,
        &economy_tuning,
        &residential_occupants,
        ZoneType::Commercial,
    );
    assert_eq!(
        candidates.upgrades.len(),
        1,
        "local viability must permit an upgrade"
    );
}

#[test]
fn only_deserted_empty_buildings_are_cleared_and_never_downgraded() {
    let mut allocator = BuildingAllocator::new();
    let level_one = register_family_asset(
        &mut allocator,
        "com_deserted_level_1",
        ZoneType::Commercial,
        Some("com_deserted_family"),
        1,
    );
    let level_two = register_family_asset(
        &mut allocator,
        "com_deserted_level_2",
        ZoneType::Commercial,
        Some("com_deserted_family"),
        2,
    );

    let mut healthy_empty = building(ZoneType::Commercial, 0.0, 0, 0, level_one);
    healthy_empty.cell_x = 0;
    let mut deserted_empty = building(ZoneType::Commercial, 0.0, 0, 0, level_two.clone());
    deserted_empty.level = 2;
    deserted_empty.cell_x = 1;
    deserted_empty.is_deserted = true;
    allocator.buildings.push(healthy_empty);
    allocator.buildings.push(deserted_empty);

    let households = HouseholdSystem::new();
    let demand = DemandSystem::new();
    let economy_tuning = load_runtime_economy_tuning().expect("runtime economy tuning must load");
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog must load");
    let residential_occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);
    let candidates = demand.collect_existing_building_candidates(
        &allocator,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
    );
    assert_eq!(candidates.despawns.len(), 1);
    assert_eq!(
        candidates.despawns[0].action.asset_id.as_str(),
        level_two.as_str()
    );
    assert!(candidates.downgrades.is_empty());

    allocator.buildings.clear();
    let mut staffed_deserted = building(ZoneType::Commercial, 0.0, 0, 1, level_two);
    staffed_deserted.level = 2;
    staffed_deserted.is_deserted = true;
    allocator.buildings.push(staffed_deserted);
    let candidates = demand.collect_existing_building_candidates(
        &allocator,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
    );
    assert!(candidates.despawns.is_empty());
    assert!(candidates.downgrades.is_empty());
}

#[test]
fn service_opening_requires_first_worker_and_overhead_from_unreserved_market() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    let prices = OperationPrices {
        utilities: crate::simulation::economy::households::local_utility_service_costs(&catalog)
            .map(|price| price * tuning.owa_import_price_multiplier),
        property_tax: crate::simulation::economy::fiscal::daily_property_tax(
            ZoneType::Commercial,
            1,
            &policy,
        ),
        imports: tuning.owa_import_price_multiplier,
        carrier_quote: tuning.logistics.dispatch_fee_currency,
        truck_payload: tuning.logistics.truck_payload_kg,
    };
    for (id, demand_id) in [
        ("personal_service_small", "personal_service_demand"),
        ("health_essentials_small", "health_essentials_demand"),
    ] {
        let profile = catalog.profile_for_id(id).unwrap();
        let demand = catalog.profile_for_id(demand_id).unwrap();
        let resource = profile.outputs[0].resource_runtime_id;
        let rate = demand.consumption_rate_per_resident;
        let revenue_per_resident = rate * catalog.unit_price_for_resource(resource).unwrap();
        assert!(profile.inputs.is_empty());
        let fixed_cost = profile.average_daily_wage()
            + profile.electricity.idle_kwh_per_day * prices.utilities[0]
            + prices.utilities[1]
            + prices.utilities[2]
            + prices.property_tax;
        let power_per_resident =
            profile.electricity.operating_kwh_per_day * prices.utilities[0] * rate
                / profile.outputs[0].units_per_day;
        let minimum_residents =
            (fixed_cost / (revenue_per_resident - power_per_resident)).floor() as u32 + 1;
        assert!(
            minimum_residents <= 60,
            "{id}: a village must fund its first shop"
        );
        let market = |residents| {
            OutputAbsorptionContext::from_resource_amounts(
                catalog.resource_count(),
                &[],
                &[(resource, rate)],
                residents,
                &[],
            )
        };
        assert!(
            !commercial_market_covers_operation(
                profile,
                &catalog,
                &market(minimum_residents - 1),
                prices,
            ),
            "{id}: unfunded first worker"
        );
        let mut viable = market(minimum_residents);
        assert!(
            commercial_market_covers_operation(profile, &catalog, &viable, prices,),
            "{id}: viable first worker"
        );
        viable.reserve_candidate(profile);
        assert!(
            !commercial_market_covers_operation(profile, &catalog, &viable, prices,),
            "{id}: buyers already committed"
        );
        assert!(
            !commercial_market_covers_operation(
                profile,
                &catalog,
                &market(minimum_residents),
                OperationPrices {
                    property_tax: prices.property_tax + revenue_per_resident,
                    ..prices
                },
            ),
            "higher property/utility bills must increase the required market"
        );
    }
}

#[test]
fn industrial_openings_require_profitable_unreserved_customers() {
    use super::spawn_need::industrial_market_covers_operation;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    let prices = OperationPrices {
        utilities: crate::simulation::economy::households::local_utility_service_costs(&catalog)
            .map(|price| price * tuning.owa_import_price_multiplier),
        property_tax: crate::simulation::economy::fiscal::daily_property_tax(
            ZoneType::Industrial,
            1,
            &policy,
        ),
        imports: tuning.owa_import_price_multiplier,
        carrier_quote: tuning.logistics.dispatch_fee_currency,
        truck_payload: tuning.logistics.truck_payload_kg,
    };
    for id in ["food_processor_basic", "machinery_factory_basic"] {
        let profile = catalog.profile_for_id(id).unwrap();
        let resource = profile.outputs[0].resource_runtime_id;
        let mut market = OutputAbsorptionContext::from_resource_amounts(
            catalog.resource_count(),
            &[],
            &[],
            0,
            &[(resource, 1.0)],
        );
        let covers = |market: &OutputAbsorptionContext, imports, exports| {
            industrial_market_covers_operation(
                profile,
                &catalog,
                market,
                OperationPrices { imports, ..prices },
                exports,
            )
        };
        assert!(
            !covers(&market, 1.25, 1.0),
            "one unit of input need cannot fund {id}"
        );
        market.set_export_expansion_available(true);
        assert!(
            covers(&market, 1.25, 1.0),
            "regional customers should support {id}"
        );
        assert!(
            !covers(&market, 1.25, 0.45),
            "loss-making regional bids cannot justify {id}"
        );
        assert!(
            !covers(&market, 4.0, 1.0),
            "input costs must be funded for {id}"
        );

        market.reserve_candidate(profile);
        assert!(
            covers(&market, 1.25, 1.0),
            "outside buyers remain available to independent {id} entrants"
        );
        market.set_export_expansion_available(false);
        assert!(
            !covers(&market, 1.25, 1.0),
            "local customers cannot be reserved twice for {id}"
        );
    }
    let allocator = BuildingAllocator::new();
    let households = HouseholdSystem::new();
    let config = load_builtin_demand_config().unwrap();
    let closed = DailyDemandSnapshot::from_runtime(
        &allocator,
        &households,
        &RegionGraph::new(),
        &config,
        100_000.0,
    );
    let food = catalog.resource_runtime_id_for_id("packaged_food").unwrap();
    assert_eq!(closed.output_absorption.unmet_units(food), 0.0);
    let open = DailyDemandSnapshot::from_runtime(
        &allocator,
        &households,
        &graph_with_connected_border(),
        &config,
        100_000.0,
    );
    assert_eq!(open.output_absorption.unmet_units(food), 0.0);
    assert!(open.output_absorption.export_expansion_available());
    assert!(!closed.output_absorption.export_expansion_available());
}

#[test]
fn export_opportunity_does_not_depend_on_other_factories_recovering_startup_cash() {
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "independent_factory", ZoneType::Industrial);
    allocator
        .buildings
        .push(building(ZoneType::Industrial, 0.0, 0, 0, asset));
    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().unwrap();
    for (cash, workers, remaining, deserted) in [
        (0.0, 0, 0, false),
        (1_000_000.0, 10, 0, false),
        (0.0, 0, 12, false),
        (-100.0, 0, 0, true),
    ] {
        let incumbent = &mut allocator.buildings[0];
        incumbent.operating_budget = cash;
        incumbent.worker_count = workers;
        incumbent.construction_remaining_hours = remaining;
        incumbent.is_deserted = deserted;
        let snapshot =
            DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
        assert!(
            snapshot.output_absorption.export_expansion_available(),
            "an unrelated incumbent cannot veto an export opportunity"
        );
    }
}

#[test]
fn clearance_depends_on_abandonment_not_citywide_pressure() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    for zone in [
        ZoneType::Residential,
        ZoneType::Commercial,
        ZoneType::Industrial,
    ] {
        let mut allocator = BuildingAllocator::new();
        let asset = register_test_asset(&mut allocator, "clearance", zone);
        let healthy = building(zone, 0.0, 0, 0, asset);
        let mut failed = healthy.clone();
        failed.is_deserted = true;
        failed.cell_x = 1;
        allocator.buildings.extend([healthy, failed]);
        let households = HouseholdSystem::new();
        let occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);
        let demand = DemandSystem::new();

        let candidates = demand
            .collect_existing_building_candidates(&allocator, &catalog, &tuning, &occupants, zone);
        assert_eq!(candidates.despawns.len(), 1);
        assert_eq!(candidates.despawns[0].action.cell_x, 1);
    }
}

#[test]
fn deserted_sites_recover_at_positive_demand_without_removing_healthy_capacity() {
    for use_kind in [
        DemandUse::Residential,
        DemandUse::Commercial,
        DemandUse::Industrial,
    ] {
        let zone_type = use_kind.zone_type();
        let mut allocator = BuildingAllocator::new();
        let asset_id = register_test_asset(&mut allocator, "failed_business", zone_type);
        let mut failed = building(zone_type, 0.0, 0, 0, asset_id.clone());
        failed.is_deserted = true;
        let mut staffed = failed.clone();
        staffed.worker_count = 1;
        staffed.cell_x = 1;
        let mut occupied = failed.clone();
        occupied.occupancy = 1;
        occupied.cell_x = 2;
        let mut healthy = building(zone_type, 0.0, 0, 0, asset_id);
        healthy.cell_x = 3;
        allocator
            .buildings
            .extend([failed, staffed, occupied, healthy]);
        allocator.rebuild_zone_index();

        let graph = graph_with_connected_border();
        let mut households = HouseholdSystem::new();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();
        demand.cheat_max_demands_enabled = true;
        let mut selected = 0;
        // At the shipped half-per-day budget, a single failed site takes two
        // days of credit. Allow one extra hour for f32 accumulation rounding.
        for hour in 1..=49 {
            demand.run_hourly_pass(&allocator, &mut households, &graph, &zoning, 100_000.0);
            assert!(demand.cheat_max_demands_enabled);
            let actions = &demand.building_actions.use_plan_mut(use_kind).despawns;
            assert!(actions.iter().all(|action| action.cell_x == 0));
            selected += actions.len();
            if hour <= 24 {
                assert_eq!(selected, 0, "clearance bypassed the action budget");
            }
        }
        assert_eq!(selected, 1, "positive demand stranded the failed site");
    }
}

#[test]
fn existing_building_candidates_follow_attachment_order_before_parcel_id() {
    let mut allocator = BuildingAllocator::new();
    let asset_id =
        register_test_asset(&mut allocator, "com_attachment_order", ZoneType::Commercial);

    let mut low_parcel_late_edge = building(ZoneType::Commercial, 0.0, 0, 0, asset_id.clone());
    low_parcel_late_edge.parcel_id = 1;
    low_parcel_late_edge.edge_idx = 8;
    low_parcel_late_edge.cell_x = 0;
    low_parcel_late_edge.is_deserted = true;

    let mut high_parcel_early_edge = building(ZoneType::Commercial, 0.0, 0, 0, asset_id);
    high_parcel_early_edge.parcel_id = 99;
    high_parcel_early_edge.edge_idx = 2;
    high_parcel_early_edge.cell_x = 0;
    high_parcel_early_edge.is_deserted = true;

    allocator.buildings.push(low_parcel_late_edge);
    allocator.buildings.push(high_parcel_early_edge);

    let households = HouseholdSystem::new();
    let demand = DemandSystem::new();
    let economy_tuning = load_runtime_economy_tuning().expect("runtime economy tuning must load");
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog must load");
    let residential_occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);
    let candidates = demand.collect_existing_building_candidates(
        &allocator,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
    );

    assert_eq!(candidates.despawns.len(), 2);
    assert_eq!(candidates.despawns[0].action.edge_idx, 2);
    assert_eq!(candidates.despawns[0].action.parcel_id, 99);
}

#[test]
fn industrial_upgrade_uses_shipped_profile_viability_gates() {
    let mut allocator = BuildingAllocator::new();
    let level_one = register_family_asset(
        &mut allocator,
        "ind_level_1",
        ZoneType::Industrial,
        Some("ind_family"),
        1,
    );
    let _level_two = register_family_asset(
        &mut allocator,
        "ind_level_2",
        ZoneType::Industrial,
        Some("ind_family"),
        2,
    );
    let mut factory = building(ZoneType::Industrial, 50.0, 0, 10, level_one);
    factory.operating_budget = 4_000.0;
    allocator.buildings.push(factory);

    let households = HouseholdSystem::new();
    let demand = DemandSystem::new();
    let economy_tuning = load_runtime_economy_tuning().expect("runtime economy tuning must load");
    let residential_occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog must load");
    let starter_factory = catalog
        .profile_for_id("food_processor_basic")
        .expect("food processor starter profile");
    assert_eq!(
        starter_factory
            .inputs
            .first()
            .and_then(|port| catalog.resource_id_for_runtime_id(port.resource_runtime_id)),
        Some("grain"),
        "shipped starter industrial profile consumes grain"
    );
    if let Some(input_port) = starter_factory.inputs.first() {
        allocator.buildings[0].set_inventory_units(input_port.resource_runtime_id, 320.0);
    }

    allocator.buildings[0].set_inventory_units(
        catalog.resource_runtime_id_for_id("machinery").unwrap(),
        80.0,
    );

    let starter_headroom = demand.collect_existing_building_candidates(
        &allocator,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Industrial,
    );
    assert_eq!(starter_headroom.upgrades.len(), 1);

    if let Some(output_port) = starter_factory.outputs.first() {
        allocator.buildings[0].set_inventory_units(output_port.resource_runtime_id, 50.0);
    }
    let same_profile = demand.collect_existing_building_candidates(
        &allocator,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Industrial,
    );
    assert_eq!(same_profile.upgrades.len(), 1);

    if let Some(output_port) = starter_factory.outputs.first() {
        allocator.buildings[0].set_inventory_units(output_port.resource_runtime_id, 630.0);
    }
    let jammed_output = demand.collect_existing_building_candidates(
        &allocator,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Industrial,
    );
    assert!(jammed_output.upgrades.is_empty());
}

#[test]
fn machinery_demand_comes_from_operating_customers_and_reserves_new_factories() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
    let factory_profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let mut allocator = BuildingAllocator::new();
    let farm_asset = register_explicit_profile_asset(&mut allocator, "farm", "grain_farm_basic");
    let factory_asset = register_family_asset_with_economy_profile(
        &mut allocator,
        "machinery",
        ZoneType::Industrial,
        None,
        1,
        Some("machinery_factory_basic"),
    );
    let food_asset = register_test_asset(&mut allocator, "food", ZoneType::Industrial);
    let mut households = HouseholdSystem::new();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = CityFiscalPolicy::from_runtime_tuning(&tuning);
    let mut funding_graph = graph_with_connected_border();
    let mut network = crate::simulation::network::TransitNetwork::new();
    network.lane_system.rebuild(&mut funding_graph);
    network.rebuild_cch_and_check(&funding_graph);
    let mut logistics = crate::simulation::economy::logistics::ShipmentSystem::new();
    // Isolate local customers from the separately tested regional market.
    let graph = RegionGraph::new();
    let config = load_builtin_demand_config().unwrap();
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert!(!nonresidential_passes_absorption_gate(
        &allocator,
        &catalog,
        &snapshot.output_absorption,
        &factory_asset,
    ));

    let mut farm = building(ZoneType::None, 0.0, 0, 6, farm_asset);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 400.0;
    farm.commercial_activity_floor_scale = 1.0;
    farm.worker_count =
        crate::simulation::economy::households::physical_worker_capacity_for_profile(
            &farm,
            farm_profile,
        );
    farm.operating_budget = 10_000_000.0;
    farm.center_x = 25.0;
    farm.center_y = 20.0;
    farm.frontage_t = 0.5;
    allocator.buildings.push(farm);
    allocator.rebuild_entrance_cache(&funding_graph, &network.lane_system);
    let refresh =
        |households: &mut HouseholdSystem,
         allocator: &BuildingAllocator,
         logistics: &mut crate::simulation::economy::logistics::ShipmentSystem| {
            households.job_funding.refresh(
                allocator,
                &households.households,
                logistics,
                &network,
                &funding_graph,
                &catalog,
                &tuning,
                &policy,
                &[],
                100_000.0,
                420,
            );
        };
    refresh(&mut households, &allocator, &mut logistics);
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(snapshot.output_absorption.unmet_units(machinery), 40.0);
    allocator.buildings[0].operating_budget = 0.0;
    refresh(&mut households, &allocator, &mut logistics);
    let idle_snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(
        idle_snapshot.output_absorption.unmet_units(machinery),
        0.0,
        "retained workers without working capital do not demand Machinery"
    );
    allocator.buildings[0].operating_budget = 10_000_000.0;
    refresh(&mut households, &allocator, &mut logistics);
    assert!(nonresidential_passes_absorption_gate(
        &allocator,
        &catalog,
        &snapshot.output_absorption,
        &factory_asset,
    ));
    assert!(!nonresidential_passes_absorption_gate(
        &allocator,
        &catalog,
        &snapshot.output_absorption,
        &food_asset
    ));
    let mut reserved = snapshot.output_absorption.clone();
    reserved.reserve_candidate(factory_profile);
    assert!(!nonresidential_passes_absorption_gate(
        &allocator,
        &catalog,
        &reserved,
        &factory_asset
    ));

    let mut factory = building(ZoneType::Industrial, 0.0, 0, 0, factory_asset.clone());
    factory.economy_profile_runtime_id = factory_profile.runtime_id;
    factory.construction_remaining_hours = 12;
    allocator.buildings.push(factory);
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(snapshot.output_absorption.unmet_units(machinery), 0.0);
    assert!(!nonresidential_passes_absorption_gate(
        &allocator,
        &catalog,
        &snapshot.output_absorption,
        &factory_asset,
    ));
    allocator.buildings[1].construction_remaining_hours = 0;
    allocator.buildings[1].worker_count = 4;
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(
        snapshot.output_absorption.unmet_units(machinery),
        0.0,
        "42 gross output also covers the factory's own two units"
    );
    assert!(!nonresidential_passes_absorption_gate(
        &allocator,
        &catalog,
        &snapshot.output_absorption,
        &factory_asset,
    ));
}

#[test]
fn machinery_demand_counts_funded_utilities_but_not_unstaffed_or_unfunded_producers() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_utility_asset(&mut allocator, "power", "power_plant_basic");
    let profile = catalog.profile_for_id("power_plant_basic").unwrap();
    let mut power = building(ZoneType::None, 0.0, 0, 20, asset);
    power.economy_profile_runtime_id = profile.runtime_id;
    allocator.buildings.push(power);
    let mut households = HouseholdSystem::new();
    // Public invoices follow the fiscal staffing ceiling even without material delivery.
    households.job_funding.capacities = vec![profile.worker_capacity];
    // Isolate local customers from the separately tested regional market.
    let graph = RegionGraph::new();
    let config = load_builtin_demand_config().unwrap();
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(snapshot.output_absorption.unmet_units(machinery), 4.0);
    allocator.buildings[0].worker_count = 0;
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(snapshot.output_absorption.unmet_units(machinery), 0.0);
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    allocator.buildings[0].asset_id =
        register_test_asset(&mut allocator, "unfunded_processor", ZoneType::Industrial);
    allocator.buildings[0].zone_type = ZoneType::Industrial;
    allocator.buildings[0].economy_profile_runtime_id = profile.runtime_id;
    allocator.buildings[0].worker_count = 10;
    for output in &profile.outputs {
        allocator.buildings[0].set_inventory_units(
            output.resource_runtime_id,
            profile.output_buffer_capacity_units_for(output),
        );
    }
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);
    assert_eq!(snapshot.output_absorption.unmet_units(machinery), 0.0);
}

#[test]
#[ignore = "manual matched release timing of business demand snapshots"]
fn benchmark_business_demand_snapshot() {
    use std::hint::black_box;
    use std::time::Instant;
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().unwrap();
    let households = HouseholdSystem::new();
    for count in [1_024, 8_192, 65_536] {
        let mut allocator = BuildingAllocator::new();
        let mut factory = building(ZoneType::Industrial, 0.0, 0, 10, "test:factory".to_owned());
        factory.operating_budget = 10_000.0;
        for port in &profile.inputs {
            factory.set_inventory_units(
                port.resource_runtime_id,
                (port.units_per_day * 4.0).max(80.0),
            );
        }
        allocator.buildings = vec![factory; count];
        let snapshot = || {
            DailyDemandSnapshot::from_runtime(
                black_box(&allocator),
                &households,
                &graph,
                &config,
                100_000.0,
            )
        };
        for _ in 0..10 {
            black_box(snapshot());
        }
        let mut samples = [0.0; 11];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..100 {
                black_box(snapshot());
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 100.0;
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "business_demand_snapshot buildings={count} median_ms={:.3}",
            samples[5]
        );
    }
}

#[test]
fn demand_snapshot_is_bitwise_identical_across_worker_counts() {
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "ordered_home", ZoneType::Residential);
    let store = register_test_asset(&mut allocator, "ordered_store", ZoneType::Commercial);
    allocator
        .buildings
        .push(building(ZoneType::Residential, 0.0, 0, 0, home));
    for idx in 0..4_099 {
        let mut grocery = building(ZoneType::Commercial, 0.0, 0, 0, store.clone());
        grocery.operating_budget = 10_000.0;
        grocery.revenue = (idx % 37) as f32 * 0.17;
        allocator.buildings.push(grocery);
    }
    let mut households = HouseholdSystem::new();
    households.households = (0..8_193)
        .map(|idx| {
            let mut household = housed_household(0, 2, 73.0 + (idx % 41) as f32 * 0.13, 0.0);
            household.stock = (idx % 97) as f32 * 0.017;
            household.stock_days = household.stock / 2.0;
            household
        })
        .collect();
    let config = load_builtin_demand_config().unwrap();
    let graph = graph_with_connected_border();
    let results: Vec<_> = [1, 2, 7]
        .into_iter()
        .map(|workers| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    let snapshot = DailyDemandSnapshot::from_runtime(
                        &allocator,
                        &households,
                        &graph,
                        &config,
                        100_000.0,
                    );
                    let scalar_bits = [
                        snapshot.move_in_job_equivalent_slots,
                        snapshot.average_move_in_job_wage_per_day,
                    ]
                    .map(f32::to_bits);
                    let resource_bits: Vec<_> = snapshot
                        .committed_unmet_commercial_consumer_demand_by_resource
                        .into_iter()
                        .map(|(resource, units)| (resource, units.to_bits()))
                        .collect();
                    (scalar_bits, resource_bits)
                })
        })
        .collect();
    assert!(
        results.iter().all(|result| *result == results[0]),
        "worker-dependent snapshots: {results:?}"
    );
}
