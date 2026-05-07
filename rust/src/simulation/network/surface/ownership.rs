//! Boolean ownership solve for canonical node-arrangement contours.

#![allow(dead_code)]

use super::arrangement::{
    NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource, seam_source_priority,
};
use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2, overlay_point_to_road, road_vec2_to_overlay_point,
};
use super::rails::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeRailConstraint, NodeRailConstraintKind, NodeRailContourSet,
};
use super::{
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeBooleanOwnership {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) footprint_shapes: NodeOverlayShapes,
    pub(crate) asphalt_shapes: NodeOverlayShapes,
    pub(crate) non_road_shapes: NodeOverlayShapes,
    pub(crate) owned_regions: Vec<NodeBooleanOwnedRegion>,
    pub(crate) owned_region_arrangement: NodeOwnedRegionArrangement,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegionArrangement {
    node_id: u32,
    piece_kind: RoadSurfaceVisualNodePieceKind,
    region_count: usize,
    edges: Vec<NodeOwnedRegionArrangementEdge>,
    diagnostics: Vec<NodeOwnedRegionArrangementDiagnostic>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegionArrangementEdge {
    pub(crate) region_index: usize,
    pub(crate) owner: NodeBandOwner,
    pub(crate) opposite_owner: Option<NodeBandOwner>,
    pub(crate) start: NodeOwnedRegionArrangementKey,
    pub(crate) end: NodeOwnedRegionArrangementKey,
    pub(crate) seam_source: NodeSeamSource,
    pub(crate) source_constraint_indices: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NodeOwnedRegionArrangementDiagnostic {
    MissingSeamConstraint {
        region_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start: NodeOwnedRegionArrangementKey,
        end: NodeOwnedRegionArrangementKey,
    },
    AmbiguousSeamConstraint {
        region_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start: NodeOwnedRegionArrangementKey,
        end: NodeOwnedRegionArrangementKey,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeOwnedRegionArrangementKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeBooleanOwnedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: NodeBandOwner,
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) shape: NodeOverlayShape,
    pub(crate) area_m2: f32,
    pub(crate) seam_constraints: Vec<NodeRegionSeamConstraint>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeBooleanOwnershipError {
    EmptyContourSet {
        node_id: u32,
    },
    EmptyFootprint {
        node_id: u32,
    },
    MissingBandOwner {
        mouth_order_index: usize,
        band_index: Option<usize>,
    },
    BooleanOperationFailed {
        stage: &'static str,
    },
    UnownedAsphaltResidual {
        shape_count: usize,
        area_m2: f32,
    },
    UnownedBandResidual {
        kind: RoadSurfaceBandKind,
        shape_count: usize,
        area_m2: f32,
    },
    UnownedNonRoadResidual {
        shape_count: usize,
        area_m2: f32,
    },
}

struct OwnedDomainResult {
    regions: Vec<NodeBooleanOwnedRegion>,
    claimed_shapes: NodeOverlayShapes,
}

struct OwnedDomainGroup<'a> {
    owner: NodeBandOwner,
    kind: RoadSurfaceBandKind,
    claim_priority: NodeGeneratedContourClaimPriority,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    domains: Vec<&'a NodeGeneratedContour>,
}

impl RoadSurfaceSystem {
    pub(super) fn build_node_boolean_ownership_from_rails(
        rails: &NodeRailContourSet,
    ) -> Result<NodeBooleanOwnership, NodeBooleanOwnershipError> {
        NodeBooleanOwnership::from_rails(rails)
    }
}

impl NodeBooleanOwnership {
    pub(crate) fn from_rails(
        rails: &NodeRailContourSet,
    ) -> Result<Self, NodeBooleanOwnershipError> {
        if rails.contours.is_empty() {
            return Err(NodeBooleanOwnershipError::EmptyContourSet {
                node_id: rails.node_id,
            });
        }

        let footprint_contours = overlay_contours_for_domains(rails, |contour| {
            contour.kind == NodeGeneratedContourKind::FullRoadbed
        });
        let mut footprint_shapes = overlay_union(&footprint_contours, "footprint_union")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut footprint_shapes);
        if footprint_shapes.is_empty() {
            return Err(NodeBooleanOwnershipError::EmptyFootprint {
                node_id: rails.node_id,
            });
        }

        let asphalt_domains = domains_for_band_kind(rails, RoadSurfaceBandKind::Carriageway);
        let asphalt_contours = asphalt_domains
            .iter()
            .map(|domain| overlay_contour_from_domain(domain))
            .collect::<Vec<_>>();
        let mut asphalt_shapes = overlay_union(&asphalt_contours, "asphalt_union")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut asphalt_shapes);

        let mut non_road_shapes =
            overlay_difference(&footprint_shapes, &asphalt_shapes, "non_road_difference")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut non_road_shapes);

        let mut owned_regions = Vec::new();
        let asphalt_result = owned_regions_from_domains(
            &asphalt_shapes,
            &asphalt_domains,
            &rails.constraints,
            ResidualKind::Asphalt,
        )?;
        owned_regions.extend(asphalt_result.regions);

        let non_road_result = split_non_road_regions(&non_road_shapes, rails)?;
        owned_regions.extend(non_road_result.regions);
        let non_road_residual = overlay_difference(
            &non_road_shapes,
            &non_road_result.claimed_shapes,
            "non_road_residual",
        )?;
        reject_residual(non_road_residual, ResidualKind::NonRoad)?;

        sort_boolean_owned_regions(&mut owned_regions);
        canonicalize_owned_region_rings(&mut owned_regions, &footprint_shapes);
        clean_canonical_owned_region_shapes(&mut owned_regions, &rails.constraints)?;
        let owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            rails.node_id,
            rails.piece_kind,
            &owned_regions,
            &footprint_shapes,
        );
        Ok(Self {
            node_id: rails.node_id,
            piece_kind: rails.piece_kind,
            footprint_shapes,
            asphalt_shapes,
            non_road_shapes,
            owned_regions,
            owned_region_arrangement,
        })
    }
}

impl NodeOwnedRegionArrangement {
    pub(crate) fn from_owned_regions(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        regions: &[NodeBooleanOwnedRegion],
        footprint_shapes: &NodeOverlayShapes,
    ) -> Self {
        let boundary_refs = owned_region_boundary_refs(regions, footprint_shapes);
        let mut edges = Vec::new();
        let mut diagnostics = Vec::new();

        for (edge_key, refs) in boundary_refs.edges {
            let refs = canonical_owned_region_edge_refs(&refs);
            for edge_ref in &refs {
                let Some(region) = regions.get(edge_ref.region_index) else {
                    continue;
                };
                let opposite_owner = opposite_owner_for_ref(&refs, *edge_ref);
                let source_constraints = owned_source_constraints_for_edge(
                    edge_key.start,
                    edge_key.end,
                    &region.seam_constraints,
                );
                if let Some(opposite_owner) = opposite_owner {
                    let start = NodeOwnedRegionArrangementKey::from_ownership_key(edge_key.start);
                    let end = NodeOwnedRegionArrangementKey::from_ownership_key(edge_key.end);
                    if source_constraints.is_empty() {
                        diagnostics.push(
                            NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
                                region_index: edge_ref.region_index,
                                owner: edge_ref.owner,
                                opposite_owner,
                                start,
                                end,
                            },
                        );
                    } else if owned_source_constraints_are_ambiguous(&source_constraints) {
                        diagnostics.push(
                            NodeOwnedRegionArrangementDiagnostic::AmbiguousSeamConstraint {
                                region_index: edge_ref.region_index,
                                owner: edge_ref.owner,
                                opposite_owner,
                                start,
                                end,
                            },
                        );
                    }
                }
                let seam_source = source_constraints
                    .first()
                    .map(|constraint| constraint.seam_source.clone())
                    .unwrap_or_else(|| seam_source_for_owner(edge_ref.owner));
                let source_constraint_indices = canonical_source_indices(
                    source_constraints
                        .iter()
                        .map(|constraint| constraint.constraint_index),
                );
                edges.push(NodeOwnedRegionArrangementEdge {
                    region_index: edge_ref.region_index,
                    owner: edge_ref.owner,
                    opposite_owner,
                    start: NodeOwnedRegionArrangementKey::from_ownership_key(edge_key.start),
                    end: NodeOwnedRegionArrangementKey::from_ownership_key(edge_key.end),
                    seam_source,
                    source_constraint_indices,
                });
            }
        }

        Self {
            node_id,
            piece_kind,
            region_count: regions.len(),
            edges,
            diagnostics,
        }
    }

    pub(crate) fn node_id(&self) -> u32 {
        self.node_id
    }

    pub(crate) fn piece_kind(&self) -> RoadSurfaceVisualNodePieceKind {
        self.piece_kind
    }

    pub(crate) fn region_count(&self) -> usize {
        self.region_count
    }

    pub(crate) fn edges(&self) -> &[NodeOwnedRegionArrangementEdge] {
        &self.edges
    }

    pub(crate) fn diagnostics(&self) -> &[NodeOwnedRegionArrangementDiagnostic] {
        &self.diagnostics
    }
}

impl NodeOwnedRegionArrangementKey {
    pub(crate) fn from_point(point: RoadVec2) -> Self {
        Self::from_ownership_key(road_point_key(point))
    }

    fn from_ownership_key(point: NodeOwnershipPointKey) -> Self {
        Self {
            x_key: point.0,
            z_key: point.1,
        }
    }

    pub(crate) fn x_mm(self) -> i64 {
        ownership_coordinate_key_to_mm(self.x_key)
    }

    pub(crate) fn z_mm(self) -> i64 {
        ownership_coordinate_key_to_mm(self.z_key)
    }
}

fn split_non_road_regions(
    non_road_shapes: &NodeOverlayShapes,
    rails: &NodeRailContourSet,
) -> Result<OwnedDomainResult, NodeBooleanOwnershipError> {
    if rails.piece_kind != RoadSurfaceVisualNodePieceKind::Bend {
        return split_non_road_regions_by_band_order(non_road_shapes, rails);
    }
    split_bend_non_road_regions_by_generated_domain(non_road_shapes, rails)
}

fn split_non_road_regions_by_band_order(
    non_road_shapes: &NodeOverlayShapes,
    rails: &NodeRailContourSet,
) -> Result<OwnedDomainResult, NodeBooleanOwnershipError> {
    let mut regions = Vec::new();
    let mut claimed_shapes = Vec::new();

    for kind in non_road_band_order() {
        let kind_domains = domains_for_band_kind(rails, kind);
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

fn split_bend_non_road_regions_by_generated_domain(
    non_road_shapes: &NodeOverlayShapes,
    rails: &NodeRailContourSet,
) -> Result<OwnedDomainResult, NodeBooleanOwnershipError> {
    let mut regions = Vec::new();
    let mut claimed_shapes = Vec::new();

    for group in non_road_domain_groups(rails)? {
        let domain_contours = group
            .domains
            .iter()
            .map(|domain| overlay_contour_from_domain(domain))
            .collect::<Vec<_>>();
        let mut domain_shapes = overlay_union(&domain_contours, "non_road_domain_union")?;
        domain_shapes = overlay_intersect(
            &domain_shapes,
            non_road_shapes,
            "non_road_domain_clip_to_target",
        )?;
        domain_shapes =
            overlay_difference(&domain_shapes, &claimed_shapes, "non_road_domain_unclaimed")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut domain_shapes);
        if domain_shapes.is_empty() {
            continue;
        }

        for shape in &domain_shapes {
            let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(shape);
            if owned_shape_is_discardable_numeric_dust(
                shape,
                area_m2,
                group.owner,
                &rails.constraints,
            ) {
                continue;
            }
            regions.push(NodeBooleanOwnedRegion {
                kind: group.kind,
                owner: group.owner,
                claim_priority: group.claim_priority,
                source_mouth_order_index: group.source_mouth_order_index,
                source_band_index: group.source_band_index,
                shape: shape.clone(),
                area_m2,
                seam_constraints: seam_constraints_for_shape(
                    shape,
                    group.owner,
                    &rails.constraints,
                ),
            });
        }
        claimed_shapes = overlay_union_shape_sets(
            &claimed_shapes,
            &domain_shapes,
            "non_road_domain_claim_union",
        )?;
    }

    RoadSurfaceSystem::sort_overlay_shapes(&mut claimed_shapes);
    Ok(OwnedDomainResult {
        regions,
        claimed_shapes,
    })
}

fn non_road_domain_groups<'a>(
    rails: &'a NodeRailContourSet,
) -> Result<Vec<OwnedDomainGroup<'a>>, NodeBooleanOwnershipError> {
    let mut domains = rails
        .contours
        .iter()
        .filter(|contour| band_kind(contour).is_some_and(is_non_road_band))
        .collect::<Vec<_>>();
    domains.sort_by_key(|contour| {
        let kind = band_kind(contour).expect("non-road domain must have a band kind");
        (
            contour.claim_priority,
            RoadSurfaceSystem::band_kind_sort_key(kind),
            contour.source_mouth_order_index,
            contour.source_band_index,
            contour.owner,
        )
    });
    owned_domain_groups(&domains)
}

fn owned_regions_from_domains(
    target_shapes: &NodeOverlayShapes,
    domains: &[&NodeGeneratedContour],
    rail_constraints: &[NodeRailConstraint],
    residual_kind: ResidualKind,
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

        for shape in &domain_shapes {
            let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(shape);
            if owned_shape_is_discardable_numeric_dust(
                shape,
                area_m2,
                group.owner,
                rail_constraints,
            ) {
                continue;
            }
            regions.push(NodeBooleanOwnedRegion {
                kind: group.kind,
                owner: group.owner,
                claim_priority: group.claim_priority,
                source_mouth_order_index: group.source_mouth_order_index,
                source_band_index: group.source_band_index,
                shape: shape.clone(),
                area_m2,
                seam_constraints: seam_constraints_for_shape(shape, group.owner, rail_constraints),
            });
        }
        claimed_shapes =
            overlay_union_shape_sets(&claimed_shapes, &domain_shapes, "domain_claim_union")?;
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

fn overlay_contours_for_domains(
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

fn domains_for_band_kind(
    rails: &NodeRailContourSet,
    kind: RoadSurfaceBandKind,
) -> Vec<&NodeGeneratedContour> {
    let mut domains = rails
        .contours
        .iter()
        .filter(|contour| band_kind(contour) == Some(kind))
        .collect::<Vec<_>>();
    domains.sort_by_key(|contour| {
        (
            contour.claim_priority,
            contour.source_mouth_order_index,
            contour.source_band_index,
        )
    });
    domains
}

fn overlay_contour_from_domain(domain: &NodeGeneratedContour) -> NodeOverlayContour {
    domain
        .points_xz
        .iter()
        .copied()
        .map(road_vec2_to_overlay_point)
        .collect()
}

fn overlay_union(
    contours: &[NodeOverlayContour],
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let mut shapes = RoadSurfaceSystem::overlay_union_contours(contours)
        .ok_or(NodeBooleanOwnershipError::BooleanOperationFailed { stage })?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

fn overlay_intersect(
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

fn overlay_difference(
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

fn reject_residual(
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

fn owned_shape_is_discardable_numeric_dust(
    shape: &NodeOverlayShape,
    area_m2: f32,
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
) -> bool {
    let protected_constraints = protected_constraints_for_owner(owner, rail_constraints);
    area_m2 <= RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(shape)
        && !shape_touches_protected_boundary_constraint(shape, &protected_constraints)
}

fn shape_touches_protected_boundary_constraint(
    shape: &NodeOverlayShape,
    protected_constraints: &[&NodeRailConstraint],
) -> bool {
    for contour in shape {
        for &point in contour {
            let point = overlay_point_key(point);
            if protected_constraints.iter().any(|constraint| {
                constraint.points_xz.windows(2).any(|segment| {
                    point_key_lies_on_segment(
                        point,
                        road_point_key(segment[0]),
                        road_point_key(segment[1]),
                    )
                })
            }) {
                return true;
            }
        }
        if contour.len() < 2 {
            continue;
        }
        for edge_index in 0..contour.len() {
            let start = contour[edge_index];
            let end = contour[(edge_index + 1) % contour.len()];
            if protected_constraints
                .iter()
                .any(|constraint| edge_lies_on_constraint(start, end, constraint))
            {
                return true;
            }
        }
    }
    false
}

fn protected_constraints_for_owner(
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
) -> Vec<&NodeRailConstraint> {
    rail_constraints
        .iter()
        .filter(move |constraint| constraint_applies_to_owner(constraint, owner))
        .filter(|constraint| {
            matches!(
                constraint.kind,
                NodeRailConstraintKind::SpanHandoff { .. }
                    | NodeRailConstraintKind::FootprintSeam { .. }
            )
        })
        .collect()
}

fn seam_constraints_for_shape(
    shape: &NodeOverlayShape,
    owner: NodeBandOwner,
    rail_constraints: &[NodeRailConstraint],
) -> Vec<NodeRegionSeamConstraint> {
    let mut seams = Vec::new();
    for contour in shape {
        if contour.len() < 2 {
            continue;
        }
        for edge_index in 0..contour.len() {
            let start = contour[edge_index];
            let end = contour[(edge_index + 1) % contour.len()];
            if overlay_point_key(start) == overlay_point_key(end) {
                continue;
            }
            for constraint in rail_constraints
                .iter()
                .filter(|constraint| constraint_applies_to_owner(constraint, owner))
            {
                if edge_lies_on_constraint(start, end, constraint) {
                    push_region_seam_constraint(
                        &mut seams,
                        constraint,
                        owner,
                        overlay_point_to_road(start),
                        overlay_point_to_road(end),
                    );
                }
                for (overlap_start, overlap_end) in
                    constraint_overlaps_shape_edge(start, end, constraint)
                {
                    push_region_seam_constraint(
                        &mut seams,
                        constraint,
                        owner,
                        road_point_from_key(overlap_start),
                        road_point_from_key(overlap_end),
                    );
                }
            }
        }
        for point in contour.iter().copied() {
            for constraint in rail_constraints
                .iter()
                .filter(|constraint| constraint_applies_to_owner(constraint, owner))
                .filter(|constraint| point_lies_on_point_constraint(point, constraint))
            {
                let point_xz = overlay_point_to_road(point);
                push_region_seam_constraint(&mut seams, constraint, owner, point_xz, point_xz);
            }
        }
    }
    canonicalize_seam_constraints(&mut seams);
    seams
}

fn push_region_seam_constraint(
    seams: &mut Vec<NodeRegionSeamConstraint>,
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    seams.push(NodeRegionSeamConstraint {
        constraint_index: constraint.constraint_index,
        seam_source: seam_source_from_constraint(constraint, owner),
        constrains_shared_height: constraint_constrains_shared_height(constraint),
        is_material_transition: constraint_is_material_transition(constraint),
        start_xz,
        end_xz,
    });
}

fn constraint_overlaps_shape_edge(
    edge_start: NodeOverlayPoint,
    edge_end: NodeOverlayPoint,
    constraint: &NodeRailConstraint,
) -> Vec<(NodeOwnershipPointKey, NodeOwnershipPointKey)> {
    let edge_start = overlay_point_key(edge_start);
    let edge_end = overlay_point_key(edge_end);
    if edge_start == edge_end || constraint.points_xz.len() < 2 {
        return Vec::new();
    }
    let mut overlaps = BTreeSet::new();
    for segment in constraint.points_xz.windows(2) {
        let start = road_point_key(segment[0]);
        let end = road_point_key(segment[1]);
        if start == end
            || !point_key_collinear_with_edge(start, edge_start, edge_end)
            || !point_key_collinear_with_edge(end, edge_start, edge_end)
        {
            continue;
        }
        let mut points = [edge_start, edge_end, start, end]
            .into_iter()
            .filter(|point| {
                point_key_lies_on_segment(*point, edge_start, edge_end)
                    && point_key_lies_on_segment(*point, start, end)
            })
            .collect::<Vec<_>>();
        points.sort_by_key(|point| segment_parameter_key(edge_start, edge_end, *point));
        points.dedup();
        let Some(first) = points.first().copied() else {
            continue;
        };
        let Some(last) = points.last().copied() else {
            continue;
        };
        if first != last {
            overlaps.insert((first, last));
        }
    }
    overlaps.into_iter().collect()
}

struct OwnedRegionBoundaryRefs {
    edges: BTreeMap<OwnedRegionEdgeKey, Vec<OwnedRegionEdgeRef>>,
    points_by_region: BTreeMap<NodeOwnershipPointKey, Vec<OwnedRegionEdgeRef>>,
}

fn owned_region_boundary_refs(
    regions: &[NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) -> OwnedRegionBoundaryRefs {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    let mut edges = BTreeMap::<OwnedRegionEdgeKey, Vec<OwnedRegionEdgeRef>>::new();
    let mut points_by_region = BTreeMap::<NodeOwnershipPointKey, Vec<OwnedRegionEdgeRef>>::new();
    for (region_index, region) in regions.iter().enumerate() {
        for contour in &region.shape {
            if contour.len() < 2 {
                continue;
            }
            for edge_index in 0..contour.len() {
                let start = overlay_point_key(contour[edge_index]);
                let end = overlay_point_key(contour[(edge_index + 1) % contour.len()]);
                if start == end {
                    continue;
                }
                let points = noded_owned_region_edge_points(start, end, &global_points);
                for segment in points.windows(2) {
                    if segment[0] == segment[1] {
                        continue;
                    }
                    let edge_ref = OwnedRegionEdgeRef {
                        region_index,
                        owner: region.owner,
                    };
                    edges
                        .entry(OwnedRegionEdgeKey::new(segment[0], segment[1]))
                        .or_default()
                        .push(edge_ref);
                    points_by_region
                        .entry(segment[0])
                        .or_default()
                        .push(edge_ref);
                    points_by_region
                        .entry(segment[1])
                        .or_default()
                        .push(edge_ref);
                }
            }
        }
    }

    OwnedRegionBoundaryRefs {
        edges,
        points_by_region,
    }
}

fn canonicalize_owned_region_rings(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    for region in regions {
        for contour in &mut region.shape {
            *contour = noded_owned_region_contour(contour, &global_points);
        }
    }
}

fn clean_canonical_owned_region_shapes(
    regions: &mut Vec<NodeBooleanOwnedRegion>,
    rail_constraints: &[NodeRailConstraint],
) -> Result<(), NodeBooleanOwnershipError> {
    let mut cleaned_regions = Vec::with_capacity(regions.len());
    for region in regions.drain(..) {
        let mut shapes = overlay_union(&region.shape, "owned_region_ring_clean")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
        for shape in shapes.into_iter().flat_map(split_self_touching_owned_shape) {
            let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&shape);
            if owned_shape_is_discardable_numeric_dust(
                &shape,
                area_m2,
                region.owner,
                rail_constraints,
            ) {
                continue;
            }
            cleaned_regions.push(NodeBooleanOwnedRegion {
                kind: region.kind,
                owner: region.owner,
                claim_priority: region.claim_priority,
                source_mouth_order_index: region.source_mouth_order_index,
                source_band_index: region.source_band_index,
                shape: shape.clone(),
                area_m2,
                seam_constraints: seam_constraints_for_shape(
                    &shape,
                    region.owner,
                    rail_constraints,
                ),
            });
        }
    }
    *regions = cleaned_regions;
    Ok(())
}

fn split_self_touching_owned_shape(shape: NodeOverlayShape) -> Vec<NodeOverlayShape> {
    if shape.len() != 1 {
        return vec![shape];
    }
    let mut pending = vec![shape[0].clone()];
    let mut split_contours = Vec::new();
    while let Some(contour) = pending.pop() {
        let Some((first, second)) = first_repeated_owned_contour_point_pair(&contour) else {
            split_contours.push(contour);
            continue;
        };

        let first_cycle = contour[first..second].to_vec();
        let mut second_cycle = Vec::with_capacity(contour.len() - (second - first));
        second_cycle.extend_from_slice(&contour[second..]);
        second_cycle.extend_from_slice(&contour[..first]);

        for cycle in [first_cycle, second_cycle] {
            if let Some(cycle) = cleaned_self_touch_split_contour(cycle) {
                pending.push(cycle);
            }
        }
    }

    if split_contours.is_empty() {
        Vec::new()
    } else {
        split_contours
            .into_iter()
            .map(|contour| vec![contour])
            .collect()
    }
}

fn first_repeated_owned_contour_point_pair(contour: &NodeOverlayContour) -> Option<(usize, usize)> {
    for first in 0..contour.len() {
        for second in first + 2..contour.len() {
            if first == 0 && second + 1 == contour.len() {
                continue;
            }
            if overlay_point_key(contour[first]) == overlay_point_key(contour[second]) {
                return Some((first, second));
            }
        }
    }
    None
}

fn cleaned_self_touch_split_contour(mut contour: NodeOverlayContour) -> Option<NodeOverlayContour> {
    dedup_consecutive_overlay_points(&mut contour);
    if contour.len() >= 2
        && overlay_point_key(contour[0])
            == overlay_point_key(*contour.last().expect("split contour has last point"))
    {
        contour.pop();
    }
    if contour.len() < 3 {
        return None;
    }
    if signed_overlay_contour_area_m2(&contour) < 0.0 {
        contour.reverse();
    }
    let shape = vec![contour.clone()];
    let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&shape);
    (area_m2 > RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(&shape)).then_some(contour)
}

fn signed_overlay_contour_area_m2(contour: &NodeOverlayContour) -> f32 {
    if contour.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for index in 0..contour.len() {
        let start = contour[index];
        let end = contour[(index + 1) % contour.len()];
        area += start[0] * end[1] - end[0] * start[1];
    }
    (area * 0.5) as f32
}

fn owned_region_global_points(
    regions: &[NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) -> Vec<NodeOwnershipPointKey> {
    let mut global_points = regions
        .iter()
        .flat_map(|region| region.shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .map(overlay_point_key)
        .chain(
            footprint_shapes
                .iter()
                .flat_map(|shape| shape.iter())
                .flat_map(|contour| contour.iter().copied())
                .map(overlay_point_key),
        )
        .collect::<Vec<_>>();
    global_points.sort_unstable();
    global_points.dedup();
    global_points
}

fn noded_owned_region_contour(
    contour: &NodeOverlayContour,
    global_points: &[NodeOwnershipPointKey],
) -> NodeOverlayContour {
    if contour.len() < 2 {
        return contour.clone();
    }

    let mut noded = Vec::with_capacity(contour.len());
    for edge_index in 0..contour.len() {
        let start = overlay_point_key(contour[edge_index]);
        let end = overlay_point_key(contour[(edge_index + 1) % contour.len()]);
        if start == end {
            continue;
        }
        let points = noded_owned_region_edge_points(start, end, global_points);
        let limit = points.len().saturating_sub(1);
        noded.extend(points.into_iter().take(limit).map(overlay_point_from_key));
    }
    dedup_consecutive_overlay_points(&mut noded);
    if noded.len() >= 2
        && overlay_point_key(noded[0])
            == overlay_point_key(*noded.last().expect("noded contour has last point"))
    {
        noded.pop();
    }
    noded
}

fn dedup_consecutive_overlay_points(points: &mut NodeOverlayContour) {
    points.dedup_by(|a, b| overlay_point_key(*a) == overlay_point_key(*b));
}

fn canonical_owned_region_edge_refs(refs: &[OwnedRegionEdgeRef]) -> Vec<OwnedRegionEdgeRef> {
    let mut refs = refs.to_vec();
    refs.sort_unstable();
    refs.dedup();
    refs
}

fn opposite_owner_for_ref(
    refs: &[OwnedRegionEdgeRef],
    edge_ref: OwnedRegionEdgeRef,
) -> Option<NodeBandOwner> {
    let mut owners = refs
        .iter()
        .map(|edge_ref| edge_ref.owner)
        .filter(|owner| *owner != edge_ref.owner)
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners.dedup();
    owners.into_iter().next()
}

fn owned_source_constraints_for_edge<'a>(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    constraints: &'a [NodeRegionSeamConstraint],
) -> Vec<&'a NodeRegionSeamConstraint> {
    let mut matches = constraints
        .iter()
        .filter(|constraint| {
            let constraint_start = road_point_key(constraint.start_xz);
            let constraint_end = road_point_key(constraint.end_xz);
            point_key_lies_on_segment(start, constraint_start, constraint_end)
                && point_key_lies_on_segment(end, constraint_start, constraint_end)
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|constraint| {
        (
            !constraint.constrains_shared_height,
            !constraint.is_material_transition,
            seam_source_priority(&constraint.seam_source),
            constraint.constraint_index,
        )
    });
    matches.dedup_by_key(|constraint| constraint.constraint_index);
    matches
}

fn owned_source_constraints_are_ambiguous(constraints: &[&NodeRegionSeamConstraint]) -> bool {
    let Some(first) = constraints.first() else {
        return false;
    };
    let first_priority = owned_seam_constraint_priority(first);
    constraints
        .iter()
        .skip(1)
        .take_while(|constraint| owned_seam_constraint_priority(constraint) == first_priority)
        .any(|constraint| constraint.seam_source != first.seam_source)
}

fn owned_seam_constraint_priority(constraint: &NodeRegionSeamConstraint) -> (bool, bool, usize) {
    (
        !constraint.constrains_shared_height,
        !constraint.is_material_transition,
        seam_source_priority(&constraint.seam_source),
    )
}

fn canonical_source_indices(sources: impl IntoIterator<Item = usize>) -> Vec<usize> {
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort_unstable();
    sources.dedup();
    sources
}

fn noded_owned_region_edge_points(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    global_points: &[NodeOwnershipPointKey],
) -> Vec<NodeOwnershipPointKey> {
    let mut split_points = global_points
        .iter()
        .copied()
        .filter(|point| *point != start && *point != end)
        .filter(|point| point_key_lies_on_segment(*point, start, end))
        .collect::<Vec<_>>();
    split_points.sort_by_key(|point| segment_parameter_key(start, end, *point));
    split_points.dedup();

    let mut points = Vec::with_capacity(split_points.len() + 2);
    points.push(start);
    points.extend(split_points);
    points.push(end);
    points
}

fn segment_parameter_key(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    point: NodeOwnershipPointKey,
) -> i128 {
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    let px = i128::from(point.0 - start.0);
    let pz = i128::from(point.1 - start.1);
    px * dx + pz * dz
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct OwnedRegionEdgeKey {
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
}

impl OwnedRegionEdgeKey {
    fn new(a: NodeOwnershipPointKey, b: NodeOwnershipPointKey) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct OwnedRegionEdgeRef {
    region_index: usize,
    owner: NodeBandOwner,
}

fn canonicalize_seam_constraints(seams: &mut Vec<NodeRegionSeamConstraint>) {
    seams.sort_by(|a, b| seam_constraint_sort_key(a).cmp(&seam_constraint_sort_key(b)));
    seams.dedup_by(|a, b| seam_constraint_sort_key(a) == seam_constraint_sort_key(b));
}

fn seam_constraint_sort_key(
    constraint: &NodeRegionSeamConstraint,
) -> (usize, NodeOwnershipPointKey, NodeOwnershipPointKey) {
    (
        constraint.constraint_index,
        road_point_key(constraint.start_xz),
        road_point_key(constraint.end_xz),
    )
}

fn road_point_from_key(point: NodeOwnershipPointKey) -> RoadVec2 {
    RoadVec2::new(
        point.0 as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
        point.1 as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
    )
}

fn overlay_point_from_key(point: NodeOwnershipPointKey) -> NodeOverlayPoint {
    [
        point.0 as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
        point.1 as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
    ]
}

fn constraint_constrains_shared_height(constraint: &NodeRailConstraint) -> bool {
    if constraint_is_point_contact(constraint) {
        return false;
    }
    matches!(
        constraint.kind,
        NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::AsphaltCurbContact
            | NodeRailConstraintKind::CurbSidewalkContact
    )
}

fn constraint_is_point_contact(constraint: &NodeRailConstraint) -> bool {
    let Some(first) = constraint.points_xz.first().copied().map(road_point_key) else {
        return false;
    };
    constraint
        .points_xz
        .iter()
        .copied()
        .map(road_point_key)
        .all(|point| point == first)
}

fn constraint_is_material_transition(constraint: &NodeRailConstraint) -> bool {
    matches!(
        constraint.kind,
        NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::AsphaltCurbContact
            | NodeRailConstraintKind::CurbSidewalkContact
            | NodeRailConstraintKind::BandBoundary { .. }
    )
}

fn constraint_applies_to_owner(constraint: &NodeRailConstraint, owner: NodeBandOwner) -> bool {
    if constraint.owner.is_some() || constraint.opposite_owner.is_some() {
        return constraint.owner == Some(owner) || constraint.opposite_owner == Some(owner);
    }
    match constraint.kind {
        NodeRailConstraintKind::FullRoadbedContour => true,
        NodeRailConstraintKind::BandContour { kind }
        | NodeRailConstraintKind::SpanHandoff { kind }
        | NodeRailConstraintKind::FootprintSeam {
            adjacent_kind: kind,
        } => kind == owner.kind(),
        NodeRailConstraintKind::AsphaltBoundary { adjacent_kind } => {
            is_carriageway(owner.kind()) || adjacent_kind == owner.kind()
        }
        NodeRailConstraintKind::AsphaltCurbContact => {
            is_carriageway(owner.kind()) || is_curb_or_shoulder(owner.kind())
        }
        NodeRailConstraintKind::CurbSidewalkContact => {
            is_curb_or_shoulder(owner.kind()) || is_sidewalk(owner.kind())
        }
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => left_kind == owner.kind() || right_kind == owner.kind(),
    }
}

fn edge_lies_on_constraint(
    edge_start: NodeOverlayPoint,
    edge_end: NodeOverlayPoint,
    constraint: &NodeRailConstraint,
) -> bool {
    if constraint.points_xz.len() < 2 {
        return false;
    }
    let edge_start = overlay_point_key(edge_start);
    let edge_end = overlay_point_key(edge_end);
    constraint.points_xz.windows(2).any(|segment| {
        let start = road_point_key(segment[0]);
        let end = road_point_key(segment[1]);
        point_key_lies_on_segment(edge_start, start, end)
            && point_key_lies_on_segment(edge_end, start, end)
    }) || edge_lies_on_constraint_polyline(edge_start, edge_end, constraint)
        || edge_endpoints_lie_on_constraint_path(edge_start, edge_end, constraint)
}

fn edge_lies_on_constraint_polyline(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    if edge_start == edge_end || constraint.points_xz.len() < 2 {
        return false;
    }
    let edge_end_parameter = segment_parameter_key(edge_start, edge_end, edge_end);
    if edge_end_parameter <= 0 {
        return false;
    }
    let mut intervals = Vec::new();
    for segment in constraint.points_xz.windows(2) {
        let start = road_point_key(segment[0]);
        let end = road_point_key(segment[1]);
        if start == end
            || !point_key_collinear_with_edge(start, edge_start, edge_end)
            || !point_key_collinear_with_edge(end, edge_start, edge_end)
        {
            continue;
        }
        let start_parameter = segment_parameter_key(edge_start, edge_end, start);
        let end_parameter = segment_parameter_key(edge_start, edge_end, end);
        let overlap_start = start_parameter.min(end_parameter).max(0);
        let overlap_end = start_parameter.max(end_parameter).min(edge_end_parameter);
        if overlap_start < overlap_end {
            intervals.push((overlap_start, overlap_end));
        }
    }
    if intervals.is_empty() {
        return false;
    }
    intervals.sort_unstable();
    let mut covered_end = 0;
    for (start, end) in intervals {
        if start > covered_end {
            return false;
        }
        covered_end = covered_end.max(end);
        if covered_end >= edge_end_parameter {
            return true;
        }
    }
    false
}

fn edge_endpoints_lie_on_constraint_path(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    if edge_start == edge_end
        || constraint.points_xz.len() < 2
        || !constraint_allows_path_chord(constraint)
    {
        return false;
    }
    constraint_path_contains_ordered_endpoints(edge_start, edge_end, constraint)
        || constraint_path_contains_ordered_endpoints(edge_end, edge_start, constraint)
        || constraint_path_contains_ordered_project_endpoints(edge_start, edge_end, constraint)
        || constraint_path_contains_ordered_project_endpoints(edge_end, edge_start, constraint)
}

fn constraint_path_contains_ordered_endpoints(
    first: NodeOwnershipPointKey,
    second: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    let mut first_seen = false;
    for segment in constraint.points_xz.windows(2) {
        let start = road_point_key(segment[0]);
        let end = road_point_key(segment[1]);
        if point_key_lies_on_segment(first, start, end) {
            first_seen = true;
        }
        if first_seen && point_key_lies_on_segment(second, start, end) {
            return true;
        }
    }
    false
}

fn constraint_path_contains_ordered_project_endpoints(
    first: NodeOwnershipPointKey,
    second: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    let first = project_ownership_key(first);
    let second = project_ownership_key(second);
    let mut first_seen = false;
    for segment in constraint.points_xz.windows(2) {
        let start = project_ownership_key(road_point_key(segment[0]));
        let end = project_ownership_key(road_point_key(segment[1]));
        if point_key_lies_on_segment(first, start, end) {
            first_seen = true;
        }
        if first_seen && point_key_lies_on_segment(second, start, end) {
            return true;
        }
    }
    false
}

fn project_ownership_key(point: NodeOwnershipPointKey) -> NodeOwnershipPointKey {
    (
        ownership_coordinate_key_to_mm(point.0),
        ownership_coordinate_key_to_mm(point.1),
    )
}

fn constraint_allows_path_chord(constraint: &NodeRailConstraint) -> bool {
    matches!(
        constraint.kind,
        NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::AsphaltCurbContact
            | NodeRailConstraintKind::CurbSidewalkContact
            | NodeRailConstraintKind::BandBoundary { .. }
    )
}

fn point_key_collinear_with_edge(
    point: NodeOwnershipPointKey,
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
) -> bool {
    let dx = i128::from(edge_end.0 - edge_start.0);
    let dz = i128::from(edge_end.1 - edge_start.1);
    let px = i128::from(point.0 - edge_start.0);
    let pz = i128::from(point.1 - edge_start.1);
    px * dz - pz * dx == 0
}

fn point_lies_on_point_constraint(
    point: NodeOverlayPoint,
    constraint: &NodeRailConstraint,
) -> bool {
    if constraint.points_xz.len() < 2 {
        return false;
    }
    let point = overlay_point_key(point);
    constraint.points_xz.windows(2).any(|segment| {
        let start = road_point_key(segment[0]);
        let end = road_point_key(segment[1]);
        start == end && point == start
    })
}

fn point_key_lies_on_segment(
    point: NodeOwnershipPointKey,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> bool {
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    let px = i128::from(point.0 - start.0);
    let pz = i128::from(point.1 - start.1);
    let cross = px * dz - pz * dx;
    if cross != 0 && cross.abs() > overlay_grid_collinearity_error_bound(dx, dz) {
        return false;
    }
    let inside_x = if start.0 == end.0 {
        point.0 == start.0
    } else {
        point.0 > start.0.min(end.0) && point.0 < start.0.max(end.0)
    };
    let inside_z = if start.1 == end.1 {
        point.1 == start.1
    } else {
        point.1 > start.1.min(end.1) && point.1 < start.1.max(end.1)
    };
    inside_x && inside_z
}

fn overlay_grid_collinearity_error_bound(dx: i128, dz: i128) -> i128 {
    // Source contours and backend-owned shapes are both projected to the overlay integer grid.
    // A point that is exactly on a source segment before projection can land within this
    // determinant envelope after independent endpoint rounding; this is representation noding,
    // not owner or height repair.
    (dx.abs() + dz.abs()) * 2
}

fn seam_source_from_constraint(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
) -> NodeSeamSource {
    match constraint.kind {
        NodeRailConstraintKind::AsphaltCurbContact => NodeSeamSource::AsphaltCurbContact {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::AsphaltBoundary { .. } => NodeSeamSource::AsphaltBoundary {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::CurbSidewalkContact => NodeSeamSource::CurbSidewalkContact {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::FootprintSeam { .. }
        | NodeRailConstraintKind::FullRoadbedContour => NodeSeamSource::FootprintBoundary {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::BandBoundary { .. } => seam_source_for_owner(owner),
    }
}

fn seam_source_for_owner(owner: NodeBandOwner) -> NodeSeamSource {
    match owner.kind() {
        RoadSurfaceBandKind::Carriageway => NodeSeamSource::AsphaltBoundary {
            owner_index: owner.owner_index(),
        },
        RoadSurfaceBandKind::CurbOrShoulder => NodeSeamSource::AsphaltCurbContact {
            owner_index: owner.owner_index(),
        },
        RoadSurfaceBandKind::Sidewalk => NodeSeamSource::SidewalkOuter {
            owner_index: owner.owner_index(),
        },
        _ => NodeSeamSource::FootprintBoundary {
            owner_index: owner.owner_index(),
        },
    }
}

pub(crate) type NodeOwnershipPointKey = (i64, i64);
const NODE_OWNERSHIP_KEY_UNITS_PER_MM: i64 = 1000;

fn ownership_coordinate_key_to_mm(value: i64) -> i64 {
    if value >= 0 {
        (value + NODE_OWNERSHIP_KEY_UNITS_PER_MM / 2) / NODE_OWNERSHIP_KEY_UNITS_PER_MM
    } else {
        (value - NODE_OWNERSHIP_KEY_UNITS_PER_MM / 2) / NODE_OWNERSHIP_KEY_UNITS_PER_MM
    }
}

fn overlay_point_key(point: NodeOverlayPoint) -> NodeOwnershipPointKey {
    (
        (point[0] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point[1] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

fn road_point_key(point: RoadVec2) -> NodeOwnershipPointKey {
    (
        (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

fn is_carriageway(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::Carriageway
}

fn is_curb_or_shoulder(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::CurbOrShoulder
}

fn is_sidewalk(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::Sidewalk
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

fn is_non_road_band(kind: RoadSurfaceBandKind) -> bool {
    non_road_band_order().contains(&kind)
}

fn sort_boolean_owned_regions(regions: &mut [NodeBooleanOwnedRegion]) {
    regions.sort_by(|a, b| {
        RoadSurfaceSystem::band_kind_sort_key(a.kind)
            .cmp(&RoadSurfaceSystem::band_kind_sort_key(b.kind))
            .then(a.claim_priority.cmp(&b.claim_priority))
            .then(a.source_mouth_order_index.cmp(&b.source_mouth_order_index))
            .then(a.source_band_index.cmp(&b.source_band_index))
            .then(a.area_m2.total_cmp(&b.area_m2))
    });
}

#[derive(Clone, Copy)]
enum ResidualKind {
    Asphalt,
    Band(RoadSurfaceBandKind),
    NonRoad,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::input::NodeArrangementInput;
    use crate::simulation::network::surface::rails::NodeRailContourSet;
    use crate::simulation::network::surface::{
        IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    };
    use godot::prelude::{Vector2, Vector3};

    fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn profile(x: f32) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.0, -4.0),
            Vector3::new(x, 4.1, -2.0),
            Vector3::new(x, 4.2, 0.0),
            Vector3::new(x, 4.3, 2.0),
            Vector3::new(x, 4.4, 4.0),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: Vector2::RIGHT,
            boundary_points_world,
            bands,
        }
    }

    fn contour_set() -> NodeRailContourSet {
        let mouth = OrderedIncidentPieceMouth {
            profile: profile(10.0),
            endpoint_profile: profile(0.0),
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        };
        let input = NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[mouth],
        )
        .expect("test mouth should produce canonical input");
        NodeRailContourSet::from_input(&input).expect("test input should produce contours")
    }

    #[test]
    fn boolean_ownership_produces_asphalt_and_band_owned_regions() {
        let ownership =
            NodeBooleanOwnership::from_rails(&contour_set()).expect("valid ownership solve");

        assert_eq!(ownership.node_id, 42);
        assert_eq!(
            ownership.piece_kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_eq!(ownership.footprint_shapes.len(), 1);
        assert_eq!(ownership.asphalt_shapes.len(), 1);
        assert_eq!(ownership.non_road_shapes.len(), 2);
        assert_eq!(ownership.owned_regions.len(), 4);
        assert_eq!(ownership.owned_region_arrangement.region_count(), 4);
        assert!(ownership.owned_region_arrangement.diagnostics().is_empty());
        assert!(!ownership.owned_region_arrangement.edges().is_empty());
        assert!(
            ownership
                .owned_regions
                .iter()
                .any(|region| region.kind == RoadSurfaceBandKind::Carriageway
                    && region.owner == NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2)
                    && !region.seam_constraints.is_empty())
        );
        assert!(
            ownership.owned_regions.iter().any(|region| {
                region.seam_constraints.iter().any(|constraint| {
                    matches!(
                        constraint.seam_source,
                        NodeSeamSource::AsphaltCurbContact { .. }
                            | NodeSeamSource::CurbSidewalkContact { .. }
                            | NodeSeamSource::FootprintBoundary { .. }
                    )
                })
            }),
            "owned regions must preserve source rail seam constraints"
        );
        assert_eq!(
            ownership
                .owned_regions
                .iter()
                .filter(|region| region.kind == RoadSurfaceBandKind::Sidewalk)
                .count(),
            2
        );
    }

    #[test]
    fn boolean_ownership_rejects_unowned_non_road_residual() {
        let mut rails = contour_set();
        rails.contours.retain(|contour| {
            contour.kind == NodeGeneratedContourKind::FullRoadbed
                || contour.kind
                    == NodeGeneratedContourKind::Band {
                        kind: RoadSurfaceBandKind::Carriageway,
                    }
        });

        let error = NodeBooleanOwnership::from_rails(&rails)
            .expect_err("non-road footprint without band contours must be rejected");

        assert!(matches!(
            error,
            NodeBooleanOwnershipError::UnownedNonRoadResidual { .. }
        ));
    }

    #[test]
    fn protected_span_handoff_dust_stays_owned() {
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3);
        let shape = vec![vec![[0.0, 0.0], [0.0001, 0.0], [0.0, 0.0001]]];
        let constraints = vec![NodeRailConstraint {
            constraint_index: 7,
            kind: NodeRailConstraintKind::SpanHandoff {
                kind: RoadSurfaceBandKind::Sidewalk,
            },
            source_mouth_order_index: 0,
            source_band_index: Some(0),
            source_boundary_index: None,
            owner: Some(owner),
            opposite_owner: None,
            points_xz: vec![RoadVec2::new(0.0, 0.0), RoadVec2::new(0.0001, 0.0)],
        }];

        assert!(
            !owned_shape_is_discardable_numeric_dust(
                &shape,
                RoadSurfaceSystem::overlay_shape_area_m2(&shape),
                owner,
                &constraints,
            ),
            "span-handoff dust must remain an owned top region so mouth/skirt seams cannot point at missing top mesh"
        );
    }

    #[test]
    fn unprotected_numeric_dust_can_still_be_discarded() {
        let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3);
        let shape = vec![vec![[0.0, 0.0], [0.0001, 0.0], [0.0, 0.0001]]];

        assert!(owned_shape_is_discardable_numeric_dust(
            &shape,
            RoadSurfaceSystem::overlay_shape_area_m2(&shape),
            owner,
            &[],
        ));
    }

    #[test]
    fn owned_region_rings_are_noded_before_explicit_seam_validation() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::Sidewalk,
                sidewalk,
                vec![[1.0, 0.0], [3.0, 0.0], [3.0, -1.0], [1.0, -1.0]],
            ),
        ];
        let footprint_shapes = Vec::new();

        canonicalize_owned_region_rings(&mut regions, &footprint_shapes);
        for region in &mut regions {
            region.seam_constraints.push(NodeRegionSeamConstraint {
                constraint_index: 0,
                seam_source: NodeSeamSource::AsphaltBoundary {
                    owner_index: region.owner.owner_index(),
                },
                constrains_shared_height: false,
                is_material_transition: true,
                start_xz: RoadVec2::new(1.0, 0.0),
                end_xz: RoadVec2::new(3.0, 0.0),
            });
            canonicalize_seam_constraints(&mut region.seam_constraints);
        }

        let carriageway_contour = &regions[0].shape[0];
        assert!(
            carriageway_contour
                .iter()
                .any(|point| overlay_point_key(*point) == overlay_point_key([1.0, 0.0]))
        );
        assert!(
            carriageway_contour
                .iter()
                .any(|point| overlay_point_key(*point) == overlay_point_key([3.0, 0.0]))
        );
        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    let start = road_point_key(constraint.start_xz);
                    let end = road_point_key(constraint.end_xz);
                    start == road_point_key(RoadVec2::new(1.0, 0.0))
                        && end == road_point_key(RoadVec2::new(3.0, 0.0))
                        && constraint.is_material_transition
                        && !constraint.constrains_shared_height
                }),
                "region {:?} must own the exact shared sub-edge seam before height/CDT without inventing height authority",
                region.owner
            );
        }
        let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &regions,
            &footprint_shapes,
        );
        assert!(arrangement.diagnostics().is_empty());
        assert!(arrangement.edges().iter().any(|edge| {
            edge.owner == carriageway
                && edge.opposite_owner == Some(sidewalk)
                && edge.start == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(1.0, 0.0))
                && edge.end == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(3.0, 0.0))
                && !edge.source_constraint_indices.is_empty()
        }));
    }

    #[test]
    fn explicit_shared_point_constraints_preserve_endpoint_context_without_height_continuity() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::Sidewalk,
                sidewalk,
                vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0]],
            ),
        ];

        for region in &mut regions {
            region.seam_constraints.push(NodeRegionSeamConstraint {
                constraint_index: 0,
                seam_source: NodeSeamSource::AsphaltBoundary {
                    owner_index: region.owner.owner_index(),
                },
                constrains_shared_height: false,
                is_material_transition: true,
                start_xz: RoadVec2::new(1.0, 1.0),
                end_xz: RoadVec2::new(1.0, 1.0),
            });
            canonicalize_seam_constraints(&mut region.seam_constraints);
        }

        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    let start = road_point_key(constraint.start_xz);
                    let end = road_point_key(constraint.end_xz);
                    start == road_point_key(RoadVec2::new(1.0, 1.0))
                        && end == start
                        && constraint.is_material_transition
                        && !constraint.constrains_shared_height
                }),
                "point-only material contacts must remain explicit seam endpoints without asserting one shared height"
            );
        }
    }

    #[test]
    fn owned_region_arrangement_reports_shared_edge_without_seam_constraint() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
        let regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::Sidewalk,
                sidewalk,
                vec![[4.0, 0.0], [6.0, 0.0], [6.0, 2.0], [4.0, 2.0]],
            ),
        ];

        let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            43,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &regions,
            &Vec::new(),
        );

        assert!(arrangement.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic,
            NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
                region_index: 0,
                owner,
                opposite_owner,
                start,
                end,
            } if *owner == carriageway
                && *opposite_owner == sidewalk
                && *start == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(4.0, 0.0))
                && *end == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(4.0, 2.0))
        )));
    }

    fn test_owned_region(
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        contour: NodeOverlayContour,
    ) -> NodeBooleanOwnedRegion {
        NodeBooleanOwnedRegion {
            kind,
            owner,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            source_mouth_order_index: owner.owner_index(),
            source_band_index: Some(owner.owner_index()),
            shape: vec![contour],
            area_m2: 1.0,
            seam_constraints: Vec::new(),
        }
    }
}
