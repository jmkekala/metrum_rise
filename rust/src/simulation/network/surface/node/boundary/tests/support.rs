// SPDX-License-Identifier: GPL-2.0-only

//! Shared fixtures for node boundary tests.

use super::*;

pub(in crate::simulation::network::surface::node::boundary::tests) fn test_boundary_point(
    point: RoadVec3,
) -> NodeFootprintBoundaryPoint {
    NodeFootprintBoundaryPoint::new(ArrangementBoundaryPointKey::from_world(point))
}

pub(in crate::simulation::network::surface::node::boundary::tests) fn test_source_edge(
    start: RoadVec3,
    end: RoadVec3,
    start_top_surface_source_index: usize,
    start_grade_authority_index: usize,
    end_top_surface_source_index: usize,
    end_grade_authority_index: usize,
) -> NodeEarthworkBoundarySourceEdge {
    test_source_edge_for_owner(
        RoadSurfaceBandKind::Sidewalk,
        5,
        start,
        end,
        start_top_surface_source_index,
        start_grade_authority_index,
        end_top_surface_source_index,
        end_grade_authority_index,
    )
}

pub(in crate::simulation::network::surface::node::boundary::tests) fn test_source_edge_for_owner(
    owner_kind: RoadSurfaceBandKind,
    owner_index: usize,
    start: RoadVec3,
    end: RoadVec3,
    start_top_surface_source_index: usize,
    start_grade_authority_index: usize,
    end_top_surface_source_index: usize,
    end_grade_authority_index: usize,
) -> NodeEarthworkBoundarySourceEdge {
    test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        owner_kind,
        owner_index,
        start,
        end,
        start_top_surface_source_index,
        start_grade_authority_index,
        end_top_surface_source_index,
        end_grade_authority_index,
    )
}

pub(in crate::simulation::network::surface::node::boundary::tests) fn test_source_edge_for_owner_and_kind(
    kind: RoadSurfaceVisualNodePieceKind,
    owner_kind: RoadSurfaceBandKind,
    owner_index: usize,
    start: RoadVec3,
    end: RoadVec3,
    start_top_surface_source_index: usize,
    start_grade_authority_index: usize,
    end_top_surface_source_index: usize,
    end_grade_authority_index: usize,
) -> NodeEarthworkBoundarySourceEdge {
    let start_point_key = ArrangementBoundaryPointKey::from_world(start);
    let end_point_key = ArrangementBoundaryPointKey::from_world(end);
    NodeEarthworkBoundarySourceEdge {
        start_point_key,
        end_point_key,
        start_key: start_point_key.xz_key(),
        end_key: end_point_key.xz_key(),
        final_footprint_boundary: false,
        node_id: 11,
        kind,
        owner_kind,
        owner_index,
        height_field_id: arrangement::NodeBandHeightFieldId::new(0, owner_index, owner_kind),
        start_source: NodeFootprintBoundaryDirectSource {
            top_surface_source_index: start_top_surface_source_index,
            grade_authority_index: start_grade_authority_index,
        },
        end_source: NodeFootprintBoundaryDirectSource {
            top_surface_source_index: end_top_surface_source_index,
            grade_authority_index: end_grade_authority_index,
        },
    }
}
