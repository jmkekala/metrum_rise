// SPDX-License-Identifier: GPL-2.0-only

//! Building-site derivation, grading, and query regression tests.

use super::derive::{
    BUILDING_SITE_ROAD_ACCESS_CLEARANCE_M, frontage_projection, frontage_projection_limit,
    required_flat_support_footprint_local,
};
use super::geometry::{signed_polygon_area, site_radius_m};
use super::grading::{
    BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M, BuildingSiteGradingRequest,
    SiteGradingContext, SiteGradingGuideSink, append_building_site_grading_guides,
    building_site_raw_tie_in_target_height, building_site_road_connection_lateral_offset_m,
    nearest_building_site_road_surface_sample,
};
use super::model::{
    BuildingSiteSurfaceClient, BuildingSiteTerrainClient, BuildingSiteTerrainSnapshot,
};
use super::{BuildingSiteClient, building_site_support_tie_in_is_valid};
use crate::assets::{Anchor, AnchorType, AssetManifest, MeshPart, SiteSurfaceMaterial};
use crate::simulation::buildings::allocator::{BuildingAllocator, indexed_test_building};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::{RoadSurfaceSystem, RoadSurfaceView};
use crate::simulation::network::types::{TransitFlags, TransitType};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtInput, TerrainCdtPatch, TerrainCdtVertex, build_road_touched_terrain_patch,
};
use godot::prelude::{Vector2, Vector3};
use std::collections::HashSet;

#[test]
fn site_radius_is_measured_from_the_indexed_lot_center() {
    let site = BuildingSiteClient {
        foundation_mesh: Default::default(),
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

#[test]
fn site_radius_tracks_replacement_removal_and_cross_chunk_height_queries() {
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::economy::households::HouseholdSystem;
    use crate::simulation::economy::logistics::ShipmentSystem;
    use crate::simulation::zoning::{ZoneType, ZoningSystem};

    let mut allocator = BuildingAllocator::new();
    for (x, width) in [(510.0, 1), (1_024.0, 4), (2_048.0, 2)] {
        let mut building = indexed_test_building(String::new(), ZoneType::Residential, 0);
        building.center_x = x;
        building.width_cells = width;
        building.depth_cells = width;
        building.support_height_m = 7.0;
        allocator.buildings.push(building);
    }
    allocator.prepare_building_site_query_index(10.0);
    let assert_radius = |allocator: &BuildingAllocator| {
        let expected = allocator
            .building_sites
            .iter()
            .map(site_radius_m)
            .fold(0.0, f32::max);
        assert_eq!(allocator.max_site_radius_m.to_bits(), expected.to_bits());
    };
    assert_radius(&allocator);
    assert_eq!(
        allocator.sample_building_site_height(Vector2::new(513.0, 0.0)),
        Some(7.0)
    );
    for (width, probe, expected) in [
        (6, 535.0, Some(9.0)),
        (2, 535.0, None),
        (2, 518.0, Some(9.0)),
    ] {
        allocator.buildings[0].width_cells = width;
        allocator.buildings[0].depth_cells = width;
        allocator.buildings[0].support_height_m = 9.0;
        allocator.rebuild_building_site_client(0, 10.0);
        assert_radius(&allocator);
        assert_eq!(
            allocator.sample_building_site_height(Vector2::new(probe, 0.0)),
            expected
        );
    }
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    // Remove a smaller site, then the maximum site; the final removal leaves an empty cache.
    for idx in [0, 1, 0] {
        assert!(allocator.remove_building_for_bulldoze(
            idx,
            &mut zoning,
            &mut agents,
            &mut households,
            &mut logistics,
            &mut 0.0
        ));
        assert_radius(&allocator);
    }
    assert_eq!(allocator.max_site_radius_m, 0.0);
}

#[test]
#[ignore = "manual matched release timing of local site rebuild and exact radius reduction"]
fn benchmark_site_radius_maintenance() {
    use crate::simulation::zoning::ZoneType;
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::hint::black_box;
    use std::time::Instant;

    for count in [1, 1_024, 65_536, 262_144] {
        let mut allocator = BuildingAllocator::new();
        for idx in 0..count {
            // Isolated site records avoid gameplay placement and routing setup in the measurement.
            let mut building = indexed_test_building(String::new(), ZoneType::Residential, 0);
            if idx > 0 {
                building.center_x = 2_048.0 + (idx % 512) as f32 * 32.0;
                building.center_y = 2_048.0 + (idx / 512) as f32 * 32.0;
            }
            building.width_cells = if idx == 0 {
                1
            } else if idx == 1 {
                4
            } else {
                2
            };
            building.depth_cells = building.width_cells;
            allocator.buildings.push(building);
        }
        allocator.rebuild_building_site_clients(10.0);
        let checksum = |allocator: &BuildingAllocator| {
            let mut hash = DefaultHasher::new();
            allocator.max_site_radius_m.to_bits().hash(&mut hash);
            for site in &allocator.building_sites {
                site.support_height_m.to_bits().hash(&mut hash);
                for point in &site.footprint_world {
                    (point.x.to_bits(), point.y.to_bits()).hash(&mut hash);
                }
            }
            hash.finish()
        };
        let expected = checksum(&allocator);
        for phase in ["local_rebuild", "radius_reduce"] {
            let mut execute = || {
                if phase == "local_rebuild" {
                    allocator.rebuild_building_site_client(black_box(0), black_box(10.0));
                } else {
                    allocator.recompute_max_site_radius_m();
                }
                black_box(allocator.max_site_radius_m);
            };
            for _ in 0..3 {
                execute();
            }
            let mut samples = [0.0; 21];
            for sample in &mut samples {
                let start = Instant::now();
                for _ in 0..4 {
                    execute();
                }
                *sample = start.elapsed().as_secs_f64() * 1_000.0 / 4.0;
            }
            samples.sort_by(f64::total_cmp);
            assert_eq!(checksum(&allocator), expected);
            eprintln!(
                "site_radius_maintenance sites={count} phase={phase} median_ms={:.9} checksum={expected}",
                samples[10]
            );
        }
    }
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
        foundation_mesh: Default::default(),
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
        foundation_mesh: Default::default(),
        footprint_world,
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
fn site_support_validation_target_uses_visible_road_surface() {
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
    let graded_height_m = building_site_raw_tie_in_target_height(
        pos,
        100.0,
        &terrain,
        RoadSurfaceView::new(&graph, &road_surface),
    );

    assert!(
        (graded_height_m - expected_height_m).abs() <= 0.001,
        "site grading must use the visible road surface: graded={graded_height_m:.3} expected={expected_height_m:.3}"
    );
}

#[test]
fn site_paving_does_not_offset_physical_support() {
    let site = square_site_with_surface();

    assert_eq!(site.height_at(Vector2::new(0.0, 0.0)), Some(2.0));
    assert_eq!(site.height_at(Vector2::new(4.0, 4.0)), Some(2.0));
}

#[test]
fn site_height_includes_surface_and_footprint_boundaries() {
    let site = square_site_with_surface();

    assert_eq!(site.height_at(Vector2::new(1.0, 0.0)), Some(2.0));
    assert_eq!(site.height_at(Vector2::new(5.0, 0.0)), Some(2.0));
}

#[test]
fn site_raycast_uses_physical_support_not_paving_offset() {
    let site = square_site_with_surface();

    let hit = site
        .raycast(Vector3::new(0.0, 10.0, 0.0), Vector3::DOWN)
        .expect("ray should hit site surface");

    assert!((hit.y - 2.0).abs() <= f32::EPSILON);
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
    let mut sample_keys = HashSet::new();

    let context = SiteGradingContext::new(&terrain, &graph, &road_surface, 2.0, 16.0);
    let mut sink = SiteGradingGuideSink::new(&mut samples, &mut sample_keys);
    append_building_site_grading_guides(&site, &context, &mut sink);

    assert!(
        samples.iter().any(|sample| {
            (sample.vertex.x + 6.0).abs() <= 0.001
                && sample.vertex.z.abs() <= 1.001
                && sample.vertex.height_m.abs() <= 0.001
        }),
        "apron samples must share the source terrain authority; CDT owns the final grade"
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
            surfaces: Vec::new(),
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
    let mut sample_keys = HashSet::new();

    snapshot.append_terrain_cdt_site_grading_guides_for_world_bounds(
        BuildingSiteGradingRequest::new(
            &terrain,
            RoadSurfaceView::new(&graph, &road_surface),
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
                && sample.vertex.height_m.abs() <= 0.001
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
    assert_eq!(direct.len(), 1);
    assert_eq!(
        direct[0].vertices.len(),
        allocator.building_sites[0].footprint_world.len()
    );
    for (vertex, point) in direct[0]
        .vertices
        .iter()
        .zip(&allocator.building_sites[0].footprint_world)
    {
        assert_eq!(
            (vertex.x, vertex.z),
            (f64::from(point.x), f64::from(point.y))
        );
        assert_eq!(
            vertex.height_m,
            allocator.building_sites[0].support_height_m
        );
    }
    for bounds in [(-8.0, -8.0, 8.0, 8.0), (20.0, 20.0, 30.0, 30.0)] {
        assert_eq!(
            snapshot.has_building_site_for_world_bounds(bounds.0, bounds.1, bounds.2, bounds.3),
            allocator.has_building_site_for_world_bounds(bounds.0, bounds.1, bounds.2, bounds.3),
        );
    }
}

#[test]
fn neighboring_different_height_site_loops_with_grading_space_compile() {
    let mut allocator = BuildingAllocator::new();
    allocator
        .building_sites
        .push(flat_site_from_bounds(-5.0, -5.0, -1.0, 5.0, 0.0));
    allocator
        .building_sites
        .push(flat_site_from_bounds(1.0, -5.0, 5.0, 5.0, 1.0));
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
    .expect("different-height pads reserve space outside their footprints for grading");

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
    let mut building = indexed_test_building(
        String::new(),
        crate::simulation::zoning::ZoneType::Residential,
        0,
    );
    building.support_height_m = 7.0;
    building.width_cells = 2;
    building.depth_cells = 2;
    building.frontage_t = 0.0;
    building.operating_budget = 0.0;
    building.profit_tax_budget_baseline = 0.0;

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
    mesh_part.imported_bounds = Some([[-0.75, 0.0, -0.75], [0.75, 1.0, 0.75]]);
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
            extractor: None,
            field: None,
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
    assert!(
        max_frontage_projection <= support_limit + 0.001,
        "access support must stay behind the road boundary: {support:?}"
    );
    assert!(
        max_frontage_projection < support_limit - 0.001,
        "driveway must not extend the flat foundation to the frontage: {support:?}"
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
        RoadSurfaceView::new(&graph, &network.road_surface),
        apron_probe,
        BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M,
    )
    .expect("nearby apron guide should find the road surface edge");

    assert!(probe.distance_to(road_edge_probe) <= 0.001);
    assert!((height_m - expected_height_m).abs() <= 0.001);
}

#[test]
fn frontage_paving_has_a_flat_yard_and_separate_boundary_tie_ins() {
    let mut manifest: AssetManifest = toml::from_str(
        r#"
asset_id = "building.test.graded_frontage"
display_name = "Graded frontage"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
lot_width_cells = 2
lot_depth_cells = 2
frontage_forward = [0, 0, -1]
[[mesh_parts]]
name = "house"
position = [0, 0, 2]
scale = 7
[[site_surfaces]]
material = "asphalt"
y_m = 0.01
vertices = [[-10, -10], [10, -10], [10, -3.5], [-10, -3.5]]
"#,
    )
    .unwrap();
    manifest.mesh_parts[0].imported_bounds = Some([[-0.75, 0.0, -0.75], [0.75, 1.0, 0.75]]);
    let support = required_flat_support_footprint_local(&manifest, 10.0, 10.0);
    assert!(
        support.iter().all(|p| p.y > -9.0),
        "paving forced a flat road-edge seam: {support:?}"
    );
    assert!(super::geometry::point_in_polygon_slice(
        Vector2::new(0.0, 2.0),
        &support
    ));
    for point in [Vector2::new(0.0, -6.0), Vector2::new(7.0, -6.0)] {
        assert!(
            super::geometry::point_in_polygon_slice(point, &support),
            "usable yard must share the flat support: {point:?} outside {support:?}"
        );
    }
    for point in [Vector2::new(0.0, -9.0), Vector2::new(9.0, -6.0)] {
        assert!(
            !super::geometry::point_in_polygon_slice(point, &support),
            "lot-edge tie-in must remain outside the flat pad: {point:?}"
        );
    }
}

#[test]
fn imported_support_bounds_apply_scale_yaw_and_pivot_without_padding() {
    let mut manifest: AssetManifest = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/building-site.toml"
    )))
    .unwrap();
    manifest.anchors.clear();
    manifest.site_surfaces.clear();
    let part = &mut manifest.mesh_parts[0];
    part.imported_bounds = Some([[-1.0, 0.0, -2.0], [3.0, 1.0, 1.0]]);
    part.position = [2.0, 0.0, -1.0];
    part.rotation_degrees = [0.0, 90.0, 0.0];
    part.pivot_offset = Some([0.5, 0.0, -0.25]);
    part.scale = 2.0;
    let support = required_flat_support_footprint_local(&manifest, 20.0, 20.0);
    assert_eq!(support.len(), 4);
    for point in [
        Vector2::new(-2.5, -8.0),
        Vector2::new(3.5, -8.0),
        Vector2::new(3.5, 0.0),
        Vector2::new(-2.5, 0.0),
    ] {
        assert!(
            support.iter().any(|p| p.distance_to(point) < 0.001),
            "{support:?}"
        );
    }
    assert!((signed_polygon_area(&support).abs() - 48.0).abs() < 0.001);
}

#[test]
fn site_grading_consumes_planned_visual_writes_and_resets() {
    use crate::simulation::terrain::{TerrainVisualOverlay, TerrainVisualSource};

    let base = TerrainSystem::with_chunking(129, 129, 1.0, 16, 0.0);
    let mut live = base.clone(); // Independent test oracle.
    let mut overlay = TerrainVisualOverlay::new(&base);
    let writes: Vec<_> = (52..=76)
        .flat_map(|z| (52..=76).map(move |x| (x, z, 0.01)))
        .collect();
    overlay.set_heights(&base, &writes, |sample| *sample);
    live.set_visual_heights_at_grid_unmarked(&writes, |sample| *sample);
    overlay.reset_region_from_source_world(&base, 0.0, 0.0, 16.0, 16.0);
    live.reset_visual_region_from_source_world(0.0, 0.0, 16.0, 16.0);
    overlay.discard_unchanged(&base);
    let sites = BuildingSiteTerrainSnapshot {
        sites: vec![BuildingSiteTerrainClient {
            surfaces: Vec::new(),
            building_idx: 0,
            footprint_world: vec![
                Vector2::new(-2.0, -2.0),
                Vector2::new(2.0, -2.0),
                Vector2::new(2.0, 2.0),
                Vector2::new(-2.0, 2.0),
            ],
            support_height_m: 0.0,
        }],
    };
    let graph = RegionGraph::new();
    let surface = RoadSurfaceSystem::new(64.0);
    let roads = RoadSurfaceView::new(&graph, &surface);
    let guides = |terrain: &dyn TerrainVisualSource| {
        let mut samples = Vec::new();
        sites.append_terrain_cdt_site_grading_guides_for_world_bounds(
            BuildingSiteGradingRequest::new(terrain, roads, (-32.0, -32.0, 32.0, 32.0), 2.0),
            &mut samples,
            &mut HashSet::new(),
        );
        samples
    };
    let planned = guides(&overlay.view(&base));
    assert!(!planned.is_empty());
    assert_ne!(
        planned,
        guides(&base),
        "site guides must sample the changed visual ground"
    );
    assert_eq!(planned, guides(&live));
    assert!(
        base.clone_visual_dense()
            .iter()
            .all(|height| *height == 0.0)
    );
}
