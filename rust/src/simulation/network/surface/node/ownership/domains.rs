//! Boolean domain claiming and overlay helpers for node ownership.

use super::super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint};
use super::super::backend::road_vec2_to_overlay_point;
use super::super::rails::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeRailConstraint, NodeRailConstraintKind, NodeRailContourSet,
};
use super::super::{NodeOverlayContour, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem};
use super::seams::ConstraintOverlapMode;
use super::seams::{owned_shape_is_discardable_numeric_dust, seam_constraints_for_shape};
use super::{NodeBooleanOwnedRegion, NodeBooleanOwnershipError};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::BTreeMap;

pub(super) struct OwnedDomainResult {
    pub(super) regions: Vec<NodeBooleanOwnedRegion>,
    pub(super) claimed_shapes: NodeOverlayShapes,
}

struct OwnedDomainGroup<'a> {
    owner: NodeBandOwner,
    kind: RoadSurfaceBandKind,
    claim_priority: NodeGeneratedContourClaimPriority,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    domains: Vec<&'a NodeGeneratedContour>,
}

#[derive(Clone, Copy)]
pub(super) enum ResidualKind {
    Asphalt,
    Band(RoadSurfaceBandKind),
    NonRoad,
}

impl ResidualKind {
    fn requires_explicit_profile_seam_rail(self) -> bool {
        match self {
            ResidualKind::Band(kind) => band_kind_requires_explicit_profile_seam_rail(kind),
            ResidualKind::Asphalt | ResidualKind::NonRoad => false,
        }
    }
}

pub(super) fn split_non_road_regions(
    non_road_shapes: &NodeOverlayShapes,
    rails: &NodeRailContourSet,
) -> Result<OwnedDomainResult, NodeBooleanOwnershipError> {
    split_non_road_regions_by_band_order(non_road_shapes, rails)
}

fn split_non_road_regions_by_band_order(
    non_road_shapes: &NodeOverlayShapes,
    rails: &NodeRailContourSet,
) -> Result<OwnedDomainResult, NodeBooleanOwnershipError> {
    let mut regions = Vec::new();
    let mut claimed_shapes = Vec::new();

    for kind in non_road_band_order() {
        let kind_domains = non_road_domains_for_band_kind(rails, kind);
        if kind_domains.is_empty() {
            continue;
        }

        let kind_contours = kind_domains
            .iter()
            .map(|domain| overlay_contour_from_domain(domain))
            .collect::<Vec<_>>();
        let mut kind_target = overlay_union(&kind_contours, "non_road_band_union")?;
        kind_target = overlay_intersect(
            &kind_target,
            non_road_shapes,
            "non_road_band_clip_to_target",
        )?;
        kind_target = overlay_difference(&kind_target, &claimed_shapes, "non_road_band_unclaimed")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut kind_target);
        if kind_target.is_empty() {
            continue;
        }

        let kind_result = owned_regions_from_domains(
            &kind_target,
            &kind_domains,
            &rails.constraints,
            ResidualKind::Band(kind),
            ConstraintOverlapMode::for_piece_kind(rails.piece_kind),
        )?;
        claimed_shapes =
            overlay_union_shape_sets(&claimed_shapes, &kind_result.claimed_shapes, "claim_union")?;
        regions.extend(kind_result.regions);
    }

    RoadSurfaceSystem::sort_overlay_shapes(&mut claimed_shapes);
    Ok(OwnedDomainResult {
        regions,
        claimed_shapes,
    })
}

pub(super) fn owned_regions_from_domains(
    target_shapes: &NodeOverlayShapes,
    domains: &[&NodeGeneratedContour],
    rail_constraints: &[NodeRailConstraint],
    residual_kind: ResidualKind,
    overlap_mode: ConstraintOverlapMode,
) -> Result<OwnedDomainResult, NodeBooleanOwnershipError> {
    if target_shapes.is_empty() {
        return Ok(OwnedDomainResult {
            regions: Vec::new(),
            claimed_shapes: Vec::new(),
        });
    }

    let mut regions = Vec::new();
    let mut claimed_shapes = Vec::new();

    for group in owned_domain_groups(domains)? {
        let domain_contours = group
            .domains
            .iter()
            .map(|domain| overlay_contour_from_domain(domain))
            .collect::<Vec<_>>();
        let mut domain_shapes = overlay_union(&domain_contours, "domain_union")?;
        domain_shapes = overlay_intersect(&domain_shapes, target_shapes, "domain_clip")?;
        domain_shapes = overlay_difference(&domain_shapes, &claimed_shapes, "domain_unclaimed")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut domain_shapes);
        if domain_shapes.is_empty() {
            continue;
        }

        let mut group_claimed_shapes = Vec::new();
        for shape in &domain_shapes {
            let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(shape);
            if owned_shape_is_discardable_numeric_dust(
                shape,
                area_m2,
                group.owner,
                rail_constraints,
            ) {
                group_claimed_shapes.push(shape.clone());
                continue;
            }
            let seam_constraints =
                seam_constraints_for_shape(shape, group.owner, rail_constraints, overlap_mode);
            if residual_kind.requires_explicit_profile_seam_rail()
                && !region_has_explicit_profile_seam_rail(&seam_constraints, rail_constraints)
            {
                continue;
            }
            group_claimed_shapes.push(shape.clone());
            regions.push(NodeBooleanOwnedRegion {
                kind: group.kind,
                owner: group.owner,
                claim_priority: group.claim_priority,
                source_mouth_order_index: group.source_mouth_order_index,
                source_band_index: group.source_band_index,
                shape: shape.clone(),
                area_m2,
                seam_constraints,
            });
        }
        claimed_shapes =
            overlay_union_shape_sets(&claimed_shapes, &group_claimed_shapes, "domain_claim_union")?;
    }

    let residual = overlay_difference(target_shapes, &claimed_shapes, "domain_residual_final")?;
    reject_residual(residual, residual_kind)?;
    Ok(OwnedDomainResult {
        regions,
        claimed_shapes,
    })
}

fn owned_domain_groups<'a>(
    domains: &[&'a NodeGeneratedContour],
) -> Result<Vec<OwnedDomainGroup<'a>>, NodeBooleanOwnershipError> {
    let mut groups = Vec::<OwnedDomainGroup<'a>>::new();
    for domain in domains {
        let owner = domain
            .owner
            .ok_or(NodeBooleanOwnershipError::MissingBandOwner {
                mouth_order_index: domain.source_mouth_order_index,
                band_index: domain.source_band_index,
            })?;
        let kind = band_kind(domain).expect("owned domain must be a band contour");
        if let Some(group) = groups.last_mut() {
            if group.owner == owner
                && group.kind == kind
                && group.claim_priority == domain.claim_priority
                && group.source_mouth_order_index == domain.source_mouth_order_index
                && group.source_band_index == domain.source_band_index
            {
                group.domains.push(*domain);
                continue;
            }
        }
        groups.push(OwnedDomainGroup {
            owner,
            kind,
            claim_priority: domain.claim_priority,
            source_mouth_order_index: domain.source_mouth_order_index,
            source_band_index: domain.source_band_index,
            domains: vec![*domain],
        });
    }
    Ok(groups)
}

pub(super) fn overlay_contours_for_domains(
    rails: &NodeRailContourSet,
    predicate: impl Fn(&NodeGeneratedContour) -> bool,
) -> Vec<NodeOverlayContour> {
    rails
        .contours
        .iter()
        .filter(|contour| predicate(contour))
        .map(overlay_contour_from_domain)
        .collect()
}

pub(super) fn asphalt_authority_domains(rails: &NodeRailContourSet) -> Vec<&NodeGeneratedContour> {
    domains_for_band_kind_matching(rails, RoadSurfaceBandKind::Carriageway, |contour| {
        contour.contributes_to_asphalt()
    })
}

pub(super) fn asphalt_owner_domains(rails: &NodeRailContourSet) -> Vec<&NodeGeneratedContour> {
    domains_for_band_kind_matching(rails, RoadSurfaceBandKind::Carriageway, |contour| {
        contour.claims_asphalt_owner_region()
    })
}

fn non_road_domains_for_band_kind(
    rails: &NodeRailContourSet,
    kind: RoadSurfaceBandKind,
) -> Vec<&NodeGeneratedContour> {
    domains_for_band_kind_matching(rails, kind, |contour| {
        contour.contributes_to_non_road_band()
    })
}

fn domains_for_band_kind_matching(
    rails: &NodeRailContourSet,
    kind: RoadSurfaceBandKind,
    predicate: impl Fn(&NodeGeneratedContour) -> bool,
) -> Vec<&NodeGeneratedContour> {
    let mut domains = rails
        .contours
        .iter()
        .filter(|contour| band_kind(contour) == Some(kind) && predicate(contour))
        .collect::<Vec<_>>();
    domains.sort_by_key(|contour| {
        (
            contour.claim_priority,
            contour.purpose,
            contour.source_mouth_order_index,
            contour.source_band_index,
        )
    });
    domains
}

pub(super) fn overlay_contour_from_domain(domain: &NodeGeneratedContour) -> NodeOverlayContour {
    domain
        .points_xz
        .iter()
        .copied()
        .map(road_vec2_to_overlay_point)
        .collect()
}

pub(super) fn overlay_union(
    contours: &[NodeOverlayContour],
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let mut shapes = RoadSurfaceSystem::overlay_union_contours(contours)
        .ok_or(NodeBooleanOwnershipError::BooleanOperationFailed { stage })?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

pub(super) fn overlay_intersect(
    subject: &NodeOverlayShapes,
    clip: &NodeOverlayShapes,
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    if subject.is_empty() || clip.is_empty() {
        return Ok(Vec::new());
    }
    let mut shapes =
        RoadSurfaceSystem::overlay_binary_shapes(subject, clip, OverlayRule::Intersect)
            .ok_or(NodeBooleanOwnershipError::BooleanOperationFailed { stage })?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

pub(super) fn overlay_difference(
    subject: &NodeOverlayShapes,
    clip: &NodeOverlayShapes,
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let mut shapes =
        RoadSurfaceSystem::overlay_binary_shapes(subject, clip, OverlayRule::Difference)
            .ok_or(NodeBooleanOwnershipError::BooleanOperationFailed { stage })?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

fn overlay_union_shape_sets(
    existing: &NodeOverlayShapes,
    added: &NodeOverlayShapes,
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    if existing.is_empty() {
        return Ok(added.clone());
    }
    if added.is_empty() {
        return Ok(existing.clone());
    }
    let mut shapes = RoadSurfaceSystem::overlay_binary_shapes(existing, added, OverlayRule::Union)
        .ok_or(NodeBooleanOwnershipError::BooleanOperationFailed { stage })?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

pub(super) fn reject_residual(
    residual: NodeOverlayShapes,
    residual_kind: ResidualKind,
) -> Result<(), NodeBooleanOwnershipError> {
    if residual.is_empty() {
        return Ok(());
    }

    let shape_count = residual.len();
    let area_m2 = residual
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum();
    if area_m2 <= RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&residual) {
        return Ok(());
    }
    match residual_kind {
        ResidualKind::Asphalt => Err(NodeBooleanOwnershipError::UnownedAsphaltResidual {
            shape_count,
            area_m2,
        }),
        ResidualKind::Band(kind) => Err(NodeBooleanOwnershipError::UnownedBandResidual {
            kind,
            shape_count,
            area_m2,
        }),
        ResidualKind::NonRoad => Err(NodeBooleanOwnershipError::UnownedNonRoadResidual {
            shape_count,
            area_m2,
        }),
    }
}

fn band_kind(contour: &NodeGeneratedContour) -> Option<RoadSurfaceBandKind> {
    match contour.kind {
        NodeGeneratedContourKind::Band { kind } => Some(kind),
        NodeGeneratedContourKind::FullRoadbed => None,
    }
}

fn non_road_band_order() -> [RoadSurfaceBandKind; 7] {
    [
        RoadSurfaceBandKind::CurbOrShoulder,
        RoadSurfaceBandKind::Sidewalk,
        RoadSurfaceBandKind::Footpath,
        RoadSurfaceBandKind::CycleTrack,
        RoadSurfaceBandKind::Median,
        RoadSurfaceBandKind::Parking,
        RoadSurfaceBandKind::TramReservation,
    ]
}

pub(super) fn sort_boolean_owned_regions(regions: &mut [NodeBooleanOwnedRegion]) {
    regions.sort_by(|a, b| {
        RoadSurfaceSystem::band_kind_sort_key(a.kind)
            .cmp(&RoadSurfaceSystem::band_kind_sort_key(b.kind))
            .then(a.claim_priority.cmp(&b.claim_priority))
            .then(a.source_mouth_order_index.cmp(&b.source_mouth_order_index))
            .then(a.source_band_index.cmp(&b.source_band_index))
            .then(a.area_m2.total_cmp(&b.area_m2))
    });
}

pub(super) fn validate_non_road_regions_have_explicit_profile_seam_rails(
    regions: &[NodeBooleanOwnedRegion],
    rail_constraints: &[NodeRailConstraint],
) -> Result<(), NodeBooleanOwnershipError> {
    let mut missing_by_kind = BTreeMap::<RoadSurfaceBandKind, (usize, f32)>::new();
    for region in regions {
        if !band_kind_requires_explicit_profile_seam_rail(region.kind)
            || region_has_explicit_profile_seam_rail(&region.seam_constraints, rail_constraints)
        {
            continue;
        }
        let entry = missing_by_kind.entry(region.kind).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += region.area_m2;
    }
    if let Some((kind, (shape_count, area_m2))) = missing_by_kind.into_iter().next() {
        return Err(NodeBooleanOwnershipError::UnownedBandResidual {
            kind,
            shape_count,
            area_m2,
        });
    }
    Ok(())
}

fn band_kind_requires_explicit_profile_seam_rail(kind: RoadSurfaceBandKind) -> bool {
    matches!(
        kind,
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
    )
}

fn region_has_explicit_profile_seam_rail(
    seam_constraints: &[NodeRegionSeamConstraint],
    rail_constraints: &[NodeRailConstraint],
) -> bool {
    seam_constraints.iter().any(|seam| {
        rail_constraints
            .iter()
            .find(|constraint| constraint.constraint_index == seam.constraint_index)
            .is_some_and(rail_constraint_is_explicit_profile_seam_rail)
    })
}

fn rail_constraint_is_explicit_profile_seam_rail(constraint: &NodeRailConstraint) -> bool {
    matches!(
        constraint.kind,
        NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::FootprintSeam { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::RaisedStepContact
            | NodeRailConstraintKind::BandBoundary { .. }
    )
}
