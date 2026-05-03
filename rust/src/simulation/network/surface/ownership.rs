//! Boolean ownership solve for canonical node-arrangement contours.

#![allow(dead_code)]

use super::arrangement::{
    NodeBandOwner, NodeHeightSource, NodeRegionSeamConstraint, NodeSeamSource,
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
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeBooleanOwnedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: NodeBandOwner,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) shape: NodeOverlayShape,
    pub(crate) area_m2: f32,
    pub(crate) height_sources: Vec<NodeHeightSource>,
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
        Ok(Self {
            node_id: rails.node_id,
            piece_kind: rails.piece_kind,
            footprint_shapes,
            asphalt_shapes,
            non_road_shapes,
            owned_regions,
        })
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

    for domain in domains {
        let owner = domain
            .owner
            .ok_or(NodeBooleanOwnershipError::MissingBandOwner {
                mouth_order_index: domain.source_mouth_order_index,
                band_index: domain.source_band_index,
            })?;
        let domain_contour = overlay_contour_from_domain(domain);
        let mut domain_shapes = overlay_union(&[domain_contour], "domain_union")?;
        domain_shapes = overlay_intersect(&domain_shapes, target_shapes, "domain_clip")?;
        domain_shapes = overlay_difference(&domain_shapes, &claimed_shapes, "domain_unclaimed")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut domain_shapes);
        if domain_shapes.is_empty() {
            continue;
        }

        for shape in &domain_shapes {
            let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(shape);
            if owned_shape_is_numeric_dust(shape, area_m2) {
                continue;
            }
            regions.push(NodeBooleanOwnedRegion {
                kind: band_kind(domain).expect("owned domain must be a band contour"),
                owner,
                source_mouth_order_index: domain.source_mouth_order_index,
                source_band_index: domain.source_band_index,
                shape: shape.clone(),
                area_m2,
                height_sources: canonical_height_sources(domain.height_sources.iter().cloned()),
                seam_constraints: seam_constraints_for_shape(shape, owner, rail_constraints),
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

fn residual_regions_from_domains(
    residual_shapes: &NodeOverlayShapes,
    domains: &[&NodeGeneratedContour],
    rail_constraints: &[NodeRailConstraint],
    stage: &'static str,
) -> Result<OwnedDomainResult, NodeBooleanOwnershipError> {
    let mut regions = Vec::new();
    let mut claimed_shapes = Vec::new();
    for domain in domains {
        let owner = domain
            .owner
            .ok_or(NodeBooleanOwnershipError::MissingBandOwner {
                mouth_order_index: domain.source_mouth_order_index,
                band_index: domain.source_band_index,
            })?;
        let domain_contour = overlay_contour_from_domain(domain);
        let mut domain_shapes = overlay_union(&[domain_contour], stage)?;
        domain_shapes = overlay_intersect(&domain_shapes, residual_shapes, stage)?;
        domain_shapes = overlay_difference(&domain_shapes, &claimed_shapes, stage)?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut domain_shapes);
        if domain_shapes.is_empty() {
            continue;
        }

        for shape in &domain_shapes {
            let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(shape);
            if owned_shape_is_numeric_dust(shape, area_m2) {
                continue;
            }
            regions.push(NodeBooleanOwnedRegion {
                kind: band_kind(domain).expect("owned domain must be a band contour"),
                owner,
                source_mouth_order_index: domain.source_mouth_order_index,
                source_band_index: domain.source_band_index,
                shape: shape.clone(),
                area_m2,
                height_sources: canonical_height_sources(domain.height_sources.iter().cloned()),
                seam_constraints: seam_constraints_for_shape(shape, owner, rail_constraints),
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
    rails
        .contours
        .iter()
        .filter(|contour| band_kind(contour) == Some(kind))
        .collect()
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

fn owned_shape_is_numeric_dust(shape: &NodeOverlayShape, area_m2: f32) -> bool {
    area_m2 <= RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(shape)
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

    let mut next_constraint_index = first_constraint_index;
    for (edge, refs) in edges {
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
    for (point, refs) in points_by_region {
        if append_shared_constraint_for_refs(regions, &refs, next_constraint_index, point, point) {
            next_constraint_index += 1;
        }
    }

    for region in regions {
        canonicalize_seam_constraints(&mut region.seam_constraints);
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
                constrains_shared_height: true,
                is_material_transition: true,
                start_xz: road_point_from_key(start),
                end_xz: road_point_from_key(end),
            });
    }
    true
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
    !matches!(
        constraint.kind,
        NodeRailConstraintKind::FootprintSeam { .. }
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
    if constraint.owner == Some(owner) || constraint.opposite_owner == Some(owner) {
        return true;
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

type NodeOwnershipPointKey = (i64, i64);

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

fn canonical_height_sources(
    sources: impl IntoIterator<Item = NodeHeightSource>,
) -> Vec<NodeHeightSource> {
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    sources
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
        assert!(ownership.owned_regions.iter().any(|region| region.kind
            == RoadSurfaceBandKind::Carriageway
            && region.owner == NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2)
            && !region.height_sources.is_empty()
            && !region.seam_constraints.is_empty()));
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
                        && constraint.constrains_shared_height
                        && constraint.is_material_transition
                }),
                "region {:?} must own the exact shared sub-edge seam before height/CDT",
                region.owner
            );
        }
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
            height_sources: vec![NodeHeightSource::ArrangementConstraint {
                constraint_index: owner.owner_index(),
            }],
            seam_constraints: Vec::new(),
        }
    }
}
