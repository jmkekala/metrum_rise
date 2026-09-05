// SPDX-License-Identifier: GPL-2.0-only

//! Terminal-cap and side-join generated rail contour orchestration.

use super::boundary_constraints::{
    push_side_join_band_boundary_constraints, push_terminal_cap_band_boundary_constraints,
};
use super::*;

mod groups;

use groups::{
    GeneratedCapOrJoinGroupKey, SideJoinBandGroup, TerminalCapBandGroup,
    push_grouped_cap_or_join_candidate_contours, side_join_band_contributes_domain,
    side_join_band_contributes_footprint, side_join_contour_purpose,
    side_join_owner_by_kind_and_source, terminal_owner_by_kind_and_source,
};

pub(in crate::simulation::network::surface::node::rails) fn push_terminal_cap_band_contours(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    cap_bands: &[NodeTerminalCapBand],
    mouth_owners: &MouthOwners,
    owners: &[NodeBandOwner],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let owner_by_kind_and_source =
        terminal_owner_by_kind_and_source(mouth, mouth_owners, cap_bands, owners);
    if piece_kind != RoadSurfaceVisualNodePieceKind::Terminal {
        return Ok(());
    }

    for (cap_band, owner) in cap_bands.iter().zip(owners) {
        let mut group = TerminalCapBandGroup::empty();
        group.push(cap_band);
        let key = GeneratedCapOrJoinGroupKey {
            kind: cap_band.band_kind,
            source_band_index: cap_band.source_band_index,
            owner: *owner,
            contributes_footprint: true,
        };
        push_grouped_cap_or_join_candidate_contours(
            mouth,
            key,
            NodeGeneratedContourPurpose::TerminalCap,
            &group.contour_world,
            &group.source_contours_world,
            NodeGeneratedContourClaimPriority::JoinOrCap,
            contours,
            constraints,
        )?;
        push_terminal_cap_band_boundary_constraints(
            mouth,
            cap_band,
            key.owner,
            &owner_by_kind_and_source,
            constraints,
        )?;
    }

    Ok(())
}

pub(in crate::simulation::network::surface::node::rails) fn push_side_join_band_contours(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    side_join_bands: &[NodeInputSideJoinBand],
    mouth_owners: &MouthOwners,
    owners: &[NodeBandOwner],
    contours: &mut Vec<NodeGeneratedContour>,
    corner_trims: &mut Vec<NodeGeneratedCornerTrim>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    if piece_kind == RoadSurfaceVisualNodePieceKind::Terminal {
        return Ok(());
    }

    let owner_by_kind_and_source =
        side_join_owner_by_kind_and_source(mouth, mouth_owners, side_join_bands, owners);
    let mut groups = BTreeMap::<GeneratedCapOrJoinGroupKey, SideJoinBandGroup>::new();
    for (side_join_band, owner) in side_join_bands.iter().zip(owners) {
        if !side_join_band_contributes_domain(side_join_band) {
            continue;
        }
        groups
            .entry(GeneratedCapOrJoinGroupKey {
                kind: side_join_band.band_kind,
                source_band_index: side_join_band.source_band_index,
                owner: *owner,
                contributes_footprint: side_join_band_contributes_footprint(
                    piece_kind,
                    side_join_band,
                ),
            })
            .or_insert_with(SideJoinBandGroup::empty)
            .push(side_join_band);
    }

    for (key, group) in groups {
        push_grouped_cap_or_join_candidate_contours(
            mouth,
            key,
            side_join_contour_purpose(piece_kind),
            &group.contour_world,
            &group.source_contours_world,
            NodeGeneratedContourClaimPriority::SideJoin,
            contours,
            constraints,
        )?;
    }
    for (side_join_band, owner) in side_join_bands.iter().zip(owners) {
        push_side_join_outer_footprint_trim(mouth, side_join_band, *owner, corner_trims)?;
    }
    for (side_join_band, owner) in side_join_bands.iter().zip(owners) {
        push_side_join_band_boundary_constraints(
            mouth,
            side_join_band,
            *owner,
            &owner_by_kind_and_source,
            constraints,
        )?;
    }

    Ok(())
}

fn push_side_join_outer_footprint_trim(
    mouth: &NodeInputMouth,
    side_join_band: &NodeInputSideJoinBand,
    owner: NodeBandOwner,
    corner_trims: &mut Vec<NodeGeneratedCornerTrim>,
) -> Result<(), NodeRailGenerationError> {
    if !side_join_band.trims_outer_footprint || side_join_band.outer_footprint_trim_world.len() < 3
    {
        return Ok(());
    }
    let points = side_join_band
        .outer_footprint_trim_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    let trim = cleaned_closed_contour(
        NodeGeneratedContourKind::FullRoadbed,
        mouth.order_index,
        None,
        points,
    )?;
    let points_xz = polyline_to_road_points(&trim);
    corner_trims.push(NodeGeneratedCornerTrim {
        source_mouth_order_index: mouth.order_index,
        source_band_index: side_join_band.source_band_index,
        source_band_kind: side_join_band.band_kind,
        source_owner: owner,
        points_xz,
    });
    Ok(())
}
