//! Building allocator unit tests.

use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, LodEntry, PlacementMode, ZoneClass};
use crate::simulation::core::config::MapConfig;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::types::VehicleFrontageAccess;
use godot::prelude::Vector2;
use rand::SeedableRng;

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
    let (residents_capacity, worker_capacity) = match zone {
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
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(zone),
            density: Some("low".to_owned()),
            lot_width_cells: 1,
            lot_depth_cells: 1,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            residents_capacity,
            worker_capacity,
            service_class: None,
            economy_profile: None,
            preview_scale: Some(1.0),
        }),
        prop: None,
        vehicle: None,
        character: None,
        pivot_offset: None,
    };
    allocator
        .registry
        .register(pack_id, manifest, String::new());
    format!("{pack_id}:{asset_id}")
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

fn stable_strip_family_hash(
    profile_runtime_id: u16,
    edge_idx: usize,
    side: i8,
    family_key: &str,
) -> u64 {
    stable_hash_bytes(&[
        &profile_runtime_id.to_le_bytes(),
        &(edge_idx as u64).to_le_bytes(),
        &[side as u8],
        family_key.as_bytes(),
    ])
}

fn stable_site_variant_hash(
    profile_runtime_id: u16,
    edge_idx: usize,
    side: i8,
    cell_x: usize,
    qualified_asset_id: &str,
) -> u64 {
    stable_hash_bytes(&[
        &profile_runtime_id.to_le_bytes(),
        &(edge_idx as u64).to_le_bytes(),
        &[side as u8],
        &(cell_x as u64).to_le_bytes(),
        qualified_asset_id.as_bytes(),
    ])
}

fn frontage_profile_runtime_id_for_building(
    allocator: &BuildingAllocator,
    building: &Building,
    zoning: &crate::simulation::grid::zoning::ZoningSystem,
    graph: &RegionGraph,
) -> u16 {
    let zone_cell_m = zoning.config.zone_cell_m;
    let edge = graph.edge(building.edge_idx);
    let t_col = (building.cell_x as f32 + 0.5) * zone_cell_m / edge.physical_length;
    let frontage_pos = allocator.get_pos_on_edge(graph, building.edge_idx, t_col);
    let frontage_tangent = allocator.get_tangent_on_edge(graph, building.edge_idx, t_col);
    let frontage_normal =
        Vector2::new(frontage_tangent.y, -frontage_tangent.x) * building.side as f32;
    let curb_dist = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH;
    let frontage_center = frontage_pos + frontage_normal * (curb_dist + zone_cell_m * 0.5);
    zoning.get_zone_profile_runtime_id_world(frontage_center.x, frontage_center.y)
}

fn setup_bootstrap_city_for_rezoning() -> (
    BuildingAllocator,
    crate::simulation::grid::zoning::ZoningSystem,
    AgentSystem,
    HouseholdSystem,
    ShipmentSystem,
    crate::simulation::network::TransitNetwork,
    RegionGraph,
    usize,
) {
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::types::NodeType;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    register_test_asset(&mut allocator, "base", "b.com.shop", ZoneClass::Commercial);

    let map_cfg = MapConfig::default();
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
    zoning.set_zone_rect(-50.0, -50.0, 45.0, 50.0, ZoneType::Residential);
    zoning.set_zone_rect(55.0, -50.0, 150.0, 50.0, ZoneType::Commercial);

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    let residential_idx = allocator
        .buildings
        .iter()
        .position(|building| building.zone_type == ZoneType::Residential)
        .expect("founding bootstrap should create one residential building");

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

    for i in 0..10 {
        allocator.buildings.push(Building {
            center_x: i as f32,
            center_y: 0.0,
            width_cells: 3,
            depth_cells: 3,
            zone_type: if i % 2 == 0 {
                ZoneType::Residential
            } else {
                ZoneType::Commercial
            },
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 0.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: i,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id: String::new(),
            level: 1,
            broken: false,
            stock: 0.0,
            revenue: 0.0,
            operating_budget: 500.0,
            utility_service_available: false,
            shipment_cooldown_days: 0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        });
    }
    allocator.dirty_index = true;
    allocator.rebuild_zone_index();

    assert_eq!(
        allocator.zone_index[ZoneType::Residential as usize].len(),
        5
    );
    assert_eq!(allocator.zone_index[ZoneType::Commercial as usize].len(), 5);

    allocator.buildings.swap_remove(0);
    allocator.dirty_index = true;
    allocator.rebuild_zone_index();

    assert_eq!(allocator.buildings.len(), 9);
    assert_eq!(
        allocator.zone_index[ZoneType::Residential as usize].len(),
        4
    );
    assert_eq!(allocator.zone_index[ZoneType::Commercial as usize].len(), 5);

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
            width_cells: 3,
            depth_cells: 3,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 0.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: i,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id: residential_asset.clone(),
            level: 1,
            broken: false,
            stock: 0.0,
            revenue: 0.0,
            operating_budget: 500.0,
            utility_service_available: false,
            shipment_cooldown_days: 0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        });
    }
    allocator.rebuild_zone_index();

    assert_eq!(
        allocator.vacancy_index[ZoneType::Residential as usize].len(),
        5
    );

    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    allocator.claim_vacancy(0);
    assert_eq!(
        allocator.vacancy_index[ZoneType::Residential as usize].len(),
        5
    );
    allocator.claim_vacancy(0);

    assert_eq!(
        allocator.vacancy_index[ZoneType::Residential as usize].len(),
        4
    );
    assert!(!allocator.vacancy_index[ZoneType::Residential as usize].contains(&0));

    allocator.release_vacancy(0);
    assert_eq!(
        allocator.vacancy_index[ZoneType::Residential as usize].len(),
        5
    );
    assert!(allocator.vacancy_index[ZoneType::Residential as usize].contains(&0));

    let mut agents = AgentSystem::new();
    for _ in 0..5 {
        agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
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
        allocator.zone_index[ZoneType::Residential as usize].len(),
        4
    );
    assert_eq!(
        allocator.vacancy_index[ZoneType::Residential as usize].len(),
        4
    );
}

#[test]
fn test_tick_does_not_auto_spawn_private_buildings_from_zones() {
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );

    let map_cfg = MapConfig::default();
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
    zoning.set_zone_rect(-50.0, -50.0, 150.0, 50.0, ZoneType::Residential);

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
fn test_tick_runs_one_time_founding_bootstrap_from_border_and_zoning() {
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    register_test_asset(&mut allocator, "base", "b.com.shop", ZoneClass::Commercial);
    let map_cfg = MapConfig::default();
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
    zoning.set_zone_rect(-50.0, -50.0, 45.0, 50.0, ZoneType::Residential);
    zoning.set_zone_rect(55.0, -50.0, 150.0, 50.0, ZoneType::Commercial);

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    assert!(allocator.founding_bootstrap_consumed);
    assert_eq!(allocator.buildings.len(), 2);
    assert_eq!(
        allocator
            .buildings
            .iter()
            .filter(|building| building.zone_type == ZoneType::Residential)
            .count(),
        1
    );
    assert_eq!(
        allocator
            .buildings
            .iter()
            .filter(|building| building.zone_type == ZoneType::Commercial)
            .count(),
        1
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
        2,
        "Founding bootstrap should only seed one residential and one commercial building"
    );
}

#[test]
fn test_founding_bootstrap_residential_family_selection_uses_strip_hash_order() {
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
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

    let map_cfg = MapConfig::default();
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
    zoning.set_zone_rect(-50.0, -50.0, 45.0, 50.0, ZoneType::Residential);
    zoning.set_zone_rect(55.0, -50.0, 150.0, 50.0, ZoneType::Commercial);

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    let residential = allocator
        .buildings
        .iter()
        .find(|building| building.zone_type == ZoneType::Residential)
        .expect("bootstrap should place a residential building");
    let profile_runtime_id =
        frontage_profile_runtime_id_for_building(&allocator, residential, &zoning, &graph);
    let expected_asset_id = if stable_strip_family_hash(
        profile_runtime_id,
        residential.edge_idx,
        residential.side,
        "family_a",
    ) <= stable_strip_family_hash(
        profile_runtime_id,
        residential.edge_idx,
        residential.side,
        "family_b",
    ) {
        family_a_id
    } else {
        family_b_id
    };

    assert_eq!(residential.asset_id, expected_asset_id);
}

#[test]
fn test_founding_bootstrap_residential_variant_selection_uses_site_hash() {
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
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

    let map_cfg = MapConfig::default();
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
    zoning.set_zone_rect(-50.0, -50.0, 45.0, 50.0, ZoneType::Residential);
    zoning.set_zone_rect(55.0, -50.0, 150.0, 50.0, ZoneType::Commercial);

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );

    let residential = allocator
        .buildings
        .iter()
        .find(|building| building.zone_type == ZoneType::Residential)
        .expect("bootstrap should place a residential building");
    let profile_runtime_id =
        frontage_profile_runtime_id_for_building(&allocator, residential, &zoning, &graph);
    let expected_asset_id = if stable_site_variant_hash(
        profile_runtime_id,
        residential.edge_idx,
        residential.side,
        residential.cell_x,
        &variant_a_id,
    ) <= stable_site_variant_hash(
        profile_runtime_id,
        residential.edge_idx,
        residential.side,
        residential.cell_x,
        &variant_b_id,
    ) {
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
    ) = setup_bootstrap_city_for_rezoning();

    let residential = allocator.buildings[residential_idx].clone();
    zoning.set_zone_rect(
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
    ) = setup_bootstrap_city_for_rezoning();

    let residential = allocator.buildings[residential_idx].clone();
    zoning.set_zone_rect(
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

    zoning.set_zone_rect(
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
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "base",
        "b.res.entrance_cache",
        ZoneClass::Residential,
    );

    let map_cfg = MapConfig::default();
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
        width_cells: 1,
        depth_cells: 1,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, -1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: 0,
        side: 1,
        cell_x: 1,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        asset_id,
        level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 0.0,
        utility_service_available: false,
        shipment_cooldown_days: 0,
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
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let manifest = AssetManifest {
        asset_id: "b.res.anchor_units".to_owned(),
        display_name: "Anchor Units".to_owned(),
        asset_set: None,
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
            position: [1.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
        }],
        building: Some(BuildingData {
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(ZoneClass::Residential),
            density: Some("low".to_owned()),
            lot_width_cells: 1,
            lot_depth_cells: 1,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            residents_capacity: Some(6),
            worker_capacity: None,
            service_class: None,
            economy_profile: None,
            preview_scale: Some(7.18),
        }),
        prop: None,
        vehicle: None,
        character: None,
        pivot_offset: Some([4.0, 0.0, -3.0]),
    };
    allocator.registry.register("base", manifest, String::new());

    let map_cfg = MapConfig::default();
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
        width_cells: 1,
        depth_cells: 1,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, -1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: 0,
        side: 1,
        cell_x: 1,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        asset_id: "base:b.res.anchor_units".to_owned(),
        level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 0.0,
        utility_service_available: false,
        shipment_cooldown_days: 0,
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
}

#[test]
fn test_building_removal_clears_zoning_occupancy() {
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use godot::prelude::Vector3;

    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    let map_cfg = MapConfig::default();
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

    allocator.buildings.push(Building {
        center_x: 5.0,
        center_y: 10.0,
        width_cells: 3,
        depth_cells: 3,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.05,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        asset_id,
        level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 500.0,
        utility_service_available: false,
        shipment_cooldown_days: 0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    zoning.mark_occupied_rect(
        5.0,
        10.0,
        godot::prelude::Vector2::new(0.0, 1.0),
        30.0,
        30.0,
        true,
    );

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
        !zoning.is_rect_occupied(5.0, 10.0, godot::prelude::Vector2::new(0.0, 1.0), 5.0, 5.0),
        "Zoning occupancy should be cleared after building removal"
    );
}

#[test]
fn test_immigration_claims_vacant_home() {
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use godot::prelude::{Vector2, Vector3};

    let mut allocator = BuildingAllocator::new();
    let residential_asset_id = register_test_asset(
        &mut allocator,
        "base",
        "b.res.house",
        ZoneClass::Residential,
    );
    let map_cfg = MapConfig::default();
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
    zoning.set_zone_rect(-50.0, -50.0, 150.0, 50.0, ZoneType::Residential);
    allocator.buildings.push(Building {
        center_x: 10.0,
        center_y: 10.0,
        width_cells: 3,
        depth_cells: 3,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.1,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: edge_id,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        asset_id: residential_asset_id,
        level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 500.0,
        utility_service_available: false,
        shipment_cooldown_days: 0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    allocator.rebuild_zone_index();

    allocator.tick(
        &mut zoning,
        &mut agents,
        &mut households,
        &mut logistics,
        &mut network,
        &mut graph,
    );
    allocator.execute_demand_household_admission(1, &mut agents, &mut households);

    assert_eq!(
        agents.len(),
        2,
        "One two-resident household should have been admitted from the demand-owned output"
    );
    assert_eq!(
        agents.home_building[0], 0,
        "Immigrant should have claimed home index 0"
    );
    assert_eq!(
        agents.target_building[0],
        usize::MAX,
        "Directly admitted immigrants should not carry a bootstrap target_building trip"
    );
    assert_eq!(
        agents.transit[0],
        crate::simulation::economy::agents::TRANSIT_IN_BUILDING,
        "Immigrant household members should now spawn directly inside their claimed home"
    );
    assert_eq!(agents.current_building[0], 0);
    assert_eq!(agents.current_node[0], u32::MAX);
    assert_eq!(agents.current_lane_id[0], usize::MAX);
    assert_eq!(agents.access_flags[0], 0);
    let expected_door = allocator.entrances[0].door_pos;
    assert!((agents.pos_x[0] - expected_door.x).abs() < 1e-4);
    assert!((agents.pos_y[0] - expected_door.y).abs() < 1e-4);
    assert_eq!(agents.household_id[0], agents.household_id[1]);
    assert_eq!(households.households.len(), 1);
    assert_eq!(households.households[0].member_count, 2);
    assert_eq!(
        allocator.buildings[0].occupancy, 2,
        "Building occupancy should match the admitted household size"
    );
}

#[test]
fn test_startup_immigration_floor_avoids_zero_rounding() {
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

    let mut zoning = crate::simulation::grid::zoning::ZoningSystem::new(&MapConfig::default());
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

    allocator.buildings.push(Building {
        center_x: 10.0,
        center_y: 10.0,
        width_cells: 2,
        depth_cells: 2,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.1,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: edge_id,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 2,
        worker_count: 0,
        asset_id: residential_asset,
        level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 500.0,
        utility_service_available: true,
        shipment_cooldown_days: 0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    allocator.buildings.push(Building {
        center_x: 40.0,
        center_y: 10.0,
        width_cells: 2,
        depth_cells: 2,
        zone_type: ZoneType::Commercial,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.4,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: edge_id,
        side: 1,
        cell_x: 4,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        asset_id: commercial_asset,
        level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 500.0,
        utility_service_available: true,
        shipment_cooldown_days: 0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let household_id = households.admit_immigrant_household(0, 2);
    for _ in 0..2 {
        let idx = agents.spawn_agent(0, 1, 0.0, 0.0, 0, 0.0, 0.0);
        agents.household_id[idx] = household_id;
    }
    households.households[household_id].budget = 120.0;
    households.households[household_id].stock = 6.0;
    households.households[household_id].stock_days = 3.0;

    let mut demand = DemandSystem::new();
    demand.run_daily_pass(&allocator, &households, &graph);
    assert!(
        demand.households_to_admit_today > 0,
        "startup support should produce a demand-owned household-admission output"
    );
    allocator.execute_demand_household_admission(
        demand.households_to_admit_today,
        &mut agents,
        &mut households,
    );

    assert!(
        households.households.len() >= 2,
        "player-seeded startup city should admit another household through the demand-owned startup output"
    );
}
