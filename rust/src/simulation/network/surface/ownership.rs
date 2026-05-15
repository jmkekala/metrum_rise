//! Boolean ownership solve for canonical node-arrangement contours.

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
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape,
    NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
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
    UnmaterializedRaisedStepAuthority {
        region_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start: NodeOwnedRegionArrangementKey,
        end: NodeOwnedRegionArrangementKey,
        source_constraint_indices: Vec<usize>,
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
    NonCanonicalOwnedRegionVertex {
        owner: NodeBandOwner,
        point_x_key: i64,
        point_z_key: i64,
        canonical_x_key: i64,
        canonical_z_key: i64,
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

struct NodeRailCanonicalPointSet {
    all_points: Vec<NodeOwnershipPointKey>,
    points_by_owner: BTreeMap<NodeBandOwner, Vec<NodeOwnershipPointKey>>,
    segments_by_owner: BTreeMap<NodeBandOwner, Vec<OwnedRegionEdgeKey>>,
    canonical_points_by_mm_key_by_owner:
        BTreeMap<NodeBandOwner, BTreeMap<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>>,
    height_points_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<NodeOwnershipPointKey>>,
    paths_by_owner: BTreeMap<NodeBandOwner, Vec<Vec<NodeOwnershipPointKey>>>,
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

        let footprint_contours =
            overlay_contours_for_domains(rails, |contour| contour.contributes_to_footprint());
        let mut footprint_shapes = overlay_union(&footprint_contours, "footprint_union")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut footprint_shapes);
        if footprint_shapes.is_empty() {
            return Err(NodeBooleanOwnershipError::EmptyFootprint {
                node_id: rails.node_id,
            });
        }

        let asphalt_authority_domains = asphalt_authority_domains(rails);
        let asphalt_contours = asphalt_authority_domains
            .iter()
            .map(|domain| overlay_contour_from_domain(domain))
            .collect::<Vec<_>>();
        let asphalt_raw_shapes = overlay_union(&asphalt_contours, "asphalt_union")?;
        let mut asphalt_shapes = overlay_intersect(
            &asphalt_raw_shapes,
            &footprint_shapes,
            "asphalt_clip_to_footprint",
        )?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut asphalt_shapes);

        let mut non_road_shapes =
            overlay_difference(&footprint_shapes, &asphalt_shapes, "non_road_difference")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut non_road_shapes);

        let allow_grid_bounded_constraint_overlap =
            rails.piece_kind == RoadSurfaceVisualNodePieceKind::Terminal;
        let mut owned_regions = Vec::new();
        let asphalt_owner_domains = asphalt_owner_domains(rails);
        let asphalt_result = owned_regions_from_domains(
            &asphalt_shapes,
            &asphalt_owner_domains,
            &rails.constraints,
            ResidualKind::Asphalt,
            allow_grid_bounded_constraint_overlap,
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
        let rail_canonical_points = canonical_points_for_rail_set(rails);
        clean_canonical_owned_region_shapes(
            &mut owned_regions,
            &footprint_shapes,
            &rails.constraints,
            &rail_canonical_points,
            allow_grid_bounded_constraint_overlap,
        )?;
        for region in &mut owned_regions {
            region.seam_constraints = seam_constraints_for_shape(
                &region.shape,
                region.owner,
                &rails.constraints,
                allow_grid_bounded_constraint_overlap,
            );
        }
        materialize_noded_region_seam_constraints(
            &mut owned_regions,
            &footprint_shapes,
            &rails.constraints,
            rails.piece_kind,
        );
        validate_non_road_regions_have_explicit_profile_seam_rails(
            &owned_regions,
            &rails.constraints,
        )?;
        let owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            rails.node_id,
            rails.piece_kind,
            &owned_regions,
            &footprint_shapes,
            &rails.constraints,
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
        rail_constraints: &[NodeRailConstraint],
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
                    if owned_boundary_requires_explicit_seam(edge_ref.owner, opposite_owner) {
                        let source_constraint_indices =
                            junctionn_unmaterialized_raised_step_authority_indices_for_edge(
                                piece_kind,
                                edge_key.start,
                                edge_key.end,
                                rail_constraints,
                                edge_ref.owner,
                                opposite_owner,
                            );
                        if !source_constraint_indices.is_empty()
                            && !source_constraints_materialize_raised_step_authority(
                                &source_constraints,
                                &source_constraint_indices,
                                edge_ref.owner,
                                opposite_owner,
                            )
                        {
                            diagnostics.push(
                                NodeOwnedRegionArrangementDiagnostic::UnmaterializedRaisedStepAuthority {
                                    region_index: edge_ref.region_index,
                                    owner: edge_ref.owner,
                                    opposite_owner,
                                    start,
                                    end,
                                    source_constraint_indices,
                                },
                            );
                        } else if source_constraints.is_empty() {
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
    #[cfg(test)]
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
            rails.piece_kind == RoadSurfaceVisualNodePieceKind::Terminal,
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
    allow_grid_bounded_constraint_overlap: bool,
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
            let seam_constraints = seam_constraints_for_shape(
                shape,
                group.owner,
                rail_constraints,
                allow_grid_bounded_constraint_overlap,
            );
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

fn asphalt_authority_domains(rails: &NodeRailContourSet) -> Vec<&NodeGeneratedContour> {
    domains_for_band_kind_matching(rails, RoadSurfaceBandKind::Carriageway, |contour| {
        contour.contributes_to_asphalt()
    })
}

fn asphalt_owner_domains(rails: &NodeRailContourSet) -> Vec<&NodeGeneratedContour> {
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
            contour.purpose,
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
    allow_grid_bounded_constraint_overlap: bool,
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
                if shape_edge_carries_full_seam_constraint(start, end, constraint) {
                    push_region_seam_constraint(
                        &mut seams,
                        constraint,
                        owner,
                        overlay_point_to_road(start),
                        overlay_point_to_road(end),
                    );
                }
                for (overlap_start, overlap_end) in constraint_overlaps_shape_edge(
                    start,
                    end,
                    constraint,
                    allow_grid_bounded_constraint_overlap,
                ) {
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
        owner: constraint.owner,
        opposite_owner: constraint.opposite_owner,
        constrains_shared_height: constraint_constrains_shared_height(constraint),
        is_material_transition: constraint_is_material_transition(constraint),
        start_xz,
        end_xz,
    });
}

fn push_materialized_region_seam_constraint(
    seams: &mut Vec<NodeRegionSeamConstraint>,
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    let (constraint_owner, constraint_opposite_owner) =
        materialized_constraint_owner_pair(constraint, owner, opposite_owner);
    let materialized_kind =
        materialized_constraint_kind_for_owned_edge(constraint, owner, opposite_owner);
    seams.push(NodeRegionSeamConstraint {
        constraint_index: constraint.constraint_index,
        seam_source: seam_source_from_materialized_constraint_kind(
            materialized_kind,
            owner,
            opposite_owner,
        ),
        owner: constraint_owner,
        opposite_owner: constraint_opposite_owner,
        constrains_shared_height: materialized_constraint_kind_constrains_shared_height(
            materialized_kind,
            owner,
            opposite_owner,
        ),
        is_material_transition: materialized_constraint_kind_is_material_transition(
            materialized_kind,
        ),
        start_xz,
        end_xz,
    });
}

fn push_materialized_endpoint_pair_region_seam_constraint(
    seams: &mut Vec<NodeRegionSeamConstraint>,
    constraint_index: usize,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start_xz: RoadVec2,
    end_xz: RoadVec2,
) {
    seams.push(NodeRegionSeamConstraint {
        constraint_index,
        seam_source: seam_source_from_materialized_constraint_kind(kind, owner, opposite_owner),
        owner: Some(owner),
        opposite_owner: Some(opposite_owner),
        constrains_shared_height: materialized_constraint_kind_constrains_shared_height(
            kind,
            owner,
            opposite_owner,
        ),
        is_material_transition: materialized_constraint_kind_is_material_transition(kind),
        start_xz,
        end_xz,
    });
}

fn constraint_overlaps_shape_edge(
    edge_start: NodeOverlayPoint,
    edge_end: NodeOverlayPoint,
    constraint: &NodeRailConstraint,
    allow_grid_bounded_constraint_overlap: bool,
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
        if start == end {
            continue;
        }
        if !allow_grid_bounded_constraint_overlap
            && (!point_key_collinear_with_edge(start, edge_start, edge_end)
                || !point_key_collinear_with_edge(end, edge_start, edge_end))
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
        let first = canonical_constraint_overlap_endpoint(first, start, end);
        let last = canonical_constraint_overlap_endpoint(last, start, end);
        if first != last {
            overlaps.insert((first, last));
        }
    }
    overlaps.into_iter().collect()
}

fn canonical_constraint_overlap_endpoint(
    point: NodeOwnershipPointKey,
    constraint_start: NodeOwnershipPointKey,
    constraint_end: NodeOwnershipPointKey,
) -> NodeOwnershipPointKey {
    if point == constraint_start {
        return constraint_start;
    }
    if point == constraint_end {
        return constraint_end;
    }
    point
}

struct OwnedRegionBoundaryRefs {
    edges: BTreeMap<OwnedRegionEdgeKey, Vec<OwnedRegionEdgeRef>>,
}

fn owned_region_boundary_refs(
    regions: &[NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) -> OwnedRegionBoundaryRefs {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    let mut edges = BTreeMap::<OwnedRegionEdgeKey, Vec<OwnedRegionEdgeRef>>::new();
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
                }
            }
        }
    }

    OwnedRegionBoundaryRefs { edges }
}

fn materialize_noded_region_seam_constraints(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_constraints: &[NodeRailConstraint],
    piece_kind: RoadSurfaceVisualNodePieceKind,
) {
    let boundary_refs = owned_region_boundary_refs(regions, footprint_shapes);
    let mut additions = vec![Vec::new(); regions.len()];
    for (edge_key, refs) in boundary_refs.edges {
        let refs = canonical_owned_region_edge_refs(&refs);
        for edge_ref in &refs {
            let Some(opposite_owner) = opposite_owner_for_ref(&refs, *edge_ref) else {
                continue;
            };
            let Some(region) = regions.get(edge_ref.region_index) else {
                continue;
            };
            let matching_constraints = rail_constraints
                .iter()
                .filter(|constraint| {
                    rail_constraint_can_materialize_for_owned_edge(
                        constraint,
                        edge_ref.owner,
                        opposite_owner,
                    )
                })
                .filter(|constraint| {
                    owned_edge_lies_on_rail_constraint(
                        edge_key.start,
                        edge_key.end,
                        constraint,
                        edge_ref.owner,
                        opposite_owner,
                        piece_kind,
                    )
                })
                .collect::<Vec<_>>();
            if matching_constraints.is_empty() {
                let endpoint_pair_sources =
                    materialized_endpoint_pair_constraint_indices_for_owned_edge(
                        edge_key.start,
                        edge_key.end,
                        rail_constraints,
                        edge_ref.owner,
                        opposite_owner,
                    );
                if !endpoint_pair_sources.is_empty() {
                    let Some(materialized_kind) =
                        material_contact_kind_for_owned_edge(edge_ref.owner, opposite_owner)
                    else {
                        continue;
                    };
                    let start_xz = road_point_from_key(edge_key.start);
                    let end_xz = road_point_from_key(edge_key.end);
                    for constraint_index in endpoint_pair_sources {
                        push_materialized_endpoint_pair_region_seam_constraint(
                            &mut additions[edge_ref.region_index],
                            constraint_index,
                            materialized_kind,
                            region.owner,
                            opposite_owner,
                            start_xz,
                            end_xz,
                        );
                    }
                    continue;
                }
                if let Some((constraint_index, materialized_kind)) =
                    materialized_source_constraint_for_owned_step_edge(
                        edge_key.start,
                        edge_key.end,
                        rail_constraints,
                        edge_ref.owner,
                        opposite_owner,
                        piece_kind,
                    )
                {
                    push_materialized_endpoint_pair_region_seam_constraint(
                        &mut additions[edge_ref.region_index],
                        constraint_index,
                        materialized_kind,
                        region.owner,
                        opposite_owner,
                        road_point_from_key(edge_key.start),
                        road_point_from_key(edge_key.end),
                    );
                }
            }
            let has_exact_owner_pair_source = matching_constraints.iter().any(|constraint| {
                rail_constraint_owner_pair_matches_edge(constraint, edge_ref.owner, opposite_owner)
            });
            for constraint in matching_constraints {
                if has_exact_owner_pair_source
                    && !rail_constraint_owner_pair_matches_edge(
                        constraint,
                        edge_ref.owner,
                        opposite_owner,
                    )
                {
                    continue;
                }
                push_materialized_region_seam_constraint(
                    &mut additions[edge_ref.region_index],
                    constraint,
                    region.owner,
                    opposite_owner,
                    road_point_from_key(edge_key.start),
                    road_point_from_key(edge_key.end),
                );
            }
        }
    }
    for (region, mut seam_additions) in regions.iter_mut().zip(additions) {
        region.seam_constraints.append(&mut seam_additions);
        canonicalize_seam_constraints(&mut region.seam_constraints);
    }
}

fn canonicalize_owned_region_rings(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    for region in regions.iter_mut() {
        for contour in &mut region.shape {
            *contour = noded_owned_region_contour(contour, &global_points);
        }
    }
}

fn clean_canonical_owned_region_shapes(
    regions: &mut Vec<NodeBooleanOwnedRegion>,
    footprint_shapes: &NodeOverlayShapes,
    rail_constraints: &[NodeRailConstraint],
    rail_canonical_points: &NodeRailCanonicalPointSet,
    allow_grid_bounded_constraint_overlap: bool,
) -> Result<(), NodeBooleanOwnershipError> {
    let mut cleaned_regions = Vec::with_capacity(regions.len());
    for region in regions.drain(..) {
        let mut shapes = overlay_union(&region.shape, "owned_region_ring_clean")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
        for shape in shapes.into_iter().flat_map(|shape| {
            split_self_touching_owned_shape(shape, allow_grid_bounded_constraint_overlap)
        }) {
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
                    allow_grid_bounded_constraint_overlap,
                ),
            });
        }
    }
    canonicalize_owned_region_rings_with_rail_point_set(
        &mut cleaned_regions,
        rail_canonical_points,
    );
    clean_owned_region_shapes_once(
        &mut cleaned_regions,
        rail_constraints,
        allow_grid_bounded_constraint_overlap,
    )?;
    canonicalize_owned_region_rings_with_rail_point_set(
        &mut cleaned_regions,
        rail_canonical_points,
    );
    canonicalize_final_owned_region_boundary_edges(
        &mut cleaned_regions,
        footprint_shapes,
        rail_canonical_points,
    );
    clean_owned_region_shapes_once(
        &mut cleaned_regions,
        rail_constraints,
        allow_grid_bounded_constraint_overlap,
    )?;
    canonicalize_final_owned_region_boundary_edges(
        &mut cleaned_regions,
        footprint_shapes,
        rail_canonical_points,
    );
    split_final_canonical_owned_region_self_touches(&mut cleaned_regions, rail_constraints, false);
    canonicalize_owned_region_rings_with_rail_point_set(
        &mut cleaned_regions,
        rail_canonical_points,
    );
    for region in &mut cleaned_regions {
        region.seam_constraints = seam_constraints_for_shape(
            &region.shape,
            region.owner,
            rail_constraints,
            allow_grid_bounded_constraint_overlap,
        );
    }
    validate_owned_region_vertices_against_source_authority(
        &cleaned_regions,
        rail_canonical_points,
    )?;
    *regions = cleaned_regions;
    Ok(())
}

fn canonicalize_final_owned_region_boundary_edges(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_canonical_points: &NodeRailCanonicalPointSet,
) {
    canonicalize_owned_region_rings_with_rail_point_set(regions, rail_canonical_points);
    node_owned_region_rings_to_global_points(regions, footprint_shapes);
    canonicalize_owned_region_rings_with_rail_point_set(regions, rail_canonical_points);
}

fn node_owned_region_rings_to_global_points(
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

fn split_final_canonical_owned_region_self_touches(
    regions: &mut Vec<NodeBooleanOwnedRegion>,
    rail_constraints: &[NodeRailConstraint],
    allow_grid_bounded_constraint_overlap: bool,
) {
    let mut split_regions = Vec::with_capacity(regions.len());
    for region in regions.drain(..) {
        let NodeBooleanOwnedRegion {
            kind,
            owner,
            claim_priority,
            source_mouth_order_index,
            source_band_index,
            shape,
            ..
        } = region;
        for shape in split_self_touching_owned_shape(shape, allow_grid_bounded_constraint_overlap) {
            let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&shape);
            if owned_shape_is_discardable_numeric_dust(&shape, area_m2, owner, rail_constraints) {
                continue;
            }
            split_regions.push(NodeBooleanOwnedRegion {
                kind,
                owner,
                claim_priority,
                source_mouth_order_index,
                source_band_index,
                shape,
                area_m2,
                seam_constraints: Vec::new(),
            });
        }
    }
    *regions = split_regions;
}

fn clean_owned_region_shapes_once(
    regions: &mut Vec<NodeBooleanOwnedRegion>,
    rail_constraints: &[NodeRailConstraint],
    allow_grid_bounded_constraint_overlap: bool,
) -> Result<(), NodeBooleanOwnershipError> {
    let mut cleaned_regions = Vec::with_capacity(regions.len());
    for region in regions.drain(..) {
        let mut shapes = overlay_union(&region.shape, "owned_region_constraint_noded_clean")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
        for shape in shapes.into_iter().flat_map(|shape| {
            split_self_touching_owned_shape(shape, allow_grid_bounded_constraint_overlap)
        }) {
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
                    allow_grid_bounded_constraint_overlap,
                ),
            });
        }
    }
    *regions = cleaned_regions;
    Ok(())
}

fn canonical_points_for_rail_set(rails: &NodeRailContourSet) -> NodeRailCanonicalPointSet {
    let mut all_points = rails
        .constraints
        .iter()
        .flat_map(|constraint| constraint.points_xz.iter().copied())
        .chain(
            rails
                .contours
                .iter()
                .flat_map(|contour| contour.points_xz.iter().copied()),
        )
        .map(road_point_key)
        .collect::<Vec<_>>();
    let mut points_by_owner = BTreeMap::<NodeBandOwner, Vec<NodeOwnershipPointKey>>::new();
    let mut segments_by_owner = BTreeMap::<NodeBandOwner, Vec<OwnedRegionEdgeKey>>::new();
    let mut height_points_by_source =
        BTreeMap::<(RoadSurfaceBandKind, usize, usize), Vec<NodeOwnershipPointKey>>::new();
    let mut paths_by_owner = BTreeMap::<NodeBandOwner, Vec<Vec<NodeOwnershipPointKey>>>::new();
    for (source, points) in &rails.height_carrier_points_by_source {
        let points = points
            .iter()
            .copied()
            .map(road_point_key)
            .collect::<Vec<_>>();
        all_points.extend(points.iter().copied());
        height_points_by_source
            .entry(*source)
            .or_default()
            .extend(points);
    }
    for constraint in &rails.constraints {
        let (Some(owner), Some(source_band_index)) =
            (constraint.owner, constraint.source_band_index)
        else {
            continue;
        };
        height_points_by_source
            .entry((
                owner.kind(),
                constraint.source_mouth_order_index,
                source_band_index,
            ))
            .or_default()
            .extend(constraint.points_xz.iter().copied().map(road_point_key));
    }
    for contour in &rails.contours {
        let path = contour
            .points_xz
            .iter()
            .copied()
            .map(road_point_key)
            .collect::<Vec<_>>();
        if let (NodeGeneratedContourKind::Band { kind }, Some(source_band_index)) =
            (contour.kind, contour.source_band_index)
        {
            height_points_by_source
                .entry((kind, contour.source_mouth_order_index, source_band_index))
                .or_default()
                .extend(path.iter().copied());
        }
        let Some(owner) = contour.owner else {
            continue;
        };
        points_by_owner
            .entry(owner)
            .or_default()
            .extend(path.iter().copied());
        if contour.height_points_world.is_some() {
            points_by_owner
                .entry(owner)
                .or_default()
                .extend(path.iter().copied());
        }
        insert_closed_source_segments(&mut segments_by_owner, owner, &path);
        paths_by_owner.entry(owner).or_default().push(path);
    }
    for constraint in &rails.constraints {
        let path = constraint
            .points_xz
            .iter()
            .copied()
            .map(road_point_key)
            .collect::<Vec<_>>();
        for owner in constraint_authority_owners(constraint) {
            points_by_owner
                .entry(owner)
                .or_default()
                .extend(path.iter().copied());
            insert_open_source_segments(&mut segments_by_owner, owner, &path);
        }
    }
    for (owner, points) in &mut points_by_owner {
        points.sort_unstable();
        points.dedup();
        let _ = owner;
    }
    for points in height_points_by_source.values_mut() {
        points.sort_unstable();
        points.dedup();
    }
    for segments in segments_by_owner.values_mut() {
        segments.sort_unstable();
        segments.dedup();
    }
    all_points.sort_unstable();
    all_points.dedup();
    let canonical_points_by_mm_key_by_owner = canonical_points_by_mm_key_by_owner(&points_by_owner);
    NodeRailCanonicalPointSet {
        all_points,
        points_by_owner,
        segments_by_owner,
        canonical_points_by_mm_key_by_owner,
        height_points_by_source,
        paths_by_owner,
    }
}

fn canonicalize_owned_region_rings_with_rail_point_set(
    regions: &mut [NodeBooleanOwnedRegion],
    rail_points: &NodeRailCanonicalPointSet,
) {
    if rail_points.all_points.is_empty() {
        return;
    }

    for region in regions {
        let owner_points = rail_points
            .points_by_owner
            .get(&region.owner)
            .map(Vec::as_slice)
            .unwrap_or(&rail_points.all_points);
        let source_height_points = region.source_band_index.and_then(|source_band_index| {
            rail_points.height_points_by_source.get(&(
                region.kind,
                region.source_mouth_order_index,
                source_band_index,
            ))
        });
        let mut preserved_points = source_height_points.cloned().unwrap_or_default();
        preserved_points.sort_unstable();
        preserved_points.dedup();
        let authority_points = source_height_points
            .map(Vec::as_slice)
            .unwrap_or(owner_points);
        let mut source_points = preserved_points.clone();
        source_points.extend(authority_points.iter().copied().map(|point| {
            rail_points
                .canonical_point_for_owner(region.owner, point)
                .unwrap_or(point)
        }));
        source_points.extend(rail_points.all_points.iter().copied().map(|point| {
            rail_points
                .canonical_point_for_owner(region.owner, point)
                .unwrap_or(point)
        }));
        source_points.sort_unstable();
        source_points.dedup();
        let owner_paths = if region.claim_priority == NodeGeneratedContourClaimPriority::JoinOrCap {
            rail_points
                .paths_by_owner
                .get(&region.owner)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        } else {
            &[]
        };

        for contour in &mut region.shape {
            canonicalize_owned_region_contour_to_owner_source_points(
                contour,
                region.owner,
                &preserved_points,
                rail_points,
            );
            *contour =
                noded_owned_region_contour_with_rail_paths(contour, &source_points, owner_paths);
        }
    }
}

fn canonicalize_owned_region_contour_to_owner_source_points(
    contour: &mut NodeOverlayContour,
    owner: NodeBandOwner,
    source_points: &[NodeOwnershipPointKey],
    rail_points: &NodeRailCanonicalPointSet,
) {
    for point in contour.iter_mut() {
        let key = overlay_point_key(*point);
        if source_points.binary_search(&key).is_ok() {
            continue;
        }
        let Some(canonical) = rail_points.canonical_point_for_owner(owner, key) else {
            continue;
        };
        if canonical == key {
            continue;
        }
        *point = overlay_point_from_key(canonical);
    }
    dedup_consecutive_overlay_points(contour);
    if contour.len() >= 2
        && overlay_point_key(contour[0])
            == overlay_point_key(*contour.last().expect("contour has last"))
    {
        contour.pop();
    }
}

fn insert_open_source_segments(
    segments_by_owner: &mut BTreeMap<NodeBandOwner, Vec<OwnedRegionEdgeKey>>,
    owner: NodeBandOwner,
    path: &[NodeOwnershipPointKey],
) {
    for segment in path.windows(2) {
        if segment[0] == segment[1] {
            continue;
        }
        segments_by_owner
            .entry(owner)
            .or_default()
            .push(OwnedRegionEdgeKey::new(segment[0], segment[1]));
    }
}

fn insert_closed_source_segments(
    segments_by_owner: &mut BTreeMap<NodeBandOwner, Vec<OwnedRegionEdgeKey>>,
    owner: NodeBandOwner,
    path: &[NodeOwnershipPointKey],
) {
    insert_open_source_segments(segments_by_owner, owner, path);
    let (Some(first), Some(last)) = (path.first().copied(), path.last().copied()) else {
        return;
    };
    if first == last {
        return;
    }
    segments_by_owner
        .entry(owner)
        .or_default()
        .push(OwnedRegionEdgeKey::new(first, last));
}

fn canonical_points_by_mm_key_by_owner(
    points_by_owner: &BTreeMap<NodeBandOwner, Vec<NodeOwnershipPointKey>>,
) -> BTreeMap<NodeBandOwner, BTreeMap<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>> {
    let mut by_owner = BTreeMap::new();
    for (owner, points) in points_by_owner {
        let mut by_mm_key =
            BTreeMap::<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>::new();
        for point in points {
            by_mm_key
                .entry(ownership_mm_key(*point))
                .or_default()
                .insert(*point);
        }
        by_owner.insert(*owner, by_mm_key);
    }
    by_owner
}

fn validate_owned_region_vertices_against_source_authority(
    regions: &[NodeBooleanOwnedRegion],
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    for region in regions {
        let source_height_points = region
            .source_band_index
            .and_then(|source_band_index| {
                rail_points.height_points_by_source.get(&(
                    region.kind,
                    region.source_mouth_order_index,
                    source_band_index,
                ))
            })
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for contour in &region.shape {
            for point in contour.iter().copied().map(overlay_point_key) {
                if source_height_points.binary_search(&point).is_ok() {
                    continue;
                }
                if rail_points.owner_source_authorizes_point(region.owner, point) {
                    continue;
                }
                let Some(canonical) =
                    rail_points.conflicting_canonical_point_for_owner(region.owner, point)
                else {
                    continue;
                };
                return Err(NodeBooleanOwnershipError::NonCanonicalOwnedRegionVertex {
                    owner: region.owner,
                    point_x_key: point.0,
                    point_z_key: point.1,
                    canonical_x_key: canonical.0,
                    canonical_z_key: canonical.1,
                });
            }
        }
    }
    Ok(())
}

impl NodeRailCanonicalPointSet {
    fn owner_source_authorizes_point(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> bool {
        if self
            .canonical_point_for_owner(owner, point)
            .is_some_and(|canonical| canonical != point)
        {
            return false;
        }
        self.points_by_owner
            .get(&owner)
            .is_some_and(|points| points.binary_search(&point).is_ok())
            || self.segments_by_owner.get(&owner).is_some_and(|segments| {
                segments
                    .iter()
                    .any(|segment| point_key_lies_on_segment(point, segment.start, segment.end))
            })
    }

    fn conflicting_canonical_point_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Option<NodeOwnershipPointKey> {
        self.canonical_point_for_owner(owner, point)
            .filter(|canonical| *canonical != point)
    }

    fn canonical_point_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Option<NodeOwnershipPointKey> {
        let candidates = self.canonical_candidates_for_owner(owner, point)?;
        candidates.iter().copied().next()
    }

    fn canonical_candidates_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Option<&BTreeSet<NodeOwnershipPointKey>> {
        self.canonical_points_by_mm_key_by_owner
            .get(&owner)?
            .get(&ownership_mm_key(point))
    }
}

fn constraint_authority_owners(constraint: &NodeRailConstraint) -> Vec<NodeBandOwner> {
    let mut owners = [constraint.owner, constraint.opposite_owner]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners.dedup();
    owners
}

fn split_self_touching_owned_shape(
    shape: NodeOverlayShape,
    clean_numeric_spikes: bool,
) -> Vec<NodeOverlayShape> {
    let shape = cleaned_owned_shape(shape, clean_numeric_spikes);
    if shape.is_empty() {
        return Vec::new();
    }
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
            if let Some(cycle) = cleaned_self_touch_split_contour(cycle, clean_numeric_spikes) {
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

fn cleaned_owned_shape(shape: NodeOverlayShape, clean_numeric_spikes: bool) -> NodeOverlayShape {
    shape
        .into_iter()
        .filter_map(|contour| cleaned_owned_contour(contour, clean_numeric_spikes))
        .collect()
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

fn cleaned_self_touch_split_contour(
    contour: NodeOverlayContour,
    clean_numeric_spikes: bool,
) -> Option<NodeOverlayContour> {
    cleaned_owned_contour(contour, clean_numeric_spikes)
}

fn cleaned_owned_contour(
    mut contour: NodeOverlayContour,
    clean_numeric_spikes: bool,
) -> Option<NodeOverlayContour> {
    dedup_consecutive_overlay_points(&mut contour);
    if clean_numeric_spikes {
        remove_numeric_spike_vertices(&mut contour);
    }
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

fn remove_numeric_spike_vertices(contour: &mut NodeOverlayContour) {
    loop {
        if contour.len() < 4 {
            return;
        }
        let mut removed = false;
        for index in 0..contour.len() {
            let previous = if index == 0 {
                contour.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == contour.len() {
                0
            } else {
                index + 1
            };
            let previous_key = overlay_point_key(contour[previous]);
            let current_key = overlay_point_key(contour[index]);
            let next_key = overlay_point_key(contour[next]);
            if previous_key == next_key
                || ownership_triangle_area_m2(previous_key, current_key, next_key)
                    <= f64::from(NODE_OVERLAY_MIN_AREA_M2)
            {
                contour.remove(index);
                removed = true;
                break;
            }
        }
        if !removed {
            return;
        }
    }
}

fn ownership_triangle_area_m2(
    a: NodeOwnershipPointKey,
    b: NodeOwnershipPointKey,
    c: NodeOwnershipPointKey,
) -> f64 {
    let ab_x = i128::from(b.0 - a.0);
    let ab_z = i128::from(b.1 - a.1);
    let ac_x = i128::from(c.0 - a.0);
    let ac_z = i128::from(c.1 - a.1);
    let double_area = (ab_x * ac_z - ab_z * ac_x).unsigned_abs() as f64;
    double_area * 0.5 / ROAD_OVERLAY_COORDINATE_SCALE.powi(2)
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
    noded_owned_region_contour_with_edge_points(contour, |start, end| {
        noded_owned_region_edge_points(start, end, global_points)
    })
}

fn noded_owned_region_contour_with_rail_paths(
    contour: &NodeOverlayContour,
    global_points: &[NodeOwnershipPointKey],
    rail_paths: &[Vec<NodeOwnershipPointKey>],
) -> NodeOverlayContour {
    noded_owned_region_contour_with_edge_points(contour, |start, end| {
        noded_owned_region_edge_points_with_rail_paths(start, end, global_points, rail_paths)
    })
}

fn noded_owned_region_contour_with_edge_points(
    contour: &NodeOverlayContour,
    mut edge_points: impl FnMut(
        NodeOwnershipPointKey,
        NodeOwnershipPointKey,
    ) -> Vec<NodeOwnershipPointKey>,
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
        let points = edge_points(start, end);
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

fn noded_owned_region_edge_points_with_rail_paths(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    global_points: &[NodeOwnershipPointKey],
    rail_paths: &[Vec<NodeOwnershipPointKey>],
) -> Vec<NodeOwnershipPointKey> {
    rail_path_points_between(start, end, rail_paths)
        .unwrap_or_else(|| noded_owned_region_edge_points(start, end, global_points))
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

fn rail_constraint_owner_pair_matches_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(
        (constraint.owner, constraint.opposite_owner),
        (Some(left), Some(right))
            if (left == owner && right == opposite_owner)
                || (left == opposite_owner && right == owner)
    )
}

fn rail_constraint_can_materialize_for_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        || rail_constraint_owner_kinds_authorize_owned_edge(constraint, owner, opposite_owner)
        || rail_constraint_band_contour_authorizes_owned_edge(constraint, owner, opposite_owner)
        || rail_constraint_role_matches_owned_edge(constraint, owner, opposite_owner)
}

fn rail_constraint_band_contour_authorizes_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    let NodeRailConstraintKind::BandContour { kind } = constraint.kind else {
        return false;
    };
    if material_contact_kind_for_owned_edge(owner, opposite_owner).is_none() {
        return false;
    }
    if kind != owner.kind() && kind != opposite_owner.kind() {
        return false;
    }
    constraint.owner.is_none_or(|constraint_owner| {
        constraint_owner == owner || constraint_owner == opposite_owner
    })
}

fn rail_constraint_owner_kinds_authorize_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    if !constraint_is_material_transition(constraint) {
        return false;
    }
    let Some((constraint_owner, constraint_opposite_owner)) =
        constraint.owner.zip(constraint.opposite_owner)
    else {
        return false;
    };
    if ![constraint_owner, constraint_opposite_owner]
        .into_iter()
        .any(|constraint_owner| constraint_owner == owner || constraint_owner == opposite_owner)
    {
        return false;
    }
    owner_sets_match_by_kind(
        owner,
        opposite_owner,
        constraint_owner,
        constraint_opposite_owner,
    )
}

fn owner_sets_match_by_kind(
    left_owner: NodeBandOwner,
    left_opposite_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    right_opposite_owner: NodeBandOwner,
) -> bool {
    (left_owner.kind() == right_owner.kind()
        && left_opposite_owner.kind() == right_opposite_owner.kind())
        || (left_owner.kind() == right_opposite_owner.kind()
            && left_opposite_owner.kind() == right_owner.kind())
}

fn rail_constraint_role_matches_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    if constraint.owner.zip(constraint.opposite_owner).is_some() {
        return false;
    }
    let Some(role_owner) = constraint.owner.or(constraint.opposite_owner) else {
        return false;
    };
    if role_owner != owner && role_owner != opposite_owner {
        return false;
    }
    match constraint.kind {
        NodeRailConstraintKind::RaisedStepContact => {
            owners_form_raised_step_contact(owner, opposite_owner)
        }
        _ => false,
    }
}

fn materialized_constraint_owner_pair(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> (Option<NodeBandOwner>, Option<NodeBandOwner>) {
    if rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner) {
        (constraint.owner, constraint.opposite_owner)
    } else {
        (Some(owner), Some(opposite_owner))
    }
}

fn materialized_constraint_kind_for_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> NodeRailConstraintKind {
    if rail_constraint_band_contour_authorizes_owned_edge(constraint, owner, opposite_owner) {
        return material_contact_kind_for_owned_edge(owner, opposite_owner)
            .expect("band contour authorization requires a material contact kind");
    }
    constraint.kind
}

fn material_contact_kind_for_owned_edge(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<NodeRailConstraintKind> {
    owners_form_raised_step_contact(owner, opposite_owner)
        .then_some(NodeRailConstraintKind::RaisedStepContact)
}

fn seam_source_from_materialized_constraint_kind(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    _opposite_owner: NodeBandOwner,
) -> NodeSeamSource {
    match kind {
        NodeRailConstraintKind::RaisedStepContact => NodeSeamSource::RaisedStepContact {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::AsphaltBoundary { .. } => NodeSeamSource::AsphaltBoundary {
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

fn materialized_constraint_kind_constrains_shared_height(
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    match kind {
        NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::AsphaltBoundary { .. } => true,
        NodeRailConstraintKind::RaisedStepContact => {
            raised_step_contact_constrains_shared_height(owner, opposite_owner)
        }
        _ => false,
    }
}

fn materialized_constraint_kind_is_material_transition(kind: NodeRailConstraintKind) -> bool {
    matches!(
        kind,
        NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::RaisedStepContact
            | NodeRailConstraintKind::BandBoundary { .. }
    )
}

fn owned_edge_lies_on_rail_constraint(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> bool {
    if start == end || constraint.points_xz.len() < 2 {
        return false;
    }
    if edge_lies_on_single_constraint_segment(start, end, constraint) {
        return true;
    }
    if matches!(constraint.kind, NodeRailConstraintKind::BandContour { .. }) {
        return false;
    }
    if materialized_edge_requires_exact_constraint_span(constraint, owner, opposite_owner)
        && (!rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
            || (constraint.source_boundary_index.is_some()
                && piece_kind != RoadSurfaceVisualNodePieceKind::Terminal))
    {
        return piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN
            && rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
            && edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint);
    }
    matches!(
        piece_kind,
        RoadSurfaceVisualNodePieceKind::Bend | RoadSurfaceVisualNodePieceKind::Terminal
    ) && edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint)
}

fn materialized_endpoint_pair_constraint_indices_for_owned_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    let Some(kind) = material_contact_kind_for_owned_edge(owner, opposite_owner) else {
        return Vec::new();
    };
    let Some(start_constraint_index) = exact_owner_pair_point_contact_constraint_index_at_key(
        start,
        rail_constraints,
        owner,
        opposite_owner,
        kind,
    ) else {
        return Vec::new();
    };
    let Some(end_constraint_index) = exact_owner_pair_point_contact_constraint_index_at_key(
        end,
        rail_constraints,
        owner,
        opposite_owner,
        kind,
    ) else {
        return Vec::new();
    };
    canonical_source_indices([start_constraint_index, end_constraint_index])
}

fn materialized_source_constraint_for_owned_step_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Option<(usize, NodeRailConstraintKind)> {
    let kind = material_contact_kind_for_owned_edge(owner, opposite_owner)?;
    rail_constraints
        .iter()
        .filter(|constraint| {
            constraint_applies_to_owner(constraint, owner)
                || constraint_applies_to_owner(constraint, opposite_owner)
        })
        .filter(|constraint| {
            owned_edge_lies_on_rail_constraint(
                start,
                end,
                constraint,
                owner,
                opposite_owner,
                piece_kind,
            )
        })
        .min_by_key(|constraint| {
            (
                constraint_is_material_transition(constraint),
                constraint.constraint_index,
            )
        })
        .map(|constraint| (constraint.constraint_index, kind))
}

fn exact_owner_pair_point_contact_constraint_index_at_key(
    key: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    kind: NodeRailConstraintKind,
) -> Option<usize> {
    rail_constraints
        .iter()
        .filter(|constraint| constraint.kind == kind)
        .filter(|constraint| {
            rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        })
        .filter(|constraint| constraint_is_point_contact(constraint))
        .filter(|constraint| constraint.points_xz.first().copied().map(road_point_key) == Some(key))
        .map(|constraint| constraint.constraint_index)
        .min()
}

fn materialized_edge_requires_exact_constraint_span(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(constraint.kind, NodeRailConstraintKind::RaisedStepContact)
        && raised_step_contact_requires_exact_constraint_span(owner, opposite_owner)
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

fn owned_boundary_requires_explicit_seam(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    owner.kind() != opposite_owner.kind()
}

fn junctionn_unmaterialized_raised_step_authority_indices_for_edge(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_constraints: &[NodeRailConstraint],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Vec<usize> {
    if piece_kind != RoadSurfaceVisualNodePieceKind::JunctionN {
        return Vec::new();
    }
    let mut source_constraint_indices = rail_constraints
        .iter()
        .filter(|constraint| constraint.kind == NodeRailConstraintKind::RaisedStepContact)
        .filter(|constraint| !constraint_is_point_contact(constraint))
        .filter(|constraint| {
            rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        })
        .filter(|constraint| {
            edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint)
        })
        .map(|constraint| constraint.constraint_index)
        .collect::<Vec<_>>();
    source_constraint_indices.sort_unstable();
    source_constraint_indices.dedup();
    source_constraint_indices
}

fn source_constraints_materialize_raised_step_authority(
    source_constraints: &[&NodeRegionSeamConstraint],
    source_constraint_indices: &[usize],
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    source_constraints.iter().any(|constraint| {
        source_constraint_indices.contains(&constraint.constraint_index)
            && constraint.is_material_transition
            && seam_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
            && matches!(
                constraint.seam_source,
                NodeSeamSource::RaisedStepContact { .. }
            )
    })
}

fn seam_constraint_owner_pair_matches_edge(
    constraint: &NodeRegionSeamConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(
        (constraint.owner, constraint.opposite_owner),
        (Some(left), Some(right))
            if (left == owner && right == opposite_owner)
                || (left == opposite_owner && right == owner)
    )
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
        .filter(|point| point_key_lies_exactly_on_segment(*point, start, end))
        .collect::<Vec<_>>();
    split_points.sort_by_key(|point| segment_parameter_key(start, end, *point));
    split_points.dedup();

    let mut points = Vec::with_capacity(split_points.len() + 2);
    points.push(start);
    points.extend(split_points);
    points.push(end);
    points
}

fn rail_path_points_between(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_paths: &[Vec<NodeOwnershipPointKey>],
) -> Option<Vec<NodeOwnershipPointKey>> {
    if start == end {
        return None;
    }
    let mut best = None;
    for points in rail_paths {
        for start_index in points
            .iter()
            .enumerate()
            .filter_map(|(index, point)| (*point == start).then_some(index))
        {
            for end_index in start_index + 1..points.len() {
                if points[end_index] != end {
                    continue;
                }
                let mut candidate = points[start_index..=end_index].to_vec();
                dedup_consecutive_ownership_keys(&mut candidate);
                if candidate.len() == 3
                    && best
                        .as_ref()
                        .is_none_or(|best: &Vec<NodeOwnershipPointKey>| {
                            candidate.len() > best.len()
                        })
                {
                    best = Some(candidate);
                }
            }
        }
        for end_index in points
            .iter()
            .enumerate()
            .filter_map(|(index, point)| (*point == end).then_some(index))
        {
            for start_index in end_index + 1..points.len() {
                if points[start_index] != start {
                    continue;
                }
                let mut candidate = points[end_index..=start_index].to_vec();
                candidate.reverse();
                dedup_consecutive_ownership_keys(&mut candidate);
                if candidate.len() == 3
                    && best
                        .as_ref()
                        .is_none_or(|best: &Vec<NodeOwnershipPointKey>| {
                            candidate.len() > best.len()
                        })
                {
                    best = Some(candidate);
                }
            }
        }
    }
    best
}

fn dedup_consecutive_ownership_keys(points: &mut Vec<NodeOwnershipPointKey>) {
    points.dedup();
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
) -> (
    usize,
    NodeOwnershipPointKey,
    NodeOwnershipPointKey,
    Option<NodeBandOwner>,
    Option<NodeBandOwner>,
) {
    (
        constraint.constraint_index,
        road_point_key(constraint.start_xz),
        road_point_key(constraint.end_xz),
        constraint.owner,
        constraint.opposite_owner,
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
    match constraint.kind {
        NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::AsphaltBoundary { .. } => true,
        NodeRailConstraintKind::RaisedStepContact => {
            let Some((owner, opposite_owner)) = constraint.owner.zip(constraint.opposite_owner)
            else {
                return false;
            };
            raised_step_contact_constrains_shared_height(owner, opposite_owner)
        }
        _ => false,
    }
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
            | NodeRailConstraintKind::RaisedStepContact
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
            owner.kind() == RoadSurfaceBandKind::Carriageway || adjacent_kind == owner.kind()
        }
        NodeRailConstraintKind::RaisedStepContact => false,
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

fn shape_edge_carries_full_seam_constraint(
    edge_start: NodeOverlayPoint,
    edge_end: NodeOverlayPoint,
    constraint: &NodeRailConstraint,
) -> bool {
    if !shape_edge_requires_exact_constraint_span(constraint) {
        return edge_lies_on_constraint(edge_start, edge_end, constraint);
    }
    edge_lies_on_single_constraint_segment(
        overlay_point_key(edge_start),
        overlay_point_key(edge_end),
        constraint,
    )
}

fn shape_edge_requires_exact_constraint_span(constraint: &NodeRailConstraint) -> bool {
    matches!(constraint.kind, NodeRailConstraintKind::RaisedStepContact)
}

fn edge_lies_on_single_constraint_segment(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    constraint.points_xz.windows(2).any(|segment| {
        let start = road_point_key(segment[0]);
        let end = road_point_key(segment[1]);
        point_key_lies_on_segment(edge_start, start, end)
            && point_key_lies_on_segment(edge_end, start, end)
    })
}

fn edge_lies_on_constraint_polyline(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    edge_lies_on_constraint_polyline_with_collinearity(
        edge_start,
        edge_end,
        constraint,
        point_key_collinear_with_edge,
    )
}

fn edge_lies_on_constraint_polyline_on_overlay_grid(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    edge_lies_on_constraint_polyline_with_collinearity(
        edge_start,
        edge_end,
        constraint,
        point_key_collinear_with_edge_on_overlay_grid,
    )
}

fn edge_lies_on_constraint_polyline_with_collinearity(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
    point_collinear_with_edge: fn(
        NodeOwnershipPointKey,
        NodeOwnershipPointKey,
        NodeOwnershipPointKey,
    ) -> bool,
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
            || !point_collinear_with_edge(start, edge_start, edge_end)
            || !point_collinear_with_edge(end, edge_start, edge_end)
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

fn ownership_mm_key(point: NodeOwnershipPointKey) -> NodeOwnershipPointKey {
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
            | NodeRailConstraintKind::RaisedStepContact
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

fn point_key_collinear_with_edge_on_overlay_grid(
    point: NodeOwnershipPointKey,
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
) -> bool {
    let dx = i128::from(edge_end.0 - edge_start.0);
    let dz = i128::from(edge_end.1 - edge_start.1);
    let px = i128::from(point.0 - edge_start.0);
    let pz = i128::from(point.1 - edge_start.1);
    let cross = px * dz - pz * dx;
    cross == 0 || cross.abs() <= overlay_grid_collinearity_error_bound(dx, dz)
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
        point.0 >= start.0.min(end.0) && point.0 <= start.0.max(end.0)
    };
    let inside_z = if start.1 == end.1 {
        point.1 == start.1
    } else {
        point.1 >= start.1.min(end.1) && point.1 <= start.1.max(end.1)
    };
    inside_x && inside_z
}

fn point_key_lies_exactly_on_segment(
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
        NodeRailConstraintKind::RaisedStepContact => NodeSeamSource::RaisedStepContact {
            owner_index: owner.owner_index(),
        },
        NodeRailConstraintKind::AsphaltBoundary { .. } => NodeSeamSource::AsphaltBoundary {
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
        RoadSurfaceBandKind::CurbOrShoulder => NodeSeamSource::RaisedStepContact {
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

fn owners_form_raised_step_contact(owner: NodeBandOwner, opposite_owner: NodeBandOwner) -> bool {
    let Some(owner_rank) = raised_step_band_kind_rank(owner.kind()) else {
        return false;
    };
    let Some(opposite_rank) = raised_step_band_kind_rank(opposite_owner.kind()) else {
        return false;
    };
    owner_rank.abs_diff(opposite_rank) == 1
}

fn raised_step_contact_priority_for_owners(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<usize> {
    let owner_rank = raised_step_band_kind_rank(owner.kind())?;
    let opposite_rank = raised_step_band_kind_rank(opposite_owner.kind())?;
    (owner_rank.abs_diff(opposite_rank) == 1).then_some(usize::from(owner_rank.min(opposite_rank)))
}

fn raised_step_contact_requires_exact_constraint_span(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    owner.kind() == RoadSurfaceBandKind::Footpath
        || opposite_owner.kind() == RoadSurfaceBandKind::Footpath
        || raised_step_contact_priority_for_owners(owner, opposite_owner) == Some(0)
}

fn raised_step_contact_constrains_shared_height(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    owners_form_raised_step_contact(owner, opposite_owner)
        && !raised_step_contact_requires_exact_constraint_span(owner, opposite_owner)
}

fn raised_step_band_kind_rank(kind: RoadSurfaceBandKind) -> Option<u8> {
    match kind {
        RoadSurfaceBandKind::Carriageway => Some(0),
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Footpath => Some(1),
        RoadSurfaceBandKind::Sidewalk => Some(2),
        RoadSurfaceBandKind::Median
        | RoadSurfaceBandKind::Parking
        | RoadSurfaceBandKind::CycleTrack
        | RoadSurfaceBandKind::TramReservation => None,
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

impl ResidualKind {
    fn requires_explicit_profile_seam_rail(self) -> bool {
        match self {
            ResidualKind::Band(kind) => band_kind_requires_explicit_profile_seam_rail(kind),
            ResidualKind::Asphalt | ResidualKind::NonRoad => false,
        }
    }
}

fn validate_non_road_regions_have_explicit_profile_seam_rails(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::backend::road_points_to_polyline;
    use crate::simulation::network::surface::input::NodeArrangementInput;
    use crate::simulation::network::surface::rails::{
        NodeGeneratedContourPurpose, NodeRailContourSet,
    };
    use crate::simulation::network::surface::validation::NodeValidationReport;
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
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
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

    fn test_rail_canonical_points_from_constraints(
        rail_constraints: &[NodeRailConstraint],
    ) -> NodeRailCanonicalPointSet {
        let mut all_points = rail_constraints
            .iter()
            .flat_map(|constraint| constraint.points_xz.iter().copied())
            .map(road_point_key)
            .collect::<Vec<_>>();
        all_points.sort_unstable();
        all_points.dedup();

        let mut points_by_owner = BTreeMap::<NodeBandOwner, Vec<NodeOwnershipPointKey>>::new();
        let mut segments_by_owner = BTreeMap::<NodeBandOwner, Vec<OwnedRegionEdgeKey>>::new();
        for constraint in rail_constraints {
            let path = constraint
                .points_xz
                .iter()
                .copied()
                .map(road_point_key)
                .collect::<Vec<_>>();
            for owner in constraint_authority_owners(constraint) {
                points_by_owner
                    .entry(owner)
                    .or_default()
                    .extend(path.iter().copied());
                insert_open_source_segments(&mut segments_by_owner, owner, &path);
            }
        }
        for points in points_by_owner.values_mut() {
            points.sort_unstable();
            points.dedup();
        }
        for segments in segments_by_owner.values_mut() {
            segments.sort_unstable();
            segments.dedup();
        }
        let canonical_points_by_mm_key_by_owner =
            canonical_points_by_mm_key_by_owner(&points_by_owner);
        NodeRailCanonicalPointSet {
            all_points,
            points_by_owner,
            segments_by_owner,
            canonical_points_by_mm_key_by_owner,
            height_points_by_source: BTreeMap::new(),
            paths_by_owner: BTreeMap::new(),
        }
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
                        NodeSeamSource::RaisedStepContact { .. }
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
    fn non_road_owner_regions_require_explicit_profile_seam_rails() {
        let mut rails = contour_set();
        rails.constraints.retain(|constraint| {
            matches!(
                constraint.kind,
                NodeRailConstraintKind::FullRoadbedContour
                    | NodeRailConstraintKind::BandContour { .. }
            )
        });

        let error = NodeBooleanOwnership::from_rails(&rails)
            .expect_err("non-road owner carriers without profile seam rails must be rejected");

        assert!(matches!(
            error,
            NodeBooleanOwnershipError::UnownedBandResidual {
                kind: RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk,
                ..
            }
        ));
        let report = NodeValidationReport::from_boolean_ownership_error(
            rails.node_id,
            rails.piece_kind,
            &error,
        );
        let dump = report.debug_dump();
        assert!(dump.contains("\"stage\":\"boolean_ownership\""));
        assert!(dump.contains("\"kind\":\"rejected_residual\""));
    }

    #[test]
    fn contour_purpose_gates_junction_footprint_and_asphalt_authority() {
        let mut rails = contour_set();
        let baseline =
            NodeBooleanOwnership::from_rails(&rails).expect("baseline ownership solve is valid");
        let ignored_footprint_points = vec![
            RoadVec2::new(100.0, 100.0),
            RoadVec2::new(102.0, 100.0),
            RoadVec2::new(102.0, 102.0),
            RoadVec2::new(100.0, 102.0),
        ];
        rails.contours.push(NodeGeneratedContour {
            kind: NodeGeneratedContourKind::FullRoadbed,
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            source_mouth_order_index: 0,
            source_band_index: None,
            owner: None,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            points_xz: ignored_footprint_points.clone(),
            height_points_world: None,
            backend_polyline: road_points_to_polyline(ignored_footprint_points, true),
        });

        let outside_asphalt_points = vec![
            RoadVec2::new(110.0, 100.0),
            RoadVec2::new(112.0, 100.0),
            RoadVec2::new(112.0, 102.0),
            RoadVec2::new(110.0, 102.0),
        ];
        rails.contours.push(NodeGeneratedContour {
            kind: NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            purpose: NodeGeneratedContourPurpose::CarriagewayCorridor,
            source_mouth_order_index: 99,
            source_band_index: Some(99),
            owner: Some(NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 99)),
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            points_xz: outside_asphalt_points.clone(),
            height_points_world: None,
            backend_polyline: road_points_to_polyline(outside_asphalt_points, true),
        });
        let owner_carrier_only_points = vec![
            RoadVec2::new(1.0, -3.5),
            RoadVec2::new(3.0, -3.5),
            RoadVec2::new(3.0, -2.5),
            RoadVec2::new(1.0, -2.5),
        ];
        rails.contours.push(NodeGeneratedContour {
            kind: NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            purpose: NodeGeneratedContourPurpose::CarriagewayOwnerCarrier,
            source_mouth_order_index: 98,
            source_band_index: Some(98),
            owner: Some(NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 98)),
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            points_xz: owner_carrier_only_points.clone(),
            height_points_world: None,
            backend_polyline: road_points_to_polyline(owner_carrier_only_points, true),
        });

        let ownership =
            NodeBooleanOwnership::from_rails(&rails).expect("extra gated contours remain valid");
        assert_eq!(ownership.footprint_shapes, baseline.footprint_shapes);
        assert_eq!(ownership.asphalt_shapes, baseline.asphalt_shapes);
        let asphalt_outside = overlay_difference(
            &ownership.asphalt_shapes,
            &ownership.footprint_shapes,
            "test_asphalt_outside_footprint",
        )
        .expect("test overlay difference succeeds");
        assert!(
            asphalt_outside.is_empty(),
            "asphalt authority must be clipped to node_footprint"
        );
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
                owner: None,
                opposite_owner: None,
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
            &[],
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
    fn materializes_seam_constraints_for_final_noded_owned_edges() {
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::Sidewalk,
                sidewalk,
                vec![[1.0, 0.0], [2.0, 0.0], [2.0, 2.0], [1.0, 2.0]],
            ),
        ];
        let footprint_shapes = vec![vec![vec![
            [0.0, 0.0],
            [2.0, 0.0],
            [2.0, 2.0],
            [1.0, 1.0],
            [0.0, 2.0],
        ]]];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 33,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(curb),
            opposite_owner: Some(sidewalk),
            points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(1.0, 2.0)],
        }];

        let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
        canonicalize_final_owned_region_boundary_edges(
            &mut regions,
            &footprint_shapes,
            &rail_canonical_points,
        );
        materialize_noded_region_seam_constraints(
            &mut regions,
            &footprint_shapes,
            &rail_constraints,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        );

        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    road_point_key(constraint.start_xz) == road_point_key(RoadVec2::new(1.0, 0.0))
                        && road_point_key(constraint.end_xz)
                            == road_point_key(RoadVec2::new(1.0, 1.0))
                        && constraint.owner == Some(curb)
                        && constraint.opposite_owner == Some(sidewalk)
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "first final subedge must carry the original raised-step seam"
            );
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    road_point_key(constraint.start_xz) == road_point_key(RoadVec2::new(1.0, 1.0))
                        && road_point_key(constraint.end_xz)
                            == road_point_key(RoadVec2::new(1.0, 2.0))
                        && constraint.owner == Some(curb)
                        && constraint.opposite_owner == Some(sidewalk)
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "second final subedge must carry the original raised-step seam"
            );
        }
        let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            42,
            RoadSurfaceVisualNodePieceKind::Terminal,
            &regions,
            &footprint_shapes,
            &rail_constraints,
        );

        assert!(arrangement.diagnostics().is_empty());
    }

    #[test]
    fn source_local_owned_boundary_preserves_explicit_height_endpoint_authority() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let local_endpoint = (1_000_001, 0);
        let canonical_endpoint = (1_000_000, 0);
        let mut regions = vec![test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![
                [0.0, 0.0],
                overlay_point_from_key(local_endpoint),
                [0.0, 1.0],
            ],
        )];
        let mut height_points_by_source = BTreeMap::new();
        height_points_by_source.insert(
            (
                RoadSurfaceBandKind::Carriageway,
                carriageway.owner_index(),
                carriageway.owner_index(),
            ),
            vec![local_endpoint],
        );
        let rail_canonical_points = NodeRailCanonicalPointSet {
            all_points: vec![canonical_endpoint],
            points_by_owner: BTreeMap::from([(carriageway, vec![canonical_endpoint])]),
            segments_by_owner: BTreeMap::new(),
            canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(
                &BTreeMap::from([(carriageway, vec![canonical_endpoint])]),
            ),
            height_points_by_source,
            paths_by_owner: BTreeMap::new(),
        };

        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points);

        let contour_keys = regions[0].shape[0]
            .iter()
            .copied()
            .map(overlay_point_key)
            .collect::<BTreeSet<_>>();
        assert!(contour_keys.contains(&canonical_endpoint));
        assert!(contour_keys.contains(&local_endpoint));
        assert!(
            validate_owned_region_vertices_against_source_authority(
                &regions,
                &rail_canonical_points
            )
            .is_ok()
        );
    }

    #[test]
    fn noncanonical_owned_region_vertex_reports_source_authority_error() {
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let drifted_endpoint = [1.000004, 0.0];
        let regions = vec![test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![drifted_endpoint, [2.0, 0.0], [2.0, 2.0]],
        )];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 33,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(curb),
            opposite_owner: None,
            points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(1.0, 2.0)],
        }];
        let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);

        assert!(matches!(
            validate_owned_region_vertices_against_source_authority(
                &regions,
                &rail_canonical_points
            ),
            Err(NodeBooleanOwnershipError::NonCanonicalOwnedRegionVertex {
                owner,
                point_x_key,
                point_z_key,
                canonical_x_key,
                canonical_z_key,
            }) if owner == curb
                && point_x_key == overlay_point_key(drifted_endpoint).0
                && point_z_key == overlay_point_key(drifted_endpoint).1
                && canonical_x_key == road_point_key(RoadVec2::new(1.0, 0.0)).0
                && canonical_z_key == road_point_key(RoadVec2::new(1.0, 0.0)).1
        ));
    }

    #[test]
    fn materializes_owner_explicit_step_for_final_edge_on_exact_constraint_span() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(2.0, 1.0);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [2.0, 1.0], [0.0, 2.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![[0.0, 0.0], [3.0, -1.0], [2.0, 1.0]],
            ),
        ];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 34,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            points_xz: vec![start, end],
        }];

        materialize_noded_region_seam_constraints(
            &mut regions,
            &Vec::new(),
            &rail_constraints,
            RoadSurfaceVisualNodePieceKind::Bend,
        );

        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    road_point_key(constraint.start_xz) == road_point_key(start)
                        && road_point_key(constraint.end_xz) == road_point_key(end)
                        && ((constraint.owner == Some(carriageway)
                            && constraint.opposite_owner == Some(curb))
                            || (constraint.owner == Some(curb)
                                && constraint.opposite_owner == Some(carriageway)))
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "final shared asphalt-curb edge must carry the owner-explicit step seam"
            );
        }
    }

    #[test]
    fn materializes_asymmetric_asphalt_curb_boundary_from_final_noded_edges() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let start = RoadVec2::new(0.0, 0.0);
        let first_split = RoadVec2::new(1.0, 0.0);
        let second_split = RoadVec2::new(2.0, 0.0);
        let end = RoadVec2::new(3.0, 0.0);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [3.0, 0.0], [3.0, -1.0], [0.0, -1.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![
                    [0.0, 0.0],
                    [1.0, 0.0],
                    [2.0, 0.0],
                    [3.0, 0.0],
                    [3.0, 1.0],
                    [0.0, 1.0],
                ],
            ),
        ];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 37,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            points_xz: vec![start, first_split, second_split, end],
        }];
        let footprint_shapes = Vec::new();

        let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
        canonicalize_final_owned_region_boundary_edges(
            &mut regions,
            &footprint_shapes,
            &rail_canonical_points,
        );
        materialize_noded_region_seam_constraints(
            &mut regions,
            &footprint_shapes,
            &rail_constraints,
            RoadSurfaceVisualNodePieceKind::Bend,
        );

        let carriageway_contour = &regions[0].shape[0];
        assert!(
            carriageway_contour
                .iter()
                .any(|point| overlay_point_key(*point) == road_point_key(first_split))
        );
        assert!(
            carriageway_contour
                .iter()
                .any(|point| overlay_point_key(*point) == road_point_key(second_split))
        );
        for (subedge_start, subedge_end) in [
            (start, first_split),
            (first_split, second_split),
            (second_split, end),
        ] {
            for region in &regions {
                assert!(
                    region.seam_constraints.iter().any(|constraint| {
                        road_point_key(constraint.start_xz) == road_point_key(subedge_start)
                            && road_point_key(constraint.end_xz) == road_point_key(subedge_end)
                            && constraint.owner == Some(carriageway)
                            && constraint.opposite_owner == Some(curb)
                            && matches!(
                                constraint.seam_source,
                                NodeSeamSource::RaisedStepContact { .. }
                            )
                    }),
                    "final owned asphalt-curb subedge must carry the exact explicit step seam"
                );
            }
        }

        let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            42,
            RoadSurfaceVisualNodePieceKind::Bend,
            &regions,
            &footprint_shapes,
            &rail_constraints,
        );
        assert!(arrangement.diagnostics().is_empty());
        assert!(!arrangement.edges().iter().any(|edge| {
            edge.owner == carriageway
                && edge.opposite_owner == Some(curb)
                && edge.start == NodeOwnedRegionArrangementKey::from_point(start)
                && edge.end == NodeOwnedRegionArrangementKey::from_point(end)
        }));
    }

    #[test]
    fn junctionn_materializes_final_step_edge_from_exact_owner_pair_polyline_authority() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(3.0, 0.0);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [3.0, 0.0], [0.0, -1.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![[0.0, 0.0], [3.0, 0.0], [3.0, 1.0], [0.0, 1.0]],
            ),
        ];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 41,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            points_xz: vec![
                start,
                RoadVec2::new(1.0, 0.000001),
                RoadVec2::new(2.0, -0.000001),
                end,
            ],
        }];
        let footprint_shapes = Vec::new();

        materialize_noded_region_seam_constraints(
            &mut regions,
            &footprint_shapes,
            &rail_constraints,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        );

        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    road_point_key(constraint.start_xz) == road_point_key(start)
                        && road_point_key(constraint.end_xz) == road_point_key(end)
                        && constraint.owner == Some(carriageway)
                        && constraint.opposite_owner == Some(curb)
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "JunctionN final asphalt-curb edge must materialize from exact source-pair polyline authority"
            );
        }

        let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &regions,
            &footprint_shapes,
            &rail_constraints,
        );
        assert!(arrangement.diagnostics().is_empty());
    }

    #[test]
    fn junctionn_reports_unmaterialized_raised_step_authority_before_height_validation() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(3.0, 0.0);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [3.0, 0.0], [0.0, -1.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![[0.0, 0.0], [3.0, 0.0], [3.0, 1.0], [0.0, 1.0]],
            ),
        ];
        for region in &mut regions {
            region.seam_constraints.push(NodeRegionSeamConstraint {
                constraint_index: 7,
                seam_source: NodeSeamSource::AsphaltBoundary {
                    owner_index: region.owner.owner_index(),
                },
                owner: Some(carriageway),
                opposite_owner: Some(curb),
                constrains_shared_height: true,
                is_material_transition: true,
                start_xz: start,
                end_xz: end,
            });
        }
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 41,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            points_xz: vec![
                start,
                RoadVec2::new(1.0, 0.000001),
                RoadVec2::new(2.0, -0.000001),
                end,
            ],
        }];

        let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &regions,
            &Vec::new(),
            &rail_constraints,
        );

        assert!(arrangement.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic,
            NodeOwnedRegionArrangementDiagnostic::UnmaterializedRaisedStepAuthority {
                region_index: 0,
                owner,
                opposite_owner,
                start,
                end,
                source_constraint_indices,
            } if *owner == carriageway
                && *opposite_owner == curb
                && *start == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(0.0, 0.0))
                && *end == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(3.0, 0.0))
                && source_constraint_indices.as_slice() == [41]
        )));
        let report = NodeValidationReport::from_owned_region_arrangement_diagnostics(&arrangement)
            .expect("unmaterialized authority must block before height validation");
        let dump = report.debug_dump();
        assert!(dump.contains("\"kind\":\"unmaterialized_raised_step_authority\""));
        assert!(dump.contains("\"backend\":\"canonical_keys\""));
        assert!(dump.contains("source_constraint_indices: [41]"));
    }

    #[test]
    fn materializes_role_only_raised_step_contact_as_exact_owned_edge_pair() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(2.0, 1.0);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [2.0, 1.0], [0.0, 2.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![[0.0, 0.0], [3.0, -1.0], [2.0, 1.0]],
            ),
        ];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 35,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(curb),
            opposite_owner: None,
            points_xz: vec![start, end],
        }];

        materialize_noded_region_seam_constraints(
            &mut regions,
            &Vec::new(),
            &rail_constraints,
            RoadSurfaceVisualNodePieceKind::Bend,
        );

        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    road_point_key(constraint.start_xz) == road_point_key(start)
                        && road_point_key(constraint.end_xz) == road_point_key(end)
                        && ((constraint.owner == Some(carriageway)
                            && constraint.opposite_owner == Some(curb))
                            || (constraint.owner == Some(curb)
                                && constraint.opposite_owner == Some(carriageway)))
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "role-only asphalt-curb contact must instantiate the actual owned edge pair"
            );
        }
    }

    #[test]
    fn materializes_same_kind_reowned_raised_step_contact_as_exact_owned_edge_pair() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let source_curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let final_curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(2.0, 0.0);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [2.0, 0.0], [0.0, -1.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                final_curb,
                vec![[0.0, 0.0], [2.0, 0.0], [0.0, 1.0]],
            ),
        ];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 35,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(carriageway),
            opposite_owner: Some(source_curb),
            points_xz: vec![start, end],
        }];

        materialize_noded_region_seam_constraints(
            &mut regions,
            &Vec::new(),
            &rail_constraints,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        );

        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    road_point_key(constraint.start_xz) == road_point_key(start)
                        && road_point_key(constraint.end_xz) == road_point_key(end)
                        && ((constraint.owner == Some(carriageway)
                            && constraint.opposite_owner == Some(final_curb))
                            || (constraint.owner == Some(final_curb)
                                && constraint.opposite_owner == Some(carriageway)))
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "final owned edge must instantiate its exact owner pair from a same-kind source rail"
            );
        }
    }

    #[test]
    fn materializes_cross_material_contact_from_exact_final_owner_band_contour_edge() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(2.0, 1.0);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [2.0, 1.0], [0.0, 2.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![[0.0, 0.0], [3.0, -1.0], [2.0, 1.0]],
            ),
        ];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 41,
            kind: NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            source_mouth_order_index: 0,
            source_band_index: Some(0),
            source_boundary_index: None,
            owner: Some(carriageway),
            opposite_owner: None,
            points_xz: vec![start, end],
        }];

        materialize_noded_region_seam_constraints(
            &mut regions,
            &Vec::new(),
            &rail_constraints,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        );

        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    road_point_key(constraint.start_xz) == road_point_key(start)
                        && road_point_key(constraint.end_xz) == road_point_key(end)
                        && ((constraint.owner == Some(carriageway)
                            && constraint.opposite_owner == Some(curb))
                            || (constraint.owner == Some(curb)
                                && constraint.opposite_owner == Some(carriageway)))
                        && !constraint.constrains_shared_height
                        && constraint.is_material_transition
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "exact final owner band contour edge must authorize the asphalt-curb step"
            );
        }
    }

    #[test]
    fn projected_material_boundary_canonicalizes_source_authorized_endpoint() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let source_start = RoadVec2::new(1.0, 0.0);
        let drifted_start = [1.000004, 0.0];
        let end = RoadVec2::new(1.0, 2.0);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![drifted_start, [2.0, 0.0], [2.0, 2.0], [1.000004, 2.0]],
            ),
        ];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 41,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            points_xz: vec![source_start, end],
        }];
        let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);

        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points);

        let curb_points = regions[1].shape[0]
            .iter()
            .copied()
            .map(overlay_point_key)
            .collect::<BTreeSet<_>>();
        assert!(curb_points.contains(&road_point_key(source_start)));
        assert!(!curb_points.contains(&overlay_point_key(drifted_start)));
        assert!(
            validate_owned_region_vertices_against_source_authority(
                &regions,
                &rail_canonical_points
            )
            .is_ok()
        );
    }

    #[test]
    fn does_not_materialize_cross_material_contact_from_band_contour_chord() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let start = RoadVec2::new(0.0, 0.0);
        let middle = RoadVec2::new(1.0, 1.0);
        let end = RoadVec2::new(2.0, 0.0);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [2.0, 0.0], [0.0, -1.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![[0.0, 0.0], [2.0, 0.0], [0.0, 1.0]],
            ),
        ];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 42,
            kind: NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            source_mouth_order_index: 0,
            source_band_index: Some(0),
            source_boundary_index: None,
            owner: Some(carriageway),
            opposite_owner: None,
            points_xz: vec![start, middle, end],
        }];

        materialize_noded_region_seam_constraints(
            &mut regions,
            &Vec::new(),
            &rail_constraints,
            RoadSurfaceVisualNodePieceKind::Bend,
        );

        for region in &regions {
            assert!(
                !region.seam_constraints.iter().any(|constraint| {
                    road_point_key(constraint.start_xz) == road_point_key(start)
                        && road_point_key(constraint.end_xz) == road_point_key(end)
                        && constraint.owner == Some(carriageway)
                        && constraint.opposite_owner == Some(curb)
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "band contours authorize final contacts only on exact source segments"
            );
        }
    }

    #[test]
    fn does_not_materialize_asphalt_curb_step_from_bend_polyline_coverage() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(2.0, 2.0);
        let mut regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![[0.0, 0.0], [2.0, 2.0], [0.0, 2.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::CurbOrShoulder,
                curb,
                vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]],
            ),
        ];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 35,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            points_xz: vec![start, RoadVec2::new(1.0, 1.0), end],
        }];

        materialize_noded_region_seam_constraints(
            &mut regions,
            &Vec::new(),
            &rail_constraints,
            RoadSurfaceVisualNodePieceKind::Bend,
        );

        for region in &regions {
            assert!(
                !region.seam_constraints.iter().any(|constraint| {
                    road_point_key(constraint.start_xz) == road_point_key(start)
                        && road_point_key(constraint.end_xz) == road_point_key(end)
                        && constraint.owner == Some(carriageway)
                        && constraint.opposite_owner == Some(curb)
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "asphalt-curb vertical steps must come from an exact rail span, not Bend polyline coverage"
            );
        }
    }

    #[test]
    fn asphalt_curb_shape_seams_use_exact_constraint_spans() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let start = RoadVec2::new(0.0, 0.0);
        let middle = RoadVec2::new(1.0, 1.0);
        let end = RoadVec2::new(2.0, 2.0);
        let shape = vec![vec![[0.0, 0.0], [2.0, 2.0], [0.0, 2.0]]];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 36,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            points_xz: vec![start, middle, end],
        }];

        let seams = seam_constraints_for_shape(&shape, carriageway, &rail_constraints, false);

        assert!(
            !seams.iter().any(|constraint| {
                road_point_key(constraint.start_xz) == road_point_key(start)
                    && road_point_key(constraint.end_xz) == road_point_key(end)
            }),
            "asphalt-curb seams must not carry a full edge just because a rail polyline covers it"
        );
        assert!(
            seams.iter().any(|constraint| {
                road_point_key(constraint.start_xz) == road_point_key(start)
                    && road_point_key(constraint.end_xz) == road_point_key(middle)
            }),
            "first exact rail span should be preserved"
        );
        assert!(
            seams.iter().any(|constraint| {
                road_point_key(constraint.start_xz) == road_point_key(middle)
                    && road_point_key(constraint.end_xz) == road_point_key(end)
            }),
            "second exact rail span should be preserved"
        );
    }

    #[test]
    fn canonicalizes_overlay_vertex_drift_to_unique_source_rail_key() {
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut regions = vec![test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [1.000004, 0.0], [1.000004, 2.0], [0.0, 2.0]],
        )];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 33,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(curb),
            opposite_owner: None,
            points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(1.0, 2.0)],
        }];

        let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points);

        let contour = &regions[0].shape[0];
        assert!(
            contour
                .iter()
                .any(|point| overlay_point_key(*point) == road_point_key(RoadVec2::new(1.0, 0.0)))
        );
        assert!(
            contour
                .iter()
                .any(|point| overlay_point_key(*point) == road_point_key(RoadVec2::new(1.0, 2.0)))
        );
        assert!(
            contour.iter().all(|point| {
                overlay_point_key(*point) != overlay_point_key([1.000004, 0.0])
                    && overlay_point_key(*point) != overlay_point_key([1.000004, 2.0])
            }),
            "owned region vertices must use the owner-authorized source rail keys, not backend drift"
        );
        assert!(
            validate_owned_region_vertices_against_source_authority(
                &regions,
                &rail_canonical_points
            )
            .is_ok()
        );
    }

    #[test]
    fn canonicalizes_closing_overlay_dust_to_source_rail_endpoint() {
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 6);
        let endpoint = RoadVec2::new(15.169048, 5.0);
        let mut regions = vec![test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![
                [15.169047, 5.0],
                [15.169048, 3.65],
                [15.979047, 3.65],
                [15.596568, 4.287465],
                [15.169048, 4.999998],
            ],
        )];
        let rail_constraints = vec![NodeRailConstraint {
            constraint_index: 34,
            kind: NodeRailConstraintKind::FootprintSeam {
                adjacent_kind: RoadSurfaceBandKind::Sidewalk,
            },
            source_mouth_order_index: 1,
            source_band_index: Some(0),
            source_boundary_index: Some(0),
            owner: Some(sidewalk),
            opposite_owner: None,
            points_xz: vec![endpoint, RoadVec2::new(15.169048, 3.65)],
        }];

        let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points);

        let contour = &regions[0].shape[0];
        let endpoint_key = road_point_key(endpoint);
        assert_eq!(overlay_point_key(contour[0]), endpoint_key);
        assert_eq!(
            contour
                .iter()
                .filter(|point| overlay_point_key(**point) == endpoint_key)
                .count(),
            1,
            "closing overlay dust must collapse onto the authorized source rail endpoint"
        );
        assert!(
            validate_owned_region_vertices_against_source_authority(
                &regions,
                &rail_canonical_points
            )
            .is_ok()
        );
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
                owner: None,
                opposite_owner: None,
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
            &[],
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
    fn same_band_owned_region_edge_does_not_require_material_seam_constraint() {
        let first = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let second = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 1);
        let regions = vec![
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                first,
                vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
            ),
            test_owned_region(
                RoadSurfaceBandKind::Carriageway,
                second,
                vec![[4.0, 0.0], [6.0, 0.0], [6.0, 2.0], [4.0, 2.0]],
            ),
        ];

        let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            44,
            RoadSurfaceVisualNodePieceKind::Bend,
            &regions,
            &Vec::new(),
            &[],
        );

        assert!(arrangement.diagnostics().is_empty());
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
