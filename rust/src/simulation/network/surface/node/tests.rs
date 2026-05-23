//! Node surface export tests.

use super::arrangement::{
    NodeArrangementEdgeId, NodeBandHeightFieldId, NodeRegionSeamConstraint, NodeSeamSource,
};
use super::backend::RoadVec2;
use super::height::{NodeGradeCarrierDecision, NodeGradeVertexAuthority};
use super::height::{NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex};
use super::*;

fn owner(kind: RoadSurfaceBandKind, owner_index: usize) -> NodeBandOwner {
    NodeBandOwner::new(kind, owner_index)
}

fn height_field(owner: NodeBandOwner) -> NodeBandHeightFieldId {
    NodeBandHeightFieldId::new(owner.owner_index(), owner.owner_index(), owner.kind())
}

fn raised_step_seam(
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    start: RoadVec2,
    end: RoadVec2,
) -> NodeRegionSeamConstraint {
    NodeRegionSeamConstraint {
        constraint_index: 7,
        seam_source: NodeSeamSource::RaisedStepContact {
            owner_index: raised_owner.owner_index(),
        },
        owner: Some(lower_owner),
        opposite_owner: Some(raised_owner),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: start,
        end_xz: end,
    }
}

fn arrangement_with_vertical_step_support(
    raised_start: RoadVec2,
    raised_end: RoadVec2,
) -> (NodeArrangement, Vec<NodeExplicitVerticalStepSegment>) {
    arrangement_with_owner_pair_vertical_step_support(
        RoadSurfaceBandKind::Carriageway,
        RoadSurfaceBandKind::CurbOrShoulder,
        raised_start,
        raised_end,
    )
}

fn arrangement_with_owner_pair_vertical_step_support(
    lower_kind: RoadSurfaceBandKind,
    raised_kind: RoadSurfaceBandKind,
    raised_start: RoadVec2,
    raised_end: RoadVec2,
) -> (NodeArrangement, Vec<NodeExplicitVerticalStepSegment>) {
    arrangement_with_owner_pair_vertical_step_support_and_heights(
        lower_kind,
        raised_kind,
        raised_start,
        raised_end,
        0.0,
        0.0,
        0.12,
        0.12,
    )
}

fn arrangement_with_owner_pair_vertical_step_support_and_heights(
    lower_kind: RoadSurfaceBandKind,
    raised_kind: RoadSurfaceBandKind,
    raised_start: RoadVec2,
    raised_end: RoadVec2,
    lower_start_height_m: f64,
    lower_end_height_m: f64,
    raised_start_height_m: f64,
    raised_end_height_m: f64,
) -> (NodeArrangement, Vec<NodeExplicitVerticalStepSegment>) {
    let lower_owner = owner(lower_kind, 0);
    let raised_owner = owner(raised_kind, 1);
    let lower_height = height_field(lower_owner);
    let raised_height = height_field(raised_owner);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let seam = raised_step_seam(lower_owner, raised_owner, start, end);
    let mut arrangement = NodeArrangement::new(42, RoadSurfaceVisualNodePieceKind::Bend);

    let lower_start = arrangement
        .insert_vertex(start, lower_start_height_m, [lower_owner], lower_height, [])
        .expect("lower start vertex is valid");
    let lower_end = arrangement
        .insert_vertex(end, lower_end_height_m, [lower_owner], lower_height, [])
        .expect("lower end vertex is valid");
    let lower_apex = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, -1.0),
            lower_start_height_m,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower apex vertex is valid");
    let lower_edge = arrangement.push_edge(
        lower_start,
        lower_end,
        lower_owner,
        lower_height,
        Some(raised_owner),
        Some(raised_height),
        false,
        false,
        true,
        NodeSeamSource::RaisedStepContact {
            owner_index: raised_owner.owner_index(),
        },
        vec![seam.constraint_index],
    );
    let lower_region = arrangement.push_region(
        lower_owner,
        lower_height,
        vec![lower_start, lower_end, lower_apex],
        Vec::new(),
        vec![lower_edge],
        1.0,
        vec![seam.clone()],
    );
    arrangement.push_face(
        lower_region,
        lower_owner,
        [lower_start, lower_end, lower_apex],
    );

    let upper_start = arrangement
        .insert_vertex(
            raised_start,
            raised_start_height_m,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("upper start vertex is valid");
    let upper_end = arrangement
        .insert_vertex(
            raised_end,
            raised_end_height_m,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("upper end vertex is valid");
    let upper_apex = arrangement
        .insert_vertex(
            RoadVec2::new(raised_start.x, 1.0),
            raised_start_height_m,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("upper apex vertex is valid");
    let upper_region = arrangement.push_region(
        raised_owner,
        raised_height,
        vec![upper_start, upper_apex, upper_end],
        Vec::new(),
        Vec::new(),
        1.0,
        vec![seam],
    );
    arrangement.push_face(
        upper_region,
        raised_owner,
        [upper_start, upper_apex, upper_end],
    );

    let segments = arrangement.explicit_vertical_step_segments();
    (arrangement, segments)
}

fn heighted_vertex_with_grade_decision(
    point_xz: RoadVec2,
    height_m: f64,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    decision: NodeGradeCarrierDecision,
) -> NodeHeightedVertex {
    NodeHeightedVertex {
        point_xz,
        height_m,
        height_field_id,
        height_authority: None,
        grade_authority: Some(NodeGradeVertexAuthority::new(
            point_xz,
            height_m,
            owner,
            height_field_id,
            decision,
        )),
    }
}

fn footprint_shapes_from_points(points: &[RoadVec2]) -> NodeOverlayShapes {
    vec![vec![
        points
            .iter()
            .copied()
            .map(backend::road_vec2_to_overlay_point)
            .collect(),
    ]]
}

fn footprint_loop_contains_xz(loop_points: &[Vector3], point_xz: RoadVec2) -> bool {
    let key = NodeArrangementKey::from_point(point_xz);
    loop_points
        .iter()
        .any(|point| ArrangementBoundaryPointKey::from_world(*point).xz_key() == key)
}

fn push_exposed_triangle_boundary_edges(
    arrangement: &mut NodeArrangement,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    vertices: [super::arrangement::NodeArrangementVertexId; 3],
) -> Vec<NodeArrangementEdgeId> {
    (0..3)
        .map(|index| {
            arrangement.push_edge(
                vertices[index],
                vertices[(index + 1) % vertices.len()],
                owner,
                height_field_id,
                None,
                None,
                true,
                false,
                false,
                NodeSeamSource::for_owner(owner),
                Vec::new(),
            )
        })
        .collect()
}

mod boundary_conflicts;
mod footprint_export;
mod height_conflicts;
mod top_sources;
mod vertical_steps;
