//! Shared road-surface test fixtures and assertions.

use super::*;
pub(super) fn span_profile_test_section(
    edge_idx: usize,
    s_m: f32,
    bands: Vec<RoadSurfaceBand>,
) -> RoadSurfaceSection {
    RoadSurfaceSection {
        edge_idx,
        s_m,
        center_xz: Vector2::new(s_m, 0.0),
        center_height_m: 0.0,
        tangent_xz: Vector2::new(1.0, 0.0),
        lateral_xz: Vector2::new(0.0, 1.0),
        bands,
    }
}

pub(super) fn assert_rejects_invalid_span_profile(
    sections_for_edge: impl FnOnce(usize) -> Vec<RoadSurfaceSection>,
    reason: &str,
) {
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(40.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)],
        5.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR,
    ));

    let mut surface = RoadSurfaceSystem::new(64.0);
    surface
        .compiled_sections
        .insert(edge_idx, sections_for_edge(edge_idx));

    assert!(
        surface
            .compile_visual_span_piece(&graph, &flat_terrain(64, 64), edge_idx)
            .is_none(),
        "span region resolution must reject {reason} instead of emitting partial top-surface or terrain-clip output"
    );
}

pub(super) fn test_edge(
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

pub(super) fn flat_terrain(width: usize, height: usize) -> TerrainSystem {
    TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0)
}

pub(super) fn sloped_terrain(width: usize, height: usize) -> TerrainSystem {
    let mut terrain = TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0);
    for z in 0..height {
        for x in 0..width {
            terrain.set_height(x, z, x as f32 * 0.05);
        }
    }
    terrain
}

pub(super) fn road_points_from_json(points_json: &str) -> Vec<Vector3> {
    serde_json::from_str::<Vec<[f32; 3]>>(points_json)
        .expect("logged road geometry points must parse")
        .into_iter()
        .map(|[x, y, z]| Vector3::new(x, y, z))
        .collect()
}

pub(super) fn terrain_clip_source_edge_for_test(
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

pub(super) fn ridge_terrain(width: usize, height: usize) -> TerrainSystem {
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

pub(super) fn planar_world_terrain(
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

pub(super) fn coarse_hillside_world_terrain(
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

pub(super) fn grounded_polyline_points_from_terrain(
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
pub(super) struct FootprintOverflowMetrics {
    pub(super) max_overflow_m: f32,
}

pub(super) fn footprint_sample_offsets(section: &RoadSurfaceSection) -> Vec<f32> {
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

pub(super) fn measure_max_footprint_overflow(
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

pub(super) fn build_coarse_grid_hillside_case(
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

pub(super) fn terrain_cdt_input_for_bounds(
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

pub(super) fn terrain_height_m(terrain: &TerrainSystem, x: f32, z: f32) -> f32 {
    terrain.sample_visual_height_world(x, z) * crate::config::HEIGHT_SCALE
}

pub(super) fn assert_surface_terrain_cdt_contract(
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

pub(super) fn assert_surface_cdt_boundary_source(
    case_name: &str,
    source: TerrainCdtRoadBoundarySource,
) {
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

pub(super) fn assert_cdt_mesh_sources_are_structured(case_name: &str, mesh: &TerrainCdtMesh) {
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

pub(super) fn assert_cdt_mesh_stays_outside_road_loops(
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

pub(super) fn road_loop_contains_road_owned_point_xz(
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

pub(super) fn terrain_cdt_loop_strictly_contains_point_xz(
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

pub(super) fn compile_committed_preview_reference(
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

pub(super) fn assert_preview_vertices_use_solved_section_height_keys(
    preview: &PreviewRoadSurfaceResult,
) {
    let solved_height_keys = preview
        .compiled_sections
        .iter()
        .flat_map(|section| section.bands.iter())
        .flat_map(|band| {
            [
                SurfaceHeightMmKey::from_m_f32(band.height_start_m),
                SurfaceHeightMmKey::from_m_f32(band.height_end_m),
            ]
        })
        .collect::<BTreeSet<_>>();
    assert!(
        !solved_height_keys.is_empty(),
        "preview height-key regression check requires compiled section bands"
    );
    assert!(
        !preview.surface_vertices.is_empty(),
        "preview height-key regression check requires preview mesh vertices"
    );

    for vertex in &preview.surface_vertices {
        let key = SurfaceHeightMmKey::from_m_f32(vertex.y);
        assert!(
            solved_height_keys.contains(&key),
            "preview mesh vertex height must come from solved section geometry without render lift: y={:.6} key={} solved_keys={:?}",
            vertex.y,
            key.as_i64(),
            solved_height_keys
        );
    }
}

pub(super) fn triangle_centroid_xz(triangle: [Vector3; 3]) -> Vector2 {
    Vector2::new(
        (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0,
        (triangle[0].z + triangle[1].z + triangle[2].z) / 3.0,
    )
}

pub(super) fn point_inside_visual_polygons(
    polygons: &[RoadSurfaceVisualPolygon],
    point: Vector2,
) -> bool {
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

pub(super) fn visual_polygon_boundary_contains_xz(
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

pub(super) fn overlay_contours_from_polygons(
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

pub(super) fn overlay_contour_from_world_points(points: &[Vector3]) -> super::NodeOverlayContour {
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

pub(super) fn overlay_contours_from_top_polygons<'a>(
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

pub(super) fn overlay_area_m2(shapes: &super::NodeOverlayShapes) -> f32 {
    shapes
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum()
}

pub(super) fn node_top_coverage_details_m2(
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

pub(super) fn assert_node_top_covers_footprint(piece: &RoadSurfaceVisualNodePiece) {
    let (missing_area_m2, extra_area_m2, budget_m2, missing_shapes, extra_shapes) =
        node_top_coverage_details_m2(piece);
    assert!(
        missing_area_m2 <= budget_m2 && extra_area_m2 <= budget_m2,
        "node top surfaces must exactly cover the canonical footprint; kind={:?} missing_area={missing_area_m2:.6} extra_area={extra_area_m2:.6} budget={budget_m2:.6} missing_shapes={missing_shapes:?} extra_shapes={extra_shapes:?}",
        piece.kind
    );
}

pub(super) fn assert_earthwork_faces_stay_outside_top_footprint(
    piece: &RoadSurfaceVisualNodePiece,
) {
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

pub(super) fn assert_node_earthwork_faces_have_footprint_provenance(
    piece: &RoadSurfaceVisualNodePiece,
) {
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

pub(super) fn assert_node_footprint_boundary_vertex_source_is_valid(
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
        NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. } => {}
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

pub(super) fn assert_span_earthwork_faces_have_support_provenance(
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

pub(super) fn assert_material_triangles_do_not_overlap(piece: &RoadSurfaceVisualNodePiece) {
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

pub(super) fn assert_terminal_mouth_handoff_surface_is_owned(
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

pub(super) fn assert_terminal_band_interval_grid_is_owned(
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

pub(super) fn assert_terminal_band_interval_grid_is_not_duplicated_by_span(
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

pub(super) fn assert_raised_step_face_lower_edge_covers(
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
pub(super) struct TestTopBoundaryEdge {
    pub(super) kind: RoadSurfaceBandKind,
    pub(super) owner_index: usize,
    pub(super) start: Vector3,
    pub(super) end: Vector3,
    pub(super) key: TestRenderEdgeKey,
    pub(super) xz_key: TestRenderXzEdgeKey,
    pub(super) avg_y_m: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct TestRenderVertexKey {
    pub(super) x_key: i64,
    pub(super) y_mm: i64,
    pub(super) z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct TestRenderEdgeKey {
    pub(super) start: TestRenderVertexKey,
    pub(super) end: TestRenderVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct TestRenderXzVertexKey {
    pub(super) x_key: i64,
    pub(super) z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct TestRenderXzEdgeKey {
    pub(super) start: TestRenderXzVertexKey,
    pub(super) end: TestRenderXzVertexKey,
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

pub(super) fn test_render_xz_vertex_key_lies_on_segment(
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

pub(super) fn assert_top_raised_step_owner_boundaries_have_vertical_faces(
    piece: &RoadSurfaceVisualNodePiece,
) {
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

pub(super) fn explicit_vertical_step_descriptions_for_xz_key(
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

pub(super) fn assert_canonical_explicit_vertical_steps_have_faces(
    piece: &RoadSurfaceVisualNodePiece,
) {
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

pub(super) fn explicit_vertical_step_has_visible_top_support(
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

pub(super) fn test_owners_form_raised_step(
    lower_kind: RoadSurfaceBandKind,
    raised_kind: RoadSurfaceBandKind,
) -> bool {
    ordered_raised_step_kinds(lower_kind, raised_kind) == Some((lower_kind, raised_kind))
}

pub(super) fn test_top_edges_form_raised_step(
    lower_edge: TestTopBoundaryEdge,
    raised_edge: TestTopBoundaryEdge,
) -> bool {
    test_owners_form_raised_step(lower_edge.kind, raised_edge.kind)
}

pub(super) fn explicit_vertical_step_segment_len_squared_m2(
    segment: super::arrangement::NodeExplicitVerticalStepSegment,
) -> f32 {
    let dx = (segment.end().x_key() - segment.start().x_key()) as f64
        / super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
    let dz = (segment.end().z_key() - segment.start().z_key()) as f64
        / super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
    (dx * dx + dz * dz) as f32
}

pub(super) fn test_owned_top_boundary_edges(
    piece: &RoadSurfaceVisualNodePiece,
) -> Vec<TestTopBoundaryEdge> {
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

pub(super) fn vertical_face_lower_edge_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<[Vector3; 2]> {
    let [first_edge, second_edge] = vertical_face_side_edges_for_test(polygon)?;
    let first_avg_y = (first_edge[0].y + first_edge[1].y) * 0.5;
    let second_avg_y = (second_edge[0].y + second_edge[1].y) * 0.5;
    Some(if first_avg_y <= second_avg_y {
        first_edge
    } else {
        second_edge
    })
}

pub(super) fn vertical_face_side_edges_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<[[Vector3; 2]; 2]> {
    let [a, b, c, d] = polygon.points_world.as_slice() else {
        return None;
    };
    Some([[*a, *d], [*b, *c]])
}

pub(super) fn test_xz_key_lies_on_segment(
    point: (i64, i64),
    start: (i64, i64),
    end: (i64, i64),
) -> bool {
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

pub(super) fn test_xz_key(point: Vector3) -> (i64, i64) {
    let point =
        super::backend::road_vec2_to_overlay_point(super::backend::godot_vec3_xz_to_road(point));
    (
        (point[0] * super::backend::ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point[1] * super::backend::ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

pub(super) fn triangle_overlap_area_m2(a: [Vector3; 3], b: [Vector3; 3]) -> f32 {
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

pub(super) fn triangle_overlap_numeric_budget_m2(a: [Vector3; 3], b: [Vector3; 3]) -> f32 {
    RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&triangle_overlay_shapes(a)).max(
        RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&triangle_overlay_shapes(b)),
    )
}

pub(super) fn triangle_overlay_shapes(triangle: [Vector3; 3]) -> super::NodeOverlayShapes {
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

pub(super) fn assert_top_mesh_centroids_inside_outer_boundary(piece: &RoadSurfaceVisualNodePiece) {
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

pub(super) fn assert_top_surface_triangles_face_up(piece: &RoadSurfaceVisualNodePiece) {
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

pub(super) fn assert_raised_step_faces_visible_from_lower_owner(
    piece: &RoadSurfaceVisualNodePiece,
) {
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

pub(super) fn test_lower_owner_from_vertical_face_source(
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

pub(super) fn vertical_face_owner_edge_for_test(
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

pub(super) fn assert_raised_step_faces_have_top_support(piece: &RoadSurfaceVisualNodePiece) {
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

pub(super) fn vertical_face_visible_direction_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<Vector3> {
    let [upper_start, lower_start, lower_end, _upper_end] = polygon.points_world.as_slice() else {
        return None;
    };
    let normal = (*lower_start - *upper_start).cross(*lower_end - *upper_start);
    (normal.length_squared() > 1e-8).then(|| -normal.normalized())
}

pub(super) fn vertical_face_upper_edge_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<[Vector3; 2]> {
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

pub(super) fn polygon_boundary_overlaps_edge_at_height_for_test(
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

pub(super) fn test_boundary_edge_contains_edge_at_height(
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

pub(super) fn test_boundary_segment_parameter_xz(
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

pub(super) fn test_interpolated_height_mm(
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

pub(super) fn test_xz_segments_overlap_with_length(
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

pub(super) fn polygon_centroid_for_test(polygon: &RoadSurfaceVisualPolygon) -> Option<Vector3> {
    let mut sum = Vector3::ZERO;
    let mut count = 0usize;
    for point in &polygon.points_world {
        sum += Vector3::new(point.x, 0.0, point.z);
        count += 1;
    }
    (count > 0).then_some(sum / count as f32)
}

pub(super) fn assert_node_piece_uses_band_owned_regions(piece: &RoadSurfaceVisualNodePiece) {
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

pub(super) fn assert_node_top_surface_sources_have_grade_authority(
    piece: &RoadSurfaceVisualNodePiece,
) {
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

pub(super) fn assert_node_terrain_clip_sources_have_footprint_provenance(
    piece: &RoadSurfaceVisualNodePiece,
) {
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

pub(super) fn assert_node_piece_has_curb_and_sidewalk_owners(piece: &RoadSurfaceVisualNodePiece) {
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

pub(super) fn assert_compiled_bend_piece<'a>(
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

pub(super) fn assert_compiled_junction_piece<'a>(
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

pub(super) fn canonical_junction_pipeline_report(
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

pub(super) fn canonical_node_pipeline_report(
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

pub(super) fn boundary_export_step_debug(
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

pub(super) fn assert_junction_rejected_with_canonical_height_diagnostic(
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
        || report.contains("vertex_outside_height_field")
        || report.contains("\"height_conflict\"")
        || report.contains("missing_raised_step_vertical_face")
        || report.contains("MissingRaisedStepVerticalFace");
    assert!(
        accepted_height_rejection,
        "{label} must reject with a canonical height diagnostic: {report}"
    );
}

pub(super) fn triangulation_height_conflict_debug(
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

pub(super) fn triangulation_duplicate_exposed_edge_debug(
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

pub(super) fn triangulation_regions_for_exposed_edge(
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

pub(super) fn normalized_test_world_mm_edge_key(
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

pub(super) fn normalized_test_mm_edge_key(
    start: (i64, i64),
    end: (i64, i64),
) -> ((i64, i64), (i64, i64)) {
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

pub(super) fn owned_region_claims_for_height_conflict(
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

pub(super) fn arrangement_key_from_overlay_keys(x_key: i64, z_key: i64) -> NodeArrangementKey {
    NodeArrangementKey::from_point(super::backend::RoadVec2::new(
        x_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
        z_key as f64 / super::backend::ROAD_OVERLAY_COORDINATE_SCALE,
    ))
}

pub(super) fn source_rail_debug_for_height_conflict(
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

pub(super) fn world_path_debug(path: &[super::backend::RoadVec3]) -> String {
    let points = path
        .iter()
        .map(|point| format!("({:.3},{:.3},{:.3})", point.x, point.y, point.z))
        .collect::<Vec<_>>();
    format!("[{}]", points.join(","))
}

pub(super) fn height_solution_vertices_at_arrangement_key(
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

pub(super) fn assert_outer_boundary_vertices_match_visible_top(piece: &RoadSurfaceVisualNodePiece) {
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

pub(super) fn assert_outer_boundary_vertices_use_visible_top_boundary_support(
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

pub(super) fn closest_point_on_segment_xz(point: Vector3, start: Vector3, end: Vector3) -> Vector3 {
    let segment = Vector2::new(end.x - start.x, end.z - start.z);
    let len_squared = segment.length_squared();
    if len_squared <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
        return start;
    }
    let to_point = Vector2::new(point.x - start.x, point.z - start.z);
    let t = (to_point.dot(segment) / len_squared).clamp(0.0, 1.0);
    start.lerp(end, t)
}

pub(super) fn visible_top_vertices(piece: &RoadSurfaceVisualNodePiece) -> Vec<Vector3> {
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

pub(super) fn assert_material_top_supports_point(
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

pub(super) fn polygon_supports_top_point(
    polygon: &RoadSurfaceVisualPolygon,
    point: Vector3,
) -> bool {
    polygon_vertices_support_top_point(&polygon.points_world, point)
        || polygon_edges_support_top_point(&polygon.points_world, point)
        || polygon.triangles_world.iter().any(|triangle| {
            triangle
                .iter()
                .any(|&candidate| top_points_match(candidate, point))
                || triangle_edges_support_top_point(*triangle, point)
        })
}

pub(super) fn polygon_vertices_support_top_point(vertices: &[Vector3], point: Vector3) -> bool {
    vertices
        .iter()
        .copied()
        .any(|candidate| top_points_match(candidate, point))
}

pub(super) fn polygon_edges_support_top_point(vertices: &[Vector3], point: Vector3) -> bool {
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

pub(super) fn triangle_edges_support_top_point(triangle: [Vector3; 3], point: Vector3) -> bool {
    (0..3)
        .any(|index| segment_supports_top_point(point, triangle[index], triangle[(index + 1) % 3]))
}

pub(super) fn segment_supports_top_point(point: Vector3, start: Vector3, end: Vector3) -> bool {
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

pub(super) fn top_points_match(candidate: Vector3, point: Vector3) -> bool {
    test_xz_key(candidate) == test_xz_key(point) && (candidate.y - point.y).abs() <= 0.004
}

pub(super) fn assert_debug_dump_mouth_seams_are_clean(dump: &str) {
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

pub(super) fn section_height_at_lateral_offset(
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

pub(super) fn assert_junction_mouth_section_profile_matches_endpoint_plane(
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
    let node_id = graph.get_valid_node(if at_start {
        edge.start_node
    } else {
        edge.end_node
    });
    let plane = graph
        .junction_endpoint_profile_plane(node_id)
        .expect("JunctionN endpoint must expose a solved profile plane");
    let tolerance_m = 0.005;
    for band in &section.bands {
        let height_offset_m = match band.kind {
            RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk => {
                CURB_STEP_HEIGHT_M
            }
            _ => 0.0,
        };
        for (lateral_m, height_m) in [
            (band.lateral_start_m, band.height_start_m),
            (band.lateral_end_m, band.height_end_m),
        ] {
            let expected_height_m = plane.height_at_xz(
                section.center_xz.x + section.lateral_xz.x * lateral_m,
                section.center_xz.y + section.lateral_xz.y * lateral_m,
            ) + height_offset_m;
            assert!(
                (height_m - expected_height_m).abs() <= tolerance_m,
                "JunctionN mouth band height must match the endpoint profile plane: edge={edge_idx} at_start={at_start} s_m={:.3} kind={:?} lateral={lateral_m:.3} height={height_m:.3} expected={expected_height_m:.3} delta={:.3}",
                section.s_m,
                band.kind,
                height_m - expected_height_m
            );
        }
    }
}

pub(super) fn outer_surface_lateral_bounds(section: &RoadSurfaceSection) -> Option<(f32, f32)> {
    Some((
        section.bands.first()?.lateral_start_m,
        section.bands.last()?.lateral_end_m,
    ))
}

pub(super) fn node_earthwork_face_edge_class(
    piece: &RoadSurfaceVisualNodePiece,
    source: RoadSurfaceEarthworkFaceSource,
) -> Option<EdgeClass> {
    let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
        owner_kind,
        owner_index,
        ..
    } = source
    else {
        return None;
    };
    piece
        .earthwork_owner_sources
        .iter()
        .find(|owner_source| {
            owner_source.owner_kind == owner_kind && owner_source.owner_index == owner_index
        })
        .map(|owner_source| owner_source.edge_class)
}
