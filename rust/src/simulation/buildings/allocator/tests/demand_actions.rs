//! Demand spawn, upgrade, and lifecycle action tests.

use super::support::*;
use super::*;

#[test]
fn test_demand_building_spawn_plan_executes_from_hourly_budget() {
    use crate::simulation::economy::demand::DemandSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    let mut zoning = crate::simulation::zoning::ZoningSystem::new(&WorldConfig::default());
    let mut graph = RegionGraph::new();
    let mut network = crate::simulation::network::TransitNetwork::new();
    let mut agents = AgentSystem::new();
    let mut logistics = ShipmentSystem::new();
    let mut households = HouseholdSystem::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(120.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    graph.set_node_type(0, crate::simulation::network::types::NodeType::Border);
    paint_zone_rect(
        &mut zoning,
        &graph,
        -40.0,
        -40.0,
        80.0,
        40.0,
        ZoneType::Residential,
    );

    let mut demand = DemandSystem::new();
    for _ in 0..24 {
        demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000.0);
        if !demand.building_actions.residential.spawns.is_empty() {
            break;
        }
    }
    assert!(
        !demand.building_actions.residential.spawns.is_empty(),
        "pioneer demand and legal zoning should produce at least one residential spawn action across hourly demand credit"
    );

    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    allocator.execute_demand_building_actions(
        &demand.building_actions,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &graph,
        &network.lane_system,
        &network.road_surface,
        &terrain,
        demand.runtime_catalog(),
        demand.runtime_tuning(),
    );

    assert!(
        allocator
            .buildings
            .iter()
            .any(|building| building.asset_id == residential_asset),
        "the demand-owned residential spawn plan should place a real building"
    );
}

#[test]
fn test_execute_demand_building_actions_applies_despawn_downgrade_and_upgrade() {
    use crate::simulation::economy::demand::{
        DemandBuildingActionKey, DemandBuildingActionPlan, DemandLevelChangeAction,
        DemandSpawnAction, DemandSystem,
    };
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let residential_level_1 = register_test_asset_with_family_level(
        &mut allocator,
        "base",
        "b.res.family_l1",
        ZoneClass::Residential,
        Some("res_family"),
        1,
    );
    let residential_level_2 = register_test_asset_with_family_level(
        &mut allocator,
        "base",
        "b.res.family_l2",
        ZoneClass::Residential,
        Some("res_family"),
        2,
    );
    let mut zoning = crate::simulation::zoning::ZoningSystem::new(&WorldConfig::default());
    let mut graph = RegionGraph::new();
    let mut network = crate::simulation::network::TransitNetwork::new();
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(120.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    paint_zone_rect(
        &mut zoning,
        &graph,
        -40.0,
        -40.0,
        120.0,
        40.0,
        ZoneType::Residential,
    );
    let occupied_parcels = {
        let mut parcels = zoning
            .parcels()
            .iter()
            .filter(|parcel| {
                parcel.edge_idx() == 0
                    && parcel.side() == 1
                    && parcel.is_available()
                    && zoning
                        .profiles
                        .zone_type_for_runtime_id(parcel.zone_profile_runtime_id())
                        == ZoneType::Residential
            })
            .map(|parcel| (parcel.frontage_center_t(), parcel.id().raw()))
            .collect::<Vec<_>>();
        parcels.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.cmp(&right.1)));
        [parcels[0].1, parcels[2].1, parcels[4].1]
    };

    allocator.buildings.push(Building {
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 1,
        depth_cells: 1,
        zone_profile_runtime_id: 0,
        parcel_id: occupied_parcels[0],
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, -1.0),
        frontage_t: 0.0,
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 6,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: residential_level_1.clone(),
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: Vec::new(),
        revenue: 0.0,
        operating_budget: 0.0,
        profit_tax_budget_baseline: 0.0,
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
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    allocator.buildings.push(Building {
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 1,
        depth_cells: 1,
        zone_profile_runtime_id: 0,
        parcel_id: occupied_parcels[1],
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, -1.0),
        frontage_t: 0.0,
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: residential_level_2.clone(),
        level: 2,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: Vec::new(),
        revenue: 0.0,
        operating_budget: 0.0,
        profit_tax_budget_baseline: 0.0,
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
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    allocator.buildings.push(Building {
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 1,
        depth_cells: 1,
        zone_profile_runtime_id: 0,
        parcel_id: occupied_parcels[2],
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, -1.0),
        frontage_t: 0.0,
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: residential_level_1.clone(),
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: Vec::new(),
        revenue: 0.0,
        operating_budget: 0.0,
        profit_tax_budget_baseline: 0.0,
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
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    for (building_idx, &parcel_id) in occupied_parcels.iter().enumerate() {
        zoning.occupy_parcel(parcel_id, building_idx);
    }
    allocator
        .recompute_derived_transforms(&graph, &zoning)
        .expect("building transforms should rebuild for test fixtures");
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let spawn_parcel_id = zoning
        .parcels()
        .iter()
        .find(|parcel| {
            parcel.id().raw() != 0
                && parcel.is_available()
                && zoning
                    .profiles
                    .zone_type_for_runtime_id(parcel.zone_profile_runtime_id())
                    == ZoneType::Residential
        })
        .expect("residential spawn parcel")
        .id()
        .raw();

    let mut plan = DemandBuildingActionPlan::default();
    plan.residential.despawns.push(DemandBuildingActionKey {
        parcel_id: occupied_parcels[2],
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        width_cells: 1,
        depth_cells: 1,
        level: 1,
        asset_id: residential_level_1.clone(),
    });
    plan.residential.downgrades.push(DemandLevelChangeAction {
        building: DemandBuildingActionKey {
            parcel_id: occupied_parcels[1],
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            width_cells: 1,
            depth_cells: 1,
            level: 2,
            asset_id: residential_level_2.clone(),
        },
        target_asset_id: residential_level_1.clone(),
    });
    plan.residential.upgrades.push(DemandLevelChangeAction {
        building: DemandBuildingActionKey {
            parcel_id: occupied_parcels[0],
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            width_cells: 1,
            depth_cells: 1,
            level: 1,
            asset_id: residential_level_1.clone(),
        },
        target_asset_id: residential_level_2.clone(),
    });
    plan.residential.spawns.push(DemandSpawnAction {
        parcel_id: spawn_parcel_id,
        asset_id: residential_level_1.clone(),
    });

    let demand = DemandSystem::new();
    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    allocator.execute_demand_building_actions(
        &plan,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &graph,
        &network.lane_system,
        &network.road_surface,
        &terrain,
        demand.runtime_catalog(),
        demand.runtime_tuning(),
    );

    let spawned_exists = allocator
        .buildings
        .iter()
        .any(|building| building.parcel_id == spawn_parcel_id);
    assert!(
        spawned_exists,
        "building_count={} spawn_parcel={} spawned_exists={}",
        allocator.buildings.len(),
        spawn_parcel_id,
        spawned_exists
    );

    assert_eq!(allocator.buildings.len(), 3);
    assert!(
        allocator.buildings.iter().any(|building| {
            building.parcel_id == occupied_parcels[0]
                && building.asset_id == residential_level_2
                && building.level == 2
        }),
        "upgrade action should replace the first building with the next family level"
    );
    assert!(
        allocator.buildings.iter().any(|building| {
            building.parcel_id == occupied_parcels[1]
                && building.asset_id == residential_level_1
                && building.level == 1
        }),
        "downgrade action should replace the second building with the previous family level"
    );
    assert!(
        allocator
            .buildings
            .iter()
            .all(|building| building.parcel_id != occupied_parcels[2]),
        "despawn action should remove the empty third building"
    );
    let spawned = allocator
        .buildings
        .iter()
        .find(|building| building.parcel_id == spawn_parcel_id)
        .expect("spawn action should place a building on the selected parcel");
    assert!(spawned.operating_budget.abs() <= f32::EPSILON);
}

#[test]
fn test_commercial_demand_spawn_startup_budget_includes_first_import_cost() {
    use crate::simulation::economy::demand::{
        DemandBuildingActionPlan, DemandSpawnAction, DemandSystem,
    };
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let commercial_asset =
        register_test_asset(&mut allocator, "base", "b.com.shop", ZoneClass::Commercial);
    let mut zoning = crate::simulation::zoning::ZoningSystem::new(&WorldConfig::default());
    let mut graph = RegionGraph::new();
    let mut network = crate::simulation::network::TransitNetwork::new();
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(120.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    paint_zone_rect(
        &mut zoning,
        &graph,
        -40.0,
        -40.0,
        120.0,
        40.0,
        ZoneType::Commercial,
    );
    let parcel_id = zoning
        .parcels()
        .iter()
        .find(|parcel| {
            parcel.is_available()
                && zoning
                    .profiles
                    .zone_type_for_runtime_id(parcel.zone_profile_runtime_id())
                    == ZoneType::Commercial
        })
        .expect("commercial spawn parcel")
        .id()
        .raw();

    let mut plan = DemandBuildingActionPlan::default();
    plan.commercial.spawns.push(DemandSpawnAction {
        parcel_id,
        asset_id: commercial_asset,
    });

    let demand = DemandSystem::new();
    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    allocator.execute_demand_building_actions(
        &plan,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &graph,
        &network.lane_system,
        &network.road_surface,
        &terrain,
        demand.runtime_catalog(),
        demand.runtime_tuning(),
    );

    let building = allocator
        .buildings
        .iter()
        .find(|building| building.parcel_id == parcel_id)
        .expect("commercial demand spawn should create a building");
    let profile = demand
        .runtime_catalog()
        .profile_by_runtime_id(building.economy_profile_runtime_id)
        .expect("spawned commercial building should have a profile");
    let first_import_base_cost = profile
        .inputs
        .iter()
        .map(|port| {
            profile.inventory_target_units_for(port)
                * demand
                    .runtime_catalog()
                    .unit_price_for_resource(port.resource_runtime_id)
                    .expect("input resource should have a unit price")
                * demand.runtime_tuning().owa_import_price_multiplier
        })
        .sum::<f32>();
    assert!(first_import_base_cost > 0.0);
    let expected_startup_budget =
        (profile.worker_capacity as f32 * profile.average_daily_wage() * 7.0
            + first_import_base_cost)
            .max(500.0);
    assert!((building.operating_budget - expected_startup_budget).abs() <= 0.01);
}
