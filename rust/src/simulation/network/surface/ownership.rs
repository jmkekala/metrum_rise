//! Boolean ownership solve for canonical node-arrangement contours.

#![allow(dead_code)]

use super::arrangement::{
    NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource, seam_source_priority,
};
use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2, overlay_point_to_road, road_vec2_to_overlay_point,
};
use super::rails::{
    NodeGeneratedContour, NodeGeneratedContourKind, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailContourSet,
};
use super::{
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::BTreeMap;

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
        append_shared_region_seam_constraints(
            &mut owned_regions,
            &footprint_shapes,
            rails.constraints.len(),
        );
        resolve_same_band_role_crossing_joins(&mut owned_regions, &rails.constraints);
        canonicalize_owned_region_rings(&mut owned_regions, &footprint_shapes);
        let next_constraint_index = next_region_constraint_index(&owned_regions);
        append_missing_shared_region_seam_constraints(
            &mut owned_regions,
            &footprint_shapes,
            next_constraint_index,
        );
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

    pub(crate) fn resolve_canonical_contact_ownership(&mut self) {
        let next_constraint_index = next_region_constraint_index(&self.owned_regions);
        append_missing_shared_region_seam_constraints(
            &mut self.owned_regions,
            &self.footprint_shapes,
            next_constraint_index,
        );
        self.owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            self.node_id,
            self.piece_kind,
            &self.owned_regions,
            &self.footprint_shapes,
        );
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
                    .unwrap_or_else(|| {
                        opposite_owner
                            .map(|opposite_owner| {
                                shared_region_seam_source(edge_ref.owner, opposite_owner)
                            })
                            .unwrap_or_else(|| seam_source_for_owner(edge_ref.owner))
                    });
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

    let residual = overlay_difference(target_shapes, &claimed_shapes, "domain_residual")?;
    if !residual.is_empty() {
        let residual_result = residual_regions_from_domains(
            &residual,
            domains,
            rail_constraints,
            "domain_residual_reclaim",
        )?;
        regions.extend(residual_result.regions);
        claimed_shapes = overlay_union_shape_sets(
            &claimed_shapes,
            &residual_result.claimed_shapes,
            "domain_residual_claim_union",
        )?;
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
            source_mouth_order_index: domain.source_mouth_order_index,
            source_band_index: domain.source_band_index,
            domains: vec![*domain],
        });
    }
    Ok(groups)
}

fn residual_regions_from_domains(
    residual_shapes: &NodeOverlayShapes,
    domains: &[&NodeGeneratedContour],
    rail_constraints: &[NodeRailConstraint],
    stage: &'static str,
) -> Result<OwnedDomainResult, NodeBooleanOwnershipError> {
    let mut regions = Vec::new();
    let mut claimed_shapes = Vec::new();
    for group in owned_domain_groups(domains)? {
        let domain_contours = group
            .domains
            .iter()
            .map(|domain| overlay_contour_from_domain(domain))
            .collect::<Vec<_>>();
        let mut domain_shapes = overlay_union(&domain_contours, stage)?;
        domain_shapes = overlay_intersect(&domain_shapes, residual_shapes, stage)?;
        domain_shapes = overlay_difference(&domain_shapes, &claimed_shapes, stage)?;
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
                source_mouth_order_index: group.source_mouth_order_index,
                source_band_index: group.source_band_index,
                shape: shape.clone(),
                area_m2,
                seam_constraints: seam_constraints_for_shape(shape, group.owner, rail_constraints),
            });
        }
        claimed_shapes = overlay_union_shape_sets(&claimed_shapes, &domain_shapes, stage)?;
    }
    Ok(OwnedDomainResult {
        regions,
        claimed_shapes,
    })
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
                .filter(|constraint| edge_lies_on_constraint(start, end, constraint))
            {
                seams.push(NodeRegionSeamConstraint {
                    constraint_index: constraint.constraint_index,
                    seam_source: seam_source_from_constraint(constraint, owner),
                    constrains_shared_height: constraint_constrains_shared_height(constraint),
                    is_material_transition: constraint_is_material_transition(constraint),
                    start_xz: overlay_point_to_road(start),
                    end_xz: overlay_point_to_road(end),
                });
            }
        }
    }
    seams.sort_by(|a, b| seam_constraint_sort_key(a).cmp(&seam_constraint_sort_key(b)));
    seams.dedup_by(|a, b| seam_constraint_sort_key(a) == seam_constraint_sort_key(b));
    seams
}

fn append_shared_region_seam_constraints(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    first_constraint_index: usize,
) {
    let boundary_refs = owned_region_boundary_refs(regions, footprint_shapes);
    let mut next_constraint_index = first_constraint_index;
    for (edge, refs) in boundary_refs.edges {
        if append_shared_constraint_for_refs(
            regions,
            &refs,
            next_constraint_index,
            edge.start,
            edge.end,
        ) {
            next_constraint_index += 1;
        }
    }
    for (point, refs) in boundary_refs.points_by_region {
        if append_shared_constraint_for_refs(regions, &refs, next_constraint_index, point, point) {
            next_constraint_index += 1;
        }
    }

    for region in regions {
        canonicalize_seam_constraints(&mut region.seam_constraints);
    }
}

pub(crate) fn append_missing_shared_region_seam_constraints(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    first_constraint_index: usize,
) {
    let boundary_refs = owned_region_boundary_refs(regions, footprint_shapes);
    let mut next_constraint_index = first_constraint_index;
    for (edge, refs) in boundary_refs.edges {
        if append_missing_shared_constraint_for_refs(
            regions,
            &refs,
            next_constraint_index,
            edge.start,
            edge.end,
        ) {
            next_constraint_index += 1;
        }
    }
    for (point, refs) in boundary_refs.points_by_region {
        if append_missing_shared_constraint_for_refs(
            regions,
            &refs,
            next_constraint_index,
            point,
            point,
        ) {
            next_constraint_index += 1;
        }
    }

    for region in regions {
        canonicalize_seam_constraints(&mut region.seam_constraints);
    }
}

fn next_region_constraint_index(regions: &[NodeBooleanOwnedRegion]) -> usize {
    regions
        .iter()
        .flat_map(|region| region.seam_constraints.iter())
        .map(|constraint| constraint.constraint_index)
        .max()
        .map_or(0, |index| index + 1)
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

fn append_shared_constraint_for_refs(
    regions: &mut [NodeBooleanOwnedRegion],
    refs: &[OwnedRegionEdgeRef],
    constraint_index: usize,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> bool {
    let mut refs = refs.to_vec();
    refs.sort_unstable();
    refs.dedup();

    let mut owners = refs
        .iter()
        .map(|edge_ref| edge_ref.owner)
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners.dedup();
    if owners.len() < 2 {
        return false;
    }

    for edge_ref in refs {
        let Some(opposite_owner) = owners
            .iter()
            .copied()
            .find(|owner| *owner != edge_ref.owner)
        else {
            continue;
        };
        regions[edge_ref.region_index]
            .seam_constraints
            .push(NodeRegionSeamConstraint {
                constraint_index,
                seam_source: shared_region_seam_source(edge_ref.owner, opposite_owner),
                constrains_shared_height: generated_shared_constraint_constrains_height(
                    edge_ref.owner,
                    opposite_owner,
                    start,
                    end,
                ),
                is_material_transition: true,
                start_xz: road_point_from_key(start),
                end_xz: road_point_from_key(end),
            });
    }
    true
}

fn append_missing_shared_constraint_for_refs(
    regions: &mut [NodeBooleanOwnedRegion],
    refs: &[OwnedRegionEdgeRef],
    constraint_index: usize,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> bool {
    let mut refs = refs.to_vec();
    refs.sort_unstable();
    refs.dedup();

    let mut owners = refs
        .iter()
        .map(|edge_ref| edge_ref.owner)
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners.dedup();
    if owners.len() < 2 {
        return false;
    }

    let mut missing_refs = Vec::new();
    let mut existing_template = None;
    for edge_ref in &refs {
        let Some(region) = regions.get(edge_ref.region_index) else {
            continue;
        };
        let source_constraints =
            owned_source_constraints_for_edge(start, end, &region.seam_constraints);
        if source_constraints.is_empty() {
            missing_refs.push(*edge_ref);
        } else if existing_template.is_none() {
            existing_template = source_constraints.first().map(|constraint| {
                (
                    constraint.constraint_index,
                    constraint.constrains_shared_height,
                    constraint.is_material_transition,
                )
            });
        }
    }
    if missing_refs.is_empty() {
        return false;
    }

    let uses_new_index = existing_template.is_none();

    for edge_ref in missing_refs {
        let Some(opposite_owner) = owners
            .iter()
            .copied()
            .find(|owner| *owner != edge_ref.owner)
        else {
            continue;
        };
        let (assigned_index, constrains_shared_height, is_material_transition) = existing_template
            .unwrap_or((
                constraint_index,
                generated_shared_constraint_constrains_height(
                    edge_ref.owner,
                    opposite_owner,
                    start,
                    end,
                ),
                true,
            ));
        regions[edge_ref.region_index]
            .seam_constraints
            .push(NodeRegionSeamConstraint {
                constraint_index: assigned_index,
                seam_source: shared_region_seam_source(edge_ref.owner, opposite_owner),
                constrains_shared_height,
                is_material_transition,
                start_xz: road_point_from_key(start),
                end_xz: road_point_from_key(end),
            });
    }
    uses_new_index
}

fn generated_shared_constraint_constrains_height(
    _owner: NodeBandOwner,
    _opposite_owner: NodeBandOwner,
    _start: NodeOwnershipPointKey,
    _end: NodeOwnershipPointKey,
) -> bool {
    false
}

#[derive(Clone, Copy)]
struct SameBandRoleJoinRewrite {
    donor_region_index: usize,
    receiver_region_index: usize,
    old_constraint_index: Option<usize>,
    new_constraint_index: usize,
    equal_key: NodeOwnershipPointKey,
    conflict_key: NodeOwnershipPointKey,
    candidate_key: NodeOwnershipPointKey,
}

#[derive(Clone, Copy)]
struct SameBandRolePointRewrite {
    donor_region_index: usize,
    new_constraint_index: usize,
    previous_key: NodeOwnershipPointKey,
    conflict_key: NodeOwnershipPointKey,
    next_key: NodeOwnershipPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum SameBandBoundaryRole {
    LowerSide,
    RaisedSide,
}

fn resolve_same_band_role_crossing_joins(
    regions: &mut [NodeBooleanOwnedRegion],
    rail_constraints: &[NodeRailConstraint],
) {
    let mut next_constraint_index = next_region_constraint_index(regions);
    let max_passes = regions.len().saturating_mul(regions.len()).max(1);

    for _ in 0..max_passes {
        if let Some(rewrite) =
            same_band_role_join_rewrite_candidate(regions, rail_constraints, next_constraint_index)
        {
            apply_same_band_role_join_rewrite(regions, rewrite);
            next_constraint_index += 1;
            continue;
        }
        if let Some(rewrite) =
            same_band_role_point_rewrite_candidate(regions, rail_constraints, next_constraint_index)
        {
            apply_same_band_role_point_rewrite(regions, rewrite);
            next_constraint_index += 1;
            continue;
        }
        return;
    }
}

fn same_band_role_point_rewrite_candidate(
    regions: &[NodeBooleanOwnedRegion],
    rail_constraints: &[NodeRailConstraint],
    new_constraint_index: usize,
) -> Option<SameBandRolePointRewrite> {
    for left_index in 0..regions.len() {
        for right_index in left_index + 1..regions.len() {
            let left = &regions[left_index];
            let right = &regions[right_index];
            if left.kind != right.kind || left.owner.kind() != right.owner.kind() {
                continue;
            }
            for key in shared_region_points(left, right) {
                let Some(left_role) = same_band_boundary_role_at_key(left, rail_constraints, key)
                else {
                    continue;
                };
                let Some(right_role) = same_band_boundary_role_at_key(right, rail_constraints, key)
                else {
                    continue;
                };
                if left_role == right_role {
                    continue;
                }
                if left_role == SameBandBoundaryRole::RaisedSide {
                    if let Some((previous_key, next_key)) =
                        removable_role_crossing_point(left, rail_constraints, key, right_role)
                    {
                        return Some(SameBandRolePointRewrite {
                            donor_region_index: left_index,
                            new_constraint_index,
                            previous_key,
                            conflict_key: key,
                            next_key,
                        });
                    }
                }
                if right_role == SameBandBoundaryRole::RaisedSide {
                    if let Some((previous_key, next_key)) =
                        removable_role_crossing_point(right, rail_constraints, key, left_role)
                    {
                        return Some(SameBandRolePointRewrite {
                            donor_region_index: right_index,
                            new_constraint_index,
                            previous_key,
                            conflict_key: key,
                            next_key,
                        });
                    }
                }
            }
        }
    }
    None
}

fn shared_region_points(
    left: &NodeBooleanOwnedRegion,
    right: &NodeBooleanOwnedRegion,
) -> Vec<NodeOwnershipPointKey> {
    let mut left_points = region_points(left);
    let mut right_points = region_points(right);
    left_points.sort_unstable();
    left_points.dedup();
    right_points.sort_unstable();
    right_points.dedup();
    left_points
        .into_iter()
        .filter(|point| right_points.binary_search(point).is_ok())
        .collect()
}

fn region_points(region: &NodeBooleanOwnedRegion) -> Vec<NodeOwnershipPointKey> {
    region
        .shape
        .iter()
        .flat_map(|contour| contour.iter().copied())
        .map(overlay_point_key)
        .collect()
}

fn removable_role_crossing_point(
    region: &NodeBooleanOwnedRegion,
    rail_constraints: &[NodeRailConstraint],
    conflict_key: NodeOwnershipPointKey,
    opposite_role: SameBandBoundaryRole,
) -> Option<(NodeOwnershipPointKey, NodeOwnershipPointKey)> {
    for contour in &region.shape {
        if contour.len() < 4 {
            continue;
        }
        let keys = contour
            .iter()
            .copied()
            .map(overlay_point_key)
            .collect::<Vec<_>>();
        for index in 0..keys.len() {
            if keys[index] != conflict_key {
                continue;
            }
            let previous_index = if index == 0 {
                keys.len() - 1
            } else {
                index - 1
            };
            let next_index = if index + 1 == keys.len() {
                0
            } else {
                index + 1
            };
            let previous_key = keys[previous_index];
            let next_key = keys[next_index];
            if previous_key == next_key
                || triangle_double_area(previous_key, conflict_key, next_key) == 0
            {
                continue;
            }
            let previous_role =
                same_band_boundary_role_at_key(region, rail_constraints, previous_key);
            let next_role = same_band_boundary_role_at_key(region, rail_constraints, next_key);
            if previous_role == Some(opposite_role) || next_role == Some(opposite_role) {
                return Some((previous_key, next_key));
            }
        }
    }
    None
}

fn apply_same_band_role_point_rewrite(
    regions: &mut [NodeBooleanOwnedRegion],
    rewrite: SameBandRolePointRewrite,
) {
    let Some(region) = regions.get_mut(rewrite.donor_region_index) else {
        return;
    };
    remove_join_corner_from_region(
        region,
        rewrite.previous_key,
        rewrite.conflict_key,
        rewrite.next_key,
    );
    region.seam_constraints.push(same_band_role_join_constraint(
        rewrite.new_constraint_index,
        region.owner.owner_index(),
        rewrite.previous_key,
        rewrite.next_key,
    ));
    region.area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&region.shape);
    canonicalize_seam_constraints(&mut region.seam_constraints);
}

fn same_band_role_join_rewrite_candidate(
    regions: &[NodeBooleanOwnedRegion],
    rail_constraints: &[NodeRailConstraint],
    new_constraint_index: usize,
) -> Option<SameBandRoleJoinRewrite> {
    for left_index in 0..regions.len() {
        for right_index in left_index + 1..regions.len() {
            let left = &regions[left_index];
            let right = &regions[right_index];
            if left.kind != right.kind || left.owner.kind() != right.owner.kind() {
                continue;
            }
            for (start, end) in shared_region_edges(left, right) {
                let start_roles_match =
                    same_band_endpoint_roles_match(left, right, rail_constraints, start);
                let end_roles_match =
                    same_band_endpoint_roles_match(left, right, rail_constraints, end);
                let Some((equal_key, conflict_key)) =
                    one_matching_role_endpoint(start, end, start_roles_match, end_roles_match)
                else {
                    continue;
                };
                let old_constraint_index =
                    same_band_shared_constraint_index(left, right, equal_key, conflict_key);
                if let Some(candidate_key) = role_matching_join_candidate(
                    left,
                    right,
                    rail_constraints,
                    equal_key,
                    conflict_key,
                ) {
                    return Some(SameBandRoleJoinRewrite {
                        donor_region_index: left_index,
                        receiver_region_index: right_index,
                        old_constraint_index,
                        new_constraint_index,
                        equal_key,
                        conflict_key,
                        candidate_key,
                    });
                }
                if let Some(candidate_key) = role_matching_join_candidate(
                    right,
                    left,
                    rail_constraints,
                    equal_key,
                    conflict_key,
                ) {
                    return Some(SameBandRoleJoinRewrite {
                        donor_region_index: right_index,
                        receiver_region_index: left_index,
                        old_constraint_index,
                        new_constraint_index,
                        equal_key,
                        conflict_key,
                        candidate_key,
                    });
                }
            }
        }
    }
    None
}

fn shared_region_edges(
    left: &NodeBooleanOwnedRegion,
    right: &NodeBooleanOwnedRegion,
) -> Vec<(NodeOwnershipPointKey, NodeOwnershipPointKey)> {
    let mut left_edges = region_edges(left);
    let mut right_edges = region_edges(right);
    left_edges.sort_unstable();
    left_edges.dedup();
    right_edges.sort_unstable();
    right_edges.dedup();
    left_edges
        .into_iter()
        .filter(|edge| right_edges.binary_search(edge).is_ok())
        .map(|edge| (edge.start, edge.end))
        .collect()
}

fn region_edges(region: &NodeBooleanOwnedRegion) -> Vec<OwnedRegionEdgeKey> {
    let mut edges = Vec::new();
    for contour in &region.shape {
        if contour.len() < 2 {
            continue;
        }
        for edge_index in 0..contour.len() {
            let start = overlay_point_key(contour[edge_index]);
            let end = overlay_point_key(contour[(edge_index + 1) % contour.len()]);
            if start != end {
                edges.push(OwnedRegionEdgeKey::new(start, end));
            }
        }
    }
    edges
}

fn same_band_endpoint_roles_match(
    left: &NodeBooleanOwnedRegion,
    right: &NodeBooleanOwnedRegion,
    rail_constraints: &[NodeRailConstraint],
    key: NodeOwnershipPointKey,
) -> Option<bool> {
    let left_role = same_band_boundary_role_at_key(left, rail_constraints, key)?;
    let right_role = same_band_boundary_role_at_key(right, rail_constraints, key)?;
    Some(left_role == right_role)
}

fn same_band_boundary_role_at_key(
    region: &NodeBooleanOwnedRegion,
    rail_constraints: &[NodeRailConstraint],
    key: NodeOwnershipPointKey,
) -> Option<SameBandBoundaryRole> {
    if region.kind != RoadSurfaceBandKind::CurbOrShoulder {
        return None;
    }

    let mut has_lower_side = false;
    let mut has_raised_side = false;
    for constraint in rail_constraints
        .iter()
        .filter(|constraint| constraint_applies_to_owner(constraint, region.owner))
        .filter(|constraint| rail_constraint_touches_key(constraint, key))
    {
        match constraint.kind {
            NodeRailConstraintKind::AsphaltCurbContact
            | NodeRailConstraintKind::AsphaltBoundary { .. } => has_lower_side = true,
            NodeRailConstraintKind::CurbSidewalkContact => has_raised_side = true,
            NodeRailConstraintKind::FootprintSeam { .. }
            | NodeRailConstraintKind::FullRoadbedContour
            | NodeRailConstraintKind::BandContour { .. }
            | NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::BandBoundary { .. } => {}
        }
    }
    for constraint in &region.seam_constraints {
        if !constraint.is_material_transition || !seam_constraint_touches_key(constraint, key) {
            continue;
        }
        match constraint.seam_source {
            NodeSeamSource::AsphaltCurbContact { .. } | NodeSeamSource::AsphaltBoundary { .. } => {
                has_lower_side = true
            }
            NodeSeamSource::CurbSidewalkContact { .. } | NodeSeamSource::SidewalkOuter { .. } => {
                has_raised_side = true
            }
            NodeSeamSource::SpanHandoff { .. }
            | NodeSeamSource::FootprintBoundary { .. }
            | NodeSeamSource::TerminalEndBand { .. } => {}
        }
    }

    if has_raised_side {
        Some(SameBandBoundaryRole::RaisedSide)
    } else if has_lower_side {
        Some(SameBandBoundaryRole::LowerSide)
    } else {
        None
    }
}

fn one_matching_role_endpoint(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    start_matches: Option<bool>,
    end_matches: Option<bool>,
) -> Option<(NodeOwnershipPointKey, NodeOwnershipPointKey)> {
    match (start_matches, end_matches) {
        (Some(true), Some(false)) => Some((start, end)),
        (Some(false), Some(true)) => Some((end, start)),
        _ => None,
    }
}

fn same_band_shared_constraint_index(
    left: &NodeBooleanOwnedRegion,
    right: &NodeBooleanOwnedRegion,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> Option<usize> {
    let mut left_indices = same_band_shared_constraint_indices_for_edge(left, start, end);
    let right_indices = same_band_shared_constraint_indices_for_edge(right, start, end);
    left_indices.retain(|index| right_indices.binary_search(index).is_ok());
    left_indices.into_iter().next()
}

fn same_band_shared_constraint_indices_for_edge(
    region: &NodeBooleanOwnedRegion,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> Vec<usize> {
    let mut indices = region
        .seam_constraints
        .iter()
        .filter(|constraint| same_band_generated_contact_constraint(constraint))
        .filter(|constraint| {
            let constraint_start = road_point_key(constraint.start_xz);
            let constraint_end = road_point_key(constraint.end_xz);
            point_key_lies_on_segment(start, constraint_start, constraint_end)
                && point_key_lies_on_segment(end, constraint_start, constraint_end)
        })
        .map(|constraint| constraint.constraint_index)
        .collect::<Vec<_>>();
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn same_band_generated_contact_constraint(constraint: &NodeRegionSeamConstraint) -> bool {
    constraint.is_material_transition
        && matches!(
            constraint.seam_source,
            NodeSeamSource::FootprintBoundary { .. }
        )
        && road_point_key(constraint.start_xz) != road_point_key(constraint.end_xz)
}

fn role_matching_join_candidate(
    donor: &NodeBooleanOwnedRegion,
    receiver: &NodeBooleanOwnedRegion,
    rail_constraints: &[NodeRailConstraint],
    equal_key: NodeOwnershipPointKey,
    conflict_key: NodeOwnershipPointKey,
) -> Option<NodeOwnershipPointKey> {
    let receiver_conflict_role =
        same_band_boundary_role_at_key(receiver, rail_constraints, conflict_key)?;
    let mut candidates = Vec::new();
    for contour in &donor.shape {
        if contour.len() < 3 {
            continue;
        }
        let keys = contour
            .iter()
            .copied()
            .map(overlay_point_key)
            .collect::<Vec<_>>();
        if !edge_exists_in_keys(&keys, equal_key, conflict_key) {
            continue;
        }
        for index in 0..keys.len() {
            if keys[index] != conflict_key {
                continue;
            }
            let prev = if index == 0 {
                keys.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == keys.len() {
                0
            } else {
                index + 1
            };
            for candidate in [keys[prev], keys[next]] {
                if candidate == equal_key
                    || candidate == conflict_key
                    || triangle_double_area(equal_key, conflict_key, candidate) == 0
                {
                    continue;
                }
                if same_band_boundary_role_at_key(donor, rail_constraints, candidate)
                    == Some(receiver_conflict_role)
                {
                    candidates.push(candidate);
                }
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates.into_iter().next()
}

fn edge_exists_in_keys(
    keys: &[NodeOwnershipPointKey],
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> bool {
    keys.iter().enumerate().any(|(index, key)| {
        let next = keys[(index + 1) % keys.len()];
        (*key == start && next == end) || (*key == end && next == start)
    })
}

fn triangle_double_area(
    a: NodeOwnershipPointKey,
    b: NodeOwnershipPointKey,
    c: NodeOwnershipPointKey,
) -> i128 {
    let ab_x = i128::from(b.0 - a.0);
    let ab_z = i128::from(b.1 - a.1);
    let ac_x = i128::from(c.0 - a.0);
    let ac_z = i128::from(c.1 - a.1);
    ab_x * ac_z - ab_z * ac_x
}

fn apply_same_band_role_join_rewrite(
    regions: &mut [NodeBooleanOwnedRegion],
    rewrite: SameBandRoleJoinRewrite,
) {
    if rewrite.donor_region_index == rewrite.receiver_region_index {
        return;
    }
    if rewrite.donor_region_index < rewrite.receiver_region_index {
        let (left, right) = regions.split_at_mut(rewrite.receiver_region_index);
        apply_same_band_role_join_rewrite_to_regions(
            &mut left[rewrite.donor_region_index],
            &mut right[0],
            rewrite,
        );
    } else {
        let (left, right) = regions.split_at_mut(rewrite.donor_region_index);
        apply_same_band_role_join_rewrite_to_regions(
            &mut right[0],
            &mut left[rewrite.receiver_region_index],
            rewrite,
        );
    }
}

fn apply_same_band_role_join_rewrite_to_regions(
    donor: &mut NodeBooleanOwnedRegion,
    receiver: &mut NodeBooleanOwnedRegion,
    rewrite: SameBandRoleJoinRewrite,
) {
    remove_join_corner_from_region(
        donor,
        rewrite.equal_key,
        rewrite.conflict_key,
        rewrite.candidate_key,
    );
    insert_join_corner_on_region_edge(
        receiver,
        rewrite.conflict_key,
        rewrite.equal_key,
        rewrite.candidate_key,
    );
    if let Some(old_constraint_index) = rewrite.old_constraint_index {
        donor
            .seam_constraints
            .retain(|constraint| constraint.constraint_index != old_constraint_index);
        receiver
            .seam_constraints
            .retain(|constraint| constraint.constraint_index != old_constraint_index);
    }
    donor.seam_constraints.push(same_band_role_join_constraint(
        rewrite.new_constraint_index,
        donor.owner.owner_index(),
        rewrite.equal_key,
        rewrite.candidate_key,
    ));
    receiver
        .seam_constraints
        .push(same_band_role_join_constraint(
            rewrite.new_constraint_index,
            receiver.owner.owner_index(),
            rewrite.equal_key,
            rewrite.candidate_key,
        ));
    donor.area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&donor.shape);
    receiver.area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&receiver.shape);
    canonicalize_seam_constraints(&mut donor.seam_constraints);
    canonicalize_seam_constraints(&mut receiver.seam_constraints);
}

fn remove_join_corner_from_region(
    region: &mut NodeBooleanOwnedRegion,
    equal_key: NodeOwnershipPointKey,
    conflict_key: NodeOwnershipPointKey,
    candidate_key: NodeOwnershipPointKey,
) {
    for contour in &mut region.shape {
        if remove_middle_key_from_contour(contour, equal_key, conflict_key, candidate_key) {
            return;
        }
    }
}

fn insert_join_corner_on_region_edge(
    region: &mut NodeBooleanOwnedRegion,
    start_key: NodeOwnershipPointKey,
    end_key: NodeOwnershipPointKey,
    insert_key: NodeOwnershipPointKey,
) {
    for contour in &mut region.shape {
        if insert_key_on_contour_edge(contour, start_key, end_key, insert_key)
            || insert_key_on_contour_edge(contour, end_key, start_key, insert_key)
        {
            return;
        }
    }
}

fn remove_middle_key_from_contour(
    contour: &mut NodeOverlayContour,
    first_key: NodeOwnershipPointKey,
    middle_key: NodeOwnershipPointKey,
    third_key: NodeOwnershipPointKey,
) -> bool {
    if contour.len() < 3 {
        return false;
    }
    for index in 0..contour.len() {
        if overlay_point_key(contour[index]) != middle_key {
            continue;
        }
        let prev = if index == 0 {
            contour.len() - 1
        } else {
            index - 1
        };
        let next = if index + 1 == contour.len() {
            0
        } else {
            index + 1
        };
        let prev_key = overlay_point_key(contour[prev]);
        let next_key = overlay_point_key(contour[next]);
        if (prev_key == first_key && next_key == third_key)
            || (prev_key == third_key && next_key == first_key)
        {
            contour.remove(index);
            dedup_consecutive_overlay_points(contour);
            remove_overlay_spikes(contour);
            return true;
        }
    }
    false
}

fn insert_key_on_contour_edge(
    contour: &mut NodeOverlayContour,
    start_key: NodeOwnershipPointKey,
    end_key: NodeOwnershipPointKey,
    insert_key: NodeOwnershipPointKey,
) -> bool {
    if contour.len() < 2 {
        return false;
    }
    for index in 0..contour.len() {
        let next = if index + 1 == contour.len() {
            0
        } else {
            index + 1
        };
        if overlay_point_key(contour[index]) == start_key
            && overlay_point_key(contour[next]) == end_key
        {
            contour.insert(next, overlay_point_from_key(insert_key));
            dedup_consecutive_overlay_points(contour);
            remove_overlay_spikes(contour);
            return true;
        }
    }
    false
}

fn same_band_role_join_constraint(
    constraint_index: usize,
    owner_index: usize,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> NodeRegionSeamConstraint {
    NodeRegionSeamConstraint {
        constraint_index,
        seam_source: NodeSeamSource::FootprintBoundary { owner_index },
        constrains_shared_height: false,
        is_material_transition: false,
        start_xz: road_point_from_key(start),
        end_xz: road_point_from_key(end),
    }
}

fn seam_constraint_touches_key(
    constraint: &NodeRegionSeamConstraint,
    key: NodeOwnershipPointKey,
) -> bool {
    point_key_lies_on_segment(
        key,
        road_point_key(constraint.start_xz),
        road_point_key(constraint.end_xz),
    )
}

fn rail_constraint_touches_key(
    constraint: &NodeRailConstraint,
    key: NodeOwnershipPointKey,
) -> bool {
    constraint.points_xz.windows(2).any(|segment| {
        point_key_lies_on_segment(key, road_point_key(segment[0]), road_point_key(segment[1]))
            || point_key_projects_to_quantized_segment(key, segment[0], segment[1])
    })
}

fn point_key_projects_to_quantized_segment(
    key: NodeOwnershipPointKey,
    start: RoadVec2,
    end: RoadVec2,
) -> bool {
    let point = road_point_from_key(key);
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f64::EPSILON {
        return false;
    }

    let offset = point - start;
    let length = length_squared.sqrt();
    let projection_eps_m = 1.0 / ROAD_OVERLAY_COORDINATE_SCALE;
    let cross = offset.x * segment.y - offset.y * segment.x;
    if cross.abs() > projection_eps_m * length {
        return false;
    }
    let t = offset.dot(segment) / length_squared;
    let endpoint_eps = projection_eps_m / length;
    (-endpoint_eps..=1.0 + endpoint_eps).contains(&t)
}

fn remove_overlay_spikes(points: &mut NodeOverlayContour) {
    loop {
        if points.len() < 3 {
            return;
        }
        let mut removed = false;
        for index in 0..points.len() {
            let prev = if index == 0 {
                points.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == points.len() {
                0
            } else {
                index + 1
            };
            if overlay_point_key(points[prev]) == overlay_point_key(points[next]) {
                points.remove(index);
                dedup_consecutive_overlay_points(points);
                removed = true;
                break;
            }
        }
        if !removed {
            return;
        }
    }
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

fn shared_region_seam_source(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> NodeSeamSource {
    let owner_kind = owner.kind();
    let opposite_kind = opposite_owner.kind();
    if (is_carriageway(owner_kind) && is_curb_or_shoulder(opposite_kind))
        || (is_curb_or_shoulder(owner_kind) && is_carriageway(opposite_kind))
    {
        return NodeSeamSource::AsphaltCurbContact {
            owner_index: owner.owner_index(),
        };
    }
    if (is_curb_or_shoulder(owner_kind) && is_sidewalk(opposite_kind))
        || (is_sidewalk(owner_kind) && is_curb_or_shoulder(opposite_kind))
    {
        return NodeSeamSource::CurbSidewalkContact {
            owner_index: owner.owner_index(),
        };
    }
    if is_carriageway(owner_kind) || is_carriageway(opposite_kind) {
        return NodeSeamSource::AsphaltBoundary {
            owner_index: owner.owner_index(),
        };
    }
    if is_sidewalk(owner_kind) || is_sidewalk(opposite_kind) {
        return NodeSeamSource::SidewalkOuter {
            owner_index: owner.owner_index(),
        };
    }
    NodeSeamSource::FootprintBoundary {
        owner_index: owner.owner_index(),
    }
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
    matches!(
        constraint.kind,
        NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::AsphaltCurbContact
            | NodeRailConstraintKind::CurbSidewalkContact
            | NodeRailConstraintKind::BandBoundary { .. }
    )
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
    if px * dz - pz * dx != 0 {
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

fn sort_boolean_owned_regions(regions: &mut [NodeBooleanOwnedRegion]) {
    regions.sort_by(|a, b| {
        RoadSurfaceSystem::band_kind_sort_key(a.kind)
            .cmp(&RoadSurfaceSystem::band_kind_sort_key(b.kind))
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
    fn owned_region_rings_are_noded_before_shared_seam_constraints() {
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
        append_shared_region_seam_constraints(&mut regions, &footprint_shapes, 0);

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
    fn shared_point_constraints_preserve_endpoint_context_without_height_continuity() {
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
        let footprint_shapes = Vec::new();

        append_shared_region_seam_constraints(&mut regions, &footprint_shapes, 0);

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

    #[test]
    fn missing_shared_seam_constraints_are_generated_after_boundary_mutation() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![[2.0, 0.0], [3.0, 0.0], [3.0, 1.0], [2.0, 1.0]],
            ),
        ];
        let footprint_shapes = Vec::new();

        append_missing_shared_region_seam_constraints(&mut regions, &footprint_shapes, 17);

        let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            44,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &regions,
            &footprint_shapes,
        );
        assert!(arrangement.diagnostics().is_empty());
        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    constraint.constraint_index == 17
                        && constraint.is_material_transition
                        && !constraint.constrains_shared_height
                        && road_point_key(constraint.start_xz)
                            == road_point_key(RoadVec2::new(2.0, 0.0))
                        && road_point_key(constraint.end_xz)
                            == road_point_key(RoadVec2::new(2.0, 1.0))
                }),
                "region {:?} must receive an explicit seam on the final shared edge",
                region.owner
            );
        }
    }

    #[test]
    fn missing_shared_seam_constraint_reuses_existing_source_index() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![[2.0, 0.0], [3.0, 0.0], [3.0, 1.0], [2.0, 1.0]],
            ),
        ];
        regions[0].seam_constraints.push(NodeRegionSeamConstraint {
            constraint_index: 6,
            seam_source: NodeSeamSource::AsphaltCurbContact { owner_index: 0 },
            constrains_shared_height: true,
            is_material_transition: true,
            start_xz: RoadVec2::new(2.0, 0.0),
            end_xz: RoadVec2::new(2.0, 1.0),
        });
        let footprint_shapes = Vec::new();

        append_missing_shared_region_seam_constraints(&mut regions, &footprint_shapes, 18);

        assert!(regions[1].seam_constraints.iter().any(|constraint| {
            constraint.constraint_index == 6
                && constraint.constrains_shared_height
                && constraint.is_material_transition
                && road_point_key(constraint.start_xz) == road_point_key(RoadVec2::new(2.0, 0.0))
                && road_point_key(constraint.end_xz) == road_point_key(RoadVec2::new(2.0, 1.0))
        }));
        assert!(
            regions[1]
                .seam_constraints
                .iter()
                .all(|constraint| constraint.constraint_index != 18)
        );
    }

    #[test]
    fn canonical_contact_ownership_does_not_transfer_same_band_geometry() {
        let curb_a = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
        let curb_b = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb_a,
                vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [-1.0, 1.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb_b,
                vec![[1.0, 0.0], [0.0, 0.0], [1.0, -1.0]],
            ),
        ];
        let footprint_shapes = Vec::new();
        append_shared_region_seam_constraints(&mut regions, &footprint_shapes, 0);
        let owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            45,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &regions,
            &footprint_shapes,
        );
        let mut ownership = NodeBooleanOwnership {
            node_id: 45,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            footprint_shapes,
            asphalt_shapes: Vec::new(),
            non_road_shapes: Vec::new(),
            owned_region_arrangement,
            owned_regions: regions,
        };

        ownership.resolve_canonical_contact_ownership();

        assert!(
            ownership.owned_region_arrangement.diagnostics().is_empty(),
            "canonical contact ownership must keep explicit seam constraints"
        );
        assert!(
            ownership.owned_regions[0].shape[0]
                .iter()
                .any(|point| overlay_point_key(*point) == overlay_point_key([1.0, 0.0])),
            "same-band contact finalization must not move region geometry"
        );
    }

    fn test_owned_region(
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        contour: NodeOverlayContour,
    ) -> NodeBooleanOwnedRegion {
        NodeBooleanOwnedRegion {
            kind,
            owner,
            source_mouth_order_index: owner.owner_index(),
            source_band_index: Some(owner.owner_index()),
            shape: vec![contour],
            area_m2: 1.0,
            seam_constraints: Vec::new(),
        }
    }
}
