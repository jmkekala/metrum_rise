// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: tests.rs
//  script_path: rust/src/simulation/economy/demand/tests.rs
//  module_name: tests
//  version: 0.1.0
//  description: Demand system regression tests: spawn need credit
//  kind: test
//  spec: none
//  internal_dependencies: [demand, households, zoning]
//  external_dependencies: [godot]
//  features: [demand-spawn, absorption-gate, spawn-credit]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// ========================================================================

//! Demand tests.

use super::credits::advance_spawn_need_credit;
use super::spawn_need::{OutputAbsorptionContext, nonresidential_passes_absorption_gate};
use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::economy::agents::household_age_composition;
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::{
    Household, HouseholdSystem, REPLENISHMENT_STABLE,
    candidate_immigrant_household_size_for_vacancy,
    candidate_immigrant_household_size_from_flat_size,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{
    EdgeClass, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::{Vector2, Vector3};

// ========================================================================
// FIXTURES
// ========================================================================

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
        budget,
        stock: stock_days * member_count as f32,
        member_count,
        child_count: 0,
        adult_count: member_count,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days,
        replenishment_state: REPLENISHMENT_STABLE,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
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
        lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
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
        frontage_class: Default::default(),
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
        vacant_household_slots: 10,
        under_construction_household_slots: 0,
        total_household_count: 4,
        housed_household_count: 4,
        unhoused_household_count: 0,
        zero_budget_household_count: 0,
        persistent_exit_eligible_household_count: 0,
        unhoused_household_ratio: 0.0,
        zero_budget_household_ratio: 0.0,
        housing_availability: 1.0,
        incoming_household_need: 1.0,
        open_job_household_pull: 1.0,
        marginal_commercial_job_household_pull: 0.0,
        regional_growth_household_pull: 0.0,
        household_affordability: 1.0,
        household_stock_stability: 1.0,
        commercial_capacity_deficit: 0.0,
        unmet_commercial_consumer_demand: 0.0,
        committed_unmet_commercial_consumer_demand: 0.0,
        committed_unmet_commercial_consumer_demand_by_resource: Vec::new(),
        industrial_input_capacity_deficit: 0.0,
        commercial_input_need_value: 0.0,
        local_industrial_input_capacity_value: 0.0,
        industrial_missing_input_value: 0.0,
        committed_industrial_missing_input_value: 0.0,
        external_connection_available: 1.0,
        connected_border_count: 1,
        city_treasury_balance: 100_000.0,
        candidate_household_size: 2.0,
        candidate_child_count: 0,
        candidate_adult_count: 2,
        candidate_elder_count: 0,
        immigrant_starter_savings_per_household: 30.0,
        candidate_daily_essential_cost: 56.0,
        unemployment_daily_benefit_per_adult: 30.0,
        pension_daily_benefit_per_elder: 30.0,
        child_support_daily_benefit_per_child: 10.0,
        existing_unemployed_member_count: 0,
        existing_child_count: 0,
        existing_elder_count: 0,
        open_job_slots: 0,
        marginal_commercial_job_slots: 0,
        marginal_commercial_job_equivalent_slots: 0.0,
        move_in_job_slots: 0,
        move_in_job_equivalent_slots: 0.0,
        average_move_in_job_wage_per_day: 0.0,
        physical_worker_capacity: 0,
        funded_worker_capacity: 0,
        open_jobs_unfunded: 0,
        output_absorption: OutputAbsorptionContext::empty(0),
        commercial_owa_dependency: 0.0,
        commercial_owa_input_value: 0.0,
    }
}

// ========================================================================
// DEMAND TESTS
// ========================================================================

#[test]
fn daily_pass_raises_commercial_and_industrial_pressure_on_shortages() {
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    // Simulate commercial building that has been buying inputs from OWA:
    // daily_owa_input_value > 0 drives industrial demand.
    let mut com = building(ZoneType::Commercial, 80.0, 0, 1, commercial_asset);
    com.daily_owa_input_value = 100.0;
    allocator.buildings.push(com);
    // occupancy=2 so resident_presence > 0, allowing organic commercial/industrial pressure.
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        2,
        0,
        residential_asset,
    ));

    let mut households = HouseholdSystem::new();
    // home_building_id=1 (residential is now at index 1 after commercial at 0)
    households
        .households
        .push(housed_household(1, 2, 120.0, 0.25));

    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();
    demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);

    assert!(demand.commercial > 0.0);
    assert!(demand.industrial > 0.0);
}

#[test]
fn daily_pass_raises_commercial_pressure_when_residents_lack_shop_capacity() {
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
        .push(housed_household(0, 5, 1_000.0, 3.0));

    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();
    demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);

    assert!(
        demand.commercial > 0.95,
        "commercial demand should anticipate missing shop capacity, got={:.3}",
        demand.commercial
    );
}

#[test]
fn daily_pass_uses_short_run_purchase_power_for_missing_shop_capacity() {
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
        .push(housed_household(0, 5, 140.0, 3.0));

    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();
    demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);

    assert!(
        demand.commercial > 0.95,
        "one reserve day should still represent immediate grocery buying power, got={:.3}",
        demand.commercial
    );
}

#[test]
fn industrial_pressure_uses_capacity_balance_and_owa_dependency() {
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    let industrial_asset = register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
    let mut commercial = building(ZoneType::Commercial, 40.0, 0, 1, commercial_asset);
    commercial.daily_owa_input_value = 13_860.0;
    allocator.buildings.push(commercial);

    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let missing_snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);

    assert!(missing_snapshot.commercial_owa_dependency > 0.99);
    assert_eq!(missing_snapshot.commercial_input_need_value, 160.0);
    assert_eq!(missing_snapshot.local_industrial_input_capacity_value, 0.0);
    assert_eq!(missing_snapshot.industrial_missing_input_value, 160.0);
    assert_eq!(missing_snapshot.industrial_input_capacity_deficit, 1.0);

    let mut demand = DemandSystem::new();
    demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);
    assert!(
        demand.industrial > 0.95,
        "missing industrial input capacity should drive industrial pressure"
    );

    allocator
        .buildings
        .push(building(ZoneType::Industrial, 0.0, 0, 0, industrial_asset));
    let covered_snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);

    assert!(covered_snapshot.commercial_owa_dependency > 0.99);
    assert_eq!(covered_snapshot.commercial_input_need_value, 160.0);
    assert_eq!(
        covered_snapshot.local_industrial_input_capacity_value,
        2_400.0
    );
    assert_eq!(covered_snapshot.industrial_missing_input_value, 0.0);
    assert_eq!(covered_snapshot.industrial_input_capacity_deficit, 0.0);

    let mut covered_demand = DemandSystem::new();
    covered_demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);
    assert!(
        covered_demand.industrial > 0.95,
        "actual OWA reliance should still raise industrial pressure even when paper local capacity covers need"
    );
}

#[test]
fn industrial_pressure_takes_owa_dependency_as_secondary_need_signal() {
    let mut snapshot = vacant_admission_snapshot();
    snapshot.industrial_input_capacity_deficit = 0.0;
    snapshot.commercial_owa_dependency = 0.625;

    let mut demand = DemandSystem::new();
    demand.update_pressure_channels_from_snapshot(&snapshot);

    assert!(
        (demand.industrial - 0.625).abs() < 0.001,
        "industrial pressure should use normalized OWA dependency when capacity deficit is lower"
    );
}

#[test]
fn commercial_spawn_uses_open_jobs_as_pull_not_full_workforce_prerequisite() {
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

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 100_000.0);

    assert!(
        required_workers > 5,
        "regression setup must cover a profile whose full staffing exceeds the starter household"
    );
    assert!(
        demand.commercial > 0.95,
        "starter residents without shop capacity should create commercial pressure, got={:.3}",
        demand.commercial
    );
    assert_eq!(
        demand.building_actions.commercial.spawns.len(),
        1,
        "commercial spawn should be selected so its open jobs can pull the next households"
    );
    assert_eq!(
        demand
            .last_building_action_diagnostics
            .commercial
            .spawn_rejected_labour,
        0
    );
}

#[test]
fn daily_pass_raises_residential_pressure_when_jobs_outrun_housing() {
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
    let commercial_asset = register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    allocator.buildings.push(building(
        ZoneType::Industrial,
        300.0,
        0,
        1,
        industrial_asset,
    ));
    allocator.buildings.push(building(
        ZoneType::Commercial,
        500.0,
        0,
        1,
        commercial_asset,
    ));
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        5,
        0,
        residential_asset,
    ));

    let mut households = HouseholdSystem::new();
    for _ in 0..5 {
        households
            .households
            .push(housed_household(2, 1, 120.0, 3.0));
    }

    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();
    demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);

    assert!(demand.residential > 0.50);
}

#[test]
fn commercial_spawn_need_rounds_unmet_output_to_missing_buildings() {
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let candidates = [DemandSpawnCandidate {
        action: DemandSpawnAction {
            parcel_id: 1,
            asset_id: commercial_asset,
        },
        density: "low".to_owned(),
    }];
    let mut snapshot = vacant_admission_snapshot();

    snapshot.committed_unmet_commercial_consumer_demand = 1.0;
    assert_eq!(
        commercial_spawn_need_buildings(&allocator, &catalog, &snapshot, &candidates),
        1.0
    );

    snapshot.committed_unmet_commercial_consumer_demand = 201.0;
    assert_eq!(
        commercial_spawn_need_buildings(&allocator, &catalog, &snapshot, &candidates),
        2.0
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

    let candidates = allocator.collect_demand_spawn_candidates_by_use(
        &zoning,
        &graph,
        catalog.as_ref(),
        &[(personal_services, 1.0)],
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
fn commercial_spawn_need_averages_output_only_across_matching_resource_candidates() {
    let mut allocator = BuildingAllocator::new();
    let grocery = register_family_asset_with_economy_profile(
        &mut allocator,
        "need_grocery",
        ZoneType::Commercial,
        None,
        1,
        Some("grocery_basic"),
    );
    let barber = register_family_asset_with_economy_profile(
        &mut allocator,
        "need_barber",
        ZoneType::Commercial,
        None,
        1,
        Some("personal_service_small"),
    );
    let pharmacy = register_family_asset_with_economy_profile(
        &mut allocator,
        "need_pharmacy",
        ZoneType::Commercial,
        None,
        1,
        Some("health_essentials_small"),
    );
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let personal_services = catalog
        .resource_runtime_id_for_id("personal_services")
        .expect("personal_services resource");
    let candidates = [
        DemandSpawnCandidate {
            action: DemandSpawnAction {
                parcel_id: 1,
                asset_id: grocery,
            },
            density: "low".to_owned(),
        },
        DemandSpawnCandidate {
            action: DemandSpawnAction {
                parcel_id: 2,
                asset_id: barber,
            },
            density: "low".to_owned(),
        },
        DemandSpawnCandidate {
            action: DemandSpawnAction {
                parcel_id: 3,
                asset_id: pharmacy,
            },
            density: "low".to_owned(),
        },
    ];
    let mut snapshot = vacant_admission_snapshot();
    snapshot.committed_unmet_commercial_consumer_demand = 160.0;
    snapshot.committed_unmet_commercial_consumer_demand_by_resource =
        vec![(personal_services, 160.0)];

    assert_eq!(
        commercial_spawn_need_buildings(&allocator, &catalog, &snapshot, &candidates),
        2.0
    );
}

#[test]
fn industrial_spawn_need_rounds_missing_input_capacity_to_missing_buildings() {
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let candidates = [DemandSpawnCandidate {
        action: DemandSpawnAction {
            parcel_id: 1,
            asset_id: industrial_asset,
        },
        density: "low".to_owned(),
    }];
    let mut snapshot = vacant_admission_snapshot();

    snapshot.commercial_input_need_value = 2_400.0;
    snapshot.industrial_missing_input_value = 2_400.0;
    snapshot.committed_industrial_missing_input_value = 2_400.0;
    assert_eq!(
        industrial_spawn_need_buildings(&allocator, &catalog, &snapshot, &candidates),
        1.0
    );

    snapshot.industrial_missing_input_value = 2_401.0;
    snapshot.committed_industrial_missing_input_value = 2_401.0;
    assert_eq!(
        industrial_spawn_need_buildings(&allocator, &catalog, &snapshot, &candidates),
        2.0
    );

    snapshot.commercial_owa_input_value = 13_860.0;
    snapshot.industrial_missing_input_value = 0.0;
    snapshot.committed_industrial_missing_input_value = 0.0;
    assert_eq!(
        industrial_spawn_need_buildings(&allocator, &catalog, &snapshot, &candidates),
        0.0
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
fn residential_spawn_need_uses_incoming_pull_not_only_vacancy_reserve() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    let candidates = [DemandSpawnCandidate {
        action: DemandSpawnAction {
            parcel_id: 1,
            asset_id: residential_asset,
        },
        density: "low".to_owned(),
    }];
    let mut snapshot = vacant_admission_snapshot();
    snapshot.total_household_count = 8;
    snapshot.vacant_household_slots = 1;
    snapshot.incoming_household_need = 0.0;

    assert_eq!(
        residential_spawn_need_buildings(&allocator, &snapshot, &candidates),
        0.0,
        "one reserve vacancy should satisfy residential need when there is no incoming pull"
    );

    snapshot.incoming_household_need = 1.4;
    assert_eq!(
        residential_spawn_need_buildings(&allocator, &snapshot, &candidates),
        1.0,
        "incoming household pull should request capacity even when one reserve home is vacant"
    );
}

#[test]
fn residential_spawn_need_counts_under_construction_slots_as_committed_capacity() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "residential", ZoneType::Residential);
    let candidates = [DemandSpawnCandidate {
        action: DemandSpawnAction {
            parcel_id: 1,
            asset_id: residential_asset,
        },
        density: "low".to_owned(),
    }];
    let mut snapshot = vacant_admission_snapshot();
    snapshot.total_household_count = 8;
    snapshot.vacant_household_slots = 0;
    snapshot.incoming_household_need = 1.4;

    assert_eq!(
        residential_spawn_need_buildings(&allocator, &snapshot, &candidates),
        1.0
    );

    snapshot.under_construction_household_slots = 6;
    assert_eq!(
        residential_spawn_need_buildings(&allocator, &snapshot, &candidates),
        0.0,
        "pending residential construction should reserve enough committed slots to prevent duplicate spawns"
    );
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

    assert!(snapshot.unmet_commercial_consumer_demand > 0.0);
    let expected_uncommitted_service_demand = 5.0 * 0.03 + 5.0 * 0.05;
    assert!(
        (snapshot.committed_unmet_commercial_consumer_demand - expected_uncommitted_service_demand)
            .abs()
            < 0.001,
        "pending grocery construction should not satisfy unrelated service-store demand"
    );
    assert!(snapshot.commercial_capacity_deficit > 0.0);

    let candidates = [DemandSpawnCandidate {
        action: DemandSpawnAction {
            parcel_id: 1,
            asset_id: commercial_asset,
        },
        density: "low".to_owned(),
    }];
    assert_eq!(
        commercial_spawn_need_buildings(&allocator, catalog.as_ref(), &snapshot, &candidates),
        0.0,
        "pending commercial output should block duplicate spawns without hiding live shortage"
    );
}

#[test]
fn commercial_spawn_need_uses_effective_shop_capacity_after_stock_recovery_demand() {
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
        snapshot.committed_unmet_commercial_consumer_demand > 90.0,
        "one 200/day grocery should not hide a 300/day pantry recovery demand"
    );

    let candidates = [DemandSpawnCandidate {
        action: DemandSpawnAction {
            parcel_id: 1,
            asset_id: commercial_asset,
        },
        density: "low".to_owned(),
    }];
    assert_eq!(
        commercial_spawn_need_buildings(&allocator, catalog.as_ref(), &snapshot, &candidates),
        1.0
    );
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
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);

    assert_eq!(snapshot.commercial_input_need_value, 160.0);
    assert_eq!(snapshot.local_industrial_input_capacity_value, 0.0);
    assert_eq!(snapshot.industrial_missing_input_value, 160.0);
    assert_eq!(snapshot.committed_industrial_missing_input_value, 0.0);
    assert_eq!(snapshot.industrial_input_capacity_deficit, 1.0);

    let candidates = [DemandSpawnCandidate {
        action: DemandSpawnAction {
            parcel_id: 1,
            asset_id: industrial_asset,
        },
        density: "low".to_owned(),
    }];
    assert_eq!(
        industrial_spawn_need_buildings(&allocator, catalog.as_ref(), &snapshot, &candidates),
        0.0,
        "pending industrial output should block duplicate spawns without hiding live shortage"
    );
}

#[test]
fn spawn_need_credit_is_not_hourly_cadence_throttled() {
    let mut credit = 0.0;

    assert_eq!(advance_spawn_need_credit(&mut credit, 0.5, 10), 0);
    assert_eq!(credit, 0.5);
    assert_eq!(advance_spawn_need_credit(&mut credit, 0.5, 10), 1);
    assert_eq!(credit, 0.0);
    assert_eq!(advance_spawn_need_credit(&mut credit, 1.0, 1), 1);
    assert_eq!(credit, 0.0);
}

#[test]
fn daily_pass_blocks_growth_without_external_connection() {
    let allocator = BuildingAllocator::new();
    let households = HouseholdSystem::new();
    let graph = RegionGraph::new();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);

    // No external connection means inflow_desire = 0.0 and removal_pressure = 0.0
    // (no households → unhoused_ratio = 0), so ResidentialGrowth is at equilibrium
    // (= 0.5) — no spawn pressure, no despawn pressure. Growth is blocked because
    // 0.5 is below the spawn threshold.
    assert!(
        demand.residential <= 0.50,
        "residential={}",
        demand.residential
    );
    assert_eq!(demand.commercial, 0.0);
    assert_eq!(demand.industrial, 0.0);
    assert_eq!(demand.households_to_admit_today, 0);
}

#[test]
fn max_demand_cheat_survives_daily_and_hourly_recompute() {
    let allocator = BuildingAllocator::new();
    let households = HouseholdSystem::new();
    let graph = RegionGraph::new();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.enable_max_demand_cheat();
    assert_eq!(demand.residential, 1.0);
    assert_eq!(demand.commercial, 1.0);
    assert_eq!(demand.industrial, 1.0);
    assert_eq!(demand.net_residential_pressure(), 1.0);
    assert_eq!(demand.net_commercial_pressure(), 1.0);
    assert_eq!(demand.net_industrial_pressure(), 1.0);

    demand.run_daily_pass(&allocator, &households, &graph, &zoning, -100.0);
    assert_eq!(demand.residential, 1.0);
    assert_eq!(demand.commercial, 1.0);
    assert_eq!(demand.industrial, 1.0);

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, -100.0);
    assert_eq!(demand.residential, 1.0);
    assert_eq!(demand.commercial, 1.0);
    assert_eq!(demand.industrial, 1.0);
    assert_eq!(demand.net_residential_pressure(), 1.0);
    assert_eq!(demand.net_commercial_pressure(), 1.0);
    assert_eq!(demand.net_industrial_pressure(), 1.0);
}

#[test]
fn max_demand_cheat_plans_commercial_spawn_without_consumer_need() {
    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = commercial_zoning_run(&graph);
    let mut demand = DemandSystem::new();

    demand.enable_max_demand_cheat();
    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000_000.0);

    assert_eq!(demand.building_actions.commercial.spawns.len(), 1);
    assert_eq!(
        demand
            .last_building_action_diagnostics
            .commercial
            .spawn_rejected_absorption,
        0
    );
}

#[test]
fn max_demand_cheat_plans_industrial_spawn_without_input_need() {
    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = industrial_zoning_run(&graph);
    let mut demand = DemandSystem::new();

    demand.enable_max_demand_cheat();
    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000_000.0);

    assert_eq!(demand.building_actions.industrial.spawns.len(), 1);
    assert_eq!(
        demand
            .last_building_action_diagnostics
            .industrial
            .spawn_rejected_absorption,
        0
    );
}

#[test]
fn residential_construction_bootstraps_from_construction_move_in_viability() {
    let mut allocator = BuildingAllocator::new();
    register_test_asset(&mut allocator, "residential", ZoneType::Residential);

    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = residential_zoning_run(&graph);
    let mut demand = DemandSystem::new();
    demand.spawn_action_credit.residential = 1.0;

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 100_000.0);

    assert!(
        demand.residential > 0.55,
        "healthy empty city should still want first residential capacity, got={:.3}",
        demand.residential
    );
    assert!(
        !demand.building_actions.residential.spawns.is_empty(),
        "healthy construction-side move-in viability should allow residential spawns"
    );
    assert!(
        demand.last_admission_diagnostics.move_in_acceptance > 0.25,
        "age-aware incoming household pull should remain visible before a vacant home exists, got={:.3}",
        demand.last_admission_diagnostics.move_in_acceptance
    );
    assert_eq!(
        demand.last_admission_diagnostics.max_actionable_households,
        0
    );
    assert_eq!(demand.last_admission_diagnostics.planned_households, 0);
    assert!(
        demand
            .last_admission_diagnostics
            .construction_move_in_acceptance
            > 0.25,
        "construction-side move-in viability should use the same age-aware pull, got={:.3}",
        demand
            .last_admission_diagnostics
            .construction_move_in_acceptance
    );
}

#[test]
fn residential_construction_stops_when_move_in_viability_is_zero() {
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
    demand.spawn_action_credit.residential = 10.0;

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, -100.0);

    assert!(
        demand.residential <= 0.50,
        "failed move-in viability should hold ResidentialGrowth at equilibrium or below, got={:.3}",
        demand.residential
    );
    assert_eq!(demand.building_actions.residential.spawns.len(), 0);
    assert_eq!(
        demand
            .last_admission_diagnostics
            .construction_move_in_acceptance,
        0.0
    );
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

    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000.0);

    assert!(demand.households_to_admit_today > 0);
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
    households
        .households
        .push(housed_household(0, 20, 10_000.0, 3.0));

    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);

    assert!(snapshot.open_job_slots > 0);
    assert!(snapshot.existing_unemployed_member_count >= snapshot.open_job_slots);
    assert_eq!(snapshot.open_job_household_pull, 0.0);
    assert_eq!(snapshot.move_in_job_slots, 0);
    assert!(snapshot.move_in_job_equivalent_slots <= 0.001);
    assert!(
        snapshot.incoming_household_need <= snapshot.regional_growth_household_pull + 0.001,
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

    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, -20_000.0);

    let diagnostics = demand.last_admission_diagnostics;
    assert!(diagnostics.open_job_slots > 0);
    assert_eq!(diagnostics.existing_unemployed_member_count, 0);
    assert!(
        diagnostics.candidate_adult_count > 0,
        "job-driven admission should evaluate a worker-capable household"
    );
    assert!(
        diagnostics.move_in_acceptance > 0.0,
        "negative treasury should not block a worker household whose job income covers essentials"
    );
    assert!(demand.prefer_worker_capable_admission());
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
    households.households.push(housed_household(
        0,
        farm_profile.worker_capacity as u16,
        10_000.0,
        3.0,
    ));

    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");
    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 100_000.0);

    assert_eq!(
        snapshot.physical_worker_capacity,
        farm_profile.worker_capacity
    );
    assert_eq!(
        snapshot.funded_worker_capacity,
        farm_profile.worker_capacity
    );
    assert_eq!(snapshot.open_job_slots, 0);
    assert_eq!(snapshot.existing_unemployed_member_count, 0);
}

#[test]
fn regional_growth_admits_households_without_open_jobs_after_bootstrap() {
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
        demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 100_000.0);
        if demand.households_to_admit_today > 0 {
            break;
        }
    }

    assert_eq!(
        demand.last_admission_diagnostics.open_job_household_pull,
        0.0
    );
    assert!(
        demand
            .last_admission_diagnostics
            .regional_growth_household_pull
            > 0.0,
        "healthy connected city should have durable regional growth pull"
    );
    assert!(
        demand.households_to_admit_today > 0,
        "regional growth should eventually spend admission credit when homes are vacant: pull={:.3} incoming={:.3} pressure={:.3} accept={:.3} credit={:.3} vacant={}",
        demand
            .last_admission_diagnostics
            .regional_growth_household_pull,
        demand.last_admission_diagnostics.incoming_household_need,
        demand.last_admission_diagnostics.pressure,
        demand.last_admission_diagnostics.move_in_acceptance,
        demand.last_admission_diagnostics.credit_after,
        demand.last_admission_diagnostics.max_actionable_households,
    );
}

#[test]
fn regional_growth_requires_external_connection() {
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
        demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 100_000.0);
    }

    assert_eq!(
        demand
            .last_admission_diagnostics
            .regional_growth_household_pull,
        0.0
    );
    assert_eq!(demand.households_to_admit_today, 0);
}

#[test]
fn regional_growth_damps_on_failure_and_soft_target() {
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

    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");

    let mut healthy_households = HouseholdSystem::new();
    healthy_households
        .households
        .push(housed_household(0, 2, 1_000.0, 3.0));
    let healthy = DailyDemandSnapshot::from_runtime(
        &allocator,
        &healthy_households,
        &graph,
        config.as_ref(),
        100_000.0,
    );

    let mut failing_households = HouseholdSystem::new();
    failing_households
        .households
        .push(housed_household(0, 2, 1_000.0, 3.0));
    failing_households
        .households
        .push(unhoused_household(2, 0.0, 0.0));
    let failing = DailyDemandSnapshot::from_runtime(
        &allocator,
        &failing_households,
        &graph,
        config.as_ref(),
        100_000.0,
    );

    let mut soft_target_households = HouseholdSystem::new();
    for _ in 0..700 {
        soft_target_households
            .households
            .push(housed_household(0, 2, 1_000.0, 3.0));
    }
    let soft_target = DailyDemandSnapshot::from_runtime(
        &allocator,
        &soft_target_households,
        &graph,
        config.as_ref(),
        100_000.0,
    );

    assert!(
        healthy.regional_growth_household_pull > 0.0,
        "healthy connected city should have a regional migration signal"
    );
    assert!(
        failing.regional_growth_household_pull < healthy.regional_growth_household_pull,
        "unhoused/zero-budget households should damp regional growth"
    );
    assert_eq!(soft_target.regional_growth_household_pull, 0.0);
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

    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 100_000.0);

    assert_eq!(
        demand.last_admission_diagnostics.candidate_household_size,
        1.0
    );
    assert!(
        demand.last_admission_diagnostics.pressure > 0.0,
        "large homes should not force an oversized household into the city without job pull"
    );
}

#[test]
fn hourly_admission_soft_damps_when_household_economy_is_failing() {
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

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, -100.0);

    assert_eq!(
        demand.households_to_admit_today, 0,
        "vacancy alone must not keep admitting households while affordability is zero, many households are unhoused, and the treasury is negative"
    );
}

#[test]
fn household_admission_diagnostics_record_pressure_breakdown() {
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

    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000.0);
    let diagnostics = demand.last_admission_diagnostics;

    assert_eq!(diagnostics.total_household_count, 0);
    assert_eq!(
        diagnostics.vacant_household_slots,
        allocator.household_capacity(0)
    );
    assert_eq!(diagnostics.connected_border_count, 1);
    assert!(diagnostics.base_pressure > 0.0);
    assert_eq!(
        diagnostics.planned_households,
        demand.households_to_admit_today
    );

    demand.record_household_admission_execution(1);

    assert_eq!(demand.last_admission_diagnostics.launched_households, 1);
}

#[test]
fn household_removal_diagnostics_record_failure_signal_counts() {
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

    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_daily_pass(&allocator, &households, &graph, &zoning, -100.0);
    let diagnostics = demand.last_removal_diagnostics;

    assert_eq!(diagnostics.total_household_count, 3);
    assert_eq!(diagnostics.housed_household_count, 1);
    assert_eq!(diagnostics.unhoused_household_count, 2);
    assert_eq!(diagnostics.zero_budget_household_count, 2);
    assert!((diagnostics.pressure - (2.0 / 3.0)).abs() < 1e-4);
    assert!((diagnostics.failure_pressure - (2.0 / 3.0)).abs() < 1e-4);
    assert_eq!(diagnostics.recent_failure_before, 0.0);
    assert!((diagnostics.recent_failure_after - (2.0 / 3.0)).abs() < 1e-4);
    assert!((demand.recent_household_failure_pressure - (2.0 / 3.0)).abs() < 1e-4);
    assert_eq!(
        diagnostics.planned_households,
        demand.households_to_remove_today
    );

    demand.record_household_removal_execution(2);

    assert_eq!(demand.last_removal_diagnostics.removed_households, 2);
}

#[test]
fn persistent_exit_removes_failed_unhoused_tail_below_crisis_threshold() {
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

    let mut households = HouseholdSystem::new();
    for _ in 0..7 {
        households
            .households
            .push(housed_household(0, 1, 200.0, 3.0));
    }
    for _ in 0..8 {
        let mut household = unhoused_household(1, 0.0, 0.0);
        household.unhoused_days_elapsed = 2;
        households.households.push(household);
    }

    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_daily_pass(&allocator, &households, &graph, &zoning, -100.0);
    let diagnostics = demand.last_removal_diagnostics;

    assert_eq!(diagnostics.unhoused_household_count, 8);
    assert_eq!(diagnostics.total_household_count, 15);
    assert!(diagnostics.pressure < diagnostics.threshold);
    assert_eq!(diagnostics.normalized_action_pressure, 0.0);
    assert_eq!(diagnostics.persistent_exit_eligible_household_count, 8);
    assert_eq!(diagnostics.persistent_exit_planned_households, 2);
    assert_eq!(demand.households_to_remove_today, 2);
}

#[test]
fn recent_failure_memory_damps_household_admission_pressure() {
    let mut healthy_demand = DemandSystem::new();
    let healthy = healthy_demand
        .update_pressure_channels_from_snapshot(&vacant_admission_snapshot())
        .admission_pressure;
    let mut cooling_demand = DemandSystem::new();
    cooling_demand.recent_household_failure_pressure = 0.8;
    let cooling_inputs =
        cooling_demand.update_pressure_channels_from_snapshot(&vacant_admission_snapshot());

    assert!(
        cooling_inputs.admission_pressure < healthy * 0.40,
        "recent failure memory should substantially reduce otherwise healthy incoming admission"
    );
    assert_eq!(
        cooling_inputs.admission_diagnostics.recent_failure_pressure,
        0.8
    );
    assert!(cooling_inputs.admission_diagnostics.recent_failure_factor < 0.35);
}

#[test]
fn admission_pressure_counts_zero_budget_households() {
    fn snapshot_with_zero_budget_ratio(zero_budget_household_ratio: f32) -> DailyDemandSnapshot {
        let mut snapshot = vacant_admission_snapshot();
        snapshot.total_household_count = 10;
        snapshot.housed_household_count = 10;
        snapshot.zero_budget_household_count = (zero_budget_household_ratio * 10.0).round() as u32;
        snapshot.zero_budget_household_ratio = zero_budget_household_ratio;
        snapshot
    }

    let mut healthy_demand = DemandSystem::new();
    let healthy_pressure = healthy_demand
        .update_pressure_channels_from_snapshot(&snapshot_with_zero_budget_ratio(0.0))
        .admission_pressure;
    let mut failing_demand = DemandSystem::new();
    let failing_pressure = failing_demand
        .update_pressure_channels_from_snapshot(&snapshot_with_zero_budget_ratio(0.8))
        .admission_pressure;

    assert!(
        failing_pressure < healthy_pressure,
        "zero-budget households must soft-damp admission pressure even when surviving housed households look affordable"
    );
}

#[test]
fn move_in_acceptance_accounts_for_transfer_treasury_coverage() {
    let mut covered_snapshot = vacant_admission_snapshot();
    covered_snapshot.existing_unemployed_member_count = 100;
    covered_snapshot.city_treasury_balance = 100_000.0;
    covered_snapshot.candidate_daily_essential_cost = 48.0;

    let mut depleted_snapshot = vacant_admission_snapshot();
    depleted_snapshot.existing_unemployed_member_count = 100;
    depleted_snapshot.city_treasury_balance = 0.0;
    depleted_snapshot.candidate_daily_essential_cost = 48.0;

    let mut covered_demand = DemandSystem::new();
    let covered_inputs = covered_demand.update_pressure_channels_from_snapshot(&covered_snapshot);
    let mut depleted_demand = DemandSystem::new();
    let depleted_inputs =
        depleted_demand.update_pressure_channels_from_snapshot(&depleted_snapshot);

    assert!(
        covered_inputs.admission_pressure > 0.9,
        "covered transfer runway should admit into available housing"
    );
    assert_eq!(
        depleted_inputs.admission_diagnostics.transfer_reliability,
        0.0
    );
    assert_eq!(
        depleted_inputs.admission_diagnostics.move_in_acceptance,
        0.0
    );
    assert_eq!(depleted_inputs.admission_pressure, 0.0);
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

    let mut demand = DemandSystem::new();
    let inputs = demand.update_pressure_channels_from_snapshot(&snapshot);
    let expected_existing_transfer_claim =
        snapshot.pension_daily_benefit_per_elder + snapshot.child_support_daily_benefit_per_child;
    assert!(
        (inputs.admission_diagnostics.existing_transfer_claim_per_day
            - expected_existing_transfer_claim)
            .abs()
            < 0.001
    );
}

#[test]
fn single_elder_candidate_uses_pension_not_unemployment_for_move_in_viability() {
    let mut pension_snapshot = vacant_admission_snapshot();
    pension_snapshot.candidate_household_size = 1.0;
    pension_snapshot.candidate_child_count = 0;
    pension_snapshot.candidate_adult_count = 0;
    pension_snapshot.candidate_elder_count = 1;
    pension_snapshot.immigrant_starter_savings_per_household = 15.0;
    pension_snapshot.candidate_daily_essential_cost = 28.0;
    pension_snapshot.unemployment_daily_benefit_per_adult = 30.0;
    pension_snapshot.pension_daily_benefit_per_elder = 30.0;
    pension_snapshot.child_support_daily_benefit_per_child = 0.0;
    pension_snapshot.city_treasury_balance = 100_000.0;

    let mut pension_demand = DemandSystem::new();
    let pension_inputs = pension_demand.update_pressure_channels_from_snapshot(&pension_snapshot);

    let mut no_pension_snapshot = pension_snapshot;
    no_pension_snapshot.pension_daily_benefit_per_elder = 0.0;
    let mut no_pension_demand = DemandSystem::new();
    let no_pension_inputs =
        no_pension_demand.update_pressure_channels_from_snapshot(&no_pension_snapshot);

    assert_eq!(
        pension_inputs
            .admission_diagnostics
            .candidate_unemployment_claim_per_day,
        0.0
    );
    assert_eq!(
        pension_inputs
            .admission_diagnostics
            .candidate_pension_claim_per_day,
        30.0
    );
    assert!(
        pension_inputs.admission_pressure > 0.9,
        "single elder should be admitted when pension covers daily essentials"
    );
    assert_eq!(
        no_pension_inputs.admission_diagnostics.move_in_acceptance, 0.0,
        "single elder must not be accepted via unemployment benefit when no adult exists"
    );
}

#[test]
fn open_jobs_make_move_in_viable_without_benefits() {
    let mut snapshot = vacant_admission_snapshot();
    snapshot.city_treasury_balance = 0.0;
    snapshot.open_job_slots = 2;
    snapshot.move_in_job_slots = 2;
    snapshot.move_in_job_equivalent_slots = 2.0;
    snapshot.average_move_in_job_wage_per_day = 100.0;

    let mut demand = DemandSystem::new();
    let inputs = demand.update_pressure_channels_from_snapshot(&snapshot);

    assert!(
        (inputs.admission_diagnostics.expected_employed_members - 2.0).abs() < 0.001,
        "expected employed adult workers should use exact candidate adult count"
    );
    assert_eq!(inputs.admission_diagnostics.daily_deficit, 0.0);
    assert!(
        inputs.admission_pressure > 0.9,
        "budget-backed open jobs should make the candidate household viable without benefit treasury"
    );
}

#[test]
fn fractional_marginal_commercial_job_improves_child_household_move_in_viability() {
    let mut blocked = vacant_admission_snapshot();
    blocked.open_job_household_pull = 0.0;
    blocked.candidate_household_size = 2.0;
    blocked.candidate_child_count = 1;
    blocked.candidate_adult_count = 1;
    blocked.candidate_elder_count = 0;
    blocked.immigrant_starter_savings_per_household = 30.0;
    blocked.candidate_daily_essential_cost = 56.0;
    blocked.unemployment_daily_benefit_per_adult = 30.0;
    blocked.pension_daily_benefit_per_elder = 30.0;
    blocked.child_support_daily_benefit_per_child = 10.0;
    blocked.city_treasury_balance = 100_000.0;

    let mut blocked_demand = DemandSystem::new();
    let blocked_inputs = blocked_demand.update_pressure_channels_from_snapshot(&blocked);

    assert_eq!(blocked_inputs.admission_diagnostics.move_in_job_slots, 0);
    assert_eq!(
        blocked_inputs
            .admission_diagnostics
            .move_in_job_equivalent_slots,
        0.0
    );
    assert_eq!(
        blocked_inputs
            .admission_diagnostics
            .expected_employed_members,
        0.0
    );
    assert_eq!(
        blocked_inputs
            .admission_diagnostics
            .candidate_transfer_claim_per_day,
        40.0
    );
    assert_eq!(blocked_inputs.admission_diagnostics.move_in_acceptance, 0.0);

    let mut fractional = blocked;
    fractional.marginal_commercial_job_household_pull = 0.25;
    fractional.marginal_commercial_job_equivalent_slots = 0.25;
    fractional.move_in_job_equivalent_slots = 0.25;
    fractional.average_move_in_job_wage_per_day = 100.0;
    let mut fractional_demand = DemandSystem::new();
    let fractional_inputs = fractional_demand.update_pressure_channels_from_snapshot(&fractional);
    let diagnostics = fractional_inputs.admission_diagnostics;

    assert_eq!(diagnostics.move_in_job_slots, 0);
    assert!((diagnostics.move_in_job_equivalent_slots - 0.25).abs() < 0.001);
    assert!((diagnostics.expected_employed_members - 0.25).abs() < 0.001);
    assert!((diagnostics.expected_unemployed_members - 0.75).abs() < 0.001);
    assert!((diagnostics.candidate_unemployment_claim_per_day - 22.5).abs() < 0.001);
    assert!((diagnostics.expected_wage_income_per_day - 25.0).abs() < 0.001);
    assert_eq!(diagnostics.daily_deficit, 0.0);
    assert!(
        fractional_inputs.admission_pressure > 0.0,
        "fractional marginal commercial income should unblock the one-adult one-child candidate"
    );
}

#[test]
fn marginal_commercial_growth_is_fractional_below_next_worker_slot() {
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
    let target_active_workers = 4;
    let resident_count = 50u16;
    let current_worker_equivalent = grocery.worker_capacity as f32
        * resident_count as f32
        * household_profile.consumption_rate_per_resident
        / household_supply_output;
    let marginal_worker_equivalent = grocery.worker_capacity as f32
        * (resident_count + candidate_household_size) as f32
        * household_profile.consumption_rate_per_resident
        / household_supply_output;
    assert_eq!(
        current_worker_equivalent.ceil() as u32,
        target_active_workers
    );
    assert_eq!(
        marginal_worker_equivalent.ceil() as u32,
        target_active_workers,
        "candidate demand must stay below the next integer staffing slot for this regression"
    );

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "fractional_marginal_home",
        ZoneType::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "fractional_marginal_shop",
        ZoneType::Commercial,
    );
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
        &[1.0, 1.0],
    );

    assert_eq!(snapshot.open_job_slots, 0);
    assert_eq!(snapshot.marginal_commercial_job_slots, 0);
    assert_eq!(snapshot.move_in_job_slots, 0);
    assert!(snapshot.marginal_commercial_job_equivalent_slots > 0.0);
    assert!(snapshot.move_in_job_equivalent_slots > 0.0);
    assert!(snapshot.average_move_in_job_wage_per_day > 0.0);
    assert!(snapshot.marginal_commercial_job_household_pull > 0.0);
    assert!(snapshot.incoming_household_need > snapshot.regional_growth_household_pull);

    let mut demand = DemandSystem::new();
    let inputs = demand.update_pressure_channels_from_snapshot(&snapshot);
    assert_eq!(
        inputs.admission_diagnostics.marginal_commercial_job_slots,
        0
    );
    assert!(
        inputs
            .admission_diagnostics
            .marginal_commercial_job_equivalent_slots
            > 0.0
    );
    assert_eq!(inputs.admission_diagnostics.move_in_job_slots, 0);
    assert!(
        inputs.admission_diagnostics.move_in_job_equivalent_slots > 0.0,
        "fractional marginal commercial capacity should feed move-in viability"
    );
    assert!(inputs.admission_diagnostics.expected_employed_members > 0.0);
    assert!(inputs.admission_diagnostics.expected_wage_income_per_day > 0.0);
}

#[test]
fn marginal_commercial_jobs_pull_households_when_current_store_cap_is_filled() {
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
        &[1.0, 1.0],
    );

    assert_eq!(snapshot.open_job_slots, 0);
    assert_eq!(snapshot.marginal_commercial_job_slots, 1);
    assert!(snapshot.marginal_commercial_job_equivalent_slots >= 1.0);
    assert_eq!(snapshot.move_in_job_slots, 1);
    assert!(snapshot.marginal_commercial_job_household_pull > 0.0);
    assert!(snapshot.incoming_household_need > snapshot.regional_growth_household_pull);

    let mut demand = DemandSystem::new();
    let inputs = demand.update_pressure_channels_from_snapshot(&snapshot);
    assert_eq!(inputs.admission_diagnostics.open_job_slots, 0);
    assert_eq!(
        inputs.admission_diagnostics.marginal_commercial_job_slots,
        1
    );
    assert!(
        inputs
            .admission_diagnostics
            .marginal_commercial_job_equivalent_slots
            >= 1.0
    );
    assert_eq!(inputs.admission_diagnostics.move_in_job_slots, 1);
    assert!(inputs.admission_diagnostics.expected_employed_members > 0.9);
    assert!(
        inputs.admission_pressure > 0.0,
        "forecast-only marginal commercial jobs should make the next household admissible"
    );
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

    let households = HouseholdSystem::new();
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
        &[1.0],
    );
    assert_eq!(
        fully_funded.open_job_slots,
        power_profile.worker_capacity - 2
    );
    assert_eq!(fully_funded.open_jobs_unfunded, 0);

    let defunded = DailyDemandSnapshot::from_runtime_with_catalog(
        &allocator,
        &households,
        &graph,
        &config,
        catalog.as_ref(),
        tuning.as_ref(),
        &fiscal_policy,
        100_000.0,
        &[0.1],
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
    assert_eq!(defunded.open_job_household_pull, 0.0);
}

#[test]
fn load_pressure_refresh_does_not_advance_action_state() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "load_refresh_home", ZoneType::Residential);
    let commercial_asset =
        register_test_asset(&mut allocator, "load_refresh_jobs", ZoneType::Commercial);
    allocator.buildings.push(building(
        ZoneType::Residential,
        0.0,
        0,
        0,
        residential_asset,
    ));
    allocator.buildings.push(building(
        ZoneType::Commercial,
        100.0,
        0,
        0,
        commercial_asset,
    ));

    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let mut demand = DemandSystem::new();
    demand.residential = 0.5;
    demand.admission_action_credit = 2.25;
    demand.households_to_admit_today = 3;
    demand.spawn_action_credit.residential = 1.75;

    demand.refresh_pressure_channels_with_service_funding(
        &allocator,
        &households,
        &graph,
        100_000.0,
        &[1.0, 1.0],
        &CityFiscalPolicy::default(),
    );

    assert!(
        demand.net_residential_pressure() > 0.0,
        "load refresh should rebuild visible demand pressure from authoritative runtime state"
    );
    assert_eq!(demand.admission_action_credit, 2.25);
    assert_eq!(demand.households_to_admit_today, 3);
    assert_eq!(demand.spawn_action_credit.residential, 1.75);
    assert!(demand.building_actions.residential.spawns.is_empty());
    assert!(demand.building_actions.commercial.spawns.is_empty());
    assert!(demand.building_actions.industrial.spawns.is_empty());
}

#[test]
fn snapshot_computes_owa_dependency_from_input_accumulators() {
    // Commercial building (grocery_basic profile) with 75 currency from OWA and 25 from local.
    // With no residents, the shop carries only its one-worker bootstrap input need:
    // expected = 160 packaged_food * 15.0/unit / 15 workers = 160.
    // denom = max(actual=100, expected=160) = 160.
    // owa_dependency = 75 / 160 = 0.46875.
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
    let mut com = building(ZoneType::Commercial, 40.0, 0, 1, commercial_asset);
    com.daily_owa_input_value = 75.0;
    com.daily_local_input_value = 25.0;
    allocator.buildings.push(com);

    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let config = load_builtin_demand_config().expect("built-in demand config must load");

    let snapshot =
        DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);

    assert!(
        (snapshot.commercial_owa_dependency - 0.46875).abs() < 1e-4,
        "owa_dependency must equal owa/max(actual,expected): got={:.6}",
        snapshot.commercial_owa_dependency
    );
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
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Residential,
        0.95,
    );
    assert!(low_affordability.upgrades.is_empty());

    households.households[0].budget = 1_200.0;
    let residential_occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);
    let high_affordability = demand.collect_existing_building_candidates(
        &allocator,
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Residential,
        0.95,
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
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
        0.95,
    );
    assert!(weak_viability.upgrades.is_empty());

    allocator.buildings[0].worker_count = 15;
    allocator.buildings[0].operating_budget = 6_000.0;
    let strong_viability = demand.collect_existing_building_candidates(
        &allocator,
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
        0.95,
    );
    assert_eq!(strong_viability.upgrades.len(), 1);
}

#[test]
fn building_action_hysteresis_keeps_existing_action_inside_margin() {
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
    let below_raw_threshold = demand.collect_existing_building_candidates(
        &allocator,
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
        0.16,
    );
    assert!(below_raw_threshold.upgrades.is_empty());

    let mut demand = DemandSystem::new();
    demand.upgrade_hysteresis_active.commercial = true;
    let inside_hysteresis_margin = demand.collect_existing_building_candidates(
        &allocator,
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
        0.16,
    );
    assert_eq!(inside_hysteresis_margin.upgrades.len(), 1);

    let outside_hysteresis_margin = demand.collect_existing_building_candidates(
        &allocator,
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
        0.12,
    );
    assert!(outside_hysteresis_margin.upgrades.is_empty());
}

#[test]
fn net_pressure_display_uses_active_hysteresis_margin() {
    let mut demand = DemandSystem::new();
    demand.commercial = 0.05;
    assert_eq!(demand.net_commercial_pressure(), 0.0);

    demand.spawn_hysteresis_active.commercial = true;
    assert!(demand.net_commercial_pressure() > 0.0);

    demand.spawn_hysteresis_active.commercial = false;
    demand.commercial = 0.03;
    assert_eq!(demand.net_commercial_pressure(), 0.0);

    demand.despawn_hysteresis_active.commercial = true;
    assert!(demand.net_commercial_pressure() < 0.0);
}

#[test]
fn deserted_buildings_are_despawn_first_and_never_downgrade() {
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
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
        0.0,
    );
    assert_eq!(candidates.despawns.len(), 2);
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
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
        0.0,
    );
    assert!(candidates.despawns.is_empty());
    assert!(candidates.downgrades.is_empty());
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

    let mut high_parcel_early_edge = building(ZoneType::Commercial, 0.0, 0, 0, asset_id);
    high_parcel_early_edge.parcel_id = 99;
    high_parcel_early_edge.edge_idx = 2;
    high_parcel_early_edge.cell_x = 0;

    allocator.buildings.push(low_parcel_late_edge);
    allocator.buildings.push(high_parcel_early_edge);

    let households = HouseholdSystem::new();
    let demand = DemandSystem::new();
    let economy_tuning = load_runtime_economy_tuning().expect("runtime economy tuning must load");
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog must load");
    let residential_occupants = ResidentialOccupantSnapshot::from_runtime(&allocator, &households);
    let candidates = demand.collect_existing_building_candidates(
        &allocator,
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Commercial,
        0.0,
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

    let starter_headroom = demand.collect_existing_building_candidates(
        &allocator,
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Industrial,
        0.95,
    );
    assert_eq!(starter_headroom.upgrades.len(), 1);

    if let Some(output_port) = starter_factory.outputs.first() {
        allocator.buildings[0].set_inventory_units(output_port.resource_runtime_id, 50.0);
    }
    let same_profile = demand.collect_existing_building_candidates(
        &allocator,
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Industrial,
        0.95,
    );
    assert_eq!(same_profile.upgrades.len(), 1);

    if let Some(output_port) = starter_factory.outputs.first() {
        allocator.buildings[0].set_inventory_units(output_port.resource_runtime_id, 630.0);
    }
    let jammed_output = demand.collect_existing_building_candidates(
        &allocator,
        &households,
        catalog.as_ref(),
        economy_tuning.as_ref(),
        &residential_occupants,
        ZoneType::Industrial,
        0.95,
    );
    assert!(jammed_output.upgrades.is_empty());
}
