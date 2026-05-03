//! Unit tests for the road-surface compiler and ownership caches.

use super::earthwork::EARTHWORK_MAX_MARGIN_M;
use super::edge::CURB_STEP_HEIGHT_M;
use super::{
    ChunkCacheKind, PreviewRoadSurfaceResult, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualNodePiece,
    RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M, SurfaceChunkKey,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtInput, TerrainCdtPatch, TerrainCdtRoadLoop, TerrainCdtVertex,
    build_road_touched_terrain_patch,
};
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet};

fn test_edge(
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
    Edge {
        start_node,
        end_node,
        primary_type,
        allowed_types,
        class,
        width,
        fwd_lanes: if (allowed_types & TransitFlags::CAR) != 0 {
            ((width / crate::config::LANE_WIDTH).round() as u8).max(1)
        } else {
            0
        },
        bkw_lanes: if (allowed_types & TransitFlags::CAR) != 0 {
            ((width / crate::config::LANE_WIDTH).round() as u8).max(1)
        } else {
            0
        },
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
    }
}

fn flat_terrain(width: usize, height: usize) -> TerrainSystem {
    TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0)
}

fn sloped_terrain(width: usize, height: usize) -> TerrainSystem {
    let mut terrain = TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0);
    for z in 0..height {
        for x in 0..width {
            terrain.set_height(x, z, x as f32 * 0.05);
        }
    }
    terrain
}

fn ridge_terrain(width: usize, height: usize) -> TerrainSystem {
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

#[allow(dead_code)]
fn planar_world_terrain(
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

fn coarse_hillside_world_terrain(width: usize, height: usize, cell_size_m: f32) -> TerrainSystem {
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

fn grounded_polyline_points_from_terrain(
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

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
struct FootprintOverflowMetrics {
    max_overflow_m: f32,
    section_s_m: f32,
    lateral_offset_m: f32,
    road_height_m: f32,
    visual_height_m: f32,
}

fn footprint_sample_offsets(section: &RoadSurfaceSection) -> Vec<f32> {
    let mut offsets = Vec::new();
    for band in &section.bands {
        if !matches!(
            band.kind,
            super::RoadSurfaceBandKind::Carriageway
                | super::RoadSurfaceBandKind::CurbOrShoulder
                | super::RoadSurfaceBandKind::Sidewalk
                | super::RoadSurfaceBandKind::Footpath
        ) {
            continue;
        }
        offsets.push(band.lateral_start_m);
        offsets.push((band.lateral_start_m + band.lateral_end_m) * 0.5);
        offsets.push(band.lateral_end_m);
    }
    offsets.sort_by(|a, b| a.total_cmp(b));
    offsets.dedup_by(|a, b| (*a - *b).abs() <= 0.001);
    offsets
}

fn measure_max_footprint_overflow(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    edge_idx: usize,
    terrain: &TerrainSystem,
) -> FootprintOverflowMetrics {
    let mut best = FootprintOverflowMetrics {
        max_overflow_m: f32::NEG_INFINITY,
        section_s_m: 0.0,
        lateral_offset_m: 0.0,
        road_height_m: 0.0,
        visual_height_m: 0.0,
    };

    let sections = surface.compiled_sections().get(&edge_idx).unwrap();
    for section in sections {
        for lateral_offset_m in footprint_sample_offsets(section) {
            let Some(road_height_m) = section_height_at_lateral_offset(section, lateral_offset_m)
            else {
                continue;
            };
            let sample_x = section.center_xz.x + section.lateral_xz.x * lateral_offset_m;
            let sample_z = section.center_xz.y + section.lateral_xz.y * lateral_offset_m;
            let visual_height_m = surface
                .sample_paved_support_height(graph, terrain, sample_x, sample_z)
                .unwrap_or_else(|| {
                    terrain.sample_visual_height_world(sample_x, sample_z)
                        * crate::config::HEIGHT_SCALE
                });
            let overflow_m = visual_height_m - road_height_m;
            if overflow_m > best.max_overflow_m {
                best = FootprintOverflowMetrics {
                    max_overflow_m: overflow_m,
                    section_s_m: section.s_m,
                    lateral_offset_m,
                    road_height_m,
                    visual_height_m,
                };
            }
        }
    }

    best
}

fn build_coarse_grid_hillside_case(
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

fn compile_committed_preview_reference(
    surface: &RoadSurfaceSystem,
    raw_points: &[Vector3],
    terrain: &TerrainSystem,
    fwd_lanes: u8,
    bkw_lanes: u8,
) -> (
    PreviewRoadSurfaceResult,
    Vec<RoadSurfaceSection>,
    Vec<RoadSurfaceVisualNodePiece>,
) {
    let preview = surface.compile_preview_surface(raw_points, fwd_lanes, bkw_lanes, terrain);
    if preview.prepared_points.len() < 2 {
        return (preview, Vec::new(), Vec::new());
    }

    let mut graph = RegionGraph::new();
    let start_node = graph.add_node(preview.prepared_points[0], NodeType::Junction);
    let end_node = graph.add_node(*preview.prepared_points.last().unwrap(), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start_node,
        end_node,
        preview.prepared_points.clone(),
        ((fwd_lanes + bkw_lanes) as f32 * crate::config::LANE_WIDTH).max(2.0),
        preview.edge_class,
        if fwd_lanes == 0 && bkw_lanes == 0 {
            TransitType::Foot
        } else {
            TransitType::Road
        },
        if fwd_lanes == 0 && bkw_lanes == 0 {
            TransitFlags::FOOT
        } else {
            TransitFlags::CAR | TransitFlags::FOOT
        },
    ));

    let mut committed = RoadSurfaceSystem::new(surface.chunk_span_m());
    committed.compile_dirty(&graph, terrain);
    let compiled_sections = committed
        .compiled_sections()
        .get(&edge_idx)
        .cloned()
        .unwrap_or_default();
    let compiled_visual_node_pieces = [start_node, end_node]
        .into_iter()
        .filter_map(|node_id| {
            committed
                .compiled_visual_node_pieces()
                .get(&node_id)
                .cloned()
        })
        .collect();
    (preview, compiled_sections, compiled_visual_node_pieces)
}

fn triangle_centroid_xz(triangle: [Vector3; 3]) -> Vector2 {
    Vector2::new(
        (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0,
        (triangle[0].z + triangle[1].z + triangle[2].z) / 3.0,
    )
}

fn point_inside_visual_polygons(polygons: &[RoadSurfaceVisualPolygon], point: Vector2) -> bool {
    polygons.iter().any(|polygon| {
        if polygon.triangles_world.is_empty() {
            RoadSurfaceSystem::polygon_contains_point_xz(&polygon.points_world, point)
        } else {
            polygon.triangles_world.iter().any(|&triangle| {
                RoadSurfaceSystem::triangle_barycentric_weights_xz(triangle, point).is_some()
            })
        }
    })
}

fn node_region_height_at_kind(
    piece: &RoadSurfaceVisualNodePiece,
    kind: RoadSurfaceBandKind,
    point: Vector2,
) -> Option<f32> {
    for region in &piece.owned_regions {
        if region.kind != kind {
            continue;
        }
        for &triangle in &region.polygon.triangles_world {
            if let Some((wa, wb, wc)) =
                RoadSurfaceSystem::triangle_barycentric_weights_xz(triangle, point)
            {
                return Some(triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc);
            }
        }
        if region.polygon.triangles_world.is_empty()
            && RoadSurfaceSystem::polygon_contains_point_xz(&region.polygon.points_world, point)
        {
            let height_sum: f32 = region
                .polygon
                .points_world
                .iter()
                .map(|point| point.y)
                .sum();
            return Some(height_sum / region.polygon.points_world.len() as f32);
        }
    }
    None
}

fn assert_material_triangles_do_not_overlap(piece: &RoadSurfaceVisualNodePiece) {
    for non_road_region in piece
        .owned_regions
        .iter()
        .filter(|region| region.kind != RoadSurfaceBandKind::Carriageway)
    {
        for &non_road_triangle in &non_road_region.polygon.triangles_world {
            for road_region in piece
                .owned_regions
                .iter()
                .filter(|region| region.kind == RoadSurfaceBandKind::Carriageway)
            {
                for &road_triangle in &road_region.polygon.triangles_world {
                    let overlap_area_m2 =
                        triangle_overlap_area_m2(non_road_triangle, road_triangle);
                    let area_budget_m2 =
                        triangle_overlap_numeric_budget_m2(non_road_triangle, road_triangle);
                    assert!(
                        overlap_area_m2 <= area_budget_m2,
                        "node material triangles must not overlap beyond numeric dust; kind={:?} overlap_area={overlap_area_m2:.8} budget={area_budget_m2:.8} non_road_triangle={non_road_triangle:?} road_triangle={road_triangle:?}",
                        non_road_region.kind
                    );
                }
            }
        }
    }
}

fn triangle_overlap_area_m2(a: [Vector3; 3], b: [Vector3; 3]) -> f32 {
    RoadSurfaceSystem::overlay_binary_shapes(
        &triangle_overlay_shapes(a),
        &triangle_overlay_shapes(b),
        OverlayRule::Intersect,
    )
    .unwrap_or_default()
    .iter()
    .map(RoadSurfaceSystem::overlay_shape_area_m2)
    .sum()
}

fn triangle_overlap_numeric_budget_m2(a: [Vector3; 3], b: [Vector3; 3]) -> f32 {
    RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&triangle_overlay_shapes(a)).max(
        RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&triangle_overlay_shapes(b)),
    )
}

fn triangle_overlay_shapes(triangle: [Vector3; 3]) -> super::NodeOverlayShapes {
    let mut contour = triangle
        .iter()
        .map(|point| [f64::from(point.x), f64::from(point.z)])
        .collect::<Vec<_>>();
    let area = (contour[1][0] - contour[0][0]) * (contour[2][1] - contour[0][1])
        - (contour[1][1] - contour[0][1]) * (contour[2][0] - contour[0][0]);
    if area < 0.0 {
        contour.swap(1, 2);
    }
    vec![vec![contour]]
}

fn assert_top_mesh_centroids_inside_outer_boundary(piece: &RoadSurfaceVisualNodePiece) {
    for triangle in piece
        .road_surface_polygons
        .iter()
        .chain(piece.sidewalk_surface_polygons.iter())
        .flat_map(|polygon| polygon.triangles_world.iter().copied())
    {
        let centroid = triangle_centroid_xz(triangle);
        assert!(
            point_inside_visual_polygons(&piece.outer_boundary_loops, centroid),
            "node outer boundary must contain emitted top-surface triangle centroids; centroid={centroid:?}"
        );
    }
}

type QuantizedXzKey = (i64, i64);
type QuantizedXzEdgeKey = (QuantizedXzKey, QuantizedXzKey);
type QuantizedTriangleEdge = (QuantizedXzEdgeKey, (f32, f32));

fn quantized_xz_key(point: Vector3) -> QuantizedXzKey {
    (
        (point.x * 1000.0).round() as i64,
        (point.z * 1000.0).round() as i64,
    )
}

fn normalized_quantized_edge_key(start: Vector3, end: Vector3) -> Option<QuantizedXzEdgeKey> {
    let start_key = quantized_xz_key(start);
    let end_key = quantized_xz_key(end);
    if start_key == end_key {
        return None;
    }
    Some(if start_key <= end_key {
        (start_key, end_key)
    } else {
        (end_key, start_key)
    })
}

fn collect_triangle_edge_heights(
    polygons: &[RoadSurfaceVisualPolygon],
) -> Vec<QuantizedTriangleEdge> {
    let mut edges: Vec<QuantizedTriangleEdge> = Vec::new();
    for triangle in polygons
        .iter()
        .flat_map(|polygon| polygon.triangles_world.iter())
    {
        for edge_index in 0..3 {
            let start = triangle[edge_index];
            let end = triangle[(edge_index + 1) % 3];
            let Some(key) = normalized_quantized_edge_key(start, end) else {
                continue;
            };
            let heights = if quantized_xz_key(start) <= quantized_xz_key(end) {
                (start.y, end.y)
            } else {
                (end.y, start.y)
            };
            edges.push((key, heights));
        }
    }
    edges.sort_by(|a, b| a.0.cmp(&b.0));
    edges
}

fn assert_shared_edges_are_height_continuous(
    polygons: &[RoadSurfaceVisualPolygon],
    tolerance_m: f32,
    label: &str,
) {
    let edges = collect_triangle_edge_heights(polygons);
    for pair in edges.windows(2) {
        if pair[0].0 != pair[1].0 {
            continue;
        }
        assert!(
            (pair[0].1.0 - pair[1].1.0).abs() <= tolerance_m
                && (pair[0].1.1 - pair[1].1.1).abs() <= tolerance_m,
            "{label} shared edge must be height-continuous; edge={:?} heights_a={:?} heights_b={:?}",
            pair[0].0,
            pair[0].1,
            pair[1].1
        );
    }
}

fn assert_non_road_shared_edges_are_height_continuous(piece: &RoadSurfaceVisualNodePiece) {
    assert_shared_edges_are_height_continuous(
        &piece.sidewalk_surface_polygons,
        0.004,
        "node non-road",
    );
}

fn assert_all_top_shared_edges_are_height_continuous(piece: &RoadSurfaceVisualNodePiece) {
    let top_polygons = piece
        .road_surface_polygons
        .iter()
        .chain(piece.sidewalk_surface_polygons.iter())
        .cloned()
        .collect::<Vec<_>>();
    assert_shared_edges_are_height_continuous(&top_polygons, 0.004, "node top");
}

fn assert_outer_boundary_edges_are_noded_by_visible_top(piece: &RoadSurfaceVisualNodePiece) {
    let top_polygons = piece
        .road_surface_polygons
        .iter()
        .chain(piece.sidewalk_surface_polygons.iter())
        .collect::<Vec<_>>();
    let mut top_edge_counts = BTreeMap::<QuantizedXzEdgeKey, usize>::new();
    for triangle in top_polygons
        .iter()
        .flat_map(|polygon| polygon.triangles_world.iter().copied())
    {
        for edge_index in 0..3 {
            let Some(key) =
                normalized_quantized_edge_key(triangle[edge_index], triangle[(edge_index + 1) % 3])
            else {
                continue;
            };
            *top_edge_counts.entry(key).or_default() += 1;
        }
    }
    let top_boundary_edges = top_edge_counts
        .into_iter()
        .filter_map(|(key, count)| (count == 1).then_some(key))
        .collect::<BTreeSet<_>>();

    let mut outer_edges = BTreeSet::new();
    for polygon in &piece.outer_boundary_loops {
        for index in 0..polygon.points_world.len() {
            let start = polygon.points_world[index];
            let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
            if let Some(key) = normalized_quantized_edge_key(start, end) {
                outer_edges.insert(key);
            }
        }
    }

    let extra = outer_edges
        .difference(&top_boundary_edges)
        .take(8)
        .copied()
        .collect::<Vec<_>>();
    assert!(
        extra.is_empty(),
        "node outer boundary must not export sparse skirt edges that are absent from the visible top mesh; extra_count={} extra_samples={extra:?}",
        outer_edges.difference(&top_boundary_edges).count()
    );
}

fn max_visual_triangle_slope_ratio(
    piece: &RoadSurfaceVisualNodePiece,
) -> (f32, RoadSurfaceBandKind, usize, Vector3, Vector3) {
    let mut max_slope = 0.0_f32;
    let mut max_kind = RoadSurfaceBandKind::Carriageway;
    let mut max_owner_index = 0usize;
    let mut max_start = Vector3::ZERO;
    let mut max_end = Vector3::ZERO;
    for region in &piece.owned_regions {
        for triangle in &region.polygon.triangles_world {
            for edge_index in 0..3 {
                let start = triangle[edge_index];
                let end = triangle[(edge_index + 1) % 3];
                let xz_distance = Vector2::new(end.x - start.x, end.z - start.z).length();
                if xz_distance <= SAMPLE_EPSILON_M {
                    continue;
                }
                let slope = (end.y - start.y).abs() / xz_distance;
                if slope > max_slope {
                    max_slope = slope;
                    max_kind = region.kind;
                    max_owner_index = region.owner_index;
                    max_start = start;
                    max_end = end;
                }
            }
        }
    }
    (max_slope, max_kind, max_owner_index, max_start, max_end)
}

fn assert_node_piece_uses_band_owned_regions(piece: &RoadSurfaceVisualNodePiece) {
    assert!(
        !piece.owned_regions.is_empty(),
        "node piece must keep explicit band-owned regions as its source of rendered top surfaces"
    );
    let carriageway_count = piece
        .owned_regions
        .iter()
        .filter(|region| region.kind == RoadSurfaceBandKind::Carriageway)
        .count();
    let non_road_count = piece
        .owned_regions
        .iter()
        .filter(|region| region.kind != RoadSurfaceBandKind::Carriageway)
        .count();
    assert_eq!(
        carriageway_count,
        piece.road_surface_polygons.len(),
        "asphalt polygons must be derived from carriageway-owned node regions"
    );
    assert_eq!(
        non_road_count,
        piece.sidewalk_surface_polygons.len(),
        "non-road polygons must be derived from curb/sidewalk-owned node regions"
    );
    assert!(
        piece
            .owned_regions
            .iter()
            .all(|region| RoadSurfaceSystem::polygon_has_area_xz(&region.polygon.points_world)),
        "owned node regions must be non-degenerate before triangulation"
    );
}

fn assert_node_piece_has_curb_and_sidewalk_owners(piece: &RoadSurfaceVisualNodePiece) {
    assert!(
        piece
            .owned_regions
            .iter()
            .any(|region| region.kind == RoadSurfaceBandKind::CurbOrShoulder),
        "node non-road hardcut must expose explicit curb/shoulder owners"
    );
    assert!(
        piece
            .owned_regions
            .iter()
            .any(|region| region.kind == RoadSurfaceBandKind::Sidewalk),
        "node non-road hardcut must expose explicit sidewalk owners"
    );
}

fn assert_outer_boundary_vertices_match_visible_top(piece: &RoadSurfaceVisualNodePiece) {
    let top_polygons = piece
        .road_surface_polygons
        .iter()
        .chain(piece.sidewalk_surface_polygons.iter())
        .collect::<Vec<_>>();
    let top_vertices = piece
        .road_surface_polygons
        .iter()
        .chain(piece.sidewalk_surface_polygons.iter())
        .flat_map(|polygon| {
            polygon.points_world.iter().chain(
                polygon
                    .triangles_world
                    .iter()
                    .flat_map(|triangle| triangle.iter()),
            )
        })
        .collect::<Vec<_>>();
    assert!(
        !top_vertices.is_empty(),
        "node piece must emit visible top vertices before boundary matching can be checked"
    );
    for boundary_point in piece
        .outer_boundary_loops
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
    {
        let overlay_match_tolerance_m = SAMPLE_EPSILON_M * 2.0;
        let Some(closest) = top_vertices.iter().min_by(|a, b| {
            let da = Vector2::new(a.x - boundary_point.x, a.z - boundary_point.z).length_squared();
            let db = Vector2::new(b.x - boundary_point.x, b.z - boundary_point.z).length_squared();
            da.total_cmp(&db)
        }) else {
            panic!("node piece emitted no top vertices");
        };
        let xz_error =
            Vector2::new(closest.x - boundary_point.x, closest.z - boundary_point.z).length();
        if xz_error <= overlay_match_tolerance_m {
            let matching_height = top_vertices.iter().any(|candidate| {
                Vector2::new(
                    candidate.x - boundary_point.x,
                    candidate.z - boundary_point.z,
                )
                .length()
                    <= overlay_match_tolerance_m
                    && (candidate.y - boundary_point.y).abs() <= overlay_match_tolerance_m
            });
            assert!(
                matching_height,
                "node outer boundary must use the colocated visible top height; boundary={boundary_point:?} closest={closest:?} xz_error={xz_error:.4}"
            );
            continue;
        }

        if let Some(height) = top_polygons.iter().find_map(|polygon| {
            polygon.triangles_world.iter().find_map(|&triangle| {
                RoadSurfaceSystem::triangle_barycentric_weights_xz(
                    triangle,
                    Vector2::new(boundary_point.x, boundary_point.z),
                )
                .map(|(wa, wb, wc)| triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc)
            })
        }) {
            assert!(
                (height - boundary_point.y).abs() <= overlay_match_tolerance_m,
                "node outer boundary must use the visible top-surface height at covered boundary points; boundary={boundary_point:?} sampled_height={height:.4}"
            );
        } else {
            panic!(
                "node outer boundary vertex must be covered by visible top geometry; boundary={boundary_point:?} closest={closest:?} xz_error={xz_error:.4}"
            );
        }
    }
}

fn polygon_area_m2(polygon: &RoadSurfaceVisualPolygon) -> f32 {
    RoadSurfaceSystem::signed_polygon_area_xz(&polygon.points_world).abs()
}

fn polygon_triangle_area_m2(polygon: &RoadSurfaceVisualPolygon) -> f32 {
    polygon
        .triangles_world
        .iter()
        .map(|triangle| {
            RoadSurfaceSystem::signed_polygon_area_xz(&[triangle[0], triangle[1], triangle[2]])
                .abs()
        })
        .sum()
}

fn assert_node_piece_material_area_closes_footprint(
    piece: &RoadSurfaceVisualNodePiece,
    tolerance_m2: f32,
) {
    let footprint_area: f32 = piece.outer_boundary_loops.iter().map(polygon_area_m2).sum();
    let asphalt_area: f32 = piece
        .road_surface_polygons
        .iter()
        .map(polygon_triangle_area_m2)
        .sum();
    let non_road_area: f32 = piece
        .sidewalk_surface_polygons
        .iter()
        .map(polygon_triangle_area_m2)
        .sum();
    assert!(
        (footprint_area - asphalt_area - non_road_area).abs() <= tolerance_m2,
        "node material ownership must close the exported footprint; footprint={footprint_area:.3} asphalt={asphalt_area:.3} non_road={non_road_area:.3} tolerance={tolerance_m2:.3}"
    );
}

#[test]
fn overlay_numeric_area_budget_accepts_logged_sub_visual_cdt_residual() {
    let small_four_edge_region = vec![vec![[0.0, 0.0], [0.02, 0.0], [0.02, 0.02], [0.0, 0.02]]];
    let budget_m2 =
        RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(&small_four_edge_region);

    assert!(
        budget_m2 > 1.6660093e-5,
        "the logged 60-degree T-junction CDT residual must be treated as numeric dust, budget={budget_m2:.8}"
    );
    assert!(
        budget_m2 <= 1.0e-3,
        "numeric dust acceptance must remain capped below visually meaningful polygon loss"
    );
}

#[test]
fn overlay_numeric_area_budget_accepts_logged_centimeter_scale_cdt_residual() {
    let meter_scale_region = vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]];
    let budget_m2 = RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(&meter_scale_region);

    assert!(
        budget_m2 > 0.00020319251,
        "the logged oblique 3-way CDT residual must be treated as numeric dust, budget={budget_m2:.8}"
    );
    assert!(
        budget_m2 <= 1.0e-3,
        "numeric dust acceptance must remain capped at 10 cm^2"
    );
}

#[test]
fn hill_crossing_input_stays_standard_instead_of_auto_tunnel() {
    let terrain = ridge_terrain(97, 33);
    let raw_points = vec![
        Vector3::new(
            -20.0,
            terrain.sample_height_world(-20.0, 0.0) * crate::config::HEIGHT_SCALE,
            0.0,
        ),
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(
            20.0,
            terrain.sample_height_world(20.0, 0.0) * crate::config::HEIGHT_SCALE,
            0.0,
        ),
    ];

    let (grounded_points, class) =
        RoadSurfaceSystem::classify_and_ground_road_points(&raw_points, &terrain);

    assert_eq!(class, EdgeClass::Standard);
    for point in grounded_points {
        let terrain_y = terrain.sample_height_world(point.x, point.z) * crate::config::HEIGHT_SCALE;
        assert!(
            (point.y - terrain_y).abs() <= 0.001,
            "standard grounding should snap to terrain at x={:.2}: point_y={:.3} terrain_y={:.3}",
            point.x,
            point.y,
            terrain_y
        );
    }
}

#[test]
fn uniformly_submerged_input_stays_auto_tunnel() {
    let terrain = flat_terrain(65, 33);
    let raw_points = vec![
        Vector3::new(-10.0, -2.5, 0.0),
        Vector3::new(0.0, -2.5, 0.0),
        Vector3::new(10.0, -2.5, 0.0),
    ];

    let (_points, class) =
        RoadSurfaceSystem::classify_and_ground_road_points(&raw_points, &terrain);
    assert_eq!(class, EdgeClass::Tunnel);
}

#[test]
fn uniformly_elevated_input_stays_auto_bridge() {
    let terrain = flat_terrain(65, 33);
    let raw_points = vec![
        Vector3::new(-10.0, 2.5, 0.0),
        Vector3::new(0.0, 2.5, 0.0),
        Vector3::new(10.0, 2.5, 0.0),
    ];

    let (_points, class) =
        RoadSurfaceSystem::classify_and_ground_road_points(&raw_points, &terrain);
    assert_eq!(class, EdgeClass::Bridge);
}

fn section_height_at_lateral_offset(
    section: &RoadSurfaceSection,
    lateral_offset_m: f32,
) -> Option<f32> {
    let mut best_height_m: Option<f32> = None;
    for band in &section.bands {
        let start = band.lateral_start_m.min(band.lateral_end_m);
        let end = band.lateral_start_m.max(band.lateral_end_m);
        if lateral_offset_m < start - 0.001 || lateral_offset_m > end + 0.001 {
            continue;
        }

        let span = band.lateral_end_m - band.lateral_start_m;
        let t = if span.abs() <= 0.001 {
            0.0
        } else {
            ((lateral_offset_m - band.lateral_start_m) / span).clamp(0.0, 1.0)
        };
        let height_m = band.height_start_m + (band.height_end_m - band.height_start_m) * t;
        best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
    }

    best_height_m
}

fn outer_surface_lateral_bounds(section: &RoadSurfaceSection) -> Option<(f32, f32)> {
    Some((
        section.bands.first()?.lateral_start_m,
        section.bands.last()?.lateral_end_m,
    ))
}

#[test]
fn mark_edge_dirty_tracks_edge_without_centerline_chunk_guess() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(5.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        n0,
        n1,
        vec![Vector3::new(5.0, 0.0, 0.0), Vector3::new(25.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(10.0);
    surface.mark_edge_dirty(&graph, edge_idx);

    assert!(surface.dirty_edges().contains(&edge_idx));
    assert!(surface.dirty_surface_chunks().is_empty());
    assert!(surface.dirty_terrain_chunks().is_empty());
}

#[test]
fn terrain_edit_marks_nearby_edges_nodes_and_chunks() {
    let mut graph = RegionGraph::new();
    let near_a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let near_b = graph.add_node(Vector3::new(8.0, 0.0, 0.0), NodeType::Junction);
    let far_a = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
    let far_b = graph.add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
    let near_edge = graph.add_edge(test_edge(
        near_a,
        near_b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(8.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let far_edge = graph.add_edge(test_edge(
        far_a,
        far_b,
        vec![Vector3::new(50.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(10.0);
    surface.mark_terrain_edit_dirty(&graph, Vector2::new(4.0, 0.0), 5.0);

    assert!(surface.dirty_edges().contains(&near_edge));
    assert!(!surface.dirty_edges().contains(&far_edge));
    assert!(surface.dirty_nodes().contains(&near_a));
    assert!(surface.dirty_nodes().contains(&near_b));
    assert!(!surface.dirty_nodes().contains(&far_a));
    assert!(!surface.dirty_nodes().contains(&far_b));
    assert!(surface.dirty_surface_chunks().contains(&(-1, -1)));
    assert!(surface.dirty_surface_chunks().contains(&(0, 0)));
    assert_eq!(
        surface.dirty_surface_chunks(),
        surface.dirty_terrain_chunks()
    );
}

#[test]
fn section_refinement_is_deterministic() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        n0,
        n1,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let terrain = flat_terrain(64, 64);
    let mut surface_a = RoadSurfaceSystem::new(16.0);
    let mut surface_b = RoadSurfaceSystem::new(16.0);
    surface_a.compile_dirty(&graph, &terrain);
    surface_b.compile_dirty(&graph, &terrain);

    let sections_a = surface_a.compiled_sections().get(&edge_idx).unwrap();
    let sections_b = surface_b.compiled_sections().get(&edge_idx).unwrap();
    assert_eq!(sections_a, sections_b);
    let s_values: Vec<f32> = sections_a.iter().map(|section| section.s_m).collect();
    assert_eq!(s_values, vec![0.0, 6.0, 8.0, 14.0, 16.0, 20.0]);
}

#[test]
fn standard_edge_sections_follow_solved_edge_profile_deterministically() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-16.0, 99.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(16.0, 99.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        n0,
        n1,
        vec![
            Vector3::new(-16.0, 99.0, 0.0),
            Vector3::new(16.0, 99.0, 0.0),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let terrain = sloped_terrain(33, 9);
    let mut surface_a = RoadSurfaceSystem::new(16.0);
    let mut surface_b = RoadSurfaceSystem::new(16.0);
    surface_a.compile_dirty(&graph, &terrain);
    surface_b.compile_dirty(&graph, &terrain);

    let sections_a = surface_a.compiled_sections().get(&edge_idx).unwrap();
    let sections_b = surface_b.compiled_sections().get(&edge_idx).unwrap();
    assert_eq!(sections_a, sections_b);
    for section in sections_a {
        let expected = 99.0;
        assert!((section.center_height_m - expected).abs() <= 0.001);
    }
}

#[test]
fn node_piece_classification_matches_surface_profiles() {
    let terrain = flat_terrain(64, 64);

    let mut pass_graph = RegionGraph::new();
    let pa = pass_graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
    let pb = pass_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let pc = pass_graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    pass_graph.add_edge(test_edge(
        pa,
        pb,
        vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    pass_graph.add_edge(test_edge(
        pb,
        pc,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut pass_surface = RoadSurfaceSystem::new(16.0);
    pass_surface.compile_dirty(&pass_graph, &terrain);
    assert!(
        pass_surface
            .compiled_visual_node_pieces()
            .get(&pb)
            .is_none()
    );

    let mut width_graph = RegionGraph::new();
    let wa = width_graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
    let wb = width_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let wc = width_graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    width_graph.add_edge(test_edge(
        wa,
        wb,
        vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    width_graph.add_edge(test_edge(
        wb,
        wc,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        14.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut width_surface = RoadSurfaceSystem::new(16.0);
    width_surface.compile_dirty(&width_graph, &terrain);
    assert!(
        width_surface
            .compiled_visual_node_pieces()
            .get(&wb)
            .is_none()
    );

    let mut junction_graph = RegionGraph::new();
    let ja = junction_graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
    let jb = junction_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let jc = junction_graph.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);
    junction_graph.add_edge(test_edge(
        ja,
        jb,
        vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    junction_graph.add_edge(test_edge(
        jb,
        jc,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 10.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut junction_surface = RoadSurfaceSystem::new(16.0);
    junction_surface.compile_dirty(&junction_graph, &terrain);
    assert_eq!(
        junction_surface
            .compiled_visual_node_pieces()
            .get(&jb)
            .unwrap()
            .kind,
        RoadSurfaceVisualNodePieceKind::Bend
    );

    let mut terminal_graph = RegionGraph::new();
    let ta = terminal_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let tb = terminal_graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    terminal_graph.add_edge(test_edge(
        ta,
        tb,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut terminal_surface = RoadSurfaceSystem::new(16.0);
    terminal_surface.compile_dirty(&terminal_graph, &terrain);
    assert_eq!(
        terminal_surface
            .compiled_visual_node_pieces()
            .get(&ta)
            .unwrap()
            .kind,
        RoadSurfaceVisualNodePieceKind::Terminal
    );
}

#[test]
fn bend_and_terminal_visual_pieces_compile_explicit_band_polygons() {
    let terrain = flat_terrain(64, 64);

    let mut bend_graph = RegionGraph::new();
    let bend_center = bend_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let bend_a = bend_graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let bend_b = bend_graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
    bend_graph.add_edge(test_edge(
        bend_center,
        bend_a,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    bend_graph.add_edge(test_edge(
        bend_center,
        bend_b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 20.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    bend_graph.rebuild_intersection_clips();
    let mut bend_surface = RoadSurfaceSystem::new(16.0);
    bend_surface.compile_dirty(&bend_graph, &terrain);
    let bend_piece = bend_surface
        .compiled_visual_node_pieces()
        .get(&bend_center)
        .unwrap();
    assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(bend_piece);
    assert_node_piece_has_curb_and_sidewalk_owners(bend_piece);
    assert!(!bend_piece.outer_boundary_loops.is_empty());
    assert!(!bend_piece.road_surface_polygons.is_empty());
    assert!(!bend_piece.sidewalk_surface_polygons.is_empty());
    assert!(
        bend_piece
            .outer_boundary_loops
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        bend_piece
            .road_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        bend_piece
            .sidewalk_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        point_inside_visual_polygons(&bend_piece.outer_boundary_loops, Vector2::new(3.0, 3.0)),
        "bend footprint must close the local round join between the two incident roadbeds"
    );
    assert!(
        point_inside_visual_polygons(&bend_piece.road_surface_polygons, Vector2::new(2.25, 2.25)),
        "bend asphalt must close its own local join instead of leaving a road-surface gap"
    );
    assert!(
        bend_piece
            .earthwork_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        bend_piece
            .render_earthwork_faces
            .iter()
            .all(|face| RoadSurfaceSystem::polygon_has_area_xz(&face.polygon.points_world))
    );

    let mut terminal_graph = RegionGraph::new();
    let terminal_center = terminal_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let terminal_end = terminal_graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let terminal_edge_idx = terminal_graph.add_edge(test_edge(
        terminal_center,
        terminal_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut terminal_surface = RoadSurfaceSystem::new(16.0);
    terminal_surface.compile_dirty(&terminal_graph, &terrain);
    let terminal_piece = terminal_surface
        .compiled_visual_node_pieces()
        .get(&terminal_center)
        .unwrap();
    assert_eq!(
        terminal_piece.kind,
        RoadSurfaceVisualNodePieceKind::Terminal
    );
    assert_eq!(terminal_piece.outer_boundary_loops.len(), 1);
    assert!(!terminal_piece.road_surface_polygons.is_empty());
    assert!(!terminal_piece.sidewalk_surface_polygons.is_empty());
    assert!(
        terminal_piece
            .road_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        terminal_piece
            .sidewalk_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    let terminal_span_piece = terminal_surface
        .compiled_visual_span_pieces()
        .get(&terminal_edge_idx)
        .unwrap();
    assert!(!terminal_span_piece.road_surface_polygons.is_empty());
    assert!(!terminal_piece.earthwork_surface_polygons.is_empty());
    assert!(!terminal_piece.earthwork_outer_boundary_loops.is_empty());
    assert!(!terminal_piece.render_earthwork_faces.is_empty());
    assert!(
        terminal_piece
            .earthwork_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        terminal_piece
            .render_earthwork_faces
            .iter()
            .all(|face| RoadSurfaceSystem::polygon_has_area_xz(&face.polygon.points_world))
    );
    assert_ne!(
        terminal_piece.earthwork_outer_boundary_loops,
        terminal_piece.outer_boundary_loops
    );
}

#[test]
fn flat_logged_curve_bend_keeps_footprint_covered_by_visible_top() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-17.539, 0.0, 12.635), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(57.560, 0.0, 4.157), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(119.799, 0.0, 82.841), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-17.539, 0.0, 12.635),
            Vector3::new(0.259, 0.0, 10.625),
            Vector3::new(30.126, 0.0, 7.254),
            Vector3::new(49.571, 0.0, 5.059),
            Vector3::new(57.560, 0.0, 4.157),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        vec![
            Vector3::new(57.560, 0.0, 4.157),
            Vector3::new(61.267, 0.0, 8.844),
            Vector3::new(71.839, 0.0, 22.209),
            Vector3::new(89.956, 0.0, 45.112),
            Vector3::new(105.986, 0.0, 65.379),
            Vector3::new(119.799, 0.0, 82.841),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .expect("logged curve should compile one bend node piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
    assert_node_piece_material_area_closes_footprint(piece, 0.25);
}

#[test]
fn logged_sixty_degree_bend_keeps_outer_corner_covered() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-131.350, 0.0, -31.215), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-21.350, 0.0, -31.215), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(13.650, 0.0, 29.406), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-131.350, 0.0, -31.215),
            Vector3::new(-21.350, 0.0, -31.215),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        vec![
            Vector3::new(-21.350, 0.0, -31.215),
            Vector3::new(13.650, 0.0, 29.406),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .expect("logged sixty-degree turn should compile one bend node piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert_all_top_shared_edges_are_height_continuous(piece);
    assert_node_piece_material_area_closes_footprint(piece, 0.25);
    let corner_point = Vector2::new(-18.850, -35.545);
    assert!(
        point_inside_visual_polygons(&piece.outer_boundary_loops, corner_point),
        "bend footprint must include the visible outer-corner join point"
    );
    assert!(
        point_inside_visual_polygons(&piece.road_surface_polygons, corner_point)
            || point_inside_visual_polygons(&piece.sidewalk_surface_polygons, corner_point),
        "bend visible top surface must cover the outer-corner join point"
    );
    let curb_curve_point = Vector2::new(-19.562, -34.311);
    let curb_height =
        node_region_height_at_kind(piece, RoadSurfaceBandKind::CurbOrShoulder, curb_curve_point)
            .expect("bend curved outer curb strip must be owned by curb/shoulder");
    assert!(
        (0.02..0.10).contains(&curb_height),
        "bend curved outer curb strip must keep the curb ramp height, point={curb_curve_point:?} height={curb_height:.4}"
    );
}

#[test]
fn logged_flat_sixty_degree_bend_uses_canonical_material_edge_heights() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-104.032, 0.0, -0.181), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-4.032, 0.0, -0.181), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(30.968, 0.0, 60.440), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-104.032, 0.0, -0.181),
            Vector3::new(-4.032, 0.0, -0.181),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        vec![
            Vector3::new(-4.032, 0.0, -0.181),
            Vector3::new(30.968, 0.0, 60.440),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .expect("flat-bend.log geometry should compile one Bend node piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert_all_top_shared_edges_are_height_continuous(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
    assert_node_piece_material_area_closes_footprint(piece, 0.001);
}

#[test]
fn logged_inside_bend_curb_anchor_stays_at_asphalt_height() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-82.047, 0.0, -9.463), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(28.584, 0.0, -15.027), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(71.960, 0.0, 47.832), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-82.047, 0.0, -9.463),
            Vector3::new(28.584, 0.0, -15.027),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        vec![
            Vector3::new(28.584, 0.0, -15.027),
            Vector3::new(71.960, 0.0, 47.832),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .expect("logged inside bend should compile");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);

    let inner_anchor_point = Vector2::new(26.635197, -11.424565);
    let (_, anchor_height) = piece
        .owned_regions
        .iter()
        .filter(|region| region.kind == RoadSurfaceBandKind::CurbOrShoulder)
        .flat_map(|region| region.polygon.points_world.iter())
        .filter_map(|point| {
            let distance_m = Vector2::new(
                point.x - inner_anchor_point.x,
                point.z - inner_anchor_point.y,
            )
            .length();
            (distance_m <= 0.01).then_some((distance_m, point.y))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .expect("logged inner bend must keep a curb vertex at the road-edge anchor");
    assert!(
        anchor_height <= 0.005,
        "inside bend curb skirt must anchor to asphalt height; point={inner_anchor_point:?} height={anchor_height:.4}"
    );
}

#[test]
fn logged_elevated_bend_keeps_non_road_edges_height_continuous() {
    let terrain = flat_terrain(1024, 1024);
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(362.721, 212.172, -543.419), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(354.920, 197.879, -455.205), NodeType::Junction);
    let c = graph.add_node(Vector3::new(389.920, 181.789, -394.583), NodeType::Junction);

    graph.add_edge(test_edge(
        a,
        bend,
        vec![
            Vector3::new(362.721, 212.172, -543.419),
            Vector3::new(354.920, 197.879, -455.205),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        c,
        vec![
            Vector3::new(354.920, 197.879, -455.205),
            Vector3::new(389.920, 181.789, -394.583),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .expect("logged elevated bend should compile");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_non_road_shared_edges_are_height_continuous(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);

    let seam_vertices = piece
        .road_surface_polygons
        .iter()
        .chain(piece.sidewalk_surface_polygons.iter())
        .flat_map(|polygon| polygon.points_world.iter())
        .collect::<Vec<_>>();
    for &edge_idx in graph.node_adjacency(bend) {
        let edge = graph.edge(edge_idx);
        let span_piece = surface
            .compiled_visual_span_pieces()
            .get(&edge_idx)
            .expect("incident edge must compile a span piece");
        let mouth = if graph.get_valid_node(edge.start_node) == bend {
            span_piece.start_mouth_profile.as_ref().unwrap()
        } else {
            span_piece.end_mouth_profile.as_ref().unwrap()
        };
        for (point_index, mouth_point) in mouth.boundary_points_world.iter().enumerate() {
            if point_index > 0 && point_index < mouth.boundary_points_world.len() - 1 {
                let before = mouth.bands[point_index - 1].kind;
                let after = mouth.bands[point_index].kind;
                if before == after {
                    continue;
                }
            }
            let Some(node_point) = seam_vertices.iter().min_by(|a, b| {
                let da = Vector2::new(a.x - mouth_point.x, a.z - mouth_point.z).length_squared();
                let db = Vector2::new(b.x - mouth_point.x, b.z - mouth_point.z).length_squared();
                da.total_cmp(&db)
            }) else {
                panic!("Bend emitted no material vertices");
            };
            let xz_error =
                Vector2::new(node_point.x - mouth_point.x, node_point.z - mouth_point.z).length();
            assert!(
                xz_error <= 0.004,
                "elevated Bend material vertex must preserve the span mouth XZ seam; mouth={mouth_point:?} closest={node_point:?} xz_error={xz_error:.4}"
            );
            assert!(
                (node_point.y - mouth_point.y).abs() <= 0.004,
                "elevated Bend mouth vertex must match the incident span height; mouth={mouth_point:?} closest={node_point:?}"
            );
        }
    }
}

#[test]
fn angled_terminal_keeps_curb_strip_covered_on_both_sides() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(40.0, 0.0, 5.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 5.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let terminal_piece = surface
        .compiled_visual_node_pieces()
        .get(&start)
        .expect("start node should compile a terminal piece");
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("terminal road should keep a visible span after terminal handoff");

    let travel = Vector2::new(40.0, 5.0).normalized();
    let lateral = RoadSurfaceSystem::left_normal_xz(travel);
    let center = Vector2::new(0.0, 0.0);
    for side in [-1.0, 1.0] {
        let curb_mid = center + lateral * side * 3.575;
        assert!(
            point_inside_visual_polygons(&terminal_piece.sidewalk_surface_polygons, curb_mid),
            "angled terminal curb strip must be owned by sidewalk/curb surface on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&terminal_piece.road_surface_polygons, curb_mid),
            "terminal curb strip must not be owned by asphalt on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&span_piece.sidewalk_surface_polygons, curb_mid),
            "terminal curb strip must not be duplicated by the span on side {side}; point={curb_mid:?}"
        );
    }
}

#[test]
fn steep_standard_terminal_compiles_visible_end_bands() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let points = vec![
        Vector3::new(178.256, 203.772, -564.088),
        Vector3::new(178.174, 203.724, -563.275),
        Vector3::new(178.103, 203.674, -562.575),
        Vector3::new(178.045, 203.619, -561.999),
        Vector3::new(177.978, 203.551, -561.337),
        Vector3::new(177.903, 203.462, -560.595),
        Vector3::new(177.820, 203.350, -559.774),
        Vector3::new(177.729, 203.220, -558.879),
        Vector3::new(177.656, 203.082, -558.161),
        Vector3::new(177.606, 202.946, -557.661),
        Vector3::new(177.554, 202.818, -557.143),
        Vector3::new(170.931, 183.624, -491.661),
    ];
    let start = graph.add_node(*points.first().unwrap(), NodeType::Junction);
    let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let start_piece = surface
        .compiled_visual_node_pieces()
        .get(&start)
        .expect("steep start terminal should compile a visible node piece");
    let end_piece = surface
        .compiled_visual_node_pieces()
        .get(&end)
        .expect("steep end terminal should compile a visible node piece");

    assert_eq!(start_piece.kind, RoadSurfaceVisualNodePieceKind::Terminal);
    assert_eq!(end_piece.kind, RoadSurfaceVisualNodePieceKind::Terminal);
    assert!(!start_piece.sidewalk_surface_polygons.is_empty());
    assert!(!end_piece.sidewalk_surface_polygons.is_empty());
}

#[test]
fn span_visual_pieces_compile_explicit_band_polygons() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .unwrap();
    assert!(!span_piece.outer_boundary_loops.is_empty());
    assert!(!span_piece.road_surface_polygons.is_empty());
    assert!(!span_piece.sidewalk_surface_polygons.is_empty());
    assert!(
        span_piece
            .road_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .sidewalk_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(!span_piece.earthwork_surface_polygons.is_empty());
    assert!(!span_piece.earthwork_outer_boundary_loops.is_empty());
    assert!(!span_piece.render_earthwork_faces.is_empty());
    assert!(
        span_piece
            .earthwork_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .render_earthwork_faces
            .iter()
            .all(|face| RoadSurfaceSystem::polygon_has_area_xz(&face.polygon.points_world))
    );
    assert_ne!(
        span_piece.earthwork_outer_boundary_loops,
        span_piece.outer_boundary_loops
    );
}

#[test]
fn span_earthwork_outer_loops_stay_outside_paved_footprint() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, -24.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("standard edge should compile a visual span piece");
    let max_inner_abs_x = span_piece
        .outer_boundary_loops
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
        .map(|point| point.x.abs())
        .fold(0.0, f32::max);
    let min_outer_abs_x = span_piece
        .earthwork_outer_boundary_loops
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
        .map(|point| point.x.abs())
        .fold(f32::INFINITY, f32::min);
    assert!(
        min_outer_abs_x >= max_inner_abs_x + 0.5,
        "expected span earthwork tie-in to stay outside the paved footprint, got min_outer_abs_x={min_outer_abs_x:.3} max_inner_abs_x={max_inner_abs_x:.3}"
    );
}

#[test]
fn terrain_clip_polygons_include_standard_grounded_footprints() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -24.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let clip_polygons =
        surface.terrain_clip_polygons_for_world_bounds(&graph, -16.0, -32.0, 16.0, 32.0);

    assert!(
        !clip_polygons.is_empty(),
        "expected grounded standard road footprint polygons to clip terrain topology"
    );
    assert!(
        clip_polygons
            .iter()
            .flat_map(|polygon| polygon.points_world.iter())
            .any(|point| point.x.abs() > 5.0),
        "expected terrain clip polygons to include the full sidewalk / shoulder footprint"
    );
    assert!(
        clip_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world)),
        "expected every terrain clip cutter to be a valid road footprint polygon"
    );
    let expected_outer_boundary_loop_count: usize = surface
        .compiled_visual_span_pieces()
        .values()
        .map(|piece| piece.outer_boundary_loops.len())
        .sum::<usize>()
        + surface
            .compiled_visual_node_pieces()
            .values()
            .map(|piece| piece.outer_boundary_loops.len())
            .sum::<usize>();
    assert!(
        clip_polygons.len() <= expected_outer_boundary_loop_count,
        "expected terrain clip cutters to be the boolean-unioned piece footprint, got {} cutters for {} raw outer loops",
        clip_polygons.len(),
        expected_outer_boundary_loop_count
    );
}

#[test]
fn terrain_clip_polygons_are_unioned_before_cdt_for_arbitrary_multiway_nodes() {
    let terrain = flat_terrain(257, 257);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    for angle_degrees in [0.0_f32, 23.0, 61.0, 137.0, 211.0, 304.0] {
        let angle = angle_degrees.to_radians();
        let endpoint = Vector3::new(angle.cos() * 64.0, 0.0, angle.sin() * 64.0);
        let node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            node,
            vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
            14.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let clip_polygons =
        surface.terrain_clip_polygons_for_world_bounds(&graph, -96.0, -96.0, 96.0, 96.0);
    assert!(
        !clip_polygons.is_empty(),
        "expected arbitrary multiway node to produce terrain clip polygons"
    );

    let road_loops = clip_polygons
        .iter()
        .enumerate()
        .map(|(index, polygon)| {
            TerrainCdtRoadLoop::new(
                index as u64,
                0,
                polygon
                    .points_world
                    .iter()
                    .map(|point| {
                        TerrainCdtVertex::new(f64::from(point.x), point.y, f64::from(point.z))
                    })
                    .collect(),
            )
        })
        .collect();
    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        TerrainCdtPatch::new(-96.0, -96.0, 96.0, 96.0, [0.0; 4]),
        road_loops,
        Vec::new(),
    ))
    .expect("unioned terrain clip footprint must be accepted by the terrain CDT");

    assert_eq!(
        mesh.stats.invalid_constraint_edges, 0,
        "terrain CDT must not see crossing constraints from arbitrary-angle piece loops"
    );
}

#[test]
fn road_locked_terrain_patches_are_bounded_to_visible_footprint() {
    let terrain = flat_terrain(257, 257);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -48.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 48.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -48.0), Vector3::new(0.0, 0.0, 48.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let mut footprint_min_x = f32::MAX;
    let mut footprint_max_x = f32::MIN;
    let mut footprint_min_z = f32::MAX;
    let mut footprint_max_z = f32::MIN;
    for point in surface
        .compiled_visual_span_pieces()
        .values()
        .flat_map(|piece| piece.outer_boundary_loops.iter())
        .chain(
            surface
                .compiled_visual_node_pieces()
                .values()
                .flat_map(|piece| piece.outer_boundary_loops.iter()),
        )
        .flat_map(|polygon| polygon.points_world.iter())
    {
        footprint_min_x = footprint_min_x.min(point.x);
        footprint_max_x = footprint_max_x.max(point.x);
        footprint_min_z = footprint_min_z.min(point.z);
        footprint_max_z = footprint_max_z.max(point.z);
    }

    let keys = surface.terrain_render_patch_keys_with_visible_road(&terrain);
    assert!(!keys.is_empty());
    assert!(
        keys.len() < terrain.render_patch_cols() * terrain.render_patch_rows() / 8,
        "road-locked render patches must stay local to the visible road footprint"
    );
    for (patch_x, patch_z) in keys {
        let patch = terrain.visual_patch_snapshot(patch_x, patch_z).unwrap();
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        assert!(
            patch.world_origin_x <= footprint_max_x
                && patch_max_x >= footprint_min_x
                && patch.world_origin_z <= footprint_max_z
                && patch_max_z >= footprint_min_z,
            "road-locked patch ({patch_x}, {patch_z}) must overlap the road footprint, not only the earthwork envelope"
        );
    }
}

#[test]
fn terrain_clip_polygons_skip_bridge_midspans() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 8.0, -24.0), Vector3::new(0.0, 8.0, 24.0)],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let clip_polygons =
        surface.terrain_clip_polygons_for_world_bounds(&graph, -16.0, -32.0, 16.0, 32.0);

    assert!(
        clip_polygons.is_empty(),
        "bridge midspans must not cut terrain topology like grounded standard roads"
    );
}

#[test]
fn earthwork_face_classification_distinguishes_slopes_from_walls() {
    assert_eq!(
        RoadSurfaceSystem::classify_earthwork_face_kind(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(2.0, 0.5, 0.0),
            Vector3::new(1.0, 0.5, 0.0),
        ),
        RoadSurfaceEarthworkFaceKind::Slope
    );
    assert_eq!(
        RoadSurfaceSystem::classify_earthwork_face_kind(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(1.1, 3.0, 0.0),
            Vector3::new(0.1, 3.0, 0.0),
        ),
        RoadSurfaceEarthworkFaceKind::RetainingWall
    );
}

#[test]
fn visual_node_pieces_are_deterministic_for_multi_arm_nodes() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    let up = graph.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);
    graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        up,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 10.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();

    let terrain = flat_terrain(64, 64);
    let mut surface_a = RoadSurfaceSystem::new(16.0);
    let mut surface_b = RoadSurfaceSystem::new(16.0);
    surface_a.compile_dirty(&graph, &terrain);
    surface_b.compile_dirty(&graph, &terrain);

    let piece_a = surface_a
        .compiled_visual_node_pieces()
        .get(&center)
        .unwrap();
    let piece_b = surface_b
        .compiled_visual_node_pieces()
        .get(&center)
        .unwrap();
    assert_eq!(piece_a.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert_eq!(piece_a, piece_b);
    assert!(
        !piece_a.outer_boundary_loops.is_empty(),
        "expected explicit visual node pieces to expose deterministic outer boundaries"
    );
    assert!(
        !piece_a.road_surface_polygons.is_empty(),
        "expected explicit JunctionN builder to emit road-owned polygons"
    );
    assert!(
        !piece_a.sidewalk_surface_polygons.is_empty(),
        "expected explicit JunctionN builder to emit overlay-owned sidewalk polygons"
    );
    assert_material_triangles_do_not_overlap(piece_a);
}

#[test]
fn oblique_t_junction_compiles_solid_cdt_owned_surface() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let oblique = graph.add_node(Vector3::new(12.0, 0.0, 20.784609), NodeType::Junction);
    graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        oblique,
        vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(12.0, 0.0, 20.784609),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("60-degree T junction must compile an explicit JunctionN piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(piece);
    assert!(
        piece
            .road_surface_polygons
            .iter()
            .chain(piece.sidewalk_surface_polygons.iter())
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world)),
        "overlay-owned JunctionN polygons must be non-degenerate"
    );
    assert_material_triangles_do_not_overlap(piece);
}

#[test]
fn editor_sized_60_degree_t_junction_width_7_compiles_node_surface() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-87.843, 0.0, -11.753), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-50.197, 0.0, -11.753), NodeType::Junction);
    let right = graph.add_node(Vector3::new(32.157, 0.0, -11.753), NodeType::Junction);
    let oblique = graph.add_node(Vector3::new(-20.197, 0.0, 40.209), NodeType::Junction);
    graph.add_edge(test_edge(
        left,
        center,
        vec![
            Vector3::new(-87.843, 0.0, -11.753),
            Vector3::new(-50.197, 0.0, -11.753),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![
            Vector3::new(-50.197, 0.0, -11.753),
            Vector3::new(32.157, 0.0, -11.753),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        oblique,
        vec![
            Vector3::new(-50.197, 0.0, -11.753),
            Vector3::new(-20.197, 0.0, 40.209),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(128, 128);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("editor-sized 60-degree T junction must compile a JunctionN piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(piece);
    assert_material_triangles_do_not_overlap(piece);

    let raw_clip_sources = surface
        .compiled_visual_span_pieces()
        .values()
        .flat_map(|piece| piece.outer_boundary_loops.iter().cloned())
        .chain(
            surface
                .compiled_visual_node_pieces()
                .values()
                .flat_map(|piece| piece.outer_boundary_loops.iter().cloned()),
        )
        .collect::<Vec<_>>();
    assert!(
        !raw_clip_sources.is_empty(),
        "editor-sized 60-degree T junction must have raw terrain clip source loops"
    );
    let unioned_clip_sources = RoadSurfaceSystem::union_terrain_clip_polygons(&raw_clip_sources);
    assert!(
        !unioned_clip_sources.is_empty(),
        "editor-sized 60-degree T junction raw clip loops must survive deterministic union"
    );

    let clip_polygons =
        surface.terrain_clip_polygons_for_world_bounds(&graph, -128.0, -32.0, 64.0, 64.0);
    assert!(
        !clip_polygons.is_empty(),
        "editor-sized 60-degree T junction must export terrain clip cutters"
    );
    let road_loops = clip_polygons
        .iter()
        .enumerate()
        .map(|(index, polygon)| {
            TerrainCdtRoadLoop::new(
                index as u64,
                0,
                polygon
                    .points_world
                    .iter()
                    .map(|point| {
                        TerrainCdtVertex::new(f64::from(point.x), point.y, f64::from(point.z))
                    })
                    .collect(),
            )
        })
        .collect();
    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        TerrainCdtPatch::new(-128.0, -32.0, 64.0, 64.0, [0.0; 4]),
        road_loops,
        Vec::new(),
    ))
    .expect("editor-sized 60-degree T terrain cutters must be accepted by terrain CDT");
    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
}

#[test]
fn logged_flat_three_way_oblique_junction_exports_noded_outer_boundary() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-60.311, 0.0, -3.324), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-12.773, 0.0, -3.324), NodeType::Junction);
    let east = graph.add_node(Vector3::new(79.689, 0.0, -3.324), NodeType::Junction);
    let oblique = graph.add_node(Vector3::new(22.227, 0.0, 57.298), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-60.311, 0.0, -3.324),
            Vector3::new(-12.773, 0.0, -3.324),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        oblique,
        vec![
            Vector3::new(-12.773, 0.0, -3.324),
            Vector3::new(22.227, 0.0, 57.298),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![
            Vector3::new(-12.773, 0.0, -3.324),
            Vector3::new(79.689, 0.0, -3.324),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(192, 192);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("logged flat 3-way oblique junction must compile a JunctionN piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_outer_boundary_edges_are_noded_by_visible_top(piece);
    assert_material_triangles_do_not_overlap(piece);
}

#[test]
fn logged_flat_oblique_t_junction_compiles_node_surface() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-140.162, 0.0, -60.230), NodeType::Junction);
    let north = graph.add_node(Vector3::new(-75.827, 0.0, 89.838), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-57.710, 0.0, 22.223), NodeType::Junction);
    let east = graph.add_node(Vector3::new(50.757, 0.0, 130.689), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-140.162, 0.0, -60.230),
            Vector3::new(-57.710, 0.0, 22.223),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        north,
        vec![
            Vector3::new(-57.710, 0.0, 22.223),
            Vector3::new(-75.827, 0.0, 89.838),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![
            Vector3::new(-57.710, 0.0, 22.223),
            Vector3::new(50.757, 0.0, 130.689),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(128, 128);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let incidents = [
        super::IncidentSurfaceEdge {
            edge_idx: 0,
            side: super::IncidentEdgeSide::End,
            direction_xz: surface
                .compiled_visual_span_pieces()
                .get(&0)
                .and_then(|piece| piece.end_mouth_profile.as_ref())
                .expect("west span must expose an end mouth")
                .inward_direction_xz,
        },
        super::IncidentSurfaceEdge {
            edge_idx: 1,
            side: super::IncidentEdgeSide::Start,
            direction_xz: surface
                .compiled_visual_span_pieces()
                .get(&1)
                .and_then(|piece| piece.start_mouth_profile.as_ref())
                .expect("north span must expose a start mouth")
                .inward_direction_xz,
        },
        super::IncidentSurfaceEdge {
            edge_idx: 2,
            side: super::IncidentEdgeSide::Start,
            direction_xz: surface
                .compiled_visual_span_pieces()
                .get(&2)
                .and_then(|piece| piece.start_mouth_profile.as_ref())
                .expect("east span must expose a start mouth")
                .inward_direction_xz,
        },
    ];
    let mut mouths = incidents
        .iter()
        .map(|incident| {
            let span_piece = surface
                .compiled_visual_span_pieces()
                .get(&incident.edge_idx)
                .expect("incident span piece must be compiled");
            let profile = match incident.side {
                super::IncidentEdgeSide::Start => span_piece.start_mouth_profile.clone(),
                super::IncidentEdgeSide::End => span_piece.end_mouth_profile.clone(),
            }
            .expect("incident span piece must expose a mouth profile");
            let sections = surface
                .compiled_sections()
                .get(&incident.edge_idx)
                .expect("incident sections must be compiled");
            let section = match incident.side {
                super::IncidentEdgeSide::Start => sections.first(),
                super::IncidentEdgeSide::End => sections.last(),
            }
            .expect("incident endpoint section must exist");
            let endpoint_profile =
                RoadSurfaceSystem::build_mouth_profile_from_section(section, incident.side)
                    .expect("incident endpoint profile must compile");
            super::OrderedIncidentPieceMouth {
                profile,
                endpoint_profile,
                direction_angle_ccw: {
                    let angle = incident.direction_xz.y.atan2(incident.direction_xz.x);
                    if angle < 0.0 {
                        angle + std::f32::consts::TAU
                    } else {
                        angle
                    }
                },
                direction_xz: incident.direction_xz,
                edge_idx: incident.edge_idx,
                side: incident.side,
            }
        })
        .collect::<Vec<_>>();
    mouths.sort_by(|a, b| {
        a.direction_angle_ccw
            .total_cmp(&b.direction_angle_ccw)
            .then(a.edge_idx.cmp(&b.edge_idx))
            .then(a.side.cmp(&b.side))
    });
    let input = RoadSurfaceSystem::build_node_arrangement_input_from_mouths(
        center,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &mouths,
    )
    .expect("logged flat oblique T input must compile");
    let rails = RoadSurfaceSystem::build_node_rail_contours_from_input(&input)
        .expect("logged flat oblique T rails must compile");
    let ownership = RoadSurfaceSystem::build_node_boolean_ownership_from_rails(&rails)
        .expect("logged flat oblique T ownership must compile");
    let heights = RoadSurfaceSystem::build_node_height_solution_from_ownership(&input, &ownership)
        .expect("logged flat oblique T heights must compile");
    RoadSurfaceSystem::build_node_triangulation_from_height_solution(&heights)
        .expect("logged flat oblique T height regions must triangulate");

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("logged flat oblique T junction must compile a JunctionN piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_material_triangles_do_not_overlap(piece);
}

#[test]
fn logged_flat_oblique_four_way_compiles_node_surface_after_new_incident_road() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-168.693, 0.0, 22.598), NodeType::Junction);
    let east = graph.add_node(Vector3::new(-9.454, 0.0, 18.003), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-125.850, 0.0, 21.362), NodeType::Junction);
    let north = graph.add_node(Vector3::new(-83.868, 0.0, 89.461), NodeType::Junction);
    let south = graph.add_node(Vector3::new(-143.870, 0.0, -84.460), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-168.693, 0.0, 22.598),
            Vector3::new(-125.850, 0.0, 21.362),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        north,
        vec![
            Vector3::new(-125.850, 0.0, 21.362),
            Vector3::new(-83.868, 0.0, 89.461),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![
            Vector3::new(-125.850, 0.0, 21.362),
            Vector3::new(-9.454, 0.0, 18.003),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        south,
        center,
        vec![
            Vector3::new(-143.870, 0.0, -84.460),
            Vector3::new(-125.850, 0.0, 21.362),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(512, 512);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface.compiled_visual_node_pieces().get(&center).expect(
        "logged flat oblique four-way must keep the center JunctionN after adding the fourth road",
    );
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_material_triangles_do_not_overlap(piece);
}

#[test]
fn arbitrary_six_way_junction_keeps_visible_ownership_disjoint() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    for angle_degrees in [0.0_f32, 23.0, 61.0, 137.0, 211.0, 304.0] {
        let angle = angle_degrees.to_radians();
        let endpoint = Vector3::new(angle.cos() * 96.0, 0.0, angle.sin() * 96.0);
        let node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            node,
            vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(192, 192);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("arbitrary six-way node must compile one JunctionN piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);

    let footprint_area: f32 = piece.outer_boundary_loops.iter().map(polygon_area_m2).sum();
    let asphalt_area: f32 = piece
        .road_surface_polygons
        .iter()
        .map(polygon_triangle_area_m2)
        .sum();
    let non_road_area: f32 = piece
        .sidewalk_surface_polygons
        .iter()
        .map(polygon_triangle_area_m2)
        .sum();
    assert!(
        (footprint_area - asphalt_area - non_road_area).abs() <= 0.25,
        "arbitrary JunctionN ownership must close the footprint without overlapping materials; footprint={footprint_area:.3} asphalt={asphalt_area:.3} non_road={non_road_area:.3}"
    );
    assert_material_triangles_do_not_overlap(piece);
}

#[test]
fn arbitrary_five_way_junction_uses_conflict_bounded_footprint() {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(2.668, 0.0, 10.799);
    let center = graph.add_node(center_pos, NodeType::Junction);
    for endpoint in [
        Vector3::new(-58.540, 0.0, 6.220),
        Vector3::new(115.507, 0.0, 19.240),
        Vector3::new(96.186, 0.0, 60.070),
        Vector3::new(35.647, 0.0, -130.899),
        Vector3::new(-27.212, 0.0, 50.632),
    ] {
        let node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            node,
            vec![center_pos, endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("arbitrary five-way node must compile one conflict-bounded JunctionN piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);

    let max_expected_radius = graph
        .node_adjacency(center)
        .iter()
        .map(|&edge_idx| {
            let edge = graph.edge(edge_idx);
            let clip = if graph.get_valid_node(edge.start_node) == center {
                edge.start_clip
            } else {
                edge.end_clip
            };
            clip.max(RoadSurfaceSystem::visual_node_handoff_limit_m(edge))
                + RoadSurfaceSystem::visual_roadbed_half_width_m(edge)
                + 0.25
        })
        .fold(0.0_f32, f32::max);
    for point in piece
        .outer_boundary_loops
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
    {
        let radius = Vector2::new(point.x - center_pos.x, point.z - center_pos.z).length();
        assert!(
            radius <= max_expected_radius,
            "visual JunctionN footprint must stay inside the conflict-bounded handoff; point={point:?} radius={radius:.3} max={max_expected_radius:.3}"
        );
    }
    assert_material_triangles_do_not_overlap(piece);
}

#[test]
fn dirty_node_recompile_refreshes_incident_span_sections_for_new_junction() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let left_edge = graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let up = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    let up_edge = graph.add_edge(test_edge(
        center,
        up,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    surface.mark_node_dirty(&graph, center);
    surface.mark_node_dirty(&graph, up);
    surface.mark_edge_dirty(&graph, up_edge);
    surface.compile_dirty(&graph, &terrain);

    let edge = graph.edge(left_edge);
    let total_length: f32 = edge
        .geometry
        .windows(2)
        .map(|pair| pair[0].distance_to(pair[1]))
        .sum();
    let expected_handoff_s = RoadSurfaceSystem::visual_end_handoff_s_m(edge, total_length);
    let sections = surface.compiled_sections().get(&left_edge).unwrap();
    assert!(
        sections
            .iter()
            .any(|section| (section.s_m - expected_handoff_s).abs() <= SAMPLE_EPSILON_M),
        "dirty node recompilation must refresh incident span sections at the new visual handoff; expected_s={expected_handoff_s:.3} sections={:?}",
        sections
            .iter()
            .map(|section| section.s_m)
            .collect::<Vec<_>>()
    );
}

#[test]
fn dirty_recompile_marks_chunks_for_expanded_arbitrary_node_piece() {
    let terrain = flat_terrain(192, 192);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    for angle_degrees in [35.0_f32, 158.0, 276.0] {
        let angle = angle_degrees.to_radians();
        let endpoint = Vector3::new(angle.cos() * 88.0, 0.0, angle.sin() * 88.0);
        let node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            node,
            vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(4.0);
    surface.compile_dirty(&graph, &terrain);

    let angle = 318.0_f32.to_radians();
    let endpoint = Vector3::new(angle.cos() * 88.0, 0.0, angle.sin() * 88.0);
    let new_node = graph.add_node(endpoint, NodeType::Junction);
    let new_edge = graph.add_edge(test_edge(
        center,
        new_node,
        vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    surface.mark_node_dirty(&graph, center);
    surface.mark_node_dirty(&graph, new_node);
    for &edge_idx in graph.node_adjacency(center) {
        surface.mark_edge_dirty(&graph, edge_idx);
    }
    surface.mark_edge_dirty(&graph, new_edge);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("expanded arbitrary junction must have a compiled node piece");
    let (min, max) = surface
        .visual_node_piece_bounds(piece, ChunkCacheKind::Surface)
        .expect("expanded arbitrary junction must have surface bounds");

    for chunk in surface.bounds_to_chunk_keys(min, max) {
        let entry = surface
            .surface_chunk_cache
            .get(&chunk)
            .unwrap_or_else(|| panic!("expected rebuilt surface chunk {chunk:?}"));
        assert!(
            entry.node_ids.contains(&center),
            "surface chunk {chunk:?} must include the expanded junction node piece"
        );
    }
}

#[test]
fn dirty_recompile_removes_node_from_old_chunks_after_topology_shrink() {
    let terrain = flat_terrain(192, 192);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let west = graph.add_node(Vector3::new(-64.0, 0.0, 0.0), NodeType::Junction);
    let east = graph.add_node(Vector3::new(64.0, 0.0, 0.0), NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 0.0, 64.0), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![Vector3::new(-64.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(64.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let removed_edge = graph.add_edge(test_edge(
        center,
        north,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 64.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(2.0);
    surface.compile_dirty(&graph, &terrain);
    let old_node_chunks = surface
        .surface_node_chunks
        .get(&center)
        .expect("three-way node must own chunks before shrink")
        .clone();
    assert!(
        old_node_chunks.len() > 1,
        "test requires node coverage wide enough to prove stale chunk removal"
    );

    graph.edges[removed_edge].deleted = true;
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();
    surface.mark_edge_dirty(&graph, removed_edge);
    surface.mark_node_dirty(&graph, center);
    surface.compile_dirty(&graph, &terrain);

    let new_node_chunks = surface
        .surface_node_chunks
        .get(&center)
        .cloned()
        .unwrap_or_default();
    let removed_chunks: Vec<SurfaceChunkKey> = old_node_chunks
        .into_iter()
        .filter(|chunk| !new_node_chunks.contains(chunk))
        .collect();
    assert!(
        !removed_chunks.is_empty(),
        "topology shrink must remove at least one old node-owned chunk"
    );
    for chunk in removed_chunks {
        if let Some(entry) = surface.surface_chunk_cache.get(&chunk) {
            assert!(
                !entry.node_ids.contains(&center),
                "stale node contributor remained in removed chunk {chunk:?}"
            );
        }
    }
}

#[test]
fn junction_node_non_road_surface_is_footprint_minus_asphalt() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let up = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        up,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface.compiled_visual_node_pieces().get(&center).unwrap();
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    let footprint_area: f32 = piece.outer_boundary_loops.iter().map(polygon_area_m2).sum();
    let asphalt_area: f32 = piece
        .road_surface_polygons
        .iter()
        .map(polygon_triangle_area_m2)
        .sum();
    let non_road_area: f32 = piece
        .sidewalk_surface_polygons
        .iter()
        .map(polygon_triangle_area_m2)
        .sum();

    assert!(
        non_road_area > 0.0,
        "JunctionN must emit non-road node surface polygons"
    );
    let max_non_road_height = piece
        .sidewalk_surface_polygons
        .iter()
        .flat_map(|polygon| polygon.triangles_world.iter())
        .flat_map(|triangle| triangle.iter())
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        max_non_road_height >= CURB_STEP_HEIGHT_M - 0.001,
        "node non-road surfaces must sample curb/sidewalk band heights instead of flattened full-roadbed height; max_non_road_height={max_non_road_height:.3}"
    );
    assert!(
        (footprint_area - asphalt_area - non_road_area).abs() <= 0.05,
        "node non-road ownership must be exactly the resolved footprint minus asphalt; footprint={footprint_area:.3} asphalt={asphalt_area:.3} non_road={non_road_area:.3}"
    );
    assert_material_triangles_do_not_overlap(piece);
}

#[test]
fn elevated_four_way_junction_keeps_span_mouth_vertices_seamless() {
    let terrain = planar_world_terrain(192, 192, 1.0, 150.0, 0.045, -0.018);
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(
        0.0,
        terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE,
        0.0,
    );
    let center = graph.add_node(center_pos, NodeType::Junction);
    for endpoint_xz in [
        Vector2::new(-72.0, 0.0),
        Vector2::new(72.0, 0.0),
        Vector2::new(0.0, -72.0),
        Vector2::new(0.0, 72.0),
    ] {
        let endpoint_pos = Vector3::new(
            endpoint_xz.x,
            terrain.sample_height_world(endpoint_xz.x, endpoint_xz.y) * crate::config::HEIGHT_SCALE,
            endpoint_xz.y,
        );
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let points = if endpoint_xz.x < 0.0 || endpoint_xz.y < 0.0 {
            grounded_polyline_points_from_terrain(&terrain, endpoint_xz, Vector2::ZERO, 24)
        } else {
            grounded_polyline_points_from_terrain(&terrain, Vector2::ZERO, endpoint_xz, 24)
        };
        if endpoint_xz.x < 0.0 || endpoint_xz.y < 0.0 {
            graph.add_edge(test_edge(
                endpoint,
                center,
                points,
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        } else {
            graph.add_edge(test_edge(
                center,
                endpoint,
                points,
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        }
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("elevated 4-way node must compile one JunctionN piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_non_road_shared_edges_are_height_continuous(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
    let seam_vertices = piece
        .road_surface_polygons
        .iter()
        .chain(piece.sidewalk_surface_polygons.iter())
        .flat_map(|polygon| polygon.points_world.iter())
        .collect::<Vec<_>>();

    for &edge_idx in graph.node_adjacency(center) {
        let edge = graph.edge(edge_idx);
        let span_piece = surface
            .compiled_visual_span_pieces()
            .get(&edge_idx)
            .unwrap();
        let mouth = if graph.get_valid_node(edge.start_node) == center {
            span_piece.start_mouth_profile.as_ref().unwrap()
        } else {
            span_piece.end_mouth_profile.as_ref().unwrap()
        };
        for (point_index, mouth_point) in mouth.boundary_points_world.iter().enumerate() {
            if point_index > 0 && point_index < mouth.boundary_points_world.len() - 1 {
                let before = mouth.bands[point_index - 1].kind;
                let after = mouth.bands[point_index].kind;
                if before == after {
                    continue;
                }
            }
            let Some(node_point) = seam_vertices.iter().min_by(|a, b| {
                let da = Vector2::new(a.x - mouth_point.x, a.z - mouth_point.z).length_squared();
                let db = Vector2::new(b.x - mouth_point.x, b.z - mouth_point.z).length_squared();
                da.total_cmp(&db)
            }) else {
                panic!("JunctionN emitted no material vertices");
            };
            let xz_error =
                Vector2::new(node_point.x - mouth_point.x, node_point.z - mouth_point.z).length();
            assert!(
                xz_error <= 0.004,
                "node material vertex must preserve the span mouth XZ seam; mouth={mouth_point:?} closest={node_point:?} xz_error={xz_error:.4}"
            );
            assert!(
                (node_point.y - mouth_point.y).abs() <= 0.004,
                "elevated JunctionN mouth vertex must match the incident span height; mouth={mouth_point:?} closest={node_point:?}"
            );
        }
    }

    let debug_edges = graph.node_adjacency(center).to_vec();
    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &debug_edges);
    assert!(dump.contains("\"kind\": \"JunctionN\""));
    assert!(dump.contains("\"sidewalk_topology\""));
    assert!(dump.contains("\"band_ownership\""));
    assert!(dump.contains("\"height_owner\""));
    assert!(dump.contains("\"seam_constraints\""));
    assert!(dump.contains("\"material_footprint_coverage\""));
    assert!(dump.contains("\"outer_boundary_top_match\""));
    assert!(dump.contains("\"mouth_seams\""));
    assert!(dump.contains("\"earthwork_face_top_match\""));
}

#[test]
fn elevated_junction_uses_endpoint_heights_for_node_side_vertices() {
    let terrain = flat_terrain(192, 192);
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(0.0, 0.0, 0.0);
    let center = graph.add_node(center_pos, NodeType::Junction);
    for (endpoint_pos, starts_at_center) in [
        (Vector3::new(-80.0, 80.0, 0.0), false),
        (Vector3::new(80.0, -80.0, 0.0), true),
        (Vector3::new(0.0, 64.0, -80.0), false),
        (Vector3::new(0.0, -64.0, 80.0), true),
    ] {
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let (start, end, points) = if starts_at_center {
            (center, endpoint, vec![center_pos, endpoint_pos])
        } else {
            (endpoint, center, vec![endpoint_pos, center_pos])
        };
        graph.add_edge(test_edge(
            start,
            end,
            points,
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("steep 4-way node must compile one JunctionN piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);

    let mut max_mouth_abs_y = 0.0_f32;
    for &edge_idx in graph.node_adjacency(center) {
        let edge = graph.edge(edge_idx);
        let span_piece = surface
            .compiled_visual_span_pieces()
            .get(&edge_idx)
            .expect("incident edge must compile a span piece");
        let mouth = if graph.get_valid_node(edge.start_node) == center {
            span_piece.start_mouth_profile.as_ref().unwrap()
        } else {
            span_piece.end_mouth_profile.as_ref().unwrap()
        };
        for point in &mouth.boundary_points_world {
            max_mouth_abs_y = max_mouth_abs_y.max(point.y.abs());
        }
    }
    assert!(
        max_mouth_abs_y >= 3.0,
        "test setup must put visible throats far above or below the endpoint; max_mouth_abs_y={max_mouth_abs_y:.3}"
    );

    let (max_slope, max_kind, max_owner_index, max_start, max_end) =
        max_visual_triangle_slope_ratio(piece);
    assert!(
        max_slope <= 1.35,
        "JunctionN top-surface triangles must follow endpoint-to-throat ramps instead of near-vertical projected throat plateaus; bounded curb-step diagonals are allowed, max_slope={max_slope:.3} kind={max_kind:?} owner={max_owner_index} edge=({max_start:?}, {max_end:?})"
    );
}

#[test]
fn elevated_three_way_junction_keeps_outer_boundary_on_visible_top() {
    let terrain = planar_world_terrain(192, 192, 1.0, 150.0, 0.045, -0.018);
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(
        0.0,
        terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE,
        0.0,
    );
    let center = graph.add_node(center_pos, NodeType::Junction);
    for endpoint_xz in [
        Vector2::new(-72.0, 0.0),
        Vector2::new(72.0, 0.0),
        Vector2::new(0.0, 72.0),
    ] {
        let endpoint_pos = Vector3::new(
            endpoint_xz.x,
            terrain.sample_height_world(endpoint_xz.x, endpoint_xz.y) * crate::config::HEIGHT_SCALE,
            endpoint_xz.y,
        );
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let points = if endpoint_xz.x < 0.0 || endpoint_xz.y < 0.0 {
            grounded_polyline_points_from_terrain(&terrain, endpoint_xz, Vector2::ZERO, 24)
        } else {
            grounded_polyline_points_from_terrain(&terrain, Vector2::ZERO, endpoint_xz, 24)
        };
        if endpoint_xz.x < 0.0 || endpoint_xz.y < 0.0 {
            graph.add_edge(test_edge(
                endpoint,
                center,
                points,
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        } else {
            graph.add_edge(test_edge(
                center,
                endpoint,
                points,
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        }
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("elevated 3-way node must compile one JunctionN piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_non_road_shared_edges_are_height_continuous(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
}

#[test]
fn skewed_elevated_four_way_junction_compiles_visible_center_surface() {
    let terrain = planar_world_terrain(256, 256, 1.0, 148.0, -0.080, -0.035);
    let mut graph = RegionGraph::new();
    let center_xz = Vector2::new(14.096, -65.592);
    let center_pos = Vector3::new(
        center_xz.x,
        terrain.sample_height_world(center_xz.x, center_xz.y) * crate::config::HEIGHT_SCALE,
        center_xz.y,
    );
    let center = graph.add_node(center_pos, NodeType::Junction);
    for (endpoint_xz, starts_at_center) in [
        (Vector2::new(-15.703, -93.471), false),
        (Vector2::new(56.138, -72.850), false),
        (Vector2::new(-17.050, -60.215), true),
        (Vector2::new(50.308, -31.714), true),
    ] {
        let endpoint_pos = Vector3::new(
            endpoint_xz.x,
            terrain.sample_height_world(endpoint_xz.x, endpoint_xz.y) * crate::config::HEIGHT_SCALE,
            endpoint_xz.y,
        );
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let (start, end, points) = if starts_at_center {
            (
                center,
                endpoint,
                grounded_polyline_points_from_terrain(&terrain, center_xz, endpoint_xz, 24),
            )
        } else {
            (
                endpoint,
                center,
                grounded_polyline_points_from_terrain(&terrain, endpoint_xz, center_xz, 24),
            )
        };
        graph.add_edge(test_edge(
            start,
            end,
            points,
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("skewed elevated 4-way node must compile one visible JunctionN piece");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_non_road_shared_edges_are_height_continuous(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
}

#[test]
fn node_overlay_preserves_skinny_closure_shapes() {
    let shapes = RoadSurfaceSystem::overlay_union_contours(&[vec![
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 0.0005],
        [0.0, 0.0005],
    ]])
    .unwrap();

    assert_eq!(
        shapes.len(),
        1,
        "millimetre-scale deterministic closure slivers must not be filtered before rendering"
    );
}

#[test]
fn visual_polygon_builder_preserves_skinny_closure_geometry() {
    let polygon = RoadSurfaceSystem::make_visual_polygon(vec![
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(0.15, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 0.02),
    ])
    .expect("centimetre-scale curb closure polygons must survive the visual polygon builder");

    assert!(
        !polygon.triangles_world.is_empty(),
        "curb closure polygons must keep renderable CDT triangles"
    );
}

#[test]
fn preview_matches_committed_sections_on_flat_terrain() {
    let terrain = flat_terrain(64, 64);
    let surface = RoadSurfaceSystem::new(16.0);
    let raw_points = vec![Vector3::new(0.0, 0.2, 0.0), Vector3::new(24.0, 0.2, 0.0)];

    let (preview, committed_sections, committed_visual_pieces) =
        compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

    assert_eq!(preview.edge_class, EdgeClass::Standard);
    assert!(preview.is_valid);
    assert_eq!(preview.compiled_sections, committed_sections);
    assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
}

#[test]
fn preview_matches_committed_sections_on_cross_slope() {
    let mut terrain = TerrainSystem::with_chunking(80, 16, 1.0, 8, 0.0);
    for z in 0..16 {
        for x in 0..80 {
            terrain.set_height(x, z, x as f32 * 0.005);
        }
    }
    let surface = RoadSurfaceSystem::new(16.0);
    let y0 = terrain.sample_height_world(-16.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2;
    let y1 = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2;
    let y2 = terrain.sample_height_world(16.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2;
    let raw_points = vec![
        Vector3::new(-16.0, y0, 0.0),
        Vector3::new(0.0, y1, 0.0),
        Vector3::new(16.0, y2, 0.0),
    ];

    let (preview, committed_sections, committed_visual_pieces) =
        compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

    assert_eq!(preview.edge_class, EdgeClass::Standard);
    assert!(preview.is_valid);
    assert_eq!(preview.compiled_sections, committed_sections);
    assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
}

#[test]
fn preview_matches_committed_sections_for_bridges() {
    let terrain = flat_terrain(96, 16);
    let surface = RoadSurfaceSystem::new(16.0);
    let raw_points = vec![
        Vector3::new(0.0, 3.0, 0.0),
        Vector3::new(16.0, 3.0, 0.0),
        Vector3::new(32.0, 3.0, 0.0),
    ];

    let (preview, committed_sections, committed_visual_pieces) =
        compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

    assert_eq!(preview.edge_class, EdgeClass::Bridge);
    assert!(preview.is_valid);
    assert_eq!(preview.compiled_sections, committed_sections);
    assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
}

#[test]
fn preview_matches_committed_sections_for_tunnels() {
    let terrain = flat_terrain(96, 16);
    let surface = RoadSurfaceSystem::new(16.0);
    let raw_points = vec![
        Vector3::new(0.0, -3.0, 0.0),
        Vector3::new(16.0, -3.0, 0.0),
        Vector3::new(32.0, -3.0, 0.0),
    ];

    let (preview, committed_sections, committed_visual_pieces) =
        compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

    assert_eq!(preview.edge_class, EdgeClass::Tunnel);
    assert!(preview.is_valid);
    assert_eq!(preview.compiled_sections, committed_sections);
    assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
}

#[test]
fn standard_road_footprint_uses_stitched_mesh_instead_of_visual_terrain_stamp() {
    let mut terrain = TerrainSystem::with_chunking(65, 65, 1.0, 8, 0.0);
    for z in 0..65 {
        for x in 0..65 {
            terrain.set_height(x, z, x as f32 * 0.01);
        }
    }

    let mut graph = RegionGraph::new();
    let grounded_height = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
    let start = graph.add_node(
        Vector3::new(0.0, grounded_height, -16.0),
        NodeType::Junction,
    );
    let end = graph.add_node(Vector3::new(0.0, grounded_height, 16.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, grounded_height, -16.0),
            Vector3::new(0.0, grounded_height, 16.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);

    let sections = surface.compiled_sections().get(&edge_idx).unwrap();
    let section = sections
        .iter()
        .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
        .unwrap();
    for lateral_offset in [-4.0_f32, 0.0, 4.0] {
        let road_height = section_height_at_lateral_offset(section, lateral_offset).unwrap();
        let sample_x = section.center_xz.x + section.lateral_xz.x * lateral_offset;
        let sample_z = section.center_xz.y + section.lateral_xz.y * lateral_offset;
        let source_height =
            terrain.sample_height_world(sample_x, sample_z) * crate::config::HEIGHT_SCALE;
        let visual_height =
            terrain.sample_visual_height_world(sample_x, sample_z) * crate::config::HEIGHT_SCALE;
        let support_height = surface
            .sample_paved_support_height(&graph, &terrain, sample_x, sample_z)
            .expect("standard paved footprint should expose a solved support surface");
        assert!(
            (visual_height - source_height).abs() <= 0.05,
            "ordinary standard roads must not stamp visual terrain at lateral_offset={lateral_offset:.1}: visual={visual_height:.3} source={source_height:.3}"
        );
        assert!(
            (support_height - road_height).abs() <= 0.05,
            "expected solved paved support to match the compiled road surface at lateral_offset={lateral_offset:.1}: support={support_height:.3} road_height={road_height:.3}"
        );
    }
}

#[test]
fn grounded_standard_roadbed_is_laterally_flat_and_footprint_stays_below_carriageway() {
    let mut terrain = TerrainSystem::with_chunking(129, 97, 1.0, 8, 0.0);
    for z in 0..97 {
        for x in 0..129 {
            terrain.set_height(x, z, x as f32 * 0.03);
        }
    }

    let mut graph = RegionGraph::new();
    let grounded_height = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
    let start = graph.add_node(
        Vector3::new(0.0, grounded_height, -24.0),
        NodeType::Junction,
    );
    let end = graph.add_node(Vector3::new(0.0, grounded_height, 24.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, grounded_height, -24.0),
            Vector3::new(0.0, grounded_height, 24.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);

    let section = surface
        .compiled_sections()
        .get(&edge_idx)
        .unwrap()
        .iter()
        .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
        .unwrap();
    let half_carriageway = graph.edge(edge_idx).width.max(crate::config::LANE_WIDTH) * 0.5;
    let left_height = section_height_at_lateral_offset(section, -half_carriageway).unwrap();
    let right_height = section_height_at_lateral_offset(section, half_carriageway).unwrap();
    let lateral_grade_rate =
        (right_height - left_height) / (half_carriageway * 2.0).max(super::SAMPLE_EPSILON_M);

    assert!(
        lateral_grade_rate.abs() <= 0.001,
        "expected grounded-road carriageway to stay laterally flat: actual_rate={lateral_grade_rate:.4}"
    );
    for sidewalk in section
        .bands
        .iter()
        .filter(|band| band.kind == RoadSurfaceBandKind::Sidewalk)
    {
        assert!(
            (sidewalk.height_start_m - section.center_height_m - CURB_STEP_HEIGHT_M).abs() <= 0.001
        );
        assert!(
            (sidewalk.height_end_m - section.center_height_m - CURB_STEP_HEIGHT_M).abs() <= 0.001
        );
    }

    let mut sampled_profile = Vec::new();
    for lateral_offset in [-half_carriageway * 0.8, 0.0, half_carriageway * 0.8] {
        let road_height = section_height_at_lateral_offset(section, lateral_offset).unwrap();
        let sample_x = section.center_xz.x + section.lateral_xz.x * lateral_offset;
        let sample_z = section.center_xz.y + section.lateral_xz.y * lateral_offset;
        let source_height =
            terrain.sample_height_world(sample_x, sample_z) * crate::config::HEIGHT_SCALE;
        let visual_height =
            terrain.sample_visual_height_world(sample_x, sample_z) * crate::config::HEIGHT_SCALE;
        let visible_surface_height = surface
            .sample_visible_surface_height(&graph, &terrain, sample_x, sample_z)
            .expect("standard road footprint should be owned by the road surface");
        sampled_profile.push((lateral_offset, road_height, visible_surface_height));
        assert!(
            (visual_height - source_height).abs() <= 0.05,
            "ordinary standard roads must not stamp visual terrain on a steep hillside: lateral_offset={lateral_offset:.2} visual_height={visual_height:.3} source_height={source_height:.3}"
        );
        assert!(
            (road_height - visible_surface_height).abs() <= 0.08,
            "expected grounded-road visible surface to follow the solved road surface: lateral_offset={lateral_offset:.2} visible_surface_height={visible_surface_height:.3} road_height={road_height:.3}"
        );
    }

    let left = sampled_profile.first().unwrap();
    let right = sampled_profile.last().unwrap();
    let road_profile_delta = right.1 - left.1;
    let support_profile_delta = right.2 - left.2;
    assert!(
        (support_profile_delta - road_profile_delta).abs() <= 0.05,
        "expected visible road footprint to follow the solved flat roadbed profile: road_profile_delta={road_profile_delta:.3} support_profile_delta={support_profile_delta:.3}"
    );
}

#[test]
fn flat_diagonal_10m_grid_keeps_paved_footprint_below_roadbed() {
    let terrain = TerrainSystem::with_chunking(129, 129, 10.0, 8, 0.0);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-160.0, 0.0, -160.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(160.0, 0.0, 160.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-160.0, 0.0, -160.0),
            Vector3::new(160.0, 0.0, 160.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut terrain = terrain;
    let mut surface = RoadSurfaceSystem::new(128.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let metrics = measure_max_footprint_overflow(&surface, &graph, edge_idx, &terrain);

    assert!(
        metrics.max_overflow_m <= 0.05,
        "expected a flat 45 degree road on a 10 m grid to keep the paved footprint below the roadbed, got {metrics:?}"
    );
}

#[test]
fn shallow_angle_10m_grid_keeps_paved_footprint_below_roadbed() {
    let mut terrain = coarse_hillside_world_terrain(97, 97, 10.0);
    let points = grounded_polyline_points_from_terrain(
        &terrain,
        Vector2::new(-180.0, 5.0),
        Vector2::new(180.0, 1.0),
        28,
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
    let metrics = measure_max_footprint_overflow(&surface, &graph, edge_idx, &terrain);

    assert!(
        metrics.max_overflow_m <= 0.05,
        "expected a shallow-angle road on a 10 m grid to keep the paved footprint below the roadbed, got {metrics:?}"
    );
}

#[test]
fn coarse_10m_hillside_case_keeps_paved_footprint_below_roadbed() {
    let (surface, terrain, graph, edge_idx) = build_coarse_grid_hillside_case(10.0);
    let metrics = measure_max_footprint_overflow(&surface, &graph, edge_idx, &terrain);

    assert!(
        metrics.max_overflow_m <= 0.05,
        "expected the coarse 10 m hillside case to keep the paved footprint below the roadbed, got {metrics:?}"
    );
}

#[test]
fn coarse_5m_hillside_case_stays_below_paved_roadbed_too() {
    let (coarse_surface, coarse_terrain, coarse_graph, coarse_edge_idx) =
        build_coarse_grid_hillside_case(10.0);
    let (fine_surface, fine_terrain, fine_graph, fine_edge_idx) =
        build_coarse_grid_hillside_case(5.0);
    let coarse_metrics = measure_max_footprint_overflow(
        &coarse_surface,
        &coarse_graph,
        coarse_edge_idx,
        &coarse_terrain,
    );
    let fine_metrics =
        measure_max_footprint_overflow(&fine_surface, &fine_graph, fine_edge_idx, &fine_terrain);

    assert!(
        coarse_metrics.max_overflow_m <= 0.05,
        "expected the coarse reference case to stay below the paved roadbed, got coarse={coarse_metrics:?} fine={fine_metrics:?}"
    );
    assert!(
        fine_metrics.max_overflow_m <= 0.05,
        "expected the same hillside case on a 5 m grid to stay below the paved roadbed too, got coarse={coarse_metrics:?} fine={fine_metrics:?}"
    );
}

#[test]
fn grounded_hillside_terrain_outside_paved_footprint_stays_near_source() {
    let mut terrain = TerrainSystem::with_chunking(129, 97, 1.0, 8, 0.0);
    for z in 0..97 {
        for x in 0..129 {
            terrain.set_height(x, z, x as f32 * 0.04);
        }
    }

    let mut graph = RegionGraph::new();
    let grounded_height = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
    let start = graph.add_node(
        Vector3::new(0.0, grounded_height, -24.0),
        NodeType::Junction,
    );
    let end = graph.add_node(Vector3::new(0.0, grounded_height, 24.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, grounded_height, -24.0),
            Vector3::new(0.0, grounded_height, 24.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);

    let sections = surface.compiled_sections().get(&edge_idx).unwrap();
    let section = sections
        .iter()
        .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
        .unwrap();
    let (left_outer, right_outer) = outer_surface_lateral_bounds(section).unwrap();

    let side_a_lateral = left_outer - 2.0;
    let side_b_lateral = right_outer + 2.0;
    let side_a_x = section.center_xz.x + section.lateral_xz.x * side_a_lateral;
    let side_a_z = section.center_xz.y + section.lateral_xz.y * side_a_lateral;
    let side_b_x = section.center_xz.x + section.lateral_xz.x * side_b_lateral;
    let side_b_z = section.center_xz.y + section.lateral_xz.y * side_b_lateral;
    let side_a_actual =
        terrain.sample_visual_height_world(side_a_x, side_a_z) * crate::config::HEIGHT_SCALE;
    let side_b_actual =
        terrain.sample_visual_height_world(side_b_x, side_b_z) * crate::config::HEIGHT_SCALE;
    let side_a_source =
        terrain.sample_height_world(side_a_x, side_a_z) * crate::config::HEIGHT_SCALE;
    let side_b_source =
        terrain.sample_height_world(side_b_x, side_b_z) * crate::config::HEIGHT_SCALE;
    assert!(
        (side_a_actual - side_a_source).abs() <= 0.12,
        "expected terrain outside the paved footprint to remain near source on hillside side A, got actual={side_a_actual:.3} source={side_a_source:.3}"
    );
    assert!(
        (side_b_actual - side_b_source).abs() <= 0.12,
        "expected terrain outside the paved footprint to remain near source on hillside side B, got actual={side_b_actual:.3} source={side_b_source:.3}"
    );

    let far_side_a_lateral = left_outer - EARTHWORK_MAX_MARGIN_M - 6.0;
    let far_side_b_lateral = right_outer + EARTHWORK_MAX_MARGIN_M + 6.0;
    let far_side_a_x = section.center_xz.x + section.lateral_xz.x * far_side_a_lateral;
    let far_side_a_z = section.center_xz.y + section.lateral_xz.y * far_side_a_lateral;
    let far_side_b_x = section.center_xz.x + section.lateral_xz.x * far_side_b_lateral;
    let far_side_b_z = section.center_xz.y + section.lateral_xz.y * far_side_b_lateral;
    let far_side_a_actual = terrain.sample_visual_height_world(far_side_a_x, far_side_a_z)
        * crate::config::HEIGHT_SCALE;
    let far_side_b_actual = terrain.sample_visual_height_world(far_side_b_x, far_side_b_z)
        * crate::config::HEIGHT_SCALE;
    let far_side_a_source =
        terrain.sample_height_world(far_side_a_x, far_side_a_z) * crate::config::HEIGHT_SCALE;
    let far_side_b_source =
        terrain.sample_height_world(far_side_b_x, far_side_b_z) * crate::config::HEIGHT_SCALE;

    assert!((far_side_a_actual - far_side_a_source).abs() <= 0.12);
    assert!((far_side_b_actual - far_side_b_source).abs() <= 0.12);
}

#[test]
fn bridge_earthworks_do_not_flatten_under_the_span() {
    let mut terrain = flat_terrain(97, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-24.0, 6.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 6.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-24.0, 6.0, 0.0),
            Vector3::new(0.0, 6.0, 0.0),
            Vector3::new(24.0, 6.0, 0.0),
        ],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);

    let span_center = terrain.sample_visual_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
    let abutment = terrain.sample_visual_height_world(-20.0, 0.0) * crate::config::HEIGHT_SCALE;
    assert!(span_center.abs() <= 0.01);
    assert!(abutment >= 1.0);
}

#[test]
fn tunnel_earthworks_only_stamp_portals() {
    let mut terrain = flat_terrain(97, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-24.0, 0.0, 0.0),
            Vector3::new(-10.0, -6.0, 0.0),
            Vector3::new(10.0, -6.0, 0.0),
            Vector3::new(24.0, 0.0, 0.0),
        ],
        10.0,
        EdgeClass::Tunnel,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);

    let center = terrain.sample_visual_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
    let portal = terrain.sample_visual_height_world(-20.0, 0.0) * crate::config::HEIGHT_SCALE;
    assert!(center.abs() <= 0.01);
    assert!(portal <= -0.1);
}

#[test]
fn dirty_terrain_earthworks_stay_bounded_to_touched_chunks() {
    let mut terrain = flat_terrain(161, 65);
    let mut graph = RegionGraph::new();
    let left_a = graph.add_node(Vector3::new(-56.0, 0.0, 0.0), NodeType::Junction);
    let left_b = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let right_a = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let right_b = graph.add_node(Vector3::new(56.0, 0.0, 0.0), NodeType::Junction);
    let left_edge = graph.add_edge(test_edge(
        left_a,
        left_b,
        vec![Vector3::new(-56.0, 0.0, 0.0), Vector3::new(-24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        right_a,
        right_b,
        vec![Vector3::new(24.0, 0.0, 0.0), Vector3::new(56.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let far_before = terrain.sample_visual_height_world(40.0, 0.0) * crate::config::HEIGHT_SCALE;

    surface.mark_edge_dirty(&graph, left_edge);
    let stamped_chunks = surface.rebuild_dirty_earthworks(&graph, &mut terrain);
    let far_after = terrain.sample_visual_height_world(40.0, 0.0) * crate::config::HEIGHT_SCALE;
    let right_chunk = surface.chunk_coords_for_world(40.0, 0.0);

    assert!(!stamped_chunks.is_empty());
    assert!(!stamped_chunks.contains(&right_chunk));
    assert!((far_after - far_before).abs() <= 0.001);
}

#[test]
fn compile_dirty_derives_edge_chunks_from_compiled_piece_coverage() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(5.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        n0,
        n1,
        vec![Vector3::new(5.0, 0.0, 0.0), Vector3::new(25.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(10.0);
    surface.compile_dirty(&graph, &terrain);

    let surface_chunks = surface
        .surface_span_chunks
        .get(&edge_idx)
        .expect("compiled span must own surface chunks")
        .clone();
    let terrain_chunks = surface
        .earthwork_span_chunks
        .get(&edge_idx)
        .expect("compiled span must own terrain chunks")
        .clone();
    assert!(!surface_chunks.is_empty());
    assert!(terrain_chunks.len() >= surface_chunks.len());

    surface.mark_edge_dirty(&graph, edge_idx);
    surface.compile_dirty(&graph, &terrain);

    for chunk in surface_chunks {
        let entry = surface
            .surface_chunk_cache
            .get(&chunk)
            .unwrap_or_else(|| panic!("surface chunk {chunk:?} must be rebuilt"));
        assert!(entry.edge_indices.contains(&edge_idx));
    }
    for chunk in terrain_chunks {
        let entry = surface
            .earthwork_chunk_cache
            .get(&chunk)
            .unwrap_or_else(|| panic!("terrain chunk {chunk:?} must be rebuilt"));
        assert!(entry.edge_indices.contains(&edge_idx));
    }
}

#[test]
fn visible_surface_height_prefers_compiled_roadbed() {
    let mut terrain = TerrainSystem::with_chunking(65, 65, 1.0, 8, 0.0);
    for z in 0..65 {
        for x in 0..65 {
            terrain.set_height(x, z, x as f32 * 0.01);
        }
    }

    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 16.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let sampled = surface
        .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
        .expect("standard road should own its paved footprint");
    let section = surface
        .compiled_sections()
        .get(&edge_idx)
        .unwrap()
        .iter()
        .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
        .unwrap();
    let expected = section_height_at_lateral_offset(section, 0.0).unwrap();
    assert!((sampled - expected).abs() <= 0.05);
}

#[test]
fn paved_support_height_matches_grounded_visible_roadbed() {
    let mut terrain = TerrainSystem::with_chunking(65, 65, 1.0, 8, 0.0);
    for z in 0..65 {
        for x in 0..65 {
            terrain.set_height(x, z, x as f32 * 0.01);
        }
    }

    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 16.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);

    let visible_height = surface
        .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
        .expect("grounded road should own its paved footprint");
    let support_height = surface
        .sample_paved_support_height(&graph, &terrain, 0.0, 0.0)
        .expect("grounded road should expose paved support clearance");

    assert!(
        (visible_height - support_height).abs() <= 0.05,
        "expected grounded-road integrated support height to match the visible roadbed instead of staying one pavement depth below it: visible_height={visible_height:.3} support_height={support_height:.3}"
    );
}

#[test]
fn visible_surface_height_skips_grounded_terminal_earthwork_margin() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        center,
        end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let terminal_piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("terminal should compile a visual node piece");
    let inner_point = terminal_piece.outer_boundary_loops[0].points_world[0];
    let outer_point = terminal_piece.earthwork_outer_boundary_loops[0].points_world[0];
    let sample_x = (inner_point.x + outer_point.x) * 0.5;
    let sample_z = (inner_point.z + outer_point.z) * 0.5;

    assert!(
        surface
            .sample_visible_surface_height(&graph, &terrain, sample_x, sample_z)
            .is_none(),
        "grounded standard terminal earthwork margin stays outside visible-surface queries; Rust-generated terrain topology owns the ordinary seam"
    );
}

#[test]
fn visible_surface_height_skips_grounded_span_earthwork_margin() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -24.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("standard edge should compile a visual span piece");
    let inner_point = span_piece.outer_boundary_loops[0].points_world[0];
    let outer_point = span_piece.earthwork_outer_boundary_loops[0].points_world[0];
    let sample_x = (inner_point.x + outer_point.x) * 0.5;
    let sample_z = (inner_point.z + outer_point.z) * 0.5;

    assert!(
        surface
            .sample_visible_surface_height(&graph, &terrain, sample_x, sample_z)
            .is_none(),
        "grounded standard span earthwork margin stays outside visible-surface queries; Rust-generated terrain topology owns the ordinary seam"
    );
}

#[test]
fn visible_surface_height_skips_buried_tunnel_midspan() {
    let terrain = flat_terrain(97, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-24.0, 0.0, 0.0),
            Vector3::new(-10.0, -6.0, 0.0),
            Vector3::new(10.0, -6.0, 0.0),
            Vector3::new(24.0, 0.0, 0.0),
        ],
        10.0,
        EdgeClass::Tunnel,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert!(
        surface
            .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
            .is_none()
    );
    assert!(
        surface
            .sample_visible_surface_height(&graph, &terrain, -20.0, 0.0)
            .is_some()
    );
}

#[test]
fn visible_surface_raycast_hits_bridge_before_terrain() {
    let terrain = flat_terrain(97, 33);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-24.0, 6.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(24.0, 6.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-24.0, 6.0, 0.0),
            Vector3::new(0.0, 6.0, 0.0),
            Vector3::new(24.0, 6.0, 0.0),
        ],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let hit = surface
        .raycast_visible_surface(
            &graph,
            &terrain,
            Vector3::new(0.0, 20.0, 0.0),
            Vector3::DOWN,
        )
        .expect("bridge should be hittable by the combined world-surface ray");
    assert!((hit.y - 6.0).abs() <= 0.05);
}

#[test]
fn debug_line_data_exposes_sections_bands_patches_and_earthwork_chunks() {
    let mut terrain = flat_terrain(65, 65);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 16.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let debug = surface.build_debug_line_data(&graph, &terrain);

    assert!(!debug.section_lines.is_empty());
    assert!(!debug.band_lines.is_empty());
    assert!(!debug.piece_boundary_lines.is_empty());
    assert!(!debug.earthwork_chunk_lines.is_empty());
}

#[test]
fn debug_geometry_dump_exposes_edge_sections_and_terrain_samples() {
    let mut terrain = sloped_terrain(65, 65);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-16.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(16.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(-16.0, -0.8, 0.0),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(16.0, 0.8, 0.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);
    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[edge_idx]);

    assert!(dump.contains("ROAD_GEOMETRY_DUMP_BEGIN"));
    assert!(dump.contains("\"edge_idx\": 0"));
    assert!(dump.contains("\"physical_geometry_world\""));
    assert!(dump.contains("\"sections\""));
    assert!(dump.contains("\"source_center_y_m\""));
    assert!(dump.contains("\"visual_center_y_m\""));
    assert!(dump.contains("\"left_outer_margin\""));
    assert!(dump.contains("\"right_outer_margin\""));
    assert!(dump.contains("\"nodes\""));
    assert!(dump.contains("\"road_topology\""));
    assert!(dump.contains("\"sidewalk_topology\""));
    assert!(dump.contains("\"band_ownership\""));
    assert!(dump.contains("\"height_owner\""));
    assert!(dump.contains("\"seam_constraints\""));
    assert!(dump.contains("\"material_footprint_coverage\""));
    assert!(dump.contains("\"outer_boundary_top_match\""));
    assert!(dump.contains("\"mouth_seams\""));
    assert!(dump.contains("\"earthwork_face_top_match\""));
    assert!(dump.contains("ROAD_GEOMETRY_DUMP_END"));
}

#[test]
fn transit_sync_to_terrain_invalidates_compiled_sections() {
    let terrain_before = flat_terrain(65, 65);
    let mut terrain_after = flat_terrain(65, 65);
    for z in 0..terrain_after.height {
        for x in 0..terrain_after.width {
            terrain_after.set_height(x, z, 0.5);
        }
    }

    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, -16.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 0.0, 16.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, 0.0, -16.0),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 16.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.road_surface.compile_dirty(&graph, &terrain_before);
    let before_height = network
        .road_surface
        .compiled_sections()
        .get(&edge_idx)
        .unwrap()[1]
        .center_height_m;

    network.sync_to_terrain(&mut graph, &terrain_after);
    assert!(
        graph.edge(edge_idx).geometry[1].y >= 9.5,
        "sync_to_terrain should resample edge geometry from terrain before recompilation"
    );

    network.road_surface.compile_dirty(&graph, &terrain_after);
    let after_height = network
        .road_surface
        .compiled_sections()
        .get(&edge_idx)
        .unwrap()[1]
        .center_height_m;

    assert!(
        after_height >= before_height + 9.5,
        "compiled roadbed cache should be invalidated after terrain sync, got before={before_height:.3} after={after_height:.3}"
    );
}
