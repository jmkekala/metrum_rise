//! Terrain clip ambiguity rejection tests.

use super::*;
use crate::simulation::network::surface::{
    NodeFootprintBoundaryDirectSource, NodeFootprintBoundarySegmentSource,
};

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
fn terrain_clip_union_materializes_same_material_node_owner_boundary_source() {
    let y = 8.0;
    let points = vec![
        Vector3::new(0.0, y, 0.0),
        Vector3::new(2.0, y, 0.0),
        Vector3::new(2.0, y, 1.0),
        Vector3::new(0.0, y, 1.0),
    ];
    let loop_for_owner = |owner_index, top_surface_source_index| RoadSurfaceTerrainClipLoop {
        source_edges: points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .enumerate()
            .map(|(index, (&start, &end))| {
                same_material_node_owner_source_edge(
                    start,
                    end,
                    owner_index,
                    top_surface_source_index,
                    index * 2,
                    index * 2 + 1,
                )
            })
            .collect(),
        points_world: points.clone(),
    };
    let unioned = RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&[
        loop_for_owner(2, 20),
        loop_for_owner(1, 21),
    ])
    .expect("same-material node owner overlap should canonicalize the output source");

    assert_eq!(unioned.len(), 1);
    assert!(
        unioned
            .iter()
            .flat_map(|loop_| loop_.source_edges.iter())
            .all(|edge| matches!(
                edge.source,
                RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                    node_id: 1,
                    kind: RoadSurfaceVisualNodePieceKind::JunctionN,
                    owner_kind: RoadSurfaceBandKind::Sidewalk,
                    owner_index: 1,
                    boundary_source: Some(NodeFootprintBoundarySegmentSource {
                        start: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. },
                        end: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. },
                    }),
                }
            )),
        "same-material overlap must preserve canonical boundary-point provenance: {unioned:?}"
    );
}

#[test]
fn terrain_clip_union_materializes_adjacent_node_owner_boundary_source() {
    let y = 8.0;
    let points = vec![
        Vector3::new(0.0, y, 0.0),
        Vector3::new(2.0, y, 0.0),
        Vector3::new(2.0, y, 1.0),
        Vector3::new(0.0, y, 1.0),
    ];
    let loop_for_owner =
        |owner_kind, owner_index, top_surface_source_index| RoadSurfaceTerrainClipLoop {
            source_edges: points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .enumerate()
                .map(|(index, (&start, &end))| {
                    node_owner_source_edge(
                        start,
                        end,
                        owner_kind,
                        owner_index,
                        top_surface_source_index,
                        index * 2,
                        index * 2 + 1,
                    )
                })
                .collect(),
            points_world: points.clone(),
        };
    let unioned = RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&[
        loop_for_owner(RoadSurfaceBandKind::CurbOrShoulder, 13, 20),
        loop_for_owner(RoadSurfaceBandKind::Sidewalk, 12, 21),
    ])
    .expect("adjacent node owner overlap should canonicalize the output source");

    assert!(
        unioned
            .iter()
            .flat_map(|loop_| loop_.source_edges.iter())
            .all(|edge| {
                edge.kind == RoadSurfaceTerrainClipEdgeKind::SidewalkOuter
                    && matches!(
                        edge.source,
                        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                            node_id: 1,
                            kind: RoadSurfaceVisualNodePieceKind::JunctionN,
                            owner_kind: RoadSurfaceBandKind::Sidewalk,
                            owner_index: 12,
                            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                                start: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. },
                                end: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. },
                            }),
                        }
                    )
            }),
        "adjacent raised-step overlap must preserve canonical sidewalk boundary provenance: {unioned:?}"
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

fn same_material_node_owner_source_edge(
    start: Vector3,
    end: Vector3,
    owner_index: usize,
    top_surface_source_index: usize,
    start_grade_authority_index: usize,
    end_grade_authority_index: usize,
) -> RoadSurfaceTerrainClipSourceEdge {
    node_owner_source_edge(
        start,
        end,
        RoadSurfaceBandKind::Sidewalk,
        owner_index,
        top_surface_source_index,
        start_grade_authority_index,
        end_grade_authority_index,
    )
}

fn node_owner_source_edge(
    start: Vector3,
    end: Vector3,
    owner_kind: RoadSurfaceBandKind,
    owner_index: usize,
    top_surface_source_index: usize,
    start_grade_authority_index: usize,
    end_grade_authority_index: usize,
) -> RoadSurfaceTerrainClipSourceEdge {
    RoadSurfaceTerrainClipSourceEdge {
        start,
        end,
        kind: match owner_kind {
            RoadSurfaceBandKind::Sidewalk => RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
            RoadSurfaceBandKind::CurbOrShoulder => RoadSurfaceTerrainClipEdgeKind::ShoulderOuter,
            _ => RoadSurfaceTerrainClipEdgeKind::FootprintBoundary,
        },
        source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id: 1,
            kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            owner_kind,
            owner_index,
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: NodeFootprintBoundaryVertexSource::Direct(
                    NodeFootprintBoundaryDirectSource {
                        top_surface_source_index,
                        grade_authority_index: start_grade_authority_index,
                    },
                ),
                end: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                    top_surface_source_index,
                    grade_authority_index: end_grade_authority_index,
                }),
            }),
        },
    }
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
