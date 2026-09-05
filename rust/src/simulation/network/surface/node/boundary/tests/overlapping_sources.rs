// SPDX-License-Identifier: GPL-2.0-only

//! Overlapping source-edge provenance tests.

use super::*;

#[test]
fn overlapping_source_edges_with_distinct_provenance_are_rejected() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Carriageway,
            3,
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            3,
            30,
            3,
            31,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            6,
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            4,
            40,
            4,
            41,
        ),
    ];
    let mut segments = Vec::new();

    let error = push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(RoadVec3::new(0.0, 0.0, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect_err("coincident source edges with different provenance must not pick sorted first");

    assert!(matches!(
        error,
        NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource { .. }
    ));
}

#[test]
fn overlapping_source_edges_with_identical_boundary_provenance_are_accepted() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            10,
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            47,
            3,
            23,
            11,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            10,
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            47,
            3,
            23,
            11,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(RoadVec3::new(0.0, 0.0, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("identical boundary provenance must merge independent of source ordering");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: NodeFootprintBoundaryVertexSource::Direct(
                    NodeFootprintBoundaryDirectSource {
                        top_surface_source_index: 47,
                        grade_authority_index: 3,
                    },
                ),
                end: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                    top_surface_source_index: 23,
                    grade_authority_index: 11,
                },),
            }),
            ..
        }
    ));
}

#[test]
fn overlapping_non_adjacent_material_edges_with_identical_boundary_provenance_use_outer_owner() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Carriageway,
            2,
            RoadVec3::new(-5.0, 150.0, 0.0),
            RoadVec3::new(0.0, 150.0, 0.0),
            89,
            0,
            90,
            0,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            11,
            RoadVec3::new(-5.0, 150.0, 0.0),
            RoadVec3::new(0.0, 150.0, 0.0),
            89,
            0,
            90,
            0,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::Bend,
        test_boundary_point(RoadVec3::new(-5.0, 150.0, 0.0)),
        test_boundary_point(RoadVec3::new(0.0, 150.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("identical final-footprint provenance must choose the outer raised-step owner");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 11,
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: NodeFootprintBoundaryVertexSource::Direct(
                    NodeFootprintBoundaryDirectSource {
                        top_surface_source_index: 89,
                        grade_authority_index: 0,
                    },
                ),
                end: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                    top_surface_source_index: 90,
                    grade_authority_index: 0,
                },),
            }),
            ..
        }
    ));
}

#[test]
fn overlapping_source_edges_with_distinct_height_carriers_are_rejected() {
    let mut first = test_source_edge_for_owner(
        RoadSurfaceBandKind::CurbOrShoulder,
        10,
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(2.0, 0.0, 0.0),
        47,
        3,
        23,
        11,
    );
    let mut second = first;
    first.height_field_id =
        arrangement::NodeBandHeightFieldId::new(0, 10, RoadSurfaceBandKind::CurbOrShoulder);
    second.height_field_id =
        arrangement::NodeBandHeightFieldId::new(1, 10, RoadSurfaceBandKind::CurbOrShoulder);
    let source_edges = vec![first, second];
    let mut segments = Vec::new();

    let error = push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(RoadVec3::new(0.0, 0.0, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect_err("matching boundary provenance still needs one height carrier");

    assert!(matches!(
        error,
        NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource { .. }
    ));
}

#[test]
fn overlapping_adjacent_material_edges_with_one_same_height_handoff_source_are_accepted() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            10,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            47,
            3,
            23,
            11,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            11,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            69,
            4,
            23,
            11,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(RoadVec3::new(0.0, 0.12, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.12, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("same-height adjacent material handoff may share one canonical endpoint");

    assert_eq!(segments.len(), 1);
}

#[test]
fn overlapping_adjacent_material_edges_with_one_sloped_handoff_source_are_accepted() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            10,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            47,
            3,
            23,
            11,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            11,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            69,
            4,
            23,
            11,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(RoadVec3::new(0.0, 0.12, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("same-mouth adjacent material handoff may share one canonical endpoint");

    assert_eq!(segments.len(), 1);
}

#[test]
fn overlapping_adjacent_material_edges_with_distinct_endpoint_sources_use_canonical_segment() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            7,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            31,
            24,
            32,
            9,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            40,
            14,
            41,
            25,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::Bend,
        test_boundary_point(RoadVec3::new(0.0, 0.12, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.12, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("adjacent material source edges may canonicalize both explicit endpoints");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: 0,
                    z_key: 0,
                    y_mm: 120,
                },
                end: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: 2_000_000,
                    z_key: 0,
                    y_mm: 120,
                },
            }),
            ..
        }
    ));
}

#[test]
fn overlapping_source_edges_with_same_owner_distinct_provenance_are_rejected() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            3,
            30,
            3,
            31,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            4,
            40,
            4,
            41,
        ),
    ];
    let mut segments = Vec::new();

    let error = push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(RoadVec3::new(0.0, 0.0, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect_err("same owner must not hide distinct boundary provenance");

    assert!(matches!(
        error,
        NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource { .. }
    ));
}

#[test]
fn overlapping_same_material_edges_with_equal_boundary_heights_use_canonical_segment_source() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            58,
            82,
            57,
            56,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            6,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            62,
            68,
            62,
            57,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(RoadVec3::new(0.0, 0.12, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.12, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("same-material equal-height source edges can normalize to a canonical segment source");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: 0,
                    z_key: 0,
                    y_mm: 120,
                },
                end: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: 2_000_000,
                    z_key: 0,
                    y_mm: 120,
                },
            }),
            ..
        }
    ));
}

#[test]
fn canonical_boundary_segment_source_matches_later_direct_source_at_same_keys() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            58,
            82,
            57,
            56,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            6,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            62,
            68,
            62,
            57,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            91,
            92,
            91,
            93,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(RoadVec3::new(0.0, 0.12, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.12, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("canonical boundary point identity should absorb later direct source at same key");

    assert_eq!(segments.len(), 1);
}

#[test]
fn partially_canonical_same_owner_segment_absorbs_later_direct_source_at_same_keys() {
    let mut source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            7,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            31,
            24,
            32,
            9,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            31,
            24,
            41,
            25,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            40,
            14,
            41,
            25,
        ),
    ];
    source_edges[2].height_field_id =
        arrangement::NodeBandHeightFieldId::new(1, 5, RoadSurfaceBandKind::Sidewalk);
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::Bend,
        test_boundary_point(RoadVec3::new(0.0, 0.12, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.12, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("partially canonical same-owner output may canonicalize the remaining endpoint");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: 0,
                    z_key: 0,
                    y_mm: 120,
                },
                end: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: 2_000_000,
                    z_key: 0,
                    y_mm: 120,
                },
            }),
            ..
        }
    ));
}

#[test]
fn partially_canonical_same_owner_segment_matches_generated_bend_boundary_keys() {
    let start = RoadVec3::new(-8.931965, 149.86, 8.170613);
    let end = RoadVec3::new(-7.159239, 149.798, 5.100163);
    let mut source_edges = vec![
        test_source_edge_for_owner_and_kind(
            RoadSurfaceVisualNodePieceKind::Bend,
            RoadSurfaceBandKind::CurbOrShoulder,
            7,
            start,
            end,
            47,
            1,
            72,
            8,
        ),
        test_source_edge_for_owner_and_kind(
            RoadSurfaceVisualNodePieceKind::Bend,
            RoadSurfaceBandKind::Sidewalk,
            11,
            start,
            end,
            47,
            1,
            73,
            9,
        ),
        test_source_edge_for_owner_and_kind(
            RoadSurfaceVisualNodePieceKind::Bend,
            RoadSurfaceBandKind::Sidewalk,
            11,
            start,
            end,
            71,
            2,
            73,
            9,
        ),
    ];
    source_edges[2].height_field_id =
        arrangement::NodeBandHeightFieldId::new(1, 11, RoadSurfaceBandKind::Sidewalk);
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        0,
        RoadSurfaceVisualNodePieceKind::Bend,
        test_boundary_point(start),
        test_boundary_point(end),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("generated Bend canonical footprint segment should canonicalize same-owner carriers");

    assert_eq!(segments.len(), 1);
}
