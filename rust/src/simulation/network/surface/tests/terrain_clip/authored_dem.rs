//! Production road-surface to terrain-CDT agreement tests on authored DEM-like terrain.

use super::*;

#[test]
fn authored_dem_span_tie_ins_preserve_production_terrain_agreement() {
    assert_authored_dem_cases(vec![
        standard_span_case(
            "supportive authored cross-slope span",
            planar_world_terrain(161, 161, 1.0, 18.0, 0.025, 0.018),
            Vector2::new(-40.0, -18.0),
            Vector2::new(40.0, 18.0),
            0.0,
            20,
            (-24.0, -18.0, 24.0, 18.0),
            false,
            false,
        ),
        standard_span_case(
            "standard span running along a steep authored slope",
            planar_world_terrain(181, 181, 1.0, 20.0, 0.18, 0.0),
            Vector2::new(-44.0, -12.0),
            Vector2::new(44.0, -12.0),
            0.0,
            24,
            (-28.0, -26.0, 28.0, 2.0),
            false,
            false,
        ),
        standard_span_case(
            "standard span crossing an extreme authored cross-slope",
            planar_world_terrain(181, 181, 1.0, 14.0, 0.0, 0.22),
            Vector2::new(-44.0, 0.0),
            Vector2::new(44.0, 0.0),
            0.0,
            24,
            (-28.0, -18.0, 28.0, 18.0),
            true,
            true,
        ),
        standard_span_case(
            "raised standard span over extreme authored terrain",
            planar_world_terrain(161, 161, 1.0, 0.0, 0.0, 0.0),
            Vector2::new(-32.0, 0.0),
            Vector2::new(32.0, 0.0),
            3.0,
            16,
            (-24.0, -18.0, 24.0, 18.0),
            true,
            true,
        ),
    ]);
}

#[test]
fn authored_dem_bend_and_junction_keep_node_footprint_sources_through_cdt() {
    let terrain = authored_ridge_valley_terrain(181, 181, 1.0);
    let center = Vector2::new(0.0, 0.0);
    let (graph, center_node) =
        standard_node_graph_with_offset_roads(&terrain, center, 2.0, &[(-36.0, 0.0), (0.0, 36.0)]);

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_eq!(
        surface
            .compiled_visual_node_pieces()
            .get(&center_node)
            .unwrap()
            .kind,
        RoadSurfaceVisualNodePieceKind::Bend
    );
    assert_authored_dem_case(AuthoredDemCase {
        name: "raised bend over authored ridge-valley terrain",
        terrain,
        graph,
        surface,
        bounds: (-52.0, -18.0, 18.0, 52.0),
        sample_step_m: 2.0,
        expect_retaining_wall: true,
        expect_widened_tie_in: true,
        expected_node_piece: None,
    });

    let terrain = authored_ridge_valley_terrain(181, 181, 1.0);
    let (graph, surface, _, terminal_node) = compile_standard_span_with_nodes_on_terrain(
        &terrain,
        Vector2::new(-44.0, -18.0),
        Vector2::new(10.0, -18.0),
        1.5,
        16,
    );
    assert_authored_dem_case(AuthoredDemCase {
        name: "raised terminal near authored ridge-valley terrain",
        terrain,
        graph,
        surface,
        bounds: (-8.0, -42.0, 34.0, 6.0),
        sample_step_m: 2.0,
        expect_retaining_wall: true,
        expect_widened_tie_in: true,
        expected_node_piece: Some((terminal_node, RoadSurfaceVisualNodePieceKind::Terminal)),
    });

    assert_authored_dem_cases(vec![
        standard_node_case(
            "raised three-way junction over authored flat DEM",
            flat_terrain(181, 181),
            center,
            3.0,
            &[(-40.0, 0.0), (40.0, 0.0), (0.0, 40.0)],
            (-56.0, -24.0, 56.0, 56.0),
            true,
            true,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        ),
        standard_node_case(
            "raised four-way junction over authored ridge-valley terrain",
            authored_ridge_valley_terrain(181, 181, 1.0),
            center,
            3.0,
            &[(-40.0, 0.0), (40.0, 0.0), (0.0, -40.0), (0.0, 40.0)],
            (-56.0, -56.0, 56.0, 56.0),
            true,
            true,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        ),
    ]);
}

#[test]
fn authored_dem_junction_edit_order_does_not_change_terrain_cdt_output() {
    let center = Vector2::new(0.0, 0.0);
    let first = standard_node_case(
        "raised four-way junction over authored ridge-valley terrain",
        authored_ridge_valley_terrain(181, 181, 1.0),
        center,
        3.0,
        &[(-40.0, 0.0), (40.0, 0.0), (0.0, -40.0), (0.0, 40.0)],
        (-56.0, -56.0, 56.0, 56.0),
        true,
        true,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );
    let reordered = standard_node_case(
        "raised four-way junction over authored ridge-valley terrain with reordered edits",
        authored_ridge_valley_terrain(181, 181, 1.0),
        center,
        3.0,
        &[(0.0, 40.0), (0.0, -40.0), (40.0, 0.0), (-40.0, 0.0)],
        (-56.0, -56.0, 56.0, 56.0),
        true,
        true,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    let first_mesh = assert_authored_dem_case(first);
    let reordered_mesh = assert_authored_dem_case(reordered);
    assert_eq!(
        first_mesh.stats, reordered_mesh.stats,
        "reordered authored-road edits changed terrain-CDT diagnostics"
    );
    assert_eq!(
        canonical_emitted_face_set(&first_mesh),
        canonical_emitted_face_set(&reordered_mesh),
        "reordered authored-road edits changed emitted terrain topology"
    );
}

struct AuthoredDemCase {
    name: &'static str,
    terrain: TerrainSystem,
    graph: RegionGraph,
    surface: RoadSurfaceSystem,
    bounds: (f32, f32, f32, f32),
    sample_step_m: f32,
    expect_retaining_wall: bool,
    expect_widened_tie_in: bool,
    expected_node_piece: Option<(u32, RoadSurfaceVisualNodePieceKind)>,
}

fn assert_authored_dem_cases(cases: Vec<AuthoredDemCase>) {
    for case in cases {
        assert_authored_dem_case(case);
    }
}

fn assert_authored_dem_case(case: AuthoredDemCase) -> TerrainCdtMesh {
    if let Some((node_id, expected_kind)) = case.expected_node_piece {
        assert_eq!(
            case.surface
                .compiled_visual_node_pieces()
                .get(&node_id)
                .unwrap_or_else(|| panic!("{}: expected compiled node piece", case.name))
                .kind,
            expected_kind,
            "{}: compiled node piece kind changed",
            case.name
        );
    }
    assert_production_authored_dem_cdt_contract(
        case.name,
        &case.surface,
        &case.graph,
        &case.terrain,
        case.bounds,
        case.sample_step_m,
        case.expect_retaining_wall,
        case.expect_widened_tie_in,
    )
}

fn standard_span_case(
    name: &'static str,
    terrain: TerrainSystem,
    start_xz: Vector2,
    end_xz: Vector2,
    road_height_offset_m: f32,
    segments: usize,
    bounds: (f32, f32, f32, f32),
    expect_retaining_wall: bool,
    expect_widened_tie_in: bool,
) -> AuthoredDemCase {
    let (graph, surface) = compile_standard_span_on_terrain(
        &terrain,
        start_xz,
        end_xz,
        road_height_offset_m,
        segments,
    );
    AuthoredDemCase {
        name,
        terrain,
        graph,
        surface,
        bounds,
        sample_step_m: 2.0,
        expect_retaining_wall,
        expect_widened_tie_in,
        expected_node_piece: None,
    }
}

fn standard_node_case(
    name: &'static str,
    terrain: TerrainSystem,
    center_xz: Vector2,
    road_height_offset_m: f32,
    endpoint_offsets: &[(f32, f32)],
    bounds: (f32, f32, f32, f32),
    expect_retaining_wall: bool,
    expect_widened_tie_in: bool,
    expected_node_kind: RoadSurfaceVisualNodePieceKind,
) -> AuthoredDemCase {
    let (graph, center_node) = standard_node_graph_with_offset_roads(
        &terrain,
        center_xz,
        road_height_offset_m,
        endpoint_offsets,
    );
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    AuthoredDemCase {
        name,
        terrain,
        graph,
        surface,
        bounds,
        sample_step_m: 2.0,
        expect_retaining_wall,
        expect_widened_tie_in,
        expected_node_piece: Some((center_node, expected_node_kind)),
    }
}

fn compile_standard_span_on_terrain(
    terrain: &TerrainSystem,
    start_xz: Vector2,
    end_xz: Vector2,
    road_height_offset_m: f32,
    segments: usize,
) -> (RegionGraph, RoadSurfaceSystem) {
    let (graph, surface, _, _) = compile_standard_span_with_nodes_on_terrain(
        terrain,
        start_xz,
        end_xz,
        road_height_offset_m,
        segments,
    );
    (graph, surface)
}

fn compile_standard_span_with_nodes_on_terrain(
    terrain: &TerrainSystem,
    start_xz: Vector2,
    end_xz: Vector2,
    road_height_offset_m: f32,
    segments: usize,
) -> (RegionGraph, RoadSurfaceSystem, u32, u32) {
    let mut points = grounded_polyline_points_from_terrain(terrain, start_xz, end_xz, segments);
    for point in &mut points {
        point.y += road_height_offset_m;
    }

    let mut graph = RegionGraph::new();
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        points,
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, terrain);
    (graph, surface, start, end)
}

fn standard_node_graph_with_offset_roads(
    terrain: &TerrainSystem,
    center_xz: Vector2,
    road_height_offset_m: f32,
    endpoint_offsets: &[(f32, f32)],
) -> (RegionGraph, u32) {
    let center_pos = terrain_offset_point(terrain, center_xz, road_height_offset_m);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(center_pos, NodeType::Junction);

    for &(offset_x, offset_z) in endpoint_offsets {
        let endpoint_xz = Vector2::new(center_xz.x + offset_x, center_xz.y + offset_z);
        let endpoint = terrain_offset_point(terrain, endpoint_xz, road_height_offset_m);
        let endpoint_node = graph.add_node(endpoint, NodeType::Junction);
        let (start, end, points) = if offset_x < 0.0 || (offset_x == 0.0 && offset_z < 0.0) {
            (endpoint_node, center, vec![endpoint, center_pos])
        } else {
            (center, endpoint_node, vec![center_pos, endpoint])
        };
        graph.add_edge(test_edge(
            start,
            end,
            points,
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    let adaptable_edges = (0..graph.edge_count()).collect::<HashSet<_>>();
    graph.solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &adaptable_edges);
    graph.rebuild_intersection_clips();

    (graph, center)
}

fn terrain_offset_point(
    terrain: &TerrainSystem,
    xz: Vector2,
    road_height_offset_m: f32,
) -> Vector3 {
    Vector3::new(
        xz.x,
        terrain_height_m(terrain, xz.x, xz.y) + road_height_offset_m,
        xz.y,
    )
}

fn assert_production_authored_dem_cdt_contract(
    case_name: &str,
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    terrain: &TerrainSystem,
    bounds: (f32, f32, f32, f32),
    sample_step_m: f32,
    expect_retaining_wall: bool,
    expect_widened_tie_in: bool,
) -> TerrainCdtMesh {
    let (min_x, min_z, max_x, max_z) = bounds;
    let (road_loops, source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(graph, min_x, min_z, max_x, max_z)
        .unwrap_or_else(|err| panic!("{case_name}: terrain clip export failed: {err:?}"));
    assert!(
        !road_loops.is_empty(),
        "{case_name}: expected production road-owned terrain loops"
    );
    assert!(
        source_count
            >= road_loops
                .iter()
                .filter(|road_loop| !road_loop.is_hole)
                .count(),
        "{case_name}: raw road-owned footprint contributors must be tracked"
    );
    for source_edge in road_loops
        .iter()
        .flat_map(|road_loop| road_loop.source_edges.iter())
    {
        assert_surface_cdt_boundary_source(case_name, source_edge.source);
    }

    let input = terrain_cdt_input_for_bounds(
        terrain,
        road_loops.clone(),
        min_x,
        min_z,
        max_x,
        max_z,
        sample_step_m,
    );
    let mesh = build_road_touched_terrain_patch(input.clone())
        .unwrap_or_else(|err| panic!("{case_name}: terrain CDT should build: {err:?}"));
    let mut reversed_input = input;
    reversed_input.source_samples.reverse();
    let reversed_mesh = build_road_touched_terrain_patch(reversed_input)
        .unwrap_or_else(|err| panic!("{case_name}: reversed terrain CDT should build: {err:?}"));

    assert_eq!(
        mesh.stats, reversed_mesh.stats,
        "{case_name}: source sample order changed terrain-CDT diagnostics"
    );
    assert_eq!(
        canonical_emitted_face_set(&mesh),
        canonical_emitted_face_set(&reversed_mesh),
        "{case_name}: source sample order changed emitted terrain topology"
    );
    assert_eq!(
        mesh.stats.invalid_constraint_edges, 0,
        "{case_name}: authored DEM must not invalidate road-owned seam constraints"
    );
    assert_eq!(
        mesh.stats.preserved_road_constraint_edges, mesh.stats.road_constraint_edges,
        "{case_name}: every road seam constraint must survive terrain CDT"
    );
    assert_eq!(
        mesh.stats.accepted_faces,
        mesh.triangles.len() + mesh.retaining_wall_triangles.len(),
        "{case_name}: accepted faces must be fully classified"
    );
    assert_eq!(
        mesh.emitted_faces.len(),
        mesh.stats.accepted_faces,
        "{case_name}: first-class emitted face provenance must cover accepted faces"
    );
    assert_eq!(
        mesh.terrain_triangle_sources.len(),
        mesh.triangles.len(),
        "{case_name}: terrain face sidecars must match terrain triangles"
    );
    assert_eq!(
        mesh.retaining_wall_triangle_sources.len(),
        mesh.retaining_wall_triangles.len(),
        "{case_name}: retaining-wall sidecars must match retaining-wall triangles"
    );
    assert_eq!(
        mesh.stats.blocking_degenerate_seam_edges, 0,
        "{case_name}: unresolved seam fragments must not reach Spade"
    );
    assert_eq!(
        mesh.stats.omitted_near_seam_source_samples, mesh.stats.tie_in_widened_source_samples,
        "{case_name}: omitted near-seam samples must be reported as widened tie-ins"
    );
    assert!(
        mesh.stats.road_seam_faces > 0,
        "{case_name}: production terrain CDT should report road-seam faces"
    );
    assert!(
        mesh.road_seam_face_samples
            .iter()
            .all(|sample| !sample.sources.is_empty()),
        "{case_name}: road-seam diagnostics must carry source provenance"
    );
    assert!(
        mesh.retaining_wall_face_samples
            .iter()
            .all(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall
                && !sample.sources.is_empty()),
        "{case_name}: retaining-wall diagnostics must carry source provenance"
    );
    assert!(
        mesh.retaining_wall_triangle_sources
            .iter()
            .all(|sources| !sources.is_empty()),
        "{case_name}: emitted retaining-wall faces must carry source provenance"
    );
    assert!(
        mesh.emitted_faces.iter().all(|face| {
            face.kind != TerrainCdtTieInKind::RetainingWall || !face.sources.is_empty()
        }),
        "{case_name}: first-class retaining-wall faces must not be anonymous"
    );

    if expect_retaining_wall {
        assert!(
            mesh.stats.retaining_wall_faces > 0,
            "{case_name}: expected explicit retaining-wall tie-ins"
        );
    } else {
        assert_eq!(
            mesh.stats.retaining_wall_faces, 0,
            "{case_name}: ordinary authored terrain must not emit retaining-wall faces"
        );
        assert!(
            mesh.retaining_wall_triangles.is_empty(),
            "{case_name}: ordinary authored terrain must not emit retaining-wall topology"
        );
    }
    if expect_widened_tie_in {
        assert!(
            mesh.stats.tie_in_widened_source_samples > 0,
            "{case_name}: expected near-road source samples to widen over-steep tie-ins"
        );
        assert!(
            mesh.tie_in_widened_samples
                .iter()
                .all(|sample| sample.required_distance_m > sample.distance_m
                    && !sample.seam_source.debug_label().is_empty()),
            "{case_name}: widened tie-in samples must preserve sourced seam evidence"
        );
    }

    assert_cdt_mesh_stays_outside_road_loops(case_name, &mesh, &road_loops);
    assert_cdt_mesh_sources_are_structured(case_name, &mesh);
    mesh
}

fn canonical_emitted_face_set(mesh: &TerrainCdtMesh) -> Vec<(i32, [(i64, i64, i64); 3])> {
    let mut faces = mesh
        .emitted_faces
        .iter()
        .map(|face| {
            let mut vertices = face
                .triangle
                .map(|index| canonical_terrain_vertex(mesh, index));
            vertices.sort_unstable();
            (face.kind.debug_code(), vertices)
        })
        .collect::<Vec<_>>();
    faces.sort_unstable();
    faces
}

fn canonical_terrain_vertex(mesh: &TerrainCdtMesh, index: usize) -> (i64, i64, i64) {
    let vertex = mesh.vertices[index];
    (
        (vertex.x * 1000.0).round() as i64,
        (f64::from(vertex.height_m) * 1000.0).round() as i64,
        (vertex.z * 1000.0).round() as i64,
    )
}

fn authored_ridge_valley_terrain(width: usize, height: usize, cell_size_m: f32) -> TerrainSystem {
    let mut terrain = TerrainSystem::with_chunking(width, height, cell_size_m, 8, 0.0);
    for z in 0..height {
        for x in 0..width {
            let (world_x, world_z) = terrain.grid_to_world_coords(x, z);
            let ridge_dx = world_x - 8.0;
            let valley_dz = world_z + 18.0;
            let ridge = 4.5 * (-(ridge_dx * ridge_dx) / (2.0 * 8.0 * 8.0)).exp();
            let valley = -3.0 * (-(valley_dz * valley_dz) / (2.0 * 10.0 * 10.0)).exp();
            let base = 7.0 + world_x * 0.02 - world_z * 0.015;
            terrain.set_height(x, z, (base + ridge + valley) / crate::config::HEIGHT_SCALE);
        }
    }
    terrain
}
