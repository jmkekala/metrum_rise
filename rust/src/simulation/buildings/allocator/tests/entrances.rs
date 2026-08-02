//! Building entrance derivation and authored-scale tests.

use super::support::*;
use super::*;

#[test]
fn test_rebuild_entrance_cache_derives_anchor_and_lane_access() {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "base",
        "b.res.entrance_cache",
        ZoneClass::Residential,
    );

    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    graph.edge_mut(0).vehicle_frontage_access = VehicleFrontageAccess::BothSides;
    network.lane_system.rebuild(&mut graph);

    allocator.buildings.push(Building {
        center_x: 10.0,
        center_y: -10.0,
        support_height_m: 0.0,
        width_cells: 1,
        depth_cells: 1,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, -1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: 1,
        cell_x: 1,
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
        work_area_scale: 1.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });

    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    assert_eq!(allocator.entrances.len(), 1);
    let entrance = &allocator.entrances[0];
    assert_eq!(
        entrance.vehicle_frontage_access,
        VehicleFrontageAccess::BothSides
    );
    assert!(entrance.flags & 0x01 != 0, "foot access should be valid");
    assert!(entrance.flags & 0x02 != 0, "car access should be valid");
    assert_ne!(entrance.foot_lane_fwd, usize::MAX);
    assert_ne!(entrance.foot_lane_bkw, usize::MAX);
    assert_ne!(entrance.car_lane_fwd, usize::MAX);
    assert_ne!(entrance.car_lane_bkw, usize::MAX);
    assert_eq!(entrance.door_pos, Vector2::new(10.0, -10.5));
    assert_ne!(entrance.curb_pos, entrance.door_pos);
    assert!(entrance.entrance_s_m >= 0.0);
    assert!(entrance.entrance_s_m <= graph.edge(0).physical_length);
}

#[test]
fn test_rebuild_entrance_cache_uses_authored_anchor_meters_without_preview_scale() {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let manifest = AssetManifest {
        asset_id: "b.res.anchor_units".to_owned(),
        display_name: "Anchor Units".to_owned(),
        asset_set: None,
        tags: vec![],
        thumbnail: None,
        lods: vec![],
        mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [1.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
            width_m: None,
            length_m: None,
            vehicle_class: None,
        }],
        site_surfaces: vec![],
        building: Some(BuildingData {
            flat_size_m2: Some(80.0),
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(ZoneClass::Residential),
            density: Some("low".to_owned()),
            lot_width_cells: 1,
            lot_depth_cells: 1,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity: Some(6),
            worker_capacity: None,
            service_class: None,
            economy_profile: None,
            extractor: None,
            field: None,
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator.registry.register("base", manifest, String::new());

    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.lane_system.rebuild(&mut graph);

    allocator.buildings.push(Building {
        center_x: 10.0,
        center_y: -10.0,
        support_height_m: 0.0,
        width_cells: 1,
        depth_cells: 1,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, -1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: 1,
        cell_x: 1,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id: "base:b.res.anchor_units".to_owned(),
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
        work_area_scale: 1.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });

    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let entrance = &allocator.entrances[0];
    assert_eq!(
        entrance.door_pos,
        Vector2::new(9.0, -10.5),
        "authored entrance anchors are stored in lot-space meters and must not be scaled or shifted by mesh preview settings"
    );
    assert!(entrance.curb_pos.y > entrance.door_pos.y);

    let mut reverse_manifest = allocator
        .registry
        .get("base:b.res.anchor_units")
        .expect("registered anchor test asset")
        .manifest
        .clone();
    reverse_manifest.asset_id = "b.res.anchor_units_reverse".to_owned();
    reverse_manifest.anchors[0].position = [0.0, 0.0, -0.5];
    reverse_manifest.anchors[0].forward = [0.0, 0.0, -1.0];
    allocator
        .registry
        .register("base", reverse_manifest, String::new());

    let mut reverse_building = allocator.buildings[0].clone();
    reverse_building.asset_id = "base:b.res.anchor_units_reverse".to_owned();
    allocator.buildings.push(reverse_building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    assert_eq!(
        allocator.entrances[1].door_pos,
        Vector2::new(10.0, -10.5),
        "asset-local -Z frontage anchors must align with the same road-facing direction"
    );
}
