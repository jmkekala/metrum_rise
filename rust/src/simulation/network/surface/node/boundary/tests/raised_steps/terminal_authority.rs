//! Terminal raised-step source-edge authority tests.

use super::*;
use crate::simulation::network::surface::height::NodeHeightCarrierProvenanceKey;

#[test]
fn terminal_raised_step_footprint_height_accepts_boundary_edge_authority() {
    let step_start = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let lower_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::Carriageway,
        0,
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let raised_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        RoadVec3::new(0.0, 0.12, 0.0),
        RoadVec3::new(-1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(step_start)
        .expect("terminal source-edge endpoints should authorize raised footprint corner");

    assert_eq!(height_mm, Some(120));
}

#[test]
fn final_boundary_raised_step_footprint_height_accepts_endpoint_edge_authority() {
    let step_start = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let mut lower_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        RoadSurfaceBandKind::Carriageway,
        0,
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let mut raised_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        RoadVec3::new(0.0, 0.12, 0.0),
        RoadVec3::new(-1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    lower_edge.final_footprint_boundary = true;
    raised_edge.final_footprint_boundary = true;
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(step_start)
        .expect("final boundary source-edge endpoints should authorize raised footprint corner");

    assert_eq!(height_mm, Some(120));
}

#[test]
fn final_boundary_rank_gap_footprint_height_rejects_endpoint_edge_authority() {
    let step_start = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let mut lower_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        RoadSurfaceBandKind::Carriageway,
        0,
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let mut raised_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        RoadSurfaceBandKind::Sidewalk,
        1,
        RoadVec3::new(0.0, 0.12, 0.0),
        RoadVec3::new(-1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    lower_edge.final_footprint_boundary = true;
    raised_edge.final_footprint_boundary = true;
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = sources
        .height_mm_at_key(step_start)
        .expect_err("final boundary endpoint proof cannot invent a missing curb step");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));
}

#[test]
fn final_boundary_source_intersection_rank_gap_accepts_endpoint_edge_authority() {
    let step_start = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let mut lower_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        RoadSurfaceBandKind::Carriageway,
        0,
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let mut raised_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        RoadSurfaceBandKind::Sidewalk,
        1,
        RoadVec3::new(0.0, 0.12, 0.0),
        RoadVec3::new(-1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    lower_edge.final_footprint_boundary = true;
    raised_edge.final_footprint_boundary = true;
    let final_height_edges = vec![
        test_final_height_edge_for_source_edge(lower_edge),
        test_final_height_edge_for_source_edge(raised_edge),
    ];
    let mut grade_authority_source_provenance = vec![None; 42];
    grade_authority_source_provenance[30] = Some(test_source_intersection_provenance(
        RoadSurfaceBandKind::Carriageway,
        0,
    ));
    grade_authority_source_provenance[31] = grade_authority_source_provenance[30];
    grade_authority_source_provenance[40] = Some(test_source_intersection_provenance(
        RoadSurfaceBandKind::Sidewalk,
        1,
    ));
    grade_authority_source_provenance[41] = grade_authority_source_provenance[40];
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        final_height_edges,
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance,
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(step_start)
        .expect("source-intersection final endpoints can authorize collapsed rank-gap corners");

    assert_eq!(height_mm, Some(120));
}

#[test]
fn terminal_raised_step_footprint_height_rejects_endpoint_quantization_drift() {
    let drifted_step_start =
        ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.000001, 0.0, 0.0)).xz_key();
    let lower_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::Carriageway,
        0,
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let raised_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        RoadVec3::new(0.0, 0.12, 0.0),
        RoadVec3::new(1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = sources
        .height_mm_at_key(drifted_step_start)
        .expect_err("terminal endpoint-scale quantization drift must not authorize raised corners");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));
}

fn test_final_height_edge_for_source_edge(
    source_edge: NodeEarthworkBoundarySourceEdge,
) -> NodeFinalFootprintBoundaryHeightEdge {
    NodeFinalFootprintBoundaryHeightEdge {
        start_point_key: source_edge.start_point_key,
        end_point_key: source_edge.end_point_key,
        owner_kind: source_edge.owner_kind,
        owner_index: source_edge.owner_index,
    }
}

fn test_source_intersection_provenance(
    kind: RoadSurfaceBandKind,
    owner_index: usize,
) -> NodeHeightCarrierProvenanceKey {
    let owner = arrangement::NodeBandOwner::new(kind, owner_index);
    let height_field_id = arrangement::NodeBandHeightFieldId::new(owner_index, owner_index, kind);
    NodeHeightCarrierProvenanceKey {
        owner,
        source_kind: kind,
        source_mouth_order_index: owner_index,
        source_band_index: owner_index,
        height_field_id,
        claim_priority:
            super::super::super::super::rails::NodeGeneratedContourClaimPriority::MouthBand,
        point: super::super::super::super::ownership::NodeOwnedRegionArrangementKey::from_point(
            RoadVec2::new(0.0, 0.0),
        ),
        origin:
            super::super::super::super::ownership::NodeCarrierProvenanceOrigin::SourceIntersection {
                peer_count: 1,
            },
    }
}

#[test]
fn terminal_raised_step_footprint_height_rejects_interior_edge_authority() {
    let step_midpoint =
        ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.5, 0.0, 0.0)).xz_key();
    let lower_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::Carriageway,
        0,
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let raised_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        RoadVec3::new(0.0, 0.12, 0.0),
        RoadVec3::new(1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = sources
        .height_mm_at_key(step_midpoint)
        .expect_err("terminal source-edge authority is limited to footprint corners");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));
}
