use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{BuildingData, MeshPart, PlacementMode, ZoneClass};
use crate::config::DEFAULT_URBAN_ROAD_SPEED_MS;
use crate::nodes::sim::core::{
    CityServicePolicy, DailyBudgetLedgerEntry, PendingDemandSpawnAction,
};
use crate::simulation::agriculture::{AgricultureSystem, FieldSite};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::agents::{
    ACCESS_PLAN_VALID, AGE_ADULT, MODE_CAR, MODE_WALK, TRANSIT_NETWORK,
};
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::economy::demand::{DemandSpawnAction, DemandSystem};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::{
    Household, HouseholdSystem, REPLENISHMENT_SHOPPING_TO_STORE,
};
use crate::simulation::economy::logistics::{
    CarrierClass, FreightRequestFailure, FreightRequestKey, Shipment, ShipmentEndpoint,
    ShipmentStatus, ShipmentSystem,
};
use crate::simulation::extraction::{ExtractorSite, ResourceExtractionSystem};
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::resources::{COAL_RESOURCE_ID, ResourceDepositSystem};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use godot::prelude::{Vector2, Vector3};
use std::collections::VecDeque;
use std::fs;

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "metrum_rise_{name}_{}_{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn register_test_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
) -> String {
    let (household_capacity, worker_capacity) = match zone {
        ZoneClass::Residential => (Some(6), None),
        ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
        ZoneClass::Mixed => (Some(4), Some(2)),
    };
    allocator.registry.register(
        pack_id,
        AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![],
            mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
            anchors: vec![],
            site_surfaces: vec![],
            building: Some(BuildingData {
                flat_size_m2: None,
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(zone),
                density: Some("low".to_owned()),
                lot_width_cells: 3,
                lot_depth_cells: 3,
                frontage_forward: None,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity,
                worker_capacity,
                service_class: None,
                economy_profile: None,
                extractor: None,
                field: None,
            }),
            prop: None,
            vehicle: None,
            character: None,
        },
        String::new(),
    );
    format!("{pack_id}:{asset_id}")
}

fn zone_at_world(zoning: &ZoningSystem, x: f32, z: f32) -> ZoneType {
    let _ = (x, z);
    zoning
        .parcels()
        .first()
        .map(|parcel| {
            zoning
                .profiles
                .zone_type_for_runtime_id(parcel.zone_profile_runtime_id())
        })
        .unwrap_or(ZoneType::None)
}

#[test]
fn sqlite_round_trip_preserves_authoritative_state() {
    let config = WorldConfig::new(100.0, 100.0, 10.0, 10.0);
    let mut time = TimeSystem::new();
    time.speed_multiplier = 2.0;
    time.time_elapsed = 1.25;
    time.day_index = 3;
    time.minute_of_day = 480;
    time.seconds_per_day = 4.0;
    let mut terrain = TerrainSystem::from_world_config(&config);
    terrain.set_height(0, 0, 1.0);
    terrain.set_height(1, 0, 1.0);
    terrain.set_height(0, 1, 1.0);
    terrain.set_height(1, 1, 1.0);
    let mut water = WaterSystem::from_world_config(&config);
    let mut baseline_depth = water.clone_baseline_depth_dense();
    baseline_depth[0] = 2.0;
    water
        .replace_baseline_depth_from_dense(&baseline_depth)
        .expect("baseline water depth dimensions should match");
    let mut resource_deposits = ResourceDepositSystem::from_world_config(&config);
    resource_deposits.set_coal_richness_at(2, 3, 450);
    resource_deposits.set_coal_richness_at(8, 7, 900);
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let edge_id = graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: DEFAULT_URBAN_ROAD_SPEED_MS,
        base_cost: 40.0,
        physical_length: 40.0,
        current_congestion: 0.1,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
    graph.add_lane_connection(n0, edge_id, 0, edge_id, 0);
    let mut zoning = ZoningSystem::new(&config);
    let residential_profile = zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .expect("residential runtime id");
    zoning
        .restore_parcel_from_attachment(1, edge_id, 1, 0.5, 39.0, 40.0, residential_profile, &graph)
        .expect("save test parcel");
    let mut pollution = PollutionSystem::new(&config);
    pollution.grid.data[0] = 3.0;
    let mut noise = NoiseSystem::new(&config);
    noise.grid.data[0] = 7.0;
    let mut demand = DemandSystem::new();
    demand.residential = 0.12;
    demand.commercial = 0.08;
    demand.industrial = 0.04;
    demand.households_to_admit_today = 2;
    demand.admission_action_credit = 1.25;
    demand.removal_action_credit = 0.50;
    demand.persistent_exit_action_credit = 0.75;
    demand.recent_household_failure_pressure = 0.75;
    demand.enable_max_demand_cheat();
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "save_residential",
        ZoneClass::Residential,
    );
    allocator.buildings.push(Building {
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 3,
        depth_cells: 3,
        zone_profile_runtime_id: residential_profile,
        parcel_id: 1,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        budget_distress: false,
        is_deserted: false,
        edge_idx: edge_id,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 2,
        worker_count: 0,
        service_funding_override: 0.65,
        asset_id: residential_asset,
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: {
            let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
            let packaged_food = catalog
                .resource_runtime_id_for_id("packaged_food")
                .expect("packaged food resource");
            let mut inventory = vec![0.0; catalog.resource_count()];
            inventory[packaged_food as usize - 1] = 42.0;
            inventory
        },
        revenue: 0.0,
        operating_budget: 500.0,
        profit_tax_budget_baseline: 375.0,
        last_day_profit: 125.0,
        shipment_cooldown_hours: 0,
        daily_owa_input_value: 0.0,
        daily_local_input_value: 0.0,
        daily_city_funded_input_cost: 0.0,
        daily_household_sales_value: 123.0,
        daily_power_service_units: 78.0,
        daily_power_served_units: 77.0,
        recent_power_service_units: 79.0,
        recent_power_served_units: 76.0,
        recent_household_sales_value: 456.0,
        commercial_activity_floor_scale: 0.0,
        work_area_scale: 1.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    allocator
        .recompute_derived_transforms(&graph, &zoning)
        .expect("transforms");
    world::repaint_building_occupancy(&mut zoning, &allocator).expect("occupancy");
    allocator.rebuild_zone_index();
    let resource_extraction = ResourceExtractionSystem::from_sites(vec![ExtractorSite {
        building_idx: 0,
        resource_id: COAL_RESOURCE_ID.to_owned(),
        polygon_world: vec![
            Vector2::new(-5.0, -5.0),
            Vector2::new(5.0, -5.0),
            Vector2::new(5.0, 5.0),
            Vector2::new(-5.0, 5.0),
        ],
        area_m2: 100.0,
        total_reserve_units: 1234.0,
        extracted_units: 321.0,
    }]);
    let agriculture = AgricultureSystem::from_sites(vec![FieldSite {
        building_idx: 0,
        resource_id: "grain".to_owned(),
        polygon_world: vec![
            Vector2::new(-8.0, -4.0),
            Vector2::new(8.0, -4.0),
            Vector2::new(8.0, 4.0),
            Vector2::new(-8.0, 4.0),
        ],
        area_m2: 128.0,
    }]);
    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: 178.0,
        stock: 3.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 1.5,
        replenishment_state: REPLENISHMENT_SHOPPING_TO_STORE,
        cooldown_hours: 0,
        replenishment_failure_count: 1,
        reserved_store_building_id: 0,
        reserved_amount: 2.5,
        reserved_total_cost: 15.0,
        shopping_agent_id: 0,
        shopping_agent_schedule_seed: 1,
        shopping_timeout_hours_remaining: 4,
        replenishment_search_cursor: 24,
        stay_failure_days: 1,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    });
    let mut logistics = ShipmentSystem::new();
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    logistics.shipments.push(Shipment {
        id: 0,
        resource_runtime_id: household_supplies,
        amount: 80.0,
        source: ShipmentEndpoint::OwaBorder(0),
        destination: ShipmentEndpoint::Building(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier_agent_id: usize::MAX,
        total_cost: 640.0,
        eta_hours: 1,
        queued_hours: 0,
    });
    logistics.request_failures.insert(
        FreightRequestKey {
            destination_building_id: 0,
            resource_runtime_id: household_supplies,
        },
        FreightRequestFailure {
            failures: 2,
            terminal: true,
        },
    );
    logistics.set_owa_export_saturation_units(household_supplies, 120.0);
    let mut network_sys = TransitNetwork::new();
    network_sys.lane_system.rebuild(&mut graph);
    let planned_lane_id = network_sys.lane_system.edge_lanes[&edge_id][0] as u32;
    let mut agents_sys = AgentSystem::new();
    agents_sys.sim_time = 42.0;
    agents::push_loaded_agent(
        &mut agents_sys,
        agents::LoadedAgentRecord {
            home_building: 0,
            household_id: 0,
            age_group: AGE_ADULT,
            pending_household_size: 0,
            freight_shipment_id: u64::MAX,
            work_building: usize::MAX,
            current_building: usize::MAX,
            target_building: 0,
            freight_target_border_node: u32::MAX,
            current_node: n0,
            planned_attach_node: n0,
            planned_detach_node: n1,
            planned_attach_lane_id: planned_lane_id,
            planned_detach_lane_id: planned_lane_id,
            planned_attach_lane_d: 3.5,
            planned_detach_lane_d: 7.5,
            access_flags: ACCESS_PLAN_VALID,
            next_replan_time: 9.5,
            current_edge: edge_id,
            current_lane_id: 0,
            lane_distance: 0.0,
            pos_x: -5.0,
            pos_y: 0.0,
            activity: 1,
            transit: TRANSIT_NETWORK,
            transit_mode: MODE_CAR,
            happiness: 88.0,
            money: 123.0,
            journey_start_time: 12.5,
            schedule_seed: 1,
            cached_commute_minutes: 12,
            next_commute_refresh_time: 24.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            has_car: true,
            vehicle_type: 0,
            current_path_index: 1,
            current_path: vec![n0, n1],
            pedestrian_type: 0,
            walk_phase: 0.0,
        },
    );
    agents::push_loaded_agent(
        &mut agents_sys,
        agents::LoadedAgentRecord {
            home_building: 0,
            household_id: 0,
            age_group: AGE_ADULT,
            pending_household_size: 0,
            freight_shipment_id: u64::MAX,
            work_building: usize::MAX,
            current_building: usize::MAX,
            target_building: usize::MAX,
            freight_target_border_node: u32::MAX,
            current_node: n1,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 9.5,
            current_edge: edge_id,
            current_lane_id: -1,
            lane_distance: 0.0,
            pos_x: 5.0,
            pos_y: 0.0,
            activity: 0,
            transit: TRANSIT_NETWORK,
            transit_mode: MODE_WALK,
            happiness: 77.0,
            money: 55.0,
            journey_start_time: 6.0,
            schedule_seed: 2,
            cached_commute_minutes: 8,
            next_commute_refresh_time: 18.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            has_car: false,
            vehicle_type: 0,
            current_path_index: 1,
            current_path: Vec::new(),
            pedestrian_type: 0,
            walk_phase: 0.0,
        },
    );
    let mut treasury = CityTreasury::new(1_000.0);
    treasury.lifetime_tax_revenue = 250.0;
    treasury.last_daily_business_profit_tax = 12.5;
    treasury.last_daily_property_tax = 30.0;
    treasury.last_daily_residential_property_tax = 10.0;
    treasury.last_daily_commercial_property_tax = 12.0;
    treasury.last_daily_industrial_property_tax = 8.0;
    treasury.pending_business_profit_tax = 3.25;
    treasury.pending_property_tax = 15.0;
    treasury.pending_residential_property_tax = 4.0;
    treasury.pending_commercial_property_tax = 5.0;
    treasury.pending_industrial_property_tax = 6.0;
    let service_policy = CityServicePolicy {
        electricity_funding: 0.35,
    };
    let mut fiscal_policy = CityFiscalPolicy::default();
    fiscal_policy.pension_per_elder_per_day = 42.0;
    fiscal_policy.child_support_per_child_per_day = 11.0;
    fiscal_policy.income_tax_rate = 0.22;
    let mut budget_history = VecDeque::new();
    budget_history.push_back(DailyBudgetLedgerEntry {
        day_index: 3,
        income: 120.0,
        expenses: 80.0,
        net: 40.0,
        treasury: 1_040.0,
        tax_income: 80.0,
        income_tax: 30.0,
        household_vat: 20.0,
        business_profit_tax: 12.0,
        property_tax: 18.0,
        residential_property_tax: 7.0,
        commercial_property_tax: 6.0,
        industrial_property_tax: 5.0,
        utility_service_revenue: 25.0,
        benefits: 10.0,
        unemployment_benefits: 4.0,
        pensions: 5.0,
        child_support: 1.0,
        city_wages: 20.0,
        fuel_input_purchases: 15.0,
        imports_owa: 5.0,
        construction_service_costs: 30.0,
        power_produced: 70.0,
        power_consumed: 60.0,
        power_unmet: 2.0,
        power_coverage: 0.95,
        coal_inventory: 300.0,
        coal_bought: 40.0,
        coal_consumed: 35.0,
        electricity_fuel_cost: 12.0,
        electricity_wage_cost: 8.0,
        electricity_revenue: 30.0,
        electricity_net: 10.0,
    });
    let mut pending_demand_spawns = VecDeque::new();
    pending_demand_spawns.push_back(PendingDemandSpawnAction {
        due_minute: 1234,
        zone_type: ZoneType::Residential,
        action: DemandSpawnAction {
            parcel_id: 42,
            asset_id: "building.residential.save_test".to_owned(),
        },
        planned_day_index: 2,
        planned_minute_of_day: 180,
    });

    let path = temp_path("round_trip");
    save_to_sqlite(
        &path,
        SaveGameView {
            config: &config,
            time: &time,
            terrain: &terrain,
            water: &water,
            resource_deposits: &resource_deposits,
            graph: &graph,
            zoning: &zoning,
            pollution: &pollution,
            noise: &noise,
            demand: &demand,
            pending_demand_spawns: &pending_demand_spawns,
            allocator: &allocator,
            households: &households,
            logistics: &logistics,
            resource_extraction: &resource_extraction,
            agriculture: &agriculture,
            agents: &agents_sys,
            network: &network_sys,
            treasury: &treasury,
            service_policy: &service_policy,
            fiscal_policy: &fiscal_policy,
            budget_history: &budget_history,
        },
    )
    .expect("save");
    let loaded = load_from_sqlite(&path, &allocator.registry).expect("load");
    fs::remove_file(&path).ok();

    assert_eq!(loaded.config.width_m, config.width_m);
    assert_eq!(loaded.config.height_m, config.height_m);
    assert_eq!(loaded.config.terrain_cell_m, config.terrain_cell_m);
    assert_eq!(loaded.config.terrain_chunk_m, config.terrain_chunk_m);
    assert_eq!(
        loaded.config.terrain_base_elevation_m,
        config.terrain_base_elevation_m
    );
    assert_eq!(loaded.time.day_index, time.day_index);
    assert_eq!(loaded.time.minute_of_day, time.minute_of_day);
    assert_eq!(
        loaded.terrain.clone_source_dense(),
        terrain.clone_source_dense()
    );
    assert_eq!(
        loaded.water.clone_baseline_depth_dense(),
        water.clone_baseline_depth_dense()
    );
    assert_eq!(
        loaded.resource_deposits.clone_coal_richness_dense(),
        resource_deposits.clone_coal_richness_dense()
    );
    assert_eq!(loaded.demand.residential, demand.residential);
    assert_eq!(loaded.demand.commercial, demand.commercial);
    assert_eq!(loaded.demand.industrial, demand.industrial);
    assert_eq!(
        loaded.demand.households_to_admit_today,
        demand.households_to_admit_today
    );
    assert_eq!(
        loaded.demand.admission_action_credit,
        demand.admission_action_credit
    );
    assert_eq!(
        loaded.demand.removal_action_credit,
        demand.removal_action_credit
    );
    assert_eq!(
        loaded.demand.persistent_exit_action_credit,
        demand.persistent_exit_action_credit
    );
    assert_eq!(
        loaded.demand.recent_household_failure_pressure,
        demand.recent_household_failure_pressure
    );
    assert!(loaded.demand.cheat_max_demands_enabled);
    assert_eq!(loaded.pending_demand_spawns.len(), 1);
    let loaded_pending = &loaded.pending_demand_spawns[0];
    assert_eq!(loaded_pending.due_minute, 1234);
    assert_eq!(loaded_pending.zone_type, ZoneType::Residential);
    assert_eq!(loaded_pending.action.parcel_id, 42);
    assert_eq!(
        loaded_pending.action.asset_id,
        "building.residential.save_test"
    );
    assert_eq!(loaded_pending.planned_day_index, 2);
    assert_eq!(loaded_pending.planned_minute_of_day, 180);
    assert_eq!(loaded.pollution.grid.data, pollution.grid.data);
    assert_eq!(loaded.noise.grid.data, noise.grid.data);
    assert_eq!(loaded.graph.edge_count(), 1);
    assert_eq!(
        loaded.graph.edge(0).vehicle_frontage_access,
        VehicleFrontageAccess::BothSides
    );
    assert_eq!(
        zone_at_world(&loaded.zoning, 0.0, 0.0),
        ZoneType::Residential
    );
    assert_eq!(loaded.allocator.buildings.len(), 1);
    assert_eq!(loaded.resource_extraction.sites().len(), 1);
    let loaded_extractor = &loaded.resource_extraction.sites()[0];
    assert_eq!(loaded_extractor.building_idx, 0);
    assert_eq!(loaded_extractor.resource_id, COAL_RESOURCE_ID);
    assert_eq!(loaded_extractor.polygon_world.len(), 4);
    assert!((loaded_extractor.total_reserve_units - 1234.0).abs() < 0.001);
    assert!((loaded_extractor.extracted_units - 321.0).abs() < 0.001);
    assert_eq!(loaded.agriculture.sites().len(), 1);
    let loaded_field = &loaded.agriculture.sites()[0];
    assert_eq!(loaded_field.building_idx, 0);
    assert_eq!(loaded_field.resource_id, "grain");
    assert_eq!(loaded_field.polygon_world.len(), 4);
    assert!((loaded_field.area_m2 - 128.0).abs() < 0.001);
    assert_eq!(loaded.households.households.len(), 1);
    assert_eq!(
        loaded.households.households[0].reserved_store_building_id,
        0
    );
    assert_eq!(loaded.households.households[0].reserved_amount, 2.5);
    assert_eq!(loaded.households.households[0].reserved_total_cost, 15.0);
    assert_eq!(
        loaded.households.households[0].replenishment_failure_count,
        1
    );
    assert_eq!(loaded.households.households[0].shopping_agent_id, 0);
    assert_eq!(
        loaded.households.households[0].shopping_agent_schedule_seed,
        1
    );
    assert_eq!(
        loaded.households.households[0].shopping_timeout_hours_remaining,
        4
    );
    assert_eq!(
        loaded.households.households[0].replenishment_search_cursor,
        24
    );
    assert_eq!(loaded.households.households[0].stay_failure_days, 1);
    assert_eq!(loaded.households.households[0].unhoused_days_elapsed, 0);
    assert_eq!(loaded.households.households[0].child_count, 0);
    assert_eq!(loaded.households.households[0].adult_count, 2);
    assert_eq!(loaded.households.households[0].elder_count, 0);
    assert_eq!(loaded.agents.len(), 2);
    assert_eq!(loaded.agents.age_group[0], AGE_ADULT);
    assert_eq!(loaded.agents.age_group[1], AGE_ADULT);
    assert_eq!(loaded.agents.current_path[0], vec![0, 1]);
    assert_eq!(loaded.agents.transit[1], TRANSIT_NETWORK);
    assert_eq!(loaded.agents.current_lane_id[1], usize::MAX);
    assert_eq!(loaded.agents.current_path[1], Vec::<u32>::new());
    assert_eq!(loaded.agents.current_path_index[1], 0);
    assert_eq!(loaded.agents.current_edge[1], 0);
    assert_eq!(loaded.agents.target_building[1], 0);
    assert_eq!(loaded.agents.next_replan_time[1], 0.0);
    assert_eq!(loaded.agents.planned_attach_node[0], 0);
    assert_eq!(loaded.agents.planned_detach_node[0], 1);
    assert_eq!(loaded.agents.planned_attach_lane_id[0], planned_lane_id);
    assert_eq!(loaded.agents.planned_detach_lane_id[0], planned_lane_id);
    assert_eq!(loaded.agents.planned_attach_lane_d[0], 3.5);
    assert_eq!(loaded.agents.planned_detach_lane_d[0], 7.5);
    assert_eq!(loaded.agents.access_flags[0], ACCESS_PLAN_VALID);
    assert_eq!(loaded.agents.next_replan_time[0], 9.5);
    assert_eq!(loaded.agents.sim_time, agents_sys.sim_time);
    assert_eq!(loaded.allocator.buildings[0].frontage_t, 0.5);
    assert!((loaded.allocator.buildings[0].service_funding_override - 0.65).abs() < 0.001);
    assert_eq!(
        loaded.allocator.buildings[0].profit_tax_budget_baseline,
        375.0
    );
    assert_eq!(loaded.allocator.buildings[0].last_day_profit, 125.0);
    assert_eq!(
        loaded.allocator.buildings[0].daily_household_sales_value,
        123.0
    );
    assert_eq!(
        loaded.allocator.buildings[0].daily_power_service_units,
        78.0
    );
    assert_eq!(loaded.allocator.buildings[0].daily_power_served_units, 77.0);
    assert_eq!(
        loaded.allocator.buildings[0].recent_power_service_units,
        79.0
    );
    assert_eq!(
        loaded.allocator.buildings[0].recent_power_served_units,
        76.0
    );
    assert_eq!(
        loaded.allocator.buildings[0].recent_household_sales_value,
        456.0
    );
    assert_eq!(loaded.treasury.balance, treasury.balance);
    assert_eq!(
        loaded.treasury.last_daily_business_profit_tax,
        treasury.last_daily_business_profit_tax
    );
    assert_eq!(
        loaded.treasury.last_daily_residential_property_tax,
        treasury.last_daily_residential_property_tax
    );
    assert_eq!(
        loaded.treasury.last_daily_commercial_property_tax,
        treasury.last_daily_commercial_property_tax
    );
    assert_eq!(
        loaded.treasury.last_daily_industrial_property_tax,
        treasury.last_daily_industrial_property_tax
    );
    assert_eq!(
        loaded.treasury.pending_business_profit_tax,
        treasury.pending_business_profit_tax
    );
    assert_eq!(
        loaded.treasury.pending_residential_property_tax,
        treasury.pending_residential_property_tax
    );
    assert_eq!(
        loaded.treasury.pending_commercial_property_tax,
        treasury.pending_commercial_property_tax
    );
    assert_eq!(
        loaded.treasury.pending_industrial_property_tax,
        treasury.pending_industrial_property_tax
    );
    assert!(
        (loaded.service_policy.electricity_funding - service_policy.electricity_funding).abs()
            < 0.001
    );
    assert!((loaded.fiscal_policy.pension_per_elder_per_day - 42.0).abs() < 0.001);
    assert!((loaded.fiscal_policy.child_support_per_child_per_day - 11.0).abs() < 0.001);
    assert!((loaded.fiscal_policy.income_tax_rate - 0.22).abs() < 0.001);
    assert_eq!(loaded.budget_history.len(), 1);
    let loaded_budget = loaded.budget_history[0];
    assert_eq!(loaded_budget.day_index, 3);
    assert_eq!(loaded_budget.income, 120.0);
    assert_eq!(loaded_budget.expenses, 80.0);
    assert_eq!(loaded_budget.net, 40.0);
    assert_eq!(loaded_budget.treasury, 1_040.0);
    assert_eq!(loaded_budget.tax_income, 80.0);
    assert_eq!(loaded_budget.income_tax, 30.0);
    assert_eq!(loaded_budget.household_vat, 20.0);
    assert_eq!(loaded_budget.business_profit_tax, 12.0);
    assert_eq!(loaded_budget.property_tax, 18.0);
    assert_eq!(loaded_budget.residential_property_tax, 7.0);
    assert_eq!(loaded_budget.commercial_property_tax, 6.0);
    assert_eq!(loaded_budget.industrial_property_tax, 5.0);
    assert_eq!(loaded_budget.unemployment_benefits, 4.0);
    assert_eq!(loaded_budget.pensions, 5.0);
    assert_eq!(loaded_budget.child_support, 1.0);
    assert_eq!(loaded_budget.power_coverage, 0.95);
    assert_eq!(loaded_budget.electricity_net, 10.0);
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    assert_eq!(
        loaded.allocator.buildings[0].inventory_units(packaged_food),
        42.0
    );
    assert_eq!(loaded.logistics.shipments.len(), 1);
    assert_eq!(
        loaded.logistics.shipments[0].destination,
        ShipmentEndpoint::Building(0)
    );
    assert_eq!(
        loaded.logistics.request_failures.get(&FreightRequestKey {
            destination_building_id: 0,
            resource_runtime_id: household_supplies,
        }),
        Some(&FreightRequestFailure {
            failures: 2,
            terminal: true,
        })
    );
    assert_eq!(
        loaded.logistics.owa_export_saturation_units()[household_supplies as usize - 1],
        120.0
    );
}

#[test]
fn load_quarantines_invalid_legacy_saved_parcels() {
    let config = WorldConfig::new(200.0, 200.0, 10.0, 10.0);
    let time = TimeSystem::new();
    let terrain = TerrainSystem::from_world_config(&config);
    let water = WaterSystem::from_world_config(&config);
    let resource_deposits = ResourceDepositSystem::from_world_config(&config);
    let pollution = PollutionSystem::new(&config);
    let noise = NoiseSystem::new(&config);
    let demand = DemandSystem::new();
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-60.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
    let edge_id = graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: DEFAULT_URBAN_ROAD_SPEED_MS,
        base_cost: 120.0,
        physical_length: 120.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });

    let mut zoning = ZoningSystem::new(&config);
    let residential_profile = zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .expect("residential runtime id");
    zoning
        .restore_parcel_from_attachment(1, edge_id, 1, 0.5, 20.0, 20.0, residential_profile, &graph)
        .expect("initial parcel is valid before legacy road edit");

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "legacy_overlap_residential",
        ZoneClass::Residential,
    );
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    allocator.buildings.push(Building {
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 2,
        depth_cells: 2,
        zone_profile_runtime_id: residential_profile,
        parcel_id: 1,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        budget_distress: false,
        is_deserted: false,
        edge_idx: edge_id,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: residential_asset,
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: vec![0.0; catalog.resource_count()],
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
    });
    world::repaint_building_occupancy(&mut zoning, &allocator).expect("occupancy");

    let households = HouseholdSystem::new();
    let logistics = ShipmentSystem::new();
    let resource_extraction = ResourceExtractionSystem::new();
    let agriculture = AgricultureSystem::new();
    let agents_sys = AgentSystem::new();
    let mut network_sys = TransitNetwork::new();
    network_sys.lane_system.rebuild(&mut graph);
    let treasury = CityTreasury::new(1_000.0);
    let service_policy = CityServicePolicy::default();
    let fiscal_policy = CityFiscalPolicy::default();
    let budget_history = VecDeque::new();
    let mut pending_demand_spawns = VecDeque::new();
    pending_demand_spawns.push_back(PendingDemandSpawnAction {
        due_minute: 10,
        zone_type: ZoneType::Residential,
        action: DemandSpawnAction {
            parcel_id: 1,
            asset_id: "test:legacy_overlap_residential".to_owned(),
        },
        planned_day_index: 1,
        planned_minute_of_day: 10,
    });
    pending_demand_spawns.push_back(PendingDemandSpawnAction {
        due_minute: 20,
        zone_type: ZoneType::Residential,
        action: DemandSpawnAction {
            parcel_id: 3,
            asset_id: "test:legacy_overlap_residential".to_owned(),
        },
        planned_day_index: 1,
        planned_minute_of_day: 20,
    });

    let path = temp_path("legacy_overlap");
    save_to_sqlite(
        &path,
        SaveGameView {
            config: &config,
            time: &time,
            terrain: &terrain,
            water: &water,
            resource_deposits: &resource_deposits,
            graph: &graph,
            zoning: &zoning,
            pollution: &pollution,
            noise: &noise,
            demand: &demand,
            pending_demand_spawns: &pending_demand_spawns,
            allocator: &allocator,
            households: &households,
            logistics: &logistics,
            resource_extraction: &resource_extraction,
            agriculture: &agriculture,
            agents: &agents_sys,
            network: &network_sys,
            treasury: &treasury,
            service_policy: &service_policy,
            fiscal_policy: &fiscal_policy,
            budget_history: &budget_history,
        },
    )
    .expect("save");

    {
        let conn = rusqlite::Connection::open(&path).expect("open saved sqlite");
        conn.execute(
            "INSERT INTO network_nodes(node_id, x, y, z, node_type) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![2_i64, 0.0_f32, 0.0_f32, -80.0_f32, 0_i64],
        )
        .expect("insert crossing start node");
        conn.execute(
            "INSERT INTO network_nodes(node_id, x, y, z, node_type) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![3_i64, 0.0_f32, 0.0_f32, 80.0_f32, 0_i64],
        )
        .expect("insert crossing end node");
        conn.execute(
            "INSERT INTO network_edges(edge_id, start_node, end_node, primary_type, allowed_types, class, width, fwd_lanes, bkw_lanes, speed_limit, base_cost, physical_length, current_congestion, start_clip, end_clip, no_building_spawn, vehicle_frontage_access)
             SELECT 1, 2, 3, primary_type, allowed_types, class, width, fwd_lanes, bkw_lanes, speed_limit, 160.0, 160.0, current_congestion, start_clip, end_clip, no_building_spawn, vehicle_frontage_access FROM network_edges WHERE edge_id = 0",
            [],
        )
        .expect("insert crossing edge");
        for physical in [0_i64, 1_i64] {
            conn.execute(
                "INSERT INTO network_edge_geometry(edge_id, point_index, x, y, z, physical) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![1_i64, 0_i64, 0.0_f32, 0.0_f32, -80.0_f32, physical],
            )
            .expect("insert crossing geometry start");
            conn.execute(
                "INSERT INTO network_edge_geometry(edge_id, point_index, x, y, z, physical) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![1_i64, 1_i64, 0.0_f32, 0.0_f32, 80.0_f32, physical],
            )
            .expect("insert crossing geometry end");
        }
        conn.execute(
            "INSERT INTO zoning_parcels(parcel_id, edge_id, side, frontage_t, frontage_m, depth_m, profile_runtime_id)
             SELECT 2, edge_id, side, 0.65, frontage_m, depth_m, profile_runtime_id FROM zoning_parcels WHERE parcel_id = 1",
            [],
        )
        .expect("insert overlapping neighboring parcel");
        conn.execute(
            "INSERT INTO zoning_parcels(parcel_id, edge_id, side, frontage_t, frontage_m, depth_m, profile_runtime_id)
             SELECT 3, edge_id, side, 0.80, frontage_m, depth_m, profile_runtime_id FROM zoning_parcels WHERE parcel_id = 1",
            [],
        )
        .expect("insert second overlapping neighboring parcel");
    }

    let loaded = load_from_sqlite(&path, &allocator.registry).expect("load legacy overlap");
    fs::remove_file(&path).ok();

    assert_eq!(loaded.zoning.parcels().len(), 1);
    assert_eq!(loaded.zoning.parcels()[0].id().raw(), 2);
    assert!(loaded.allocator.buildings.is_empty());
    assert!(loaded.pending_demand_spawns.is_empty());
}
