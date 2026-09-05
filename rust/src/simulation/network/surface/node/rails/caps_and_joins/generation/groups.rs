// SPDX-License-Identifier: GPL-2.0-only

//! Grouping and contour-union helpers for generated cap and side-join bands.

use super::super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct GeneratedCapOrJoinGroupKey {
    pub(super) kind: RoadSurfaceBandKind,
    pub(super) source_band_index: usize,
    pub(super) owner: NodeBandOwner,
    pub(super) contributes_footprint: bool,
}

pub(super) struct TerminalCapBandGroup<'a> {
    pub(super) contour_world: Vec<NodeOverlayContour>,
    pub(super) source_contours_world: Vec<&'a [RoadVec3]>,
    pub(super) cap_bands: Vec<&'a NodeTerminalCapBand>,
}

impl<'a> TerminalCapBandGroup<'a> {
    pub(super) fn empty() -> Self {
        Self {
            contour_world: Vec::new(),
            source_contours_world: Vec::new(),
            cap_bands: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, cap_band: &'a NodeTerminalCapBand) {
        let mut contour = cap_band
            .contour_world
            .iter()
            .map(|point| [point.x, point.z])
            .collect::<Vec<_>>();
        if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
            contour.reverse();
        }
        self.contour_world.push(contour);
        self.source_contours_world
            .push(cap_band.contour_world.as_slice());
        self.cap_bands.push(cap_band);
    }
}

pub(super) struct SideJoinBandGroup<'a> {
    pub(super) contour_world: Vec<NodeOverlayContour>,
    pub(super) source_contours_world: Vec<&'a [RoadVec3]>,
}

impl<'a> SideJoinBandGroup<'a> {
    pub(super) fn empty() -> Self {
        Self {
            contour_world: Vec::new(),
            source_contours_world: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, side_join_band: &'a NodeInputSideJoinBand) {
        let mut contour = side_join_band
            .contour_world
            .iter()
            .map(|point| [point.x, point.z])
            .collect::<Vec<_>>();
        if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
            contour.reverse();
        }
        self.contour_world.push(contour);
        self.source_contours_world
            .push(side_join_band.contour_world.as_slice());
    }
}

pub(super) fn terminal_owner_by_kind_and_source(
    mouth: &NodeInputMouth,
    mouth_owners: &MouthOwners,
    cap_bands: &[NodeTerminalCapBand],
    owners: &[NodeBandOwner],
) -> BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner> {
    let mut owner_by_kind_and_source = BTreeMap::new();
    for (interval, owner) in mouth.band_intervals.iter().zip(&mouth_owners.band_owners) {
        owner_by_kind_and_source.insert((interval.band_kind, interval.band_index), *owner);
    }
    for (cap_band, owner) in cap_bands.iter().zip(owners) {
        owner_by_kind_and_source.insert((cap_band.band_kind, cap_band.source_band_index), *owner);
    }
    owner_by_kind_and_source
}

pub(super) fn side_join_owner_by_kind_and_source(
    mouth: &NodeInputMouth,
    mouth_owners: &MouthOwners,
    side_join_bands: &[NodeInputSideJoinBand],
    owners: &[NodeBandOwner],
) -> BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner> {
    let mut owner_by_kind_and_source = BTreeMap::new();
    for (interval, owner) in mouth.band_intervals.iter().zip(&mouth_owners.band_owners) {
        owner_by_kind_and_source.insert((interval.band_kind, interval.band_index), *owner);
    }
    for (side_join_band, owner) in side_join_bands.iter().zip(owners) {
        owner_by_kind_and_source.insert(
            (side_join_band.band_kind, side_join_band.source_band_index),
            *owner,
        );
    }
    owner_by_kind_and_source
}

pub(super) fn push_grouped_cap_or_join_candidate_contours(
    mouth: &NodeInputMouth,
    key: GeneratedCapOrJoinGroupKey,
    purpose: NodeGeneratedContourPurpose,
    contour_world: &[NodeOverlayContour],
    source_contours_world: &[&[RoadVec3]],
    claim_priority: NodeGeneratedContourClaimPriority,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let Some(mut shapes) = RoadSurfaceSystem::overlay_union_contours(contour_world) else {
        return Ok(());
    };
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);

    for shape in shapes {
        for contour in shape {
            let points = contour
                .into_iter()
                .map(|point| RoadVec2::new(point[0], point[1]))
                .collect::<Vec<_>>();
            if key.contributes_footprint {
                let footprint = cleaned_closed_contour(
                    NodeGeneratedContourKind::FullRoadbed,
                    mouth.order_index,
                    None,
                    points.clone(),
                )?;
                let footprint_points_xz = polyline_to_road_points(&footprint);
                contours.push(NodeGeneratedContour {
                    kind: NodeGeneratedContourKind::FullRoadbed,
                    purpose,
                    source_mouth_order_index: mouth.order_index,
                    source_band_index: None,
                    owner: None,
                    claim_priority: NodeGeneratedContourClaimPriority::Footprint,
                    points_xz: footprint_points_xz.clone(),
                    height_points_world: None,
                    backend_polyline: footprint,
                });
                push_constraint(
                    constraints,
                    NodeRailConstraintKind::FullRoadbedContour,
                    mouth.order_index,
                    None,
                    None,
                    None,
                    None,
                    footprint_points_xz,
                )?;
            }

            let kind = NodeGeneratedContourKind::Band { kind: key.kind };
            let band_contour = cleaned_closed_contour(
                kind,
                mouth.order_index,
                Some(key.source_band_index),
                points,
            )?;
            let points_xz = polyline_to_road_points(&band_contour);
            let height_points_world =
                align_height_points_to_source_contours(&points_xz, source_contours_world);
            contours.push(NodeGeneratedContour {
                kind,
                purpose,
                source_mouth_order_index: mouth.order_index,
                source_band_index: Some(key.source_band_index),
                owner: Some(key.owner),
                claim_priority,
                points_xz: points_xz.clone(),
                height_points_world,
                backend_polyline: band_contour,
            });
            push_constraint(
                constraints,
                NodeRailConstraintKind::BandContour { kind: key.kind },
                mouth.order_index,
                Some(key.source_band_index),
                None,
                Some(key.owner),
                None,
                points_xz,
            )?;
        }
    }

    Ok(())
}

pub(super) fn side_join_contour_purpose(
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> NodeGeneratedContourPurpose {
    match piece_kind {
        RoadSurfaceVisualNodePieceKind::Bend => NodeGeneratedContourPurpose::BendSideJoin,
        RoadSurfaceVisualNodePieceKind::JunctionN => NodeGeneratedContourPurpose::JunctionSideJoin,
        RoadSurfaceVisualNodePieceKind::Terminal => NodeGeneratedContourPurpose::TerminalCap,
    }
}

pub(super) fn side_join_band_contributes_domain(side_join_band: &NodeInputSideJoinBand) -> bool {
    matches!(
        side_join_band.boundary_mode,
        NodeInputSideJoinBandBoundaryMode::MaterialBand
            | NodeInputSideJoinBandBoundaryMode::MaterialBandWithSameOwnerOuterCap
            | NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap
    )
}

pub(super) fn side_join_band_contributes_footprint(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    side_join_band: &NodeInputSideJoinBand,
) -> bool {
    if !matches!(
        piece_kind,
        RoadSurfaceVisualNodePieceKind::Bend | RoadSurfaceVisualNodePieceKind::JunctionN
    ) {
        return false;
    }
    if side_join_band.gap.role == NodeInputSideJoinGapRole::Exterior {
        return side_join_band.band_kind != RoadSurfaceBandKind::Carriageway
            && matches!(
                side_join_band.boundary_mode,
                NodeInputSideJoinBandBoundaryMode::MaterialBand
                    | NodeInputSideJoinBandBoundaryMode::MaterialBandWithSameOwnerOuterCap
            );
    }
    match side_join_band.boundary_mode {
        NodeInputSideJoinBandBoundaryMode::MaterialBand
        | NodeInputSideJoinBandBoundaryMode::MaterialBandWithSameOwnerOuterCap => true,
        NodeInputSideJoinBandBoundaryMode::SameOwnerOuterCap => false,
    }
}
