// SPDX-License-Identifier: GPL-2.0-only

//! Shared allocator fixtures, asset registration, and startup-demand setup.

use super::*;

pub(super) fn zone_bucket(zone: ZoneType) -> usize {
    baseline_private_zone_slot(zone).expect("tests should only query baseline private zones")
}

pub(super) fn flat_test_terrain() -> TerrainSystem {
    TerrainSystem::new(32, 32)
}

pub(super) fn indexed_test_building(asset_id: String, zone_type: ZoneType, idx: i32) -> Building {
    Building {
        center_x: idx as f32 * 16.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 3,
        depth_cells: 3,
        zone_profile_runtime_id: 0,
        parcel_id: idx.max(0) as u64,
        zone_type,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.5,
        side_offset: 0.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: 1,
        cell_x: idx.max(0) as usize,
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
    }
}

pub(super) fn compiled_flat_test_terrain(
    network: &mut TransitNetwork,
    graph: &RegionGraph,
) -> TerrainSystem {
    let terrain = flat_test_terrain();
    network.road_surface.compile_dirty(graph, &terrain);
    terrain
}

pub(super) fn paint_zone_rect(
    zoning: &mut crate::simulation::zoning::ZoningSystem,
    graph: &RegionGraph,
    x0: f32,
    z0: f32,
    x1: f32,
    z1: f32,
    zone: ZoneType,
) {
    let runtime_id = zoning
        .profiles
        .default_runtime_id_for_zone_type(zone)
        .unwrap_or(0);
    let min_x = x0.min(x1);
    let max_x = x0.max(x1);
    let min_z = z0.min(z1);
    let max_z = z0.max(z1);
    let parcel_ids: Vec<u64> = zoning
        .parcels()
        .iter()
        .filter(|parcel| {
            let center = parcel.center();
            center.x >= min_x && center.x <= max_x && center.y >= min_z && center.y <= max_z
        })
        .map(|parcel| parcel.id().raw())
        .collect();
    for parcel_id in parcel_ids {
        if let Some(parcel) = zoning.parcel_by_raw_id_mut(parcel_id) {
            parcel.set_zone_profile_runtime_id(runtime_id);
        }
    }
    for edge_idx in 0..graph.edge_count() {
        let edge = graph.edge(edge_idx);
        if edge.deleted || edge.physical_length < 20.0 || edge.physical_geometry.len() < 2 {
            continue;
        }
        let count = (edge.physical_length / 20.0).floor() as usize;
        for side in [1_i8, -1_i8] {
            for i in 0..count {
                let s_m = (i as f32 + 0.5) * 20.0;
                let t = s_m / edge.physical_length;
                let geometry = crate::simulation::zoning::parcels::geometry_from_attachment(
                    graph, edge_idx, side, t, 20.0, 30.0,
                );
                let center = geometry.center;
                if center.x < min_x || center.x > max_x || center.y < min_z || center.y > max_z {
                    continue;
                }
                let _ = zoning.restore_parcel_from_attachment(
                    edge_idx as u64 * 10_000 + side.max(0) as u64 * 1_000 + i as u64 + 1,
                    edge_idx,
                    side,
                    t,
                    20.0,
                    30.0,
                    runtime_id,
                    graph,
                );
            }
        }
    }
}

/// Registers a minimal 1×1 building asset for the given zone type so placement tests pass.
pub(super) fn register_test_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
) -> String {
    register_test_asset_with_family(allocator, pack_id, asset_id, zone, None)
}

pub(super) fn register_test_asset_with_family(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
    asset_set: Option<&str>,
) -> String {
    register_test_asset_with_family_level(allocator, pack_id, asset_id, zone, asset_set, 1)
}

pub(super) fn register_test_asset_with_family_level(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
    asset_set: Option<&str>,
    level: u8,
) -> String {
    let (household_capacity, worker_capacity) = match zone {
        ZoneClass::Residential => (Some(6), None),
        ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
        ZoneClass::Mixed => (Some(4), Some(2)),
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
            flat_size_m2: if matches!(zone, ZoneClass::Residential | ZoneClass::Mixed) {
                Some(80.0)
            } else {
                None
            },
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(zone),
            density: Some("low".to_owned()),
            lot_width_cells: 1,
            lot_depth_cells: 1,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level,
            household_capacity,
            worker_capacity,
            service_class: None,
            economy_profile: match zone {
                ZoneClass::Commercial => Some("grocery_basic".to_owned()),
                ZoneClass::Industrial => Some("food_processor_basic".to_owned()),
                _ => None,
            },
            extractor: None,
            field: None,
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator
        .registry
        .register(pack_id, manifest, String::new());
    format!("{pack_id}:{asset_id}")
}

pub(super) fn register_test_power_service_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
) -> String {
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Test Power Service".to_owned(),
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
            economy_profile: Some("power_plant_basic".to_owned()),
            extractor: None,
            field: None,
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator
        .registry
        .register(pack_id, manifest, String::new());
    format!("{pack_id}:{asset_id}")
}

pub(super) fn stable_hash_bytes(parts: &[&[u8]]) -> u64 {
    let mut state = 0xcbf29ce484222325u64;
    for part in parts {
        for &byte in *part {
            state ^= byte as u64;
            state = state.wrapping_mul(0x100000001b3);
        }
        state ^= 0xff;
        state = state.wrapping_mul(0x100000001b3);
    }
    state
}

pub(super) fn stable_strip_family_hash(
    profile_runtime_id: u16,
    parcel_id: u64,
    family_key: &str,
) -> u64 {
    stable_hash_bytes(&[
        &profile_runtime_id.to_le_bytes(),
        &parcel_id.to_le_bytes(),
        family_key.as_bytes(),
    ])
}

pub(super) fn stable_site_variant_hash(
    profile_runtime_id: u16,
    parcel_id: u64,
    qualified_asset_id: &str,
) -> u64 {
    stable_hash_bytes(&[
        &profile_runtime_id.to_le_bytes(),
        &parcel_id.to_le_bytes(),
        qualified_asset_id.as_bytes(),
    ])
}

pub(super) fn frontage_profile_runtime_id_for_building(
    allocator: &BuildingAllocator,
    building: &Building,
    zoning: &crate::simulation::zoning::ZoningSystem,
    graph: &RegionGraph,
) -> u16 {
    let _ = (allocator, graph);
    zoning
        .parcel_by_raw_id(building.parcel_id)
        .map(|parcel| parcel.zone_profile_runtime_id())
        .unwrap_or(0)
}

pub(super) fn execute_startup_demand_building_pass(
    allocator: &mut BuildingAllocator,
    zoning: &mut crate::simulation::zoning::ZoningSystem,
    agents: &mut AgentSystem,
    households: &mut HouseholdSystem,
    logistics: &mut ShipmentSystem,
    network: &mut TransitNetwork,
    graph: &RegionGraph,
) {
    use crate::simulation::economy::demand::DemandSystem;

    let mut demand = DemandSystem::new();
    let terrain = compiled_flat_test_terrain(network, graph);
    for _ in 0..24 {
        let building_count_before = allocator.buildings.len();
        demand.run_hourly_pass(allocator, households, graph, zoning, 1_000.0);
        allocator.execute_demand_building_actions(
            &demand.building_actions,
            zoning,
            agents,
            households,
            logistics,
            graph,
            &network.lane_system,
            &network.road_surface,
            &terrain,
            demand.runtime_catalog(),
            demand.runtime_tuning(),
        );
        if allocator.buildings.len() > building_count_before {
            break;
        }
    }
}

pub(super) fn setup_startup_spawn_city_for_rezoning() -> (
    BuildingAllocator,
    crate::simulation::zoning::ZoningSystem,
    AgentSystem,
    HouseholdSystem,
    ShipmentSystem,
    crate::simulation::network::TransitNetwork,
    RegionGraph,
    usize,
) {
    use crate::simulation::economy::demand::{DemandBuildingActionPlan, DemandSystem};
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::types::NodeType;
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
    graph.set_node_type(0, NodeType::Border);
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

    let mut demand = DemandSystem::new();
    // Jack up demand and credits to ensure we get buildings in the first tick
    demand.residential = 1.0;
    demand.commercial = 1.0;
    demand.spawn_action_credit.residential = 10.0;
    demand.spawn_action_credit.commercial = 10.0;

    demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000.0);
    let mut startup_plan = DemandBuildingActionPlan::default();
    if let Some(action) = demand.building_actions.residential.spawns.first() {
        startup_plan.residential.spawns.push(action.clone());
    }
    if let Some(action) = demand.building_actions.commercial.spawns.first() {
        startup_plan.commercial.spawns.push(action.clone());
    }
    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    allocator.execute_demand_building_actions(
        &startup_plan,
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

    allocator.execute_demand_household_admission(2, &mut agents, &network, &graph); // Occupy buildings to protect from instant removal

    // Commercial demand cannot fire before households exist (goods_shortage=0 → base_commercial=0),
    // so push one commercial building directly to give rezoning tests a 2-building city.
    {
        let zone_cell_m = map_cfg.zone_cell_m;
        let parcel = zoning
            .parcels()
            .iter()
            .find(|parcel| {
                zoning
                    .profiles
                    .zone_type_for_runtime_id(parcel.zone_profile_runtime_id())
                    == ZoneType::Commercial
                    && parcel.is_available()
            })
            .expect("commercial test parcel")
            .clone();
        let edge = graph.edge(parcel.edge_idx());
        let curb_dist = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH;
        let center = parcel.front_center() + parcel.normal() * (zone_cell_m * 0.5);
        let building_idx = allocator.buildings.len();
        allocator.buildings.push(Building {
            center_x: center.x,
            center_y: center.y,
            support_height_m: 0.0,
            width_cells: 1,
            depth_cells: 1,
            zone_profile_runtime_id: parcel.zone_profile_runtime_id(),
            parcel_id: parcel.id().raw(),
            zone_type: ZoneType::Commercial,
            facing_dir: parcel.normal(),
            frontage_t: parcel.frontage_center_t(),
            side_offset: curb_dist,
            is_deserted: false,
            budget_distress: false,
            edge_idx: parcel.edge_idx(),
            side: parcel.side(),
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            service_funding_override: -1.0,
            asset_id: "base:b.com.shop".to_owned(),
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
        zoning.occupy_parcel(parcel.id().raw(), building_idx);
        allocator.rebuild_zone_index();
    }

    let residential_idx = allocator
        .buildings
        .iter()
        .position(|building| building.zone_type == ZoneType::Residential)
        .expect("pioneer demand should create one seeded residential building for rezoning tests");

    (
        allocator,
        zoning,
        agents,
        households,
        logistics,
        network,
        graph,
        residential_idx,
    )
}
