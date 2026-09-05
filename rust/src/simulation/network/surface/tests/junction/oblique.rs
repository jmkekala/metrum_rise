// SPDX-License-Identifier: GPL-2.0-only

//! Oblique junction regression tests.

use super::*;

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
fn logged_bend_upgraded_to_junctionn_preserves_exterior_non_road_arc() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-90.958466, 0.0, -17.827614),
        NodeType::Junction,
    );
    let center = graph.add_node(Vector3::new(14.830101, 0.0, -29.937523), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(70.083397, 0.0, 73.646095), NodeType::Junction);
    let branch = graph.add_node(Vector3::new(-3.189828, 0.0, 24.678764), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-90.958466, 0.0, -17.827614),
            Vector3::new(14.830101, 0.0, -29.937523),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        northeast,
        vec![
            Vector3::new(14.830101, 0.0, -29.937523),
            Vector3::new(70.083397, 0.0, 73.646095),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        branch,
        center,
        vec![
            Vector3::new(-3.189828, 0.0, 24.678764),
            Vector3::new(14.830101, 0.0, -29.937523),
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
    let expected_outer_bend_fill = logged_bend_upgrade_outer_fill_point();

    assert!(
        point_inside_visual_polygons(&piece.outer_boundary_loops, expected_outer_bend_fill),
        "upgraded JunctionN must preserve the reflex exterior non-road arc; point={expected_outer_bend_fill:?} outer_loops={:?}",
        piece.outer_boundary_loops
    );

    let (terrain_clip_loops, _) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -128.0, -64.0, 128.0, 128.0)
        .expect("logged JunctionN terrain clip export should be source-complete");
    let point_xz = expected_outer_bend_fill.to_road_xz();
    assert!(
        terrain_cdt_road_loops_contain_point(&terrain_clip_loops, point_xz),
        "upgraded JunctionN terrain cutter must keep clipping the reflex exterior non-road arc; point={expected_outer_bend_fill:?}"
    );
}

#[test]
fn logged_current_bend_upgraded_to_junctionn_preserves_exterior_non_road_without_asphalt() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-87.623878, 0.0, -15.375183),
        NodeType::Junction,
    );
    let center = graph.add_node(Vector3::new(4.538681, 0.0, -18.090569), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(46.647079, 0.0, 80.608551), NodeType::Junction);
    let branch = graph.add_node(Vector3::new(-20.544514, 0.0, 42.822044), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-87.623878, 0.0, -15.375183),
            Vector3::new(4.538681, 0.0, -18.090569),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        northeast,
        vec![
            Vector3::new(4.538681, 0.0, -18.090569),
            Vector3::new(46.647079, 0.0, 80.608551),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        branch,
        center,
        vec![
            Vector3::new(-20.544514, 0.0, 42.822044),
            Vector3::new(4.538681, 0.0, -18.090569),
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
    let expected_outer_bend_fill = logged_current_bend_upgrade_outer_asphalt_fill_point();

    assert!(
        !point_inside_visual_polygons(&piece.road_surface_polygons, expected_outer_bend_fill),
        "upgraded JunctionN must not fill the reflex exterior lobe with asphalt; point={expected_outer_bend_fill:?}"
    );
    assert!(
        point_inside_visual_polygons(&piece.outer_boundary_loops, expected_outer_bend_fill),
        "upgraded JunctionN footprint must preserve the reflex exterior non-road fill point; point={expected_outer_bend_fill:?}"
    );

    let (terrain_clip_loops, _) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -128.0, -64.0, 128.0, 128.0)
        .expect("current logged JunctionN terrain clip export should be source-complete");
    assert!(
        terrain_cdt_road_loops_contain_point(
            &terrain_clip_loops,
            expected_outer_bend_fill.to_road_xz()
        ),
        "upgraded JunctionN terrain cutter must preserve the reflex exterior non-road lobe; point={expected_outer_bend_fill:?}"
    );
}

#[test]
fn logged_bent_t_junction_preserves_exterior_non_road_without_asphalt() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-76.432709, 0.0, 3.041950), NodeType::Junction);
    let center = graph.add_node(Vector3::new(2.923641, 0.0, -1.916813), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(56.311253, 0.0, 51.254074), NodeType::Junction);
    let branch = graph.add_node(Vector3::new(-29.772259, 0.0, 78.427689), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-76.432709, 0.0, 3.041950),
            Vector3::new(2.923641, 0.0, -1.916813),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        northeast,
        vec![
            Vector3::new(2.923641, 0.0, -1.916813),
            Vector3::new(56.311253, 0.0, 51.254074),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        branch,
        center,
        vec![
            Vector3::new(-29.772259, 0.0, 78.427689),
            Vector3::new(2.923641, 0.0, -1.916813),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_junction_piece(&surface, &graph, center);
    let compile_input = surface
        .visual_node_compile_input(&graph, center)
        .expect("logged bent T should expose JunctionN compile input");
    let arrangement_input = RoadSurfaceSystem::build_node_arrangement_input_from_mouths(
        center,
        compile_input.kind,
        &compile_input.mouths,
    )
    .expect("logged bent T should produce arrangement input");
    let rails = RoadSurfaceSystem::build_node_rail_contours_from_input(&arrangement_input)
        .expect("logged bent T should produce rail contours");
    let ownership = RoadSurfaceSystem::build_node_boolean_ownership_from_rails(&rails)
        .expect("logged bent T should solve ownership");
    let debug = node::NodeBooleanDebugSnapshot::from_rails_and_ownership(&rails, &ownership, false);
    let exterior_gaps = debug
        .side_join_gaps
        .iter()
        .filter(|gap| gap.role == node::joins::NodeInputSideJoinGapRole::Exterior)
        .collect::<Vec<_>>();
    assert_eq!(
        exterior_gaps.len(),
        1,
        "logged bent T should classify exactly one reflex exterior gap: {:?}",
        debug.side_join_gaps
    );
    let exterior_gap = exterior_gaps[0];
    assert!(
        exterior_gap
            .suppressed_band_kinds
            .contains(&RoadSurfaceBandKind::Carriageway),
        "exterior gap must record suppressed asphalt side-join emission: {exterior_gap:?}"
    );
    assert!(
        !exterior_gap
            .emitted_band_kinds
            .contains(&RoadSurfaceBandKind::Carriageway),
        "exterior gap must not emit asphalt side-join bands: {exterior_gap:?}"
    );
    assert!(
        !exterior_gap
            .suppressed_band_kinds
            .contains(&RoadSurfaceBandKind::CurbOrShoulder)
            && !exterior_gap
                .suppressed_band_kinds
                .contains(&RoadSurfaceBandKind::Sidewalk),
        "exterior gap must not suppress non-road side-join bands: {exterior_gap:?}"
    );
    assert!(
        exterior_gap
            .emitted_band_kinds
            .contains(&RoadSurfaceBandKind::CurbOrShoulder)
            && exterior_gap
                .emitted_band_kinds
                .contains(&RoadSurfaceBandKind::Sidewalk),
        "exterior gap must emit curb/sidewalk continuation bands: {exterior_gap:?}"
    );
    assert!(
        exterior_gap.final_asphalt_area_m2 <= 0.001,
        "exterior gap must not finally own asphalt side-join area: {exterior_gap:?}"
    );
    assert!(
        exterior_gap.final_curb_area_m2 > 0.001 && exterior_gap.final_sidewalk_area_m2 > 0.001,
        "exterior gap must finally own non-road continuation area: {exterior_gap:?}"
    );
    assert!(
        !debug.owned_regions.iter().any(|region| {
            region.claim_priority == rails::NodeGeneratedContourClaimPriority::SideJoin
                && region.source_mouth_order_index == exterior_gap.from_mouth_order_index
                && region.kind == RoadSurfaceBandKind::Carriageway
        }),
        "no exterior/reflex JunctionN gap may produce final Carriageway side-join ownership: {:?}",
        debug.owned_regions
    );
    assert!(
        debug.side_join_material_trims.iter().any(|trim| {
            trim.kind == RoadSurfaceBandKind::Carriageway
                && trim.role == Some(node::joins::NodeInputSideJoinGapRole::Interior)
                && trim.raw_area_m2 > 0.001
                && trim.trimmed_area_m2 <= trim.raw_area_m2 + 0.001
        }),
        "JunctionN side-join asphalt debug must expose raw/trimmed asphalt area: {:?}",
        debug.side_join_material_trims
    );
    assert!(
        debug.side_join_material_trims.iter().any(|trim| {
            trim.kind == RoadSurfaceBandKind::Sidewalk
                && trim.role == Some(node::joins::NodeInputSideJoinGapRole::Exterior)
                && trim.final_owned_area_m2 > 0.001
        }),
        "JunctionN side-join debug must expose preserved exterior sidewalk area: {:?}",
        debug.side_join_material_trims
    );
}

#[test]
fn logged_current_bent_t_junction_trims_exterior_asphalt_islands() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-85.998276, 0.0, 15.480217), NodeType::Junction);
    let center = graph.add_node(Vector3::new(17.261505, 0.0, 36.514523), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(64.834915, 0.0, 117.753670), NodeType::Junction);
    let branch = graph.add_node(Vector3::new(-28.215271, 0.0, 82.433357), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-85.998276, 0.0, 15.480217),
            Vector3::new(17.261505, 0.0, 36.514523),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        northeast,
        vec![
            Vector3::new(17.261505, 0.0, 36.514523),
            Vector3::new(64.834915, 0.0, 117.753670),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        branch,
        center,
        vec![
            Vector3::new(-28.215271, 0.0, 82.433357),
            Vector3::new(17.261505, 0.0, 36.514523),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    let bad_islands = [
        Vector2::new(20.482485, 35.878922),
        Vector2::new(16.822950, 33.260852),
    ];
    for point in bad_islands {
        assert!(
            !point_inside_visual_polygons(&piece.road_surface_polygons, point),
            "exterior trim must remove logged asphalt island from road surface; point={point:?}"
        );
        assert!(
            !point_inside_visual_polygons(&piece.outer_boundary_loops, point),
            "exterior trim must remove logged asphalt island from node footprint; point={point:?}"
        );
    }

    let curved_sidewalk = Vector2::new(20.873661, 37.576516);
    assert!(
        point_inside_visual_polygons(&piece.sidewalk_surface_polygons, curved_sidewalk),
        "exterior trim must preserve the curved sidewalk band; point={curved_sidewalk:?}"
    );
    assert!(
        point_inside_visual_polygons(&piece.outer_boundary_loops, curved_sidewalk),
        "exterior trim must preserve footprint over the curved sidewalk band; point={curved_sidewalk:?}"
    );
    assert!(
        !point_inside_visual_polygons(&piece.road_surface_polygons, curved_sidewalk),
        "curved sidewalk sample must not become asphalt; point={curved_sidewalk:?}"
    );
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

fn logged_bend_upgrade_outer_fill_point() -> Vector2 {
    let center = backend::RoadVec2::new(14.830101, -29.937523);
    let west = backend::RoadVec2::new(-90.958466, -17.827614);
    let northeast = backend::RoadVec2::new(70.083397, 73.646095);
    bend_upgrade_outer_fill_point(center, west, northeast, 4.5)
}

fn logged_current_bend_upgrade_outer_asphalt_fill_point() -> Vector2 {
    let center = backend::RoadVec2::new(4.538681, -18.090569);
    let west = backend::RoadVec2::new(-87.623878, -15.375183);
    let northeast = backend::RoadVec2::new(46.647079, 80.608551);
    bend_upgrade_outer_fill_point(center, west, northeast, 2.5)
}

fn bend_upgrade_outer_fill_point(
    center: backend::RoadVec2,
    west: backend::RoadVec2,
    northeast: backend::RoadVec2,
    radius_m: f64,
) -> Vector2 {
    let first_direction = normalized_test_direction(northeast - center);
    let second_direction = normalized_test_direction(west - center);
    let start = center + test_left_perp(second_direction) * 5.0;
    let end = center - test_left_perp(first_direction) * 5.0;
    let start_radius = start - center;
    let end_radius = end - center;
    let start_angle = start_radius.y.atan2(start_radius.x);
    let end_angle = end_radius.y.atan2(end_radius.x);
    let ccw_sweep = (end_angle - start_angle).rem_euclid(std::f64::consts::TAU);
    let cw_sweep = -((start_angle - end_angle).rem_euclid(std::f64::consts::TAU));
    let sweep = if ccw_sweep <= cw_sweep.abs() {
        ccw_sweep
    } else {
        cw_sweep
    };
    let mid_angle = start_angle + sweep * 0.5;
    Vector2::new(
        (center.x + mid_angle.cos() * radius_m) as f32,
        (center.y + mid_angle.sin() * radius_m) as f32,
    )
}

fn terrain_cdt_road_loops_contain_point(
    terrain_clip_loops: &[TerrainCdtRoadLoop],
    point_xz: backend::RoadVec2,
) -> bool {
    terrain_clip_loops.iter().any(|road_loop| {
        RoadSurfaceSystem::polygon_contains_point_xz(
            &road_loop
                .vertices
                .iter()
                .map(|vertex| {
                    backend::RoadVec3::new(vertex.x, f64::from(vertex.height_m), vertex.z)
                })
                .collect::<Vec<_>>(),
            point_xz,
        )
    })
}

fn normalized_test_direction(direction: backend::RoadVec2) -> backend::RoadVec2 {
    direction / direction.length()
}

fn test_left_perp(direction: backend::RoadVec2) -> backend::RoadVec2 {
    backend::RoadVec2::new(-direction.y, direction.x)
}
