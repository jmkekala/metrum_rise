//! Generated contour emission from rail paths.

use super::cleaning::cleaned_closed_contour;
use super::constraints::push_constraint;
use super::height_points::align_height_points_to_contour;
use super::paths::{
    append_world_path_points, push_world_path_point, remove_closing_world_path_duplicate,
};
use super::*;

pub(in crate::simulation::network::surface::node::rails) fn push_path_strip_contours(
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    validate_paired_path_band_height_carrier(
        kind,
        mouth_order_index,
        band_index,
        start_path_world,
        end_path_world,
    )?;
    let mut first_error = None;
    let mut pushed = false;
    for points_world in path_strip_contours_world(start_path_world, end_path_world) {
        let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
        match push_generated_contour_with_purpose(
            kind,
            purpose,
            mouth_order_index,
            band_index,
            owner,
            claim_priority,
            constraint_kind,
            points,
            Some(points_world),
            contours,
            constraints,
        ) {
            Ok(()) => pushed = true,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        };
    }
    if pushed {
        Ok(())
    } else {
        Err(
            first_error.unwrap_or(NodeRailGenerationError::DegenerateContour {
                kind,
                mouth_order_index,
                band_index,
                area_m2: 0.0,
                vertex_count: 0,
            }),
        )
    }
}

pub(in crate::simulation::network::surface::node::rails) fn push_generated_contour(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    points: Vec<RoadVec2>,
    height_points_world: Option<Vec<RoadVec3>>,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let purpose = default_generated_contour_purpose(kind);
    push_generated_contour_with_purpose(
        kind,
        purpose,
        mouth_order_index,
        band_index,
        owner,
        claim_priority,
        constraint_kind,
        points,
        height_points_world,
        contours,
        constraints,
    )
}

pub(in crate::simulation::network::surface::node::rails) fn push_generated_contour_with_purpose(
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    points: Vec<RoadVec2>,
    height_points_world: Option<Vec<RoadVec3>>,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let contour = cleaned_closed_contour(kind, mouth_order_index, band_index, points)?;
    let points_xz = polyline_to_road_points(&contour);
    let height_points_world = match height_points_world.as_deref() {
        Some(points_world) => Some(
            align_height_points_to_contour(&points_xz, points_world).ok_or(
                NodeRailGenerationError::InvalidHeightCarrier {
                    kind,
                    mouth_order_index,
                    band_index,
                    reason: "height_points_do_not_match_contour",
                },
            )?,
        ),
        None => None,
    };
    contours.push(NodeGeneratedContour {
        kind,
        purpose,
        source_mouth_order_index: mouth_order_index,
        source_band_index: band_index,
        owner,
        claim_priority,
        points_xz: points_xz.clone(),
        height_points_world,
        backend_polyline: contour,
    });
    push_constraint(
        constraints,
        constraint_kind,
        mouth_order_index,
        band_index,
        None,
        owner,
        None,
        points_xz,
    )
}

pub(in crate::simulation::network::surface::node::rails) fn default_generated_contour_purpose(
    kind: NodeGeneratedContourKind,
) -> NodeGeneratedContourPurpose {
    match kind {
        NodeGeneratedContourKind::FullRoadbed => NodeGeneratedContourPurpose::FullRoadbedCorridor,
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        } => NodeGeneratedContourPurpose::CarriagewayCorridor,
        NodeGeneratedContourKind::Band { .. } => NodeGeneratedContourPurpose::NonRoadBand,
    }
}

pub(in crate::simulation::network::surface::node::rails) fn push_path_band_contour(
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    validate_paired_path_band_height_carrier(
        kind,
        mouth_order_index,
        band_index,
        start_path_world,
        end_path_world,
    )?;
    let mut points_world = Vec::with_capacity(start_path_world.len() + end_path_world.len());
    append_world_path_points(&mut points_world, start_path_world.iter());
    append_world_path_points(&mut points_world, end_path_world.iter().rev());
    remove_closing_world_path_duplicate(&mut points_world);
    let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
    push_generated_contour_with_purpose(
        kind,
        purpose,
        mouth_order_index,
        band_index,
        owner,
        claim_priority,
        constraint_kind,
        points,
        Some(points_world),
        contours,
        constraints,
    )
}

fn path_strip_contours_world(
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Vec<Vec<RoadVec3>> {
    let point_count = start_path_world.len();
    if point_count < 2 {
        return Vec::new();
    }
    let mut strips = Vec::with_capacity(point_count - 1);
    for index in 0..point_count - 1 {
        let mut points = Vec::with_capacity(4);
        push_world_path_point(&mut points, start_path_world[index]);
        push_world_path_point(&mut points, end_path_world[index]);
        push_world_path_point(&mut points, end_path_world[index + 1]);
        push_world_path_point(&mut points, start_path_world[index + 1]);
        remove_closing_world_path_duplicate(&mut points);
        strips.push(points);
    }
    strips
}

fn validate_paired_path_band_height_carrier(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Result<(), NodeRailGenerationError> {
    if start_path_world.len() != end_path_world.len() {
        return Err(NodeRailGenerationError::InvalidHeightCarrier {
            kind,
            mouth_order_index,
            band_index,
            reason: "mismatched_path_height_carrier_lengths",
        });
    }
    if start_path_world.len() < 2 {
        return Err(NodeRailGenerationError::InvalidHeightCarrier {
            kind,
            mouth_order_index,
            band_index,
            reason: "too_few_path_height_carrier_points",
        });
    }
    Ok(())
}
