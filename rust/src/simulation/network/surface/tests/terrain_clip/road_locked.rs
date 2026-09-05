// SPDX-License-Identifier: GPL-2.0-only

//! Road-locked terrain patch selection tests.

use super::*;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::terrain::{
    TerrainSystem, terrain_cdt_local_sample_margin_m, terrain_cdt_road_query_margin_m,
};

#[test]
fn road_locked_terrain_patches_include_visible_footprint_without_margin() {
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

    let mut footprint_min_x = f64::MAX;
    let mut footprint_max_x = f64::MIN;
    let mut footprint_min_z = f64::MAX;
    let mut footprint_max_z = f64::MIN;
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

    let keys = surface.terrain_render_patch_keys_with_visible_road_margin(&graph, &terrain, 0.0);
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
            f64::from(patch.world_origin_x) <= footprint_max_x
                && f64::from(patch_max_x) >= footprint_min_x
                && f64::from(patch.world_origin_z) <= footprint_max_z
                && f64::from(patch_max_z) >= footprint_min_z,
            "road-locked patch ({patch_x}, {patch_z}) must overlap the road footprint, not only the earthwork envelope"
        );
    }
}

#[test]
fn road_locked_terrain_patches_expand_for_cdt_seam_margin() {
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

    let unexpanded =
        surface.terrain_render_patch_keys_with_visible_road_margin(&graph, &terrain, 0.0);
    let expanded =
        surface.terrain_render_patch_keys_with_visible_road_margin(&graph, &terrain, 8.0);
    assert!(
        unexpanded.iter().all(|key| expanded.contains(key)),
        "seam-expanded road-locked patch selection must preserve visible road patches"
    );
    assert!(
        expanded.len() > unexpanded.len(),
        "local CDT seam margin should road-lock neighboring patches before a patch-edge crack can form"
    );
}

#[test]
fn road_locked_terrain_patches_expand_for_required_grading_envelope() {
    let terrain = flat_terrain(257, 257);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 16.0, -48.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(0.0, 16.0, 48.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, 16.0, -48.0),
            Vector3::new(0.0, 16.0, 48.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let render_step_m = 2.0;
    let base_margin_m = terrain_cdt_local_sample_margin_m(&terrain, render_step_m);
    let required_margin_m = surface.terrain_cdt_required_grading_margin_for_visible_roads(
        &graph,
        &terrain,
        render_step_m,
    );
    assert!(
        required_margin_m > base_margin_m,
        "high fill should request a grading envelope wider than the fixed seam margin"
    );

    let base_keys =
        surface.terrain_render_patch_keys_with_visible_road_margin(&graph, &terrain, base_margin_m);
    let expanded_keys = surface.terrain_render_patch_keys_with_visible_road_margin(
        &graph,
        &terrain,
        required_margin_m,
    );
    assert!(
        base_keys.iter().all(|key| expanded_keys.contains(key)),
        "required grading envelope must preserve base road-locked patches"
    );
    assert!(
        expanded_keys.len() > base_keys.len(),
        "high fill should road-lock extra render patches for the full tie-in envelope"
    );
}

#[test]
fn road_locked_required_grading_margin_samples_outward_terrain() {
    let mut terrain = flat_terrain(257, 257);
    for z in 0..terrain.height {
        for x in 0..terrain.width {
            let (world_x, _) = terrain.grid_to_world_coords(x, z);
            if world_x < -12.0 {
                terrain.set_height(x, z, -16.0 / crate::config::HEIGHT_SCALE);
            }
        }
    }
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

    let render_step_m = 2.0;
    let base_margin_m = terrain_cdt_local_sample_margin_m(&terrain, render_step_m);
    let required_margin_m = surface.terrain_cdt_required_grading_margin_for_visible_roads(
        &graph,
        &terrain,
        render_step_m,
    );

    assert!(
        required_margin_m > base_margin_m + 16.0,
        "required grading margin must account for terrain sampled outward from the road seam"
    );
}

#[test]
fn road_locked_patch_margins_do_not_globalize_high_fill() {
    let terrain = flat_terrain(257, 257);
    let mut graph = RegionGraph::new();
    let high_start = graph.add_node(Vector3::new(-80.0, 16.0, -32.0), NodeType::Junction);
    let high_end = graph.add_node(Vector3::new(-80.0, 16.0, 32.0), NodeType::Junction);
    graph.add_edge(test_edge(
        high_start,
        high_end,
        vec![
            Vector3::new(-80.0, 16.0, -32.0),
            Vector3::new(-80.0, 16.0, 32.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let flat_start = graph.add_node(Vector3::new(80.0, 0.0, -32.0), NodeType::Junction);
    let flat_end = graph.add_node(Vector3::new(80.0, 0.0, 32.0), NodeType::Junction);
    graph.add_edge(test_edge(
        flat_start,
        flat_end,
        vec![
            Vector3::new(80.0, 0.0, -32.0),
            Vector3::new(80.0, 0.0, 32.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let render_step_m = 2.0;
    let base_margin_m = terrain_cdt_local_sample_margin_m(&terrain, render_step_m);
    let global_margin_m = surface.terrain_cdt_required_grading_margin_for_visible_roads(
        &graph,
        &terrain,
        render_step_m,
    );
    let patch_margins = surface.terrain_render_patch_grading_margins_for_visible_roads(
        &graph,
        &terrain,
        render_step_m,
    );
    let flat_patch_keys = terrain.render_patch_keys_for_world_bounds(72.0, -16.0, 88.0, 16.0);
    let flat_patch_margin_m = flat_patch_keys
        .iter()
        .filter_map(|key| patch_margins.get(key).copied())
        .fold(0.0_f32, f32::max);

    assert!(
        global_margin_m > base_margin_m,
        "the high road should still request a larger global debug summary margin"
    );
    assert!(
        flat_patch_margin_m > 0.0,
        "the flat-road probe patches must be covered by the road-locked margin map"
    );
    assert!(
        flat_patch_margin_m <= base_margin_m + 0.001,
        "flat-road patches must not inherit an unrelated high-fill road's grading margin"
    );
}

#[test]
fn road_locked_patch_margins_follow_local_grading_envelope_on_one_road() {
    let terrain = flat_terrain(257, 257);
    let mut graph = RegionGraph::new();
    let high_start = graph.add_node(Vector3::new(0.0, 16.0, -96.0), NodeType::Junction);
    let low_end = graph.add_node(Vector3::new(0.0, 0.0, 96.0), NodeType::Junction);
    graph.add_edge(test_edge(
        high_start,
        low_end,
        vec![Vector3::new(0.0, 16.0, -96.0), Vector3::new(0.0, 0.0, 96.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let render_step_m = 2.0;
    let base_margin_m = terrain_cdt_local_sample_margin_m(&terrain, render_step_m);
    let global_margin_m = surface.terrain_cdt_required_grading_margin_for_visible_roads(
        &graph,
        &terrain,
        render_step_m,
    );
    let patch_margins = surface.terrain_render_patch_grading_margins_for_visible_roads(
        &graph,
        &terrain,
        render_step_m,
    );
    let low_end_patch_keys = terrain.render_patch_keys_for_world_bounds(-4.0, 80.0, 4.0, 112.0);
    let low_end_patch_margin_m = low_end_patch_keys
        .iter()
        .filter_map(|key| patch_margins.get(key).copied())
        .fold(0.0_f32, f32::max);

    assert!(
        global_margin_m > base_margin_m,
        "the high end of the road should still produce a larger debug summary margin"
    );
    assert!(
        low_end_patch_margin_m > 0.0,
        "the low end of the road must remain covered by the road-locked patch map"
    );
    assert!(
        low_end_patch_margin_m <= base_margin_m + 0.01,
        "the low end of one road must not inherit the high end's grading margin: low_end_patch_margin_m={low_end_patch_margin_m:.3} base_margin_m={base_margin_m:.3} global_margin_m={global_margin_m:.3}"
    );
}

#[test]
fn road_locked_grading_pad_is_preserved_in_clip_query_margin() {
    let render_step_m = 2.0;
    let patch_span_m = 512.0;
    let config = WorldConfig::new(1_280.0, 1_280.0, 40.0, 10.0)
        .with_terrain_resolution(10.0)
        .with_chunking(patch_span_m, 0.0);
    let terrain = TerrainSystem::from_world_config(&config);
    let base_margin_m = terrain_cdt_local_sample_margin_m(&terrain, render_step_m);
    let query_pad_m = render_step_m.max(terrain.cell_size_m());
    let (target_patch_min_x, _, _, _) = terrain
        .render_patch_world_bounds(1, 1)
        .expect("fixture must contain the target neighboring patch");

    let mut probe_graph = RegionGraph::new();
    let probe_start = probe_graph.add_node(Vector3::new(0.0, 0.0, -48.0), NodeType::Junction);
    let probe_end = probe_graph.add_node(Vector3::new(0.0, 0.0, 48.0), NodeType::Junction);
    probe_graph.add_edge(test_edge(
        probe_start,
        probe_end,
        vec![Vector3::new(0.0, 0.0, -48.0), Vector3::new(0.0, 0.0, 48.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut probe_surface = RoadSurfaceSystem::new(patch_span_m);
    probe_surface.compile_dirty(&probe_graph, &terrain);
    let probe_east_extent_m = probe_surface
        .compiled_visual_span_pieces()
        .values()
        .flat_map(|piece| &piece.terrain_clip_boundary_loops)
        .chain(
            probe_surface
                .compiled_visual_node_pieces()
                .values()
                .flat_map(|piece| &piece.terrain_clip_boundary_loops),
        )
        .flat_map(|boundary_loop| &boundary_loop.points_world)
        .map(|point| point.x)
        .fold(f64::NEG_INFINITY, f64::max) as f32;
    assert!(probe_east_extent_m.is_finite());

    // Put the east road seam inside the grading-ray selection pad, but outside the raw grading
    // margin of the neighboring patch. This reproduces the reported bridge-approach boundary case.
    let road_x = target_patch_min_x - base_margin_m - query_pad_m * 0.5 - probe_east_extent_m;
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(road_x, 0.0, -48.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(road_x, 0.0, 48.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(road_x, 0.0, -48.0),
            Vector3::new(road_x, 0.0, 48.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut surface = RoadSurfaceSystem::new(patch_span_m);
    surface.compile_dirty(&graph, &terrain);

    let patch_margins = surface.terrain_render_patch_grading_margins_for_visible_roads(
        &graph,
        &terrain,
        render_step_m,
    );
    let target_key = (1, 1);
    let grading_margin_m = *patch_margins
        .get(&target_key)
        .expect("grading-ray safety pad must select the neighboring patch");
    let (min_x, min_z, max_x, max_z) = terrain
        .render_patch_world_bounds(target_key.0, target_key.1)
        .expect("selected patch bounds must exist");
    let (_, raw_source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(
            &graph,
            min_x - grading_margin_m,
            min_z - grading_margin_m,
            max_x + grading_margin_m,
            max_z + grading_margin_m,
        )
        .expect("raw clip query should remain structurally valid");
    assert_eq!(
        raw_source_count, 0,
        "fixture must keep the road seam beyond the unpadded grading query"
    );

    let query_margin_m = terrain_cdt_road_query_margin_m(&terrain, render_step_m, grading_margin_m);
    let (road_loops, padded_source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(
            &graph,
            min_x - query_margin_m,
            min_z - query_margin_m,
            max_x + query_margin_m,
            max_z + query_margin_m,
        )
        .expect("padded clip query should remain structurally valid");
    assert!(
        padded_source_count > 0 && !road_loops.is_empty(),
        "a road-locked patch selected by the grading pad must retain its road clip sources"
    );
}

#[test]
fn road_locked_terrain_patches_skip_bridge_and_tunnel_only_surfaces() {
    let terrain = flat_terrain(257, 257);
    let mut graph = RegionGraph::new();
    let bridge_start = graph.add_node(Vector3::new(-32.0, 8.0, -48.0), NodeType::Junction);
    let bridge_end = graph.add_node(Vector3::new(-32.0, 8.0, 48.0), NodeType::Junction);
    graph.add_edge(test_edge(
        bridge_start,
        bridge_end,
        vec![
            Vector3::new(-32.0, 8.0, -48.0),
            Vector3::new(-32.0, 8.0, 48.0),
        ],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let tunnel_start = graph.add_node(Vector3::new(32.0, -8.0, -48.0), NodeType::Junction);
    let tunnel_end = graph.add_node(Vector3::new(32.0, -8.0, 48.0), NodeType::Junction);
    graph.add_edge(test_edge(
        tunnel_start,
        tunnel_end,
        vec![
            Vector3::new(32.0, -8.0, -48.0),
            Vector3::new(32.0, -8.0, 48.0),
        ],
        10.0,
        EdgeClass::Tunnel,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let keys = surface.terrain_render_patch_keys_with_visible_road_margin(&graph, &terrain, 0.0);
    assert!(
        keys.is_empty(),
        "bridge/tunnel-only surfaces must not request grounded-road CDT terrain clips"
    );
}

#[test]
fn terrain_clip_loops_merge_standard_pass_through_span_handoffs() {
    let terrain = flat_terrain(257, 257);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(-48.0, 0.0, 0.0), NodeType::Junction);
    let middle = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(48.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        middle,
        vec![Vector3::new(-48.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        middle,
        end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(48.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let (road_loops, source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -16.0, -16.0, 16.0, 16.0)
        .expect("standard pass-through handoff terrain clip export should stay sourced");

    assert!(
        !road_loops.is_empty() && source_count > 0,
        "standard pass-through handoffs must keep terrain-CDT road holes"
    );
}

#[test]
fn terrain_clip_loops_skip_bridge_midspans() {
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

    let (road_loops, source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -16.0, -32.0, 16.0, 32.0)
        .expect("bridge midspan terrain clip export should succeed");

    assert!(
        road_loops.is_empty() && source_count == 0,
        "bridge midspans must not cut terrain topology like grounded standard roads"
    );
}

#[test]
fn terrain_clip_loops_include_grounded_bridge_abutments_only() {
    let terrain = flat_terrain(129, 65);
    let mut graph = RegionGraph::new();
    let grounded = graph.add_node(Vector3::new(-48.0, 0.0, 0.0), NodeType::Junction);
    let elevated = graph.add_node(Vector3::new(48.0, 7.5, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        grounded,
        elevated,
        vec![Vector3::new(-48.0, 0.0, 0.0), Vector3::new(48.0, 7.5, 0.0)],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let bridge_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("bridge ramp must compile");

    assert!(
        !bridge_piece.terrain_clip_boundary_loops.is_empty(),
        "the grounded bridge ramp end must own a terrain cutout"
    );
    assert!(bridge_piece.start_terrain_clip_node);
    assert!(!bridge_piece.end_terrain_clip_node);
    assert!(
        bridge_piece
            .terrain_clip_boundary_loops
            .iter()
            .flat_map(|boundary_loop| &boundary_loop.source_edges)
            .all(|source_edge| matches!(
                source_edge.source,
                RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                    edge_class: EdgeClass::Bridge,
                    support_policy: RoadSurfaceEarthworkSupportPolicy::BridgeEndpointAbutments,
                    ..
                }
            )),
        "bridge abutment terrain clips must retain structural provenance"
    );

    let (grounded_loops, grounded_source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -64.0, -16.0, -32.0, 16.0)
        .expect("grounded bridge abutment terrain clip export must succeed");
    assert!(
        !grounded_loops.is_empty() && grounded_source_count > 0,
        "the grounded bridge span must contribute a terrain cutout"
    );

    let (elevated_loops, elevated_source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, 24.0, -16.0, 64.0, 16.0)
        .expect("elevated bridge terrain clip query must succeed");
    assert!(
        elevated_loops.is_empty() && elevated_source_count == 0,
        "the elevated bridge run and terminal must remain outside terrain clipping"
    );
}

#[test]
fn grounded_bridge_abutment_cdt_keeps_grade_compliant_faces_in_terrain_bucket() {
    let terrain = flat_terrain(129, 65);
    let mut graph = RegionGraph::new();
    let grounded = graph.add_node(Vector3::new(-48.0, 0.0, 0.0), NodeType::Junction);
    let elevated = graph.add_node(Vector3::new(48.0, 7.5, 0.0), NodeType::Junction);
    graph.add_edge(test_edge(
        grounded,
        elevated,
        vec![Vector3::new(-48.0, 0.0, 0.0), Vector3::new(48.0, 7.5, 0.0)],
        10.0,
        EdgeClass::Bridge,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let mesh = assert_production_dem_cdt_contract(
        "grade-compliant grounded bridge abutment",
        &surface,
        &graph,
        &terrain,
        (-64.0, -16.0, -32.0, 16.0),
        2.0,
        false,
        true,
    );

    assert!(
        mesh.stats.road_seam_max_slope_ratio
            <= crate::simulation::terrain::cdt::MAX_TERRAIN_TIE_IN_SLOPE_RATIO + 0.0001,
        "bridge abutment seam must remain within the terrain slope budget"
    );
    assert!(
        mesh.emitted_faces.iter().any(|face| {
            face.kind == TerrainCdtTieInKind::OrdinaryTerrain
                && face.sources.iter().copied().any(|source| {
                    matches!(
                        source,
                        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                            edge_class: crate::simulation::terrain::cdt::TerrainCdtEdgeClass::Bridge,
                            support_policy:
                                crate::simulation::terrain::cdt::TerrainCdtEarthworkSupportPolicy::BridgeEndpointAbutments,
                            ..
                        }
                    )
                })
        }),
        "production bridge-abutment faces must retain structural provenance in the terrain bucket"
    );
}

#[test]
fn connected_bridge_ramps_keep_abutment_cutouts_beside_standard_bends() {
    let terrain = flat_terrain(193, 97);
    let mut graph = RegionGraph::new();
    let west_elevated = graph.add_node(Vector3::new(-80.0, 5.0, -24.0), NodeType::Junction);
    let west_landing = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let east_landing = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let east_elevated = graph.add_node(Vector3::new(80.0, 5.0, 24.0), NodeType::Junction);
    for (start, end, points, class) in [
        (
            west_elevated,
            west_landing,
            vec![
                Vector3::new(-80.0, 5.0, -24.0),
                Vector3::new(-24.0, 0.0, 0.0),
            ],
            EdgeClass::Bridge,
        ),
        (
            west_landing,
            east_landing,
            vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
            EdgeClass::Standard,
        ),
        (
            east_landing,
            east_elevated,
            vec![Vector3::new(24.0, 0.0, 0.0), Vector3::new(80.0, 5.0, 24.0)],
            EdgeClass::Bridge,
        ),
    ] {
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

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert!(
        surface
            .compiled_visual_node_pieces()
            .contains_key(&west_landing)
            && surface
                .compiled_visual_node_pieces()
                .contains_key(&east_landing),
        "the two mixed bridge/standard landing bends must compile"
    );

    let (road_loops, source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -96.0, -48.0, 96.0, 48.0)
        .expect("connected bridge landing terrain cutouts must union successfully");
    assert!(!road_loops.is_empty());
    assert!(
        source_count >= 5,
        "both bridge abutments, both landing bends, and the grounded connector must contribute terrain cutouts; sources={source_count}"
    );
}
