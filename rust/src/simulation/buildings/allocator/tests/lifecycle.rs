// SPDX-License-Identifier: GPL-2.0-only

//! Removal, immigration, and hourly admission lifecycle tests.

use super::support::*;
use super::*;

#[test]
fn test_building_removal_clears_zoning_occupancy() {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut network = TransitNetwork::new();
    let mut logistics = ShipmentSystem::new();
    let mut graph = RegionGraph::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );

    paint_zone_rect(
        &mut zoning,
        &graph,
        -50.0,
        -50.0,
        150.0,
        50.0,
        ZoneType::Residential,
    );
    let parcel = zoning
        .parcels()
        .iter()
        .find(|parcel| parcel.is_available())
        .expect("residential test parcel")
        .clone();
    let center = parcel.front_center() + parcel.normal() * (map_cfg.zone_cell_m * 0.5);
    allocator.buildings.push(Building {
        center_x: center.x,
        center_y: center.y,
        support_height_m: 0.0,
        width_cells: 1,
        depth_cells: 1,
        zone_profile_runtime_id: parcel.zone_profile_runtime_id(),
        parcel_id: parcel.id().raw(),
        zone_type: ZoneType::Residential,
        facing_dir: parcel.normal(),
        frontage_t: parcel.frontage_center_t(),
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: parcel.side(),
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id,
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: Vec::new(),
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
    zoning.occupy_parcel(parcel.id().raw(), 0);
    let commercial_profile = zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Commercial)
        .expect("commercial profile");
    zoning
        .parcel_by_raw_id_mut(parcel.id().raw())
        .expect("parcel")
        .set_zone_profile_runtime_id(commercial_profile);

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert_eq!(allocator.buildings.len(), 1);
    assert!(allocator.buildings[0].pending_redevelopment);

    for _ in 0..3 {
        allocator.tick(
            &mut zoning,
            &mut agents,
            &mut households,
            &mut logistics,
            &mut network,
            &mut graph,
        );
    }

    assert_eq!(
        allocator.buildings.len(),
        0,
        "Building should be removed after the rezoning grace expires"
    );
    assert!(
        zoning
            .parcel_by_raw_id(parcel.id().raw())
            .and_then(|parcel| parcel.occupied_building())
            .is_none(),
        "Parcel occupancy should be cleared after building removal"
    );
}

#[test]
fn test_immigration_claims_vacant_home() {
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::zoning::{ZoneType, ZoningSystem};
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let residential_asset_id = register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut network = TransitNetwork::new();
    let mut logistics = ShipmentSystem::new();
    let mut graph = RegionGraph::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let edge_id = graph.edge_count() - 1;

    graph.set_node_type(0, crate::simulation::network::types::NodeType::Border);
    paint_zone_rect(
        &mut zoning,
        &graph,
        -50.0,
        -50.0,
        150.0,
        50.0,
        ZoneType::Residential,
    );
    let parcel = zoning
        .parcels()
        .iter()
        .find(|parcel| {
            zoning
                .profiles
                .zone_type_for_runtime_id(parcel.zone_profile_runtime_id())
                == ZoneType::Residential
                && parcel.is_available()
        })
        .expect("residential test parcel")
        .clone();
    let center = parcel.front_center() + parcel.normal() * (map_cfg.zone_cell_m * 0.5);
    allocator.buildings.push(Building {
        center_x: center.x,
        center_y: center.y,
        support_height_m: 0.0,
        width_cells: 1,
        depth_cells: 1,
        zone_profile_runtime_id: parcel.zone_profile_runtime_id(),
        parcel_id: parcel.id().raw(),
        zone_type: ZoneType::Residential,
        facing_dir: parcel.normal(),
        frontage_t: parcel.frontage_center_t(),
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: edge_id,
        side: parcel.side(),
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: residential_asset_id,
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: Vec::new(),
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
    zoning.occupy_parcel(parcel.id().raw(), 0);
    allocator.rebuild_zone_index();

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );
    let expected_household_size = allocator
        .next_household_admission_candidate()
        .expect("vacant residential admission candidate")
        .1;
    allocator.execute_demand_household_admission(1, &mut agents, &network, &graph);

    assert_eq!(
        agents.len(),
        1,
        "One household should launch one arrival carrier from the demand-owned output"
    );
    assert_eq!(
        agents.home_building[0], 0,
        "Arrival carrier should reserve home index 0"
    );
    assert_eq!(
        agents.target_building[0], 0,
        "Arrival carrier should target its reserved home"
    );
    assert_eq!(
        agents.transit[0],
        crate::simulation::economy::agents::TRANSIT_IMMIGRATING,
        "Arrival carrier should start on the border-origin immigration path"
    );
    assert_eq!(
        agents.transit_mode[0],
        crate::simulation::economy::agents::MODE_CAR
    );
    assert_eq!(agents.pending_household_size[0], expected_household_size);
    assert_eq!(agents.current_building[0], usize::MAX);
    assert_eq!(agents.current_node[0], 0);
    assert_eq!(agents.current_lane_id[0], usize::MAX);
    assert_eq!(agents.access_flags[0], 0);
    let expected_door = allocator.entrances[0].door_pos;
    agents.transit[0] = crate::simulation::economy::agents::TRANSIT_IN_BUILDING;
    agents.current_building[0] = 0;
    agents.target_building[0] = usize::MAX;
    agents.pos_x[0] = expected_door.x;
    agents.pos_y[0] = expected_door.y;
    let mut treasury_balance = 0.0;
    households.operational_hour_tick(
        &mut agents,
        &mut allocator,
        &mut logistics,
        &network,
        &graph,
        0,
        0,
        &mut treasury_balance,
        &[],
        &crate::simulation::economy::fiscal::CityFiscalPolicy::default(),
    );
    assert_eq!(agents.len(), expected_household_size as usize);
    assert_eq!(households.households.len(), 1);
    assert_eq!(
        households.households[0].member_count,
        expected_household_size
    );
    for agent_idx in 0..expected_household_size as usize {
        assert_eq!(agents.household_id[agent_idx], agents.household_id[0]);
    }
    assert_eq!(agents.pending_household_size[0], 0);
    assert!((agents.pos_x[0] - expected_door.x).abs() < 1e-4);
    assert!((agents.pos_y[0] - expected_door.y).abs() < 1e-4);
    assert_eq!(
        allocator.buildings[0].occupancy, 1,
        "Building occupancy should match the admitted household count (1)"
    );
}

#[test]
fn test_hourly_startup_admission_avoids_zero_rounding() {
    use crate::simulation::economy::demand::DemandSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    let commercial_asset =
        register_test_asset(&mut allocator, "base", "b.com.shop", ZoneClass::Commercial);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut graph = RegionGraph::new();

    let mut zoning = crate::simulation::zoning::ZoningSystem::new(&WorldConfig::default());
    let mut network = crate::simulation::network::TransitNetwork::new();
    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let edge_id = graph.edge_count() - 1;
    graph.set_node_type(0, crate::simulation::network::types::NodeType::Border);
    let catalog = load_runtime_economy_catalog().expect("catalog");
    let grocery_profile_runtime_id = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile")
        .runtime_id;

    allocator.buildings.push(Building {
        center_x: 10.0,
        center_y: 10.0,
        support_height_m: 0.0,
        width_cells: 2,
        depth_cells: 2,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.1,
        side_offset: 1.0,
        budget_distress: false,
        is_deserted: false,
        edge_idx: edge_id,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 1,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: residential_asset.clone(),
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: Vec::new(),
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
    allocator.buildings.push(Building {
        center_x: 40.0,
        center_y: 10.0,
        support_height_m: 0.0,
        width_cells: 2,
        depth_cells: 2,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type: ZoneType::Commercial,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.4,
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: edge_id,
        side: 1,
        cell_x: 4,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: commercial_asset,
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: grocery_profile_runtime_id,
        economy_broken: false,
        resource_inventory: Vec::new(),
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
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let tuning =
        crate::simulation::economy::definitions::load_runtime_economy_tuning().expect("tuning");
    let household_id = households.admit_immigrant_household(&catalog, &tuning, 0, 2);
    for _ in 0..2 {
        let idx = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.household_id[idx] = household_id;
    }
    households.households[household_id].budget = 1_000.0;
    households.households[household_id].stock = 6.0;
    households.households[household_id].stock_days = 3.0;

    let zoning = crate::simulation::zoning::ZoningSystem::new(&WorldConfig::default());
    let mut demand = DemandSystem::new();
    for _ in 0..4 {
        demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000.0);
        if demand.households_to_admit_today > 0 {
            break;
        }
    }
    assert!(
        demand.households_to_admit_today > 0,
        "hourly demand credit should accumulate into a household-admission output from open-job pull; credit={:.3} residential={:.3}",
        demand.admission_action_credit,
        demand.residential,
    );
    allocator.execute_demand_household_admission(
        demand.households_to_admit_today,
        &mut agents,
        &network,
        &graph,
    );

    assert!(
        agents.pending_household_size.iter().any(|&size| size > 0),
        "player-seeded startup city should launch a pending household carrier through the demand-owned startup output"
    );
}
