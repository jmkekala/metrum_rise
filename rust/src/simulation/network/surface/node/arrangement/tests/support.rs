// SPDX-License-Identifier: GPL-2.0-only

//! Shared fixtures for node-arrangement tests.

use super::*;

pub(super) fn owner(kind: RoadSurfaceBandKind, owner_index: usize) -> NodeBandOwner {
    NodeBandOwner::new(kind, owner_index)
}
pub(super) fn height_field_id(
    kind: RoadSurfaceBandKind,
    band_index: usize,
) -> NodeBandHeightFieldId {
    NodeBandHeightFieldId::new(0, band_index, kind)
}
pub(super) fn seam_source(owner_index: usize) -> NodeSeamSource {
    NodeSeamSource::FootprintBoundary { owner_index }
}
pub(super) fn two_region_height_solution(
    carriageway: NodeBandOwner,
    sidewalk: NodeBandOwner,
    carriageway_seams: Vec<NodeRegionSeamConstraint>,
    sidewalk_seams: Vec<NodeRegionSeamConstraint>,
) -> NodeHeightSolution {
    two_region_height_solution_with_material_heights(
        carriageway,
        sidewalk,
        0.0,
        0.0,
        carriageway_seams,
        sidewalk_seams,
    )
}
pub(super) fn two_region_height_solution_with_material_heights(
    carriageway: NodeBandOwner,
    sidewalk: NodeBandOwner,
    carriageway_height_m: f64,
    sidewalk_height_m: f64,
    carriageway_seams: Vec<NodeRegionSeamConstraint>,
    sidewalk_seams: Vec<NodeRegionSeamConstraint>,
) -> NodeHeightSolution {
    NodeHeightSolution {
        node_id: 11,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![
            test_height_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![
                    height_vertex(0.0, 0.0, carriageway_height_m),
                    height_vertex(1.0, 0.0, carriageway_height_m),
                    height_vertex(1.0, 1.0, carriageway_height_m),
                    height_vertex(0.0, 1.0, carriageway_height_m),
                ],
                carriageway_seams,
            ),
            test_height_region_with_seams(
                RoadSurfaceBandKind::Sidewalk,
                sidewalk,
                vec![
                    height_vertex(1.0, 0.0, sidewalk_height_m),
                    height_vertex(2.0, 0.0, sidewalk_height_m),
                    height_vertex(2.0, 1.0, sidewalk_height_m),
                    height_vertex(1.0, 1.0, sidewalk_height_m),
                ],
                sidewalk_seams,
            ),
        ],
    }
}
pub(super) fn test_height_region_with_seams(
    kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    contour: Vec<NodeHeightedVertex>,
    seam_constraints: Vec<NodeRegionSeamConstraint>,
) -> NodeHeightedRegion {
    let height_field_id =
        NodeBandHeightFieldId::new(owner.owner_index(), owner.owner_index(), kind);
    let contour = contour
        .into_iter()
        .map(|mut vertex| {
            vertex.height_field_id = height_field_id;
            vertex.grade_authority = Some(NodeGradeVertexAuthority::new(
                vertex.point_xz,
                vertex.height_m,
                owner,
                height_field_id,
                NodeGradeCarrierDecision::SourceCarrier { authority: None },
            ));
            vertex
        })
        .collect();
    NodeHeightedRegion {
        kind,
        owner,
        height_field_id,
        shape: vec![contour],
        area_m2: 1.0,
        seam_constraints,
    }
}
pub(super) fn height_vertex(x: f64, z: f64, height_m: f64) -> NodeHeightedVertex {
    NodeHeightedVertex {
        point_xz: RoadVec2::new(x, z),
        height_m,
        height_field_id: height_field_id(RoadSurfaceBandKind::Sidewalk, 0),
        height_authority: None,
        source_provenance: None,
        grade_authority: None,
    }
}
