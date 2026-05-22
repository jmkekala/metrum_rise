//! Grounded road, earthwork, and terrain-support regression tests.

use super::*;

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
fn mixed_standard_bridge_node_earthwork_visibility_is_owner_scoped() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let standard_end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let bridge_end = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        center,
        standard_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        bridge_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .unwrap_or_else(|| {
            panic!(
                "mixed standard/bridge bend should compile a node piece: {}",
                canonical_node_pipeline_report(
                    &surface,
                    &graph,
                    center,
                    RoadSurfaceVisualNodePieceKind::Bend
                )
            )
        });

    let mut saw_standard_face = false;
    let mut saw_bridge_face = false;
    for face in &piece.render_earthwork_faces {
        let Some(edge_class) = node_earthwork_face_edge_class(piece, face.source) else {
            continue;
        };
        let visible = surface
            .node_earthwork_face_uses_visible_earthwork(&graph, &terrain, center, piece, face);
        match edge_class {
            EdgeClass::Standard => {
                saw_standard_face = true;
                assert!(
                    !visible,
                    "standard-owned node earthwork face must remain terrain/CDT-only"
                );
            }
            EdgeClass::Bridge => {
                saw_bridge_face = true;
                assert!(
                    visible,
                    "bridge-owned node earthwork face should remain structural"
                );
            }
            EdgeClass::Tunnel => {}
        }
    }

    assert!(
        saw_standard_face,
        "test setup should expose a standard-owned node boundary face"
    );
    assert!(
        saw_bridge_face,
        "test setup should expose a bridge-owned node boundary face"
    );
}

#[test]
fn mixed_standard_visible_tunnel_node_earthwork_visibility_is_owner_scoped() {
    let terrain = flat_terrain(97, 97);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let standard_end = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let tunnel_end = graph.add_node(Vector3::new(0.0, -6.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        center,
        standard_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        tunnel_end,
        vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 16.0),
            Vector3::new(0.0, -6.0, 24.0),
        ],
        10.0,
        EdgeClass::Tunnel,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .unwrap_or_else(|| {
            panic!(
                "mixed standard/visible-tunnel bend should compile a node piece: {}",
                canonical_node_pipeline_report(
                    &surface,
                    &graph,
                    center,
                    RoadSurfaceVisualNodePieceKind::Bend
                )
            )
        });

    let mut saw_standard_face = false;
    let mut saw_tunnel_face = false;
    for face in &piece.render_earthwork_faces {
        let Some(edge_class) = node_earthwork_face_edge_class(piece, face.source) else {
            continue;
        };
        let visible = surface
            .node_earthwork_face_uses_visible_earthwork(&graph, &terrain, center, piece, face);
        match edge_class {
            EdgeClass::Standard => {
                saw_standard_face = true;
                assert!(
                    !visible,
                    "standard-owned node earthwork face must remain terrain/CDT-only"
                );
            }
            EdgeClass::Tunnel => {
                saw_tunnel_face = true;
                assert!(
                    visible,
                    "visible tunnel-owned node earthwork face should remain structural"
                );
            }
            EdgeClass::Bridge => {}
        }
    }

    assert!(
        saw_standard_face,
        "test setup should expose a standard-owned node boundary face"
    );
    assert!(
        saw_tunnel_face,
        "test setup should expose a visible tunnel-owned node boundary face"
    );
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
