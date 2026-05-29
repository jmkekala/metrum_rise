//! Terrain clip ambiguity rejection tests.

use super::*;
use crate::simulation::network::surface::backend::RoadVec3;
use crate::simulation::network::surface::{
    NodeFootprintBoundaryDirectSource, NodeFootprintBoundarySegmentSource,
    RoadSurfaceVisualNodePieceKind,
};

#[test]
fn terrain_clip_union_rejects_ambiguous_source_chain_recovery() {
    let p0 = RoadVec3::new(0.0, 9.0, 0.0);
    let p1_a = RoadVec3::new(0.45, 9.4, 0.18);
    let p1_b = RoadVec3::new(0.55, 9.6, -0.16);
    let p2 = RoadVec3::new(1.0, 10.0, 0.0);
    let p3 = RoadVec3::new(1.0, 10.0, 0.5);
    let p4 = RoadVec3::new(0.0, 9.0, 0.5);
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
        RoadVec3::new(0.0, y, 0.0),
        RoadVec3::new(2.0, y, 0.0),
        RoadVec3::new(2.0, y, 1.0),
        RoadVec3::new(0.0, y, 1.0),
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
fn terrain_clip_union_materializes_same_material_node_owner_boundary_handoff() {
    let y = 8.0;
    let points = vec![
        RoadVec3::new(0.0, y, 0.0),
        RoadVec3::new(2.0, y, 0.0),
        RoadVec3::new(2.0, y, 1.0),
        RoadVec3::new(0.0, y, 1.0),
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
    .expect("same-material different-owner overlap should export canonical handoff provenance");

    assert_eq!(unioned.len(), 1);
    assert!(
        unioned
            .iter()
            .flat_map(|loop_| loop_.source_edges.iter())
            .all(|edge| matches!(
                edge.source,
                RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff {
                    node_id: 1,
                    kind: RoadSurfaceVisualNodePieceKind::JunctionN,
                    owner_kind: RoadSurfaceBandKind::Sidewalk,
                    owner_index_a: 1,
                    owner_index_b: 2,
                    boundary_source: Some(NodeFootprintBoundarySegmentSource {
                        start: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. },
                        end: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. },
                    }),
                }
            )),
        "same-material different-owner output segment must keep canonical handoff provenance: {unioned:?}"
    );
}

#[test]
fn terrain_clip_union_materializes_same_node_owner_adjacent_boundary_sources() {
    let y = 8.0;
    let points = vec![
        RoadVec3::new(0.0, y, 0.0),
        RoadVec3::new(2.0, y, 0.0),
        RoadVec3::new(2.0, y, 1.0),
        RoadVec3::new(0.0, y, 1.0),
    ];
    let loop_for_source = |top_surface_source_index| RoadSurfaceTerrainClipLoop {
        source_edges: points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .enumerate()
            .map(|(index, (&start, &end))| {
                same_material_node_owner_source_edge(
                    start,
                    end,
                    6,
                    top_surface_source_index,
                    index * 2,
                    index * 2 + 1,
                )
            })
            .collect(),
        points_world: points.clone(),
    };
    let unioned = RoadSurfaceSystem::union_terrain_clip_boundary_loops_with_sources(&[
        loop_for_source(421),
        loop_for_source(422),
    ])
    .expect("same node owner boundary-source overlap should canonicalize the output source");

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
                    owner_index: 6,
                    boundary_source: Some(NodeFootprintBoundarySegmentSource {
                        start: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. },
                        end: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. },
                    }),
                }
            )),
        "same-owner output segment must keep canonical node boundary provenance: {unioned:?}"
    );
}

#[test]
fn terrain_clip_union_rejects_adjacent_node_owner_boundary_source_ambiguity() {
    let y = 8.0;
    let points = vec![
        RoadVec3::new(0.0, y, 0.0),
        RoadVec3::new(2.0, y, 0.0),
        RoadVec3::new(2.0, y, 1.0),
        RoadVec3::new(0.0, y, 1.0),
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
    ]);

    let Err(RoadSurfaceTerrainClipExportError::AmbiguousOutputBoundaryOwner { context, .. }) =
        unioned
    else {
        panic!("adjacent different-owner overlap must reject provenance, got {unioned:?}");
    };
    assert!(
        context.contains("sources_disagree"),
        "ambiguous adjacent owner diagnostic should name provenance disagreement: {context}"
    );
}

#[test]
fn terrain_clip_union_rejects_output_source_ambiguity_across_kind_priority() {
    let y = 8.0;
    let points = vec![
        RoadVec3::new(0.0, y, 0.0),
        RoadVec3::new(2.0, y, 0.0),
        RoadVec3::new(2.0, y, 1.0),
        RoadVec3::new(0.0, y, 1.0),
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
    start: RoadVec3,
    end: RoadVec3,
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
    start: RoadVec3,
    end: RoadVec3,
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
    let p0 = RoadVec3::new(0.0, 10.0, 0.0);
    let p1 = RoadVec3::new(0.5, 10.5, 0.0);
    let d0 = RoadVec3::new(0.50002, raw_boundary_y, 0.00008);
    let d1 = RoadVec3::new(0.49998, raw_boundary_y, 0.00016);
    let d2 = RoadVec3::new(0.50001, raw_boundary_y, 0.00024);
    let p2 = RoadVec3::new(0.5, 10.7, 0.00032);
    let p3 = RoadVec3::new(1.0, 11.0, 0.0);
    let p4 = RoadVec3::new(1.0, 11.0, 0.1);
    let p5 = RoadVec3::new(0.0, 10.0, 0.1);
    let conflict_a0 = RoadVec3::new(d1.x - 0.0002, 20.0, d1.z);
    let conflict_a1 = RoadVec3::new(d1.x + 0.0002, 20.0, d1.z);
    let conflict_b0 = RoadVec3::new(d1.x - 0.0002, 21.0, d1.z);
    let conflict_b1 = RoadVec3::new(d1.x + 0.0002, 21.0, d1.z);
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
            RoadVec3::new(p0.x, raw_boundary_y, p0.z),
            RoadVec3::new(p1.x, raw_boundary_y, p1.z),
            d0,
            d1,
            d2,
            RoadVec3::new(p2.x, raw_boundary_y, p2.z),
            RoadVec3::new(p3.x, raw_boundary_y, p3.z),
            RoadVec3::new(p4.x, raw_boundary_y, p4.z),
            RoadVec3::new(p5.x, raw_boundary_y, p5.z),
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
