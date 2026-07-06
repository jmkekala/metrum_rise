//! Building allocator unit tests.

use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::types::VehicleFrontageAccess;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::ZoneType;
use godot::prelude::{Vector2, Vector3};
use rand::SeedableRng;

fn zone_bucket(zone: ZoneType) -> usize {
    baseline_private_zone_slot(zone).expect("tests should only query baseline private zones")
}

fn flat_test_terrain() -> TerrainSystem {
    TerrainSystem::new(32, 32)
}

fn indexed_test_building(asset_id: String, zone_type: ZoneType, idx: i32) -> Building {
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
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    }
}

fn compiled_flat_test_terrain(network: &mut TransitNetwork, graph: &RegionGraph) -> TerrainSystem {
    let terrain = flat_test_terrain();
    network.road_surface.compile_dirty(graph, &terrain);
    terrain
}

fn paint_zone_rect(
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
fn register_test_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
) -> String {
    register_test_asset_with_family(allocator, pack_id, asset_id, zone, None)
}

fn register_test_asset_with_family(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
    asset_set: Option<&str>,
) -> String {
    register_test_asset_with_family_level(allocator, pack_id, asset_id, zone, asset_set, 1)
}

fn register_test_asset_with_family_level(
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

fn register_test_power_service_asset(
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

#[test]
fn explicit_service_preview_rejects_site_overlapping_nearby_road() {
    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_power_service_asset(&mut allocator, "base", "building.power.test");

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 20.0), Vector3::new(100.0, 0.0, 20.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");

    let preview = allocator
        .preview_explicit_service_placement(
            &asset_id,
            Vector2::new(50.0, 8.0),
            map_cfg.zone_cell_m,
            &graph,
            &network.road_surface,
            &terrain,
            &catalog,
        )
        .expect("service placement should resolve to the nearest frontage");

    assert!(!preview.valid);
    assert_eq!(
        preview.rejection,
        Some(ExplicitServicePlacementRejection::RoadOverlap)
    );
}

#[test]
fn zoning_projection_overlaps_explicit_service_site_is_blocked() {
    let map_cfg = WorldConfig::default();
    let mut zoning = ZoningSystem::new(&map_cfg);
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_power_service_asset(&mut allocator, "base", "building.power.test");

    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let terrain = compiled_flat_test_terrain(&mut network, &graph);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning =
        crate::simulation::economy::definitions::load_runtime_economy_tuning().expect("tuning");

    allocator
        .execute_explicit_service_placement(
            &asset_id,
            Vector2::new(50.0, 8.0),
            map_cfg.zone_cell_m,
            &graph,
            &network.road_surface,
            &terrain,
            &catalog,
            &tuning,
        )
        .expect("flat service placement should commit");
    let parcel_geometry = zoning
        .preview_parcel_at(50.0, 8.0, 20.0, 20.0, &graph)
        .expect("zoning projection should still resolve geometrically");

    assert!(
        allocator.parcel_geometry_overlaps_explicit_site(&parcel_geometry),
        "explicit service lot should reserve land against zoning placement"
    );
}

fn stable_hash_bytes(parts: &[&[u8]]) -> u64 {
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

fn stable_strip_family_hash(profile_runtime_id: u16, parcel_id: u64, family_key: &str) -> u64 {
    stable_hash_bytes(&[
        &profile_runtime_id.to_le_bytes(),
        &parcel_id.to_le_bytes(),
        family_key.as_bytes(),
    ])
}

fn stable_site_variant_hash(
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

fn frontage_profile_runtime_id_for_building(
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

fn execute_startup_demand_building_pass(
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

fn setup_startup_spawn_city_for_rezoning() -> (
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

#[test]
fn test_zone_index_consistency() {
    let mut allocator = BuildingAllocator::new();
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "zone_index_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "zone_index_com",
        ZoneClass::Commercial,
    );

    for i in 0..10 {
        allocator.buildings.push(Building {
            center_x: i as f32,
            center_y: 0.0,
            support_height_m: 0.0,
            width_cells: 3,
            depth_cells: 3,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type: if i % 2 == 0 {
                ZoneType::Residential
            } else {
                ZoneType::Commercial
            },
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 0.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx: 0,
            side: 1,
            cell_x: i,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            service_funding_override: -1.0,
            asset_id: if i % 2 == 0 {
                residential_asset.clone()
            } else {
                commercial_asset.clone()
            },
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
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        });
    }
    allocator.dirty_index = true;
    allocator.rebuild_zone_index();

    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Residential)].len(),
        5
    );
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Commercial)].len(),
        5
    );

    allocator.buildings.swap_remove(0);
    allocator.dirty_index = true;
    allocator.rebuild_zone_index();

    assert_eq!(allocator.buildings.len(), 9);
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Residential)].len(),
        4
    );
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Commercial)].len(),
        5
    );

    let pick = allocator.get_random_building_by_zone(ZoneType::Commercial, &mut rng);
    assert!(pick.is_some());
    assert_eq!(
        allocator.buildings[pick.unwrap()].zone_type,
        ZoneType::Commercial
    );
}

#[test]
fn test_vacancy_index_consistency() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.vacancy",
        ZoneClass::Residential,
    );
    let _rng = rand::rngs::StdRng::seed_from_u64(42);

    for i in 0..5 {
        allocator.buildings.push(Building {
            center_x: i as f32,
            center_y: 0.0,
            support_height_m: 0.0,
            width_cells: 3,
            depth_cells: 3,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 0.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx: 0,
            side: 1,
            cell_x: i,
            cell_y: 0,
            occupancy: 0,
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
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        });
    }
    allocator.rebuild_zone_index();

    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        5
    );

    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        5
    );
    allocator.claim_vacancy(0);

    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        4
    );
    assert!(!allocator.vacancy_index[zone_bucket(ZoneType::Residential)].contains(&0));

    allocator.release_vacancy(0);
    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        5
    );
    assert!(allocator.vacancy_index[zone_bucket(ZoneType::Residential)].contains(&0));

    let mut agents = AgentSystem::new();
    for _ in 0..5 {
        agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
    }

    let last_idx = allocator.buildings.len() - 1;
    let i = 1;
    let mut mapping = std::collections::HashMap::new();
    mapping.insert(last_idx, i);
    agents.remap_building_indices(&mapping);

    allocator.buildings.swap_remove(i);
    allocator.rebuild_zone_index();

    assert_eq!(allocator.buildings.len(), 4);
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Residential)].len(),
        4
    );
    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        4
    );
}

#[test]
fn test_index_appended_building_matches_full_rebuild() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.incremental_index",
        ZoneClass::Residential,
    );

    allocator.buildings.push(indexed_test_building(
        residential_asset.clone(),
        ZoneType::Residential,
        0,
    ));
    allocator.rebuild_zone_index();

    let mut expected = allocator.clone();
    allocator.buildings.push(indexed_test_building(
        residential_asset.clone(),
        ZoneType::Residential,
        1,
    ));
    expected.buildings.push(indexed_test_building(
        residential_asset,
        ZoneType::Residential,
        1,
    ));

    assert!(allocator.index_appended_building(1));
    expected.rebuild_zone_index();

    assert!(!allocator.dirty_index);
    assert_eq!(allocator.zone_index, expected.zone_index);
    assert_eq!(allocator.vacancy_index, expected.vacancy_index);
    assert_eq!(allocator.vacancy_pos, expected.vacancy_pos);
    assert_eq!(allocator.building_chunks, expected.building_chunks);
    assert_eq!(
        allocator.max_lot_radius_cells,
        expected.max_lot_radius_cells
    );
}

#[test]
fn test_construction_completion_enables_capacity_and_vacancy_indexing() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "base",
        "b.res.construction",
        ZoneClass::Residential,
    );

    allocator.buildings.push(Building {
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 3,
        depth_cells: 3,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.5,
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
        asset_id: residential_asset,
        level: 1,
        construction_total_hours: 2,
        construction_remaining_hours: 2,
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
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    allocator.rebuild_zone_index();

    assert_eq!(allocator.household_capacity(0), 0);
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Residential)].len(),
        0
    );
    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        0
    );

    allocator.advance_construction_hour();
    assert_eq!(allocator.buildings[0].construction_remaining_hours, 1);
    assert_eq!(allocator.household_capacity(0), 0);

    allocator.advance_construction_hour();
    assert!(!allocator.buildings[0].is_under_construction());
    assert_eq!(allocator.buildings[0].construction_total_hours, 0);
    assert_eq!(allocator.household_capacity(0), 6);
    assert_eq!(
        allocator.zone_index[zone_bucket(ZoneType::Residential)].len(),
        1
    );
    assert_eq!(
        allocator.vacancy_index[zone_bucket(ZoneType::Residential)].len(),
        1
    );
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

#[test]
fn test_startup_demand_residential_family_selection_uses_strip_hash_order() {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let family_a_id = register_test_asset_with_family(
        &mut allocator,
        "base",
        "b.res.family_a_variant",
        ZoneClass::Residential,
        Some("family_a"),
    );
    let family_b_id = register_test_asset_with_family(
        &mut allocator,
        "base",
        "b.res.family_b_variant",
        ZoneClass::Residential,
        Some("family_b"),
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

    execute_startup_demand_building_pass(
        &mut allocator,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &graph,
    );

    let residential = allocator
        .buildings
        .iter()
        .find(|building| building.zone_type == ZoneType::Residential)
        .expect("pioneer demand should place a residential building");
    let profile_runtime_id =
        frontage_profile_runtime_id_for_building(&allocator, residential, &zoning, &graph);
    let expected_asset_id =
        if stable_strip_family_hash(profile_runtime_id, residential.parcel_id, "family_a")
            <= stable_strip_family_hash(profile_runtime_id, residential.parcel_id, "family_b")
        {
            family_a_id
        } else {
            family_b_id
        };

    assert_eq!(residential.asset_id, expected_asset_id);
}

#[test]
fn test_startup_demand_residential_variant_selection_uses_site_hash() {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let variant_a_id = register_test_asset_with_family(
        &mut allocator,
        "base",
        "b.res.shared_family_a",
        ZoneClass::Residential,
        Some("shared_family"),
    );
    let variant_b_id = register_test_asset_with_family(
        &mut allocator,
        "base",
        "b.res.shared_family_b",
        ZoneClass::Residential,
        Some("shared_family"),
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

    execute_startup_demand_building_pass(
        &mut allocator,
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &graph,
    );

    let residential = allocator
        .buildings
        .iter()
        .find(|building| building.zone_type == ZoneType::Residential)
        .expect("pioneer demand should place a residential building");
    let profile_runtime_id =
        frontage_profile_runtime_id_for_building(&allocator, residential, &zoning, &graph);
    let expected_asset_id =
        if stable_site_variant_hash(profile_runtime_id, residential.parcel_id, &variant_a_id)
            <= stable_site_variant_hash(profile_runtime_id, residential.parcel_id, &variant_b_id)
        {
            variant_a_id
        } else {
            variant_b_id
        };

    assert_eq!(residential.asset_id, expected_asset_id);
}

#[test]
fn test_incompatible_rezoning_enters_pending_redevelopment_before_removal() {
    let (
        mut allocator,
        mut zoning,
        mut agents,
        mut households,
        mut logistics,
        mut network,
        mut graph,
        residential_idx,
    ) = setup_startup_spawn_city_for_rezoning();

    let residential = allocator.buildings[residential_idx].clone();
    paint_zone_rect(
        &mut zoning,
        &graph,
        residential.center_x - 10.0,
        residential.center_y - 10.0,
        residential.center_x + 10.0,
        residential.center_y + 10.0,
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

    assert_eq!(allocator.buildings.len(), 2);
    assert!(allocator.buildings[residential_idx].pending_redevelopment);
    assert_eq!(
        allocator.buildings[residential_idx].rezone_grace_days_remaining,
        3
    );

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

    assert_eq!(allocator.buildings.len(), 1);
    assert!(
        allocator
            .buildings
            .iter()
            .all(|building| building.zone_type != ZoneType::Residential)
    );
}

#[test]
fn test_rezoning_recovery_clears_pending_redevelopment() {
    let (
        mut allocator,
        mut zoning,
        mut agents,
        mut households,
        mut logistics,
        mut network,
        mut graph,
        residential_idx,
    ) = setup_startup_spawn_city_for_rezoning();

    let residential = allocator.buildings[residential_idx].clone();
    paint_zone_rect(
        &mut zoning,
        &graph,
        residential.center_x - 10.0,
        residential.center_y - 10.0,
        residential.center_x + 10.0,
        residential.center_y + 10.0,
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

    assert!(allocator.buildings[residential_idx].pending_redevelopment);

    paint_zone_rect(
        &mut zoning,
        &graph,
        residential.center_x - 10.0,
        residential.center_y - 10.0,
        residential.center_x + 10.0,
        residential.center_y + 10.0,
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

    assert_eq!(allocator.buildings.len(), 2);
    assert!(!allocator.buildings[residential_idx].pending_redevelopment);
    assert_eq!(
        allocator.buildings[residential_idx].rezone_grace_days_remaining,
        0
    );
}

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
    assert_eq!(agents.pending_household_size[0], 2);
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
    );
    assert_eq!(agents.len(), 2);
    assert_eq!(households.households.len(), 1);
    assert_eq!(households.households[0].member_count, 2);
    assert_eq!(agents.household_id[0], agents.household_id[1]);
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
    let execution = allocator.execute_demand_building_actions(
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

    let expected_property_tax = crate::simulation::economy::fiscal::construction_property_tax(
        ZoneType::Residential,
        1,
        &demand.runtime_tuning().fiscal,
    );
    let spawned_exists = allocator
        .buildings
        .iter()
        .any(|building| building.parcel_id == spawn_parcel_id);
    assert!(
        (execution.property_tax_paid - expected_property_tax).abs() <= f32::EPSILON,
        "property tax paid {} should match expected {}; building_count={} spawn_parcel={} spawned_exists={}",
        execution.property_tax_paid,
        expected_property_tax,
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
    assert!((spawned.operating_budget + expected_property_tax).abs() <= f32::EPSILON);
}

#[test]
fn test_commercial_demand_spawn_startup_budget_includes_business_purchase_tax() {
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
    let execution = allocator.execute_demand_building_actions(
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
    let first_import_cost = first_import_base_cost
        + crate::simulation::economy::fiscal::tax_amount(
            first_import_base_cost,
            demand.runtime_tuning().fiscal.business_purchase_tax_rate,
        );
    let expected_startup_budget =
        (profile.worker_capacity as f32 * profile.average_daily_wage() * 7.0 + first_import_cost)
            .max(500.0);
    let expected_property_tax = crate::simulation::economy::fiscal::construction_property_tax(
        ZoneType::Commercial,
        1,
        &demand.runtime_tuning().fiscal,
    );

    assert!((execution.property_tax_paid - expected_property_tax).abs() <= f32::EPSILON);
    assert!(
        (building.operating_budget - (expected_startup_budget - expected_property_tax)).abs()
            <= 0.01
    );
}
