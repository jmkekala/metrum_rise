// SPDX-License-Identifier: GPL-2.0-only

//! Runtime accumulator, attachment repair, and allocator tick tests.

use super::support::*;
use super::*;

#[test]
fn reset_daily_accumulators_rolls_power_output_for_summaries() {
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(Building {
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 1,
        depth_cells: 1,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type: ZoneType::Industrial,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.0,
        side_offset: 0.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: "base:power".to_owned(),
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
        daily_owa_input_value: 4.0,
        daily_local_input_value: 3.0,
        daily_city_funded_input_cost: 2.0,
        daily_household_sales_value: 1.0,
        daily_power_service_units: 78.0,
        daily_power_served_units: 56.0,
        recent_power_service_units: 0.0,
        recent_power_served_units: 0.0,
        recent_household_sales_value: 0.0,
        commercial_activity_floor_scale: 0.0,
        work_area_scale: 1.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });

    allocator.reset_daily_input_accumulators();

    let building = &allocator.buildings[0];
    assert_eq!(building.daily_power_service_units, 0.0);
    assert_eq!(building.recent_power_service_units, 78.0);
    assert_eq!(building.daily_power_served_units, 0.0);
    assert_eq!(building.recent_power_served_units, 56.0);
    assert_eq!(building.daily_city_funded_input_cost, 0.0);
    assert_eq!(building.recent_household_sales_value, 1.0);
}

#[test]
fn test_tick_does_not_auto_spawn_private_buildings_from_zones() {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    register_test_asset(
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

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert_eq!(
        allocator.buildings.len(),
        0,
        "Zoning alone should not create buildings when the founding prerequisites are incomplete"
    );
}

#[test]
fn repair_road_attachments_reprojects_far_stored_edge() {
    use godot::prelude::Vector3;

    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    let config = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&config);
    let mut allocator = BuildingAllocator::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let near_edge = graph.edge_count() - 1;
    network.add_road(
        &mut graph,
        vec![Vector3::new(300.0, 0.0, 0.0), Vector3::new(400.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let far_edge = graph.edge_count() - 1;

    let zone_cell_m = zoning.config.zone_cell_m;
    let side = -1;
    let depth_cells = 2;
    let frontage_t = 0.5;
    let near_centerline = BuildingAllocator::sample_pos_on_edge(&graph, near_edge, frontage_t);
    let tangent = BuildingAllocator::sample_tangent_on_edge(&graph, near_edge, frontage_t);
    let outward = Vector2::new(tangent.y, -tangent.x).normalized() * side as f32;
    let road_offset = graph.edge(near_edge).width * 0.5 + crate::config::SIDEWALK_WIDTH;
    let frontage_center = near_centerline + outward * road_offset;
    let center = frontage_center + outward * (depth_cells as f32 * zone_cell_m * 0.5);

    allocator.buildings.push(Building {
        center_x: center.x,
        center_y: center.y,
        support_height_m: 0.0,
        width_cells: 2,
        depth_cells,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type: ZoneType::Residential,
        facing_dir: -outward,
        frontage_t: 0.5,
        side_offset: graph.edge(far_edge).width * 0.5 + crate::config::SIDEWALK_WIDTH,
        is_deserted: false,
        budget_distress: false,
        edge_idx: far_edge,
        side,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: "test:repair".to_owned(),
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

    let repaired = allocator.repair_road_attachments_after_topology_edit(&graph, &mut zoning);

    assert_eq!(repaired, 1);
    assert_eq!(allocator.buildings[0].edge_idx, near_edge);
    assert_eq!(allocator.buildings[0].side, side);
    assert!((allocator.buildings[0].frontage_t - frontage_t).abs() <= 0.001);
}

#[test]
fn repair_road_attachments_reprojects_stale_parcel_attachment() {
    use godot::prelude::Vector3;

    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    let config = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&config);
    let mut allocator = BuildingAllocator::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let near_edge = graph.edge_count() - 1;
    network.add_road(
        &mut graph,
        vec![Vector3::new(300.0, 0.0, 0.0), Vector3::new(400.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let far_edge = graph.edge_count() - 1;

    let parcel_id = zoning
        .restore_parcel_from_attachment(77, far_edge, 1, 0.5, 20.0, 20.0, 0, &graph)
        .expect("far test parcel")
        .raw();

    let zone_cell_m = zoning.config.zone_cell_m;
    let side = -1;
    let depth_cells = 2;
    let frontage_t = 0.5;
    let near_centerline = BuildingAllocator::sample_pos_on_edge(&graph, near_edge, frontage_t);
    let tangent = BuildingAllocator::sample_tangent_on_edge(&graph, near_edge, frontage_t);
    let outward = Vector2::new(tangent.y, -tangent.x).normalized() * side as f32;
    let road_offset = graph.edge(near_edge).width * 0.5 + crate::config::SIDEWALK_WIDTH;
    let frontage_center = near_centerline + outward * road_offset;
    let center = frontage_center + outward * (depth_cells as f32 * zone_cell_m * 0.5);

    allocator.buildings.push(Building {
        center_x: center.x,
        center_y: center.y,
        support_height_m: 0.0,
        width_cells: 2,
        depth_cells,
        zone_profile_runtime_id: 0,
        parcel_id,
        zone_type: ZoneType::Residential,
        facing_dir: -outward,
        frontage_t: 0.5,
        side_offset: graph.edge(far_edge).width * 0.5 + crate::config::SIDEWALK_WIDTH,
        is_deserted: false,
        budget_distress: false,
        edge_idx: far_edge,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: "test:parcel_repair".to_owned(),
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
    zoning.occupy_parcel(parcel_id, 0);

    let repaired = allocator.repair_road_attachments_after_topology_edit(&graph, &mut zoning);

    let repaired_parcel = zoning
        .parcel_by_raw_id(parcel_id)
        .expect("parcel should still exist");
    assert_eq!(repaired, 1);
    assert_eq!(repaired_parcel.edge_idx(), near_edge);
    assert_eq!(repaired_parcel.side(), side);
    assert!((repaired_parcel.frontage_center_t() - frontage_t).abs() <= 0.001);
    assert_eq!(allocator.buildings[0].edge_idx, near_edge);
    assert_eq!(allocator.buildings[0].side, side);
    assert!((allocator.buildings[0].frontage_t - frontage_t).abs() <= 0.001);
}

#[test]
fn test_allocator_tick_does_not_place_founding_buildings() {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    register_test_asset(&mut allocator, "base", "b.com.shop", ZoneClass::Commercial);
    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
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
        -50.0,
        -50.0,
        45.0,
        50.0,
        ZoneType::Residential,
    );
    paint_zone_rect(
        &mut zoning,
        &graph,
        55.0,
        -50.0,
        150.0,
        50.0,
        ZoneType::Commercial,
    );

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert_eq!(
        allocator.buildings.len(),
        0,
        "allocator tick should no longer seed founding buildings on its own"
    );
}
