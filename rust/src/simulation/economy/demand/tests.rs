//! Demand tests.

use super::credits::advance_spawn_need_credit;
use super::spawn_need::{OutputAbsorptionContext, nonresidential_passes_absorption_gate};
use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, LodEntry, PlacementMode, ZoneClass};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::economy::households::{Household, HouseholdSystem, REPLENISHMENT_STABLE};
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
        lods: vec![LodEntry {
            file: "lod0.glb".to_owned(),
            distance_min_m: 0.0,
            distance_max_m: None,
        }],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [0.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
        }],
        building: Some(BuildingData {
            flat_size_m2: if matches!(zone_type, ZoneType::Residential | ZoneType::Mixed) {
                Some(80.0)
            } else {
                None
            },
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(zone_class),
            density: Some("low".to_owned()),
            lot_width_cells: 2,
            lot_depth_cells: 2,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level,
            household_capacity,
            worker_capacity,
            service_class: None,
            economy_profile: economy_profile.map(str::to_owned),
            preview_scale: Some(1.0),
        }),
        prop: None,
        vehicle: None,
        character: None,
        pivot_offset: None,
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
        asset_id,
        level: 1,
        broken: false,
        economy_profile_runtime_id: runtime_id,
        economy_broken: false,
        resource_inventory,
        revenue: 0.0,
        operating_budget: 500.0,
        shipment_cooldown_hours: 0,
        daily_owa_input_value: 0.0,
        daily_local_input_value: 0.0,
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
        consumption_rate: 1.0,
        stock_days,
        replenishment_state: REPLENISHMENT_STABLE,
        cooldown_hours: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        pickup_eta_hours: 0,
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

fn vacant_admission_snapshot() -> DailyDemandSnapshot {
    DailyDemandSnapshot {
        vacant_household_slots: 10,
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
        household_affordability: 1.0,
        household_stock_stability: 1.0,
        commercial_capacity_deficit: 0.0,
        unmet_commercial_consumer_demand: 0.0,
        industrial_input_capacity_deficit: 0.0,
        commercial_input_need_value: 0.0,
        local_industrial_input_capacity_value: 0.0,
        industrial_missing_input_value: 0.0,
        external_connection_available: 1.0,
        connected_border_count: 1,
        city_treasury_balance: 100_000.0,
        candidate_household_size: 2.0,
        immigrant_starter_savings_per_household: 30.0,
        candidate_daily_essential_cost: 56.0,
        unemployment_daily_benefit_per_member: 30.0,
        existing_unemployed_member_count: 0,
        open_job_slots: 0,
        average_open_job_wage_per_day: 0.0,
        output_absorption: OutputAbsorptionContext::empty(0),
        commercial_owa_dependency: 0.0,
        commercial_owa_input_value: 0.0,
    }
}

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
fn industrial_pressure_uses_capacity_balance_not_owa_accumulator() {
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
    assert_eq!(missing_snapshot.commercial_input_need_value, 2_400.0);
    assert_eq!(missing_snapshot.local_industrial_input_capacity_value, 0.0);
    assert_eq!(missing_snapshot.industrial_missing_input_value, 2_400.0);
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
    assert_eq!(covered_snapshot.commercial_input_need_value, 2_400.0);
    assert_eq!(
        covered_snapshot.local_industrial_input_capacity_value,
        2_400.0
    );
    assert_eq!(covered_snapshot.industrial_missing_input_value, 0.0);
    assert_eq!(covered_snapshot.industrial_input_capacity_deficit, 0.0);

    let mut covered_demand = DemandSystem::new();
    covered_demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);
    assert_eq!(covered_demand.industrial, 0.0);
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
    let required_workers = allocator.worker_capacity_for_asset(&commercial_asset);

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

    snapshot.unmet_commercial_consumer_demand = 1.0;
    assert_eq!(
        commercial_spawn_need_buildings(&allocator, &catalog, &snapshot, &candidates),
        1.0
    );

    snapshot.unmet_commercial_consumer_demand = 201.0;
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
    assert_eq!(
        industrial_spawn_need_buildings(&allocator, &catalog, &snapshot, &candidates),
        1.0
    );

    snapshot.industrial_missing_input_value = 2_401.0;
    assert_eq!(
        industrial_spawn_need_buildings(&allocator, &catalog, &snapshot, &candidates),
        2.0
    );

    snapshot.commercial_owa_input_value = 13_860.0;
    snapshot.industrial_missing_input_value = 0.0;
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
    let staple_food = catalog
        .resource_runtime_id_for_id("staple_food")
        .expect("staple_food resource");
    let absorption = OutputAbsorptionContext::from_resource_amounts(
        catalog.resource_count(),
        &[],
        &[],
        0,
        &[(staple_food, 160.0)],
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
        demand.last_admission_diagnostics.move_in_acceptance > 0.9,
        "incoming household pull should remain visible even before a vacant home exists"
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
            > 0.9
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

    let households = HouseholdSystem::new();
    let graph = graph_with_connected_border();
    let zoning = empty_zoning();
    let mut demand = DemandSystem::new();

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000.0);

    assert!(demand.households_to_admit_today > 0);
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
fn move_in_acceptance_accounts_for_benefit_treasury_coverage() {
    let mut covered_snapshot = vacant_admission_snapshot();
    covered_snapshot.existing_unemployed_member_count = 100;
    covered_snapshot.city_treasury_balance = 100_000.0;

    let mut depleted_snapshot = vacant_admission_snapshot();
    depleted_snapshot.existing_unemployed_member_count = 100;
    depleted_snapshot.city_treasury_balance = 0.0;

    let mut covered_demand = DemandSystem::new();
    let covered_inputs = covered_demand.update_pressure_channels_from_snapshot(&covered_snapshot);
    let mut depleted_demand = DemandSystem::new();
    let depleted_inputs =
        depleted_demand.update_pressure_channels_from_snapshot(&depleted_snapshot);

    assert!(
        covered_inputs.admission_pressure > 0.9,
        "covered benefit runway should admit into available housing"
    );
    assert_eq!(
        depleted_inputs.admission_diagnostics.benefit_reliability,
        0.0
    );
    assert_eq!(
        depleted_inputs.admission_diagnostics.move_in_acceptance,
        0.0
    );
    assert_eq!(depleted_inputs.admission_pressure, 0.0);
}

#[test]
fn open_jobs_make_move_in_viable_without_benefits() {
    let mut snapshot = vacant_admission_snapshot();
    snapshot.city_treasury_balance = 0.0;
    snapshot.open_job_slots = 2;
    snapshot.average_open_job_wage_per_day = 100.0;

    let mut demand = DemandSystem::new();
    let inputs = demand.update_pressure_channels_from_snapshot(&snapshot);

    assert_eq!(inputs.admission_diagnostics.expected_employed_members, 2.0);
    assert_eq!(inputs.admission_diagnostics.daily_deficit, 0.0);
    assert!(
        inputs.admission_pressure > 0.9,
        "budget-backed open jobs should make the candidate household viable without benefit treasury"
    );
}

#[test]
fn snapshot_computes_owa_dependency_from_input_accumulators() {
    // Commercial building (grocery_basic profile) with 75 currency from OWA and 25 from local.
    // Expected daily input = 160 staple_food * 15.0/unit = 2400.
    // denom = max(actual=100, expected=2400) = 2400.
    // owa_dependency = 75 / 2400 = 0.03125.
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
        (snapshot.commercial_owa_dependency - 0.03125).abs() < 1e-4,
        "owa_dependency must equal owa/max(actual,expected): got={:.6}",
        snapshot.commercial_owa_dependency
    );
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
    assert!(
        starter_factory.inputs.is_empty(),
        "shipped starter industrial profile is currently inputless"
    );

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

    if let Some(input_port) = starter_factory.inputs.first() {
        allocator.buildings[0].set_inventory_units(input_port.resource_runtime_id, 320.0);
    }
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
