// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: fixtures.rs
//  script_path: rust/src/simulation/network/surface/tests/support/fixtures.rs
//  module_name: fixtures
//  version: 0.1.0
//  description: Shared graph, terrain, and logged-input fixtures for
//           road-surface tests.
//  kind: test
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Shared graph, terrain, and logged-input fixtures for road-surface tests.

use super::*;

// ========================================================================
// FIXTURES
// ========================================================================

pub(in crate::simulation::network::surface::tests) fn test_edge(
    start_node: u32,
    end_node: u32,
    points: Vec<Vector3>,
    width: f32,
    class: EdgeClass,
    primary_type: TransitType,
    allowed_types: u8,
) -> Edge {
    let length = points
        .windows(2)
        .map(|segment| segment[0].distance_to(segment[1]))
        .sum();
    let lane_count: u8 = if (allowed_types & TransitFlags::CAR) != 0 {
        ((width / crate::config::LANE_WIDTH).round() as u8).max(1)
    } else {
        0
    };
    Edge {
        start_node,
        end_node,
        primary_type,
        allowed_types,
        class,
        width,
        lanes: crate::simulation::network::graph::LaneLayout::from_counts(lane_count, lane_count),
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
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        frontage_class: Default::default(),
    }
}

pub(in crate::simulation::network::surface::tests) fn flat_terrain(
    width: usize,
    height: usize,
) -> TerrainSystem {
    TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0)
}

pub(in crate::simulation::network::surface::tests) fn sloped_terrain(
    width: usize,
    height: usize,
) -> TerrainSystem {
    let mut terrain = TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0);
    for z in 0..height {
        for x in 0..width {
            terrain.set_height(x, z, x as f32 * 0.05);
        }
    }
    terrain
}

pub(in crate::simulation::network::surface::tests) fn road_points_from_json(
    points_json: &str,
) -> Vec<Vector3> {
    serde_json::from_str::<Vec<[f32; 3]>>(points_json)
        .expect("logged road geometry points must parse")
        .into_iter()
        .map(|[x, y, z]| Vector3::new(x, y, z))
        .collect()
}

pub(in crate::simulation::network::surface::tests) fn terrain_clip_source_edge_for_test(
    start: backend::RoadVec3,
    end: backend::RoadVec3,
) -> RoadSurfaceTerrainClipSourceEdge {
    RoadSurfaceTerrainClipSourceEdge {
        start,
        end,
        kind: RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
        source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id: 0,
            kind: RoadSurfaceVisualNodePieceKind::Terminal,
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 0,
            boundary_source: None,
        },
    }
}

pub(in crate::simulation::network::surface::tests) fn ridge_terrain(
    width: usize,
    height: usize,
) -> TerrainSystem {
    let mut terrain = TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0);
    let center_x = (width as f32 - 1.0) * 0.5;
    for z in 0..height {
        for x in 0..width {
            let dx = x as f32 - center_x;
            let ridge = (1.0 - (dx.abs() / 12.0).min(1.0)) * 6.0;
            terrain.set_height(x, z, ridge.max(0.0));
        }
    }
    terrain
}

pub(in crate::simulation::network::surface::tests) fn planar_world_terrain(
    width: usize,
    height: usize,
    cell_size_m: f32,
    base_height_m: f32,
    slope_x_m_per_m: f32,
    slope_z_m_per_m: f32,
) -> TerrainSystem {
    let mut terrain = TerrainSystem::with_chunking(width, height, cell_size_m, 8, 0.0);
    for z in 0..height {
        for x in 0..width {
            let (world_x, world_z) = terrain.grid_to_world_coords(x, z);
            let height_m = base_height_m + world_x * slope_x_m_per_m + world_z * slope_z_m_per_m;
            terrain.set_height(x, z, height_m / crate::config::HEIGHT_SCALE);
        }
    }
    terrain
}

pub(in crate::simulation::network::surface::tests) fn coarse_hillside_world_terrain(
    width: usize,
    height: usize,
    cell_size_m: f32,
) -> TerrainSystem {
    let mut terrain = TerrainSystem::with_chunking(width, height, cell_size_m, 8, 0.0);
    for z in 0..height {
        for x in 0..width {
            let (world_x, world_z) = terrain.grid_to_world_coords(x, z);
            let ridge_dx = world_x + 45.0;
            let ridge = 8.0 * (-(ridge_dx * ridge_dx) / (2.0 * 55.0 * 55.0)).exp();
            let shoulder_dx = world_x - world_z * 0.12 + 25.0;
            let shoulder = 4.0 * (-(shoulder_dx * shoulder_dx) / (2.0 * 85.0 * 85.0)).exp();
            let height_m = 150.0 + world_x * 0.06 - world_z * 0.012 + ridge + shoulder;
            terrain.set_height(x, z, height_m / crate::config::HEIGHT_SCALE);
        }
    }
    terrain
}

pub(in crate::simulation::network::surface::tests) fn grounded_polyline_points_from_terrain(
    terrain: &TerrainSystem,
    start_xz: Vector2,
    end_xz: Vector2,
    segments: usize,
) -> Vec<Vector3> {
    let segments = segments.max(1);
    (0..=segments)
        .map(|idx| {
            let t = idx as f32 / segments as f32;
            let world_x = start_xz.x + (end_xz.x - start_xz.x) * t;
            let world_z = start_xz.y + (end_xz.y - start_xz.y) * t;
            let world_y =
                terrain.sample_height_world(world_x, world_z) * crate::config::HEIGHT_SCALE;
            Vector3::new(world_x, world_y, world_z)
        })
        .collect()
}

pub(in crate::simulation::network::surface::tests) fn build_coarse_grid_hillside_case(
    cell_size_m: f32,
) -> (RoadSurfaceSystem, TerrainSystem, RegionGraph, usize) {
    let cells = ((800.0 / cell_size_m).round() as usize).max(2) + 1;
    let mut terrain = coarse_hillside_world_terrain(cells, cells, cell_size_m);
    let points = grounded_polyline_points_from_terrain(
        &terrain,
        Vector2::new(120.0, 40.0),
        Vector2::new(-180.0, -220.0),
        24,
    );

    let mut graph = RegionGraph::new();
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(128.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    (surface, terrain, graph, edge_idx)
}
