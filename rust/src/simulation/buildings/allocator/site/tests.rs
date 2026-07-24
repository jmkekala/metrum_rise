//! Building-site derivation, grading, and query regression tests.

use super::derive::{
    BUILDING_SITE_ROAD_ACCESS_CLEARANCE_M, frontage_projection, frontage_projection_limit,
    required_flat_support_footprint_local,
};
use super::geometry::{signed_polygon_area, site_radius_m};
use super::grading::{
    BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M, BuildingSiteGradingRequest,
    SiteGradingContext, SiteGradingGuideSink, append_building_site_grading_guides,
    building_site_grading_target_height, building_site_road_connection_lateral_offset_m,
    nearest_building_site_road_surface_sample,
};
use super::model::{
    BuildingSiteSurfaceClient, BuildingSiteTerrainClient, BuildingSiteTerrainSnapshot,
};
use super::{BuildingSiteClient, building_site_support_tie_in_is_valid};
use crate::assets::{Anchor, AnchorType, AssetManifest, MeshPart, SiteSurfaceMaterial};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::network::types::{TransitFlags, TransitType};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtInput, TerrainCdtPatch, TerrainCdtVertex, build_road_touched_terrain_patch,
};
use godot::prelude::{Vector2, Vector3};
use std::collections::BTreeMap;

#[test]
fn site_radius_is_measured_from_the_indexed_lot_center() {
    let site = BuildingSiteClient {
        footprint_world: vec![
            Vector2::new(18.0, -1.0),
            Vector2::new(20.0, -1.0),
            Vector2::new(20.0, 1.0),
            Vector2::new(18.0, 1.0),
        ],
        lot_footprint_world: [
            Vector2::new(-2.0, -2.0),
            Vector2::new(2.0, -2.0),
            Vector2::new(2.0, 2.0),
            Vector2::new(-2.0, 2.0),
        ],
        support_height_m: 0.0,
        surfaces: Vec::new(),
    };

    assert!(site_radius_m(&site) >= 20.0);
}

fn road_test_edge(
    start_node: u32,
    end_node: u32,
    points: Vec<Vector3>,
    width: f32,
    class: crate::simulation::network::types::EdgeClass,
) -> crate::simulation::network::graph::Edge {
    let length = points
        .windows(2)
        .map(|segment| segment[0].distance_to(segment[1]))
        .sum();
    crate::simulation::network::graph::Edge {
        start_node,
        end_node,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class,
        width,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: length,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: points.clone(),
        physical_geometry: points,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    }
}

fn square_site_with_surface() -> BuildingSiteClient {
    BuildingSiteClient {
        footprint_world: vec![
            Vector2::new(-5.0, -5.0),
            Vector2::new(-5.0, 5.0),
            Vector2::new(5.0, 5.0),
            Vector2::new(5.0, -5.0),
        ],
        lot_footprint_world: [
            Vector2::new(-5.0, -5.0),
            Vector2::new(-5.0, 5.0),
            Vector2::new(5.0, 5.0),
            Vector2::new(5.0, -5.0),
        ],
        support_height_m: 2.0,
        surfaces: vec![BuildingSiteSurfaceClient {
            material: SiteSurfaceMaterial::Asphalt,
            name: "asphalt".to_owned(),
            height_m: 2.4,
            vertices_world: vec![
                Vector2::new(-1.0, -1.0),
                Vector2::new(-1.0, 1.0),
                Vector2::new(1.0, 1.0),
                Vector2::new(1.0, -1.0),
            ],
        }],
    }
}

fn flat_site_from_bounds(
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
    support_height_m: f32,
) -> BuildingSiteClient {
    let footprint_world = vec![
        Vector2::new(min_x, min_z),
        Vector2::new(min_x, max_z),
        Vector2::new(max_x, max_z),
        Vector2::new(max_x, min_z),
    ];
    BuildingSiteClient {
        footprint_world: footprint_world.clone(),
        lot_footprint_world: [
            Vector2::new(min_x, min_z),
            Vector2::new(min_x, max_z),
            Vector2::new(max_x, max_z),
            Vector2::new(max_x, min_z),
        ],
        support_height_m,
        surfaces: Vec::new(),
    }
}

#[test]
fn site_grading_target_uses_visible_road_surface() {
    let terrain = TerrainSystem::with_chunking(65, 65, 1.0, 8, 1.0);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(
        Vector3::new(0.0, 0.0, -16.0),
        crate::simulation::network::types::NodeType::Junction,
    );
    let end = graph.add_node(
        Vector3::new(0.0, 0.0, 16.0),
        crate::simulation::network::types::NodeType::Junction,
    );
    graph.add_edge(road_test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 16.0)],
        10.0,
        crate::simulation::network::types::EdgeClass::Bridge,
    ));
    let mut road_surface = RoadSurfaceSystem::new(16.0);
    road_surface.compile_dirty(&graph, &terrain);

    let pos = Vector2::ZERO;
    let expected_height_m = road_surface
        .sample_visible_surface_height(&graph, &terrain, pos.x, pos.y)
        .expect("bridge surface should own the grading sample");
    let graded_height_m =
        building_site_grading_target_height(10.0, pos, 100.0, &terrain, &graph, &road_surface);

    assert!(
        (graded_height_m - expected_height_m).abs() <= 0.001,
        "site grading must use the visible road surface: graded={graded_height_m:.3} expected={expected_height_m:.3}"
    );
}

#[test]
fn site_height_prefers_authored_surface_offset() {
    let site = square_site_with_surface();

    assert_eq!(site.height_at(Vector2::new(0.0, 0.0)), Some(2.4));
    assert_eq!(site.height_at(Vector2::new(4.0, 4.0)), Some(2.0));
}

#[test]
fn site_height_includes_surface_and_footprint_boundaries() {
    let site = square_site_with_surface();

    assert_eq!(site.height_at(Vector2::new(1.0, 0.0)), Some(2.4));
    assert_eq!(site.height_at(Vector2::new(5.0, 0.0)), Some(2.0));
}

#[test]
fn site_raycast_hits_authored_surface_before_support_plane() {
    let site = square_site_with_surface();

    let hit = site
        .raycast(Vector3::new(0.0, 10.0, 0.0), Vector3::DOWN)
        .expect("ray should hit site surface");

    assert!((hit.y - 2.4).abs() <= f32::EPSILON);
}

#[test]
fn site_grading_guides_are_soft_samples_outside_flat_support() {
    let site = BuildingSiteClient {
        support_height_m: 4.0,
        surfaces: Vec::new(),
        ..square_site_with_surface()
    };
    let terrain = TerrainSystem::with_chunking(8, 8, 1.0, 4, 0.0);
    let graph = RegionGraph::new();
    let road_surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);
    let mut samples = Vec::new();
    let mut sample_keys = BTreeMap::new();

    let context = SiteGradingContext::new(&terrain, &graph, &road_surface, 2.0, 16.0);
    let mut sink = SiteGradingGuideSink::new(&mut samples, &mut sample_keys);
    append_building_site_grading_guides(&site, &context, &mut sink);

    assert!(
        samples.iter().any(|sample| {
            (sample.vertex.x + 6.0).abs() <= 0.001
                && sample.vertex.z.abs() <= 1.001
                && (sample.vertex.height_m - 3.5).abs() <= 0.001
        }),
        "first apron ring should sit outside the footprint and respect the tie-in slope budget"
    );
    assert!(samples.iter().all(|sample| {
        !site.contains_point(Vector2::new(sample.vertex.x as f32, sample.vertex.z as f32))
    }));
}

#[test]
fn site_grading_apron_reaches_a_tile_whose_core_misses_the_footprint() {
    let terrain = TerrainSystem::with_chunking(257, 65, 1.0, 64, 0.0);
    let graph = RegionGraph::new();
    let road_surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);
    let snapshot = BuildingSiteTerrainSnapshot {
        sites: vec![BuildingSiteTerrainClient {
            building_idx: 0,
            footprint_world: vec![
                Vector2::new(58.0, 10.0),
                Vector2::new(62.0, 10.0),
                Vector2::new(62.0, 14.0),
                Vector2::new(58.0, 14.0),
            ],
            support_height_m: 4.0,
        }],
    };
    let mut samples = Vec::new();
    let mut sample_keys = BTreeMap::new();

    snapshot.append_terrain_cdt_site_grading_guides_for_world_bounds(
        BuildingSiteGradingRequest::new(
            &terrain,
            &graph,
            &road_surface,
            (64.0, 0.0, 128.0, 64.0),
            2.0,
        ),
        &mut samples,
        &mut sample_keys,
    );

    assert!(
        samples.iter().any(|sample| {
            (sample.vertex.x - 64.0).abs() <= 0.001
                && (sample.vertex.z - 12.0).abs() <= 0.001
                && (sample.vertex.height_m - 3.0).abs() <= 0.001
        }),
        "the apron must enter the right tile even though the site footprint ends at x=62"
    );
}

#[test]
fn terrain_site_snapshot_preserves_stable_cdt_ownership() {
    let mut allocator = BuildingAllocator::new();
    allocator.building_sites.push(square_site_with_surface());
    let direct = allocator.terrain_cdt_site_loops_for_world_bounds(-8.0, -8.0, 8.0, 8.0);
    let snapshot = allocator.terrain_site_snapshot_for_world_bounds(-8.0, -8.0, 8.0, 8.0);
    let detached = snapshot.terrain_cdt_site_loops_for_world_bounds(-8.0, -8.0, 8.0, 8.0);

    assert_eq!(detached, direct);
}

#[test]
fn adjacent_different_height_site_loops_do_not_conflict_in_cdt() {
    let mut allocator = BuildingAllocator::new();
    allocator
        .building_sites
        .push(flat_site_from_bounds(-5.0, -5.0, 0.0, 5.0, 0.0));
    allocator
        .building_sites
        .push(flat_site_from_bounds(0.0, -5.0, 5.0, 5.0, 1.0));
    let loops = allocator.terrain_cdt_site_loops_for_world_bounds(-8.0, -8.0, 8.0, 8.0);
    let source_samples = vec![
        TerrainCdtVertex::new(-8.0, 0.0, -8.0),
        TerrainCdtVertex::new(-8.0, 0.0, 8.0),
        TerrainCdtVertex::new(8.0, 0.0, 8.0),
        TerrainCdtVertex::new(8.0, 0.0, -8.0),
    ];

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        TerrainCdtPatch::new(-8.0, -8.0, 8.0, 8.0, [0.0; 4]),
        loops,
        source_samples,
    ))
    .expect("adjacent yards at different road-derived heights must not emit duplicate X/Z CDT boundary vertices");

    assert_eq!(
        mesh.stats.building_site_constraint_edges, mesh.stats.road_constraint_edges,
        "site CDT ownership loops must be tracked separately from hard road seams"
    );
}

#[test]
fn support_tie_in_accepts_flat_surroundings() {
    let terrain = TerrainSystem::with_chunking(32, 32, 1.0, 8, 0.0);
    let graph = RegionGraph::new();
    let road_surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);

    assert!(building_site_support_tie_in_is_valid(
        &square_site_with_surface().footprint_world,
        0.0,
        &terrain,
        &graph,
        &road_surface,
    ));
}

#[test]
fn support_tie_in_rejects_oversteep_surroundings() {
    let terrain = TerrainSystem::with_chunking(32, 32, 1.0, 8, 0.0);
    let graph = RegionGraph::new();
    let road_surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);

    assert!(!building_site_support_tie_in_is_valid(
        &square_site_with_surface().footprint_world,
        5.0,
        &terrain,
        &graph,
        &road_surface,
    ));
}

#[test]
fn derived_site_client_uses_required_flat_support_footprint() {
    let allocator = BuildingAllocator::new();
    let building = Building {
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 7.0,
        width_cells: 2,
        depth_cells: 2,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type: crate::simulation::zoning::ZoneType::Residential,
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
        asset_id: String::new(),
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
    };

    let site = allocator.derive_building_site_client(&building, 10.0);

    assert!((signed_polygon_area(&site.footprint_world).abs() - 400.0).abs() <= 0.001);
    assert!((signed_polygon_area(&site.lot_footprint_world).abs() - 400.0).abs() <= 0.001);
    assert!(site.contains_point(Vector2::new(9.9, 0.0)));
    assert!(!site.contains_point(Vector2::new(10.1, 0.0)));
    assert_eq!(site.support_height_m, 7.0);
}

#[test]
fn required_support_footprint_keeps_driveway_clear_of_road_boundary() {
    use crate::assets::BuildingData;
    use crate::assets::asset::PlacementMode;

    let mut mesh_part = MeshPart::single_lod0("main", "main.glb");
    mesh_part.position = [7.0, 0.0, 0.0];
    mesh_part.scale = 2.0;
    let manifest = AssetManifest {
        asset_id: "building.test.site".to_owned(),
        display_name: "Site Test".to_owned(),
        asset_set: None,
        tags: Vec::new(),
        thumbnail: None,
        lods: Vec::new(),
        mesh_parts: vec![mesh_part],
        anchors: vec![
            Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [4.0, 0.0, -2.0],
                forward: [0.0, 0.0, -1.0],
                width_m: None,
                length_m: None,
                vehicle_class: None,
            },
            Anchor {
                anchor_type: AnchorType::Driveway,
                name: String::new(),
                position: [0.0, 0.0, -15.0],
                forward: [0.0, 0.0, 1.0],
                width_m: Some(3.0),
                length_m: None,
                vehicle_class: Some("car".to_owned()),
            },
        ],
        site_surfaces: Vec::new(),
        building: Some(BuildingData {
            placement_mode: PlacementMode::Explicit,
            zone_type: None,
            density: None,
            lot_width_cells: 4,
            lot_depth_cells: 3,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity: None,
            worker_capacity: Some(1),
            flat_size_m2: None,
            service_class: None,
            economy_profile: None,
        }),
        prop: None,
        vehicle: None,
        character: None,
    };

    let support = required_flat_support_footprint_local(&manifest, 20.0, 15.0);
    let frontage_dir = Vector2::new(0.0, -1.0);
    let frontage_limit = frontage_projection_limit(frontage_dir, 20.0, 15.0);
    let support_limit = frontage_limit - BUILDING_SITE_ROAD_ACCESS_CLEARANCE_M;
    let max_frontage_projection = support
        .iter()
        .map(|point| frontage_projection(*point, frontage_dir))
        .fold(f32::NEG_INFINITY, f32::max);
    let access_edge_points = support
        .iter()
        .filter(|point| (frontage_projection(**point, frontage_dir) - support_limit).abs() <= 0.001)
        .collect::<Vec<_>>();

    assert!(
        max_frontage_projection <= support_limit + 0.001,
        "access support must stay behind the road boundary: {support:?}"
    );
    assert!(
        !access_edge_points.is_empty(),
        "driveway support should still define an interior access edge: {support:?}"
    );
    assert!(
        access_edge_points.iter().all(|point| point.x.abs() <= 2.0),
        "road-facing access support should stay near the driveway width: {support:?}"
    );
    assert!(
        signed_polygon_area(&support).abs() < 40.0 * 30.0,
        "required support must not silently become the full lot"
    );
}

#[test]
fn site_grading_nearest_road_sample_uses_visible_surface_edge() {
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::types::EdgeClass;
    use crate::simulation::zoning::ZoningSystem;

    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let mut allocator = BuildingAllocator::new();
    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 6.0, 20.0), Vector3::new(60.0, 6.0, 20.0)],
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    let terrain = TerrainSystem::with_chunking(96, 96, 1.0, 16, 0.0);
    network.road_surface.compile_dirty(&graph, &terrain);

    let edge_idx = graph.edge_count() - 1;
    let edge = graph.edge(edge_idx);
    let center = BuildingAllocator::sample_pos_on_edge(&graph, edge_idx, 0.5);
    let tangent = BuildingAllocator::sample_tangent_on_edge(&graph, edge_idx, 0.5);
    let normal = Vector2::new(tangent.y, -tangent.x).normalized();
    let road_edge_probe = center + normal * building_site_road_connection_lateral_offset_m(edge);
    let apron_probe = road_edge_probe + normal * 0.5;
    let expected_height_m = network
        .road_surface
        .sample_visible_surface_height(&graph, &terrain, road_edge_probe.x, road_edge_probe.y)
        .expect("road surface edge should be queryable");

    let (probe, height_m) = nearest_building_site_road_surface_sample(
        &terrain,
        &graph,
        &network.road_surface,
        apron_probe,
        BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M,
    )
    .expect("nearby apron guide should find the road surface edge");

    assert!(probe.distance_to(road_edge_probe) <= 0.001);
    assert!((height_m - expected_height_m).abs() <= 0.001);
}
