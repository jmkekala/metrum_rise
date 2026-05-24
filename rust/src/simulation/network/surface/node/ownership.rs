//! Boolean ownership solve for canonical node-arrangement contours.

use super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource};
use super::rails::{
    NodeGeneratedContourClaimPriority, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailContourSet,
};
use super::{
    NodeOverlayContour, NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};

mod boundaries;
mod contact_semantics;
mod domains;
mod rail_authority;
mod rings;
mod seams;
mod topology_keys;

use domains::{
    ResidualKind, asphalt_authority_domains, asphalt_owner_domains, overlay_contour_from_domain,
    overlay_contours_for_domains, overlay_difference, overlay_intersect, overlay_union,
    owned_regions_from_domains, reject_residual, sort_boolean_owned_regions,
    split_non_road_regions, validate_non_road_regions_have_explicit_profile_seam_rails,
};

use rings::{canonicalize_owned_region_rings, clean_canonical_owned_region_shapes};

use rail_authority::canonical_points_for_rail_set;

use seams::{
    ConstraintOverlapMode, materialize_noded_region_seam_constraints, seam_constraints_for_shape,
};
use std::collections::BTreeMap;
use topology_keys::{
    NodeOwnershipPointKey, overlay_point_from_key, ownership_key_from_overlay_point,
    ownership_key_from_road_point, ownership_mm_key, point_key_lies_on_segment,
    segment_parameter_key,
};

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
    AmbiguousOwnedBoundaryEdge {
        region_index: usize,
        owner: NodeBandOwner,
        opposite_owners: Vec<NodeBandOwner>,
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
    AmbiguousCanonicalOwnedRegionVertex {
        owner: NodeBandOwner,
        point_x_key: i64,
        point_z_key: i64,
        candidates: Vec<NodeOwnershipPointKey>,
    },
}

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_node_boolean_ownership_from_rails(
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

        let constraint_overlap_mode = ConstraintOverlapMode::for_piece_kind(rails.piece_kind);
        let mut owned_regions = Vec::new();
        let asphalt_owner_domains = asphalt_owner_domains(rails);
        let asphalt_result = owned_regions_from_domains(
            &asphalt_shapes,
            &asphalt_owner_domains,
            &rails.constraints,
            ResidualKind::Asphalt,
            constraint_overlap_mode,
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
            constraint_overlap_mode,
        )?;
        footprint_shapes =
            final_footprint_shapes_from_owned_regions(rails.node_id, &owned_regions)?;
        canonicalize_footprint_shapes_with_final_points(&mut footprint_shapes, &owned_regions);
        rebuild_owned_region_seam_constraints(
            &mut owned_regions,
            &footprint_shapes,
            &rails.constraints,
            constraint_overlap_mode,
            rails.piece_kind,
        );
        let mut owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
            rails.node_id,
            rails.piece_kind,
            &owned_regions,
            &footprint_shapes,
            &rails.constraints,
        );
        if promote_source_authorized_asphalt_adjacent_sidewalk_slivers(
            &mut owned_regions,
            &owned_region_arrangement,
            &rails.constraints,
        ) {
            footprint_shapes =
                final_footprint_shapes_from_owned_regions(rails.node_id, &owned_regions)?;
            canonicalize_footprint_shapes_with_final_points(&mut footprint_shapes, &owned_regions);
            rebuild_owned_region_seam_constraints(
                &mut owned_regions,
                &footprint_shapes,
                &rails.constraints,
                constraint_overlap_mode,
                rails.piece_kind,
            );
            owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
                rails.node_id,
                rails.piece_kind,
                &owned_regions,
                &footprint_shapes,
                &rails.constraints,
            );
        }
        for _ in 0..2 {
            if !materialize_final_boundary_vertices_for_height(
                rails.node_id,
                &mut owned_regions,
                &mut footprint_shapes,
                &mut owned_region_arrangement,
                &rails.constraints,
                constraint_overlap_mode,
                rails.piece_kind,
            )? {
                break;
            }
        }
        validate_non_road_regions_have_explicit_profile_seam_rails(
            &owned_regions,
            &rails.constraints,
        )?;
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

fn promote_source_authorized_asphalt_adjacent_sidewalk_slivers(
    owned_regions: &mut [NodeBooleanOwnedRegion],
    arrangement: &NodeOwnedRegionArrangement,
    rail_constraints: &[NodeRailConstraint],
) -> bool {
    let curb_region_sources = curb_region_sources_by_owner(owned_regions);
    let mut promotions = BTreeMap::<usize, (NodeBandOwner, usize, Option<usize>)>::new();
    for diagnostic in arrangement.diagnostics() {
        let NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
            region_index,
            owner,
            opposite_owner,
            start,
            end,
        } = diagnostic
        else {
            continue;
        };
        if owner.kind() != RoadSurfaceBandKind::Sidewalk
            || opposite_owner.kind() != RoadSurfaceBandKind::Carriageway
        {
            continue;
        }
        let Some(region) = owned_regions.get(*region_index) else {
            continue;
        };
        let Some(curb_source) = source_authorized_curb_owner_for_sidewalk_asphalt_sliver(
            region,
            *owner,
            *start,
            *end,
            rail_constraints,
        ) else {
            continue;
        };
        match promotions.get(region_index).copied() {
            Some(existing) if existing != curb_source => {
                promotions.remove(region_index);
            }
            Some(_) => {}
            None => {
                promotions.insert(*region_index, curb_source);
            }
        }
    }

    let mut changed = false;
    for (region_index, (curb_owner, source_mouth_order_index, source_band_index)) in promotions {
        let Some(region) = owned_regions.get_mut(region_index) else {
            continue;
        };
        let (source_mouth_order_index, source_band_index) = if source_band_index.is_some() {
            (source_mouth_order_index, source_band_index)
        } else if let Some(source) = curb_region_sources.get(&curb_owner).copied() {
            source
        } else {
            continue;
        };
        region.kind = RoadSurfaceBandKind::CurbOrShoulder;
        region.owner = curb_owner;
        region.source_mouth_order_index = source_mouth_order_index;
        region.source_band_index = source_band_index;
        changed = true;
    }
    changed
}

fn curb_region_sources_by_owner(
    owned_regions: &[NodeBooleanOwnedRegion],
) -> BTreeMap<NodeBandOwner, (usize, Option<usize>)> {
    let mut sources = BTreeMap::new();
    for region in owned_regions {
        if region.owner.kind() != RoadSurfaceBandKind::CurbOrShoulder
            || region.source_band_index.is_none()
        {
            continue;
        }
        sources
            .entry(region.owner)
            .or_insert((region.source_mouth_order_index, region.source_band_index));
    }
    sources
}

fn source_authorized_curb_owner_for_sidewalk_asphalt_sliver(
    region: &NodeBooleanOwnedRegion,
    sidewalk_owner: NodeBandOwner,
    start: NodeOwnedRegionArrangementKey,
    end: NodeOwnedRegionArrangementKey,
    rail_constraints: &[NodeRailConstraint],
) -> Option<(NodeBandOwner, usize, Option<usize>)> {
    let edge_start = (start.x_key, start.z_key);
    let edge_end = (end.x_key, end.z_key);
    let mut candidates = region
        .seam_constraints
        .iter()
        .filter(|constraint| constraint.is_material_transition)
        .filter(|constraint| {
            matches!(
                constraint.seam_source,
                NodeSeamSource::RaisedStepContact { .. }
            )
        })
        .filter_map(|constraint| {
            let curb_owner = raised_step_curb_owner_for_sidewalk_constraint(
                constraint.owner,
                constraint.opposite_owner,
                sidewalk_owner,
            )?;
            let constraint_start = ownership_key_from_road_point(constraint.start_xz);
            let constraint_end = ownership_key_from_road_point(constraint.end_xz);
            let touches_missing_edge =
                point_key_lies_on_segment(edge_start, constraint_start, constraint_end)
                    || point_key_lies_on_segment(edge_end, constraint_start, constraint_end)
                    || point_key_lies_on_segment(constraint_start, edge_start, edge_end)
                    || point_key_lies_on_segment(constraint_end, edge_start, edge_end);
            if !touches_missing_edge {
                return None;
            }
            let source = rail_constraints
                .iter()
                .find(|rail_constraint| {
                    rail_constraint.constraint_index == constraint.constraint_index
                        && rail_constraint.kind == NodeRailConstraintKind::RaisedStepContact
                        && [rail_constraint.owner, rail_constraint.opposite_owner]
                            .contains(&Some(curb_owner))
                })
                .map(|rail_constraint| {
                    (
                        rail_constraint.source_mouth_order_index,
                        rail_constraint.source_band_index,
                    )
                })
                .unwrap_or((region.source_mouth_order_index, None));
            Some((curb_owner, source.0, source.1))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    (candidates.len() == 1).then_some(candidates[0])
}

fn raised_step_curb_owner_for_sidewalk_constraint(
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    sidewalk_owner: NodeBandOwner,
) -> Option<NodeBandOwner> {
    match (owner, opposite_owner) {
        (Some(left), Some(right))
            if left == sidewalk_owner && right.kind() == RoadSurfaceBandKind::CurbOrShoulder =>
        {
            Some(right)
        }
        (Some(left), Some(right))
            if right == sidewalk_owner && left.kind() == RoadSurfaceBandKind::CurbOrShoulder =>
        {
            Some(left)
        }
        _ => None,
    }
}

fn materialize_final_boundary_vertices_for_height(
    node_id: u32,
    owned_regions: &mut Vec<NodeBooleanOwnedRegion>,
    footprint_shapes: &mut NodeOverlayShapes,
    owned_region_arrangement: &mut NodeOwnedRegionArrangement,
    rail_constraints: &[NodeRailConstraint],
    constraint_overlap_mode: ConstraintOverlapMode,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Result<bool, NodeBooleanOwnershipError> {
    let regions_changed = materialize_final_footprint_vertices_in_owned_regions(
        owned_regions,
        footprint_shapes,
        owned_region_arrangement,
    );
    if !regions_changed {
        return Ok(false);
    }
    rebuild_owned_region_seam_constraints(
        owned_regions,
        footprint_shapes,
        rail_constraints,
        constraint_overlap_mode,
        piece_kind,
    );
    *owned_region_arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        node_id,
        piece_kind,
        owned_regions,
        footprint_shapes,
        rail_constraints,
    );
    Ok(true)
}

fn final_footprint_shapes_from_owned_regions(
    node_id: u32,
    owned_regions: &[NodeBooleanOwnedRegion],
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let contours = owned_regions
        .iter()
        .flat_map(|region| region.shape.iter().cloned())
        .collect::<Vec<_>>();
    if contours.is_empty() {
        return Err(NodeBooleanOwnershipError::EmptyFootprint { node_id });
    }
    overlay_union(&contours, "final_owned_footprint_union")
}

fn rebuild_owned_region_seam_constraints(
    owned_regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_constraints: &[NodeRailConstraint],
    constraint_overlap_mode: ConstraintOverlapMode,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) {
    for region in owned_regions.iter_mut() {
        region.seam_constraints = seam_constraints_for_shape(
            &region.shape,
            region.owner,
            rail_constraints,
            constraint_overlap_mode,
        );
    }
    materialize_noded_region_seam_constraints(
        owned_regions,
        footprint_shapes,
        rail_constraints,
        piece_kind,
    );
}

fn materialize_final_footprint_vertices_in_owned_regions(
    owned_regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    arrangement: &NodeOwnedRegionArrangement,
) -> bool {
    let final_footprint_points = final_footprint_point_keys(footprint_shapes);
    let final_footprint_edges = final_footprint_edge_keys(footprint_shapes);
    if final_footprint_points.is_empty() {
        return false;
    }
    let mut changed = false;
    for (region_index, region) in owned_regions.iter_mut().enumerate() {
        for contour in &mut region.shape {
            let materialized = materialized_contour_with_final_footprint_points(
                contour,
                region_index,
                &final_footprint_edges,
                arrangement,
            );
            if contour_point_keys(&materialized) == contour_point_keys(contour) {
                continue;
            }
            *contour = materialized;
            changed = true;
        }
    }
    changed
}

fn final_footprint_point_keys(footprint_shapes: &NodeOverlayShapes) -> Vec<NodeOwnershipPointKey> {
    let mut points = footprint_shapes
        .iter()
        .flat_map(|shape| shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .map(ownership_key_from_overlay_point)
        .collect::<Vec<_>>();
    points.sort_unstable();
    points.dedup();
    points
}

fn final_footprint_edge_keys(
    footprint_shapes: &NodeOverlayShapes,
) -> Vec<(NodeOwnershipPointKey, NodeOwnershipPointKey)> {
    let mut edges = Vec::new();
    for contour in footprint_shapes
        .iter()
        .flat_map(|shape| shape.iter())
        .filter(|contour| contour.len() >= 2)
    {
        for index in 0..contour.len() {
            let start = ownership_key_from_overlay_point(contour[index]);
            let end = ownership_key_from_overlay_point(contour[(index + 1) % contour.len()]);
            if start != end {
                edges.push((start, end));
            }
        }
    }
    edges
}

fn materialized_contour_with_final_footprint_points(
    contour: &NodeOverlayContour,
    region_index: usize,
    final_footprint_edges: &[(NodeOwnershipPointKey, NodeOwnershipPointKey)],
    arrangement: &NodeOwnedRegionArrangement,
) -> NodeOverlayContour {
    if contour.len() < 2 {
        return contour.clone();
    }
    let mut materialized = Vec::with_capacity(contour.len());
    for edge_index in 0..contour.len() {
        let start = ownership_key_from_overlay_point(contour[edge_index]);
        let end = ownership_key_from_overlay_point(contour[(edge_index + 1) % contour.len()]);
        if start == end {
            continue;
        }
        if !region_edge_is_exposed_final_footprint_boundary(arrangement, region_index, start, end) {
            materialized.push(start);
            continue;
        }
        let mut edge_points =
            supported_final_footprint_boundary_points_for_edge(start, end, final_footprint_edges);
        edge_points.sort_by_key(|point| segment_parameter_key(start, end, *point));
        edge_points.dedup();
        materialized.push(start);
        materialized.extend(edge_points);
    }
    dedup_materialized_contour(&mut materialized);
    materialized
        .into_iter()
        .map(overlay_point_from_key)
        .collect()
}

fn supported_final_footprint_boundary_points_for_edge(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    final_footprint_edges: &[(NodeOwnershipPointKey, NodeOwnershipPointKey)],
) -> Vec<NodeOwnershipPointKey> {
    let mut points = Vec::new();
    for (edge_start, edge_end) in final_footprint_edges {
        if !footprint_edges_overlap(start, end, *edge_start, *edge_end) {
            continue;
        }
        points.extend(
            [*edge_start, *edge_end]
                .into_iter()
                .filter(|point| *point != start && *point != end)
                .filter(|point| point_key_lies_on_segment(*point, start, end)),
        );
    }
    points.sort_by_key(|point| segment_parameter_key(start, end, *point));
    points.dedup();
    points
}

fn footprint_edges_overlap(
    left_start: NodeOwnershipPointKey,
    left_end: NodeOwnershipPointKey,
    right_start: NodeOwnershipPointKey,
    right_end: NodeOwnershipPointKey,
) -> bool {
    point_key_lies_on_segment(left_start, right_start, right_end)
        || point_key_lies_on_segment(left_end, right_start, right_end)
        || point_key_lies_on_segment(right_start, left_start, left_end)
        || point_key_lies_on_segment(right_end, left_start, left_end)
}

fn region_edge_is_exposed_final_footprint_boundary(
    arrangement: &NodeOwnedRegionArrangement,
    region_index: usize,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> bool {
    arrangement
        .edges()
        .iter()
        .filter(|edge| edge.region_index == region_index && edge.opposite_owner.is_none())
        .any(|edge| {
            let edge_start = (edge.start.x_key, edge.start.z_key);
            let edge_end = (edge.end.x_key, edge.end.z_key);
            point_key_lies_on_segment(start, edge_start, edge_end)
                && point_key_lies_on_segment(end, edge_start, edge_end)
        })
}

fn dedup_materialized_contour(points: &mut Vec<NodeOwnershipPointKey>) {
    points.dedup_by(|a, b| ownership_mm_key(*a) == ownership_mm_key(*b));
    if points.len() >= 2 && points.first().copied() == points.last().copied() {
        points.pop();
    }
    if points.len() >= 2
        && ownership_mm_key(points[0])
            == ownership_mm_key(*points.last().expect("materialized contour has last point"))
    {
        points.pop();
    }
}

fn contour_point_keys(contour: &NodeOverlayContour) -> Vec<NodeOwnershipPointKey> {
    contour
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect()
}

fn canonicalize_footprint_shapes_with_final_points(
    footprint_shapes: &mut NodeOverlayShapes,
    owned_regions: &[NodeBooleanOwnedRegion],
) {
    let mut canonical_points_by_mm = BTreeMap::<NodeOwnershipPointKey, Vec<_>>::new();
    for point in owned_regions
        .iter()
        .flat_map(|region| region.shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .map(ownership_key_from_overlay_point)
    {
        canonical_points_by_mm
            .entry(ownership_mm_key(point))
            .or_default()
            .push(point);
    }
    for candidates in canonical_points_by_mm.values_mut() {
        candidates.sort_unstable();
        candidates.dedup();
    }

    for shape in &mut *footprint_shapes {
        for contour in shape {
            for point in contour.iter_mut() {
                let key = ownership_key_from_overlay_point(*point);
                let Some(canonical) = canonical_footprint_point(key, &canonical_points_by_mm)
                else {
                    continue;
                };
                *point = overlay_point_from_key(canonical);
            }
            contour.dedup_by(|a, b| {
                ownership_key_from_overlay_point(*a) == ownership_key_from_overlay_point(*b)
            });
            if contour.len() >= 2
                && ownership_key_from_overlay_point(contour[0])
                    == ownership_key_from_overlay_point(
                        *contour.last().expect("footprint contour has last point"),
                    )
            {
                contour.pop();
            }
        }
    }
    RoadSurfaceSystem::sort_overlay_shapes(footprint_shapes);
}

fn canonical_footprint_point(
    point: NodeOwnershipPointKey,
    canonical_points_by_mm: &BTreeMap<NodeOwnershipPointKey, Vec<NodeOwnershipPointKey>>,
) -> Option<NodeOwnershipPointKey> {
    let candidates = canonical_points_by_mm.get(&ownership_mm_key(point))?;
    if candidates.binary_search(&point).is_ok() {
        return Some(point);
    }
    if candidates.len() == 1 {
        Some(candidates[0])
    } else {
        None
    }
}

impl NodeOwnedRegionArrangement {
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

#[cfg(test)]
mod tests;
