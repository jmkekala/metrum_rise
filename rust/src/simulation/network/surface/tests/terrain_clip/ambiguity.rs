//! Terrain clip ambiguity rejection tests.

use super::*;

#[test]
fn terrain_clip_union_rejects_ambiguous_source_chain_recovery() {
    let p0 = Vector3::new(0.0, 9.0, 0.0);
    let p1_a = Vector3::new(0.45, 9.4, 0.18);
    let p1_b = Vector3::new(0.55, 9.6, -0.16);
    let p2 = Vector3::new(1.0, 10.0, 0.0);
    let p3 = Vector3::new(1.0, 10.0, 0.5);
    let p4 = Vector3::new(0.0, 9.0, 0.5);
    let loop_for_node = |node_id, midpoint| RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_node_test(p0, midpoint, node_id),
            terrain_clip_source_edge_for_node_test(midpoint, p2, node_id),
            terrain_clip_source_edge_for_node_test(p2, p3, node_id),
            terrain_clip_source_edge_for_node_test(p3, p4, node_id),
            terrain_clip_source_edge_for_node_test(p4, p0, node_id),
        ],
        points_world: vec![p0, p2, p3, p4],
    };

    let unioned = RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&[
        loop_for_node(1, p1_a),
        loop_for_node(1, p1_b),
    ]);

    let Err(RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner { context, .. }) = unioned
    else {
        panic!("ambiguous source-chain recovery must reject provenance, got {unioned:?}");
    };
    assert!(
        context.contains("ambiguous_source_chain"),
        "ambiguous source-chain diagnostic should stay visible: {context}"
    );
}

#[test]
fn terrain_clip_union_rejects_matching_height_output_source_ambiguity() {
    let y = 8.0;
    let points = vec![
        Vector3::new(0.0, y, 0.0),
        Vector3::new(2.0, y, 0.0),
        Vector3::new(2.0, y, 1.0),
        Vector3::new(0.0, y, 1.0),
    ];
    let raw_clip_sources = vec![
        terrain_clip_loop_for_node_test(&points, 1),
        terrain_clip_loop_for_node_test(&points, 2),
    ];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources);

    let Err(RoadSurfaceTerrainClipExportError::AmbiguousOutputBoundaryOwner { context, .. }) =
        unioned
    else {
        panic!(
            "coincident matching-height source edges with different provenance must reject, got {unioned:?}"
        );
    };
    assert!(
        context.contains("sources_disagree"),
        "ambiguous output source diagnostic should name provenance disagreement: {context}"
    );
}

#[test]
fn terrain_clip_union_rejects_output_source_ambiguity_across_kind_priority() {
    let y = 8.0;
    let points = vec![
        Vector3::new(0.0, y, 0.0),
        Vector3::new(2.0, y, 0.0),
        Vector3::new(2.0, y, 1.0),
        Vector3::new(0.0, y, 1.0),
    ];
    let raw_clip_sources = vec![
        terrain_clip_loop_for_node_kind_test(
            &points,
            1,
            RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
        ),
        terrain_clip_loop_for_node_kind_test(
            &points,
            2,
            RoadSurfaceTerrainClipEdgeKind::FootprintBoundary,
        ),
    ];

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources);

    let Err(RoadSurfaceTerrainClipExportError::AmbiguousOutputBoundaryOwner { context, .. }) =
        unioned
    else {
        panic!(
            "output source selection must reject differing provenance instead of choosing the highest-priority edge kind, got {unioned:?}"
        );
    };
    assert!(
        context.contains("sources_disagree"),
        "ambiguous output source diagnostic should name cross-kind provenance disagreement: {context}"
    );
}

#[test]
fn terrain_clip_union_rejects_dust_connector_conflicting_same_xz_heights() {
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
    let conflict_a0 = Vector3::new(d1.x - 0.0002, 20.0, d1.z);
    let conflict_a1 = Vector3::new(d1.x + 0.0002, 20.0, d1.z);
    let conflict_b0 = Vector3::new(d1.x - 0.0002, 21.0, d1.z);
    let conflict_b1 = Vector3::new(d1.x + 0.0002, 21.0, d1.z);
    let raw_clip_sources = vec![RoadSurfaceTerrainClipLoop {
        source_edges: vec![
            terrain_clip_source_edge_for_test(p0, p1),
            terrain_clip_source_edge_for_test(p2, p3),
            terrain_clip_source_edge_for_test(p3, p4),
            terrain_clip_source_edge_for_test(p4, p5),
            terrain_clip_source_edge_for_test(p5, p0),
            terrain_clip_source_edge_for_test(conflict_a0, conflict_a1),
            terrain_clip_source_edge_for_test(conflict_b0, conflict_b1),
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

    let unioned =
        RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&raw_clip_sources);

    let Err(RoadSurfaceTerrainClipExportError::AmbiguousDustConnectorHeight { context, .. }) =
        unioned
    else {
        panic!(
            "dust connector with conflicting same-XZ source heights must reject, got {unioned:?}"
        );
    };
    assert!(
        context.contains("conflicting_source_heights"),
        "dust connector height ambiguity should name conflicting height keys: {context}"
    );
}
