// SPDX-License-Identifier: GPL-2.0-only

//! Mouth band and corridor contour generation for node rails.

use super::super::arrangement::NodeBandOwner;
use super::super::backend::road_vec3_xz as xz;
use super::super::input::{NodeInputBandInterval, NodeInputMouth};
use super::super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use super::contours::{
    append_world_path_points, append_world_path_xz, default_generated_contour_purpose,
    push_generated_contour, push_generated_contour_with_purpose, push_path_band_contour,
    push_path_strip_contours, push_road_path_point, push_world_path_point,
    remove_closing_road_path_duplicate, remove_closing_world_path_duplicate,
    subdivided_world_chord,
};
use super::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeGeneratedContourPurpose, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailGenerationError,
};

pub(super) fn push_full_roadbed_contour(
    mouth: &NodeInputMouth,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let first = mouth
        .boundary_rails
        .first()
        .expect("validated input has rails");
    let last = mouth
        .boundary_rails
        .last()
        .expect("validated input has rails");
    let mut points = Vec::new();
    push_road_path_point(&mut points, xz(first.mouth_world));
    push_road_path_point(&mut points, xz(last.mouth_world));
    append_world_path_xz(&mut points, last.path_world.iter().skip(1));
    append_world_path_xz(&mut points, first.path_world.iter().rev());
    remove_closing_road_path_duplicate(&mut points);
    push_generated_contour(
        NodeGeneratedContourKind::FullRoadbed,
        mouth.order_index,
        None,
        None,
        NodeGeneratedContourClaimPriority::Footprint,
        NodeRailConstraintKind::FullRoadbedContour,
        points,
        None,
        contours,
        constraints,
    )
}

pub(super) fn push_band_contour(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    interval: &NodeInputBandInterval,
    owner: NodeBandOwner,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let kind = NodeGeneratedContourKind::Band {
        kind: interval.band_kind,
    };
    let purpose = band_contour_purpose(piece_kind, interval.band_kind);
    let last_band_index = mouth.band_intervals.len().saturating_sub(1);
    if mouth.uses_explicit_band_domain_paths {
        let uses_paired_explicit_paths =
            interval.start_path_world.len() > 2 || interval.end_path_world.len() > 2;
        let uses_explicit_outer_chord = interval.band_index == 0
            && interval.start_path_world.len() > 2
            && interval.end_path_world.len() == 2
            || interval.band_index == last_band_index
                && interval.start_path_world.len() == 2
                && interval.end_path_world.len() > 2;
        if uses_paired_explicit_paths
            && interval.start_path_world.len() != interval.end_path_world.len()
            && !uses_explicit_outer_chord
        {
            return Err(NodeRailGenerationError::InvalidHeightCarrier {
                kind,
                mouth_order_index: mouth.order_index,
                band_index: Some(interval.band_index),
                reason: "mismatched_path_height_carrier_lengths",
            });
        }
        if uses_paired_explicit_paths
            && interval.start_path_world.len() == interval.end_path_world.len()
        {
            return push_path_band_contour(
                kind,
                purpose,
                mouth.order_index,
                Some(interval.band_index),
                Some(owner),
                NodeGeneratedContourClaimPriority::MouthBand,
                NodeRailConstraintKind::BandContour {
                    kind: interval.band_kind,
                },
                &interval.start_path_world,
                &interval.end_path_world,
                contours,
                constraints,
            );
        }
    }
    if interval.band_index == 0 && interval.start_path_world.len() > 2 {
        let inner_path = subdivided_world_chord(
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.start_path_world.len(),
        );
        return push_path_strip_contours(
            kind,
            purpose,
            mouth.order_index,
            Some(interval.band_index),
            Some(owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: interval.band_kind,
            },
            &interval.start_path_world,
            &inner_path,
            contours,
            constraints,
        );
    }
    if interval.band_index == last_band_index && interval.end_path_world.len() > 2 {
        let inner_path = subdivided_world_chord(
            interval.mouth_start_world,
            interval.endpoint_start_world,
            interval.end_path_world.len(),
        );
        return push_path_strip_contours(
            kind,
            purpose,
            mouth.order_index,
            Some(interval.band_index),
            Some(owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: interval.band_kind,
            },
            &inner_path,
            &interval.end_path_world,
            contours,
            constraints,
        );
    }
    let mut points_world = Vec::new();
    push_world_path_point(&mut points_world, interval.mouth_start_world);
    push_world_path_point(&mut points_world, interval.mouth_end_world);
    if interval.band_index == last_band_index {
        append_world_path_points(&mut points_world, interval.end_path_world.iter().skip(1));
    } else {
        push_world_path_point(&mut points_world, interval.endpoint_end_world);
    }
    if interval.band_index == 0 {
        append_world_path_points(&mut points_world, interval.start_path_world.iter().rev());
    } else {
        push_world_path_point(&mut points_world, interval.endpoint_start_world);
    }
    remove_closing_world_path_duplicate(&mut points_world);
    let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
    push_generated_contour_with_purpose(
        kind,
        purpose,
        mouth.order_index,
        Some(interval.band_index),
        Some(owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: interval.band_kind,
        },
        points,
        Some(points_world),
        contours,
        constraints,
    )
}

fn band_contour_purpose(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    band_kind: RoadSurfaceBandKind,
) -> NodeGeneratedContourPurpose {
    if piece_kind != RoadSurfaceVisualNodePieceKind::Terminal
        && band_kind == RoadSurfaceBandKind::Carriageway
    {
        NodeGeneratedContourPurpose::CarriagewayOwnerCarrier
    } else {
        default_generated_contour_purpose(NodeGeneratedContourKind::Band { kind: band_kind })
    }
}
