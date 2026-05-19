//! Unit tests for the road-surface compiler and ownership caches.

use super::arrangement::{
    NodeArrangement, NodeArrangementError, NodeArrangementKey, NodeBandOwner,
};
use super::band_semantics::ordered_raised_step_kinds;
use super::earthwork::EARTHWORK_MAX_MARGIN_M;
use super::edge::CURB_STEP_HEIGHT_M;
use super::height::NodeHeightFieldError;
use super::validation::{NodeGeometryDiagnosticKind, NodeValidationReport};
use super::{
    NodeFootprintBoundaryVertexSource, PreviewRoadSurfaceResult, RoadSurfaceBand,
    RoadSurfaceBandKind, RoadSurfaceEarthworkFaceKind, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkSupportPolicy, RoadSurfaceSection, RoadSurfaceSpanRegionRole,
    RoadSurfaceSystem, RoadSurfaceTerrainClipEdgeKind, RoadSurfaceTerrainClipExportError,
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
    TerrainCdtInput, TerrainCdtMesh, TerrainCdtPatch, TerrainCdtRoadBoundarySource,
    TerrainCdtRoadLoop, TerrainCdtTieInKind, TerrainCdtVertex, build_road_touched_terrain_patch,
};
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet, HashSet};

mod terrain_clip;

#[test]
fn span_raised_step_generation_uses_resolved_regions() {
    let span_source = include_str!("span.rs");
    for forbidden in [
        "curb_vertical_face_polygon_for_section_pair",
        "curb_vertical_face",
        "curb_asphalt_boundary",
        "compile_surface_polygons_for_ranges",
        "compile_span_explicit_vertical_step_faces_for_ranges",
        "SpanExplicitVerticalStepBoundary",
    ] {
        assert!(
            !span_source.contains(forbidden),
            "span output must consume resolved regions and generic raised-step constraints, not legacy section-window helper `{forbidden}`"
        );
    }
    assert!(
        span_source.contains("resolve_span_regions_for_ranges")
            && span_source.contains("span_raised_step_faces_from_constraints"),
        "span output must route through resolved regions and raised-step constraints"
    );
}

#[test]
fn span_vertical_steps_include_carriageway_sidewalk_boundaries_when_profile_has_no_curb() {
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        5.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR,
    ));
    let section_at = |s_m: f32| RoadSurfaceSection {
        edge_idx,
        s_m,
        center_xz: Vector2::new(s_m, 0.0),
        center_height_m: 0.0,
        tangent_xz: Vector2::new(1.0, 0.0),
        lateral_xz: Vector2::new(0.0, 1.0),
        bands: vec![
            RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Carriageway,
                lateral_start_m: -3.0,
                lateral_end_m: 0.0,
                height_start_m: 0.0,
                height_end_m: 0.0,
            },
            RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: 0.0,
                lateral_end_m: 2.0,
                height_start_m: CURB_STEP_HEIGHT_M,
                height_end_m: CURB_STEP_HEIGHT_M,
            },
        ],
    };
    let sections = vec![
        section_at(0.0),
        section_at(4.0),
        section_at(6.0),
        section_at(10.0),
    ];
    let mut surface = RoadSurfaceSystem::new(64.0);
    surface.compiled_sections.insert(edge_idx, sections);

    let span_piece = surface
        .compile_visual_span_piece(&graph, &flat_terrain(32, 32), edge_idx)
        .expect("direct carriageway-sidewalk span should compile");
    assert!(!span_piece.raised_step_face_polygons.is_empty());
    assert!(
        span_piece.raised_step_face_polygons.iter().any(|face| {
            face.points_world
                .iter()
                .any(|point| (point.y - CURB_STEP_HEIGHT_M).abs() <= SAMPLE_EPSILON_M)
                && face
                    .points_world
                    .iter()
                    .any(|point| point.y.abs() <= SAMPLE_EPSILON_M)
        }),
        "direct carriageway-sidewalk span boundary must emit a raised vertical face"
    );
}

#[test]
fn span_vertical_steps_include_generic_non_road_owner_pairs() {
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        5.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR,
    ));
    let sidewalk_height_m = CURB_STEP_HEIGHT_M * 2.0;
    let section_at = |s_m: f32| RoadSurfaceSection {
        edge_idx,
        s_m,
        center_xz: Vector2::new(s_m, 0.0),
        center_height_m: 0.0,
        tangent_xz: Vector2::new(1.0, 0.0),
        lateral_xz: Vector2::new(0.0, 1.0),
        bands: vec![
            RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Carriageway,
                lateral_start_m: -3.0,
                lateral_end_m: 0.0,
                height_start_m: 0.0,
                height_end_m: 0.0,
            },
            RoadSurfaceBand {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
                lateral_start_m: 0.0,
                lateral_end_m: 0.5,
                height_start_m: CURB_STEP_HEIGHT_M,
                height_end_m: CURB_STEP_HEIGHT_M,
            },
            RoadSurfaceBand {
                kind: RoadSurfaceBandKind::Sidewalk,
                lateral_start_m: 0.5,
                lateral_end_m: 2.0,
                height_start_m: sidewalk_height_m,
                height_end_m: sidewalk_height_m,
            },
        ],
    };
    let mut surface = RoadSurfaceSystem::new(64.0);
    surface.compiled_sections.insert(
        edge_idx,
        vec![
            section_at(0.0),
            section_at(4.0),
            section_at(6.0),
            section_at(10.0),
        ],
    );

    let span_piece = surface
        .compile_visual_span_piece(&graph, &flat_terrain(32, 32), edge_idx)
        .expect("curb-sidewalk stepped span should compile");
    assert!(
        span_piece.raised_step_face_polygons.iter().any(|face| {
            face.points_world
                .iter()
                .any(|point| (point.y - sidewalk_height_m).abs() <= SAMPLE_EPSILON_M)
                && face
                    .points_world
                    .iter()
                    .any(|point| (point.y - CURB_STEP_HEIGHT_M).abs() <= SAMPLE_EPSILON_M)
        }),
        "span raised-step output must be owner-pair generic, including curb / sidewalk"
    );
}

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
        source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id: 0,
            kind: RoadSurfaceVisualNodePieceKind::Terminal,
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 0,
            boundary_source: None,
        },
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

fn terrain_cdt_input_for_bounds(
    terrain: &TerrainSystem,
    road_loops: Vec<TerrainCdtRoadLoop>,
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
    sample_step_m: f32,
) -> TerrainCdtInput {
    let patch = TerrainCdtPatch::new(
        f64::from(min_x),
        f64::from(min_z),
        f64::from(max_x),
        f64::from(max_z),
        [
            terrain_height_m(terrain, min_x, min_z),
            terrain_height_m(terrain, min_x, max_z),
            terrain_height_m(terrain, max_x, max_z),
            terrain_height_m(terrain, max_x, min_z),
        ],
    );
    let mut source_samples = Vec::new();
    let step = sample_step_m.max(1.0);
    let mut z = min_z;
    while z <= max_z + SAMPLE_EPSILON_M {
        let mut x = min_x;
        while x <= max_x + SAMPLE_EPSILON_M {
            source_samples.push(TerrainCdtVertex::new(
                f64::from(x),
                terrain_height_m(terrain, x, z),
                f64::from(z),
            ));
            x += step;
        }
        z += step;
    }
    TerrainCdtInput::new(patch, road_loops, source_samples)
}

fn terrain_height_m(terrain: &TerrainSystem, x: f32, z: f32) -> f32 {
    terrain.sample_visual_height_world(x, z) * crate::config::HEIGHT_SCALE
}

fn assert_surface_terrain_cdt_contract(
    case_name: &str,
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    terrain: &TerrainSystem,
    bounds: (f32, f32, f32, f32),
    expect_retaining_wall: bool,
) {
    let (min_x, min_z, max_x, max_z) = bounds;
    let (road_loops, source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(graph, min_x, min_z, max_x, max_z)
        .unwrap_or_else(|err| panic!("{case_name}: terrain clip export failed: {err:?}"));
    assert!(
        !road_loops.is_empty(),
        "{case_name}: expected production terrain CDT road loops"
    );
    assert!(
        source_count
            >= road_loops
                .iter()
                .filter(|road_loop| !road_loop.is_hole)
                .count(),
        "{case_name}: source loop count should name the raw owned footprint contributors"
    );
    for edge_source in road_loops
        .iter()
        .flat_map(|road_loop| road_loop.source_edges.iter())
    {
        assert_surface_cdt_boundary_source(case_name, edge_source.source);
    }

    let mesh = build_road_touched_terrain_patch(terrain_cdt_input_for_bounds(
        terrain,
        road_loops.clone(),
        min_x,
        min_z,
        max_x,
        max_z,
        8.0,
    ))
    .unwrap_or_else(|err| {
        panic!("{case_name}: production terrain CDT input should build: {err:?}")
    });

    assert_eq!(
        mesh.stats.invalid_constraint_edges, 0,
        "{case_name}: production CDT input must not contain invalid road constraints"
    );
    assert_eq!(
        mesh.stats.preserved_road_constraint_edges, mesh.stats.road_constraint_edges,
        "{case_name}: accepted terrain faces must preserve every road seam constraint"
    );
    assert_eq!(
        mesh.stats.accepted_faces,
        mesh.triangles.len() + mesh.retaining_wall_triangles.len(),
        "{case_name}: accepted faces must project into terrain or retaining-wall buckets"
    );
    assert_eq!(
        mesh.emitted_faces.len(),
        mesh.stats.accepted_faces,
        "{case_name}: first-class emitted face provenance must cover accepted faces"
    );
    assert_eq!(
        mesh.terrain_triangle_sources.len(),
        mesh.triangles.len(),
        "{case_name}: terrain face source sidecars must match terrain triangles"
    );
    assert_eq!(
        mesh.retaining_wall_triangle_sources.len(),
        mesh.retaining_wall_triangles.len(),
        "{case_name}: retaining-wall face source sidecars must match wall triangles"
    );
    assert!(
        mesh.stats.road_seam_faces > 0,
        "{case_name}: road-touched CDT should expose road-seam diagnostics"
    );
    assert!(
        mesh.road_seam_face_samples
            .iter()
            .all(|sample| !sample.sources.is_empty()),
        "{case_name}: road-seam diagnostics must name source owners"
    );
    assert!(
        mesh.retaining_wall_face_samples
            .iter()
            .all(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall
                && !sample.sources.is_empty()),
        "{case_name}: retaining-wall diagnostics must name source owners"
    );
    assert!(
        mesh.retaining_wall_triangle_sources
            .iter()
            .all(|sources| !sources.is_empty()),
        "{case_name}: retaining-wall emitted faces must not be anonymous"
    );
    assert_eq!(
        mesh.stats.blocking_degenerate_seam_edges, 0,
        "{case_name}: production CDT input must not pass unresolved sub-budget seam fragments to Spade"
    );
    assert_eq!(
        mesh.stats.omitted_near_seam_source_samples, mesh.stats.tie_in_widened_source_samples,
        "{case_name}: omitted near-seam terrain samples must stay visible as tie-in diagnostics"
    );
    assert!(
        mesh.emitted_faces.iter().all(|face| {
            face.kind != TerrainCdtTieInKind::RetainingWall || !face.sources.is_empty()
        }),
        "{case_name}: first-class retaining-wall emitted faces must carry source provenance"
    );
    if expect_retaining_wall {
        assert!(
            mesh.stats.retaining_wall_faces > 0,
            "{case_name}: elevated or extreme authored terrain should expose wall tie-ins"
        );
    }
    assert_cdt_mesh_stays_outside_road_loops(case_name, &mesh, &road_loops);
    assert_cdt_mesh_sources_are_structured(case_name, &mesh);
}

fn assert_surface_cdt_boundary_source(case_name: &str, source: TerrainCdtRoadBoundarySource) {
    assert!(
        !source.debug_label().is_empty(),
        "{case_name}: source label should be available for human debug"
    );
    match source {
        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
            start_section_index,
            end_section_index,
            start_s_m,
            end_s_m,
            ..
        } => {
            assert_eq!(source.source_kind_code(), 0);
            assert!(source.primary_id_code() >= 0);
            assert!(source.edge_class_code() >= 0);
            assert!(source.owner_kind_code() >= 0);
            assert!(source.owner_index_code() >= 0);
            assert!(source.support_policy_code() >= 0);
            assert!(source.role_code() >= 0);
            assert!(end_section_index >= start_section_index);
            assert!(end_s_m >= start_s_m);
            assert_eq!(
                source.section_range_codes(),
                [
                    i32::try_from(start_section_index).unwrap(),
                    i32::try_from(end_section_index).unwrap()
                ]
            );
            assert_eq!(source.s_range_values(), [start_s_m, end_s_m]);
        }
        TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
            owner_index,
            boundary_source,
            ..
        } => {
            assert_eq!(source.source_kind_code(), 1);
            assert!(source.primary_id_code() >= 0);
            assert!(source.node_kind_code() >= 0);
            assert!(source.owner_kind_code() >= 0);
            assert!(
                boundary_source.is_some(),
                "{case_name}: production node CDT source must preserve endpoint boundary provenance"
            );
            assert_eq!(
                source.owner_index_code(),
                i32::try_from(owner_index).unwrap()
            );
            assert_eq!(source.edge_class_code(), -1);
            assert_eq!(source.support_policy_code(), -1);
            assert_eq!(source.role_code(), -1);
            assert_eq!(source.section_range_codes(), [-1, -1]);
            assert_eq!(source.s_range_values(), [-1.0, -1.0]);
        }
        TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. } => {
            panic!("{case_name}: production terrain CDT export must not use synthetic sources")
        }
    }
}

fn assert_cdt_mesh_sources_are_structured(case_name: &str, mesh: &TerrainCdtMesh) {
    for source in mesh
        .emitted_faces
        .iter()
        .flat_map(|face| face.sources.iter().copied())
        .chain(
            mesh.road_seam_face_samples
                .iter()
                .flat_map(|sample| sample.sources.iter().copied()),
        )
        .chain(
            mesh.retaining_wall_face_samples
                .iter()
                .flat_map(|sample| sample.sources.iter().copied()),
        )
        .chain(
            mesh.tie_in_widened_samples
                .iter()
                .map(|sample| sample.seam_source),
        )
    {
        assert_surface_cdt_boundary_source(case_name, source);
    }
}

fn assert_cdt_mesh_stays_outside_road_loops(
    case_name: &str,
    mesh: &TerrainCdtMesh,
    road_loops: &[TerrainCdtRoadLoop],
) {
    for (triangle_index, triangle) in mesh
        .triangles
        .iter()
        .chain(mesh.retaining_wall_triangles.iter())
        .enumerate()
    {
        let center = {
            let a = mesh.vertices[triangle[0]];
            let b = mesh.vertices[triangle[1]];
            let c = mesh.vertices[triangle[2]];
            Vector2::new(
                ((a.x + b.x + c.x) / 3.0) as f32,
                ((a.z + b.z + c.z) / 3.0) as f32,
            )
        };
        if let Some((loop_index, road_loop)) = road_loops
            .iter()
            .enumerate()
            .filter(|(_, road_loop)| !road_loop.is_hole)
            .find(|(_, road_loop)| {
                road_loop_contains_road_owned_point_xz(road_loops, road_loop, center)
            })
        {
            panic!(
                "{case_name}: accepted terrain triangle centroid leaked inside road-owned footprint; triangle_index={triangle_index} center=({:.3},{:.3}) loop_index={loop_index} footprint_group_id={}",
                center.x, center.y, road_loop.footprint_group_id
            );
        }
    }
}

fn road_loop_contains_road_owned_point_xz(
    road_loops: &[TerrainCdtRoadLoop],
    outer_loop: &TerrainCdtRoadLoop,
    point: Vector2,
) -> bool {
    if !terrain_cdt_loop_strictly_contains_point_xz(outer_loop, point) {
        return false;
    }
    !road_loops.iter().any(|candidate| {
        candidate.is_hole
            && candidate.footprint_group_id == outer_loop.footprint_group_id
            && terrain_cdt_loop_strictly_contains_point_xz(candidate, point)
    })
}

fn terrain_cdt_loop_strictly_contains_point_xz(
    road_loop: &TerrainCdtRoadLoop,
    point: Vector2,
) -> bool {
    if road_loop.vertices.len() < 3 {
        return false;
    }
    let mut inside = false;
    for index in 0..road_loop.vertices.len() {
        let start = road_loop.vertices[index];
        let end = road_loop.vertices[(index + 1) % road_loop.vertices.len()];
        if (start.z as f32 > point.y) != (end.z as f32 > point.y) {
            let edge_x_at_point_z = ((end.x - start.x) as f32) * (point.y - start.z as f32)
                / ((end.z - start.z) as f32)
                + start.x as f32;
            if point.x < edge_x_at_point_z {
                inside = !inside;
            }
        }
    }
    inside
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

fn visual_polygon_boundary_contains_xz(
    polygons: &[RoadSurfaceVisualPolygon],
    point: Vector2,
) -> bool {
    polygons
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
        .any(|candidate| {
            Vector2::new(candidate.x - point.x, candidate.z - point.y).length()
                <= SAMPLE_EPSILON_M * 2.0
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

fn assert_earthwork_faces_stay_outside_top_footprint(piece: &RoadSurfaceVisualNodePiece) {
    let top_contours = overlay_contours_from_top_polygons(
        piece
            .road_surface_polygons
            .iter()
            .chain(piece.curb_surface_polygons.iter())
            .chain(piece.sidewalk_surface_polygons.iter()),
    );
    let top_shapes = RoadSurfaceSystem::overlay_union_contours(&top_contours)
        .expect("node top overlay union should succeed");
    for face in &piece.render_earthwork_faces {
        let face_contour = overlay_contour_from_world_points(&face.polygon.points_world);
        if face_contour.len() < 3 {
            continue;
        }
        let face_shapes = RoadSurfaceSystem::overlay_union_contours(&[face_contour])
            .expect("earthwork face overlay union should succeed");
        let overlap = RoadSurfaceSystem::overlay_binary_shapes(
            &face_shapes,
            &top_shapes,
            OverlayRule::Intersect,
        )
        .expect("earthwork/top overlap check should succeed");
        let overlap_area_m2 = overlay_area_m2(&overlap);
        let budget_m2 = RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&face_shapes)
            .max(RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(
                &top_shapes,
            ));
        assert!(
            overlap_area_m2 <= budget_m2,
            "earthwork face must not intrude into road-owned top footprint; kind={:?} inner={:?}->{:?} face={:?} overlap_area={overlap_area_m2:.6} budget={budget_m2:.6}",
            piece.kind,
            face.inner_start,
            face.inner_end,
            face.polygon.points_world
        );
    }
}

fn assert_node_earthwork_faces_have_footprint_provenance(piece: &RoadSurfaceVisualNodePiece) {
    assert!(
        !piece.render_earthwork_faces.is_empty(),
        "node earthwork faces should be generated from owned footprint boundaries"
    );
    for face in &piece.render_earthwork_faces {
        let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id,
            kind,
            owner_kind,
            owner_index,
            boundary_source,
        } = face.source
        else {
            panic!(
                "node earthwork face must carry node footprint provenance, got {:?}",
                face.source
            );
        };
        assert_eq!(node_id, piece.node_id);
        assert_eq!(kind, piece.kind);
        assert!(
            piece
                .owned_regions
                .iter()
                .any(|region| region.kind == owner_kind && region.owner_index == owner_index),
            "node earthwork face owner must refer to a canonical owned top region"
        );
        let boundary_source = boundary_source
            .expect("node earthwork face must carry exact boundary endpoint provenance");
        assert_node_footprint_boundary_vertex_source_is_valid(piece, boundary_source.start);
        assert_node_footprint_boundary_vertex_source_is_valid(piece, boundary_source.end);
    }
}

fn assert_node_footprint_boundary_vertex_source_is_valid(
    piece: &RoadSurfaceVisualNodePiece,
    source: NodeFootprintBoundaryVertexSource,
) {
    match source {
        NodeFootprintBoundaryVertexSource::Direct(direct) => {
            assert!(
                direct.top_surface_source_index < piece.node_top_surface_sources.len(),
                "direct boundary source must reference an emitted top surface source"
            );
            assert!(
                direct.grade_authority_index < piece.node_grade_authorities.len(),
                "direct boundary source must reference node grade authority"
            );
        }
        NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start,
            owning_segment_end,
            ..
        } => {
            assert_node_footprint_boundary_vertex_source_is_valid(
                piece,
                NodeFootprintBoundaryVertexSource::Direct(owning_segment_start),
            );
            assert_node_footprint_boundary_vertex_source_is_valid(
                piece,
                NodeFootprintBoundaryVertexSource::Direct(owning_segment_end),
            );
        }
    }
}

fn assert_span_earthwork_faces_have_support_provenance(
    piece: &super::RoadSurfaceVisualSpanPiece,
    edge_idx: usize,
    edge_class: EdgeClass,
) {
    assert!(
        !piece.render_earthwork_faces.is_empty(),
        "span earthwork faces should be generated from span support region boundaries"
    );
    let expected_policy = RoadSurfaceEarthworkSupportPolicy::from_edge_class(edge_class);
    for face in &piece.render_earthwork_faces {
        let RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
            edge_idx: source_edge_idx,
            edge_class: source_edge_class,
            support_policy,
            owner,
            role,
            start_section_index,
            end_section_index,
            start_s_m,
            end_s_m,
        } = face.source
        else {
            panic!(
                "span earthwork face must carry span support provenance, got {:?}",
                face.source
            );
        };
        assert_eq!(source_edge_idx, edge_idx);
        assert_eq!(source_edge_class, edge_class);
        assert_eq!(support_policy, expected_policy);
        assert!(
            piece.span_earthwork_support_regions.iter().any(|region| {
                region.edge_idx == source_edge_idx
                    && region.owner == owner
                    && region.role == role
                    && region.start_section_index == start_section_index
                    && region.end_section_index == end_section_index
                    && (region.start_s_m - start_s_m).abs() <= SAMPLE_EPSILON_M
                    && (region.end_s_m - end_s_m).abs() <= SAMPLE_EPSILON_M
            }),
            "span earthwork face source must refer to a stored support region"
        );
    }
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

fn assert_raised_step_face_lower_edge_covers(
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
        "raised-step face lower edge must cover expected segment; label={label} start={start:?} end={end:?} covered={covered_length:.4} expected={expected_length:.4}"
    );
}

#[derive(Clone, Copy, Debug)]
struct TestTopBoundaryEdge {
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    start: Vector3,
    end: Vector3,
    key: TestRenderEdgeKey,
    xz_key: TestRenderXzEdgeKey,
    avg_y_m: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct TestRenderVertexKey {
    x_key: i64,
    y_mm: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct TestRenderEdgeKey {
    start: TestRenderVertexKey,
    end: TestRenderVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct TestRenderXzVertexKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct TestRenderXzEdgeKey {
    start: TestRenderXzVertexKey,
    end: TestRenderXzVertexKey,
}

impl TestRenderVertexKey {
    fn from_point(point: Vector3) -> Self {
        let (x_key, z_key) = test_xz_key(point);
        Self {
            x_key,
            y_mm: (point.y * 1000.0).round() as i64,
            z_key,
        }
    }

    fn xz(self) -> TestRenderXzVertexKey {
        TestRenderXzVertexKey {
            x_key: self.x_key,
            z_key: self.z_key,
        }
    }
}

impl TestRenderXzVertexKey {
    fn from_arrangement_key(key: super::arrangement::NodeArrangementKey) -> Self {
        Self {
            x_key: key.x_key(),
            z_key: key.z_key(),
        }
    }
}

impl TestRenderEdgeKey {
    fn normalized(start: Vector3, end: Vector3) -> Option<Self> {
        let start = TestRenderVertexKey::from_point(start);
        let end = TestRenderVertexKey::from_point(end);
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }

    fn xz(self) -> TestRenderXzEdgeKey {
        let start = self.start.xz();
        let end = self.end.xz();
        if start <= end {
            TestRenderXzEdgeKey { start, end }
        } else {
            TestRenderXzEdgeKey {
                start: end,
                end: start,
            }
        }
    }
}

impl TestRenderXzEdgeKey {
    fn normalized_from_arrangement_keys(
        start: super::arrangement::NodeArrangementKey,
        end: super::arrangement::NodeArrangementKey,
    ) -> Option<Self> {
        let start = TestRenderXzVertexKey::from_arrangement_key(start);
        let end = TestRenderXzVertexKey::from_arrangement_key(end);
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }

    fn contains(self, edge: Self) -> bool {
        test_render_xz_vertex_key_lies_on_segment(edge.start, self.start, self.end)
            && test_render_xz_vertex_key_lies_on_segment(edge.end, self.start, self.end)
    }
}

fn test_render_xz_vertex_key_lies_on_segment(
    point: TestRenderXzVertexKey,
    start: TestRenderXzVertexKey,
    end: TestRenderXzVertexKey,
) -> bool {
    let dx = i128::from(end.x_key - start.x_key);
    let dz = i128::from(end.z_key - start.z_key);
    let px = i128::from(point.x_key - start.x_key);
    let pz = i128::from(point.z_key - start.z_key);
    dx * pz - dz * px == 0
        && point.x_key >= start.x_key.min(end.x_key)
        && point.x_key <= start.x_key.max(end.x_key)
        && point.z_key >= start.z_key.min(end.z_key)
        && point.z_key <= start.z_key.max(end.z_key)
}

fn assert_top_raised_step_owner_boundaries_have_vertical_faces(piece: &RoadSurfaceVisualNodePiece) {
    let top_edges = test_owned_top_boundary_edges(piece);
    let face_lower_keys = piece
        .raised_step_face_polygons
        .iter()
        .filter_map(vertical_face_lower_edge_for_test)
        .filter_map(|edge| TestRenderEdgeKey::normalized(edge[0], edge[1]).map(|key| key.xz()))
        .collect::<Vec<_>>();
    let mut edges_by_xz = BTreeMap::<TestRenderXzEdgeKey, Vec<TestTopBoundaryEdge>>::new();
    for edge in top_edges {
        edges_by_xz.entry(edge.xz_key).or_default().push(edge);
    }

    for edges in edges_by_xz.values() {
        for (left_index, left_edge) in edges.iter().enumerate() {
            for right_edge in edges.iter().skip(left_index + 1) {
                let (lower_edge, raised_edge) = if left_edge.avg_y_m <= right_edge.avg_y_m {
                    (*left_edge, *right_edge)
                } else {
                    (*right_edge, *left_edge)
                };
                if lower_edge.key == raised_edge.key
                    || lower_edge.avg_y_m >= raised_edge.avg_y_m
                    || !test_top_edges_form_raised_step(lower_edge, raised_edge)
                {
                    continue;
                }
                let matching_canonical_steps =
                    explicit_vertical_step_descriptions_for_xz_key(piece, lower_edge.xz_key);
                if matching_canonical_steps.is_empty() {
                    continue;
                }
                assert!(
                    face_lower_keys
                        .iter()
                        .copied()
                        .any(|face_key| face_key.contains(lower_edge.xz_key)),
                    "surviving raised-step owner boundary must emit an explicit vertical face; kind={:?} xz_key={:?} lower_owner={:?}[{}] lower={:?}->{:?} raised_owner={:?}[{}] raised={:?}->{:?} matching_canonical_steps={:?} face_lower_keys={:?}",
                    piece.kind,
                    lower_edge.xz_key,
                    lower_edge.kind,
                    lower_edge.owner_index,
                    lower_edge.start,
                    lower_edge.end,
                    raised_edge.kind,
                    raised_edge.owner_index,
                    raised_edge.start,
                    raised_edge.end,
                    matching_canonical_steps,
                    face_lower_keys
                );
            }
        }
    }
}

fn explicit_vertical_step_descriptions_for_xz_key(
    piece: &RoadSurfaceVisualNodePiece,
    xz_key: TestRenderXzEdgeKey,
) -> Vec<String> {
    piece
        .explicit_vertical_step_segments
        .iter()
        .enumerate()
        .filter_map(|(step_index, segment)| {
            TestRenderXzEdgeKey::normalized_from_arrangement_keys(segment.start(), segment.end())
                .filter(|step_key| step_key.contains(xz_key))
                .map(|_| {
                    format!(
                        "#{step_index} {:?}<->{:?} {:?}->{:?}",
                        segment.owner(),
                        segment.opposite_owner(),
                        segment.start(),
                        segment.end()
                    )
                })
        })
        .collect()
}

fn assert_canonical_explicit_vertical_steps_have_faces(piece: &RoadSurfaceVisualNodePiece) {
    let top_edges = test_owned_top_boundary_edges(piece);
    let mut top_edges_by_xz = BTreeMap::<TestRenderXzEdgeKey, Vec<TestTopBoundaryEdge>>::new();
    for edge in top_edges {
        top_edges_by_xz.entry(edge.xz_key).or_default().push(edge);
    }
    let face_source_segments = piece
        .raised_step_face_sources
        .iter()
        .map(|source| source.segment())
        .collect::<BTreeSet<_>>();

    for (step_index, segment) in piece.explicit_vertical_step_segments.iter().enumerate() {
        let owner = segment.owner();
        let opposite_owner = segment.opposite_owner();
        let owner_pair_requires_face =
            test_owners_form_raised_step(owner.kind(), opposite_owner.kind());
        if !owner_pair_requires_face {
            continue;
        }
        if explicit_vertical_step_segment_len_squared_m2(*segment)
            <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M
        {
            continue;
        }
        if !explicit_vertical_step_has_visible_top_support(*segment, &top_edges_by_xz) {
            continue;
        }

        assert!(
            face_source_segments.contains(segment),
            "canonical explicit vertical step must be consumed by a rendered vertical face; kind={:?} step_index={} segment={:?}",
            piece.kind,
            step_index,
            segment
        );
    }
}

fn explicit_vertical_step_has_visible_top_support(
    segment: super::arrangement::NodeExplicitVerticalStepSegment,
    top_edges_by_xz: &BTreeMap<TestRenderXzEdgeKey, Vec<TestTopBoundaryEdge>>,
) -> bool {
    let Some(xz_key) =
        TestRenderXzEdgeKey::normalized_from_arrangement_keys(segment.start(), segment.end())
    else {
        return false;
    };
    let Some(edges) = top_edges_by_xz.get(&xz_key) else {
        return false;
    };
    edges.iter().any(|lower_edge| {
        edges.iter().any(|raised_edge| {
            lower_edge.avg_y_m < raised_edge.avg_y_m
                && test_top_edges_form_raised_step(*lower_edge, *raised_edge)
        })
    })
}

fn test_owners_form_raised_step(
    lower_kind: RoadSurfaceBandKind,
    raised_kind: RoadSurfaceBandKind,
) -> bool {
    ordered_raised_step_kinds(lower_kind, raised_kind) == Some((lower_kind, raised_kind))
}

fn test_top_edges_form_raised_step(
    lower_edge: TestTopBoundaryEdge,
    raised_edge: TestTopBoundaryEdge,
) -> bool {
    test_owners_form_raised_step(lower_edge.kind, raised_edge.kind)
}

fn explicit_vertical_step_segment_len_squared_m2(
    segment: super::arrangement::NodeExplicitVerticalStepSegment,
) -> f32 {
    let dx = (segment.end().x_key() - segment.start().x_key()) as f64
        / super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
    let dz = (segment.end().z_key() - segment.start().z_key()) as f64
        / super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
    (dx * dx + dz * dz) as f32
}

fn test_owned_top_boundary_edges(piece: &RoadSurfaceVisualNodePiece) -> Vec<TestTopBoundaryEdge> {
    let mut boundary_edges = Vec::new();
    for region in &piece.owned_regions {
        let mut edge_counts = BTreeMap::<TestRenderEdgeKey, (usize, Vector3, Vector3)>::new();
        if region.polygon.triangles_world.is_empty() {
            let points = &region.polygon.points_world;
            if points.len() >= 2 {
                for index in 0..points.len() {
                    if let Some(key) = TestRenderEdgeKey::normalized(
                        points[index],
                        points[(index + 1) % points.len()],
                    ) {
                        edge_counts
                            .entry(key)
                            .and_modify(|entry| entry.0 += 1)
                            .or_insert((1, points[index], points[(index + 1) % points.len()]));
                    }
                }
            }
        } else {
            for triangle in &region.polygon.triangles_world {
                for edge_index in 0..3 {
                    if let Some(key) = TestRenderEdgeKey::normalized(
                        triangle[edge_index],
                        triangle[(edge_index + 1) % 3],
                    ) {
                        edge_counts
                            .entry(key)
                            .and_modify(|entry| entry.0 += 1)
                            .or_insert((1, triangle[edge_index], triangle[(edge_index + 1) % 3]));
                    }
                }
            }
        }
        for (key, (count, start, end)) in edge_counts {
            if count == 1 {
                boundary_edges.push(TestTopBoundaryEdge {
                    kind: region.kind,
                    owner_index: region.owner_index,
                    start,
                    end,
                    key,
                    xz_key: key.xz(),
                    avg_y_m: (start.y + end.y) * 0.5,
                });
            }
        }
    }
    boundary_edges
}

fn vertical_face_lower_edge_for_test(polygon: &RoadSurfaceVisualPolygon) -> Option<[Vector3; 2]> {
    let [first_edge, second_edge] = vertical_face_side_edges_for_test(polygon)?;
    let first_avg_y = (first_edge[0].y + first_edge[1].y) * 0.5;
    let second_avg_y = (second_edge[0].y + second_edge[1].y) * 0.5;
    Some(if first_avg_y <= second_avg_y {
        first_edge
    } else {
        second_edge
    })
}

fn vertical_face_side_edges_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<[[Vector3; 2]; 2]> {
    let [a, b, c, d] = polygon.points_world.as_slice() else {
        return None;
    };
    Some([[*a, *d], [*b, *c]])
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

fn assert_raised_step_faces_visible_from_lower_owner(piece: &RoadSurfaceVisualNodePiece) {
    let top_edges = test_owned_top_boundary_edges(piece);
    for (face, source) in piece
        .raised_step_face_polygons
        .iter()
        .zip(piece.raised_step_face_sources.iter())
    {
        let Some(lower_owner) = test_lower_owner_from_vertical_face_source(*source) else {
            continue;
        };
        let Some(visible_direction) = vertical_face_visible_direction_for_test(face) else {
            continue;
        };
        let visible_direction =
            Vector3::new(visible_direction.x, 0.0, visible_direction.z).normalized();
        let Some(lower_edge) = vertical_face_owner_edge_for_test(face, &top_edges, lower_owner)
        else {
            continue;
        };
        let midpoint = (lower_edge[0] + lower_edge[1]) * 0.5;
        let mut best_dot: Option<f32> = None;

        for region in piece.owned_regions.iter().filter(|region| {
            region.kind == lower_owner.kind() && region.owner_index == lower_owner.owner_index()
        }) {
            let Some(centroid) = polygon_centroid_for_test(&region.polygon) else {
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
                dot > -0.25,
                "raised-step face must be visible from its lower owner; kind={:?} face={:?} visible_direction={visible_direction:?} dot={dot:.6}",
                piece.kind,
                face.points_world
            );
        }
    }
}

fn test_lower_owner_from_vertical_face_source(
    source: super::RoadSurfaceVerticalFaceSource,
) -> Option<NodeBandOwner> {
    let segment = source.segment();
    let owner = segment.owner();
    let opposite_owner = segment.opposite_owner();
    let (lower_kind, _) = ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
    Some(if owner.kind() == lower_kind {
        owner
    } else {
        opposite_owner
    })
}

fn vertical_face_owner_edge_for_test(
    face: &RoadSurfaceVisualPolygon,
    top_edges: &[TestTopBoundaryEdge],
    owner: NodeBandOwner,
) -> Option<[Vector3; 2]> {
    let [first_edge, second_edge] = vertical_face_side_edges_for_test(face)?;
    [first_edge, second_edge].into_iter().find(|edge| {
        let Some(edge_key) = TestRenderEdgeKey::normalized(edge[0], edge[1]).map(|key| key.xz())
        else {
            return false;
        };
        top_edges.iter().any(|top_edge| {
            top_edge.xz_key == edge_key
                && top_edge.kind == owner.kind()
                && top_edge.owner_index == owner.owner_index()
        })
    })
}

fn assert_raised_step_faces_have_top_support(piece: &RoadSurfaceVisualNodePiece) {
    for face in &piece.raised_step_face_polygons {
        let Some(lower_edge) = vertical_face_lower_edge_for_test(face) else {
            panic!(
                "raised-step face must expose a non-degenerate lower edge; face={:?}",
                face.points_world
            );
        };
        let Some(upper_edge) = vertical_face_upper_edge_for_test(face) else {
            panic!(
                "raised-step face must expose a non-degenerate upper edge; face={:?}",
                face.points_world
            );
        };
        let lower_matches = piece
            .owned_regions
            .iter()
            .filter(|region| {
                polygon_boundary_overlaps_edge_at_height_for_test(&region.polygon, lower_edge)
            })
            .collect::<Vec<_>>();
        let upper_matches = piece
            .owned_regions
            .iter()
            .filter(|region| {
                polygon_boundary_overlaps_edge_at_height_for_test(&region.polygon, upper_edge)
            })
            .collect::<Vec<_>>();
        assert!(
            !lower_matches.is_empty(),
            "raised-step face lower edge must be backed by a top owner; lower_edge={lower_edge:?} face={:?}",
            face.points_world
        );
        assert!(
            !upper_matches.is_empty(),
            "raised-step face upper edge must be backed by a top owner; upper_edge={upper_edge:?} face={:?}",
            face.points_world
        );
        assert!(
            lower_matches.iter().any(|lower_match| {
                upper_matches.iter().any(|upper_match| {
                    test_owners_form_raised_step(lower_match.kind, upper_match.kind)
                })
            }),
            "raised-step face support edges must belong to an explicit raised-step owner pair; lower_edge={lower_edge:?} upper_edge={upper_edge:?} face={:?}",
            face.points_world
        );
    }
}

fn vertical_face_visible_direction_for_test(polygon: &RoadSurfaceVisualPolygon) -> Option<Vector3> {
    let [upper_start, lower_start, lower_end, _upper_end] = polygon.points_world.as_slice() else {
        return None;
    };
    let normal = (*lower_start - *upper_start).cross(*lower_end - *upper_start);
    (normal.length_squared() > 1e-8).then(|| -normal.normalized())
}

fn vertical_face_upper_edge_for_test(polygon: &RoadSurfaceVisualPolygon) -> Option<[Vector3; 2]> {
    let [a, b, c, d] = polygon.points_world.as_slice() else {
        return None;
    };
    let first_edge = [*a, *d];
    let second_edge = [*b, *c];
    let first_avg_y = (first_edge[0].y + first_edge[1].y) * 0.5;
    let second_avg_y = (second_edge[0].y + second_edge[1].y) * 0.5;
    Some(if first_avg_y >= second_avg_y {
        first_edge
    } else {
        second_edge
    })
}

fn polygon_boundary_overlaps_edge_at_height_for_test(
    polygon: &RoadSurfaceVisualPolygon,
    edge: [Vector3; 2],
) -> bool {
    if !polygon.triangles_world.is_empty() {
        let mut triangle_edges = BTreeMap::<TestRenderEdgeKey, (usize, [Vector3; 2])>::new();
        for triangle in &polygon.triangles_world {
            for edge_index in 0..3 {
                let start = triangle[edge_index];
                let end = triangle[(edge_index + 1) % 3];
                let Some(key) = TestRenderEdgeKey::normalized(start, end) else {
                    continue;
                };
                triangle_edges
                    .entry(key)
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((1, [start, end]));
            }
        }
        return triangle_edges
            .into_values()
            .filter_map(|(count, boundary_edge)| (count == 1).then_some(boundary_edge))
            .any(|boundary_edge| test_boundary_edge_contains_edge_at_height(boundary_edge, edge));
    }

    let points = &polygon.points_world;
    if points.len() < 2 {
        return false;
    }
    (0..points.len()).any(|index| {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        test_boundary_edge_contains_edge_at_height([start, end], edge)
    })
}

fn test_boundary_edge_contains_edge_at_height(
    boundary_edge: [Vector3; 2],
    edge: [Vector3; 2],
) -> bool {
    let boundary_start = TestRenderVertexKey::from_point(boundary_edge[0]);
    let boundary_end = TestRenderVertexKey::from_point(boundary_edge[1]);
    let edge_start = TestRenderVertexKey::from_point(edge[0]);
    let edge_end = TestRenderVertexKey::from_point(edge[1]);
    if !test_xz_segments_overlap_with_length(
        (boundary_start.x_key, boundary_start.z_key),
        (boundary_end.x_key, boundary_end.z_key),
        (edge_start.x_key, edge_start.z_key),
        (edge_end.x_key, edge_end.z_key),
    ) {
        return false;
    }
    let Some((start_numerator, start_denominator)) =
        test_boundary_segment_parameter_xz(edge_start, boundary_start, boundary_end)
    else {
        return false;
    };
    let Some((end_numerator, end_denominator)) =
        test_boundary_segment_parameter_xz(edge_end, boundary_start, boundary_end)
    else {
        return false;
    };
    if start_numerator < 0
        || start_numerator > start_denominator
        || end_numerator < 0
        || end_numerator > end_denominator
    {
        return false;
    }
    (test_interpolated_height_mm(
        boundary_start,
        boundary_end,
        start_numerator,
        start_denominator,
    ) - edge_start.y_mm)
        .abs()
        <= 1
        && (test_interpolated_height_mm(
            boundary_start,
            boundary_end,
            end_numerator,
            end_denominator,
        ) - edge_end.y_mm)
            .abs()
            <= 1
}

fn test_boundary_segment_parameter_xz(
    point: TestRenderVertexKey,
    start: TestRenderVertexKey,
    end: TestRenderVertexKey,
) -> Option<(i128, i128)> {
    let dx = end.x_key - start.x_key;
    let dz = end.z_key - start.z_key;
    let px = point.x_key - start.x_key;
    let pz = point.z_key - start.z_key;
    let length_squared = i128::from(dx) * i128::from(dx) + i128::from(dz) * i128::from(dz);
    if length_squared == 0 || i128::from(dx) * i128::from(pz) - i128::from(dz) * i128::from(px) != 0
    {
        return None;
    }
    Some((
        i128::from(px) * i128::from(dx) + i128::from(pz) * i128::from(dz),
        length_squared,
    ))
}

fn test_interpolated_height_mm(
    start: TestRenderVertexKey,
    end: TestRenderVertexKey,
    numerator: i128,
    denominator: i128,
) -> i64 {
    let value =
        i128::from(start.y_mm) * denominator + i128::from(end.y_mm - start.y_mm) * numerator;
    if value >= 0 {
        ((value + denominator / 2) / denominator) as i64
    } else {
        -(((-value + denominator / 2) / denominator) as i64)
    }
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
    assert_node_top_surface_sources_have_grade_authority(piece);
    assert_node_terrain_clip_sources_have_footprint_provenance(piece);
}

fn assert_node_top_surface_sources_have_grade_authority(piece: &RoadSurfaceVisualNodePiece) {
    assert_eq!(
        piece.node_top_surface_sources.len(),
        piece.owned_regions.len(),
        "every emitted node top region must carry one provenance record"
    );
    assert!(
        !piece.node_grade_authorities.is_empty(),
        "node top provenance must reference a non-empty grade-authority table"
    );
    for source in &piece.node_top_surface_sources {
        assert!(
            !source.vertex_sources.is_empty(),
            "node top provenance must name polygon vertex sources"
        );
        assert!(
            !source.triangle_sources.is_empty(),
            "node top provenance must name emitted triangle sources"
        );
        for grade_authority_index in
            source
                .vertex_sources
                .iter()
                .map(|source| source.grade_authority_index)
                .chain(source.triangle_sources.iter().flat_map(|triangle| {
                    triangle.iter().map(|source| source.grade_authority_index)
                }))
        {
            assert!(
                grade_authority_index < piece.node_grade_authorities.len(),
                "node top provenance index {grade_authority_index} must reference an emitted grade-authority row"
            );
        }
    }
}

fn assert_node_terrain_clip_sources_have_footprint_provenance(piece: &RoadSurfaceVisualNodePiece) {
    for edge in piece
        .terrain_clip_boundary_loops
        .iter()
        .flat_map(|boundary_loop| boundary_loop.source_edges.iter())
    {
        let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id,
            kind,
            owner_kind,
            owner_index,
            boundary_source,
        } = edge.source
        else {
            panic!(
                "node terrain clip edge must carry node footprint provenance, got {:?}",
                edge.source
            );
        };
        assert_eq!(node_id, piece.node_id);
        assert_eq!(kind, piece.kind);
        assert!(
            piece
                .owned_regions
                .iter()
                .any(|region| region.kind == owner_kind && region.owner_index == owner_index),
            "node terrain clip edge owner must refer to a canonical owned top region"
        );
        let boundary_source =
            boundary_source.expect("node terrain clip edge must carry exact endpoint provenance");
        assert_node_footprint_boundary_vertex_source_is_valid(piece, boundary_source.start);
        assert_node_footprint_boundary_vertex_source_is_valid(piece, boundary_source.end);
    }
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

fn assert_compiled_bend_piece<'a>(
    surface: &'a RoadSurfaceSystem,
    graph: &RegionGraph,
    bend: u32,
) -> &'a RoadSurfaceVisualNodePiece {
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .unwrap_or_else(|| {
            panic!(
                "bend should compile through canonical owned regions: {}",
                canonical_node_pipeline_report(
                    surface,
                    graph,
                    bend,
                    RoadSurfaceVisualNodePieceKind::Bend
                )
            )
        });
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(
        !piece.render_earthwork_faces.is_empty(),
        "bend piece must emit terrain skirt faces from its canonical outer boundary"
    );
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.curb_surface_polygons.is_empty());
    assert!(!piece.raised_step_face_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(piece);
    assert_top_surface_triangles_face_up(piece);
    assert_raised_step_faces_have_top_support(piece);
    assert_raised_step_faces_visible_from_lower_owner(piece);
    assert_top_raised_step_owner_boundaries_have_vertical_faces(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
    assert_node_top_covers_footprint(piece);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    assert_earthwork_faces_stay_outside_top_footprint(piece);
    piece
}

fn assert_compiled_junction_piece<'a>(
    surface: &'a RoadSurfaceSystem,
    graph: &RegionGraph,
    junction: u32,
) -> &'a RoadSurfaceVisualNodePiece {
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&junction)
        .unwrap_or_else(|| {
            panic!(
                "junction should compile through canonical owned regions: {}",
                canonical_junction_pipeline_report(surface, graph, junction)
            )
        });
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_piece_has_curb_and_sidewalk_owners(piece);
    assert_material_triangles_do_not_overlap(piece);
    assert!(!piece.outer_boundary_loops.is_empty());
    assert!(
        !piece.render_earthwork_faces.is_empty(),
        "junction piece must emit terrain skirt faces from its canonical outer boundary"
    );
    assert!(!piece.road_surface_polygons.is_empty());
    assert!(!piece.curb_surface_polygons.is_empty());
    assert!(!piece.raised_step_face_polygons.is_empty());
    assert!(!piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(piece);
    assert_top_surface_triangles_face_up(piece);
    assert_raised_step_faces_have_top_support(piece);
    assert_raised_step_faces_visible_from_lower_owner(piece);
    assert_top_raised_step_owner_boundaries_have_vertical_faces(piece);
    assert_outer_boundary_vertices_match_visible_top(piece);
    assert_node_top_covers_footprint(piece);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    assert_earthwork_faces_stay_outside_top_footprint(piece);
    piece
}

fn canonical_junction_pipeline_report(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    node_id: u32,
) -> String {
    canonical_node_pipeline_report(
        surface,
        graph,
        node_id,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    )
}

fn canonical_node_pipeline_report(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    node_id: u32,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> String {
    let valid = graph.get_valid_node(node_id);
    let incidents = surface.sorted_incident_surface_edges(graph, valid);
    let Some(mouths) = surface.build_ordered_piece_mouths(&incidents) else {
        return format!("node {node_id}: failed to build ordered mouths");
    };
    let input = match RoadSurfaceSystem::build_node_arrangement_input_from_mouths(
        node_id, piece_kind, &mouths,
    ) {
        Ok(input) => input,
        Err(error) => return format!("node {node_id}: input extraction failed: {error:?}"),
    };
    let rails = match RoadSurfaceSystem::build_node_rail_contours_from_input(&input) {
        Ok(rails) => rails,
        Err(error) => {
            return NodeValidationReport::from_rail_generation_error(node_id, piece_kind, &error)
                .debug_dump();
        }
    };
    let ownership = match RoadSurfaceSystem::build_node_boolean_ownership_from_rails(&rails) {
        Ok(ownership) => ownership,
        Err(error) => {
            return format!(
                "{} error={error:?}",
                NodeValidationReport::from_boolean_ownership_error(node_id, piece_kind, &error)
                    .debug_dump()
            );
        }
    };
    if let Some(report) = NodeValidationReport::from_owned_region_arrangement_diagnostics(
        &ownership.owned_region_arrangement,
    ) {
        return report.debug_dump();
    }
    let heights = match RoadSurfaceSystem::build_node_height_solution_from_ownership(
        &input, &rails, &ownership,
    ) {
        Ok(heights) => heights,
        Err(error) => {
            if let NodeHeightFieldError::SharedSourceHeightConflict {
                constraint_index: Some(constraint_index),
                ..
            } = &error
            {
                return format!(
                    "{} {}",
                    NodeValidationReport::from_height_field_error(node_id, piece_kind, &error,)
                        .debug_dump(),
                    source_rail_debug_for_height_conflict(
                        &input,
                        rails.constraints.get(*constraint_index)
                    )
                );
            }
            return NodeValidationReport::from_height_field_error(node_id, piece_kind, &error)
                .debug_dump();
        }
    };
    let mut arrangement = match NodeArrangement::from_height_solution(&heights) {
        Ok(arrangement) => arrangement,
        Err(error) => {
            if let NodeArrangementError::DuplicateVertexHeightConflict { key, .. } = &error {
                return format!(
                    "{} vertices_at_key={:?}",
                    NodeValidationReport::from_arrangement_error(node_id, piece_kind, &error,)
                        .debug_dump(),
                    height_solution_vertices_at_arrangement_key(&heights, *key)
                );
            }
            return NodeValidationReport::from_arrangement_error(node_id, piece_kind, &error)
                .debug_dump();
        }
    };
    if let Some(report) = NodeValidationReport::from_arrangement_diagnostics(&arrangement) {
        return report.debug_dump();
    }
    let triangulation =
        match RoadSurfaceSystem::build_node_triangulation_from_arrangement(&arrangement) {
            Ok(triangulation) => triangulation,
            Err(error) => {
                return NodeValidationReport::from_triangulation_error(node_id, piece_kind, &error)
                    .debug_dump();
            }
        };
    match RoadSurfaceSystem::validate_node_triangulation_solution(&triangulation) {
        Ok(report) => {
            if !report.diagnostics.is_empty() {
                return report.debug_dump();
            }
        }
        Err(error) => {
            if let Some(extra) =
                triangulation_height_conflict_debug(&heights, &ownership, &error.report)
            {
                return format!("{} {extra}", error.report.debug_dump());
            }
            if let Some(extra) =
                triangulation_duplicate_exposed_edge_debug(&triangulation, &error.report)
            {
                return format!("{} {extra}", error.report.debug_dump());
            }
            return error.report.debug_dump();
        }
    }
    if let Err(error) = arrangement.attach_triangulation(&triangulation) {
        return NodeValidationReport::from_arrangement_error(node_id, piece_kind, &error)
            .debug_dump();
    }
    if let Err(error) = RoadSurfaceSystem::node_surface_regions_from_arrangement(
        &arrangement,
        &ownership.footprint_shapes,
    ) {
        return format!(
            "boundary export failed: {error:?} {}",
            boundary_export_step_debug(&arrangement, &error)
        );
    }
    format!("canonical {piece_kind:?} pipeline reached boundary export")
}

fn boundary_export_step_debug(
    arrangement: &NodeArrangement,
    error: &super::node::boundary::NodeBoundaryExportError,
) -> String {
    if matches!(
        error,
        super::node::boundary::NodeBoundaryExportError::DegenerateOuterBoundaryLoop
    ) {
        let mut degree = BTreeMap::<(i64, i64), usize>::new();
        let mut exposed = Vec::new();
        for edge in arrangement
            .edges()
            .iter()
            .filter(|edge| edge.exposed_boundary())
        {
            let Some(start) = arrangement.vertices().get(edge.start().index()) else {
                continue;
            };
            let Some(end) = arrangement.vertices().get(edge.end().index()) else {
                continue;
            };
            let start_key = (start.key().x_key(), start.key().z_key(), start.height_mm());
            let end_key = (end.key().x_key(), end.key().z_key(), end.height_mm());
            exposed.push((start_key, end_key));
            *degree
                .entry((start.key().x_key(), start.key().z_key()))
                .or_default() += 1;
            *degree
                .entry((end.key().x_key(), end.key().z_key()))
                .or_default() += 1;
        }
        let bad_degree = degree
            .into_iter()
            .filter(|(_, count)| *count != 2)
            .take(24)
            .collect::<Vec<_>>();
        return format!(
            "exposed_edge_count={} bad_xz_degrees={bad_degree:?} first_edges={:?}",
            exposed.len(),
            exposed.into_iter().take(24).collect::<Vec<_>>()
        );
    }
    let super::node::boundary::NodeBoundaryExportError::ConflictingFootprintBoundaryHeight {
        x_key,
        z_key,
        existing_owner_kind,
        existing_owner_index,
        incoming_owner_kind,
        incoming_owner_index,
        ..
    } = error
    else {
        return String::new();
    };
    let key = NodeArrangementKey::from_point(super::backend::RoadVec2::new(
        *x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
        *z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
    ));
    let existing_owner = NodeBandOwner::new(*existing_owner_kind, *existing_owner_index);
    let incoming_owner = NodeBandOwner::new(*incoming_owner_kind, *incoming_owner_index);
    let step_segments = arrangement.explicit_vertical_step_segments();
    let owner_pair_segments = step_segments
        .iter()
        .filter(|segment| {
            (segment.owner() == existing_owner && segment.opposite_owner() == incoming_owner)
                || (segment.owner() == incoming_owner && segment.opposite_owner() == existing_owner)
        })
        .copied()
        .collect::<Vec<_>>();
    let key_segments = owner_pair_segments
        .iter()
        .filter(|segment| {
            super::segments::arrangement_key_lies_on_segment(key, segment.start(), segment.end())
        })
        .copied()
        .collect::<Vec<_>>();
    format!(
        "boundary_key={key:?} owner_pair_segments={owner_pair_segments:?} key_segments={key_segments:?}"
    )
}

fn assert_junction_rejected_with_canonical_height_diagnostic(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    node_id: u32,
    label: &str,
) {
    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&node_id),
        "{label} unexpectedly compiled after same-XZ height disagreement"
    );
    let report = canonical_junction_pipeline_report(surface, graph, node_id);
    let accepted_height_rejection = report.contains("shared_source_height_conflict")
        || report.contains("source_height_field_conflict")
        || report.contains("generated_contour_source_handoff_height_mismatch")
        || report.contains("vertex_outside_height_field");
    assert!(
        accepted_height_rejection,
        "{label} must reject with a canonical height diagnostic: {report}"
    );
}

fn triangulation_height_conflict_debug(
    heights: &super::height::NodeHeightSolution,
    ownership: &super::ownership::NodeBooleanOwnership,
    report: &NodeValidationReport,
) -> Option<String> {
    report.diagnostics.iter().find_map(|diagnostic| {
        if let NodeGeometryDiagnosticKind::CrossRegionHeightConflict {
            edge_start_x_key,
            edge_start_z_key,
            edge_end_x_key,
            edge_end_z_key,
            ..
        } = diagnostic.kind
        {
            let start_key = arrangement_key_from_overlay_keys(edge_start_x_key, edge_start_z_key);
            let end_key = arrangement_key_from_overlay_keys(edge_end_x_key, edge_end_z_key);
            Some(format!(
                "start_vertices={:?} end_vertices={:?} ownership={:?}",
                height_solution_vertices_at_arrangement_key(heights, start_key),
                height_solution_vertices_at_arrangement_key(heights, end_key),
                owned_region_claims_for_height_conflict(ownership, diagnostic)
            ))
        } else {
            None
        }
    })
}

fn triangulation_duplicate_exposed_edge_debug(
    triangulation: &super::triangulation::NodeTriangulationSolution,
    report: &NodeValidationReport,
) -> Option<String> {
    report.diagnostics.iter().find_map(|diagnostic| {
        if let NodeGeometryDiagnosticKind::DuplicateExposedEdge {
            start_x_mm,
            start_z_mm,
            end_x_mm,
            end_z_mm,
            ..
        } = diagnostic.kind
        {
            Some(format!(
                "duplicate_edge_regions={:?}",
                triangulation_regions_for_exposed_edge(
                    triangulation,
                    (start_x_mm, start_z_mm),
                    (end_x_mm, end_z_mm),
                )
            ))
        } else {
            None
        }
    })
}

fn triangulation_regions_for_exposed_edge(
    triangulation: &super::triangulation::NodeTriangulationSolution,
    start_mm: (i64, i64),
    end_mm: (i64, i64),
) -> Vec<String> {
    let expected = normalized_test_mm_edge_key(start_mm, end_mm);
    let mut matches = Vec::new();
    for (region_index, region) in triangulation.regions.iter().enumerate() {
        let mut edge_counts = BTreeMap::<((i64, i64), (i64, i64)), usize>::new();
        for triangle in &region.triangles {
            for edge_index in 0..3 {
                let start = &region.vertices[triangle.vertices[edge_index]];
                let end = &region.vertices[triangle.vertices[(edge_index + 1) % 3]];
                *edge_counts
                    .entry(normalized_test_world_mm_edge_key(
                        start.point_world.x as f32,
                        start.point_world.z as f32,
                        end.point_world.x as f32,
                        end.point_world.z as f32,
                    ))
                    .or_default() += 1;
            }
        }
        if let Some(count) = edge_counts.get(&expected).copied() {
            matches.push(format!(
                "region={} owner={:?} height_field={:?} local_count={}",
                region_index, region.owner, region.height_field_id, count
            ));
        }
    }
    matches
}

fn normalized_test_world_mm_edge_key(
    start_x: f32,
    start_z: f32,
    end_x: f32,
    end_z: f32,
) -> ((i64, i64), (i64, i64)) {
    normalized_test_mm_edge_key(
        (
            (start_x * 1000.0).round() as i64,
            (start_z * 1000.0).round() as i64,
        ),
        (
            (end_x * 1000.0).round() as i64,
            (end_z * 1000.0).round() as i64,
        ),
    )
}

fn normalized_test_mm_edge_key(start: (i64, i64), end: (i64, i64)) -> ((i64, i64), (i64, i64)) {
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

fn owned_region_claims_for_height_conflict(
    ownership: &super::ownership::NodeBooleanOwnership,
    diagnostic: &super::validation::NodeGeometryDiagnostic,
) -> Vec<String> {
    let NodeGeometryDiagnosticKind::CrossRegionHeightConflict {
        existing_region_index,
        incoming_region_index,
        ..
    } = diagnostic.kind
    else {
        return Vec::new();
    };
    [existing_region_index, incoming_region_index]
        .into_iter()
        .filter_map(|region_index| {
            ownership.owned_regions.get(region_index).map(|region| {
                format!(
                    "region={} kind={:?} owner={:?} claim={:?} source_mouth={} source_band={:?} area={:.6}",
                    region_index,
                    region.kind,
                    region.owner,
                    region.claim_priority,
                    region.source_mouth_order_index,
                    region.source_band_index,
                    region.area_m2
                )
            })
        })
        .collect()
}

fn arrangement_key_from_overlay_keys(x_key: i64, z_key: i64) -> NodeArrangementKey {
    NodeArrangementKey::from_point(super::backend::RoadVec2::new(
        x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
        z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
    ))
}

fn source_rail_debug_for_height_conflict(
    input: &super::input::NodeArrangementInput,
    constraint: Option<&super::rails::NodeRailConstraint>,
) -> String {
    let Some(constraint) = constraint else {
        return "rail_constraint=<missing>".to_string();
    };
    let mut parts = vec![format!("rail_constraint={constraint:?}")];
    let Some(boundary_index) = constraint.source_boundary_index else {
        return parts.join(" ");
    };
    let Some(mouth) = input
        .mouths
        .iter()
        .find(|mouth| mouth.order_index == constraint.source_mouth_order_index)
    else {
        parts.push("mouth=<missing>".to_string());
        return parts.join(" ");
    };
    if let Some(boundary_rail) = mouth.boundary_rails.get(boundary_index) {
        parts.push(format!(
            "boundary_path={}",
            world_path_debug(&boundary_rail.path_world)
        ));
    }
    if let Some(left_band) = boundary_index
        .checked_sub(1)
        .and_then(|index| mouth.band_intervals.get(index))
    {
        parts.push(format!(
            "left_band={:?} start_path={} end_path={}",
            left_band.band_kind,
            world_path_debug(&left_band.start_path_world),
            world_path_debug(&left_band.end_path_world)
        ));
    }
    if let Some(right_band) = mouth.band_intervals.get(boundary_index) {
        parts.push(format!(
            "right_band={:?} start_path={} end_path={}",
            right_band.band_kind,
            world_path_debug(&right_band.start_path_world),
            world_path_debug(&right_band.end_path_world)
        ));
    }
    parts.join(" ")
}

fn world_path_debug(path: &[super::backend::RoadVec3]) -> String {
    let points = path
        .iter()
        .map(|point| format!("({:.3},{:.3},{:.3})", point.x, point.y, point.z))
        .collect::<Vec<_>>();
    format!("[{}]", points.join(","))
}

fn height_solution_vertices_at_arrangement_key(
    heights: &super::height::NodeHeightSolution,
    key: NodeArrangementKey,
) -> Vec<String> {
    let mut matches = Vec::new();
    for (region_index, region) in heights.regions.iter().enumerate() {
        for vertex in region.shape.iter().flat_map(|contour| contour.iter()) {
            if NodeArrangementKey::from_point(vertex.point_xz) != key {
                continue;
            }
            let touching_seams = region
                .seam_constraints
                .iter()
                .filter(|constraint| {
                    let start = NodeArrangementKey::from_point(constraint.start_xz);
                    let end = NodeArrangementKey::from_point(constraint.end_xz);
                    start == key || end == key
                })
                .map(|constraint| {
                    format!(
                        "#{} {:?} owner={:?} opposite={:?} shared={} material={}",
                        constraint.constraint_index,
                        constraint.seam_source,
                        constraint.owner,
                        constraint.opposite_owner,
                        constraint.constrains_shared_height,
                        constraint.is_material_transition
                    )
                })
                .collect::<Vec<_>>();
            matches.push(format!(
                "region={} kind={:?} owner={:?} field={:?} height={:.3} seams={:?}",
                region_index,
                region.kind,
                region.owner,
                vertex.height_field_id,
                vertex.height_m,
                touching_seams
            ));
        }
    }
    matches
}

fn assert_outer_boundary_vertices_match_visible_top(piece: &RoadSurfaceVisualNodePiece) {
    let top_polygons = piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .collect::<Vec<_>>();
    let top_vertices = visible_top_vertices(piece);
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
        let mut sampled_visible_top = false;
        let mut sampled_matching_height = false;
        for polygon in &top_polygons {
            for &triangle in &polygon.triangles_world {
                let Some((wa, wb, wc)) = RoadSurfaceSystem::triangle_barycentric_weights_xz(
                    triangle,
                    Vector2::new(boundary_point.x, boundary_point.z),
                ) else {
                    continue;
                };
                sampled_visible_top = true;
                let height = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                if (height - boundary_point.y).abs() <= overlay_match_tolerance_m {
                    sampled_matching_height = true;
                    break;
                }
            }
            if sampled_matching_height {
                break;
            }
        }
        if sampled_visible_top {
            assert!(
                sampled_matching_height,
                "node outer boundary must use a visible top-surface height at covered boundary points; boundary={boundary_point:?}"
            );
            continue;
        }

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

fn assert_outer_boundary_vertices_use_visible_top_boundary_support(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let top_polygons = piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .collect::<Vec<_>>();
    for boundary_point in piece
        .outer_boundary_loops
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
    {
        let Some(closest) = top_polygons
            .iter()
            .flat_map(|polygon| {
                polygon
                    .points_world
                    .windows(2)
                    .map(|segment| {
                        closest_point_on_segment_xz(*boundary_point, segment[0], segment[1])
                    })
                    .chain((!polygon.points_world.is_empty()).then(|| {
                        let last = *polygon.points_world.last().unwrap();
                        closest_point_on_segment_xz(*boundary_point, last, polygon.points_world[0])
                    }))
                    .chain(polygon.triangles_world.iter().flat_map(|triangle| {
                        (0..3).map(|index| {
                            closest_point_on_segment_xz(
                                *boundary_point,
                                triangle[index],
                                triangle[(index + 1) % 3],
                            )
                        })
                    }))
            })
            .min_by(|a, b| {
                let da =
                    Vector2::new(a.x - boundary_point.x, a.z - boundary_point.z).length_squared();
                let db =
                    Vector2::new(b.x - boundary_point.x, b.z - boundary_point.z).length_squared();
                da.total_cmp(&db).then(
                    (a.y - boundary_point.y)
                        .abs()
                        .total_cmp(&(b.y - boundary_point.y).abs()),
                )
            })
        else {
            panic!("node piece emitted no top boundary support");
        };
        let xz_error =
            Vector2::new(closest.x - boundary_point.x, closest.z - boundary_point.z).length();
        let y_error = (closest.y - boundary_point.y).abs();
        assert!(
            xz_error <= SAMPLE_EPSILON_M * 2.0 && y_error <= SAMPLE_EPSILON_M * 2.0,
            "node outer boundary vertices must lie on canonical visible top boundary support; boundary={boundary_point:?} closest={closest:?} xz_error={xz_error:.4} y_error={y_error:.4}"
        );
    }
}

fn closest_point_on_segment_xz(point: Vector3, start: Vector3, end: Vector3) -> Vector3 {
    let segment = Vector2::new(end.x - start.x, end.z - start.z);
    let len_squared = segment.length_squared();
    if len_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
        return start;
    }
    let to_point = Vector2::new(point.x - start.x, point.z - start.z);
    let t = (to_point.dot(segment) / len_squared).clamp(0.0, 1.0);
    start.lerp(end, t)
}

fn visible_top_vertices(piece: &RoadSurfaceVisualNodePiece) -> Vec<Vector3> {
    piece
        .road_surface_polygons
        .iter()
        .chain(piece.curb_surface_polygons.iter())
        .chain(piece.sidewalk_surface_polygons.iter())
        .flat_map(|polygon| {
            polygon.points_world.iter().copied().chain(
                polygon
                    .triangles_world
                    .iter()
                    .flat_map(|triangle| triangle.iter().copied()),
            )
        })
        .collect()
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

fn assert_junction_mouth_section_profile_laterally_flat(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    edge_idx: usize,
    at_start: bool,
) {
    let sections = surface
        .compiled_sections()
        .get(&edge_idx)
        .unwrap_or_else(|| panic!("edge {edge_idx} must have compiled sections"));
    let section = if at_start {
        sections
            .iter()
            .min_by(|a, b| a.s_m.total_cmp(&b.s_m))
            .unwrap()
    } else {
        sections
            .iter()
            .max_by(|a, b| a.s_m.total_cmp(&b.s_m))
            .unwrap()
    };
    let edge = graph.edge(edge_idx);
    let tolerance_m = 0.005;
    let mut carriageway_count = 0;
    for band in section
        .bands
        .iter()
        .filter(|band| band.kind == RoadSurfaceBandKind::Carriageway)
    {
        carriageway_count += 1;
        for height_m in [band.height_start_m, band.height_end_m] {
            assert!(
                (height_m - section.center_height_m).abs() <= tolerance_m,
                "JunctionN mouth carriageway must be laterally flat: edge={edge_idx} at_start={at_start} s_m={:.3} height={height_m:.3} center={:.3} delta={:.3}",
                section.s_m,
                section.center_height_m,
                height_m - section.center_height_m
            );
        }
    }
    assert!(
        carriageway_count > 0,
        "edge {edge_idx} must expose carriageway bands at the JunctionN mouth"
    );

    let expected_non_road_height_m = section.center_height_m + CURB_STEP_HEIGHT_M;
    for band in section.bands.iter().filter(|band| {
        band.kind == RoadSurfaceBandKind::CurbOrShoulder
            || band.kind == RoadSurfaceBandKind::Sidewalk
    }) {
        for height_m in [band.height_start_m, band.height_end_m] {
            assert!(
                (height_m - expected_non_road_height_m).abs() <= tolerance_m,
                "JunctionN mouth curb/sidewalk must use the explicit curb step from the road height: edge={edge_idx} at_start={at_start} width={:.3} s_m={:.3} kind={:?} height={height_m:.3} expected={expected_non_road_height_m:.3} delta={:.3}",
                edge.width,
                section.s_m,
                band.kind,
                height_m - expected_non_road_height_m
            );
        }
    }
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
            .expect("short right-angle bend should compile through raw corridor ownership")
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
    assert_node_earthwork_faces_have_footprint_provenance(terminal_piece);
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

    assert_compiled_bend_piece(&surface, &graph, bend);
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

    assert_compiled_bend_piece(&surface, &graph, bend);
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

    assert_compiled_bend_piece(&surface, &graph, bend);
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

    assert_compiled_bend_piece(&surface, &graph, bend);
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

    assert_compiled_bend_piece(&surface, &graph, bend);
}

#[test]
fn logged_outer_bend_skips_one_sided_curb_step_slivers() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-116.890, 0.0, -31.104), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-53.167, 0.0, -27.526), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(-17.253, 0.0, 19.023), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        road_points_from_json(
            "[[-116.89,0.0,-31.104],[-116.174,0.0,-31.064],[-115.314,0.0,-31.015],[-114.769,0.0,-30.985],[-114.152,0.0,-30.95],[-113.464,0.0,-30.912],[-112.709,0.0,-30.869],[-111.889,0.0,-30.823],[-111.009,0.0,-30.774],[-110.07,0.0,-30.721],[-109.33,0.0,-30.679],[-108.819,0.0,-30.651],[-108.296,0.0,-30.621],[-107.76,0.0,-30.591],[-107.211,0.0,-30.561],[-106.651,0.0,-30.529],[-106.08,0.0,-30.497],[-105.497,0.0,-30.464],[-104.904,0.0,-30.431],[-104.3,0.0,-30.397],[-103.686,0.0,-30.363],[-103.063,0.0,-30.328],[-102.43,0.0,-30.292],[-101.788,0.0,-30.256],[-101.138,0.0,-30.22],[-100.479,0.0,-30.183],[-99.813,0.0,-30.145],[-99.139,0.0,-30.107],[-98.458,0.0,-30.069],[-97.771,0.0,-30.03],[-97.077,0.0,-29.991],[-96.377,0.0,-29.952],[-95.671,0.0,-29.913],[-94.96,0.0,-29.873],[-94.244,0.0,-29.832],[-93.523,0.0,-29.792],[-92.799,0.0,-29.751],[-92.07,0.0,-29.71],[-91.338,0.0,-29.669],[-90.603,0.0,-29.628],[-89.865,0.0,-29.587],[-89.125,0.0,-29.545],[-88.383,0.0,-29.503],[-87.639,0.0,-29.462],[-86.894,0.0,-29.42],[-86.148,0.0,-29.378],[-85.402,0.0,-29.336],[-84.655,0.0,-29.294],[-83.908,0.0,-29.252],[-83.162,0.0,-29.21],[-82.417,0.0,-29.168],[-81.673,0.0,-29.127],[-80.931,0.0,-29.085],[-80.191,0.0,-29.043],[-79.453,0.0,-29.002],[-78.718,0.0,-28.961],[-77.986,0.0,-28.92],[-77.258,0.0,-28.879],[-76.533,0.0,-28.838],[-75.813,0.0,-28.798],[-75.097,0.0,-28.757],[-74.386,0.0,-28.718],[-73.68,0.0,-28.678],[-72.98,0.0,-28.639],[-72.286,0.0,-28.6],[-71.598,0.0,-28.561],[-70.917,0.0,-28.523],[-70.243,0.0,-28.485],[-69.577,0.0,-28.448],[-68.919,0.0,-28.411],[-68.268,0.0,-28.374],[-67.627,0.0,-28.338],[-66.994,0.0,-28.302],[-66.37,0.0,-28.267],[-65.756,0.0,-28.233],[-65.153,0.0,-28.199],[-64.559,0.0,-28.166],[-63.977,0.0,-28.133],[-63.405,0.0,-28.101],[-62.845,0.0,-28.07],[-62.297,0.0,-28.039],[-61.761,0.0,-28.009],[-61.237,0.0,-27.979],[-60.727,0.0,-27.951],[-59.986,0.0,-27.909],[-59.047,0.0,-27.856],[-58.167,0.0,-27.807],[-57.348,0.0,-27.761],[-56.593,0.0,-27.719],[-55.905,0.0,-27.68],[-55.287,0.0,-27.645],[-54.742,0.0,-27.615],[-53.882,0.0,-27.566],[-53.167,0.0,-27.526]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        road_points_from_json(
            "[[-53.167,0.0,-27.526],[-52.763,0.0,-27.003],[-52.279,0.0,-26.376],[-51.972,0.0,-25.977],[-51.624,0.0,-25.526],[-51.236,0.0,-25.023],[-50.81,0.0,-24.472],[-50.349,0.0,-23.874],[-49.853,0.0,-23.23],[-49.323,0.0,-22.545],[-48.763,0.0,-21.818],[-48.173,0.0,-21.054],[-47.868,0.0,-20.658],[-47.555,0.0,-20.253],[-47.236,0.0,-19.839],[-46.911,0.0,-19.418],[-46.58,0.0,-18.988],[-46.242,0.0,-18.551],[-45.899,0.0,-18.106],[-45.55,0.0,-17.654],[-45.196,0.0,-17.195],[-44.837,0.0,-16.73],[-44.473,0.0,-16.258],[-44.104,0.0,-15.78],[-43.731,0.0,-15.296],[-43.353,0.0,-14.806],[-42.971,0.0,-14.311],[-42.586,0.0,-13.812],[-42.196,0.0,-13.307],[-41.803,0.0,-12.798],[-41.407,0.0,-12.284],[-41.008,0.0,-11.767],[-40.606,0.0,-11.245],[-40.201,0.0,-10.721],[-39.794,0.0,-10.193],[-39.384,0.0,-9.662],[-38.973,0.0,-9.129],[-38.559,0.0,-8.593],[-38.144,0.0,-8.055],[-37.728,0.0,-7.515],[-37.31,0.0,-6.973],[-36.891,0.0,-6.431],[-36.472,0.0,-5.887],[-36.051,0.0,-5.342],[-35.631,0.0,-4.797],[-35.21,0.0,-4.251],[-34.789,0.0,-3.706],[-34.368,0.0,-3.161],[-33.948,0.0,-2.616],[-33.529,0.0,-2.072],[-33.11,0.0,-1.529],[-32.692,0.0,-0.988],[-32.276,0.0,-0.448],[-31.861,0.0,0.09],[-31.447,0.0,0.626],[-31.036,0.0,1.159],[-30.626,0.0,1.69],[-30.219,0.0,2.218],[-29.814,0.0,2.743],[-29.412,0.0,3.264],[-29.013,0.0,3.781],[-28.616,0.0,4.295],[-28.223,0.0,4.804],[-27.834,0.0,5.309],[-27.448,0.0,5.809],[-27.067,0.0,6.303],[-26.689,0.0,6.793],[-26.316,0.0,7.277],[-25.947,0.0,7.755],[-25.583,0.0,8.227],[-25.223,0.0,8.693],[-24.869,0.0,9.151],[-24.521,0.0,9.603],[-24.178,0.0,10.048],[-23.84,0.0,10.485],[-23.509,0.0,10.915],[-23.183,0.0,11.337],[-22.865,0.0,11.75],[-22.552,0.0,12.155],[-22.247,0.0,12.551],[-21.657,0.0,13.315],[-21.096,0.0,14.042],[-20.567,0.0,14.728],[-20.071,0.0,15.371],[-19.609,0.0,15.969],[-19.184,0.0,16.521],[-18.796,0.0,17.023],[-18.448,0.0,17.475],[-18.141,0.0,17.873],[-17.656,0.0,18.501],[-17.253,0.0,19.023]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let bend_piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .expect("bend should compile through canonical owned regions");
    assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert!(
        visual_polygon_boundary_contains_xz(
            &bend_piece.outer_boundary_loops,
            Vector2::new(-53.814, -20.179),
        ),
        "outer bend terrain cutter must preserve sampled outer span rail points; outer_loops={:?}",
        bend_piece.outer_boundary_loops
    );
}

#[test]
fn logged_curved_terminal_exports_outer_boundary_from_visible_top_support() {
    let terrain = flat_terrain(384, 384);
    let points = road_points_from_json(
        "[[-26.262,0.000,-35.164],[-25.870,0.000,-34.826],[-25.195,0.000,-34.246],[-24.743,0.000,-33.856],[-24.217,0.000,-33.404],[-23.622,0.000,-32.890],[-22.958,0.000,-32.319],[-22.230,0.000,-31.692],[-21.843,0.000,-31.359],[-21.440,0.000,-31.012],[-21.023,0.000,-30.653],[-20.591,0.000,-30.281],[-20.145,0.000,-29.897],[-19.686,0.000,-29.501],[-19.213,0.000,-29.094],[-18.727,0.000,-28.676],[-18.229,0.000,-28.246],[-17.718,0.000,-27.806],[-17.195,0.000,-27.356],[-16.661,0.000,-26.896],[-16.115,0.000,-26.426],[-15.558,0.000,-25.947],[-14.991,0.000,-25.458],[-14.414,0.000,-24.961],[-13.827,0.000,-24.456],[-13.230,0.000,-23.942],[-12.624,0.000,-23.420],[-12.010,0.000,-22.891],[-11.387,0.000,-22.354],[-10.756,0.000,-21.811],[-10.117,0.000,-21.261],[-9.471,0.000,-20.704],[-8.818,0.000,-20.142],[-8.158,0.000,-19.574],[-7.491,0.000,-19.000],[-6.819,0.000,-18.421],[-6.141,0.000,-17.837],[-5.458,0.000,-17.249],[-4.770,0.000,-16.656],[-4.077,0.000,-16.060],[-3.381,0.000,-15.460],[-2.680,0.000,-14.856],[-1.976,0.000,-14.250],[-1.268,0.000,-13.641],[-0.558,0.000,-13.029],[0.155,0.000,-12.416],[0.869,0.000,-11.800],[1.586,0.000,-11.183],[2.304,0.000,-10.565],[3.023,0.000,-9.946],[3.743,0.000,-9.326],[4.463,0.000,-8.706],[5.183,0.000,-8.086],[5.902,0.000,-7.466],[6.621,0.000,-6.847],[7.339,0.000,-6.228],[8.056,0.000,-5.611],[8.771,0.000,-4.996],[9.483,0.000,-4.382],[10.193,0.000,-3.771],[10.901,0.000,-3.161],[11.605,0.000,-2.555],[12.306,0.000,-1.952],[13.003,0.000,-1.351],[13.695,0.000,-0.755],[14.383,0.000,-0.162],[15.066,0.000,0.426],[15.744,0.000,1.010],[16.416,0.000,1.588],[17.083,0.000,2.162],[17.743,0.000,2.730],[18.396,0.000,3.293],[19.042,0.000,3.849],[19.681,0.000,4.400],[20.312,0.000,4.943],[20.935,0.000,5.480],[21.550,0.000,6.009],[22.155,0.000,6.530],[22.752,0.000,7.044],[23.339,0.000,7.550],[23.916,0.000,8.047],[24.483,0.000,8.535],[25.040,0.000,9.015],[25.586,0.000,9.485],[26.120,0.000,9.945],[26.643,0.000,10.395],[27.154,0.000,10.835],[27.652,0.000,11.264],[28.138,0.000,11.683],[28.611,0.000,12.090],[29.070,0.000,12.485],[29.516,0.000,12.869],[29.948,0.000,13.241],[30.365,0.000,13.601],[30.768,0.000,13.947],[31.155,0.000,14.281],[31.883,0.000,14.908],[32.547,0.000,15.479],[33.143,0.000,15.992],[33.668,0.000,16.445],[34.121,0.000,16.834],[34.795,0.000,17.415],[35.187,0.000,17.753]]",
    );
    let mut graph = RegionGraph::new();
    let start = graph.add_node(points[0], NodeType::Junction);
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
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let terminal_piece = surface
        .compiled_visual_node_pieces()
        .get(&end)
        .expect("logged curved terminal should compile");
    assert_eq!(
        terminal_piece.kind,
        RoadSurfaceVisualNodePieceKind::Terminal
    );
    assert_outer_boundary_vertices_match_visible_top(terminal_piece);
    assert_outer_boundary_vertices_use_visible_top_boundary_support(terminal_piece);
}

#[test]
fn logged_current_bend_keeps_curved_inner_asphalt_curb_steps() {
    let terrain = flat_terrain(512, 512);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-191.431, 0.0, -105.786), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-118.080, 0.0, -99.065), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(-70.293, 0.0, -45.373), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        road_points_from_json(
            "[[-191.431,0.0,-105.786],[-190.608,0.0,-105.711],[-189.899,0.0,-105.646],[-189.315,0.0,-105.592],[-188.646,0.0,-105.531],[-187.894,0.0,-105.462],[-187.063,0.0,-105.386],[-186.156,0.0,-105.303],[-185.429,0.0,-105.236],[-184.922,0.0,-105.190],[-184.398,0.0,-105.142],[-183.858,0.0,-105.092],[-183.301,0.0,-105.041],[-182.729,0.0,-104.989],[-182.141,0.0,-104.935],[-181.539,0.0,-104.880],[-180.922,0.0,-104.823],[-180.291,0.0,-104.766],[-179.646,0.0,-104.706],[-178.988,0.0,-104.646],[-178.318,0.0,-104.585],[-177.635,0.0,-104.522],[-176.940,0.0,-104.458],[-176.233,0.0,-104.394],[-175.515,0.0,-104.328],[-174.787,0.0,-104.261],[-174.048,0.0,-104.194],[-173.300,0.0,-104.125],[-172.542,0.0,-104.055],[-171.775,0.0,-103.985],[-170.999,0.0,-103.914],[-170.215,0.0,-103.842],[-169.424,0.0,-103.770],[-168.625,0.0,-103.697],[-167.819,0.0,-103.623],[-167.007,0.0,-103.548],[-166.188,0.0,-103.473],[-165.364,0.0,-103.398],[-164.535,0.0,-103.322],[-163.701,0.0,-103.245],[-162.862,0.0,-103.169],[-162.019,0.0,-103.091],[-161.173,0.0,-103.014],[-160.324,0.0,-102.936],[-159.472,0.0,-102.858],[-158.618,0.0,-102.780],[-157.761,0.0,-102.701],[-156.904,0.0,-102.623],[-156.045,0.0,-102.544],[-155.186,0.0,-102.465],[-154.326,0.0,-102.386],[-153.467,0.0,-102.308],[-152.608,0.0,-102.229],[-151.750,0.0,-102.150],[-150.894,0.0,-102.072],[-150.040,0.0,-101.994],[-149.188,0.0,-101.916],[-148.339,0.0,-101.838],[-147.492,0.0,-101.760],[-146.650,0.0,-101.683],[-145.811,0.0,-101.606],[-144.977,0.0,-101.530],[-144.148,0.0,-101.454],[-143.324,0.0,-101.378],[-142.505,0.0,-101.303],[-141.693,0.0,-101.229],[-140.887,0.0,-101.155],[-140.088,0.0,-101.082],[-139.297,0.0,-101.009],[-138.513,0.0,-100.937],[-137.737,0.0,-100.866],[-136.970,0.0,-100.796],[-136.212,0.0,-100.727],[-135.464,0.0,-100.658],[-134.725,0.0,-100.590],[-133.996,0.0,-100.524],[-133.279,0.0,-100.458],[-132.572,0.0,-100.393],[-131.877,0.0,-100.329],[-131.194,0.0,-100.267],[-130.523,0.0,-100.205],[-129.865,0.0,-100.145],[-129.221,0.0,-100.086],[-128.590,0.0,-100.028],[-127.973,0.0,-99.972],[-127.370,0.0,-99.917],[-126.783,0.0,-99.863],[-126.210,0.0,-99.810],[-125.654,0.0,-99.759],[-125.114,0.0,-99.710],[-124.590,0.0,-99.662],[-124.083,0.0,-99.615],[-123.356,0.0,-99.549],[-122.449,0.0,-99.466],[-121.618,0.0,-99.389],[-120.866,0.0,-99.321],[-120.197,0.0,-99.259],[-119.612,0.0,-99.206],[-118.904,0.0,-99.141],[-118.080,0.0,-99.065]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        road_points_from_json(
            "[[-118.080,0.0,-99.065],[-117.544,0.0,-98.462],[-117.082,0.0,-97.944],[-116.702,0.0,-97.516],[-116.265,0.0,-97.026],[-115.775,0.0,-96.476],[-115.234,0.0,-95.867],[-114.644,0.0,-95.204],[-114.006,0.0,-94.487],[-113.670,0.0,-94.110],[-113.324,0.0,-93.721],[-112.966,0.0,-93.319],[-112.599,0.0,-92.906],[-112.221,0.0,-92.482],[-111.833,0.0,-92.046],[-111.436,0.0,-91.600],[-111.029,0.0,-91.143],[-110.614,0.0,-90.676],[-110.189,0.0,-90.199],[-109.756,0.0,-89.713],[-109.315,0.0,-89.217],[-108.866,0.0,-88.713],[-108.410,0.0,-88.200],[-107.946,0.0,-87.678],[-107.475,0.0,-87.149],[-106.997,0.0,-86.612],[-106.512,0.0,-86.068],[-106.022,0.0,-85.516],[-105.525,0.0,-84.958],[-105.022,0.0,-84.394],[-104.514,0.0,-83.823],[-104.001,0.0,-83.246],[-103.483,0.0,-82.664],[-102.960,0.0,-82.077],[-102.433,0.0,-81.484],[-101.902,0.0,-80.887],[-101.367,0.0,-80.286],[-100.828,0.0,-79.681],[-100.286,0.0,-79.072],[-99.741,0.0,-78.460],[-99.193,0.0,-77.845],[-98.643,0.0,-77.226],[-98.091,0.0,-76.606],[-97.537,0.0,-75.983],[-96.981,0.0,-75.359],[-96.424,0.0,-74.733],[-95.865,0.0,-74.105],[-95.306,0.0,-73.477],[-94.746,0.0,-72.848],[-94.187,0.0,-72.219],[-93.627,0.0,-71.590],[-93.067,0.0,-70.961],[-92.508,0.0,-70.333],[-91.949,0.0,-69.705],[-91.392,0.0,-69.079],[-90.836,0.0,-68.455],[-90.282,0.0,-67.832],[-89.730,0.0,-67.211],[-89.180,0.0,-66.593],[-88.632,0.0,-65.978],[-88.087,0.0,-65.366],[-87.545,0.0,-64.757],[-87.006,0.0,-64.152],[-86.471,0.0,-63.551],[-85.940,0.0,-62.954],[-85.413,0.0,-62.361],[-84.890,0.0,-61.774],[-84.372,0.0,-61.192],[-83.859,0.0,-60.615],[-83.351,0.0,-60.044],[-82.848,0.0,-59.480],[-82.352,0.0,-58.922],[-81.861,0.0,-58.370],[-81.376,0.0,-57.826],[-80.898,0.0,-57.289],[-80.427,0.0,-56.759],[-79.963,0.0,-56.238],[-79.507,0.0,-55.725],[-79.058,0.0,-55.221],[-78.617,0.0,-54.725],[-78.184,0.0,-54.239],[-77.759,0.0,-53.762],[-77.344,0.0,-53.295],[-76.937,0.0,-52.838],[-76.540,0.0,-52.392],[-76.152,0.0,-51.956],[-75.775,0.0,-51.532],[-75.407,0.0,-51.119],[-75.049,0.0,-50.717],[-74.703,0.0,-50.328],[-74.367,0.0,-49.950],[-73.729,0.0,-49.234],[-73.139,0.0,-48.571],[-72.598,0.0,-47.962],[-72.108,0.0,-47.412],[-71.671,0.0,-46.922],[-71.291,0.0,-46.494],[-70.829,0.0,-45.976],[-70.293,0.0,-45.373]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let bend_piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .expect("bend should compile through canonical owned regions");
    assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_top_covers_footprint(bend_piece);
    assert_top_raised_step_owner_boundaries_have_vertical_faces(bend_piece);
    assert_canonical_explicit_vertical_steps_have_faces(bend_piece);
    assert_earthwork_faces_stay_outside_top_footprint(bend_piece);
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
    assert_compiled_bend_piece(&surface, &graph, bend);
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
    assert_compiled_bend_piece(&surface, &graph, bend);
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

        let sidewalk_corner = center - travel * 0.075 + lateral * side * 4.325;
        assert!(
            point_inside_visual_polygons(
                &terminal_piece.sidewalk_surface_polygons,
                sidewalk_corner
            ),
            "terminal sidewalk must close the endpoint-to-cap curb-depth corner on side {side}; point={sidewalk_corner:?}"
        );
        assert!(
            !point_inside_visual_polygons(&terminal_piece.curb_surface_polygons, sidewalk_corner),
            "terminal sidewalk corner closure must not be owned by curb on side {side}; point={sidewalk_corner:?}"
        );
        assert!(
            !point_inside_visual_polygons(&terminal_piece.road_surface_polygons, sidewalk_corner),
            "terminal sidewalk corner closure must not be owned by asphalt on side {side}; point={sidewalk_corner:?}"
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

        let sidewalk_corner = end_center - end_travel * 0.075 + end_lateral * side * 4.325;
        assert!(
            point_inside_visual_polygons(
                &end_terminal_piece.sidewalk_surface_polygons,
                sidewalk_corner
            ),
            "opposite terminal sidewalk must close the endpoint-to-cap curb-depth corner on side {side}; point={sidewalk_corner:?}"
        );
        assert!(
            !point_inside_visual_polygons(
                &end_terminal_piece.curb_surface_polygons,
                sidewalk_corner
            ),
            "opposite terminal sidewalk corner closure must not be owned by curb on side {side}; point={sidewalk_corner:?}"
        );
        assert!(
            !point_inside_visual_polygons(
                &end_terminal_piece.road_surface_polygons,
                sidewalk_corner
            ),
            "opposite terminal sidewalk corner closure must not be owned by asphalt on side {side}; point={sidewalk_corner:?}"
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
fn logged_terminal_with_tiny_boundary_dust_exports_final_top_footprint() {
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

    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[0]);
    for node_id in [start, end] {
        let piece = surface
            .compiled_visual_node_pieces()
            .get(&node_id)
            .unwrap_or_else(|| {
                panic!(
                    "tiny boundary dust should not survive into final top footprint; node_id={node_id} report={} dump={dump}",
                    canonical_node_pipeline_report(
                        &surface,
                        &graph,
                        node_id,
                        RoadSurfaceVisualNodePieceKind::Terminal,
                    )
                )
            });
        assert_node_top_covers_footprint(piece);
    }
    assert!(
        dump.contains("\"missing_source_count\":0")
            && dump.contains("\"boundary_interpolation_source_count\":0"),
        "tiny boundary dust must be absent from final top-owned footprint export, not repaired by boundary interpolation; dump={dump}"
    );
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
    assert_raised_step_face_lower_edge_covers(
        &start_terminal.raised_step_face_polygons,
        start_endpoint.boundary_points_world[2],
        start_mouth.boundary_points_world[2],
        "left longitudinal raised-step face at logged terminal handoff",
    );
    assert_raised_step_face_lower_edge_covers(
        &start_terminal.raised_step_face_polygons,
        start_endpoint.boundary_points_world[4],
        start_mouth.boundary_points_world[4],
        "right longitudinal raised-step face at logged terminal handoff",
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
    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[0]);
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&start),
        "steep terminal should compile with explicit terminal cap height ownership; dump={dump}"
    );
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&end),
        "opposite steep terminal should compile with explicit terminal cap height ownership; dump={dump}"
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
    assert!(!span_piece.raised_step_face_polygons.is_empty());
    assert!(!span_piece.sidewalk_surface_polygons.is_empty());
    assert!(!span_piece.span_owned_regions.is_empty());
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::Asphalt)
            .count(),
        span_piece.road_surface_polygons.len()
    );
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::CurbOrShoulder)
            .count(),
        span_piece.curb_surface_polygons.len()
    );
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::NonRoad)
            .count(),
        span_piece.sidewalk_surface_polygons.len()
    );
    assert!(
        span_piece.span_owned_regions.iter().all(|region| {
            region.edge_idx == edge_idx
                && region.end_section_index == region.start_section_index + 1
                && region.end_s_m > region.start_s_m
        }),
        "span owned regions must preserve edge, section interval, and solved section authority"
    );
    assert!(!span_piece.span_earthwork_support_regions.is_empty());
    assert_eq!(
        span_piece.span_earthwork_support_regions.len(),
        span_piece.span_owned_regions.len(),
        "grounded standard span support regions should cover the same solved band-owned footprint as the visible span"
    );
    for role in [
        RoadSurfaceSpanRegionRole::Asphalt,
        RoadSurfaceSpanRegionRole::CurbOrShoulder,
        RoadSurfaceSpanRegionRole::NonRoad,
    ] {
        assert!(
            span_piece
                .span_earthwork_support_regions
                .iter()
                .any(|region| region.role == role),
            "span earthwork support regions must retain role/material provenance for {role:?}"
        );
    }
    assert!(
        span_piece
            .span_earthwork_support_regions
            .iter()
            .all(|region| {
                region.edge_idx == edge_idx
                    && region.end_section_index == region.start_section_index + 1
                    && region.end_s_m > region.start_s_m
                    && RoadSurfaceSystem::polygon_has_area_xz(&region.polygon.points_world)
            }),
        "span earthwork support regions must preserve edge, section interval, source band, and top-surface geometry"
    );
    assert_eq!(
        span_piece.span_raised_step_sources.len(),
        span_piece.raised_step_face_polygons.len()
    );
    assert!(
        span_piece.span_raised_step_sources.iter().all(|source| {
            source.lower_owner.kind != source.raised_owner.kind
                && source.end_section_index == source.start_section_index + 1
                && source.end_s_m > source.start_s_m
                && source.start_raised_world.y > source.start_lower_world.y
                && source.end_raised_world.y > source.end_lower_world.y
        }),
        "span raised-step faces must carry owner-pair and solved section provenance"
    );
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
        "curb top surface must be flat; vertical drop belongs to explicit raised-step faces"
    );
    assert!(
        span_piece
            .raised_step_face_polygons
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
    assert_span_earthwork_faces_have_support_provenance(span_piece, edge_idx, EdgeClass::Standard);
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
    let earthwork_outer_points = span_piece
        .earthwork_outer_boundary_loops
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
        .copied()
        .collect::<Vec<_>>();
    let min_outer_footprint_distance_m = earthwork_outer_points
        .iter()
        .map(|outer_point| {
            span_piece
                .outer_boundary_loops
                .iter()
                .flat_map(|footprint| {
                    (0..footprint.points_world.len()).map(|index| {
                        let start = footprint.points_world[index];
                        let end =
                            footprint.points_world[(index + 1) % footprint.points_world.len()];
                        let start_xz = Vector2::new(start.x, start.z);
                        let end_xz = Vector2::new(end.x, end.z);
                        let point_xz = Vector2::new(outer_point.x, outer_point.z);
                        let segment = end_xz - start_xz;
                        if segment.length_squared() <= SAMPLE_EPSILON_M {
                            point_xz.distance_to(start_xz)
                        } else {
                            let t = ((point_xz - start_xz).dot(segment) / segment.length_squared())
                                .clamp(0.0, 1.0);
                            point_xz.distance_to(start_xz + segment * t)
                        }
                    })
                })
                .fold(f32::INFINITY, f32::min)
        })
        .fold(f32::INFINITY, f32::min);
    assert!(
        earthwork_outer_points.iter().all(|outer_point| {
            let point_xz = Vector2::new(outer_point.x, outer_point.z);
            span_piece.outer_boundary_loops.iter().all(|footprint| {
                !RoadSurfaceSystem::polygon_contains_point_xz(&footprint.points_world, point_xz)
            })
        }) && min_outer_footprint_distance_m >= 0.5,
        "expected span earthwork tie-in to stay outside the paved footprint, got min_outer_footprint_distance_m={min_outer_footprint_distance_m:.3}"
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

    assert_eq!(
        surface_a
            .compiled_visual_node_pieces()
            .get(&center)
            .expect("flat multi-arm node should compile through raw corridor ownership")
            .kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
    assert_eq!(
        surface_a.compiled_visual_node_pieces().get(&center),
        surface_b.compiled_visual_node_pieces().get(&center)
    );
}

#[test]
fn oblique_t_junction_compiles_with_canonical_side_join_ownership() {
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

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn editor_sized_60_degree_t_junction_width_7_compiles_side_join_ownership() {
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

    assert_compiled_junction_piece(&surface, &graph, center);

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
        RoadSurfaceSystem::union_terrain_clip_boundary_export(&raw_clip_sources)
            .expect("editor-sized 60-degree T junction clip union should be source-complete");
    assert!(
        !unioned_clip_sources.loops.is_empty(),
        "editor-sized 60-degree T junction raw clip loops must survive deterministic union"
    );

    let (road_loops, _) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -128.0, -32.0, 64.0, 64.0)
        .expect("editor-sized 60-degree T junction clip export should be source-complete");
    assert!(
        !road_loops.is_empty(),
        "editor-sized 60-degree T junction must export terrain clip loops"
    );
    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        TerrainCdtPatch::new(-128.0, -32.0, 64.0, 64.0, [0.0; 4]),
        road_loops,
        Vec::new(),
    ))
    .expect("editor-sized 60-degree T terrain cutters must be accepted by terrain CDT");
    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
}

#[test]
fn logged_flat_three_way_oblique_junction_compiles_side_join_ownership() {
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

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "logged flat three-way oblique JunctionN did not compile: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn logged_current_flat_three_way_oblique_junction_compiles_side_join_ownership() {
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

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn logged_flat_three_way_right_angle_junction_compiles_explicit_raised_steps() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-102.807, 0.0, -14.721), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-35.427, 0.0, -14.721), NodeType::Junction);
    let east = graph.add_node(Vector3::new(37.193, 0.0, -14.721), NodeType::Junction);
    let north = graph.add_node(Vector3::new(-35.427, 0.0, 35.279), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-102.807, 0.0, -14.721),
            Vector3::new(-35.427, 0.0, -14.721),
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
            Vector3::new(-35.427, 0.0, -14.721),
            Vector3::new(-35.427, 0.0, 35.279),
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
            Vector3::new(-35.427, 0.0, -14.721),
            Vector3::new(37.193, 0.0, -14.721),
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

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_canonical_explicit_vertical_steps_have_faces(piece);
}

#[test]
fn logged_flat_three_way_oblique_variant_compiles_with_explicit_vertical_steps() {
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

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_canonical_explicit_vertical_steps_have_faces(piece);
}

#[test]
fn logged_elevated_three_way_oblique_junction_rejects_same_material_height_conflict() {
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
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "logged elevated oblique JunctionN",
    );
}

#[test]
fn logged_current_elevated_oblique_three_way_compiles_with_endpoint_profile_solve() {
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

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "current elevated oblique 3-way JunctionN did not compile after endpoint profile solve: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
}

#[test]
fn logged_latest_elevated_oblique_three_way_compiles_with_endpoint_profile_solve() {
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

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "latest elevated oblique 3-way JunctionN did not compile after endpoint profile solve: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }

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
    if !network
        .road_surface
        .compiled_visual_node_pieces()
        .contains_key(&edit_center)
    {
        panic!(
            "add_road elevated oblique JunctionN did not compile after endpoint profile solve: {}",
            canonical_junction_pipeline_report(&network.road_surface, &edit_graph, edit_center)
        );
    }
}

#[test]
fn logged_regenerated_elevated_three_way_rejects_same_material_height_conflict() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let edge0_points = road_points_from_json(
        r#"[[-11.903,142.295,-17.011],[-12.021,142.386,-17.571],[-12.165,142.477,-18.25],[-12.29,142.566,-18.841],[-12.438,142.65,-19.539],[-12.607,142.724,-20.34],[-12.797,142.785,-21.238],[-12.953,142.832,-21.974],[-13.063,142.87,-22.493],[-13.177,142.902,-23.035],[-13.296,142.933,-23.598],[-13.42,142.967,-24.183],[-13.548,143.005,-24.788],[-13.68,143.046,-25.414],[-13.817,143.087,-26.06],[-13.958,143.129,-26.725],[-14.102,143.17,-27.409],[-14.251,143.216,-28.111],[-14.403,143.272,-28.83],[-14.559,143.344,-29.568],[-14.718,143.437,-30.322],[-14.881,143.553,-31.092],[-15.047,143.687,-31.878],[-15.217,143.835,-32.679],[-15.389,143.989,-33.495],[-15.565,144.145,-34.326],[-15.744,144.299,-35.17],[-15.925,144.452,-36.027],[-16.109,144.607,-36.898],[-16.296,144.77,-37.78],[-16.485,144.95,-38.675],[-16.676,145.151,-39.58],[-16.87,145.372,-40.497],[-17.066,145.606,-41.424],[-17.264,145.835,-42.36],[-17.464,146.045,-43.306],[-17.565,146.226,-43.782],[-17.666,146.377,-44.26],[-17.768,146.507,-44.741],[-17.87,146.627,-45.223],[-17.972,146.747,-45.707],[-18.075,146.871,-46.194],[-18.178,147.0,-46.682],[-18.282,147.131,-47.172],[-18.386,147.262,-47.663],[-18.49,147.393,-48.156],[-18.595,147.523,-48.651],[-18.7,147.656,-49.147],[-18.805,147.794,-49.645],[-18.911,147.939,-50.144],[-19.016,148.092,-50.644],[-19.122,148.251,-51.146],[-19.229,148.415,-51.648],[-19.335,148.581,-52.152],[-19.442,148.748,-52.657],[-19.549,148.915,-53.163],[-19.656,149.083,-53.67],[-19.764,149.251,-54.178],[-19.871,149.419,-54.687],[-19.979,149.586,-55.196],[-20.087,149.752,-55.706],[-20.195,149.918,-56.217],[-20.303,150.085,-56.728],[-20.411,150.255,-57.24],[-20.52,150.429,-57.752],[-20.628,150.605,-58.264],[-20.737,150.775,-58.777],[-20.845,150.93,-59.29],[-20.954,151.065,-59.804],[-21.062,151.178,-60.317],[-21.126,151.278,-60.618]]"#,
    );
    let edge1_points = road_points_from_json(
        r#"[[-21.126,151.278,-60.618],[-20.467,151.293,-60.757],[-19.675,151.303,-60.925],[-19.173,151.305,-61.031],[-18.603,151.298,-61.151],[-17.97,151.285,-61.285],[-17.274,151.268,-61.432],[-16.52,151.249,-61.592],[-15.708,151.23,-61.763],[-14.844,151.212,-61.946],[-13.928,151.196,-62.139],[-12.963,151.181,-62.343],[-12.464,151.169,-62.449],[-11.953,151.159,-62.556],[-11.432,151.151,-62.666],[-10.9,151.14,-62.779],[-10.359,151.126,-62.893],[-9.807,151.106,-63.009],[-9.246,151.08,-63.128],[-8.676,151.049,-63.248],[-8.097,151.013,-63.37],[-7.51,150.976,-63.494],[-6.915,150.937,-63.619],[-6.312,150.898,-63.747],[-5.702,150.858,-63.875],[-5.084,150.817,-64.006],[-4.46,150.775,-64.137],[-3.83,150.732,-64.27],[-3.193,150.688,-64.404],[-2.551,150.643,-64.54],[-1.903,150.596,-64.676],[-1.251,150.547,-64.814],[-0.593,150.496,-64.953],[0.068,150.442,-65.092],[0.734,150.384,-65.233],[1.404,150.323,-65.374],[2.076,150.259,-65.516],[2.752,150.195,-65.658],[3.431,150.129,-65.801],[4.112,150.064,-65.945],[4.795,149.997,-66.089],[5.479,149.93,-66.233],[6.165,149.862,-66.378],[6.852,149.792,-66.523],[7.54,149.722,-66.668],[8.228,149.651,-66.813],[8.916,149.582,-66.958],[9.603,149.514,-67.103],[10.29,149.45,-67.247],[10.976,149.39,-67.392],[11.661,149.332,-67.536],[12.344,149.276,-67.68],[13.024,149.22,-67.824],[13.703,149.164,-67.967],[14.379,149.108,-68.11],[15.052,149.052,-68.251],[15.721,148.995,-68.393],[16.387,148.937,-68.533],[17.049,148.879,-68.672],[17.706,148.82,-68.811],[18.359,148.762,-68.949],[19.006,148.708,-69.085],[19.649,148.659,-69.221],[20.285,148.618,-69.355],[20.916,148.583,-69.488],[21.54,148.554,-69.62],[22.157,148.528,-69.75],[22.767,148.502,-69.879],[23.37,148.476,-70.006],[23.966,148.447,-70.131],[24.553,148.418,-70.255],[25.131,148.388,-70.377],[25.701,148.359,-70.498],[26.262,148.331,-70.616],[26.814,148.303,-70.732],[27.356,148.276,-70.847],[27.887,148.25,-70.959],[28.409,148.223,-71.069],[28.919,148.194,-71.177],[29.419,148.164,-71.282],[30.383,148.132,-71.486],[31.299,148.1,-71.679],[32.164,148.068,-71.862],[32.975,148.039,-72.033],[33.729,148.013,-72.193],[34.425,147.989,-72.34],[35.059,147.966,-72.474],[35.628,147.943,-72.594],[36.13,147.92,-72.7],[36.922,147.895,-72.868],[37.581,147.869,-73.007]]"#,
    );
    let edge2_points = road_points_from_json(
        r#"[[-21.126,151.278,-60.618],[-21.171,151.349,-60.831],[-21.279,151.427,-61.344],[-21.388,151.514,-61.858],[-21.497,151.61,-62.371],[-21.605,151.712,-62.884],[-21.714,151.817,-63.397],[-21.822,151.921,-63.91],[-21.93,152.024,-64.422],[-22.039,152.125,-64.934],[-22.147,152.226,-65.445],[-22.255,152.327,-65.955],[-22.363,152.429,-66.465],[-22.47,152.532,-66.975],[-22.578,152.638,-67.483],[-22.685,152.746,-67.991],[-22.792,152.854,-68.498],[-22.899,152.957,-69.004],[-23.006,153.049,-69.509],[-23.113,153.123,-70.013],[-23.219,153.178,-70.516],[-23.325,153.216,-71.017],[-23.431,153.243,-71.518],[-23.537,153.264,-72.017],[-23.642,153.285,-72.514],[-23.747,153.308,-73.011],[-23.851,153.333,-73.505],[-23.956,153.36,-73.998],[-24.06,153.388,-74.49],[-24.163,153.421,-74.98],[-24.318,153.458,-75.711],[-24.472,153.501,-76.438],[-24.675,153.549,-77.401],[-24.877,153.602,-78.356],[-25.077,153.659,-79.302],[-25.275,153.718,-80.238],[-25.471,153.778,-81.165],[-25.665,153.84,-82.081],[-25.857,153.902,-82.987],[-26.046,153.964,-83.881],[-26.233,154.025,-84.764],[-26.417,154.086,-85.634],[-26.598,154.145,-86.492],[-26.777,154.204,-87.336],[-26.952,154.262,-88.166],[-27.125,154.321,-88.982],[-27.294,154.38,-89.784],[-27.461,154.44,-90.57],[-27.623,154.5,-91.34],[-27.783,154.559,-92.094],[-27.939,154.617,-92.831],[-28.091,154.674,-93.551],[-28.24,154.729,-94.253],[-28.384,154.782,-94.937],[-28.525,154.833,-95.602],[-28.661,154.882,-96.247],[-28.794,154.928,-96.873],[-28.922,154.969,-97.479],[-29.045,155.007,-98.063],[-29.165,155.043,-98.627],[-29.279,155.084,-99.168],[-29.389,155.136,-99.687],[-29.494,155.202,-100.184],[-29.642,155.284,-100.885],[-29.822,155.38,-101.735],[-29.98,155.484,-102.484],[-30.117,155.593,-103.129],[-30.23,155.703,-103.666],[-30.439,155.813,-104.65]]"#,
    );

    let mut graph = RegionGraph::new();
    let west = graph.add_node(edge0_points[0], NodeType::Junction);
    let center = graph.add_node(*edge0_points.last().unwrap(), NodeType::Junction);
    let east = graph.add_node(*edge1_points.last().unwrap(), NodeType::Junction);
    let south = graph.add_node(*edge2_points.last().unwrap(), NodeType::Junction);
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
        east,
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
        "regenerated elevated edge 0 must clip into the JunctionN; clip={:.3}",
        graph.edge(0).end_clip
    );
    assert!(
        graph.edge(1).start_clip > 0.0,
        "regenerated elevated edge 1 must clip into the JunctionN; clip={:.3}",
        graph.edge(1).start_clip
    );
    assert!(
        graph.edge(2).start_clip > 0.0,
        "regenerated elevated edge 2 must clip into the JunctionN; clip={:.3}",
        graph.edge(2).start_clip
    );

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "regenerated elevated JunctionN",
    );
}

#[test]
fn logged_current_elevated_three_way_rejects_same_material_height_conflict() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let edge0_points = road_points_from_json(
        r#"[[-36.17,139.833,-5.769],[-36.277,139.832,-6.399],[-36.406,139.83,-7.164],[-36.518,139.829,-7.83],[-36.651,139.83,-8.615],[-36.803,139.834,-9.516],[-36.93,139.842,-10.264],[-37.02,139.854,-10.796],[-37.114,139.87,-11.355],[-37.213,139.888,-11.94],[-37.316,139.907,-12.549],[-37.423,139.926,-13.183],[-37.534,139.945,-13.841],[-37.649,139.963,-14.523],[-37.768,139.979,-15.227],[-37.891,139.992,-15.954],[-38.017,140.002,-16.702],[-38.147,140.01,-17.472],[-38.28,140.023,-18.262],[-38.417,140.046,-19.072],[-38.557,140.088,-19.902],[-38.701,140.15,-20.751],[-38.847,140.233,-21.617],[-38.997,140.331,-22.502],[-39.149,140.44,-23.404],[-39.304,140.552,-24.323],[-39.462,140.661,-25.257],[-39.622,140.762,-26.208],[-39.785,140.851,-27.173],[-39.909,140.926,-27.906],[-39.992,140.989,-28.399],[-40.076,141.046,-28.896],[-40.161,141.107,-29.396],[-40.246,141.179,-29.899],[-40.331,141.266,-30.406],[-40.417,141.37,-30.915],[-40.504,141.485,-31.428],[-40.591,141.606,-31.944],[-40.679,141.728,-32.463],[-40.767,141.849,-32.984],[-40.855,141.969,-33.508],[-40.944,142.088,-34.035],[-41.034,142.207,-34.565],[-41.124,142.326,-35.097],[-41.214,142.447,-35.632],[-41.305,142.568,-36.169],[-41.396,142.689,-36.709],[-41.487,142.809,-37.251],[-41.579,142.927,-37.795],[-41.672,143.046,-38.341],[-41.764,143.168,-38.889],[-41.857,143.297,-39.44],[-41.95,143.435,-39.992],[-42.044,143.584,-40.546],[-42.138,143.744,-41.102],[-42.232,143.91,-41.66],[-42.326,144.079,-42.219],[-42.421,144.249,-42.78],[-42.516,144.419,-43.342],[-42.611,144.587,-43.906],[-42.707,144.755,-44.471],[-42.803,144.923,-45.038],[-42.898,145.091,-45.605],[-42.995,145.26,-46.174],[-43.091,145.429,-46.744],[-43.187,145.598,-47.316],[-43.284,145.767,-47.888],[-43.381,145.936,-48.46],[-43.478,146.106,-49.034],[-43.575,146.279,-49.609],[-43.672,146.456,-50.184],[-43.769,146.638,-50.759],[-43.866,146.825,-51.336],[-43.964,147.016,-51.912],[-44.061,147.204,-52.489],[-44.159,147.386,-53.067],[-44.256,147.555,-53.644],[-44.354,147.713,-54.222],[-44.411,147.862,-54.564]]"#,
    );
    let edge1_points = road_points_from_json(
        r#"[[-44.411,147.862,-54.564],[-43.727,147.963,-54.68],[-43.099,148.062,-54.786],[-42.291,148.157,-54.922],[-41.575,148.248,-55.043],[-41.049,148.335,-55.132],[-40.486,148.421,-55.227],[-39.887,148.509,-55.328],[-39.255,148.602,-55.435],[-38.592,148.701,-55.547],[-37.899,148.809,-55.664],[-37.177,148.922,-55.786],[-36.43,149.041,-55.912],[-35.658,149.164,-56.043],[-34.864,149.294,-56.177],[-34.049,149.429,-56.314],[-33.215,149.571,-56.455],[-32.363,149.714,-56.599],[-31.497,149.852,-56.746],[-30.617,149.978,-56.894],[-29.725,150.086,-57.045],[-28.823,150.174,-57.197],[-27.913,150.244,-57.351],[-26.997,150.303,-57.506],[-26.076,150.358,-57.661],[-25.153,150.414,-57.817],[-24.229,150.473,-57.973],[-23.305,150.535,-58.129],[-22.385,150.595,-58.285],[-21.468,150.649,-58.44],[-20.558,150.692,-58.593],[-19.657,150.721,-58.746],[-18.765,150.735,-58.896],[-17.885,150.738,-59.045],[-17.018,150.732,-59.191],[-16.167,150.724,-59.335],[-15.333,150.715,-59.476],[-14.518,150.707,-59.614],[-13.724,150.698,-59.748],[-12.952,150.688,-59.878],[-12.204,150.676,-60.005],[-11.483,150.659,-60.126],[-10.79,150.64,-60.243],[-10.126,150.616,-60.356],[-9.495,150.59,-60.462],[-8.896,150.56,-60.563],[-8.333,150.527,-60.658],[-7.807,150.491,-60.747],[-7.091,150.453,-60.868],[-6.283,150.413,-61.005],[-5.655,150.371,-61.111],[-4.97,150.329,-61.226]]"#,
    );
    let edge2_points = road_points_from_json(
        r#"[[-44.411,147.862,-54.564],[-44.451,147.995,-54.8],[-44.549,148.139,-55.378],[-44.647,148.3,-55.956],[-44.744,148.48,-56.534],[-44.842,148.673,-57.112],[-44.939,148.87,-57.689],[-45.037,149.067,-58.266],[-45.134,149.257,-58.843],[-45.231,149.436,-59.419],[-45.329,149.602,-59.995],[-45.426,149.755,-60.57],[-45.523,149.896,-61.144],[-45.62,150.028,-61.718],[-45.716,150.157,-62.291],[-45.813,150.285,-62.863],[-45.91,150.414,-63.434],[-46.006,150.545,-64.004],[-46.102,150.676,-64.573],[-46.198,150.807,-65.141],[-46.293,150.937,-65.707],[-46.389,151.067,-66.272],[-46.484,151.196,-66.836],[-46.579,151.324,-67.399],[-46.674,151.453,-67.959],[-46.768,151.58,-68.519],[-46.862,151.707,-69.076],[-46.956,151.831,-69.632],[-47.05,151.954,-70.186],[-47.143,152.075,-70.739],[-47.236,152.195,-71.289],[-47.329,152.314,-71.837],[-47.421,152.434,-72.383],[-47.513,152.555,-72.928],[-47.604,152.676,-73.469],[-47.696,152.799,-74.009],[-47.786,152.922,-74.546],[-47.877,153.044,-75.081],[-47.966,153.167,-75.613],[-48.056,153.29,-76.143],[-48.145,153.415,-76.67],[-48.233,153.542,-77.194],[-48.322,153.672,-77.716],[-48.409,153.803,-78.234],[-48.496,153.932,-78.75],[-48.583,154.051,-79.263],[-48.669,154.154,-79.772],[-48.755,154.237,-80.279],[-48.84,154.299,-80.782],[-48.924,154.349,-81.282],[-49.008,154.395,-81.779],[-49.091,154.447,-82.272],[-49.215,154.512,-83.006],[-49.378,154.589,-83.971],[-49.538,154.677,-84.921],[-49.696,154.769,-85.856],[-49.851,154.863,-86.774],[-50.004,154.955,-87.676],[-50.153,155.046,-88.561],[-50.3,155.137,-89.428],[-50.443,155.229,-90.276],[-50.583,155.322,-91.106],[-50.72,155.415,-91.916],[-50.853,155.509,-92.706],[-50.983,155.6,-93.476],[-51.11,155.69,-94.224],[-51.232,155.777,-94.951],[-51.351,155.862,-95.655],[-51.467,155.945,-96.337],[-51.578,156.026,-96.995],[-51.685,156.104,-97.629],[-51.788,156.179,-98.239],[-51.886,156.25,-98.823],[-51.981,156.317,-99.382],[-52.071,156.381,-99.914],[-52.156,156.443,-100.42],[-52.276,156.507,-101.127],[-52.418,156.573,-101.971],[-52.541,156.643,-102.697],[-52.643,156.717,-103.301],[-52.83,156.793,-104.409]]"#,
    );

    let mut graph = RegionGraph::new();
    let west = graph.add_node(edge0_points[0], NodeType::Junction);
    let center = graph.add_node(*edge0_points.last().unwrap(), NodeType::Junction);
    let east = graph.add_node(*edge1_points.last().unwrap(), NodeType::Junction);
    let south = graph.add_node(*edge2_points.last().unwrap(), NodeType::Junction);
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
        east,
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
    graph.solve_junction_endpoint_profiles_for_edges(
        &HashSet::from([center]),
        &HashSet::from([0, 1, 2]),
    );
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_junction_mouth_section_profile_laterally_flat(&surface, &graph, 0, false);
    assert_junction_mouth_section_profile_laterally_flat(&surface, &graph, 1, true);
    assert_junction_mouth_section_profile_laterally_flat(&surface, &graph, 2, true);
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "current elevated JunctionN",
    );

    let mut edit_graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let config = crate::simulation::core::config::WorldConfig::default();
    let mut zoning = crate::simulation::grid::zoning::ZoningSystem::new(&config);
    let mut allocator = crate::simulation::buildings::allocator::BuildingAllocator::new();
    for points in [edge0_points, edge1_points, edge2_points] {
        network.add_road(
            &mut edit_graph,
            points,
            1,
            1,
            EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        network.road_surface.compile_dirty(&edit_graph, &terrain);
    }
    let edit_center = (0..edit_graph.node_count() as u32)
        .find(|&node_id| {
            edit_graph
                .node_adjacency(node_id)
                .iter()
                .filter(|&&edge_idx| !edit_graph.edge(edge_idx).deleted)
                .count()
                == 3
        })
        .expect("add_road edit path must create the elevated 3-way junction node");
    assert_junction_rejected_with_canonical_height_diagnostic(
        &network.road_surface,
        &edit_graph,
        edit_center,
        "add_road current elevated JunctionN",
    );
}

#[test]
fn logged_flat_oblique_t_junction_compiles_with_explicit_curb_sidewalk_endpoint_authority() {
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

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .unwrap_or_else(|| {
            panic!(
                "logged flat oblique T must compile with explicit curb/sidewalk endpoint path: {}",
                canonical_junction_pipeline_report(&surface, &graph, center)
            )
        });
    assert_top_raised_step_owner_boundaries_have_vertical_faces(piece);
    assert_canonical_explicit_vertical_steps_have_faces(piece);
}

#[test]
fn logged_flat_oblique_four_way_compiles_with_explicit_height_carriers() {
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

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn arbitrary_six_way_junction_compiles_with_explicit_height_carriers() {
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

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "arbitrary six-way JunctionN did not compile: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn arbitrary_five_way_junction_compiles_with_explicit_height_carriers() {
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

    assert_compiled_junction_piece(&surface, &graph, center);
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
    let start_kind = surface.classify_surface_node_kind_from_graph_geometry(
        &graph,
        graph.get_valid_node(edge.start_node),
    );
    let end_kind = surface.classify_surface_node_kind_from_graph_geometry(
        &graph,
        graph.get_valid_node(edge.end_node),
    );
    let (_, expected_handoff_s) = surface
        .visual_surface_handoff_range_for_edge(
            &graph,
            left_edge,
            edge,
            total_length,
            start_kind,
            end_kind,
        )
        .expect("left edge should have a visible span range after pairwise handoff");
    let local_handoff_s = RoadSurfaceSystem::visual_end_handoff_s_m(edge, total_length);
    assert!(
        expected_handoff_s < local_handoff_s - SAMPLE_EPSILON_M,
        "pairwise node ownership must extend the visual handoff before the old local limit"
    );
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
fn dirty_recompile_expanded_arbitrary_node_piece_compiles_with_explicit_height_carriers() {
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

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "expanded arbitrary JunctionN did not compile: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
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

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn elevated_four_way_junction_rejects_same_material_height_conflict_after_endpoint_profile_solve() {
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
    let adaptable_edges = (0..graph.edge_count()).collect::<HashSet<_>>();
    graph.solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &adaptable_edges);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "elevated 4-way JunctionN after endpoint profile solve",
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
    let adaptable_edges = (0..graph.edge_count()).collect::<HashSet<_>>();
    graph.solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &adaptable_edges);
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
    if surface.compiled_visual_node_pieces().contains_key(&center) {
        let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[0, 1, 2, 3]);
        assert!(
            !dump.contains("source_height_field_conflict")
                && !dump.contains("shared_source_height_conflict")
                && !dump.contains("height_conflict"),
            "steep JunctionN may compile only when same-XZ side vertices are resolved without hidden height conflicts: {dump}"
        );
    }
}

#[test]
fn elevated_three_way_junction_rejects_same_material_height_conflict_after_endpoint_profile_solve()
{
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
    let adaptable_edges = (0..graph.edge_count()).collect::<HashSet<_>>();
    graph.solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &adaptable_edges);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "elevated 3-way JunctionN after endpoint profile solve",
    );
}

#[test]
fn skewed_elevated_four_way_junction_rejects_same_material_height_conflict() {
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
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "skewed elevated 4-way JunctionN",
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
    let edge_idx = graph.add_edge(test_edge(
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
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("bridge span should compile");
    assert!(!span_piece.span_earthwork_support_regions.is_empty());
    assert_span_earthwork_faces_have_support_provenance(span_piece, edge_idx, EdgeClass::Bridge);
    assert!(
        span_piece
            .span_earthwork_support_regions
            .iter()
            .all(|region| !(region.start_s_m < 24.0 && region.end_s_m > 24.0)),
        "bridge support regions must stay at endpoint abutments instead of owning midspan terrain"
    );
    assert!(
        span_piece.render_earthwork_faces.iter().all(|face| {
            let RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                start_s_m,
                end_s_m,
                support_policy,
                ..
            } = face.source
            else {
                return false;
            };
            support_policy == RoadSurfaceEarthworkSupportPolicy::BridgeEndpointAbutments
                && !(start_s_m < 24.0 && end_s_m > 24.0)
        }),
        "bridge earthwork faces must preserve endpoint abutment support provenance"
    );

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
    let edge_idx = graph.add_edge(test_edge(
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
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("tunnel span should compile");
    assert!(!span_piece.span_earthwork_support_regions.is_empty());
    assert_span_earthwork_faces_have_support_provenance(span_piece, edge_idx, EdgeClass::Tunnel);
    assert!(
        span_piece
            .span_earthwork_support_regions
            .iter()
            .all(|region| !(region.start_s_m < 24.0 && region.end_s_m > 24.0)),
        "tunnel support regions must stay at visible portals instead of owning buried midspan terrain"
    );
    assert!(
        span_piece.render_earthwork_faces.iter().all(|face| {
            let RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                start_s_m,
                end_s_m,
                support_policy,
                ..
            } = face.source
            else {
                return false;
            };
            support_policy == RoadSurfaceEarthworkSupportPolicy::TunnelVisiblePortals
                && !(start_s_m < 24.0 && end_s_m > 24.0)
        }),
        "tunnel earthwork faces must preserve visible portal support provenance"
    );

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
fn visible_surface_height_ignores_non_surface_node_adjacency() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let road_end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        center,
        road_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let rail_end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        center,
        rail_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Rail,
        TransitFlags::RAIL,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .expect("surface node piece should compile from the road adjacency");
    let sample = piece
        .road_surface_polygons
        .iter()
        .chain(&piece.curb_surface_polygons)
        .chain(&piece.sidewalk_surface_polygons)
        .flat_map(|polygon| polygon.triangles_world.iter().copied())
        .map(triangle_centroid_xz)
        .next()
        .expect("compiled node piece should contain visible top-surface triangles");
    assert!(
        surface
            .sample_visible_surface_height(&graph, &terrain, sample.x, sample.y)
            .is_some(),
        "non-surface adjacency must not hide a valid road-owned node surface"
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
fn visible_surface_raycast_hits_road_without_terrain_hit() {
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
        .raycast_visible_surface(&graph, &terrain, Vector3::new(0.0, 2.0, 0.0), Vector3::UP)
        .expect("road-owned visible surface should be hittable even when terrain is not");
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
    assert!(dump.contains("\"span_ownership\""));
    assert!(dump.contains("\"owned_region_count\""));
    assert!(dump.contains("\"source_band_index\""));
    assert!(dump.contains("\"start_section_index\""));
    assert!(dump.contains("\"span_earthwork_support\""));
    assert!(dump.contains("\"support_region_count\""));
    assert!(dump.contains("\"support_policy\""));
    assert!(dump.contains("\"span_earthwork_face_sources\""));
    assert!(dump.contains("\"source_kind\":\"span_support_boundary\""));
    assert!(dump.contains("\"sourced_earthwork_face_count\""));
    assert!(dump.contains("\"missing_earthwork_face_source_count\":0"));
    assert!(dump.contains("\"span_raised_step_face_sources\""));
    assert!(dump.contains("\"lower_owner\""));
    assert!(dump.contains("\"raised_owner\""));
    assert!(dump.contains("\"terrain_clip_source_edges\""));
    assert!(dump.contains("\"span_projection_diagnostics\""));
    assert!(dump.contains("\"road_projection_matches\":true"));
    assert!(dump.contains("\"curb_projection_matches\":true"));
    assert!(dump.contains("\"sidewalk_projection_matches\":true"));
    assert!(dump.contains("\"earthwork_support_region_count\""));
    assert!(dump.contains("\"raised_step_source_count_matches\":true"));
    assert!(dump.contains("\"source_center_y_m\""));
    assert!(dump.contains("\"visual_center_y_m\""));
    assert!(dump.contains("\"left_outer_margin\""));
    assert!(dump.contains("\"right_outer_margin\""));
    assert!(dump.contains("\"node_compile_status\""));
    assert!(dump.contains("\"compiled\": true"));
    assert!(dump.contains("\"nodes\""));
    assert!(dump.contains("\"road_topology\""));
    assert!(dump.contains("\"sidewalk_topology\""));
    assert!(dump.contains("\"raised_step_face_details\""));
    assert!(dump.contains("\"expected_raised_steps\""));
    assert!(dump.contains("\"source_constraint_count\""));
    assert!(dump.contains("\"final_required_face_count\""));
    assert!(dump.contains("\"missing_required_face_count\""));
    assert!(dump.contains("\"non_exposed_source_constraint_count\""));
    assert!(dump.contains("\"materialization_status\""));
    assert!(dump.contains("\"band_ownership\""));
    assert!(dump.contains("\"height_owner\""));
    assert!(dump.contains("\"node_grade_authority\""));
    assert!(dump.contains("\"decision\":\"source_carrier\""));
    assert!(dump.contains("\"seam_constraints\""));
    assert!(dump.contains("\"material_footprint_coverage\""));
    assert!(dump.contains("\"outer_boundary_top_match\""));
    assert!(dump.contains("\"direct_source_count\""));
    assert!(dump.contains("\"top_surface_source_index\""));
    assert!(dump.contains("\"grade_authority_index\""));
    assert!(dump.contains("\"mouth_seams\""));
    assert!(dump.contains("\"earthwork_face_sources\""));
    assert!(dump.contains("\"source_kind\":\"node_footprint_boundary\""));
    assert!(dump.contains("\"boundary_source\""));
    assert!(dump.contains("\"node_footprint_source_count\""));
    assert!(dump.contains("\"missing_source_count\":0"));
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
