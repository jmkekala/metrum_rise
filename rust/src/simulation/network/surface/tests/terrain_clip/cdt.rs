//! Road-touched terrain CDT tests.

use super::*;

#[test]
fn surface_terrain_cdt_authored_piece_matrix_preserves_owned_sources() {
    {
        let terrain = coarse_hillside_world_terrain(161, 161, 1.0);
        let points = grounded_polyline_points_from_terrain(
            &terrain,
            Vector2::new(-40.0, -24.0),
            Vector2::new(40.0, -24.0),
            20,
        );
        let mut graph = RegionGraph::new();
        let start = graph.add_node(points[0], NodeType::Junction);
        let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
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
        surface.compile_dirty(&graph, &terrain);
        assert!(
            surface
                .compiled_visual_span_pieces()
                .contains_key(&edge_idx)
        );
        assert_eq!(
            surface
                .compiled_visual_node_pieces()
                .values()
                .filter(|piece| piece.kind == RoadSurfaceVisualNodePieceKind::Terminal)
                .count(),
            2,
            "straight road should cover span plus both terminal footprints"
        );
        assert_terrain_clip_loops_cover_node_top_footprint_bounds(
            "straight terminal footprint terrain clip coverage",
            &surface,
            &graph,
            (-56.0, -44.0, 56.0, 4.0),
        );
        assert_surface_terrain_cdt_contract(
            "straight span with terminal footprints on authored hillside",
            &surface,
            &graph,
            &terrain,
            (-56.0, -44.0, 56.0, 4.0),
            false,
        );
    }

    {
        let terrain = flat_terrain(161, 161);
        let center_xz = Vector2::new(0.0, 0.0);
        let west_xz = Vector2::new(-36.0, 0.0);
        let north_xz = Vector2::new(0.0, 36.0);
        let center_pos = Vector3::new(
            center_xz.x,
            terrain_height_m(&terrain, center_xz.x, center_xz.y),
            center_xz.y,
        );
        let west_points = grounded_polyline_points_from_terrain(&terrain, west_xz, center_xz, 12);
        let north_points = grounded_polyline_points_from_terrain(&terrain, center_xz, north_xz, 12);
        let mut graph = RegionGraph::new();
        let west = graph.add_node(west_points[0], NodeType::Junction);
        let center = graph.add_node(center_pos, NodeType::Junction);
        let north = graph.add_node(*north_points.last().unwrap(), NodeType::Junction);
        graph.add_edge(test_edge(
            west,
            center,
            west_points,
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.add_edge(test_edge(
            center,
            north,
            north_points,
            10.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        assert_eq!(
            surface
                .compiled_visual_node_pieces()
                .get(&center)
                .unwrap()
                .kind,
            RoadSurfaceVisualNodePieceKind::Bend
        );
        assert_surface_terrain_cdt_contract(
            "bend footprint",
            &surface,
            &graph,
            &terrain,
            (-52.0, -16.0, 16.0, 52.0),
            false,
        );
    }

    {
        let terrain = flat_terrain(129, 129);
        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        for endpoint in [
            Vector3::new(-40.0, 0.0, 0.0),
            Vector3::new(40.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 40.0),
        ] {
            let node = graph.add_node(endpoint, NodeType::Junction);
            let (start, end, points) = if endpoint.x < 0.0 {
                (node, center, vec![endpoint, Vector3::new(0.0, 0.0, 0.0)])
            } else {
                (center, node, vec![Vector3::new(0.0, 0.0, 0.0), endpoint])
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
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        assert_eq!(
            surface
                .compiled_visual_node_pieces()
                .get(&center)
                .unwrap()
                .kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_surface_terrain_cdt_contract(
            "flat JunctionN footprint",
            &surface,
            &graph,
            &terrain,
            (-56.0, -24.0, 56.0, 56.0),
            false,
        );
    }

    {
        let terrain = flat_terrain(129, 129);
        let road_y = 3.0;
        let center_pos = Vector3::new(0.0, road_y, 0.0);
        let mut graph = RegionGraph::new();
        let center = graph.add_node(center_pos, NodeType::Junction);
        for endpoint in [
            Vector3::new(-40.0, road_y, 0.0),
            Vector3::new(40.0, road_y, 0.0),
            Vector3::new(0.0, road_y, 40.0),
        ] {
            let node = graph.add_node(endpoint, NodeType::Junction);
            let (start, end, points) = if endpoint.x < 0.0 {
                (node, center, vec![endpoint, center_pos])
            } else {
                (center, node, vec![center_pos, endpoint])
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
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        assert_eq!(
            surface
                .compiled_visual_node_pieces()
                .get(&center)
                .unwrap()
                .kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_surface_terrain_cdt_contract(
            "elevated Standard JunctionN footprint over flat authored terrain",
            &surface,
            &graph,
            &terrain,
            (-56.0, -24.0, 56.0, 56.0),
            false,
        );
    }
}

fn assert_terrain_clip_loops_cover_node_top_footprint_bounds(
    case_name: &str,
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    bounds: (f32, f32, f32, f32),
) {
    let (min_x, min_z, max_x, max_z) = bounds;
    let (road_loops, _) = surface
        .terrain_cdt_road_loops_for_world_bounds(graph, min_x, min_z, max_x, max_z)
        .unwrap_or_else(|err| panic!("{case_name}: terrain clip export failed: {err:?}"));
    let clip_bounds = xz_bounds(
        road_loops
            .iter()
            .flat_map(|road_loop| road_loop.vertices.iter())
            .map(|vertex| (vertex.x, vertex.z)),
    )
    .expect("terrain clip should export road loops");
    let top_bounds = xz_bounds(
        surface
            .compiled_visual_node_pieces()
            .values()
            .filter(|piece| piece.kind == RoadSurfaceVisualNodePieceKind::Terminal)
            .flat_map(|piece| {
                piece
                    .road_surface_polygons
                    .iter()
                    .chain(piece.curb_surface_polygons.iter())
                    .chain(piece.sidewalk_surface_polygons.iter())
            })
            .flat_map(|polygon| polygon.points_world.iter())
            .map(|point| (point.x, point.z)),
    )
    .expect("straight road should have terminal top polygons");
    let raw_terminal_clip_bounds = xz_bounds(
        surface
            .compiled_visual_node_pieces()
            .values()
            .filter(|piece| piece.kind == RoadSurfaceVisualNodePieceKind::Terminal)
            .flat_map(|piece| piece.terrain_clip_boundary_loops.iter())
            .flat_map(|boundary_loop| boundary_loop.points_world.iter())
            .map(|point| (point.x, point.z)),
    )
    .expect("straight road should have terminal terrain clip loops");
    let epsilon = 0.001;
    assert!(
        clip_bounds.0 <= top_bounds.0 + epsilon
            && clip_bounds.1 <= top_bounds.1 + epsilon
            && clip_bounds.2 >= top_bounds.2 - epsilon
            && clip_bounds.3 >= top_bounds.3 - epsilon,
        "{case_name}: terrain clip bounds {clip_bounds:?} must cover terminal top bounds {top_bounds:?}; raw terminal clip bounds {raw_terminal_clip_bounds:?}"
    );
}

fn xz_bounds(points: impl IntoIterator<Item = (f64, f64)>) -> Option<(f64, f64, f64, f64)> {
    points.into_iter().fold(None, |bounds, (x, z)| {
        Some(match bounds {
            Some((min_x, min_z, max_x, max_z)) => {
                (min_x.min(x), min_z.min(z), max_x.max(x), max_z.max(z))
            }
            None => (x, z, x, z),
        })
    })
}

#[test]
fn terrain_cdt_preserves_source_samples_inside_road_hole() {
    let outer = vec![
        TerrainCdtVertex::new(0.0, 0.0, 0.0),
        TerrainCdtVertex::new(4.0, 0.0, 0.0),
        TerrainCdtVertex::new(4.0, 0.0, 4.0),
        TerrainCdtVertex::new(0.0, 0.0, 4.0),
    ];
    let hole = vec![
        TerrainCdtVertex::new(1.0, 0.0, 1.0),
        TerrainCdtVertex::new(1.0, 0.0, 3.0),
        TerrainCdtVertex::new(3.0, 0.0, 3.0),
        TerrainCdtVertex::new(3.0, 0.0, 1.0),
    ];
    let hole_sample = TerrainCdtVertex::new(2.0, 0.0, 2.0);

    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        TerrainCdtPatch::new(-1.0, -1.0, 5.0, 5.0, [0.0; 4]),
        vec![
            TerrainCdtRoadLoop::new_with_source_edges_and_topology(
                10,
                10,
                0,
                false,
                outer,
                Vec::new(),
            ),
            TerrainCdtRoadLoop::new_with_source_edges_and_topology(
                11,
                10,
                1,
                true,
                hole,
                Vec::new(),
            ),
        ],
        vec![hole_sample],
    ))
    .expect("CDT should accept road footprint holes as constrained terrain islands");

    assert!(
        mesh.vertices
            .iter()
            .any(
                |vertex| (vertex.x - hole_sample.x).abs() <= f64::from(SAMPLE_EPSILON_M)
                    && (vertex.z - hole_sample.z).abs() <= f64::from(SAMPLE_EPSILON_M)
            ),
        "source terrain sample inside a road footprint hole must not be rejected as road-owned"
    );
}

#[test]
fn terrain_clip_loops_are_unioned_before_cdt_for_arbitrary_multiway_nodes() {
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

    let (road_loops, _) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -96.0, -96.0, 96.0, 96.0)
        .expect("arbitrary multiway terrain clip export should succeed");
    assert!(
        !road_loops.is_empty(),
        "expected arbitrary multiway node to produce terrain clip loops"
    );

    let mesh = build_road_touched_terrain_patch(terrain_cdt_input_for_bounds(
        &terrain, road_loops, -96.0, -96.0, 96.0, 96.0, 8.0,
    ))
    .expect("unioned terrain clip footprint must be accepted by the terrain CDT");

    assert_eq!(
        mesh.stats.invalid_constraint_edges, 0,
        "terrain CDT must not see crossing constraints from arbitrary-angle piece loops"
    );
}

#[test]
fn terrain_cdt_grading_envelope_constrains_single_convex_footprint() {
    let terrain = flat_terrain(65, 65);
    let road_loop = TerrainCdtRoadLoop::new(
        10,
        0,
        vec![
            TerrainCdtVertex::new(-8.0, 0.0, -4.0),
            TerrainCdtVertex::new(8.0, 0.0, -4.0),
            TerrainCdtVertex::new(8.0, 0.0, 4.0),
            TerrainCdtVertex::new(-8.0, 0.0, 4.0),
        ],
    );
    let mut guide_samples = Vec::new();
    let mut guide_constraints = Vec::new();
    let mut sample_keys = BTreeMap::new();

    RoadSurfaceSystem::append_terrain_cdt_roadbed_grading_envelope(
        &terrain,
        &[road_loop],
        4.0,
        &mut guide_samples,
        &mut guide_constraints,
        &mut sample_keys,
    );

    assert!(
        !guide_samples.is_empty(),
        "single convex footprints should emit grade-limited terrain guide samples"
    );
    assert!(
        !guide_constraints.is_empty(),
        "single convex footprints should constrain guide rails between adjacent seam samples"
    );
}

#[test]
fn terrain_cdt_grading_envelope_leaves_concave_junction_rails_unconstrained() {
    let terrain = flat_terrain(65, 65);
    let road_loop = TerrainCdtRoadLoop::new(
        11,
        0,
        vec![
            TerrainCdtVertex::new(-8.0, 0.0, -8.0),
            TerrainCdtVertex::new(8.0, 0.0, -8.0),
            TerrainCdtVertex::new(8.0, 0.0, 0.0),
            TerrainCdtVertex::new(2.0, 0.0, 0.0),
            TerrainCdtVertex::new(2.0, 0.0, 8.0),
            TerrainCdtVertex::new(-8.0, 0.0, 8.0),
        ],
    );
    let mut guide_samples = Vec::new();
    let mut guide_constraints = Vec::new();
    let mut sample_keys = BTreeMap::new();

    RoadSurfaceSystem::append_terrain_cdt_roadbed_grading_envelope(
        &terrain,
        &[road_loop],
        4.0,
        &mut guide_samples,
        &mut guide_constraints,
        &mut sample_keys,
    );

    assert!(
        !guide_samples.is_empty(),
        "concave junction-style footprints still need grade-limited terrain guide samples"
    );
    assert!(
        guide_constraints.is_empty(),
        "concave junction-style footprints must not add guide rail constraints that can cross the final roadbed footprint"
    );
}

#[test]
fn terrain_cdt_grading_envelope_leaves_hole_footprint_sets_unconstrained() {
    let terrain = flat_terrain(65, 65);
    let outer_loop = TerrainCdtRoadLoop::new(
        12,
        0,
        vec![
            TerrainCdtVertex::new(-8.0, 0.0, -8.0),
            TerrainCdtVertex::new(8.0, 0.0, -8.0),
            TerrainCdtVertex::new(8.0, 0.0, 8.0),
            TerrainCdtVertex::new(-8.0, 0.0, 8.0),
        ],
    );
    let hole_loop = TerrainCdtRoadLoop::new_with_source_edges_and_topology(
        13,
        12,
        1,
        true,
        vec![
            TerrainCdtVertex::new(-2.0, 0.0, -2.0),
            TerrainCdtVertex::new(-2.0, 0.0, 2.0),
            TerrainCdtVertex::new(2.0, 0.0, 2.0),
            TerrainCdtVertex::new(2.0, 0.0, -2.0),
        ],
        Vec::new(),
    );
    let mut guide_samples = Vec::new();
    let mut guide_constraints = Vec::new();
    let mut sample_keys = BTreeMap::new();

    RoadSurfaceSystem::append_terrain_cdt_roadbed_grading_envelope(
        &terrain,
        &[outer_loop, hole_loop],
        4.0,
        &mut guide_samples,
        &mut guide_constraints,
        &mut sample_keys,
    );

    assert!(
        !guide_samples.is_empty(),
        "outer footprint still needs grade-limited guide samples"
    );
    assert!(
        guide_constraints.is_empty(),
        "footprint sets with holes must stay sample-only"
    );
}

#[test]
fn terrain_cdt_grading_envelope_ignores_building_site_loops() {
    let terrain = flat_terrain(65, 65);
    let road_loop = TerrainCdtRoadLoop::new(
        14,
        0,
        vec![
            TerrainCdtVertex::new(-8.0, 0.0, -4.0),
            TerrainCdtVertex::new(8.0, 0.0, -4.0),
            TerrainCdtVertex::new(8.0, 0.0, 4.0),
            TerrainCdtVertex::new(-8.0, 0.0, 4.0),
        ],
    );
    let site_loop = building_site_terrain_cdt_loop(
        15,
        vec![
            TerrainCdtVertex::new(16.0, 0.0, -4.0),
            TerrainCdtVertex::new(24.0, 0.0, -4.0),
            TerrainCdtVertex::new(24.0, 0.0, 4.0),
            TerrainCdtVertex::new(16.0, 0.0, 4.0),
        ],
    );
    let mut road_only_samples = Vec::new();
    let mut road_only_constraints = Vec::new();
    let mut road_only_keys = BTreeMap::new();
    RoadSurfaceSystem::append_terrain_cdt_roadbed_grading_envelope(
        &terrain,
        &[road_loop.clone()],
        4.0,
        &mut road_only_samples,
        &mut road_only_constraints,
        &mut road_only_keys,
    );

    let mut mixed_samples = Vec::new();
    let mut mixed_constraints = Vec::new();
    let mut mixed_keys = BTreeMap::new();
    RoadSurfaceSystem::append_terrain_cdt_roadbed_grading_envelope(
        &terrain,
        &[road_loop, site_loop],
        4.0,
        &mut mixed_samples,
        &mut mixed_constraints,
        &mut mixed_keys,
    );

    assert_eq!(
        mixed_samples, road_only_samples,
        "building-site loops must not contribute roadbed grading samples"
    );
    assert!(
        mixed_constraints.is_empty(),
        "mixed road/site footprint sets must not constrain roadbed guide rails"
    );
}

fn building_site_terrain_cdt_loop(
    stable_piece_id: u64,
    vertices: Vec<TerrainCdtVertex>,
) -> TerrainCdtRoadLoop {
    let source_edges = vertices
        .iter()
        .copied()
        .enumerate()
        .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
            start,
            end: vertices[(index + 1) % vertices.len()],
            source: TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                building_idx: stable_piece_id,
                local_loop_index: 0,
                local_edge_index: u32::try_from(index).unwrap_or(u32::MAX),
            },
        })
        .collect();
    TerrainCdtRoadLoop::new_with_source_edges(stable_piece_id, 0, vertices, source_edges)
}

#[test]
fn surface_terrain_cdt_skips_bridge_and_tunnel_midspan_support() {
    for (case_name, edge_class, points) in [
        (
            "bridge structural span",
            EdgeClass::Bridge,
            vec![
                Vector3::new(-24.0, 6.0, 0.0),
                Vector3::new(0.0, 6.0, 0.0),
                Vector3::new(24.0, 6.0, 0.0),
            ],
        ),
        (
            "tunnel visible portals",
            EdgeClass::Tunnel,
            vec![
                Vector3::new(-24.0, 0.0, 0.0),
                Vector3::new(-10.0, -6.0, 0.0),
                Vector3::new(10.0, -6.0, 0.0),
                Vector3::new(24.0, 0.0, 0.0),
            ],
        ),
    ] {
        let terrain = flat_terrain(97, 97);
        let mut graph = RegionGraph::new();
        let start = graph.add_node(points[0], NodeType::Junction);
        let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            points,
            10.0,
            edge_class,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        graph.rebuild_intersection_clips();

        let mut surface = RoadSurfaceSystem::new(16.0);
        surface.compile_dirty(&graph, &terrain);
        let span_piece = surface
            .compiled_visual_span_pieces()
            .get(&edge_idx)
            .unwrap_or_else(|| panic!("{case_name}: span should compile"));
        if edge_class == EdgeClass::Bridge {
            assert!(span_piece.span_earthwork_support_regions.is_empty());
            assert!(span_piece.render_earthwork_faces.is_empty());
        } else {
            assert!(!span_piece.span_earthwork_support_regions.is_empty());
            assert_span_earthwork_faces_have_support_provenance(span_piece, edge_idx, edge_class);
            assert!(
                span_piece
                    .span_earthwork_support_regions
                    .iter()
                    .all(|region| !(region.start_s_m < 24.0 && region.end_s_m > 24.0)),
                "{case_name}: support regions must stay out of the midspan"
            );
        }
        let (road_loops, source_count) = surface
            .terrain_cdt_road_loops_for_world_bounds(&graph, -8.0, -12.0, 8.0, 12.0)
            .expect("bridge/tunnel midspan query should not fail terrain clip export");
        assert!(
            road_loops.is_empty() && source_count == 0,
            "{case_name}: bridge/tunnel midspans must not feed road-touched terrain CDT"
        );
    }
}
