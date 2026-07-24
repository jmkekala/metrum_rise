//! Regression tests for simulation state, snapshots, demand cadence, and budget behavior.

use super::{
    CityTreasury, RenderSnapshot, RoadPreviewRequest, SimCore, absolute_operational_minute,
    demand_plan_has_non_spawn_actions, demand_plan_without_spawns, pedestrian_lane_surface_height,
    pedestrian_needs_access_surface, road_tool_snapshots_from_core,
};
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::{
    AgentSystem, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS, TRANSIT_IN_BUILDING,
    TRANSIT_NETWORK,
};
use crate::simulation::economy::demand::{
    DemandBuildingActionKey, DemandBuildingActionPlan, DemandSpawnAction, DemandSystem,
};
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::lanes::{Lane, LaneType};
use crate::simulation::network::surface::CURB_STEP_HEIGHT_M;
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::network::{TransitNetwork, graph::Edge, graph::RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use godot::prelude::Vector3;
use std::collections::HashSet;
use std::collections::{HashMap, VecDeque};

fn test_core() -> SimCore {
    let config = WorldConfig::default();
    SimCore {
        time: TimeSystem::new(),
        heightmap: TerrainSystem::from_world_config(&config),
        watermap: WaterSystem::from_world_config(&config),
        region_graph: RegionGraph::new(),
        transit_network: TransitNetwork::new_with_surface_chunk_span(config.terrain_chunk_m),
        zoning: ZoningSystem::new(&config),
        pollution: PollutionSystem::new(&config),
        noise: NoiseSystem::new(&config),
        desirability: DesirabilitySystem::new(&config),
        demand: DemandSystem::new(),
        pending_demand_spawns: VecDeque::new(),
        allocator: BuildingAllocator::new(),
        agents: AgentSystem::new(),
        households: HouseholdSystem::new(),
        logistics: ShipmentSystem::new(),
        config,
        treasury: CityTreasury::new(0.0),
        service_policy: Default::default(),
        budget_history: VecDeque::new(),
        budget_last_lifetime_build_cost: 0.0,
        debug_household_admissions_since_daily: 0,
        undo_stack: VecDeque::new(),
        world_lake_fills: Vec::new(),
        world_open_water_fills: Vec::new(),
        world_lake_fill_preview: None,
        authored_water_patch_fill_debug_cache: HashMap::new(),
        terrain_stroke_active: false,
        terrain_stroke_has_changes: false,
        terrain_dirty: false,
        water_dirty: false,
        network_dirty: false,
        benchmark_mode: true,
        last_tick_duration: 0.0,
        last_agent_tick_us: 0,
        last_road_timing: String::new(),
        last_surface_debug_edges: Vec::new(),
        refined_terrain_patch_cache: HashMap::new(),
        road_locked_terrain_patch_keys: Vec::new(),
        road_locked_terrain_patch_margins: std::collections::BTreeMap::new(),
        building_site_owned_terrain_patch_keys: HashSet::new(),
        engineered_terrain_patch_keys: Vec::new(),
        engineered_terrain_patch_margins: std::collections::BTreeMap::new(),
        terrain_payload_generation_counter: 1,
        terrain_payload_global_generation: 1,
        terrain_payload_patch_generations: HashMap::new(),
        refined_terrain_assembly_ledgers: HashMap::new(),
        cached_road_mesh_data: None,
        cached_road_mesh_generation: 0,
        cached_network_node_positions: std::sync::Arc::new(Vec::new()),
        cached_network_node_positions_dirty: true,
        road_tool_surface_generation: 1,
        camera_aabb: (0.0, 0.0, 0.0, 0.0),
    }
}

#[test]
fn test_core_keeps_road_surface_chunks_aligned_to_terrain_chunks() {
    let core = test_core();
    assert_eq!(
        core.transit_network.road_surface.chunk_span_m(),
        core.config.terrain_chunk_m
    );
}

#[test]
fn stale_terrain_acknowledgement_preserves_newer_patch_dirtiness() {
    let mut core = test_core();
    core.heightmap.mark_render_patch_dirty(0, 0);
    let stale_generation = core.terrain_payload_generation_for_patch(0, 0);
    core.bump_terrain_payload_patch_generations(&[(0, 0)]);

    assert!(!core.acknowledge_terrain_render_patch(0, 0, stale_generation));
    assert!(core.heightmap.dirty_render_patches().contains(&(0, 0)));

    let current_generation = core.terrain_payload_generation_for_patch(0, 0);
    assert!(core.acknowledge_terrain_render_patch(0, 0, current_generation));
    assert!(!core.heightmap.dirty_render_patches().contains(&(0, 0)));
}

#[test]
fn local_road_terrain_scopes_accumulate_until_exact_acknowledgement() {
    let mut core = test_core();
    let initial_generation = core.terrain_payload_generation_for_patch(0, 0);
    core.bump_local_road_terrain_payload_generations(&[(0, 0)], &[(1, 2)]);
    let first_generation = core.terrain_payload_generation_for_patch(0, 0);
    core.bump_local_road_terrain_payload_generations(&[(0, 0)], &[(3, 4)]);
    let second_generation = core.terrain_payload_generation_for_patch(0, 0);

    let ledger = core
        .refined_terrain_assembly_ledgers
        .get(&(0, 0))
        .expect("local road edits should retain assembly scope");
    assert!(first_generation > initial_generation);
    assert!(second_generation > first_generation);
    assert_eq!(
        ledger.road_query_chunk_dirty_at.get(&(1, 2)),
        Some(&first_generation)
    );
    assert_eq!(
        ledger.road_query_chunk_dirty_at.get(&(3, 4)),
        Some(&second_generation)
    );

    assert!(!core.acknowledge_terrain_render_patch(0, 0, first_generation));
    assert!(
        core.refined_terrain_assembly_ledgers.contains_key(&(0, 0)),
        "a stale acknowledgement must not discard newer local scope"
    );
    assert!(core.acknowledge_terrain_render_patch(0, 0, second_generation));
    assert!(
        !core.refined_terrain_assembly_ledgers.contains_key(&(0, 0)),
        "the exact uploaded generation may garbage-collect incorporated scope"
    );
}

#[test]
fn full_patch_invalidation_dominates_local_road_scope() {
    let mut core = test_core();
    core.bump_local_road_terrain_payload_generations(&[(0, 0)], &[(1, 2)]);
    core.bump_terrain_payload_patch_generations(&[(0, 0)]);
    let current_generation = core.terrain_payload_generation_for_patch(0, 0);
    let ledger = core
        .refined_terrain_assembly_ledgers
        .get(&(0, 0))
        .expect("full invalidation should retain a generation stamp");

    assert_eq!(ledger.full_dirty_at, Some(current_generation));
    assert_eq!(ledger.road_query_chunk_dirty_at.len(), 1);
}

#[test]
fn terrain_stroke_steps_advance_touched_patch_revisions() {
    let mut core = test_core();
    for (patch_x, patch_z, generation) in core.terrain_dirty_patch_states() {
        core.acknowledge_terrain_render_patch(patch_x, patch_z, generation);
    }

    core.start_terrain_stroke_internal();
    core.sculpt_terrain_stroke_step_internal(godot::prelude::Vector2::ZERO, 12.0, 0.1);
    let first_states = core.terrain_dirty_patch_states();
    let &(patch_x, patch_z, stale_generation) = first_states
        .first()
        .expect("terrain stroke should dirty at least one render patch");

    core.sculpt_terrain_stroke_step_internal(godot::prelude::Vector2::ZERO, 12.0, 0.1);
    let current_generation = core.terrain_payload_generation_for_patch(patch_x, patch_z);

    assert!(current_generation > stale_generation);
    assert!(!core.acknowledge_terrain_render_patch(patch_x, patch_z, stale_generation));
    assert!(
        core.heightmap
            .dirty_render_patches()
            .contains(&(patch_x, patch_z))
    );
}

#[test]
fn stale_network_acknowledgement_preserves_newer_render_revision() {
    let mut core = test_core();
    core.mark_network_render_dirty();
    let stale_generation = core.road_tool_surface_generation;
    core.mark_network_render_dirty();

    assert!(!core.acknowledge_network_render_generation(stale_generation));
    assert!(core.network_dirty);

    let current_generation = core.road_tool_surface_generation;
    assert!(core.acknowledge_network_render_generation(current_generation));
    assert!(!core.network_dirty);
}

#[test]
fn local_network_render_invalidation_preserves_unrelated_terrain_generation() {
    let mut core = test_core();
    core.cached_road_mesh_data = Some(std::sync::Arc::new(
        crate::simulation::network::NetworkMeshData::new(),
    ));
    core.cached_road_mesh_generation = core.road_tool_surface_generation;
    core.cached_network_node_positions = std::sync::Arc::new(vec![Vector3::new(12.0, 3.0, -8.0)]);
    core.cached_network_node_positions_dirty = false;
    let previous_mesh = std::sync::Arc::clone(
        core.cached_road_mesh_data
            .as_ref()
            .expect("test road mesh cache"),
    );
    let previous_node_positions = std::sync::Arc::clone(&core.cached_network_node_positions);
    let unchanged_generation = core.terrain_payload_generation_for_patch(9, 9);
    let road_query_generation = core.road_tool_surface_generation;

    core.bump_terrain_payload_patch_generations(&[(1, 2)]);
    core.mark_local_network_render_dirty();

    assert!(
        core.terrain_payload_generation_for_patch(1, 2) > unchanged_generation,
        "the road-touched patch must advance"
    );
    assert_eq!(
        core.terrain_payload_generation_for_patch(9, 9),
        unchanged_generation,
        "a local road edit must not cancel unrelated terrain payload work"
    );
    assert!(core.road_tool_surface_generation > road_query_generation);
    assert!(core.network_dirty);
    assert!(std::sync::Arc::ptr_eq(
        core.cached_road_mesh_data
            .as_ref()
            .expect("local invalidation must retain the last-good road mesh"),
        &previous_mesh
    ));
    let snapshot = core.build_snapshot();
    assert_eq!(
        snapshot.network_generation, road_query_generation,
        "a retained mesh must keep its last-successful generation token"
    );
    assert!(
        std::sync::Arc::ptr_eq(&snapshot.node_positions, &previous_node_positions),
        "a failed-generation snapshot must retain node positions matching the last-good mesh"
    );

    core.mark_network_render_dirty();
    assert!(
        core.cached_road_mesh_data.is_none(),
        "world-wide invalidation must discard a mesh from the previous world generation"
    );
}

#[test]
fn build_snapshot_reuses_cached_network_nodes_until_network_dirty() {
    let mut core = test_core();
    add_test_border_road(&mut core);
    core.mark_network_render_dirty();
    core.precompute_road_mesh_data();

    let first = core.build_snapshot();
    assert_eq!(first.node_positions.len(), 2);
    let first_nodes = std::sync::Arc::clone(&first.node_positions);

    let second = core.build_snapshot();
    assert!(std::sync::Arc::ptr_eq(&first_nodes, &second.node_positions));

    core.mark_network_render_dirty();
    core.precompute_road_mesh_data();
    let third = core.build_snapshot();
    assert_eq!(third.node_positions.len(), 2);
    assert!(!std::sync::Arc::ptr_eq(&first_nodes, &third.node_positions));
}

#[test]
fn build_snapshot_publishes_terrain_layout_before_first_tick() {
    let mut core = test_core();
    let expected_width = core.heightmap.width;
    let expected_height = core.heightmap.height;
    let expected_world_size = core.heightmap.world_size();

    let snapshot = core.build_snapshot();

    assert_eq!(snapshot.heightmap_width, expected_width);
    assert_eq!(snapshot.heightmap_height, expected_height);
    assert_eq!(
        snapshot.terrain_world_size,
        godot::prelude::Vector2::new(expected_world_size.0, expected_world_size.1)
    );
    assert!(snapshot.terrain_patch_cols > 0);
    assert!(snapshot.terrain_patch_rows > 0);
}

#[test]
fn recycled_snapshot_keeps_capacity_without_stale_instances() {
    let mut core = test_core();
    let mut recycled = RenderSnapshot::default();
    let mut transforms = Vec::with_capacity(64);
    transforms.extend_from_slice(&[1.0; 16]);
    recycled.pedestrian_transforms.insert(7, transforms);
    let original_capacity = recycled.pedestrian_transforms[&7].capacity();

    let snapshot = core.build_snapshot_reusing(recycled);

    assert!(snapshot.pedestrian_transforms[&7].is_empty());
    assert_eq!(
        snapshot.pedestrian_transforms[&7].capacity(),
        original_capacity
    );
}

#[test]
fn road_preview_rejects_a_mismatched_surface_generation() {
    let mut core = test_core();
    core.precompute_road_mesh_data();
    let (context, query) =
        road_tool_snapshots_from_core(&core).expect("the initial surface must be publishable");
    let preview = super::road_preview::compile_road_preview_from_context(
        &context,
        RoadPreviewRequest {
            request_id: 11,
            surface_generation: query.surface_generation.wrapping_add(1),
            points: vec![Vector3::new(-5.0, 0.0, 0.0), Vector3::new(5.0, 0.0, 0.0)],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        },
    );

    assert_eq!(preview.surface_generation, 0);
    assert!(!preview.is_valid);
    assert_eq!(
        preview.validation.invalid_reason,
        "stale_surface_generation"
    );
}

#[test]
fn failed_junction_precompute_retains_matching_render_and_road_tool_generation() {
    let mut core = test_core();
    let center_pos = Vector3::ZERO;
    let center = core.region_graph.add_node(center_pos, NodeType::Junction);
    let mut endpoints = Vec::new();
    let mut edge_ids = Vec::new();
    for (endpoint_pos, starts_at_center) in [
        (Vector3::new(-80.0, 0.0, 0.0), false),
        (Vector3::new(80.0, 0.0, 0.0), true),
        (Vector3::new(0.0, 0.0, -80.0), false),
        (Vector3::new(0.0, 0.0, 80.0), true),
    ] {
        let endpoint = core.region_graph.add_node(endpoint_pos, NodeType::Junction);
        let (start, end, points) = if starts_at_center {
            (center, endpoint, vec![center_pos, endpoint_pos])
        } else {
            (endpoint, center, vec![endpoint_pos, center_pos])
        };
        endpoints.push(endpoint);
        edge_ids.push(
            core.region_graph
                .add_edge(test_road_edge(start, end, points)),
        );
    }
    core.region_graph.rebuild_adjacency_list();
    core.region_graph.rebuild_intersection_clips();
    core.precompute_road_mesh_data();

    let published_mesh = std::sync::Arc::clone(
        core.cached_road_mesh_data
            .as_ref()
            .expect("the flat four-way junction must publish a baseline mesh"),
    );
    let published_generation = core.cached_road_mesh_generation;
    let published_node_positions = core.build_snapshot().node_positions;
    assert!(
        road_tool_snapshots_from_core(&core).is_some(),
        "the baseline road-tool source must be publishable"
    );

    for (&endpoint, height_m) in endpoints.iter().zip([80.0, -80.0, 64.0, -64.0]) {
        let mut pos = core.region_graph.node(endpoint).pos;
        pos.y = height_m;
        core.region_graph.set_node_pos(endpoint, pos);
    }
    for &edge_idx in &edge_ids {
        let edge = core.region_graph.edge(edge_idx);
        let points = vec![
            core.region_graph.node(edge.start_node).pos,
            core.region_graph.node(edge.end_node).pos,
        ];
        let edge = core.region_graph.edge_mut(edge_idx);
        edge.geometry = points.clone();
        edge.physical_geometry = points;
    }
    core.transit_network.solve_dirty_junction_endpoint_profiles(
        &mut core.region_graph,
        &HashSet::from([center]),
        &edge_ids.iter().copied().collect(),
    );
    core.region_graph.rebuild_intersection_clips();
    for &edge_idx in &edge_ids {
        core.transit_network
            .road_surface
            .mark_edge_dirty(&core.region_graph, edge_idx);
    }
    core.transit_network
        .road_surface
        .mark_node_dirty(&core.region_graph, center);
    core.mark_local_network_render_dirty();
    core.precompute_road_mesh_data();

    assert!(
        !core
            .transit_network
            .road_surface
            .published_generation_matches_source(),
        "the contradictory JunctionN must exercise the failed-production path"
    );
    assert!(std::sync::Arc::ptr_eq(
        core.cached_road_mesh_data
            .as_ref()
            .expect("failed precompute must retain the last-good mesh"),
        &published_mesh
    ));
    assert_eq!(core.cached_road_mesh_generation, published_generation);
    assert!(
        road_tool_snapshots_from_core(&core).is_none(),
        "a current graph with a stale latched surface must not become a road-tool snapshot"
    );
    let failed_snapshot = core.build_snapshot();
    assert_eq!(failed_snapshot.network_generation, published_generation);
    assert!(std::sync::Arc::ptr_eq(
        failed_snapshot
            .road_mesh_data
            .as_ref()
            .expect("failed render snapshot must retain the last-good mesh"),
        &published_mesh
    ));
    assert!(std::sync::Arc::ptr_eq(
        &failed_snapshot.node_positions,
        &published_node_positions
    ));
}

#[test]
fn bulk_road_finalizer_solves_profiles_before_surface_compile() {
    let mut core = test_core();
    let center_pos = Vector3::new(0.0, 10.0, 0.0);
    let west_pos = Vector3::new(-48.0, 10.0, 0.0);
    let north_pos = Vector3::new(0.0, 22.0, 48.0);
    let center = core.region_graph.add_node(center_pos, NodeType::Junction);
    let west = core.region_graph.add_node(west_pos, NodeType::Junction);
    let north = core.region_graph.add_node(north_pos, NodeType::Junction);

    let stable_edge =
        core.region_graph
            .add_edge(test_road_edge(west, center, vec![west_pos, center_pos]));
    let new_edge =
        core.region_graph
            .add_edge(test_road_edge(center, north, vec![center_pos, north_pos]));

    core.transit_network.bulk_dirty_edges.insert(new_edge);
    core.transit_network
        .road_surface
        .mark_edge_dirty(&core.region_graph, new_edge);
    core.transit_network
        .road_surface
        .mark_node_dirty(&core.region_graph, center);
    core.transit_network
        .road_surface
        .mark_node_dirty(&core.region_graph, north);

    let stable_before = core.region_graph.edge(stable_edge).geometry.clone();
    let finalized = core.finalize_bulk_road_geometry_for_dirty_edges();

    assert!(finalized.affected_nodes.contains(&center));
    assert!(finalized.dirty_edges.contains(&new_edge));
    assert!(core.transit_network.bulk_dirty_edges.is_empty());
    assert_eq!(
        core.region_graph.edge(stable_edge).geometry,
        stable_before,
        "stable bend authority edge must not be rewritten by bulk finalization"
    );
    assert!(
        core.region_graph.edge(new_edge).geometry.len() >= 8,
        "bulk finalization must adapt the new junction mouth before surface compilation"
    );
    assert!(
        core.transit_network
            .road_surface
            .dirty_edges()
            .contains(&new_edge),
        "changed profile edge must stay marked for the following road-surface compile"
    );
}

fn add_test_border_road(core: &mut SimCore) {
    let border = core
        .region_graph
        .add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Border);
    let junction = core
        .region_graph
        .add_node(Vector3::new(180.0, 0.0, 0.0), NodeType::Junction);
    core.region_graph.add_edge(Edge {
        start_node: border,
        end_node: junction,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 13.89,
        base_cost: 180.0,
        physical_length: 180.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(180.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(180.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
}

fn test_road_edge(start_node: u32, end_node: u32, geometry: Vec<Vector3>) -> Edge {
    let physical_length = geometry
        .windows(2)
        .map(|points| points[0].distance_to(points[1]))
        .sum();
    Edge {
        start_node,
        end_node,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 13.89,
        base_cost: physical_length,
        physical_length,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        physical_geometry: geometry.clone(),
        geometry,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    }
}

fn register_test_asset(
    allocator: &mut BuildingAllocator,
    asset_id: &str,
    zone_type: ZoneType,
) -> String {
    let (zone_class, household_capacity, worker_capacity, economy_profile) = match zone_type {
        ZoneType::Residential => (ZoneClass::Residential, Some(6), None, None),
        ZoneType::Commercial => (
            ZoneClass::Commercial,
            None,
            Some(4),
            Some("grocery_basic".to_owned()),
        ),
        ZoneType::Industrial => (
            ZoneClass::Industrial,
            None,
            Some(4),
            Some("food_processor_basic".to_owned()),
        ),
        _ => panic!("test asset requires a baseline private-use zone"),
    };
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Test".to_owned(),
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
            flat_size_m2: household_capacity.map(|_| 80.0),
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(zone_class),
            density: Some("low".to_owned()),
            lot_width_cells: 2,
            lot_depth_cells: 2,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity,
            worker_capacity,
            service_class: None,
            economy_profile,
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator.registry.register("test", manifest, String::new());
    format!("test:{asset_id}")
}

fn place_test_parcel_run(core: &mut SimCore, zone_type: ZoneType, start_x: f32, end_x: f32) {
    let profile = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(zone_type)
        .expect("test zoning profile");
    core.zoning
        .place_parcel_run_at(
            start_x,
            -20.0,
            end_x,
            -20.0,
            profile,
            20.0,
            30.0,
            0.0,
            &core.region_graph,
        )
        .expect("test parcel run");
}

#[test]
fn absolute_operational_minute_is_day_stable() {
    assert_eq!(absolute_operational_minute(1, 0), 0);
    assert_eq!(absolute_operational_minute(1, 1439), 1439);
    assert_eq!(absolute_operational_minute(2, 0), 1440);
    assert_eq!(absolute_operational_minute(3, 60), 2940);
}

#[test]
fn immediate_demand_plan_strips_spawns_only() {
    let mut plan = DemandBuildingActionPlan::default();
    plan.residential.spawns.push(DemandSpawnAction {
        parcel_id: 7,
        asset_id: "building.residential.test".to_owned(),
    });
    plan.residential.despawns.push(DemandBuildingActionKey {
        parcel_id: 8,
        edge_idx: 1,
        side: 1,
        cell_x: 2,
        width_cells: 3,
        depth_cells: 4,
        level: 1,
        asset_id: "building.residential.old".to_owned(),
    });

    let immediate = demand_plan_without_spawns(&plan);

    assert!(immediate.residential.spawns.is_empty());
    assert_eq!(immediate.residential.despawns.len(), 1);
    assert!(demand_plan_has_non_spawn_actions(&immediate));
}

#[test]
fn max_demand_cheat_runtime_queues_and_executes_rci_spawns() {
    let mut core = test_core();
    add_test_border_road(&mut core);
    register_test_asset(&mut core.allocator, "residential", ZoneType::Residential);
    register_test_asset(&mut core.allocator, "commercial", ZoneType::Commercial);
    register_test_asset(&mut core.allocator, "industrial", ZoneType::Industrial);
    place_test_parcel_run(&mut core, ZoneType::Residential, 10.0, 50.0);
    place_test_parcel_run(&mut core, ZoneType::Commercial, 60.0, 100.0);
    place_test_parcel_run(&mut core, ZoneType::Industrial, 110.0, 150.0);

    core.apply_money_and_max_demand_cheat(1_000_000.0);
    core.execute_hourly_demand_pass(1, 0, &[]);

    assert!(
        core.pending_demand_spawns
            .iter()
            .any(|pending| pending.zone_type == ZoneType::Residential)
    );
    assert!(
        core.pending_demand_spawns
            .iter()
            .any(|pending| pending.zone_type == ZoneType::Commercial)
    );
    assert!(
        core.pending_demand_spawns
            .iter()
            .any(|pending| pending.zone_type == ZoneType::Industrial)
    );

    let queued_spawn_count = core.pending_demand_spawns.len();
    let mut executed_spawn_count = 0_usize;
    for minute_offset in 1..=queued_spawn_count {
        executed_spawn_count +=
            core.execute_pending_demand_spawns_for_minute(1, minute_offset as u16);
    }

    assert_eq!(executed_spawn_count, queued_spawn_count);
    assert!(core.pending_demand_spawns.is_empty());
    assert!(
        core.allocator
            .buildings
            .iter()
            .any(|building| building.zone_type == ZoneType::Residential)
    );
    assert!(
        core.allocator
            .buildings
            .iter()
            .any(|building| building.zone_type == ZoneType::Commercial)
    );
    assert!(
        core.allocator
            .buildings
            .iter()
            .any(|building| building.zone_type == ZoneType::Industrial)
    );
}

#[test]
fn pedestrian_lane_surface_height_matches_lane_semantics() {
    let sidewalk = Lane {
        edge_id: 7,
        lane_idx: 100,
        lane_type: LaneType::Foot,
        ..Lane::default()
    };
    assert_eq!(
        pedestrian_lane_surface_height(&sidewalk, 4.0),
        4.0 + CURB_STEP_HEIGHT_M
    );

    let crosswalk = Lane {
        edge_id: usize::MAX,
        crosswalk_edge_id: Some(7),
        lane_type: LaneType::Foot,
        ..Lane::default()
    };
    assert_eq!(pedestrian_lane_surface_height(&crosswalk, 4.0), 4.0);

    let footpath = Lane {
        edge_id: 7,
        lane_idx: 0,
        lane_type: LaneType::Foot,
        ..Lane::default()
    };
    assert_eq!(pedestrian_lane_surface_height(&footpath, 4.0), 4.0);
}

#[test]
fn pedestrian_access_surface_is_limited_to_door_transitions() {
    assert!(pedestrian_needs_access_surface(TRANSIT_ACCESS_EGRESS));
    assert!(pedestrian_needs_access_surface(TRANSIT_ACCESS_INGRESS));
    assert!(!pedestrian_needs_access_surface(TRANSIT_NETWORK));
    assert!(!pedestrian_needs_access_surface(TRANSIT_IN_BUILDING));
}
