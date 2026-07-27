//! Temporary road preview regression tests.

use super::*;
use crate::simulation::network::surface::RoadSurfaceCompileReason;

#[test]
fn visual_polygon_builder_preserves_skinny_closure_geometry() {
    let polygon = RoadSurfaceSystem::make_visual_polygon(vec![
        backend::RoadVec3::new(0.0, 0.0, 0.0),
        backend::RoadVec3::new(0.15, 0.0, 0.0),
        backend::RoadVec3::new(0.0, 0.0, 0.02),
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
    assert_preview_vertices_use_solved_section_height_keys(&preview);
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
fn preview_keeps_terrain_to_raised_bridge_ramp_height() {
    let terrain = flat_terrain(96, 16);
    let surface = RoadSurfaceSystem::new(16.0);
    let raw_points = vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(32.0, 2.5, 0.0)];

    let (preview, committed_sections, committed_visual_pieces) =
        compile_committed_preview_reference(&surface, &raw_points, &terrain, 1, 1);

    assert_eq!(preview.edge_class, EdgeClass::Bridge);
    assert!(preview.is_valid);
    assert!((preview.prepared_points[0].y - raw_points[0].y).abs() <= 0.001);
    assert!((preview.prepared_points[1].y - raw_points[1].y).abs() <= 0.001);
    assert_eq!(preview.compiled_sections, committed_sections);
    assert_eq!(preview.compiled_visual_node_pieces, committed_visual_pieces);
}

#[test]
fn elevated_bridge_terminal_to_ground_stays_a_structural_ramp_in_both_directions() {
    let terrain = flat_terrain(128, 64);
    let mut graph = RegionGraph::new();
    let bridge_start_pos = Vector3::new(-32.0, 5.0, 0.0);
    let bridge_terminal_pos = Vector3::new(0.0, 5.0, 0.0);
    let bridge_start = graph.add_node(bridge_start_pos, NodeType::Junction);
    let bridge_terminal = graph.add_node(bridge_terminal_pos, NodeType::Junction);
    graph.add_edge(test_edge(
        bridge_start,
        bridge_terminal,
        vec![bridge_start_pos, bridge_terminal_pos],
        7.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut existing_surface = RoadSurfaceSystem::new(16.0);
    existing_surface.compile_dirty(&graph, &terrain);
    let preview_surface = RoadSurfaceSystem::new(16.0);
    let ground_pos = Vector3::new(32.0, 0.0, 0.0);

    for raw_points in [
        vec![bridge_terminal_pos, ground_pos],
        vec![ground_pos, bridge_terminal_pos],
    ] {
        let preview = preview_surface.compile_preview_surface_mesh_only_with_existing_surface(
            &raw_points,
            1,
            1,
            &terrain,
            &graph,
            &existing_surface,
        );

        assert_eq!(
            preview.edge_class,
            EdgeClass::Bridge,
            "a grounded continuation of an elevated bridge terminal must remain structural"
        );
        assert!(
            preview.is_valid,
            "the structural bridge ramp must pass the same grade/clearance validation: {:?}",
            preview.validation
        );
        assert!(
            preview.prepared_points.len() > 2,
            "the bridge-to-ground transition must keep the dense grade profile"
        );
        let midpoint = preview.prepared_points[preview.prepared_points.len() / 2];
        assert!(
            midpoint.y > 1.5,
            "the structural ramp must descend across its full run instead of following terrain early: midpoint={midpoint:?}"
        );
    }
}

#[test]
fn dense_bridge_ground_transition_may_enter_the_clearance_zone_without_becoming_invalid() {
    let terrain = flat_terrain(64, 32);
    let surface = RoadSurfaceSystem::new(16.0);
    let points = vec![
        Vector3::new(-12.0, 1.5, 0.0),
        Vector3::new(0.0, 0.75, 0.0),
        Vector3::new(12.0, 0.0, 0.0),
    ];

    let validation =
        surface.validate_prepared_road_surface(&points, EdgeClass::Bridge, 1, 1, &terrain);

    assert!(
        validation.is_valid,
        "a bridge approach must be allowed to descend through the ordinary midspan clearance zone: {validation:?}"
    );
}

#[test]
fn dense_bridge_ground_transition_rejects_a_deck_buried_below_terrain() {
    let terrain = flat_terrain(96, 32);
    let surface = RoadSurfaceSystem::new(16.0);
    let points = vec![
        Vector3::new(-30.0, 3.0, 0.0),
        Vector3::new(0.0, -0.2, 0.0),
        Vector3::new(30.0, 0.0, 0.0),
    ];

    let validation =
        surface.validate_prepared_road_surface(&points, EdgeClass::Bridge, 1, 1, &terrain);

    assert!(!validation.is_valid);
    assert_eq!(validation.invalid_reason, "bridge_clearance");
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
fn preview_conditioning_preserves_snapped_visible_road_height() {
    let terrain = flat_terrain(96, 64);
    let mut graph = RegionGraph::new();
    let existing_y = 5.0;
    let start = graph.add_node(Vector3::new(0.0, existing_y, -16.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, existing_y, 16.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, existing_y, -16.0),
            Vector3::new(0.0, existing_y, 16.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut existing_surface = RoadSurfaceSystem::new(16.0);
    existing_surface.compile_dirty(&graph, &terrain);
    let visible_y = existing_surface
        .sample_visible_surface_height(&graph, &terrain, 0.0, 0.0)
        .expect("existing elevated standard road must expose visible snap support");
    assert!(
        (visible_y - existing_y).abs() <= 0.05,
        "test setup expected visible support at the elevated road height: visible={visible_y:.3}"
    );

    let raw_points = vec![
        Vector3::new(0.0, visible_y, 0.0),
        Vector3::new(24.0, 0.0, 0.0),
    ];
    let preview_surface = RoadSurfaceSystem::new(16.0);
    let preview = preview_surface.compile_preview_surface_mesh_only_with_existing_surface(
        &raw_points,
        1,
        1,
        &terrain,
        &graph,
        &existing_surface,
    );

    assert_eq!(preview.edge_class, EdgeClass::Standard);
    assert!(
        preview.prepared_points.len() > 2,
        "long preview strokes should store dense solved physical geometry"
    );
    assert!(
        (preview.prepared_points[0].y - visible_y).abs() <= 0.05,
        "snapped endpoint must keep visible road height instead of source terrain: prepared={:.3} visible={visible_y:.3}",
        preview.prepared_points[0].y
    );
    let last = preview.prepared_points.last().unwrap();
    assert!(
        last.y.abs() <= 0.001,
        "non-road endpoint should still ground to source terrain"
    );
}

#[test]
fn preview_accepts_over_limit_standard_grade_and_reports_metrics() {
    let terrain = flat_terrain(96, 64);
    let surface = RoadSurfaceSystem::new(16.0);
    let prepared_points = vec![
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(6.0, 1.2, 0.0),
        Vector3::new(12.0, 2.4, 0.0),
    ];

    let validation = surface.validate_prepared_road_surface(
        &prepared_points,
        EdgeClass::Standard,
        1,
        1,
        &terrain,
    );

    assert!(validation.is_valid);
    assert_eq!(validation.invalid_reason, "");
    assert!(
        validation.max_grade > validation.allowed_grade,
        "over-limit preview should still report grade metrics: max={:.3} allowed={:.3}",
        validation.max_grade,
        validation.allowed_grade
    );
    assert!(validation.offending_span_end_m > validation.offending_span_start_m);
    assert!(validation.offending_span_run_m > 0.0);
    assert!(
        (validation.offending_span_height_delta_m.abs() / validation.offending_span_run_m
            - validation.max_grade)
            .abs()
            <= 0.001
    );
    assert!(
        (validation.offending_span_start_height_m
            - validation.offending_span_start_terrain_height_m
            - validation.offending_span_start_support_delta_m)
            .abs()
            <= 0.001
    );
    assert!(
        (validation.offending_span_end_height_m
            - validation.offending_span_end_terrain_height_m
            - validation.offending_span_end_support_delta_m)
            .abs()
            <= 0.001
    );
}

#[test]
fn preview_rejects_logged_tight_switchback_when_surface_span_cannot_compile() {
    let terrain = flat_terrain(512, 512);
    let existing_points = road_points_from_json(
        "[[222.524506,197.763962,-509.727112],[226.562973,197.436737,-505.481628],\
        [230.601440,197.109985,-501.236115],[234.639908,196.543076,-496.990631],\
        [238.678375,195.935669,-492.745117],[242.716843,195.368607,-488.499634],\
        [246.755310,194.896790,-484.254120],[250.793793,194.398849,-480.008636],\
        [254.832260,193.695709,-475.763123],[258.870728,192.992569,-471.517639],\
        [262.909180,192.289429,-467.272156],[266.947662,191.586288,-463.026642],\
        [270.986145,190.883148,-458.781128],[275.024597,190.180008,-454.535645],\
        [279.063049,189.476868,-450.290161],[283.101532,188.773727,-446.044647],\
        [287.140015,188.070587,-441.799133],[291.178467,187.367447,-437.553650],\
        [295.216949,186.664307,-433.308167],[299.255402,185.961166,-429.062653],\
        [303.293884,185.258026,-424.817169],[307.332336,184.554886,-420.571655],\
        [311.370819,183.851746,-416.326172],[315.409302,183.148605,-412.080688],\
        [319.447754,182.445465,-407.835175]]",
    );
    let new_points = road_points_from_json(
        "[[319.447754,182.445465,-407.835175],[322.132141,182.097412,-405.010742],\
        [324.807831,181.629883,-402.178711],[327.461060,181.162628,-399.328766],\
        [330.070618,180.695709,-396.442627],[332.594910,180.229630,-393.490753],\
        [334.932983,179.767288,-390.428467],[336.836090,179.305298,-387.081879],\
        [336.848785,178.870453,-383.458160],[333.587708,178.417953,-381.564758],\
        [329.690277,177.944839,-380.969635],[325.730927,177.469376,-380.818359],\
        [321.763397,176.993225,-380.877594],[317.796234,176.516678,-381.057587],\
        [313.830933,176.039841,-381.314728],[309.869324,175.563004,-381.623932],\
        [305.909943,175.086060,-381.970154],[301.952759,174.609100,-382.343140],\
        [297.997101,174.132095,-382.735809],[294.042725,173.655060,-383.143219],\
        [290.089233,173.177994,-383.561523],[286.136658,172.700928,-383.987915],\
        [282.184479,172.223831,-384.420227],[278.232880,171.746750,-384.856628],\
        [274.281403,171.484329,-385.295746],[270.330322,171.330429,-385.736572],\
        [266.378937,170.936310,-386.178162]]",
    );

    let mut graph = RegionGraph::new();
    let start = graph.add_node(existing_points[0], NodeType::Junction);
    let bend = graph.add_node(*existing_points.last().unwrap(), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        bend,
        existing_points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let surface = RoadSurfaceSystem::new(16.0);

    let validation = surface.validate_prepared_road_surface_against_graph(
        &new_points,
        EdgeClass::Standard,
        1,
        1,
        &terrain,
        &graph,
    );

    assert!(!validation.is_valid);
    assert_eq!(validation.invalid_reason, "surface_geometry_invalid");
}

#[test]
fn preview_accepts_connected_bend_when_surface_geometry_compiles() {
    let terrain = flat_terrain(96, 96);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        bend,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let surface = RoadSurfaceSystem::new(16.0);
    let prepared_points = vec![Vector3::new(24.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 24.0)];

    let validation = surface.validate_prepared_road_surface_against_graph(
        &prepared_points,
        EdgeClass::Standard,
        1,
        1,
        &terrain,
        &graph,
    );

    assert!(
        validation.is_valid,
        "ordinary connected bends must remain placeable: {validation:?}"
    );
}

#[test]
fn fast_candidate_validation_allows_endpoint_branch_overlap_for_full_validator() {
    let terrain = flat_terrain(96, 96);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let terminal = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        terminal,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut existing_surface = RoadSurfaceSystem::new(16.0);
    existing_surface.compile_dirty(&graph, &terrain);
    let candidate_surface = RoadSurfaceSystem::new(16.0);

    let forward_points = vec![Vector3::new(24.0, 0.0, 0.0), Vector3::new(48.0, 0.0, 0.0)];
    let forward_input = RoadSurfaceSystem::prepare_road_input_with_extension_to_visible_surface(
        &forward_points,
        &terrain,
        &graph,
        &existing_surface,
    );
    let forward_validation = candidate_surface.validate_prepared_road_candidate_fast(
        &forward_input,
        1,
        1,
        &terrain,
        &graph,
    );
    assert!(
        forward_validation.is_valid,
        "straight terminal extensions must remain buildable: {forward_validation:?}"
    );

    let overlap_points = vec![Vector3::new(24.0, 0.0, 0.0), Vector3::new(12.0, 0.0, 0.0)];
    let overlap_input = RoadSurfaceSystem::prepare_road_input_with_extension_to_visible_surface(
        &overlap_points,
        &terrain,
        &graph,
        &existing_surface,
    );
    let overlap_validation = candidate_surface.validate_prepared_road_candidate_fast(
        &overlap_input,
        1,
        1,
        &terrain,
        &graph,
    );
    assert!(
        overlap_validation.is_valid,
        "fast validation must not reject tight endpoint branch angles: {overlap_validation:?}"
    );
}

#[test]
fn fast_candidate_validation_is_not_more_permissive_than_full_surface_validation() {
    let terrain = flat_terrain(160, 160);
    let mut graph = RegionGraph::new();
    let far = graph.add_node(Vector3::new(-48.0, 0.0, 0.0), NodeType::Junction);
    let terminal = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        far,
        terminal,
        vec![Vector3::new(-48.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut existing_surface = RoadSurfaceSystem::new(16.0);
    existing_surface.compile_dirty(&graph, &terrain);
    let candidate_surface = RoadSurfaceSystem::new(16.0);

    for (angle_degrees, fwd_lanes, bkw_lanes) in [
        (18.0_f32, 1_u8, 1_u8),
        (30.0, 1, 1),
        (45.0, 2, 2),
        (90.0, 1, 1),
        (135.0, 2, 2),
    ] {
        let radians = angle_degrees.to_radians();
        let raw_points = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(radians.cos() * 48.0, 0.0, radians.sin() * 48.0),
        ];
        let prepared_input =
            RoadSurfaceSystem::prepare_road_input_with_extension_to_visible_surface(
                &raw_points,
                &terrain,
                &graph,
                &existing_surface,
            );
        let fast_validation = candidate_surface.validate_prepared_road_candidate_fast(
            &prepared_input,
            fwd_lanes,
            bkw_lanes,
            &terrain,
            &graph,
        );
        let new_edge_validation = candidate_surface.validate_prepared_road_surface(
            &prepared_input.points,
            prepared_input.class,
            fwd_lanes,
            bkw_lanes,
            &terrain,
        );
        let full_validation = candidate_surface
            .validate_prepared_road_input_against_graph_with_compile_reason(
                &prepared_input,
                fwd_lanes,
                bkw_lanes,
                &terrain,
                &graph,
                new_edge_validation,
                RoadSurfaceCompileReason::CommitValidator,
            );

        assert!(
            !fast_validation.is_valid || full_validation.is_valid,
            "fast validator must not accept a candidate rejected by full validation: angle={angle_degrees:.1} lanes=({fwd_lanes},{bkw_lanes}) fast={fast_validation:?} full={full_validation:?}"
        );
    }
}

#[test]
fn preview_validation_uses_endpoint_snap_before_reporting_valid() {
    let terrain = flat_terrain(96, 64);
    let mut graph = RegionGraph::new();
    graph.add_node(Vector3::new(0.0, 1.5, 0.0), NodeType::Junction);
    let existing_surface = RoadSurfaceSystem::new(16.0);
    let preview_surface = RoadSurfaceSystem::new(16.0);
    let raw_points = vec![Vector3::new(1.0, 0.0, 0.0), Vector3::new(8.0, 0.0, 0.0)];

    let preview = preview_surface.compile_preview_surface_mesh_only_with_existing_surface(
        &raw_points,
        1,
        1,
        &terrain,
        &graph,
        &existing_surface,
    );

    assert!(preview.is_valid);
    assert_eq!(preview.validation.invalid_reason, "");
    assert!(
        (preview.prepared_points[0].y - 1.5).abs() <= 0.001,
        "preview must validate the endpoint height that commit will snap to"
    );
    assert_eq!(preview.validation.start_endpoint_snapped_node_id, 0);
    assert_eq!(preview.validation.end_endpoint_snapped_node_id, -1);
    assert!((preview.validation.start_endpoint_support_delta_m - 1.5).abs() <= 0.001);
}

#[test]
fn commit_validation_graph_splits_interior_crossings_before_surface_compile() {
    let terrain = flat_terrain(128, 128);
    let mut graph = RegionGraph::new();
    let west_pos = Vector3::new(-32.0, 0.0, 0.0);
    let east_pos = Vector3::new(32.0, 0.0, 0.0);
    let west = graph.add_node(west_pos, NodeType::Junction);
    let east = graph.add_node(east_pos, NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        east,
        vec![west_pos, east_pos],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let surface = RoadSurfaceSystem::new(16.0);
    let prepared_input = RoadSurfaceSystem::prepare_road_input_for_tool(
        &[Vector3::new(0.0, 0.0, -32.0), Vector3::new(0.0, 0.0, 32.0)],
        &terrain,
        &graph,
        &surface,
        true,
    );

    let (validation_graph, new_edge_idx, _, _) = surface
        .build_surface_validation_graph_for_test(
            &prepared_input.points,
            prepared_input.class,
            1,
            1,
            &graph,
        )
        .expect("crossing candidate should produce a local validation graph");
    assert!(
        !validation_graph.edge(new_edge_idx).deleted,
        "candidate edge must survive local topology processing"
    );
    let crossing_node = (0..validation_graph.node_count())
        .map(|node_id| node_id as u32)
        .find(|&node_id| {
            let pos = validation_graph.node(node_id).pos;
            let active_degree = validation_graph
                .node_adjacency(node_id)
                .iter()
                .filter(|&&edge_idx| !validation_graph.edge(edge_idx).deleted)
                .count();
            pos.distance_to(Vector3::ZERO) <= 0.001 && active_degree == 4
        });
    assert!(
        crossing_node.is_some(),
        "commit validation graph must include the same four-arm split node as a real commit"
    );

    let new_edge_validation = surface.validate_prepared_road_surface(
        &prepared_input.points,
        prepared_input.class,
        1,
        1,
        &terrain,
    );
    let validation = surface.validate_prepared_road_input_against_graph_with_compile_reason(
        &prepared_input,
        1,
        1,
        &terrain,
        &graph,
        new_edge_validation,
        RoadSurfaceCompileReason::CommitValidator,
    );
    assert!(
        validation.is_valid,
        "ordinary interior crossing should remain valid after local split topology: {validation:?}"
    );
}

#[test]
fn preview_can_disable_existing_road_endpoint_snap() {
    let terrain = flat_terrain(96, 64);
    let mut graph = RegionGraph::new();
    graph.add_node(Vector3::new(0.0, 1.5, 0.0), NodeType::Junction);
    let existing_surface = RoadSurfaceSystem::new(16.0);
    let preview_surface = RoadSurfaceSystem::new(16.0);
    let raw_points = vec![Vector3::new(1.0, 0.0, 0.0), Vector3::new(8.0, 0.0, 0.0)];

    let preview = preview_surface.compile_preview_surface_mesh_only_with_existing_surface_snap(
        &raw_points,
        1,
        1,
        &terrain,
        &graph,
        &existing_surface,
        false,
    );

    assert!(preview.is_valid);
    assert_eq!(preview.validation.invalid_reason, "");
    assert!(
        (preview.prepared_points[0].x - raw_points[0].x).abs() <= 0.001,
        "no-snap preview must preserve the authored endpoint XZ"
    );
    assert!(
        preview.prepared_points[0].y.abs() <= 0.001,
        "no-snap preview must not lift the endpoint to the nearby road node"
    );
    assert_eq!(preview.validation.start_endpoint_snapped_node_id, -1);
    assert_eq!(preview.validation.end_endpoint_snapped_node_id, -1);
}

#[test]
fn terminal_extension_reprofiles_existing_edge_instead_of_pinching_new_segment() {
    let terrain = flat_terrain(160, 64);
    let mut graph = RegionGraph::new();
    let far_old = graph.add_node(Vector3::new(-48.0, 0.0, 0.0), NodeType::Junction);
    let terminal = graph.add_node(Vector3::new(0.0, 8.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        far_old,
        terminal,
        vec![
            Vector3::new(-48.0, 0.0, 0.0),
            Vector3::new(-24.0, 4.0, 0.0),
            Vector3::new(0.0, 8.0, 0.0),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut existing_surface = RoadSurfaceSystem::new(16.0);
    existing_surface.compile_dirty(&graph, &terrain);
    let raw_points = vec![Vector3::new(0.0, 8.0, 0.0), Vector3::new(48.0, 0.0, 0.0)];
    let preview_surface = RoadSurfaceSystem::new(16.0);
    let preview = preview_surface.compile_preview_surface_mesh_only_with_existing_surface(
        &raw_points,
        1,
        1,
        &terrain,
        &graph,
        &existing_surface,
    );

    assert!(
        preview.is_valid,
        "terminal extension should validate against the combined corridor, not only the new half: {:?}",
        preview.validation
    );
    assert_eq!(
        preview.validation.start_endpoint_snapped_node_id,
        terminal as i32
    );
    assert!(
        preview.prepared_points[0].y.abs() <= 0.05,
        "the snapped terminal should become an internal reprofile point near the combined corridor height, got {:.3}",
        preview.prepared_points[0].y
    );

    let prepared = RoadSurfaceSystem::prepare_road_input_with_extension_to_visible_surface(
        &raw_points,
        &terrain,
        &graph,
        &existing_surface,
    );
    let extension = prepared
        .extension
        .expect("degree-1 standard terminal should produce an existing-edge reprofile");
    assert_eq!(extension.snapped_node_id, terminal);
    assert!(
        extension.snapped_node_pos.y.abs() <= 0.05,
        "reprofile should update the shared terminal height instead of keeping the old hard pin"
    );
    assert!(
        extension.existing_points.last().unwrap().y.abs() <= 0.05,
        "existing edge endpoint must match the reprofiled shared terminal"
    );
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
        let sample_x = section.center_xz.x + section.lateral_xz.x * f64::from(lateral_offset);
        let sample_z = section.center_xz.y + section.lateral_xz.y * f64::from(lateral_offset);
        let source_height = terrain.sample_height_world(sample_x as f32, sample_z as f32)
            * crate::config::HEIGHT_SCALE;
        let visual_height = terrain.sample_visual_height_world(sample_x as f32, sample_z as f32)
            * crate::config::HEIGHT_SCALE;
        let support_height = surface
            .sample_paved_support_height(&graph, &terrain, sample_x as f32, sample_z as f32)
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
