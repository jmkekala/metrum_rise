//! Boolean domain claiming and owned-region extraction.

use super::*;

pub(in crate::simulation::network::surface::node::ownership) fn split_non_road_regions(
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

pub(in crate::simulation::network::surface::node::ownership) fn owned_regions_from_domains(
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
                && domains_may_share_claim_group(group.domains[0], domain)
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

fn domains_may_share_claim_group(
    existing: &NodeGeneratedContour,
    candidate: &NodeGeneratedContour,
) -> bool {
    !matches!(
        (existing.purpose, candidate.purpose),
        (
            NodeGeneratedContourPurpose::TerminalCap,
            NodeGeneratedContourPurpose::TerminalCap
        )
    )
}
