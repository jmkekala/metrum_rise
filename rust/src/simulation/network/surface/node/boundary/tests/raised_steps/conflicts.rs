// SPDX-License-Identifier: GPL-2.0-only

//! Raised-step boundary conflict tests.

use super::*;

#[test]
fn unauthorized_asphalt_curb_boundary_height_conflict_still_rejects() {
    let key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let raised_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 0,
    };
    let raised_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 120,
    };
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(
        lower_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 1,
                grade_authority_index: 91,
            }),
            owner_kind: lower_owner.kind(),
            owner_index: lower_owner.owner_index(),
        }],
    );
    direct_vertex_source_candidates.insert(
        raised_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 23,
                grade_authority_index: 92,
            }),
            owner_kind: raised_owner.kind(),
            owner_index: raised_owner.owner_index(),
        }],
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = sources
        .boundary_height_mm_at_key(key)
        .expect_err("material ranks without explicit owner-pair topology must reject");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));
}

#[test]
fn raised_step_footprint_height_accepts_multiple_explicit_raised_owners_with_order_independent_sources()
 {
    let key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0)).xz_key();
    let step_end_a = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.0, 0.0)).xz_key();
    let step_end_b = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 1.0)).xz_key();
    let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 14);
    let raised_owner_a = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let raised_owner_b = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 13);
    let lower_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 0,
    };
    let raised_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 120,
    };
    let lower = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 43,
            grade_authority_index: 63,
        }),
        owner_kind: lower_owner.kind(),
        owner_index: lower_owner.owner_index(),
    };
    let raised_a = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 106,
            grade_authority_index: 64,
        }),
        owner_kind: raised_owner_a.kind(),
        owner_index: raised_owner_a.owner_index(),
    };
    let raised_b = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 115,
            grade_authority_index: 65,
        }),
        owner_kind: raised_owner_b.kind(),
        owner_index: raised_owner_b.owner_index(),
    };
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(lower_point, vec![lower]);
    direct_vertex_source_candidates.insert(raised_point, vec![raised_a, raised_b]);
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: vec![
            arrangement::NodeExplicitVerticalStepSegment::new(
                key,
                step_end_a,
                lower_owner,
                raised_owner_a,
            )
            .expect("first test step should be non-degenerate"),
            arrangement::NodeExplicitVerticalStepSegment::new(
                key,
                step_end_b,
                lower_owner,
                raised_owner_b,
            )
            .expect("second test step should be non-degenerate"),
        ],
    };

    let height_mm = sources
        .boundary_height_mm_at_key(key)
        .expect("all distinct raised owners are explicitly authorized at this footprint corner");

    assert_eq!(height_mm, Some(120));
}
