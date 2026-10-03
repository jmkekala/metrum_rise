// SPDX-License-Identifier: GPL-2.0-only

//! Terrain clip loop export tests.

use super::*;

#[test]
fn long_road_terrain_exports_stay_local_and_preserve_sources() {
    let terrain = TerrainSystem::with_chunking(2041, 2041, 10.0, 51, 100.0);
    let mut graph = RegionGraph::new();
    let a = Vector3::new(2048.0, 100.0, 2048.0);
    let b = Vector3::new(4838.0, 100.0, 2048.0);
    let start = graph.add_node(a, NodeType::Junction);
    let end = graph.add_node(b, NodeType::Junction);
    graph.add_edge(crate::simulation::network::build_surface_edge(
        start,
        end,
        vec![a, b],
        1,
        1,
        EdgeClass::Standard,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();
    let mut surface = RoadSurfaceSystem::new(512.0);
    assert!(surface.compile_dirty(&graph, &terrain));
    for min_x in [1466.0, 1976.0, 2486.0, 2996.0, 3506.0, 4016.0, 4526.0] {
        let (loops, _) = surface
            .terrain_cdt_road_loops_for_world_bounds(&graph, min_x, 1976.0, min_x + 638.0, 2614.0)
            .expect("ROAD-14: a patch query must not union a whole 2790 m footprint");
        assert!(!loops.is_empty());
        for road_loop in loops {
            assert!(!road_loop.source_edges.is_empty());
            for point in road_loop.vertices {
                assert!((point.height_m - 100.0).abs() < 1.0);
                assert!(point.x >= f64::from(min_x) - 128.0 && point.x <= f64::from(min_x) + 766.0);
            }
        }
    }
}

#[test]
fn terrain_clip_loops_include_standard_grounded_footprints() {
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

    let (cdt_road_loops, cdt_source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -16.0, -32.0, 16.0, 32.0)
        .expect("production terrain clip export should keep source-owned loops");

    assert!(
        !cdt_road_loops.is_empty(),
        "expected grounded standard road footprint loops to clip terrain topology"
    );
    assert!(
        cdt_road_loops
            .iter()
            .flat_map(|road_loop| road_loop.vertices.iter())
            .any(|point| point.x.abs() > 5.0),
        "expected terrain clip loops to include the full sidewalk / shoulder footprint"
    );
    assert!(
        cdt_road_loops
            .iter()
            .all(|road_loop| road_loop.vertices.len() >= 3),
        "expected every terrain clip loop to be a valid road footprint contour"
    );
    let expected_terrain_clip_source_loop_count: usize = surface
        .compiled_visual_span_pieces()
        .values()
        .map(|piece| piece.terrain_clip_boundary_loops().len())
        .sum::<usize>()
        + surface
            .compiled_visual_node_pieces()
            .values()
            .map(|piece| piece.terrain_clip_boundary_loops.len())
            .sum::<usize>();
    assert!(
        cdt_road_loops.len() <= expected_terrain_clip_source_loop_count,
        "expected terrain clip cutters to be the boolean-unioned piece footprint, got {} loops for {} raw clip loops",
        cdt_road_loops.len(),
        expected_terrain_clip_source_loop_count
    );
    assert_eq!(cdt_source_count, expected_terrain_clip_source_loop_count);
    assert!(
        cdt_road_loops
            .iter()
            .flat_map(|road_loop| road_loop.source_edges.iter())
            .all(|edge| !matches!(
                edge.source,
                TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. }
            )),
        "production terrain CDT loops must carry real span/node boundary sources, not synthetic polygon ids"
    );
    assert!(
        cdt_road_loops
            .iter()
            .flat_map(|road_loop| road_loop.source_edges.iter())
            .any(|edge| matches!(
                edge.source,
                TerrainCdtRoadBoundarySource::SpanSupportBoundary { .. }
                    | TerrainCdtRoadBoundarySource::NodeFootprintBoundary { .. }
            )),
        "expected source-preserving CDT export to expose final owned terrain boundary provenance"
    );
}

// A curved hillside road and a bridge with grounded abutments, compiled.
fn hillside_road_and_bridge_surface() -> RoadSurfaceSystem {
    let terrain = coarse_hillside_world_terrain(201, 201, 4.0);
    let ground = |x: f32, z: f32, lift_m: f32| {
        Vector3::new(
            x,
            terrain.sample_height_world(x, z) * crate::config::HEIGHT_SCALE + lift_m,
            z,
        )
    };
    let curve = (0..=24)
        .map(|index| {
            let x = -300.0 + index as f32 * 25.0;
            ground(x, -150.0 + 60.0 * (x / 140.0).sin(), 0.0)
        })
        .collect::<Vec<_>>();
    let bridge = (0..=20)
        .map(|index| {
            let x = -300.0 + index as f32 * 15.0;
            ground(x, 200.0, ((x + 120.0) * 0.08).max(0.0))
        })
        .collect::<Vec<_>>();
    let mut graph = RegionGraph::new();
    for (points, class) in [(curve, EdgeClass::Standard), (bridge, EdgeClass::Bridge)] {
        let start = graph.add_node(points[0], NodeType::Junction);
        let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
        graph.add_edge(test_edge(
            start,
            end,
            points,
            10.0,
            class,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();
    let mut surface = RoadSurfaceSystem::new(64.0);
    assert!(surface.compile_dirty(&graph, &terrain));
    surface
}

#[test]
fn compiled_span_clip_loops_store_keyed_edges_and_remap_sources() {
    // Every compiled span loop must take the keyed form, and a remapped piece must rebuild
    // the remapped sources.
    let surface = hillside_road_and_bridge_surface();
    let mut classes = Vec::new();
    for (&edge_idx, piece) in surface.compiled_visual_span_pieces() {
        assert!(!piece.terrain_clip_loops.is_empty(), "edge {edge_idx}");
        assert!(
            piece
                .terrain_clip_loops
                .iter()
                .all(|clip_loop| clip_loop.is_keyed()),
            "edge {edge_idx} clip loops must not fall back to explicit sources"
        );
        classes.push(piece.edge_class);

        let mut expected = piece.terrain_clip_boundary_loops();
        for edge in expected
            .iter_mut()
            .flat_map(|clip_loop| &mut clip_loop.source_edges)
        {
            edge.source = edge.source.with_span_identity(edge_idx + 40);
        }
        let remapped = piece.clone_with_edge_identity(edge_idx + 40);
        assert_eq!(remapped.terrain_clip_boundary_loops(), expected);
        let moved = (**piece).clone().into_with_edge_identity(edge_idx + 40);
        assert_eq!(moved.terrain_clip_boundary_loops(), expected);
    }
    assert!(classes.contains(&EdgeClass::Standard) && classes.contains(&EdgeClass::Bridge));
}

#[test]
fn compiled_span_regions_derive_sources_from_sections() {
    // Regions keep only start section and band; edge id, end section, stations and role come
    // from the piece's sections, so remapping an edge id shares the region lists unchanged.
    use crate::simulation::network::surface::{
        RoadSurfaceEarthworkFaceSource, RoadSurfaceSpanOwnedRegion,
    };
    use std::sync::Arc;
    assert_eq!(std::mem::size_of::<RoadSurfaceSpanOwnedRegion>(), 8);
    let surface = hillside_road_and_bridge_surface();
    let mut separate_support_regions = false;
    for (&edge_idx, piece) in surface.compiled_visual_span_pieces() {
        let remapped = piece.clone_with_edge_identity(edge_idx + 40);
        let moved = (**piece).clone().into_with_edge_identity(edge_idx + 40);
        assert!(Arc::ptr_eq(
            &remapped.span_owned_regions,
            &piece.span_owned_regions
        ));
        separate_support_regions |= !Arc::ptr_eq(
            &piece.span_owned_regions,
            &piece.span_earthwork_support_regions,
        );
        let regions = piece
            .span_owned_regions
            .iter()
            .chain(piece.span_earthwork_support_regions.iter());
        for region in regions {
            let source = region.support_boundary_source(&piece.sections, piece.edge_class);
            let RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                edge_idx: source_edge_idx,
                owner,
                role,
                start_section_index,
                end_section_index,
                start_s_m,
                end_s_m,
                ..
            } = source
            else {
                unreachable!("span regions emit span support sources");
            };
            let (start, end) = (
                &piece.sections[start_section_index],
                &piece.sections[end_section_index],
            );
            assert_eq!(source_edge_idx, edge_idx);
            assert_eq!(end_section_index, start_section_index + 1);
            assert_eq!(start_s_m.to_bits(), start.s_m.to_bits());
            assert_eq!(end_s_m.to_bits(), end.s_m.to_bits());
            assert_eq!(start.bands[owner.source_band_index].kind, owner.kind);
            assert_eq!(end.bands[owner.source_band_index].kind, owner.kind);
            assert_eq!(role, region.role());
            let expected = source.with_span_identity(edge_idx + 40);
            for remapped_piece in [&remapped, &moved] {
                assert_eq!(
                    region.support_boundary_source(&remapped_piece.sections, piece.edge_class),
                    expected
                );
            }
        }
    }
    assert!(
        separate_support_regions,
        "the bridge must resolve its own support regions"
    );
}
