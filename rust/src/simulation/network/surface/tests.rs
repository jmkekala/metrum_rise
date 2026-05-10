//! Unit tests for the road-surface compiler and ownership caches.

use super::earthwork::EARTHWORK_MAX_MARGIN_M;
use super::edge::CURB_STEP_HEIGHT_M;
use super::{
    PreviewRoadSurfaceResult, RoadSurfaceBandKind, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceTerrainClipEdgeKind,
    RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipSourceEdge, RoadSurfaceVisualNodePiece,
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

fn road_points_from_json(points_json: &str) -> Vec<Vector3> {
    serde_json::from_str::<Vec<[f32; 3]>>(points_json)
        .expect("logged road geometry points must parse")
        .into_iter()
        .map(|[x, y, z]| Vector3::new(x, y, z))
        .collect()
}

fn terrain_clip_source_edge_for_test(
    start: Vector3,
    end: Vector3,
) -> RoadSurfaceTerrainClipSourceEdge {
    RoadSurfaceTerrainClipSourceEdge {
        start,
        end,
        kind: RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
    }
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

#[derive(Clone, Copy, Debug)]
struct FootprintOverflowMetrics {
    max_overflow_m: f32,
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

fn overlay_contours_from_polygons(
    polygons: &[RoadSurfaceVisualPolygon],
) -> Vec<super::NodeOverlayContour> {
    polygons
        .iter()
        .filter_map(|polygon| {
            let contour = overlay_contour_from_world_points(&polygon.points_world);
            (contour.len() >= 3).then_some(contour)
        })
        .collect()
}

fn overlay_contour_from_world_points(points: &[Vector3]) -> super::NodeOverlayContour {
    let mut contour = Vec::with_capacity(points.len());
    for point in points {
        let overlay_point = super::backend::road_vec2_to_overlay_point(
            super::backend::godot_vec3_xz_to_road(*point),
        );
        if contour.last().is_none_or(|last| *last != overlay_point) {
            contour.push(overlay_point);
        }
    }
    if contour.len() >= 2 && contour.first() == contour.last() {
        contour.pop();
    }
    contour
}

fn overlay_contours_from_top_polygons<'a>(
    polygons: impl IntoIterator<Item = &'a RoadSurfaceVisualPolygon>,
) -> Vec<super::NodeOverlayContour> {
    let mut contours = Vec::new();
    for polygon in polygons {
        if polygon.triangles_world.is_empty() {
            let contour = overlay_contour_from_world_points(&polygon.points_world);
            if contour.len() >= 3 {
                contours.push(contour);
            }
            continue;
        }
        for triangle in &polygon.triangles_world {
            let contour = overlay_contour_from_world_points(triangle);
            if contour.len() >= 3 {
                contours.push(contour);
            }
        }
    }
    contours
}

fn overlay_area_m2(shapes: &super::NodeOverlayShapes) -> f32 {
    shapes
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum()
}

fn node_top_coverage_details_m2(
    piece: &RoadSurfaceVisualNodePiece,
) -> (
    f32,
    f32,
    f32,
    super::NodeOverlayShapes,
    super::NodeOverlayShapes,
) {
    let footprint_contours = overlay_contours_from_polygons(&piece.outer_boundary_loops);
    let footprint_shapes = RoadSurfaceSystem::overlay_union_contours(&footprint_contours)
        .expect("node footprint overlay union should succeed");
    let top_contours = overlay_contours_from_top_polygons(
        piece
            .road_surface_polygons
            .iter()
            .chain(piece.curb_surface_polygons.iter())
            .chain(piece.sidewalk_surface_polygons.iter()),
    );
    let top_shapes = RoadSurfaceSystem::overlay_union_contours(&top_contours)
        .expect("node top overlay union should succeed");
    let missing_shapes = RoadSurfaceSystem::overlay_binary_shapes(
        &footprint_shapes,
        &top_shapes,
        OverlayRule::Difference,
    )
    .expect("node footprint/top difference should succeed");
    let extra_shapes = RoadSurfaceSystem::overlay_binary_shapes(
        &top_shapes,
        &footprint_shapes,
        OverlayRule::Difference,
    )
    .expect("node top/footprint difference should succeed");
    let budget_m2 = RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&footprint_shapes)
        .max(RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(
            &top_shapes,
        ));
    (
        overlay_area_m2(&missing_shapes),
        overlay_area_m2(&extra_shapes),
        budget_m2,
        missing_shapes,
        extra_shapes,
    )
}

fn assert_node_top_covers_footprint(piece: &RoadSurfaceVisualNodePiece) {
    let (missing_area_m2, extra_area_m2, budget_m2, missing_shapes, extra_shapes) =
        node_top_coverage_details_m2(piece);
    assert!(
        missing_area_m2 <= budget_m2 && extra_area_m2 <= budget_m2,
        "node top surfaces must exactly cover the canonical footprint; kind={:?} missing_area={missing_area_m2:.6} extra_area={extra_area_m2:.6} budget={budget_m2:.6} missing_shapes={missing_shapes:?} extra_shapes={extra_shapes:?}",
        piece.kind
    );
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

fn assert_terminal_mouth_handoff_surface_is_owned(
    piece: &RoadSurfaceVisualNodePiece,
    mouth: &super::IncidentMouthProfile,
    material: RoadSurfaceBandKind,
    start_boundary_index: usize,
    end_boundary_index: usize,
    label: &str,
) {
    let start = mouth.boundary_points_world[start_boundary_index];
    let end = mouth.boundary_points_world[end_boundary_index];
    let inward = mouth.inward_direction_xz.normalized();
    let sample = Vector2::new(
        (start.x + end.x) * 0.5 - inward.x * 0.1,
        (start.z + end.z) * 0.5 - inward.y * 0.1,
    );
    let polygons = match material {
        RoadSurfaceBandKind::CurbOrShoulder => &piece.curb_surface_polygons,
        RoadSurfaceBandKind::Sidewalk => &piece.sidewalk_surface_polygons,
        RoadSurfaceBandKind::Carriageway => &piece.road_surface_polygons,
        _ => &piece.sidewalk_surface_polygons,
    };
    assert!(
        point_inside_visual_polygons(polygons, sample),
        "terminal handoff surface must be owned by {material:?}; label={label} sample={sample:?}"
    );
}

fn assert_terminal_band_interval_grid_is_owned(
    piece: &RoadSurfaceVisualNodePiece,
    endpoint: &super::IncidentMouthProfile,
    mouth: &super::IncidentMouthProfile,
    material: RoadSurfaceBandKind,
    start_boundary_index: usize,
    end_boundary_index: usize,
    label: &str,
) {
    let polygons = match material {
        RoadSurfaceBandKind::CurbOrShoulder => &piece.curb_surface_polygons,
        RoadSurfaceBandKind::Sidewalk => &piece.sidewalk_surface_polygons,
        RoadSurfaceBandKind::Carriageway => &piece.road_surface_polygons,
        _ => &piece.sidewalk_surface_polygons,
    };
    for longitudinal_t in [0.1_f32, 0.5, 0.9, 0.98] {
        for lateral_t in [0.05_f32, 0.5, 0.95] {
            let endpoint_start = endpoint.boundary_points_world[start_boundary_index];
            let endpoint_end = endpoint.boundary_points_world[end_boundary_index];
            let mouth_start = mouth.boundary_points_world[start_boundary_index];
            let mouth_end = mouth.boundary_points_world[end_boundary_index];
            let endpoint_sample = endpoint_start.lerp(endpoint_end, lateral_t);
            let mouth_sample = mouth_start.lerp(mouth_end, lateral_t);
            let sample_world = endpoint_sample.lerp(mouth_sample, longitudinal_t);
            let sample = Vector2::new(sample_world.x, sample_world.z);
            assert!(
                point_inside_visual_polygons(polygons, sample),
                "terminal band interval must be owned by {material:?}; label={label} longitudinal_t={longitudinal_t} lateral_t={lateral_t} sample={sample:?}"
            );
        }
    }
}

fn assert_terminal_band_interval_grid_is_not_duplicated_by_span(
    span_piece: &super::RoadSurfaceVisualSpanPiece,
    endpoint: &super::IncidentMouthProfile,
    mouth: &super::IncidentMouthProfile,
    start_boundary_index: usize,
    end_boundary_index: usize,
    label: &str,
) {
    for longitudinal_t in [0.1_f32, 0.5, 0.9, 0.98] {
        for lateral_t in [0.05_f32, 0.5, 0.95] {
            let endpoint_start = endpoint.boundary_points_world[start_boundary_index];
            let endpoint_end = endpoint.boundary_points_world[end_boundary_index];
            let mouth_start = mouth.boundary_points_world[start_boundary_index];
            let mouth_end = mouth.boundary_points_world[end_boundary_index];
            let endpoint_sample = endpoint_start.lerp(endpoint_end, lateral_t);
            let mouth_sample = mouth_start.lerp(mouth_end, lateral_t);
            let sample_world = endpoint_sample.lerp(mouth_sample, longitudinal_t);
            let sample = Vector2::new(sample_world.x, sample_world.z);
            let duplicated =
                point_inside_visual_polygons(&span_piece.road_surface_polygons, sample)
                    || point_inside_visual_polygons(&span_piece.curb_surface_polygons, sample)
                    || point_inside_visual_polygons(&span_piece.sidewalk_surface_polygons, sample);
            assert!(
                !duplicated,
                "terminal band interval must not be duplicated by span top surfaces; label={label} longitudinal_t={longitudinal_t} lateral_t={lateral_t} sample={sample:?}"
            );
        }
    }
}

fn assert_vertical_curb_face_lower_edge_covers(
    polygons: &[RoadSurfaceVisualPolygon],
    start: Vector3,
    end: Vector3,
    label: &str,
) {
    let start_key = test_xz_key(start);
    let end_key = test_xz_key(end);
    let expected_length = Vector2::new(end.x - start.x, end.z - start.z).length();
    let covered_length = polygons
        .iter()
        .filter_map(vertical_face_lower_edge_for_test)
        .filter(|edge| {
            test_xz_key_lies_on_segment(test_xz_key(edge[0]), start_key, end_key)
                && test_xz_key_lies_on_segment(test_xz_key(edge[1]), start_key, end_key)
        })
        .map(|edge| Vector2::new(edge[1].x - edge[0].x, edge[1].z - edge[0].z).length())
        .sum::<f32>();

    assert!(
        covered_length + 0.001 >= expected_length,
        "vertical curb face lower edge must cover expected segment; label={label} start={start:?} end={end:?} covered={covered_length:.4} expected={expected_length:.4}"
    );
}

fn vertical_face_lower_edge_for_test(polygon: &RoadSurfaceVisualPolygon) -> Option<[Vector3; 2]> {
    if polygon.points_world.len() != 4 {
        return None;
    }
    let lower_y = polygon
        .points_world
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let lower_points = polygon
        .points_world
        .iter()
        .copied()
        .filter(|point| (point.y - lower_y).abs() <= SAMPLE_EPSILON_M)
        .collect::<Vec<_>>();
    if lower_points.len() != 2 {
        return None;
    }
    Some([lower_points[0], lower_points[1]])
}

fn test_xz_key_lies_on_segment(point: (i64, i64), start: (i64, i64), end: (i64, i64)) -> bool {
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    let px = i128::from(point.0 - start.0);
    let pz = i128::from(point.1 - start.1);
    if px * dz - pz * dx != 0 {
        return false;
    }
    let dot = px * dx + pz * dz;
    let len_squared = dx * dx + dz * dz;
    dot >= 0 && dot <= len_squared
}

fn test_xz_key(point: Vector3) -> (i64, i64) {
    let point =
        super::backend::road_vec2_to_overlay_point(super::backend::godot_vec3_xz_to_road(point));
    (
        (point[0] * super::backend::ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point[1] * super::backend::ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
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
        .chain(piece.curb_surface_polygons.iter())
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

fn assert_top_surface_triangles_face_up(piece: &RoadSurfaceVisualNodePiece) {
    for triangle in piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .flat_map(|polygon| polygon.triangles_world.iter().copied())
    {
        let double_area_xz = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
            - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
        assert!(
            double_area_xz >= -0.001,
            "node top-surface triangles must remain front-facing from above; kind={:?} triangle={triangle:?} double_area_xz={double_area_xz:.6}",
            piece.kind
        );
    }
}

fn assert_curb_vertical_faces_visible_from_carriageway(piece: &RoadSurfaceVisualNodePiece) {
    for face in &piece.curb_vertical_face_polygons {
        let Some(lower_edge) = vertical_face_lower_edge_for_test(face) else {
            continue;
        };
        let Some(visible_direction) = vertical_face_visible_direction_for_test(face) else {
            continue;
        };
        let visible_direction =
            Vector3::new(visible_direction.x, 0.0, visible_direction.z).normalized();
        let midpoint = (lower_edge[0] + lower_edge[1]) * 0.5;
        let mut best_dot: Option<f32> = None;

        for road_polygon in &piece.road_surface_polygons {
            if !polygon_boundary_overlaps_edge_at_height_for_test(road_polygon, lower_edge) {
                continue;
            }
            let Some(centroid) = polygon_centroid_for_test(road_polygon) else {
                continue;
            };
            let owner_direction =
                Vector3::new(centroid.x - midpoint.x, 0.0, centroid.z - midpoint.z);
            if owner_direction.length_squared() <= 1e-8 {
                continue;
            }
            let dot = visible_direction.dot(owner_direction.normalized());
            best_dot = Some(best_dot.map_or(dot, |current| current.max(dot)));
        }

        if let Some(dot) = best_dot {
            assert!(
                dot > 0.0,
                "curb vertical face must be visible from the lower carriageway owner; kind={:?} face={:?} visible_direction={visible_direction:?} dot={dot:.6}",
                piece.kind,
                face.points_world
            );
        }
    }
}

fn vertical_face_visible_direction_for_test(polygon: &RoadSurfaceVisualPolygon) -> Option<Vector3> {
    let [upper_start, lower_start, lower_end, _upper_end] = polygon.points_world.as_slice() else {
        return None;
    };
    let normal = (*lower_start - *upper_start).cross(*lower_end - *upper_start);
    (normal.length_squared() > 1e-8).then_some(-normal.normalized())
}

fn polygon_boundary_overlaps_edge_at_height_for_test(
    polygon: &RoadSurfaceVisualPolygon,
    edge: [Vector3; 2],
) -> bool {
    let points = &polygon.points_world;
    if points.len() < 2 {
        return false;
    }
    let expected_y = (edge[0].y + edge[1].y) * 0.5;
    let edge_start = test_xz_key(edge[0]);
    let edge_end = test_xz_key(edge[1]);
    (0..points.len()).any(|index| {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        (start.y - expected_y).abs() <= SAMPLE_EPSILON_M
            && (end.y - expected_y).abs() <= SAMPLE_EPSILON_M
            && test_xz_segments_overlap_with_length(
                test_xz_key(start),
                test_xz_key(end),
                edge_start,
                edge_end,
            )
    })
}

fn test_xz_segments_overlap_with_length(
    a_start: (i64, i64),
    a_end: (i64, i64),
    b_start: (i64, i64),
    b_end: (i64, i64),
) -> bool {
    if a_start == a_end || b_start == b_end {
        return false;
    }
    let a_dx = i128::from(a_end.0 - a_start.0);
    let a_dz = i128::from(a_end.1 - a_start.1);
    let b_dx = i128::from(b_end.0 - b_start.0);
    let b_dz = i128::from(b_end.1 - b_start.1);
    if a_dx * b_dz - a_dz * b_dx != 0 {
        return false;
    }
    if !test_xz_key_lies_on_segment(a_start, b_start, b_end)
        && !test_xz_key_lies_on_segment(a_end, b_start, b_end)
        && !test_xz_key_lies_on_segment(b_start, a_start, a_end)
        && !test_xz_key_lies_on_segment(b_end, a_start, a_end)
    {
        return false;
    }
    let use_x = (a_end.0 - a_start.0).abs() >= (a_end.1 - a_start.1).abs();
    let coordinate = |key: (i64, i64)| {
        if use_x { key.0 } else { key.1 }
    };
    let a0 = coordinate(a_start);
    let a1 = coordinate(a_end);
    let b0 = coordinate(b_start);
    let b1 = coordinate(b_end);
    a0.min(a1).max(b0.min(b1)) < a0.max(a1).min(b0.max(b1))
}

fn polygon_centroid_for_test(polygon: &RoadSurfaceVisualPolygon) -> Option<Vector3> {
    let mut sum = Vector3::ZERO;
    let mut count = 0usize;
    for point in &polygon.points_world {
        sum += Vector3::new(point.x, 0.0, point.z);
        count += 1;
    }
    (count > 0).then_some(sum / count as f32)
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
        .filter(|region| {
            region.kind != RoadSurfaceBandKind::Carriageway
                && region.kind != RoadSurfaceBandKind::CurbOrShoulder
        })
        .count();
    let curb_count = piece
        .owned_regions
        .iter()
        .filter(|region| region.kind == RoadSurfaceBandKind::CurbOrShoulder)
        .count();
    assert_eq!(
        carriageway_count,
        piece.road_surface_polygons.len(),
        "asphalt polygons must be derived from carriageway-owned node regions"
    );
    assert_eq!(
        curb_count,
        piece.curb_surface_polygons.len(),
        "curb polygons must be derived from curb/shoulder-owned node regions"
    );
    assert_eq!(
        non_road_count,
        piece.sidewalk_surface_polygons.len(),
        "sidewalk polygons must be derived from sidewalk-owned node regions"
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

fn assert_compiled_bend_piece(
    surface: &RoadSurfaceSystem,
    bend: u32,
) -> &RoadSurfaceVisualNodePiece {
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .expect("bend should compile through canonical owned regions");
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.curb_surface_polygons.is_empty());
    assert!(!piece.curb_vertical_face_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(piece);
    assert_top_surface_triangles_face_up(piece);
    assert_curb_vertical_faces_visible_from_carriageway(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
    assert_node_top_covers_footprint(piece);
    piece
}

fn assert_outer_boundary_vertices_match_visible_top(piece: &RoadSurfaceVisualNodePiece) {
    let top_polygons = piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .collect::<Vec<_>>();
    let top_vertices = piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
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

fn assert_material_top_supports_point(
    polygons: &[RoadSurfaceVisualPolygon],
    point: Vector3,
    label: &str,
) {
    assert!(
        polygons
            .iter()
            .any(|polygon| polygon_supports_top_point(polygon, point)),
        "material top surface must support anchor point; label={label} point={point:?}"
    );
}

fn polygon_supports_top_point(polygon: &RoadSurfaceVisualPolygon, point: Vector3) -> bool {
    polygon_vertices_support_top_point(&polygon.points_world, point)
        || polygon_edges_support_top_point(&polygon.points_world, point)
        || polygon.triangles_world.iter().any(|triangle| {
            triangle
                .iter()
                .any(|&candidate| top_points_match(candidate, point))
                || triangle_edges_support_top_point(*triangle, point)
        })
}

fn polygon_vertices_support_top_point(vertices: &[Vector3], point: Vector3) -> bool {
    vertices
        .iter()
        .copied()
        .any(|candidate| top_points_match(candidate, point))
}

fn polygon_edges_support_top_point(vertices: &[Vector3], point: Vector3) -> bool {
    if vertices.len() < 2 {
        return false;
    }
    (0..vertices.len()).any(|index| {
        segment_supports_top_point(
            point,
            vertices[index],
            vertices[(index + 1) % vertices.len()],
        )
    })
}

fn triangle_edges_support_top_point(triangle: [Vector3; 3], point: Vector3) -> bool {
    (0..3)
        .any(|index| segment_supports_top_point(point, triangle[index], triangle[(index + 1) % 3]))
}

fn segment_supports_top_point(point: Vector3, start: Vector3, end: Vector3) -> bool {
    let segment = Vector2::new(end.x - start.x, end.z - start.z);
    let len_squared = segment.length_squared();
    if len_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
        return false;
    }
    let to_point = Vector2::new(point.x - start.x, point.z - start.z);
    let t = (to_point.dot(segment) / len_squared).clamp(0.0, 1.0);
    let candidate = start.lerp(end, t);
    top_points_match(candidate, point)
}

fn top_points_match(candidate: Vector3, point: Vector3) -> bool {
    test_xz_key(candidate) == test_xz_key(point) && (candidate.y - point.y).abs() <= 0.004
}

fn assert_debug_dump_mouth_seams_are_clean(dump: &str) {
    let json_start = dump
        .find('{')
        .expect("road geometry dump should contain a JSON object");
    let json_end = dump
        .rfind('}')
        .expect("road geometry dump should contain a JSON object");
    let json: serde_json::Value = serde_json::from_str(&dump[json_start..=json_end])
        .expect("road geometry dump JSON should parse");
    let nodes = json["nodes"]
        .as_array()
        .expect("road geometry dump should include nodes");
    let mut checked = 0usize;
    for node in nodes {
        let node_id = node["node_id"].as_u64().unwrap_or_default();
        let mouth_seams = node["mouth_seams"]
            .as_array()
            .expect("node debug dump should include mouth seams");
        for seam in mouth_seams {
            checked += 1;
            let problem_count = seam["problem_count"]
                .as_u64()
                .expect("mouth seam debug should include a problem count");
            assert_eq!(
                problem_count, 0,
                "mouth seam debug must be clean; node_id={node_id} seam={seam}"
            );
        }
    }
    assert!(
        checked > 0,
        "road geometry dump should include mouth seam checks"
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
    assert!(
        junction_surface
            .compiled_visual_node_pieces()
            .get(&jb)
            .is_none(),
        "short right-angle bend remains rejected until its full curb/sidewalk ownership is generated before heighting"
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
        .expect("bend should compile once generated curb join ownership is explicit");
    assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(bend_piece);
    assert_node_piece_has_curb_and_sidewalk_owners(bend_piece);
    assert_material_triangles_do_not_overlap(bend_piece);
    assert!(!bend_piece.outer_boundary_loops.is_empty());
    assert!(!bend_piece.road_surface_polygons.is_empty());
    assert!(!bend_piece.curb_surface_polygons.is_empty());
    assert!(!bend_piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(bend_piece);
    assert_outer_boundary_vertices_match_visible_top(bend_piece);

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
    assert_node_piece_uses_band_owned_regions(terminal_piece);
    assert_node_piece_has_curb_and_sidewalk_owners(terminal_piece);
    assert_material_triangles_do_not_overlap(terminal_piece);
    assert!(!terminal_piece.outer_boundary_loops.is_empty());
    assert!(!terminal_piece.road_surface_polygons.is_empty());
    assert!(!terminal_piece.curb_surface_polygons.is_empty());
    assert!(!terminal_piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(terminal_piece);
    assert_outer_boundary_vertices_match_visible_top(terminal_piece);
    assert_node_top_covers_footprint(terminal_piece);
    assert!(
        terminal_piece
            .road_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        terminal_piece
            .curb_surface_polygons
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
fn flat_logged_curve_bend_compiles_with_explicit_point_contact_curb_ownership() {
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
    graph.rebuild_adjacency_list();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_bend_piece(&surface, bend);
}

#[test]
fn logged_sixty_degree_bend_compiles_with_explicit_curb_sidewalk_endpoint_authority() {
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

    assert_compiled_bend_piece(&surface, bend);
}

#[test]
fn logged_flat_sixty_degree_bend_compiles_with_explicit_curb_sidewalk_endpoint_authority() {
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

    assert_compiled_bend_piece(&surface, bend);
}

#[test]
fn logged_oblique_curve_bend_top_surfaces_cover_footprint() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-137.811, 0.0, -32.495), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-62.948, 0.0, -30.476), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(-0.213, 0.0, 15.063), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-137.811, 0.0, -32.495),
            Vector3::new(-62.948, 0.0, -30.476),
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
            Vector3::new(-62.948, 0.0, -30.476),
            Vector3::new(-0.213, 0.0, 15.063),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_bend_piece(&surface, bend);
}

#[test]
fn logged_bend_with_fragmented_asphalt_curb_step_compiles() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-107.559, 0.0, -28.209), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-54.287, 0.0, -22.547), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(-16.205, 0.0, 23.182), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-107.559, 0.0, -28.209),
            Vector3::new(-97.788, 0.0, -27.170),
            Vector3::new(-82.795, 0.0, -25.577),
            Vector3::new(-69.410, 0.0, -24.155),
            Vector3::new(-58.119, 0.0, -22.954),
            Vector3::new(-54.287, 0.0, -22.547),
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
            Vector3::new(-54.287, 0.0, -22.547),
            Vector3::new(-53.860, 0.0, -22.034),
            Vector3::new(-52.240, 0.0, -20.089),
            Vector3::new(-49.618, 0.0, -16.940),
            Vector3::new(-45.836, 0.0, -12.398),
            Vector3::new(-40.968, 0.0, -6.553),
            Vector3::new(-35.693, 0.0, -0.218),
            Vector3::new(-30.386, 0.0, 6.154),
            Vector3::new(-25.038, 0.0, 12.576),
            Vector3::new(-20.875, 0.0, 17.575),
            Vector3::new(-16.205, 0.0, 23.182),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_bend_piece(&surface, bend);
}

#[test]
fn logged_inside_bend_compiles_with_explicit_point_contact_curb_ownership() {
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
    assert_compiled_bend_piece(&surface, bend);
}

#[test]
fn logged_loop_bend_does_not_assign_sidewalk_join_outside_height_field() {
    let terrain = flat_terrain(512, 512);
    let mut graph = RegionGraph::new();
    let northwest = graph.add_node(Vector3::new(-76.169, 0.0, 80.632), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-118.592, 0.0, 36.658), NodeType::Junction);
    let south = graph.add_node(Vector3::new(-125.370, 0.0, -4.912), NodeType::Junction);

    graph.add_edge(test_edge(
        northwest,
        bend,
        road_points_from_json(
            "[[-76.169,0.0,80.632],[-76.646,0.0,80.138],[-77.218,0.0,79.545],[-77.581,0.0,79.169],[-77.992,0.0,78.742],[-78.450,0.0,78.267],[-78.953,0.0,77.746],[-79.498,0.0,77.181],[-80.084,0.0,76.574],[-80.709,0.0,75.926],[-81.371,0.0,75.240],[-81.890,0.0,74.701],[-82.247,0.0,74.331],[-82.612,0.0,73.953],[-82.985,0.0,73.567],[-83.366,0.0,73.172],[-83.754,0.0,72.770],[-84.149,0.0,72.361],[-84.551,0.0,71.944],[-84.959,0.0,71.520],[-85.374,0.0,71.090],[-85.796,0.0,70.653],[-86.223,0.0,70.210],[-86.656,0.0,69.762],[-87.094,0.0,69.307],[-87.538,0.0,68.847],[-87.986,0.0,68.382],[-88.440,0.0,67.913],[-88.897,0.0,67.438],[-89.360,0.0,66.959],[-89.826,0.0,66.476],[-90.295,0.0,65.989],[-90.769,0.0,65.498],[-91.245,0.0,65.004],[-91.725,0.0,64.507],[-92.208,0.0,64.007],[-92.693,0.0,63.504],[-93.180,0.0,62.999],[-93.669,0.0,62.492],[-94.160,0.0,61.983],[-94.653,0.0,61.472],[-95.147,0.0,60.960],[-95.642,0.0,60.447],[-96.139,0.0,59.932],[-96.635,0.0,59.418],[-97.132,0.0,58.902],[-97.629,0.0,58.387],[-98.126,0.0,57.872],[-98.623,0.0,57.357],[-99.119,0.0,56.843],[-99.614,0.0,56.330],[-100.108,0.0,55.817],[-100.601,0.0,55.307],[-101.092,0.0,54.798],[-101.582,0.0,54.290],[-102.069,0.0,53.785],[-102.554,0.0,53.282],[-103.036,0.0,52.782],[-103.516,0.0,52.285],[-103.993,0.0,51.791],[-104.466,0.0,51.300],[-104.936,0.0,50.813],[-105.402,0.0,50.330],[-105.864,0.0,49.851],[-106.322,0.0,49.377],[-106.775,0.0,48.907],[-107.224,0.0,48.442],[-107.667,0.0,47.982],[-108.106,0.0,47.528],[-108.539,0.0,47.079],[-108.966,0.0,46.636],[-109.387,0.0,46.199],[-109.802,0.0,45.769],[-110.211,0.0,45.346],[-110.613,0.0,44.929],[-111.008,0.0,44.519],[-111.396,0.0,44.117],[-111.776,0.0,43.723],[-112.149,0.0,43.336],[-112.514,0.0,42.958],[-112.871,0.0,42.588],[-113.391,0.0,42.050],[-114.052,0.0,41.364],[-114.677,0.0,40.716],[-115.264,0.0,40.108],[-115.809,0.0,39.543],[-116.312,0.0,39.022],[-116.770,0.0,38.547],[-117.181,0.0,38.121],[-117.544,0.0,37.745],[-118.116,0.0,37.152],[-118.592,0.0,36.658]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        south,
        road_points_from_json(
            "[[-118.592,0.0,36.658],[-118.710,0.0,35.936],[-118.818,0.0,35.275],[-118.957,0.0,34.423],[-119.080,0.0,33.668],[-119.170,0.0,33.114],[-119.267,0.0,32.520],[-119.370,0.0,31.889],[-119.479,0.0,31.223],[-119.593,0.0,30.524],[-119.712,0.0,29.793],[-119.836,0.0,29.033],[-119.964,0.0,28.246],[-120.097,0.0,27.432],[-120.233,0.0,26.595],[-120.373,0.0,25.736],[-120.517,0.0,24.857],[-120.663,0.0,23.960],[-120.812,0.0,23.046],[-120.963,0.0,22.119],[-121.116,0.0,21.179],[-121.271,0.0,20.228],[-121.428,0.0,19.269],[-121.585,0.0,18.304],[-121.743,0.0,17.333],[-121.902,0.0,16.360],[-122.061,0.0,15.386],[-122.220,0.0,14.413],[-122.378,0.0,13.442],[-122.535,0.0,12.477],[-122.692,0.0,11.518],[-122.847,0.0,10.567],[-123.000,0.0,9.627],[-123.151,0.0,8.700],[-123.300,0.0,7.786],[-123.446,0.0,6.889],[-123.590,0.0,6.010],[-123.730,0.0,5.151],[-123.866,0.0,4.314],[-123.999,0.0,3.501],[-124.127,0.0,2.713],[-124.251,0.0,1.953],[-124.370,0.0,1.222],[-124.484,0.0,0.523],[-124.593,0.0,-0.143],[-124.696,0.0,-0.774],[-124.792,0.0,-1.367],[-124.883,0.0,-1.922],[-125.006,0.0,-2.677],[-125.145,0.0,-3.529],[-125.253,0.0,-4.190],[-125.370,0.0,-4.912]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_compiled_bend_piece(&surface, bend);
}

#[test]
fn logged_elevated_bend_rejects_implicit_cross_owner_cdt_height_edge() {
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
    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&bend),
        "elevated bend must reject implicit cross-owner CDT height sharing until join ownership is legal"
    );
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
        .expect("angled terminal should compile a terminal piece");
    let end_terminal_piece = surface
        .compiled_visual_node_pieces()
        .get(&end)
        .expect("opposite angled terminal should compile a terminal piece");
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
            point_inside_visual_polygons(&terminal_piece.curb_surface_polygons, curb_mid),
            "angled terminal curb strip must be owned by curb surface on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&terminal_piece.road_surface_polygons, curb_mid),
            "terminal curb strip must not be owned by asphalt on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&span_piece.curb_surface_polygons, curb_mid),
            "terminal curb strip must not be duplicated by the span on side {side}; point={curb_mid:?}"
        );
    }

    let end_travel = Vector2::new(-40.0, -5.0).normalized();
    let end_lateral = RoadSurfaceSystem::left_normal_xz(end_travel);
    let end_center = Vector2::new(40.0, 5.0);
    for side in [-1.0, 1.0] {
        let curb_mid = end_center + end_lateral * side * 3.575;
        assert!(
            point_inside_visual_polygons(&end_terminal_piece.curb_surface_polygons, curb_mid),
            "opposite angled terminal curb strip must be owned by curb surface on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&end_terminal_piece.road_surface_polygons, curb_mid),
            "opposite terminal curb strip must not be owned by asphalt on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&span_piece.curb_surface_polygons, curb_mid),
            "opposite terminal curb strip must not be duplicated by the span on side {side}; point={curb_mid:?}"
        );
    }
}

#[test]
fn logged_oblique_terminal_top_surfaces_cover_footprint() {
    let terrain = flat_terrain(256, 256);
    let points = road_points_from_json(
        "[[56.267,0.0,-24.078],[57.235,0.0,-24.012],[58.162,0.0,-23.950],\
        [59.047,0.0,-23.890],[59.889,0.0,-23.833],[60.687,0.0,-23.779],\
        [61.440,0.0,-23.728],[62.147,0.0,-23.680],[62.808,0.0,-23.635],\
        [63.421,0.0,-23.594],[63.985,0.0,-23.556],[64.501,0.0,-23.521],\
        [65.379,0.0,-23.462],[66.049,0.0,-23.416],[66.762,0.0,-23.368]]",
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();

    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
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
    for node_id in [start, end] {
        let terminal_piece = surface
            .compiled_visual_node_pieces()
            .get(&node_id)
            .expect("logged oblique road endpoint should compile a terminal piece");
        assert_eq!(
            terminal_piece.kind,
            RoadSurfaceVisualNodePieceKind::Terminal
        );
        assert_node_top_covers_footprint(terminal_piece);
    }
}

#[test]
fn logged_curved_terminal_top_surfaces_cover_footprint() {
    let terrain = flat_terrain(384, 384);
    let points = road_points_from_json(
        "[[-52.080,0.0,25.947],[-52.858,0.0,26.111],[-53.527,0.0,26.253],\
        [-54.079,0.0,26.370],[-54.711,0.0,26.503],[-55.422,0.0,26.654],\
        [-56.206,0.0,26.820],[-57.063,0.0,27.001],[-57.987,0.0,27.197],\
        [-58.723,0.0,27.352],[-59.233,0.0,27.460],[-59.759,0.0,27.572],\
        [-60.299,0.0,27.686],[-60.854,0.0,27.803],[-61.424,0.0,27.924],\
        [-62.006,0.0,28.047],[-62.602,0.0,28.173],[-63.211,0.0,28.302],\
        [-63.833,0.0,28.434],[-64.466,0.0,28.568],[-65.111,0.0,28.704],\
        [-65.768,0.0,28.843],[-66.435,0.0,28.984],[-67.113,0.0,29.128],\
        [-67.801,0.0,29.273],[-68.499,0.0,29.421],[-69.206,0.0,29.571],\
        [-69.922,0.0,29.722],[-70.646,0.0,29.875],[-71.379,0.0,30.030],\
        [-72.119,0.0,30.187],[-72.867,0.0,30.345],[-73.621,0.0,30.505],\
        [-74.382,0.0,30.666],[-75.150,0.0,30.828],[-75.923,0.0,30.992],\
        [-76.701,0.0,31.157],[-77.484,0.0,31.323],[-78.272,0.0,31.489],\
        [-79.064,0.0,31.657],[-79.860,0.0,31.825],[-80.659,0.0,31.994],\
        [-81.461,0.0,32.164],[-82.266,0.0,32.334],[-83.073,0.0,32.505],\
        [-83.882,0.0,32.676],[-84.692,0.0,32.848],[-85.503,0.0,33.019],\
        [-86.315,0.0,33.191],[-87.126,0.0,33.363],[-87.938,0.0,33.535],\
        [-88.749,0.0,33.706],[-89.559,0.0,33.878],[-90.368,0.0,34.049],\
        [-91.175,0.0,34.220],[-91.980,0.0,34.390],[-92.782,0.0,34.560],\
        [-93.581,0.0,34.729],[-94.377,0.0,34.897],[-95.169,0.0,35.065],\
        [-95.957,0.0,35.232],[-96.740,0.0,35.397],[-97.518,0.0,35.562],\
        [-98.292,0.0,35.726],[-99.059,0.0,35.888],[-99.820,0.0,36.049],\
        [-100.575,0.0,36.209],[-101.322,0.0,36.367],[-102.062,0.0,36.524],\
        [-102.795,0.0,36.679],[-103.520,0.0,36.832],[-104.235,0.0,36.983],\
        [-104.942,0.0,37.133],[-105.640,0.0,37.281],[-106.328,0.0,37.426],\
        [-107.006,0.0,37.570],[-107.673,0.0,37.711],[-108.330,0.0,37.850],\
        [-108.975,0.0,37.986],[-109.609,0.0,38.120],[-110.230,0.0,38.252],\
        [-110.839,0.0,38.381],[-111.435,0.0,38.507],[-112.018,0.0,38.630],\
        [-112.587,0.0,38.751],[-113.142,0.0,38.868],[-113.682,0.0,38.982],\
        [-114.208,0.0,39.094],[-114.718,0.0,39.202],[-115.454,0.0,39.357],\
        [-116.379,0.0,39.553],[-117.235,0.0,39.734],[-118.020,0.0,39.900],\
        [-118.730,0.0,40.051],[-119.362,0.0,40.184],[-119.914,0.0,40.301],\
        [-120.583,0.0,40.443],[-121.361,0.0,40.607]]",
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();

    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    let mut edge = test_edge(
        start,
        end,
        points,
        14.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    );
    edge.fwd_lanes = 2;
    edge.bkw_lanes = 2;
    graph.add_edge(edge);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[0]);
    for node_id in [start, end] {
        let terminal_piece = surface
            .compiled_visual_node_pieces()
            .get(&node_id)
            .unwrap_or_else(|| panic!("logged curved road endpoint should compile a terminal piece; node_id={node_id} dump={dump}"));
        assert_eq!(
            terminal_piece.kind,
            RoadSurfaceVisualNodePieceKind::Terminal
        );
        assert_node_top_covers_footprint(terminal_piece);
        assert_material_triangles_do_not_overlap(terminal_piece);
    }
}

#[test]
fn logged_terminal_with_tiny_boundary_dust_compiles_both_terminals() {
    let terrain = flat_terrain(256, 256);
    let points = road_points_from_json(
        r#"[[98.445,0.0,22.22],[98.058,0.0,22.613],[97.589,0.0,23.089],[97.18,0.0,23.504],[96.698,0.0,23.994],[96.145,0.0,24.556],[95.524,0.0,25.186],[95.015,0.0,25.703],[94.656,0.0,26.067],[94.282,0.0,26.447],[93.892,0.0,26.843],[93.488,0.0,27.253],[93.07,0.0,27.678],[92.637,0.0,28.117],[92.191,0.0,28.57],[91.731,0.0,29.037],[91.259,0.0,29.517],[90.774,0.0,30.009],[90.276,0.0,30.514],[89.767,0.0,31.032],[89.246,0.0,31.561],[88.713,0.0,32.101],[88.17,0.0,32.653],[87.616,0.0,33.215],[87.052,0.0,33.788],[86.478,0.0,34.371],[85.895,0.0,34.963],[85.302,0.0,35.565],[84.7,0.0,36.176],[84.09,0.0,36.795],[83.472,0.0,37.423],[82.846,0.0,38.059],[82.213,0.0,38.702],[81.572,0.0,39.352],[80.925,0.0,40.009],[80.271,0.0,40.673],[79.612,0.0,41.343],[78.946,0.0,42.018],[78.275,0.0,42.7],[77.599,0.0,43.386],[76.919,0.0,44.077],[76.234,0.0,44.772],[75.545,0.0,45.472],[74.853,0.0,46.175],[74.157,0.0,46.881],[73.458,0.0,47.591],[72.932,0.0,48.125],[72.581,0.0,48.481],[72.229,0.0,48.839],[71.877,0.0,49.196],[71.524,0.0,49.554],[71.171,0.0,49.913],[70.818,0.0,50.272],[70.464,0.0,50.631],[70.11,0.0,50.991],[69.755,0.0,51.351],[69.401,0.0,51.711],[69.046,0.0,52.071],[68.691,0.0,52.431],[68.336,0.0,52.791],[67.981,0.0,53.152],[67.626,0.0,53.512],[67.272,0.0,53.872],[66.917,0.0,54.233],[66.562,0.0,54.593],[66.208,0.0,54.953],[65.854,0.0,55.312],[65.5,0.0,55.671],[65.146,0.0,56.03],[64.793,0.0,56.389],[64.44,0.0,56.747],[64.088,0.0,57.105],[63.736,0.0,57.462],[63.385,0.0,57.819],[62.859,0.0,58.353],[62.161,0.0,59.062],[61.465,0.0,59.768],[60.772,0.0,60.472],[60.083,0.0,61.171],[59.399,0.0,61.866],[58.718,0.0,62.557],[58.042,0.0,63.244],[57.371,0.0,63.925],[56.706,0.0,64.601],[56.046,0.0,65.27],[55.392,0.0,65.934],[54.745,0.0,66.591],[54.105,0.0,67.242],[53.471,0.0,67.885],[52.845,0.0,68.52],[52.227,0.0,69.148],[51.617,0.0,69.767],[51.016,0.0,70.378],[50.423,0.0,70.98],[49.84,0.0,71.572],[49.266,0.0,72.155],[48.702,0.0,72.728],[48.148,0.0,73.29],[47.604,0.0,73.842],[47.072,0.0,74.382],[46.551,0.0,74.912],[46.041,0.0,75.429],[45.544,0.0,75.934],[45.059,0.0,76.427],[44.586,0.0,76.907],[44.126,0.0,77.373],[43.68,0.0,77.826],[43.248,0.0,78.266],[42.829,0.0,78.69],[42.425,0.0,79.101],[42.036,0.0,79.496],[41.661,0.0,79.876],[41.302,0.0,80.241],[40.794,0.0,80.757],[40.173,0.0,81.388],[39.62,0.0,81.949],[39.137,0.0,82.439],[38.729,0.0,82.854],[38.259,0.0,83.331],[37.872,0.0,83.724]]"#,
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();

    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    let mut edge = test_edge(
        start,
        end,
        points,
        3.5,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    );
    edge.bkw_lanes = 0;
    graph.add_edge(edge);

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    for node_id in [start, end] {
        let terminal_piece = surface
            .compiled_visual_node_pieces()
            .get(&node_id)
            .expect("logged terminal endpoint should compile a terminal piece");
        assert_eq!(
            terminal_piece.kind,
            RoadSurfaceVisualNodePieceKind::Terminal
        );
        assert_node_top_covers_footprint(terminal_piece);
        assert_material_triangles_do_not_overlap(terminal_piece);
    }
}

#[test]
fn logged_terminal_handoff_keeps_both_sidewalk_edges_owned() {
    let terrain = flat_terrain(128, 128);
    let points = road_points_from_json(
        "[[-67.97,0.0,12.333],[-67.147,0.0,12.502],[-66.439,0.0,12.648],\
        [-65.855,0.0,12.769],[-65.186,0.0,12.907],[-64.435,0.0,13.061],\
        [-63.605,0.0,13.232],[-62.699,0.0,13.419],[-61.972,0.0,13.569],\
        [-61.466,0.0,13.673],[-60.942,0.0,13.781],[-60.402,0.0,13.892],\
        [-59.846,0.0,14.007],[-59.274,0.0,14.125],[-58.687,0.0,14.246],\
        [-58.085,0.0,14.37],[-57.469,0.0,14.497],[-56.838,0.0,14.627],\
        [-56.194,0.0,14.76],[-55.537,0.0,14.895],[-54.867,0.0,15.033],\
        [-54.184,0.0,15.174],[-53.49,0.0,15.317],[-52.783,0.0,15.463],\
        [-52.066,0.0,15.61],[-51.339,0.0,15.76],[-50.6,0.0,15.912],\
        [-49.852,0.0,16.067],[-49.095,0.0,16.223],[-48.329,0.0,16.381],\
        [-47.554,0.0,16.54],[-46.77,0.0,16.702],[-45.979,0.0,16.865],\
        [-45.181,0.0,17.029],[-44.376,0.0,17.195],[-43.564,0.0,17.362],\
        [-42.746,0.0,17.531],[-41.923,0.0,17.701],[-41.094,0.0,17.871],\
        [-40.261,0.0,18.043],[-39.423,0.0,18.216],[-38.581,0.0,18.389],\
        [-37.736,0.0,18.564],[-36.887,0.0,18.739],[-36.036,0.0,18.914],\
        [-35.182,0.0,19.09],[-34.326,0.0,19.266],[-33.469,0.0,19.443],\
        [-32.611,0.0,19.62],[-31.753,0.0,19.797],[-30.894,0.0,19.974],\
        [-30.035,0.0,20.151],[-29.177,0.0,20.327],[-28.32,0.0,20.504],\
        [-27.465,0.0,20.68],[-26.611,0.0,20.856],[-25.76,0.0,21.032],\
        [-24.911,0.0,21.207],[-24.065,0.0,21.381],[-23.224,0.0,21.554],\
        [-22.386,0.0,21.727],[-21.552,0.0,21.899],[-20.723,0.0,22.07],\
        [-19.9,0.0,22.239],[-19.082,0.0,22.408],[-18.27,0.0,22.575],\
        [-17.465,0.0,22.741],[-16.667,0.0,22.906],[-15.876,0.0,23.069],\
        [-15.093,0.0,23.23],[-14.318,0.0,23.39],[-13.551,0.0,23.548],\
        [-12.794,0.0,23.704],[-12.046,0.0,23.858],[-11.308,0.0,24.01],\
        [-10.58,0.0,24.16],[-9.863,0.0,24.308],[-9.157,0.0,24.453],\
        [-8.462,0.0,24.596],[-7.78,0.0,24.737],[-7.11,0.0,24.875],\
        [-6.452,0.0,25.011],[-5.808,0.0,25.143],[-5.178,0.0,25.273],\
        [-4.561,0.0,25.4],[-3.959,0.0,25.524],[-3.372,0.0,25.645],\
        [-2.8,0.0,25.763],[-2.244,0.0,25.878],[-1.704,0.0,25.989],\
        [-1.181,0.0,26.097],[-0.674,0.0,26.201],[0.052,0.0,26.351],\
        [0.958,0.0,26.538],[1.788,0.0,26.709],[2.54,0.0,26.864],\
        [3.209,0.0,27.002],[3.793,0.0,27.122],[4.5,0.0,27.268],\
        [5.323,0.0,27.437]]",
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();

    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    let mut edge = test_edge(
        start,
        end,
        points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    );
    edge.fwd_lanes = 2;
    edge.bkw_lanes = 0;
    let edge_idx = graph.add_edge(edge);

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("logged terminal road should keep a visible span after terminal handoff");
    let start_terminal = surface
        .compiled_visual_node_pieces()
        .get(&start)
        .expect("logged terminal road start should compile a terminal piece");
    let start_mouth = span_piece
        .start_mouth_profile
        .as_ref()
        .expect("logged terminal span should expose a start mouth profile");
    let start_endpoint = RoadSurfaceSystem::build_mouth_profile_from_section(
        surface
            .compiled_sections()
            .get(&edge_idx)
            .and_then(|sections| sections.first())
            .expect("logged terminal road should compile endpoint sections"),
        super::IncidentEdgeSide::Start,
    )
    .expect("logged terminal endpoint section should expose a profile");

    assert_terminal_mouth_handoff_surface_is_owned(
        start_terminal,
        start_mouth,
        RoadSurfaceBandKind::CurbOrShoulder,
        4,
        5,
        "right curb at logged terminal handoff",
    );
    assert_terminal_mouth_handoff_surface_is_owned(
        start_terminal,
        start_mouth,
        RoadSurfaceBandKind::Sidewalk,
        5,
        6,
        "right sidewalk at logged terminal handoff",
    );
    assert_terminal_band_interval_grid_is_owned(
        start_terminal,
        &start_endpoint,
        start_mouth,
        RoadSurfaceBandKind::CurbOrShoulder,
        4,
        5,
        "right curb interval at logged terminal start",
    );
    assert_terminal_band_interval_grid_is_owned(
        start_terminal,
        &start_endpoint,
        start_mouth,
        RoadSurfaceBandKind::Sidewalk,
        5,
        6,
        "right sidewalk interval at logged terminal start",
    );
    assert_terminal_band_interval_grid_is_not_duplicated_by_span(
        span_piece,
        &start_endpoint,
        start_mouth,
        4,
        5,
        "right curb interval at logged terminal start",
    );
    assert_terminal_band_interval_grid_is_not_duplicated_by_span(
        span_piece,
        &start_endpoint,
        start_mouth,
        5,
        6,
        "right sidewalk interval at logged terminal start",
    );
    assert_vertical_curb_face_lower_edge_covers(
        &start_terminal.curb_vertical_face_polygons,
        start_endpoint.boundary_points_world[2],
        start_mouth.boundary_points_world[2],
        "left longitudinal curb face at logged terminal handoff",
    );
    assert_vertical_curb_face_lower_edge_covers(
        &start_terminal.curb_vertical_face_polygons,
        start_endpoint.boundary_points_world[4],
        start_mouth.boundary_points_world[4],
        "right longitudinal curb face at logged terminal handoff",
    );
}

#[test]
fn straight_terminal_keeps_curb_strip_covered_on_both_sides() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(40.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[edge_idx]);
    assert_debug_dump_mouth_seams_are_clean(&dump);
    let terminal_piece = surface
        .compiled_visual_node_pieces()
        .get(&start)
        .expect("straight terminal should compile a terminal piece");
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("terminal road should keep a visible span after terminal handoff");
    let start_mouth = span_piece
        .start_mouth_profile
        .as_ref()
        .expect("terminal span should expose a start mouth profile");

    let left_curb_upper = start_mouth.bands[1].end_point_world;
    let left_road_lower = start_mouth.bands[2].start_point_world;
    assert_eq!(test_xz_key(left_curb_upper), test_xz_key(left_road_lower));
    assert!(
        (left_curb_upper.y - left_road_lower.y - CURB_STEP_HEIGHT_M).abs() <= 0.004,
        "left asphalt-curb mouth seam should keep the explicit vertical step"
    );
    assert_material_top_supports_point(
        &terminal_piece.curb_surface_polygons,
        left_curb_upper,
        "straight terminal left curb upper mouth seam",
    );
    assert_material_top_supports_point(
        &terminal_piece.road_surface_polygons,
        left_road_lower,
        "straight terminal left asphalt lower mouth seam",
    );

    let right_road_lower = start_mouth.bands[3].end_point_world;
    let right_curb_upper = start_mouth.bands[4].start_point_world;
    assert_eq!(test_xz_key(right_road_lower), test_xz_key(right_curb_upper));
    assert!(
        (right_curb_upper.y - right_road_lower.y - CURB_STEP_HEIGHT_M).abs() <= 0.004,
        "right asphalt-curb mouth seam should keep the explicit vertical step"
    );
    assert_material_top_supports_point(
        &terminal_piece.road_surface_polygons,
        right_road_lower,
        "straight terminal right asphalt lower mouth seam",
    );
    assert_material_top_supports_point(
        &terminal_piece.curb_surface_polygons,
        right_curb_upper,
        "straight terminal right curb upper mouth seam",
    );

    let travel = Vector2::new(40.0, 0.0).normalized();
    let lateral = RoadSurfaceSystem::left_normal_xz(travel);
    let center = Vector2::new(0.0, 0.0);
    for side in [-1.0, 1.0] {
        let curb_mid = center + lateral * side * 3.575;
        assert!(
            point_inside_visual_polygons(&terminal_piece.curb_surface_polygons, curb_mid),
            "straight terminal curb strip must be owned by curb surface on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&terminal_piece.road_surface_polygons, curb_mid),
            "terminal curb strip must not be owned by asphalt on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&span_piece.curb_surface_polygons, curb_mid),
            "terminal curb strip must not be duplicated by the span on side {side}; point={curb_mid:?}"
        );
    }
    assert_terminal_mouth_handoff_surface_is_owned(
        terminal_piece,
        start_mouth,
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        2,
        "left curb at handoff",
    );
    assert_terminal_mouth_handoff_surface_is_owned(
        terminal_piece,
        start_mouth,
        RoadSurfaceBandKind::Sidewalk,
        0,
        1,
        "left sidewalk at handoff",
    );
    assert_terminal_mouth_handoff_surface_is_owned(
        terminal_piece,
        start_mouth,
        RoadSurfaceBandKind::CurbOrShoulder,
        4,
        5,
        "right curb at handoff",
    );
    assert_terminal_mouth_handoff_surface_is_owned(
        terminal_piece,
        start_mouth,
        RoadSurfaceBandKind::Sidewalk,
        5,
        6,
        "right sidewalk at handoff",
    );
}

#[test]
fn steep_standard_terminal_compiles_legal_height_ownership() {
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
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&start),
        "steep terminal should compile with explicit terminal cap height ownership"
    );
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&end),
        "opposite steep terminal should compile with explicit terminal cap height ownership"
    );
}

#[test]
fn logged_one_way_elevated_terminal_compiles_mouth_vertical_step() {
    let terrain = coarse_hillside_world_terrain(1025, 1025, 1.0);
    let points = road_points_from_json(
        r#"[[-155.772,156.540,-191.943],[-156.491,156.482,-192.388],[-157.109,156.423,-192.771],[-157.619,156.362,-193.087],[-158.203,156.296,-193.449],[-158.860,156.222,-193.855],[-159.585,156.137,-194.304],[-160.376,156.043,-194.794],[-161.011,155.947,-195.188],[-161.453,155.853,-195.462],[-161.910,155.764,-195.745],[-162.382,155.680,-196.037],[-162.867,155.601,-196.338],[-163.367,155.523,-196.647],[-163.880,155.444,-196.965],[-164.405,155.365,-197.290],[-164.944,155.284,-197.624],[-165.494,155.204,-197.965],[-166.057,155.125,-198.313],[-166.631,155.051,-198.669],[-167.216,154.982,-199.031],[-167.813,154.918,-199.401],[-168.419,154.855,-199.776],[-169.036,154.783,-200.158],[-169.662,154.694,-200.546],[-170.298,154.581,-200.940],[-170.942,154.441,-201.339],[-171.596,154.277,-201.744],[-172.257,154.096,-202.154],[-172.927,153.906,-202.568],[-173.603,153.712,-202.987],[-174.288,153.515,-203.411],[-174.978,153.316,-203.839],[-175.676,153.116,-204.271],[-176.379,152.915,-204.706],[-177.088,152.712,-205.145],[-177.802,152.504,-205.588],[-178.521,152.284,-206.033],[-179.245,152.041,-206.482],[-179.973,151.768,-206.933],[-180.705,151.462,-207.386],[-181.440,151.127,-207.841],[-182.178,150.771,-208.299],[-182.920,150.408,-208.758],[-183.663,150.046,-209.218],[-184.409,149.692,-209.680],[-185.156,149.347,-210.143],[-185.904,149.010,-210.606],[-186.654,148.678,-211.071],[-187.404,148.354,-211.535],[-188.154,148.039,-212.000],[-188.904,147.740,-212.464],[-189.653,147.462,-212.928],[-190.402,147.207,-213.392],[-191.149,146.976,-213.855],[-191.895,146.764,-214.317],[-192.638,146.563,-214.777],[-193.379,146.368,-215.236],[-194.118,146.175,-215.694],[-194.853,145.982,-216.149],[-195.585,145.789,-216.602],[-196.313,145.597,-217.053],[-197.037,145.405,-217.501],[-197.756,145.216,-217.947],[-198.470,145.037,-218.389],[-199.179,144.873,-218.828],[-199.882,144.733,-219.264],[-200.579,144.619,-219.696],[-201.270,144.528,-220.124],[-201.954,144.453,-220.547],[-202.631,144.384,-220.967],[-203.301,144.311,-221.381],[-203.962,144.230,-221.791],[-204.615,144.139,-222.196],[-205.260,144.039,-222.595],[-205.896,143.931,-222.989],[-206.522,143.815,-223.376],[-207.139,143.690,-223.758],[-207.745,143.558,-224.134],[-208.341,143.422,-224.503],[-208.927,143.292,-224.866],[-209.501,143.176,-225.221],[-210.063,143.082,-225.570],[-210.614,143.013,-225.911],[-211.152,142.962,-226.245],[-211.678,142.923,-226.570],[-212.191,142.885,-226.888],[-212.690,142.844,-227.197],[-213.176,142.797,-227.498],[-213.648,142.742,-227.790],[-214.105,142.677,-228.073],[-214.547,142.600,-228.347],[-214.974,142.509,-228.612],[-215.585,142.404,-228.990],[-216.344,142.290,-229.460],[-217.035,142.169,-229.888],[-217.656,142.046,-230.273],[-218.203,141.921,-230.612],[-218.675,141.794,-230.904],[-219.786,141.665,-231.592]]"#,
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();
    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    let mut edge = test_edge(
        start,
        end,
        points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    );
    edge.fwd_lanes = 2;
    edge.bkw_lanes = 0;
    graph.add_edge(edge);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&start),
        "start terminal should compile with explicit mouth asphalt-curb height ownership"
    );
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&end),
        "end terminal should compile with explicit mouth asphalt-curb height ownership"
    );
}

#[test]
fn logged_two_lane_elevated_terminal_compiles_endpoint_vertical_step() {
    let terrain = coarse_hillside_world_terrain(1025, 1025, 1.0);
    let points = road_points_from_json(
        r#"[[-92.509,156.967,-122.123],[-92.307,156.897,-121.439],[-92.065,156.823,-120.618],[-91.912,156.738,-120.097],[-91.738,156.638,-119.507],[-91.544,156.520,-118.849],[-91.331,156.383,-118.128],[-91.101,156.233,-117.345],[-90.853,156.080,-116.504],[-90.588,155.934,-115.607],[-90.380,155.802,-114.899],[-90.236,155.687,-114.411],[-90.088,155.585,-113.911],[-89.937,155.491,-113.399],[-89.783,155.399,-112.875],[-89.625,155.304,-112.340],[-89.464,155.206,-111.794],[-89.300,155.108,-111.237],[-89.133,155.012,-110.670],[-88.963,154.919,-110.093],[-88.790,154.831,-109.506],[-88.615,154.747,-108.911],[-88.436,154.664,-108.306],[-88.256,154.581,-107.693],[-88.072,154.497,-107.071],[-87.887,154.412,-106.442],[-87.699,154.325,-105.805],[-87.510,154.237,-105.162],[-87.318,154.148,-104.511],[-87.124,154.057,-103.854],[-86.929,153.965,-103.191],[-86.731,153.872,-102.522],[-86.533,153.779,-101.847],[-86.332,153.687,-101.168],[-86.131,153.599,-100.484],[-85.928,153.516,-99.795],[-85.724,153.438,-99.103],[-85.519,153.362,-98.407],[-85.312,153.288,-97.707],[-85.105,153.213,-97.005],[-84.898,153.135,-96.300],[-84.689,153.053,-95.592],[-84.480,152.969,-94.883],[-84.271,152.882,-94.172],[-84.061,152.792,-93.461],[-83.851,152.700,-92.748],[-83.640,152.605,-92.034],[-83.430,152.510,-91.321],[-83.220,152.414,-90.607],[-83.010,152.319,-89.894],[-82.800,152.225,-89.182],[-82.590,152.134,-88.472],[-82.381,152.045,-87.763],[-82.173,151.956,-87.055],[-81.965,151.868,-86.350],[-81.758,151.780,-85.648],[-81.552,151.693,-84.948],[-81.347,151.606,-84.252],[-81.143,151.520,-83.560],[-80.940,151.435,-82.871],[-80.738,151.352,-82.187],[-80.538,151.268,-81.508],[-80.339,151.182,-80.833],[-80.142,151.093,-80.164],[-79.946,151.000,-79.501],[-79.753,150.903,-78.844],[-79.561,150.803,-78.193],[-79.371,150.703,-77.550],[-79.183,150.604,-76.913],[-78.998,150.506,-76.284],[-78.815,150.411,-75.662],[-78.634,150.316,-75.049],[-78.456,150.223,-74.444],[-78.280,150.132,-73.849],[-78.107,150.044,-73.262],[-77.937,149.959,-72.685],[-77.770,149.878,-72.118],[-77.606,149.800,-71.561],[-77.445,149.721,-71.015],[-77.287,149.633,-70.480],[-77.133,149.530,-69.956],[-76.982,149.400,-69.444],[-76.835,149.239,-68.944],[-76.691,149.043,-68.456],[-76.482,148.816,-67.748],[-76.218,148.570,-66.851],[-75.970,148.320,-66.010],[-75.739,148.081,-65.227],[-75.527,147.860,-64.506],[-75.333,147.658,-63.848],[-75.159,147.468,-63.258],[-75.005,147.280,-62.737],[-74.763,147.089,-61.916],[-74.562,146.893,-61.232]]"#,
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();
    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    let mut edge = test_edge(
        start,
        end,
        points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    );
    edge.fwd_lanes = 1;
    edge.bkw_lanes = 1;
    graph.add_edge(edge);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&start),
        "start terminal should compile with explicit terminal endpoint asphalt-curb height ownership"
    );
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&end),
        "end terminal should compile with explicit terminal endpoint asphalt-curb height ownership"
    );
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
    assert!(!span_piece.curb_surface_polygons.is_empty());
    assert!(!span_piece.curb_vertical_face_polygons.is_empty());
    assert!(!span_piece.sidewalk_surface_polygons.is_empty());
    assert!(
        span_piece
            .road_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .curb_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece.curb_surface_polygons.iter().all(|polygon| {
            polygon.triangles_world.iter().all(|triangle| {
                let min_y = triangle[0].y.min(triangle[1].y).min(triangle[2].y);
                let max_y = triangle[0].y.max(triangle[1].y).max(triangle[2].y);
                max_y - min_y <= 0.001
            })
        }),
        "curb top surface must be flat; vertical drop belongs to explicit curb faces"
    );
    assert!(
        span_piece
            .curb_vertical_face_polygons
            .iter()
            .all(|polygon| !RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
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
fn terrain_clip_union_preserves_endpoint_owned_numeric_connector() {
    let y = 12.0;
    let points = vec![
        Vector3::new(0.0, y, 0.0),
        Vector3::new(0.5, y, 0.0),
        Vector3::new(0.501, y, 0.0001),
        Vector3::new(1.0, y, 0.0),
        Vector3::new(1.0, y, 0.1),
        Vector3::new(0.0, y, 0.1),
    ];
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(points[0], points[1]),
            terrain_clip_source_edge_for_test(points[2], points[3]),
            terrain_clip_source_edge_for_test(points[3], points[4]),
            terrain_clip_source_edge_for_test(points[4], points[5]),
            terrain_clip_source_edge_for_test(points[5], points[0]),
        ],
        points_world: points,
    }];

    let clip_polygons = RoadSurfaceSystem::union_terrain_clip_boundary_loops(&raw_clip_sources);

    assert_eq!(
        clip_polygons.len(),
        1,
        "unioned terrain clip contour must keep source-owned endpoint continuity instead of dropping the road cutter"
    );
    assert!(
        clip_polygons[0]
            .points_world
            .iter()
            .all(|point| (point.y - y).abs() <= SAMPLE_EPSILON_M),
        "accepted connector must reuse canonical source endpoint heights"
    );
}

#[test]
fn terrain_clip_union_preserves_boundary_only_connector_by_interpolation() {
    let raw_boundary_y = -99.0;
    let p0 = Vector3::new(0.0, 10.0, 0.0);
    let p1 = Vector3::new(0.5, 10.5, 0.0);
    let d0 = Vector3::new(0.50002, raw_boundary_y, 0.00008);
    let d1 = Vector3::new(0.49998, raw_boundary_y, 0.00016);
    let d2 = Vector3::new(0.50001, raw_boundary_y, 0.00024);
    let p2 = Vector3::new(0.5, 10.7, 0.00032);
    let p3 = Vector3::new(1.0, 11.0, 0.0);
    let p4 = Vector3::new(1.0, 11.0, 0.1);
    let p5 = Vector3::new(0.0, 10.0, 0.1);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p1),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p4),
            terrain_clip_source_edge_for_test(p4, p5),
            terrain_clip_source_edge_for_test(p5, p0),
        ],
        points_world: vec![
            Vector3::new(p0.x, raw_boundary_y, p0.z),
            Vector3::new(p1.x, raw_boundary_y, p1.z),
            d0,
            d1,
            d2,
            Vector3::new(p2.x, raw_boundary_y, p2.z),
            Vector3::new(p3.x, raw_boundary_y, p3.z),
            Vector3::new(p4.x, raw_boundary_y, p4.z),
            Vector3::new(p5.x, raw_boundary_y, p5.z),
        ],
    }];

    let clip_polygons = RoadSurfaceSystem::union_terrain_clip_boundary_loops(&raw_clip_sources);

    assert_eq!(
        clip_polygons.len(),
        1,
        "unioned terrain clip cutter must survive a sub-budget boundary-only connector"
    );
    assert!(
        RoadSurfaceSystem::polygon_has_area_xz(&clip_polygons[0].points_world),
        "preserved terrain clip cutter must remain a valid road footprint polygon"
    );
    assert!(
        clip_polygons[0]
            .points_world
            .iter()
            .all(|point| (point.y - raw_boundary_y).abs() > SAMPLE_EPSILON_M),
        "boundary-only connector heights must come from solved source contour interpolation"
    );
    assert!(
        clip_polygons[0]
            .points_world
            .iter()
            .any(|point| point.y > p1.y && point.y < p2.y),
        "sub-budget connector must carry interpolated seam heights between adjacent solved footprint vertices"
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
fn visual_node_rejection_is_deterministic_for_multi_arm_nodes() {
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

    assert!(
        !surface_a
            .compiled_visual_node_pieces()
            .contains_key(&center),
        "multi-arm node must reject implicit cross-owner CDT height sharing deterministically"
    );
    assert_eq!(
        surface_a.compiled_visual_node_pieces().get(&center),
        surface_b.compiled_visual_node_pieces().get(&center)
    );
}

#[test]
fn oblique_t_junction_rejects_implicit_cross_owner_cdt_height_edge() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "60-degree T junction must reject implicit cross-owner CDT height sharing"
    );
}

#[test]
fn editor_sized_60_degree_t_junction_width_7_rejects_cdt_height_edge_conflict() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "editor-sized 60-degree T junction must reject implicit cross-owner CDT height sharing"
    );

    let raw_clip_sources = surface
        .compiled_visual_span_pieces()
        .values()
        .flat_map(|piece| piece.terrain_clip_boundary_loops.iter().cloned())
        .chain(
            surface
                .compiled_visual_node_pieces()
                .values()
                .flat_map(|piece| piece.terrain_clip_boundary_loops.iter().cloned()),
        )
        .collect::<Vec<_>>();
    assert!(
        !raw_clip_sources.is_empty(),
        "editor-sized 60-degree T junction must have raw terrain clip source loops"
    );
    let unioned_clip_sources =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops(&raw_clip_sources);
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
fn logged_flat_three_way_oblique_junction_rejects_implicit_height_repair() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "flat oblique 3-way must not emit a JunctionN while its curb/sidewalk seam still depends on implicit same-XZ height repair"
    );
}

#[test]
fn logged_current_flat_three_way_oblique_junction_rejects_cross_owner_cdt_height_edge() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-82.716, 0.0, -14.881), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-25.618, 0.0, -14.881), NodeType::Junction);
    let east = graph.add_node(Vector3::new(57.284, 0.0, -14.881), NodeType::Junction);
    let oblique = graph.add_node(Vector3::new(30.950, 0.0, 41.687), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-82.716, 0.0, -14.881),
            Vector3::new(-25.618, 0.0, -14.881),
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
            Vector3::new(-25.618, 0.0, -14.881),
            Vector3::new(30.950, 0.0, 41.687),
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
            Vector3::new(-25.618, 0.0, -14.881),
            Vector3::new(57.284, 0.0, -14.881),
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "current flat oblique 3-way must reject the remaining implicit cross-owner CDT height edge"
    );
}

#[test]
fn logged_flat_three_way_oblique_variant_rejects_implicit_same_band_curb_height_edge() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-74.754, 0.0, -4.117), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-20.950, 0.0, -6.649), NodeType::Junction);
    let east = graph.add_node(Vector3::new(40.079, 0.0, -9.522), NodeType::Junction);
    let branch = graph.add_node(Vector3::new(25.060, 0.0, 55.624), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-74.754, 0.0, -4.117),
            Vector3::new(-20.950, 0.0, -6.649),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        branch,
        vec![
            Vector3::new(-20.950, 0.0, -6.649),
            Vector3::new(25.060, 0.0, 55.624),
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
            Vector3::new(-20.950, 0.0, -6.649),
            Vector3::new(40.079, 0.0, -9.522),
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "flat oblique 3-way variant must reject until same-band curb transition ownership removes the 0-to-120 mm implicit shared edge"
    );
}

#[test]
fn logged_elevated_three_way_oblique_junction_rejects_contradictory_sidewalk_seam() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-5.708, 139.500, 43.670), NodeType::Junction);
    let center = graph.add_node(Vector3::new(51.778, 146.820, 55.467), NodeType::Junction);
    let branch = graph.add_node(Vector3::new(126.913, 143.009, 5.921), NodeType::Junction);
    let east = graph.add_node(Vector3::new(161.991, 147.143, 78.086), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-5.708, 139.500, 43.670),
            Vector3::new(51.778, 146.820, 55.467),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        branch,
        vec![
            Vector3::new(51.778, 146.820, 55.467),
            Vector3::new(126.913, 143.009, 5.921),
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
            Vector3::new(51.778, 146.820, 55.467),
            Vector3::new(161.991, 147.143, 78.086),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "elevated oblique 3-way must not emit a JunctionN while its sidewalk seam has contradictory same-XZ heights"
    );
}

#[test]
fn logged_current_elevated_oblique_three_way_rejects_contradictory_junction_node() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-6.578, 141.206, -5.989), NodeType::Junction);
    let south = graph.add_node(Vector3::new(-43.834, 158.291, -122.338), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-23.211, 150.463, -57.933), NodeType::Junction);
    let branch = graph.add_node(Vector3::new(8.837, 153.266, -120.160), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-6.578, 141.206, -5.989),
            Vector3::new(-6.816, 141.251, -6.732),
            Vector3::new(-6.996, 141.296, -7.296),
            Vector3::new(-7.224, 141.339, -8.008),
            Vector3::new(-7.499, 141.380, -8.865),
            Vector3::new(-7.653, 141.423, -9.346),
            Vector3::new(-7.817, 141.469, -9.860),
            Vector3::new(-7.993, 141.523, -10.408),
            Vector3::new(-8.179, 141.586, -10.988),
            Vector3::new(-8.375, 141.659, -11.601),
            Vector3::new(-8.581, 141.740, -12.244),
            Vector3::new(-8.797, 141.830, -12.919),
            Vector3::new(-9.022, 141.925, -13.623),
            Vector3::new(-9.257, 142.028, -14.356),
            Vector3::new(-9.501, 142.138, -15.119),
            Vector3::new(-9.754, 142.255, -15.909),
            Vector3::new(-10.016, 142.377, -16.726),
            Vector3::new(-10.286, 142.500, -17.570),
            Vector3::new(-10.565, 142.617, -18.440),
            Vector3::new(-10.851, 142.718, -19.336),
            Vector3::new(-11.146, 142.798, -20.256),
            Vector3::new(-11.372, 142.854, -20.961),
            Vector3::new(-11.525, 142.890, -21.439),
            Vector3::new(-11.680, 142.913, -21.923),
            Vector3::new(-11.837, 142.930, -22.412),
            Vector3::new(-11.995, 142.948, -22.907),
            Vector3::new(-12.155, 142.970, -23.408),
            Vector3::new(-12.317, 142.996, -23.913),
            Vector3::new(-12.481, 143.025, -24.425),
            Vector3::new(-12.646, 143.055, -24.941),
            Vector3::new(-12.814, 143.086, -25.463),
            Vector3::new(-12.982, 143.117, -25.990),
            Vector3::new(-13.153, 143.149, -26.522),
            Vector3::new(-13.325, 143.180, -27.059),
            Vector3::new(-13.498, 143.211, -27.601),
            Vector3::new(-13.673, 143.242, -28.147),
            Vector3::new(-13.850, 143.277, -28.698),
            Vector3::new(-14.028, 143.320, -29.254),
            Vector3::new(-14.207, 143.376, -29.815),
            Vector3::new(-14.388, 143.447, -30.380),
            Vector3::new(-14.570, 143.534, -30.949),
            Vector3::new(-14.754, 143.632, -31.522),
            Vector3::new(-14.939, 143.737, -32.100),
            Vector3::new(-15.125, 143.845, -32.682),
            Vector3::new(-15.313, 143.954, -33.268),
            Vector3::new(-15.502, 144.062, -33.857),
            Vector3::new(-15.692, 144.170, -34.451),
            Vector3::new(-15.883, 144.279, -35.049),
            Vector3::new(-16.075, 144.390, -35.650),
            Vector3::new(-16.269, 144.502, -36.255),
            Vector3::new(-16.464, 144.614, -36.863),
            Vector3::new(-16.660, 144.726, -37.475),
            Vector3::new(-16.857, 144.839, -38.090),
            Vector3::new(-17.055, 144.957, -38.708),
            Vector3::new(-17.254, 145.083, -39.330),
            Vector3::new(-17.454, 145.221, -39.955),
            Vector3::new(-17.655, 145.372, -40.583),
            Vector3::new(-17.857, 145.535, -41.213),
            Vector3::new(-18.060, 145.706, -41.847),
            Vector3::new(-18.264, 145.880, -42.483),
            Vector3::new(-18.468, 146.056, -43.122),
            Vector3::new(-18.674, 146.231, -43.764),
            Vector3::new(-18.880, 146.405, -44.408),
            Vector3::new(-19.087, 146.579, -45.055),
            Vector3::new(-19.295, 146.753, -45.704),
            Vector3::new(-19.504, 146.926, -46.356),
            Vector3::new(-19.713, 147.097, -47.009),
            Vector3::new(-19.923, 147.266, -47.665),
            Vector3::new(-20.133, 147.434, -48.323),
            Vector3::new(-20.345, 147.606, -48.983),
            Vector3::new(-20.557, 147.786, -49.644),
            Vector3::new(-20.769, 147.976, -50.307),
            Vector3::new(-20.982, 148.177, -50.973),
            Vector3::new(-21.195, 148.386, -51.639),
            Vector3::new(-21.409, 148.602, -52.308),
            Vector3::new(-21.624, 148.822, -52.977),
            Vector3::new(-21.839, 149.046, -53.648),
            Vector3::new(-22.054, 149.275, -54.321),
            Vector3::new(-22.270, 149.506, -54.994),
            Vector3::new(-22.486, 149.732, -55.669),
            Vector3::new(-22.702, 149.946, -56.345),
            Vector3::new(-22.919, 150.138, -57.021),
            Vector3::new(-23.136, 150.308, -57.699),
            Vector3::new(-23.211, 150.463, -57.933),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        branch,
        vec![
            Vector3::new(-23.211, 150.463, -57.933),
            Vector3::new(-22.851, 150.678, -58.632),
            Vector3::new(-22.541, 150.881, -59.233),
            Vector3::new(-22.286, 151.062, -59.728),
            Vector3::new(-21.994, 151.223, -60.296),
            Vector3::new(-21.665, 151.370, -60.934),
            Vector3::new(-21.302, 151.508, -61.639),
            Vector3::new(-20.906, 151.642, -62.408),
            Vector3::new(-20.478, 151.768, -63.239),
            Vector3::new(-20.138, 151.883, -63.900),
            Vector3::new(-19.902, 151.983, -64.358),
            Vector3::new(-19.659, 152.069, -64.830),
            Vector3::new(-19.409, 152.146, -65.316),
            Vector3::new(-19.152, 152.220, -65.814),
            Vector3::new(-18.889, 152.297, -66.326),
            Vector3::new(-18.619, 152.380, -66.849),
            Vector3::new(-18.343, 152.470, -67.384),
            Vector3::new(-18.062, 152.565, -67.931),
            Vector3::new(-17.774, 152.660, -68.489),
            Vector3::new(-17.481, 152.749, -69.058),
            Vector3::new(-17.183, 152.825, -69.638),
            Vector3::new(-16.879, 152.883, -70.227),
            Vector3::new(-16.571, 152.924, -70.827),
            Vector3::new(-16.257, 152.950, -71.436),
            Vector3::new(-15.939, 152.969, -72.054),
            Vector3::new(-15.616, 152.986, -72.680),
            Vector3::new(-15.289, 153.006, -73.315),
            Vector3::new(-14.958, 153.030, -73.958),
            Vector3::new(-14.623, 153.058, -74.609),
            Vector3::new(-14.284, 153.089, -75.267),
            Vector3::new(-13.941, 153.122, -75.932),
            Vector3::new(-13.595, 153.159, -76.603),
            Vector3::new(-13.246, 153.198, -77.281),
            Vector3::new(-12.894, 153.239, -77.965),
            Vector3::new(-12.539, 153.280, -78.654),
            Vector3::new(-12.182, 153.318, -79.348),
            Vector3::new(-11.822, 153.351, -80.047),
            Vector3::new(-11.459, 153.377, -80.751),
            Vector3::new(-11.095, 153.396, -81.458),
            Vector3::new(-10.729, 153.405, -82.170),
            Vector3::new(-10.360, 153.405, -82.885),
            Vector3::new(-9.991, 153.396, -83.602),
            Vector3::new(-9.620, 153.376, -84.323),
            Vector3::new(-9.247, 153.348, -85.046),
            Vector3::new(-8.874, 153.314, -85.770),
            Vector3::new(-8.500, 153.275, -86.497),
            Vector3::new(-8.125, 153.235, -87.224),
            Vector3::new(-7.750, 153.195, -87.953),
            Vector3::new(-7.375, 153.158, -88.682),
            Vector3::new(-6.999, 153.124, -89.411),
            Vector3::new(-6.624, 153.095, -90.140),
            Vector3::new(-6.248, 153.071, -90.868),
            Vector3::new(-5.874, 153.053, -91.596),
            Vector3::new(-5.500, 153.038, -92.322),
            Vector3::new(-5.126, 153.025, -93.047),
            Vector3::new(-4.754, 153.012, -93.770),
            Vector3::new(-4.383, 152.999, -94.490),
            Vector3::new(-4.013, 152.986, -95.208),
            Vector3::new(-3.645, 152.972, -95.923),
            Vector3::new(-3.279, 152.958, -96.634),
            Vector3::new(-2.914, 152.943, -97.342),
            Vector3::new(-2.552, 152.930, -98.045),
            Vector3::new(-2.192, 152.919, -98.744),
            Vector3::new(-1.834, 152.913, -99.439),
            Vector3::new(-1.479, 152.915, -100.128),
            Vector3::new(-1.127, 152.926, -100.811),
            Vector3::new(-0.778, 152.944, -101.489),
            Vector3::new(-0.432, 152.968, -102.161),
            Vector3::new(-0.090, 152.994, -102.826),
            Vector3::new(0.249, 153.021, -103.484),
            Vector3::new(0.584, 153.047, -104.134),
            Vector3::new(0.915, 153.072, -104.777),
            Vector3::new(1.242, 153.096, -105.412),
            Vector3::new(1.565, 153.119, -106.039),
            Vector3::new(1.883, 153.142, -106.657),
            Vector3::new(2.197, 153.164, -107.266),
            Vector3::new(2.506, 153.186, -107.865),
            Vector3::new(2.809, 153.208, -108.455),
            Vector3::new(3.108, 153.228, -109.034),
            Vector3::new(3.401, 153.245, -109.603),
            Vector3::new(3.688, 153.258, -110.161),
            Vector3::new(3.970, 153.268, -110.708),
            Vector3::new(4.246, 153.275, -111.244),
            Vector3::new(4.515, 153.279, -111.767),
            Vector3::new(4.778, 153.282, -112.278),
            Vector3::new(5.035, 153.284, -112.777),
            Vector3::new(5.285, 153.287, -113.262),
            Vector3::new(5.528, 153.289, -113.734),
            Vector3::new(5.764, 153.291, -114.193),
            Vector3::new(6.105, 153.292, -114.854),
            Vector3::new(6.532, 153.292, -115.684),
            Vector3::new(6.928, 153.292, -116.453),
            Vector3::new(7.291, 153.291, -117.158),
            Vector3::new(7.620, 153.288, -117.796),
            Vector3::new(7.912, 153.285, -118.364),
            Vector3::new(8.168, 153.279, -118.860),
            Vector3::new(8.477, 153.273, -119.461),
            Vector3::new(8.837, 153.266, -120.160),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        south,
        vec![
            Vector3::new(-23.211, 150.463, -57.933),
            Vector3::new(-23.353, 150.663, -58.377),
            Vector3::new(-23.570, 150.859, -59.056),
            Vector3::new(-23.788, 151.047, -59.736),
            Vector3::new(-24.006, 151.223, -60.416),
            Vector3::new(-24.224, 151.384, -61.097),
            Vector3::new(-24.442, 151.532, -61.778),
            Vector3::new(-24.660, 151.670, -62.459),
            Vector3::new(-24.878, 151.805, -63.141),
            Vector3::new(-25.097, 151.940, -63.823),
            Vector3::new(-25.315, 152.078, -64.504),
            Vector3::new(-25.533, 152.219, -65.186),
            Vector3::new(-25.751, 152.364, -65.868),
            Vector3::new(-25.970, 152.514, -66.549),
            Vector3::new(-26.188, 152.667, -67.230),
            Vector3::new(-26.406, 152.821, -67.911),
            Vector3::new(-26.624, 152.968, -68.591),
            Vector3::new(-26.841, 153.101, -69.271),
            Vector3::new(-27.059, 153.210, -69.950),
            Vector3::new(-27.276, 153.292, -70.628),
            Vector3::new(-27.493, 153.351, -71.306),
            Vector3::new(-27.709, 153.392, -71.982),
            Vector3::new(-27.926, 153.425, -72.658),
            Vector3::new(-28.142, 153.458, -73.333),
            Vector3::new(-28.358, 153.494, -74.006),
            Vector3::new(-28.573, 153.534, -74.679),
            Vector3::new(-28.788, 153.576, -75.350),
            Vector3::new(-29.002, 153.618, -76.020),
            Vector3::new(-29.216, 153.660, -76.688),
            Vector3::new(-29.430, 153.702, -77.355),
            Vector3::new(-29.643, 153.743, -78.020),
            Vector3::new(-29.855, 153.784, -78.683),
            Vector3::new(-30.067, 153.827, -79.345),
            Vector3::new(-30.278, 153.871, -80.004),
            Vector3::new(-30.489, 153.918, -80.662),
            Vector3::new(-30.699, 153.966, -81.318),
            Vector3::new(-30.908, 154.015, -81.971),
            Vector3::new(-31.117, 154.064, -82.623),
            Vector3::new(-31.324, 154.113, -83.272),
            Vector3::new(-31.532, 154.162, -83.919),
            Vector3::new(-31.738, 154.211, -84.563),
            Vector3::new(-31.943, 154.259, -85.205),
            Vector3::new(-32.148, 154.307, -85.844),
            Vector3::new(-32.352, 154.355, -86.480),
            Vector3::new(-32.555, 154.403, -87.114),
            Vector3::new(-32.757, 154.452, -87.745),
            Vector3::new(-32.958, 154.500, -88.372),
            Vector3::new(-33.158, 154.547, -88.997),
            Vector3::new(-33.357, 154.593, -89.619),
            Vector3::new(-33.555, 154.637, -90.237),
            Vector3::new(-33.752, 154.680, -90.852),
            Vector3::new(-33.948, 154.721, -91.464),
            Vector3::new(-34.143, 154.761, -92.073),
            Vector3::new(-34.336, 154.800, -92.677),
            Vector3::new(-34.529, 154.838, -93.279),
            Vector3::new(-34.720, 154.875, -93.876),
            Vector3::new(-34.910, 154.912, -94.470),
            Vector3::new(-35.099, 154.949, -95.059),
            Vector3::new(-35.287, 154.984, -95.645),
            Vector3::new(-35.473, 155.019, -96.227),
            Vector3::new(-35.658, 155.052, -96.805),
            Vector3::new(-35.841, 155.082, -97.378),
            Vector3::new(-36.024, 155.110, -97.948),
            Vector3::new(-36.205, 155.139, -98.512),
            Vector3::new(-36.384, 155.172, -99.073),
            Vector3::new(-36.562, 155.214, -99.629),
            Vector3::new(-36.739, 155.267, -100.180),
            Vector3::new(-36.914, 155.333, -100.726),
            Vector3::new(-37.087, 155.409, -101.268),
            Vector3::new(-37.259, 155.491, -101.805),
            Vector3::new(-37.429, 155.575, -102.337),
            Vector3::new(-37.598, 155.658, -102.864),
            Vector3::new(-37.765, 155.739, -103.386),
            Vector3::new(-37.931, 155.818, -103.902),
            Vector3::new(-38.094, 155.895, -104.414),
            Vector3::new(-38.256, 155.969, -104.920),
            Vector3::new(-38.417, 156.041, -105.420),
            Vector3::new(-38.575, 156.111, -105.915),
            Vector3::new(-38.732, 156.184, -106.404),
            Vector3::new(-38.887, 156.264, -106.888),
            Vector3::new(-39.040, 156.358, -107.366),
            Vector3::new(-39.266, 156.467, -108.072),
            Vector3::new(-39.560, 156.591, -108.991),
            Vector3::new(-39.847, 156.724, -109.887),
            Vector3::new(-40.125, 156.860, -110.757),
            Vector3::new(-40.396, 156.992, -111.601),
            Vector3::new(-40.657, 157.117, -112.418),
            Vector3::new(-40.910, 157.233, -113.208),
            Vector3::new(-41.155, 157.341, -113.971),
            Vector3::new(-41.389, 157.442, -114.704),
            Vector3::new(-41.615, 157.537, -115.408),
            Vector3::new(-41.831, 157.627, -116.083),
            Vector3::new(-42.037, 157.710, -116.726),
            Vector3::new(-42.233, 157.789, -117.339),
            Vector3::new(-42.419, 157.863, -117.919),
            Vector3::new(-42.594, 157.934, -118.467),
            Vector3::new(-42.759, 158.002, -118.982),
            Vector3::new(-42.913, 158.067, -119.462),
            Vector3::new(-43.187, 158.129, -120.319),
            Vector3::new(-43.415, 158.187, -121.032),
            Vector3::new(-43.596, 158.240, -121.595),
            Vector3::new(-43.834, 158.291, -122.338),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();
    assert!((graph.edge(0).end_clip - 12.071).abs() <= 0.01);
    assert!((graph.edge(1).start_clip - 12.071).abs() <= 0.01);
    assert!((graph.edge(2).start_clip - 12.071).abs() <= 0.01);

    let mut main_geometry = graph.edge(0).geometry.clone();
    main_geometry.extend(graph.edge(2).geometry.iter().skip(1).copied());
    let mut stale_graph = RegionGraph::new();
    let stale_west = stale_graph.add_node(graph.node(west).pos, NodeType::Junction);
    let stale_south = stale_graph.add_node(graph.node(south).pos, NodeType::Junction);
    stale_graph.add_edge(test_edge(
        stale_west,
        stale_south,
        main_geometry,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    stale_graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&stale_graph, &terrain);
    for edge_idx in 0..graph.edge_count() {
        surface.mark_edge_dirty(&graph, edge_idx);
    }
    for node_id in [west, south, center, branch] {
        surface.mark_node_dirty(&graph, node_id);
    }
    surface.compile_dirty(&graph, &terrain);

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "current elevated oblique 3-way must not emit a JunctionN while its sidewalk seam has contradictory same-XZ heights"
    );
}

#[test]
fn logged_latest_elevated_oblique_three_way_rejects_contradictory_junction_node() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let edge0_points = road_points_from_json(
        r#"[[-29.527,139.925,4.210],[-29.585,139.927,3.491],[-29.629,139.928,2.946],[-29.685,139.930,2.256],[-29.752,139.931,1.428],[-29.809,139.933,0.718],[-29.851,139.936,0.204],[-29.895,139.940,-0.342],[-29.941,139.944,-0.919],[-29.991,139.950,-1.526],[-30.042,139.956,-2.164],[-30.096,139.962,-2.831],[-30.152,139.969,-3.526],[-30.211,139.975,-4.250],[-30.271,139.981,-5.000],[-30.334,139.984,-5.778],[-30.399,139.986,-6.582],[-30.466,139.989,-7.411],[-30.535,139.996,-8.265],[-30.606,140.014,-9.143],[-30.679,140.046,-10.044],[-30.754,140.091,-10.969],[-30.830,140.146,-11.915],[-30.909,140.204,-12.884],[-30.969,140.258,-13.623],[-31.009,140.304,-14.123],[-31.050,140.342,-14.628],[-31.091,140.375,-15.138],[-31.132,140.406,-15.652],[-31.174,140.438,-16.171],[-31.217,140.470,-16.696],[-31.260,140.502,-17.224],[-31.303,140.533,-17.757],[-31.346,140.564,-18.295],[-31.390,140.598,-18.837],[-31.434,140.636,-19.384],[-31.479,140.684,-19.934],[-31.523,140.741,-20.489],[-31.569,140.808,-21.048],[-31.614,140.881,-21.611],[-31.660,140.958,-22.177],[-31.706,141.036,-22.748],[-31.753,141.113,-23.322],[-31.799,141.190,-23.900],[-31.846,141.266,-24.482],[-31.894,141.342,-25.067],[-31.941,141.419,-25.655],[-31.989,141.496,-26.247],[-32.037,141.571,-26.842],[-32.085,141.645,-27.440],[-32.134,141.719,-28.041],[-32.183,141.796,-28.646],[-32.232,141.881,-29.253],[-32.281,141.977,-29.863],[-32.331,142.088,-30.476],[-32.381,142.212,-31.092],[-32.431,142.347,-31.710],[-32.481,142.486,-32.331],[-32.531,142.628,-32.954],[-32.582,142.768,-33.579],[-32.632,142.908,-34.207],[-32.683,143.047,-34.837],[-32.734,143.187,-35.470],[-32.786,143.326,-36.104],[-32.837,143.464,-36.740],[-32.889,143.601,-37.378],[-32.940,143.739,-38.018],[-32.992,143.880,-38.660],[-33.044,144.030,-39.303],[-33.096,144.192,-39.948],[-33.149,144.369,-40.595],[-33.201,144.558,-41.243],[-33.254,144.756,-41.892],[-33.306,144.959,-42.542],[-33.359,145.162,-43.194],[-33.412,145.365,-43.846],[-33.464,145.566,-44.500],[-33.517,145.766,-45.155],[-33.570,145.965,-45.810],[-33.623,146.164,-46.466],[-33.676,146.362,-47.123],[-33.730,146.562,-47.780],[-33.783,146.765,-48.438],[-33.836,146.974,-49.097],[-33.889,147.190,-49.756],[-33.943,147.407,-50.415],[-33.996,147.618,-51.074],[-34.049,147.816,-51.733],[-34.102,147.998,-52.393],[-34.129,148.170,-52.715]]"#,
    );
    let edge1_points = road_points_from_json(
        r#"[[-34.129,148.170,-52.715],[-33.619,148.388,-53.314],[-33.181,148.608,-53.829],[-32.820,148.832,-54.254],[-32.406,149.068,-54.741],[-31.941,149.318,-55.287],[-31.428,149.580,-55.891],[-30.867,149.841,-56.550],[-30.262,150.085,-57.262],[-29.944,150.299,-57.636],[-29.615,150.481,-58.023],[-29.275,150.637,-58.422],[-28.926,150.779,-58.832],[-28.568,150.914,-59.254],[-28.200,151.045,-59.687],[-27.823,151.169,-60.130],[-27.437,151.284,-60.584],[-27.043,151.389,-61.047],[-26.640,151.486,-61.521],[-26.229,151.580,-62.004],[-25.811,151.675,-62.496],[-25.385,151.772,-62.997],[-24.952,151.872,-63.507],[-24.511,151.973,-64.024],[-24.064,152.074,-64.550],[-23.611,152.175,-65.083],[-23.151,152.275,-65.624],[-22.685,152.377,-66.172],[-22.214,152.481,-66.726],[-21.737,152.587,-67.287],[-21.255,152.694,-67.854],[-20.768,152.797,-68.427],[-20.276,152.889,-69.005],[-19.780,152.963,-69.588],[-19.280,153.014,-70.176],[-18.776,153.042,-70.769],[-18.268,153.053,-71.366],[-17.757,153.056,-71.968],[-17.242,153.057,-72.572],[-16.725,153.062,-73.180],[-16.206,153.072,-73.792],[-15.684,153.088,-74.405],[-15.160,153.106,-75.022],[-14.634,153.126,-75.640],[-14.106,153.150,-76.261],[-13.577,153.176,-76.882],[-13.047,153.208,-77.505],[-12.517,153.244,-78.129],[-11.986,153.281,-78.754],[-11.454,153.315,-79.379],[-10.923,153.337,-80.004],[-10.392,153.342,-80.628],[-9.861,153.327,-81.252],[-9.331,153.291,-81.875],[-8.803,153.241,-82.497],[-8.275,153.182,-83.118],[-7.749,153.121,-83.736],[-7.225,153.060,-84.352],[-6.703,153.001,-84.966],[-6.183,152.943,-85.577],[-5.666,152.884,-86.186],[-5.152,152.824,-86.790],[-4.641,152.763,-87.391],[-4.133,152.700,-87.988],[-3.629,152.637,-88.581],[-3.129,152.577,-89.170],[-2.633,152.521,-89.753],[-2.141,152.471,-90.331],[-1.654,152.427,-90.904],[-1.172,152.388,-91.471],[-0.695,152.353,-92.032],[-0.224,152.321,-92.586],[0.242,152.291,-93.134],[0.702,152.262,-93.674],[1.155,152.234,-94.208],[1.603,152.207,-94.733],[2.043,152.182,-95.251],[2.476,152.157,-95.761],[2.902,152.134,-96.262],[3.321,152.111,-96.754],[3.731,152.089,-97.237],[4.134,152.067,-97.710],[4.528,152.046,-98.174],[4.914,152.025,-98.628],[5.291,152.007,-99.071],[5.659,151.991,-99.504],[6.018,151.979,-99.925],[6.367,151.970,-100.336],[6.706,151.965,-100.734],[7.035,151.962,-101.121],[7.661,151.960,-101.858],[8.244,151.957,-102.544],[8.782,151.954,-103.175],[9.271,151.950,-103.751],[9.711,151.944,-104.268],[10.099,151.936,-104.724],[10.433,151.926,-105.117],[11.220,151.915,-106.042]]"#,
    );
    let edge2_points = road_points_from_json(
        r#"[[-34.129,148.170,-52.715],[-34.156,148.341,-53.052],[-34.209,148.523,-53.712],[-34.262,148.722,-54.371],[-34.316,148.937,-55.029],[-34.369,149.163,-55.688],[-34.422,149.394,-56.346],[-34.475,149.626,-57.003],[-34.528,149.856,-57.660],[-34.581,150.080,-58.316],[-34.634,150.296,-58.972],[-34.687,150.498,-59.626],[-34.740,150.683,-60.280],[-34.793,150.851,-60.933],[-34.845,151.004,-61.584],[-34.898,151.149,-62.235],[-34.950,151.291,-62.884],[-35.003,151.434,-63.532],[-35.055,151.579,-64.178],[-35.107,151.726,-64.823],[-35.159,151.873,-65.466],[-35.211,152.022,-66.108],[-35.263,152.172,-66.748],[-35.314,152.325,-67.386],[-35.366,152.477,-68.022],[-35.417,152.625,-68.657],[-35.468,152.761,-69.289],[-35.519,152.880,-69.919],[-35.570,152.978,-70.547],[-35.621,153.059,-71.172],[-35.671,153.126,-71.796],[-35.721,153.188,-72.416],[-35.771,153.249,-73.035],[-35.821,153.313,-73.650],[-35.870,153.380,-74.263],[-35.920,153.448,-74.873],[-35.969,153.518,-75.481],[-36.018,153.587,-76.085],[-36.066,153.658,-76.686],[-36.115,153.729,-77.285],[-36.163,153.801,-77.880],[-36.211,153.873,-78.471],[-36.258,153.941,-79.060],[-36.305,154.005,-79.645],[-36.352,154.061,-80.226],[-36.399,154.109,-80.804],[-36.446,154.151,-81.379],[-36.492,154.189,-81.949],[-36.537,154.226,-82.516],[-36.583,154.263,-83.079],[-36.628,154.300,-83.637],[-36.673,154.338,-84.192],[-36.717,154.376,-84.743],[-36.762,154.414,-85.289],[-36.805,154.451,-85.831],[-36.849,154.489,-86.369],[-36.892,154.526,-86.902],[-36.935,154.562,-87.431],[-36.977,154.598,-87.955],[-37.019,154.633,-88.474],[-37.061,154.667,-88.989],[-37.102,154.700,-89.498],[-37.143,154.733,-90.003],[-37.183,154.769,-90.503],[-37.243,154.809,-91.243],[-37.321,154.852,-92.211],[-37.398,154.899,-93.158],[-37.472,154.947,-94.082],[-37.545,154.994,-94.984],[-37.616,155.036,-95.862],[-37.685,155.074,-96.716],[-37.752,155.110,-97.545],[-37.817,155.149,-98.348],[-37.880,155.196,-99.126],[-37.941,155.257,-99.877],[-37.999,155.334,-100.600],[-38.056,155.423,-101.296],[-38.109,155.519,-101.963],[-38.161,155.616,-102.600],[-38.210,155.708,-103.208],[-38.257,155.798,-103.785],[-38.301,155.886,-104.331],[-38.342,155.978,-104.844],[-38.400,156.074,-105.554],[-38.467,156.175,-106.383],[-38.522,156.277,-107.072],[-38.567,156.379,-107.617],[-38.625,156.481,-108.336]]"#,
    );

    let mut graph = RegionGraph::new();
    let west = graph.add_node(edge0_points[0], NodeType::Junction);
    let south = graph.add_node(edge2_points.last().copied().unwrap(), NodeType::Junction);
    let center = graph.add_node(edge0_points.last().copied().unwrap(), NodeType::Junction);
    let branch = graph.add_node(edge1_points.last().copied().unwrap(), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        center,
        edge0_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        branch,
        edge1_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        south,
        edge2_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();
    assert!(
        graph.edge(0).end_clip > 0.0,
        "latest elevated edge 0 must be clipped into the junction; clip={:.3}",
        graph.edge(0).end_clip
    );
    assert!(
        graph.edge(1).start_clip > 0.0,
        "latest elevated edge 1 must be clipped into the junction; clip={:.3}",
        graph.edge(1).start_clip
    );
    assert!(
        graph.edge(2).start_clip > 0.0,
        "latest elevated edge 2 must be clipped into the junction; clip={:.3}",
        graph.edge(2).start_clip
    );

    let mut edit_path_main_geometry = edge0_points.clone();
    edit_path_main_geometry.extend(edge2_points.iter().skip(1).copied());

    let mut stale_main_geometry = edge0_points;
    stale_main_geometry.extend(edge2_points.iter().skip(1).copied());
    let mut stale_graph = RegionGraph::new();
    let stale_west = stale_graph.add_node(graph.node(west).pos, NodeType::Junction);
    let stale_south = stale_graph.add_node(graph.node(south).pos, NodeType::Junction);
    stale_graph.add_edge(test_edge(
        stale_west,
        stale_south,
        stale_main_geometry,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    stale_graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&stale_graph, &terrain);
    for edge_idx in 0..graph.edge_count() {
        surface.mark_edge_dirty(&graph, edge_idx);
    }
    for node_id in [west, south, center, branch] {
        surface.mark_node_dirty(&graph, node_id);
    }
    surface.compile_dirty(&graph, &terrain);

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "latest elevated oblique 3-way must not emit a JunctionN with contradictory same-XZ sidewalk seam heights"
    );

    let mut edit_graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let config = crate::simulation::core::config::WorldConfig::default();
    let mut zoning = crate::simulation::grid::zoning::ZoningSystem::new(&config);
    let mut allocator = crate::simulation::buildings::allocator::BuildingAllocator::new();
    network.add_road(
        &mut edit_graph,
        edit_path_main_geometry,
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.road_surface.compile_dirty(&edit_graph, &terrain);
    network.add_road(
        &mut edit_graph,
        edge1_points,
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.road_surface.compile_dirty(&edit_graph, &terrain);

    let edit_center = (0..edit_graph.node_count() as u32)
        .find(|&node_id| {
            edit_graph
                .node_adjacency(node_id)
                .iter()
                .filter(|&&edge_idx| !edit_graph.edge(edge_idx).deleted)
                .count()
                == 3
        })
        .expect("add_road edit path must create a 3-way junction node");
    assert!(
        !network
            .road_surface
            .compiled_visual_node_pieces()
            .contains_key(&edit_center),
        "add_road edit path must not emit the elevated oblique JunctionN while its sidewalk seam has contradictory same-XZ heights"
    );
}

#[test]
fn logged_flat_oblique_t_junction_rejects_missing_explicit_curb_sidewalk_endpoint_authority() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "logged flat oblique T must reject same-key carriageway/sidewalk height contact until rails emit the explicit curb/sidewalk endpoint path"
    );
}

#[test]
fn logged_flat_oblique_four_way_rejects_cross_owner_cdt_height_edge() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "logged flat oblique four-way must reject remaining implicit cross-owner CDT height sharing"
    );
}

#[test]
fn arbitrary_six_way_junction_rejects_implicit_cross_owner_cdt_height_edge() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "arbitrary six-way node must reject implicit cross-owner CDT height sharing"
    );
}

#[test]
fn arbitrary_five_way_junction_rejects_implicit_cross_owner_cdt_height_edge() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "arbitrary five-way node must reject implicit cross-owner CDT height sharing"
    );
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
fn dirty_recompile_rejects_expanded_arbitrary_node_piece_with_cdt_height_conflict() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "expanded arbitrary junction must reject implicit cross-owner CDT height sharing"
    );
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "JunctionN must reject until explicit legal curb/sidewalk join ownership is generated before heighting"
    );
}

#[test]
fn elevated_four_way_junction_rejects_implicit_cross_owner_cdt_height_edge() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "elevated 4-way node must reject implicit cross-owner CDT height sharing"
    );
}

#[test]
fn elevated_junction_rejects_contradictory_side_vertex_heights() {
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
    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "steep JunctionN must not emit same-XZ side vertices at contradictory same-band heights"
    );
}

#[test]
fn elevated_three_way_junction_rejects_implicit_cross_owner_cdt_height_edge() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "elevated 3-way node must reject implicit cross-owner CDT height sharing"
    );
}

#[test]
fn skewed_elevated_four_way_junction_rejects_implicit_cross_owner_cdt_height_edge() {
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

    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&center),
        "skewed elevated 4-way node must reject implicit cross-owner CDT height sharing"
    );
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
    assert!(dump.contains("\"curb_vertical_face_details\""));
    assert!(dump.contains("\"expected_asphalt_curb_steps\""));
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
