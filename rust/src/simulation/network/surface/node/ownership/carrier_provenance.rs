//! Node-local carrier provenance closure for boolean-owned region vertices.

use super::rail_authority::{
    NodeRailCanonicalPointSet, NodeRailSourceSegmentAuthority, NodeRailSourceSegmentMaterialization,
};
use super::topology_keys::{
    NodeOwnershipPointKey, ownership_key_from_overlay_point, ownership_key_from_road_point,
    ownership_mm_key, point_key_lies_on_segment,
};
use super::*;
use crate::simulation::network::surface::keys::SurfaceHeightMmKey;
use crate::simulation::network::surface::node::backend::{RoadVec3, road_vec3_xz};
use crate::simulation::network::surface::node::rails::{
    NodeGeneratedContour, NodeGeneratedContourKind, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailHeightCarrierPaths,
};
use std::collections::{BTreeMap, BTreeSet};

type NodeCarrierSourceKey = (RoadSurfaceBandKind, usize, usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeCarrierPointContext {
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    height_field_id: NodeBandHeightFieldId,
}

pub(super) struct NodeCarrierProvenanceContext<'a> {
    source_height_points: BTreeMap<NodeCarrierSourceKey, BTreeSet<NodeOwnershipPointKey>>,
    final_point_contexts: BTreeMap<NodeOwnershipPointKey, Vec<NodeCarrierPointContext>>,
    rails: &'a NodeRailContourSet,
    rail_points: &'a NodeRailCanonicalPointSet,
}

impl<'a> NodeCarrierProvenanceContext<'a> {
    pub(super) fn new(
        rails: &'a NodeRailContourSet,
        rail_points: &'a NodeRailCanonicalPointSet,
    ) -> Self {
        Self {
            source_height_points: source_height_points_by_key(rails),
            final_point_contexts: BTreeMap::new(),
            rails,
            rail_points,
        }
    }

    fn with_owned_regions(
        rails: &'a NodeRailContourSet,
        rail_points: &'a NodeRailCanonicalPointSet,
        regions: &[NodeBooleanOwnedRegion],
    ) -> Self {
        Self {
            source_height_points: source_height_points_by_key(rails),
            final_point_contexts: final_point_contexts_by_key(regions),
            rails,
            rail_points,
        }
    }

    pub(super) fn origin_for_region_point(
        &self,
        region: &NodeBooleanOwnedRegion,
        point: NodeOwnershipPointKey,
    ) -> Result<Option<NodeCarrierProvenanceOrigin>, NodeBooleanOwnershipError> {
        self.origin_for_owned_source_point(
            region.owner,
            region.kind,
            region.source_mouth_order_index,
            region.source_band_index,
            region.claim_priority,
            point,
        )
    }

    pub(super) fn origin_for_owned_source_point(
        &self,
        owner: NodeBandOwner,
        kind: RoadSurfaceBandKind,
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        claim_priority: NodeGeneratedContourClaimPriority,
        point: NodeOwnershipPointKey,
    ) -> Result<Option<NodeCarrierProvenanceOrigin>, NodeBooleanOwnershipError> {
        let Some(source_band_index) = source_band_index else {
            return Ok(None);
        };
        provenance_origin_for_point(
            owner,
            (kind, source_mouth_order_index, source_band_index),
            claim_priority,
            point,
            &self.source_height_points,
            &self.final_point_contexts,
            self.rails,
            self.rail_points,
        )
    }

    pub(super) fn region_has_missing_provenance(
        &self,
        region: &NodeBooleanOwnedRegion,
    ) -> Result<bool, NodeBooleanOwnershipError> {
        for point in owned_region_support_point_keys(region) {
            if self.origin_for_region_point(region, point)?.is_none() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl NodeCarrierProvenanceClosure {
    pub(super) fn from_owned_regions(
        regions: &[NodeBooleanOwnedRegion],
        rails: &NodeRailContourSet,
        rail_points: &NodeRailCanonicalPointSet,
    ) -> Result<Self, NodeBooleanOwnershipError> {
        let context = NodeCarrierProvenanceContext::with_owned_regions(rails, rail_points, regions);
        let mut records = Vec::new();
        let mut record_keys = BTreeSet::new();

        for region in regions {
            let Some(source_band_index) = region.source_band_index else {
                continue;
            };
            let source = (
                region.kind,
                region.source_mouth_order_index,
                source_band_index,
            );
            let height_field_id = NodeBandHeightFieldId::new(source.1, source.2, region.kind);
            for point in owned_region_support_point_keys(region) {
                let Some(origin) = context.origin_for_region_point(region, point)? else {
                    return Err(NodeBooleanOwnershipError::MissingCarrierProvenance {
                        owner: region.owner,
                        point_x_key: point.0,
                        point_z_key: point.1,
                        source_kind: source.0,
                        source_mouth_order_index: source.1,
                        source_band_index: source.2,
                        height_field_id,
                    });
                };
                let record = NodeCarrierProvenanceRecord {
                    owner: region.owner,
                    source_kind: source.0,
                    source_mouth_order_index: source.1,
                    source_band_index: source.2,
                    height_field_id,
                    claim_priority: region.claim_priority,
                    point: NodeOwnedRegionArrangementKey::from_ownership_key(point),
                    origin,
                };
                if record_keys.insert(record) {
                    records.push(record);
                }
            }
        }

        records.sort_unstable();
        Ok(Self { records })
    }
}

fn source_height_points_by_key(
    rails: &NodeRailContourSet,
) -> BTreeMap<NodeCarrierSourceKey, BTreeSet<NodeOwnershipPointKey>> {
    rails
        .height_carrier_points_by_source
        .iter()
        .map(|(source, points)| {
            (
                *source,
                points
                    .iter()
                    .copied()
                    .map(|point| ownership_key_from_road_point(road_vec3_xz(point)))
                    .collect(),
            )
        })
        .collect()
}

fn final_point_contexts_by_key(
    regions: &[NodeBooleanOwnedRegion],
) -> BTreeMap<NodeOwnershipPointKey, Vec<NodeCarrierPointContext>> {
    let mut contexts_by_key =
        BTreeMap::<NodeOwnershipPointKey, Vec<NodeCarrierPointContext>>::new();
    for region in regions {
        let Some(source_band_index) = region.source_band_index else {
            continue;
        };
        let source = (
            region.kind,
            region.source_mouth_order_index,
            source_band_index,
        );
        let context = NodeCarrierPointContext {
            owner: region.owner,
            source,
            height_field_id: NodeBandHeightFieldId::new(source.1, source.2, source.0),
        };
        for point in owned_region_support_point_keys(region) {
            contexts_by_key.entry(point).or_default().push(context);
        }
    }
    for contexts in contexts_by_key.values_mut() {
        contexts.sort_unstable();
        contexts.dedup();
    }
    contexts_by_key
}

fn owned_region_support_point_keys(region: &NodeBooleanOwnedRegion) -> Vec<NodeOwnershipPointKey> {
    let mut points = region
        .shape
        .iter()
        .flat_map(|contour| contour.iter().copied())
        .map(ownership_key_from_overlay_point)
        .chain(
            region
                .seam_constraints
                .iter()
                .flat_map(|constraint| [constraint.start_xz, constraint.end_xz])
                .map(ownership_key_from_road_point),
        )
        .collect::<Vec<_>>();
    points.sort_unstable();
    points.dedup();
    points
}

fn provenance_origin_for_point(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    claim_priority: NodeGeneratedContourClaimPriority,
    point: NodeOwnershipPointKey,
    source_height_points: &BTreeMap<NodeCarrierSourceKey, BTreeSet<NodeOwnershipPointKey>>,
    final_point_contexts: &BTreeMap<NodeOwnershipPointKey, Vec<NodeCarrierPointContext>>,
    rails: &NodeRailContourSet,
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<Option<NodeCarrierProvenanceOrigin>, NodeBooleanOwnershipError> {
    if source_height_points
        .get(&source)
        .is_some_and(|points| points.contains(&point))
    {
        return Ok(Some(NodeCarrierProvenanceOrigin::SourceVertex));
    }
    if let Some(origin) =
        generated_carrier_vertex_origin(owner, source, claim_priority, point, rails)
    {
        return Ok(Some(origin));
    }
    if let Some(origin) = source_intersection_origin(
        owner,
        source,
        claim_priority,
        point,
        final_point_contexts,
        rails,
    ) {
        return Ok(Some(origin));
    }
    match source_segment_provenance_for_point(
        owner,
        source,
        claim_priority,
        point,
        source_height_points,
        rails,
        rail_points,
    ) {
        Ok(Some(origin)) => return Ok(Some(origin)),
        Ok(None) => {}
        Err(error) => return Err(error),
    }
    Ok(generated_carrier_surface_origin(
        owner,
        source,
        claim_priority,
        point,
        rails,
    ))
}

fn source_segment_provenance_for_point(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    claim_priority: NodeGeneratedContourClaimPriority,
    point: NodeOwnershipPointKey,
    source_height_points: &BTreeMap<NodeCarrierSourceKey, BTreeSet<NodeOwnershipPointKey>>,
    rails: &NodeRailContourSet,
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<Option<NodeCarrierProvenanceOrigin>, NodeBooleanOwnershipError> {
    let source_authorities = rail_points
        .source_carriers
        .source_segments_by_owner
        .get(&owner)
        .into_iter()
        .flatten()
        .filter(|authority| authority.source == source)
        .filter(|authority| {
            authority.materialization == NodeRailSourceSegmentMaterialization::DirectHeight
        })
        .copied()
        .collect::<Vec<_>>();
    let endpoint_degrees = source_segment_authority_endpoint_degrees(&source_authorities);
    let mut candidates = source_authorities
        .iter()
        .copied()
        .filter_map(|authority| source_segment_authorization_candidate(point, authority))
        .filter(|candidate| {
            source_segment_candidate_has_height_support(
                candidate,
                source,
                source_height_points,
                rails,
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    if candidates.is_empty() {
        return Ok(None);
    }
    if let Some(origin) = generated_side_join_surface_over_dust_near_source_carriers_origin(
        owner,
        source,
        claim_priority,
        point,
        &candidates,
        rails,
    ) {
        return Ok(Some(origin));
    }
    if candidates.len() == 1 {
        return Ok(Some(source_segment_origin(candidates[0])));
    }
    if let Some(candidate) = collinear_duplicate_carrier_candidate(point, &candidates) {
        return Ok(Some(source_segment_origin(candidate)));
    }
    if let Some(candidate) = source_intersection_with_connected_secondary_cluster_candidate(
        point,
        &candidates,
        &endpoint_degrees,
    ) {
        return Ok(Some(source_segment_origin(candidate)));
    }
    if let Some(candidate) =
        connected_endpoint_cluster_candidate(point, &candidates, &endpoint_degrees)
    {
        return Ok(Some(source_segment_origin(candidate)));
    }
    if let Some(candidate) =
        single_exact_same_direction_projection_noise_candidate(point, &candidates)
    {
        return Ok(Some(source_segment_origin(candidate)));
    }
    if let Some(candidate) = nearest_parallel_projection_noise_candidate(point, &candidates) {
        return Ok(Some(source_segment_origin(candidate)));
    }
    if let Some(origin) = generated_source_carrier_intersection_origin(
        owner,
        source,
        claim_priority,
        point,
        &candidates,
        rails,
    ) {
        return Ok(Some(origin));
    }
    Err(
        NodeBooleanOwnershipError::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
            owner,
            point_x_key: point.0,
            point_z_key: point.1,
            source_kind: source.0,
            source_mouth_order_index: source.1,
            source_band_index: source.2,
            candidates,
        },
    )
}

fn generated_side_join_surface_over_dust_near_source_carriers_origin(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    claim_priority: NodeGeneratedContourClaimPriority,
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
    rails: &NodeRailContourSet,
) -> Option<NodeCarrierProvenanceOrigin> {
    if rails.piece_kind != RoadSurfaceVisualNodePieceKind::JunctionN
        || claim_priority != NodeGeneratedContourClaimPriority::SideJoin
        || candidates.len() < 2
        || !candidates
            .iter()
            .all(|candidate| projection_is_inside_same_dust_cluster(point, candidate))
    {
        return None;
    }
    let mut generated_surfaces = rails.contours.iter().enumerate().filter(|(_, contour)| {
        contour.purpose == NodeGeneratedContourPurpose::JunctionSideJoin
            && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
            && generated_contour_matches_source(owner, source, contour)
            && generated_contour_boundary_contains_point(contour, point)
    });
    let (contour_index, contour) = generated_surfaces.next()?;
    if generated_surfaces.next().is_some() {
        return None;
    }
    let mut exact_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.canonical_point == point);
    let exact = exact_candidates.next();
    if exact_candidates.next().is_some()
        || exact.is_some_and(|candidate| {
            !generated_contour_contains_candidate_segment(contour, candidate)
        })
        || !candidates
            .iter()
            .filter(|candidate| Some(**candidate) != exact)
            .all(|candidate| {
                candidate_is_declared_generated_contour_segment(
                    *candidate,
                    owner,
                    source,
                    NodeGeneratedContourPurpose::CarriagewayOwnerCarrier,
                    NodeGeneratedContourClaimPriority::MouthBand,
                    rails,
                )
            })
    {
        return None;
    }
    Some(NodeCarrierProvenanceOrigin::GeneratedCarrierSurface {
        contour_index,
        purpose: contour.purpose,
        claim_priority: contour.claim_priority,
    })
}

fn candidate_is_declared_generated_contour_segment(
    candidate: NodeSourceSegmentAuthorizationCandidate,
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    purpose: NodeGeneratedContourPurpose,
    claim_priority: NodeGeneratedContourClaimPriority,
    rails: &NodeRailContourSet,
) -> bool {
    rails.contours.iter().any(|contour| {
        contour.purpose == purpose
            && contour.claim_priority == claim_priority
            && generated_contour_matches_source(owner, source, contour)
            && generated_contour_contains_candidate_segment(contour, candidate)
    })
}

fn generated_contour_contains_candidate_segment(
    contour: &NodeGeneratedContour,
    candidate: NodeSourceSegmentAuthorizationCandidate,
) -> bool {
    if indexed_path_contains_segment(
        contour.points_xz.len(),
        |index| ownership_key_from_road_point(contour.points_xz[index]),
        candidate.segment_start,
        candidate.segment_end,
        true,
    ) {
        return true;
    }
    contour.height_points_world.as_ref().is_some_and(|points| {
        indexed_path_contains_segment(
            points.len(),
            |index| ownership_key_from_road_point(road_vec3_xz(points[index])),
            candidate.segment_start,
            candidate.segment_end,
            true,
        )
    })
}

fn generated_contour_boundary_contains_point(
    contour: &NodeGeneratedContour,
    point: NodeOwnershipPointKey,
) -> bool {
    indexed_path_contains_point(
        contour.points_xz.len(),
        |index| ownership_key_from_road_point(contour.points_xz[index]),
        point,
        true,
    )
}

fn indexed_path_contains_segment(
    len: usize,
    mut key_at: impl FnMut(usize) -> NodeOwnershipPointKey,
    segment_start: NodeOwnershipPointKey,
    segment_end: NodeOwnershipPointKey,
    closed: bool,
) -> bool {
    if len < 2 {
        return false;
    }
    let first = key_at(0);
    let mut previous = first;
    for index in 1..len {
        let current = key_at(index);
        if segment_matches_keys(previous, current, segment_start, segment_end) {
            return true;
        }
        previous = current;
    }
    closed && len > 2 && segment_matches_keys(previous, first, segment_start, segment_end)
}

fn indexed_path_contains_point(
    len: usize,
    mut key_at: impl FnMut(usize) -> NodeOwnershipPointKey,
    point: NodeOwnershipPointKey,
    closed: bool,
) -> bool {
    if len < 2 {
        return false;
    }
    let first = key_at(0);
    let mut previous = first;
    for index in 1..len {
        let current = key_at(index);
        if point_key_lies_on_segment(point, previous, current) {
            return true;
        }
        previous = current;
    }
    closed && len > 2 && point_key_lies_on_segment(point, previous, first)
}

fn generated_source_carrier_intersection_origin(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    claim_priority: NodeGeneratedContourClaimPriority,
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
    rails: &NodeRailContourSet,
) -> Option<NodeCarrierProvenanceOrigin> {
    if candidates.len() < 2
        || !generated_source_surface_contains_point(owner, source, claim_priority, point, rails)
        || !candidates
            .iter()
            .all(|candidate| projection_is_inside_same_dust_cluster(point, candidate))
        || !source_carrier_intersection_heights_match(source, candidates, rails)
    {
        return None;
    }
    Some(NodeCarrierProvenanceOrigin::SourceIntersection {
        peer_count: candidates.len(),
    })
}

fn source_carrier_intersection_heights_match(
    source: NodeCarrierSourceKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
    rails: &NodeRailContourSet,
) -> bool {
    let mut height_keys = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let Some(height_m) = source_segment_candidate_height(source, candidate, rails) else {
            return false;
        };
        height_keys.push(SurfaceHeightMmKey::from_m_f64(height_m));
    }
    let Some(reference) = height_keys.first().copied() else {
        return false;
    };
    height_keys.iter().all(|height| *height == reference)
}

fn source_segment_candidate_height(
    source: NodeCarrierSourceKey,
    candidate: &NodeSourceSegmentAuthorizationCandidate,
    rails: &NodeRailContourSet,
) -> Option<f64> {
    let source_point_heights = source_point_heights_by_key(source, rails)?;
    if let Some(height_m) = height_for_keyed_segment(
        candidate.canonical_point,
        candidate.segment_start,
        candidate.segment_end,
        &source_point_heights,
    ) {
        return Some(height_m);
    }
    if let Some(paths_for_source) = rails.height_carrier_paths_by_source.get(&source)
        && let Some(height_m) = paths_for_source
            .iter()
            .find_map(|paths| height_from_source_paths(candidate, paths))
    {
        return Some(height_m);
    }
    if let Some(height_m) = height_from_source_constraints(candidate, rails, &source_point_heights)
    {
        return Some(height_m);
    }
    height_from_generated_contours(candidate, rails)
}

fn source_point_heights_by_key(
    source: NodeCarrierSourceKey,
    rails: &NodeRailContourSet,
) -> Option<BTreeMap<NodeOwnershipPointKey, f64>> {
    let mut heights_by_key = BTreeMap::new();
    for point in rails.height_carrier_points_by_source.get(&source)? {
        let key = ownership_key_from_road_point(road_vec3_xz(*point));
        if let Some(existing) = heights_by_key.get(&key).copied()
            && SurfaceHeightMmKey::from_m_f64(existing) != SurfaceHeightMmKey::from_m_f64(point.y)
        {
            return None;
        }
        heights_by_key.insert(key, point.y);
    }
    Some(heights_by_key)
}

fn height_for_keyed_segment(
    point: NodeOwnershipPointKey,
    segment_start: NodeOwnershipPointKey,
    segment_end: NodeOwnershipPointKey,
    heights_by_key: &BTreeMap<NodeOwnershipPointKey, f64>,
) -> Option<f64> {
    let start_height_m = heights_by_key.get(&segment_start).copied()?;
    let end_height_m = heights_by_key.get(&segment_end).copied()?;
    height_on_key_segment(
        point,
        segment_start,
        segment_end,
        start_height_m,
        end_height_m,
    )
}

fn height_from_source_paths(
    candidate: &NodeSourceSegmentAuthorizationCandidate,
    paths: &NodeRailHeightCarrierPaths,
) -> Option<f64> {
    let mut path = Vec::with_capacity(paths.start_path_world.len() + paths.end_path_world.len());
    path.extend(paths.start_path_world.iter().copied());
    path.extend(paths.end_path_world.iter().rev().copied());
    height_from_world_path(candidate, &path, true)
}

fn height_from_source_constraints(
    candidate: &NodeSourceSegmentAuthorizationCandidate,
    rails: &NodeRailContourSet,
    heights_by_key: &BTreeMap<NodeOwnershipPointKey, f64>,
) -> Option<f64> {
    rails
        .constraints
        .iter()
        .filter(|constraint| source_constraint_matches_candidate(candidate, constraint))
        .filter_map(|constraint| {
            let path = constraint
                .points_xz
                .iter()
                .copied()
                .map(ownership_key_from_road_point)
                .collect::<Vec<_>>();
            let segment_index = path.windows(2).position(|segment| {
                segment_matches_keys(
                    segment[0],
                    segment[1],
                    candidate.segment_start,
                    candidate.segment_end,
                )
            })?;
            height_from_supported_path_segment(
                candidate.canonical_point,
                &path,
                heights_by_key,
                segment_index,
            )
        })
        .next()
}

fn source_constraint_matches_candidate(
    candidate: &NodeSourceSegmentAuthorizationCandidate,
    constraint: &NodeRailConstraint,
) -> bool {
    constraint.kind != NodeRailConstraintKind::RaisedStepContact
        && constraint.source_mouth_order_index == candidate.source_mouth_order_index
        && constraint.source_band_index == Some(candidate.source_band_index)
        && [constraint.owner, constraint.opposite_owner]
            .contains(&Some(candidate.source_segment_id.owner))
        && candidate.source_segment_id.owner.kind() == candidate.source_kind
}

fn height_from_supported_path_segment(
    point: NodeOwnershipPointKey,
    path: &[NodeOwnershipPointKey],
    heights_by_key: &BTreeMap<NodeOwnershipPointKey, f64>,
    segment_index: usize,
) -> Option<f64> {
    let (start, start_height_m) = (0..=segment_index).rev().find_map(|candidate_index| {
        let point = path[candidate_index];
        heights_by_key
            .get(&point)
            .copied()
            .map(|height| (point, height))
    })?;
    let (end, end_height_m) = (segment_index + 1..path.len()).find_map(|candidate_index| {
        let point = path[candidate_index];
        heights_by_key
            .get(&point)
            .copied()
            .map(|height| (point, height))
    })?;
    height_on_key_segment(point, start, end, start_height_m, end_height_m)
}

fn height_from_generated_contours(
    candidate: &NodeSourceSegmentAuthorizationCandidate,
    rails: &NodeRailContourSet,
) -> Option<f64> {
    rails
        .contours
        .iter()
        .filter(|contour| {
            generated_contour_matches_source(
                candidate.source_segment_id.owner,
                (
                    candidate.source_kind,
                    candidate.source_mouth_order_index,
                    candidate.source_band_index,
                ),
                contour,
            )
        })
        .filter_map(|contour| {
            height_from_world_path(candidate, contour.height_points_world.as_deref()?, true)
        })
        .next()
}

fn height_from_world_path(
    candidate: &NodeSourceSegmentAuthorizationCandidate,
    path: &[RoadVec3],
    closed: bool,
) -> Option<f64> {
    for segment in path.windows(2) {
        if world_segment_matches_candidate(candidate, segment[0], segment[1]) {
            return height_on_world_segment(candidate.canonical_point, segment[0], segment[1]);
        }
    }
    if closed
        && path.len() > 2
        && let (Some(start), Some(end)) = (path.last().copied(), path.first().copied())
        && world_segment_matches_candidate(candidate, start, end)
    {
        return height_on_world_segment(candidate.canonical_point, start, end);
    }
    None
}

fn world_segment_matches_candidate(
    candidate: &NodeSourceSegmentAuthorizationCandidate,
    start: RoadVec3,
    end: RoadVec3,
) -> bool {
    segment_matches_keys(
        ownership_key_from_road_point(road_vec3_xz(start)),
        ownership_key_from_road_point(road_vec3_xz(end)),
        candidate.segment_start,
        candidate.segment_end,
    )
}

fn height_on_world_segment(
    point: NodeOwnershipPointKey,
    start: RoadVec3,
    end: RoadVec3,
) -> Option<f64> {
    height_on_key_segment(
        point,
        ownership_key_from_road_point(road_vec3_xz(start)),
        ownership_key_from_road_point(road_vec3_xz(end)),
        start.y,
        end.y,
    )
}

fn height_on_key_segment(
    point: NodeOwnershipPointKey,
    segment_start: NodeOwnershipPointKey,
    segment_end: NodeOwnershipPointKey,
    start_height_m: f64,
    end_height_m: f64,
) -> Option<f64> {
    if segment_start == segment_end || !point_key_lies_on_segment(point, segment_start, segment_end)
    {
        return None;
    }
    let dx = segment_end.0 - segment_start.0;
    let dz = segment_end.1 - segment_start.1;
    let denominator = if dx.abs() >= dz.abs() { dx } else { dz };
    if denominator == 0 {
        return None;
    }
    let numerator = if dx.abs() >= dz.abs() {
        point.0 - segment_start.0
    } else {
        point.1 - segment_start.1
    };
    let t = numerator as f64 / denominator as f64;
    Some(start_height_m + (end_height_m - start_height_m) * t)
}

fn source_segment_candidate_has_height_support(
    candidate: &NodeSourceSegmentAuthorizationCandidate,
    source: NodeCarrierSourceKey,
    source_height_points: &BTreeMap<NodeCarrierSourceKey, BTreeSet<NodeOwnershipPointKey>>,
    rails: &NodeRailContourSet,
) -> bool {
    source_height_points.get(&source).is_some_and(|points| {
        points.contains(&candidate.segment_start) && points.contains(&candidate.segment_end)
    }) || source_height_paths_contain_segment(
        source,
        candidate.segment_start,
        candidate.segment_end,
        rails,
    ) || generated_height_contours_contain_segment(candidate, rails)
        || source_constraints_contain_supported_segment(candidate, source_height_points, rails)
}

fn source_height_paths_contain_segment(
    source: NodeCarrierSourceKey,
    segment_start: NodeOwnershipPointKey,
    segment_end: NodeOwnershipPointKey,
    rails: &NodeRailContourSet,
) -> bool {
    let Some(paths_for_source) = rails.height_carrier_paths_by_source.get(&source) else {
        return false;
    };
    paths_for_source.iter().any(|paths| {
        let mut path =
            Vec::with_capacity(paths.start_path_world.len() + paths.end_path_world.len());
        path.extend(
            paths
                .start_path_world
                .iter()
                .copied()
                .map(|point| ownership_key_from_road_point(road_vec3_xz(point))),
        );
        path.extend(
            paths
                .end_path_world
                .iter()
                .rev()
                .copied()
                .map(|point| ownership_key_from_road_point(road_vec3_xz(point))),
        );
        path_contains_segment(&path, segment_start, segment_end, true)
    })
}

fn generated_height_contours_contain_segment(
    candidate: &NodeSourceSegmentAuthorizationCandidate,
    rails: &NodeRailContourSet,
) -> bool {
    rails.contours.iter().any(|contour| {
        generated_contour_matches_source(
            candidate.source_segment_id.owner,
            (
                candidate.source_kind,
                candidate.source_mouth_order_index,
                candidate.source_band_index,
            ),
            contour,
        ) && contour.height_points_world.as_ref().is_some_and(|points| {
            let path = points
                .iter()
                .copied()
                .map(|point| ownership_key_from_road_point(road_vec3_xz(point)))
                .collect::<Vec<_>>();
            path_contains_segment(&path, candidate.segment_start, candidate.segment_end, true)
        })
    })
}

fn source_constraints_contain_supported_segment(
    candidate: &NodeSourceSegmentAuthorizationCandidate,
    source_height_points: &BTreeMap<NodeCarrierSourceKey, BTreeSet<NodeOwnershipPointKey>>,
    rails: &NodeRailContourSet,
) -> bool {
    let source = (
        candidate.source_kind,
        candidate.source_mouth_order_index,
        candidate.source_band_index,
    );
    let Some(height_points) = source_height_points.get(&source) else {
        return false;
    };
    rails.constraints.iter().any(|constraint| {
        constraint.source_mouth_order_index == candidate.source_mouth_order_index
            && constraint.source_band_index == Some(candidate.source_band_index)
            && [constraint.owner, constraint.opposite_owner]
                .contains(&Some(candidate.source_segment_id.owner))
            && candidate.source_segment_id.owner.kind() == candidate.source_kind
            && {
                let path = constraint
                    .points_xz
                    .iter()
                    .copied()
                    .map(ownership_key_from_road_point)
                    .collect::<Vec<_>>();
                path.windows(2).enumerate().any(|(index, segment)| {
                    segment_matches_keys(
                        segment[0],
                        segment[1],
                        candidate.segment_start,
                        candidate.segment_end,
                    ) && path[..=index]
                        .iter()
                        .any(|point| height_points.contains(point))
                        && path[index + 1..]
                            .iter()
                            .any(|point| height_points.contains(point))
                })
            }
    })
}

fn path_contains_segment(
    path: &[NodeOwnershipPointKey],
    segment_start: NodeOwnershipPointKey,
    segment_end: NodeOwnershipPointKey,
    closed: bool,
) -> bool {
    if path
        .windows(2)
        .any(|segment| segment_matches_keys(segment[0], segment[1], segment_start, segment_end))
    {
        return true;
    }
    if !closed || path.len() <= 2 {
        return false;
    }
    let (Some(first), Some(last)) = (path.first().copied(), path.last().copied()) else {
        return false;
    };
    segment_matches_keys(last, first, segment_start, segment_end)
}

fn segment_matches_keys(
    left_start: NodeOwnershipPointKey,
    left_end: NodeOwnershipPointKey,
    right_start: NodeOwnershipPointKey,
    right_end: NodeOwnershipPointKey,
) -> bool {
    (left_start == right_start && left_end == right_end)
        || (left_start == right_end && left_end == right_start)
}

fn projection_is_inside_same_dust_cluster(
    point: NodeOwnershipPointKey,
    candidate: &NodeSourceSegmentAuthorizationCandidate,
) -> bool {
    ownership_mm_key(candidate.canonical_point) == ownership_mm_key(point)
        && key_distance_sq(candidate.canonical_point, point).is_some_and(|distance_sq| {
            distance_sq <= candidate.dust_budget_key_units * candidate.dust_budget_key_units
        })
}

fn collinear_duplicate_carrier_candidate(
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> Option<NodeSourceSegmentAuthorizationCandidate> {
    if !candidates
        .iter()
        .all(|candidate| projection_is_inside_same_dust_cluster(point, candidate))
    {
        return None;
    }
    let reference = candidates
        .iter()
        .filter(|candidate| candidate.canonical_point == point)
        .min_by_key(|candidate| {
            (
                candidate.distance_key_units_sq,
                candidate.canonical_point,
                candidate.source_segment_id,
            )
        })?;
    if !candidates.iter().all(|candidate| {
        source_segment_line_is_inside_dust(
            candidate.segment_start,
            reference.segment_start,
            reference.segment_end,
        ) && source_segment_line_is_inside_dust(
            candidate.segment_end,
            reference.segment_start,
            reference.segment_end,
        )
    }) {
        return None;
    }
    Some(*reference)
}

fn source_intersection_with_connected_secondary_cluster_candidate(
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
    endpoint_degrees: &BTreeMap<NodeOwnershipPointKey, usize>,
) -> Option<NodeSourceSegmentAuthorizationCandidate> {
    if !candidates
        .iter()
        .all(|candidate| projection_is_inside_same_dust_cluster(point, candidate))
    {
        return None;
    }
    let reference = candidates
        .iter()
        .filter(|candidate| candidate.canonical_point == point)
        .min_by_key(|candidate| {
            (
                candidate.distance_key_units_sq,
                candidate.canonical_point,
                candidate.source_segment_id,
            )
        })?;
    let secondary = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.source_segment_id != reference.source_segment_id)
        .collect::<Vec<_>>();
    if secondary.len() < 2
        || connected_endpoint_cluster_candidate(point, &secondary, endpoint_degrees).is_none()
    {
        return None;
    }
    Some(*reference)
}

fn source_segment_line_is_inside_dust(
    point: NodeOwnershipPointKey,
    segment_start: NodeOwnershipPointKey,
    segment_end: NodeOwnershipPointKey,
) -> bool {
    let Some((cross_sq, length_sq)) = point_line_distance_ratio(point, segment_start, segment_end)
    else {
        return false;
    };
    let dust_budget_sq = i128::from(
        SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS
            * SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS,
    );
    cross_sq <= dust_budget_sq * length_sq
}

fn point_line_distance_ratio(
    point: NodeOwnershipPointKey,
    segment_start: NodeOwnershipPointKey,
    segment_end: NodeOwnershipPointKey,
) -> Option<(i128, i128)> {
    if segment_start == segment_end {
        return None;
    }
    let dx = i128::from(segment_end.0 - segment_start.0);
    let dz = i128::from(segment_end.1 - segment_start.1);
    let length_sq = dx * dx + dz * dz;
    if length_sq == 0 {
        return None;
    }
    let px = i128::from(point.0 - segment_start.0);
    let pz = i128::from(point.1 - segment_start.1);
    let cross = px * dz - pz * dx;
    Some((cross * cross, length_sq))
}

fn connected_endpoint_cluster_candidate(
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
    endpoint_degrees: &BTreeMap<NodeOwnershipPointKey, usize>,
) -> Option<NodeSourceSegmentAuthorizationCandidate> {
    if !candidates
        .iter()
        .all(|candidate| projection_is_inside_same_dust_cluster(point, candidate))
    {
        return None;
    }
    let mut projections = candidates
        .iter()
        .map(|candidate| candidate.canonical_point)
        .collect::<Vec<_>>();
    projections.sort_unstable();
    projections.dedup();
    if projections.is_empty()
        || !projection_cluster_has_connected_source_endpoint_path(
            &projections,
            candidates,
            endpoint_degrees,
        )
    {
        return None;
    }
    candidates
        .iter()
        .copied()
        .min_by_key(|candidate| (candidate.distance_key_units_sq, candidate.canonical_point))
}

fn single_exact_same_direction_projection_noise_candidate(
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> Option<NodeSourceSegmentAuthorizationCandidate> {
    if candidates.len() < 2
        || !candidates
            .iter()
            .all(|candidate| projection_is_inside_same_dust_cluster(point, candidate))
    {
        return None;
    }

    let mut exact_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.canonical_point == point);
    let exact = exact_candidates.next()?;
    if exact_candidates.next().is_some()
        || !candidates.iter().all(|candidate| {
            source_segment_directions_have_same_general_alignment(&exact, candidate)
        })
    {
        return None;
    }

    Some(exact)
}

fn nearest_parallel_projection_noise_candidate(
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> Option<NodeSourceSegmentAuthorizationCandidate> {
    if candidates.len() < 2
        || !candidates
            .iter()
            .all(|candidate| projection_is_inside_same_dust_cluster(point, candidate))
    {
        return None;
    }

    for left_index in 0..candidates.len() {
        for right in candidates.iter().skip(left_index + 1) {
            if !source_segment_directions_are_nearly_parallel(&candidates[left_index], right) {
                return None;
            }
        }
    }

    candidates.iter().copied().min_by_key(|candidate| {
        (
            candidate.distance_key_units_sq,
            candidate.canonical_point,
            candidate.source_segment_id,
        )
    })
}

fn source_segment_directions_have_same_general_alignment(
    left: &NodeSourceSegmentAuthorizationCandidate,
    right: &NodeSourceSegmentAuthorizationCandidate,
) -> bool {
    source_segment_direction_cross_ratio(left, right).is_some_and(|cross_ratio| cross_ratio <= 0.1)
}

fn source_segment_directions_are_nearly_parallel(
    left: &NodeSourceSegmentAuthorizationCandidate,
    right: &NodeSourceSegmentAuthorizationCandidate,
) -> bool {
    source_segment_direction_cross_ratio(left, right)
        .is_some_and(|cross_ratio| cross_ratio <= 0.001)
}

fn source_segment_direction_cross_ratio(
    left: &NodeSourceSegmentAuthorizationCandidate,
    right: &NodeSourceSegmentAuthorizationCandidate,
) -> Option<f64> {
    let left_dx = i128::from(left.segment_end.0 - left.segment_start.0);
    let left_dz = i128::from(left.segment_end.1 - left.segment_start.1);
    let right_dx = i128::from(right.segment_end.0 - right.segment_start.0);
    let right_dz = i128::from(right.segment_end.1 - right.segment_start.1);
    let left_len_sq = left_dx * left_dx + left_dz * left_dz;
    let right_len_sq = right_dx * right_dx + right_dz * right_dz;
    if left_len_sq == 0 || right_len_sq == 0 {
        return None;
    }
    let cross = left_dx * right_dz - left_dz * right_dx;
    Some((cross.abs() as f64) / ((left_len_sq as f64).sqrt() * (right_len_sq as f64).sqrt()))
}

fn projection_cluster_has_connected_source_endpoint_path(
    projections: &[NodeOwnershipPointKey],
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
    endpoint_degrees: &BTreeMap<NodeOwnershipPointKey, usize>,
) -> bool {
    if projections.len() <= 1 {
        return true;
    }
    let mut adjacency = vec![Vec::<usize>::new(); projections.len()];
    for candidate in candidates {
        let Some(start_index) = projection_index_near_key(projections, candidate.segment_start)
        else {
            continue;
        };
        let Some(end_index) = projection_index_near_key(projections, candidate.segment_end) else {
            continue;
        };
        if start_index == end_index {
            continue;
        }
        adjacency[start_index].push(end_index);
        adjacency[end_index].push(start_index);
    }
    for left_index in 0..candidates.len() {
        for right_index in left_index + 1..candidates.len() {
            let left = &candidates[left_index];
            let right = &candidates[right_index];
            if !source_segment_candidates_share_endpoint(left, right)
                && !source_segment_candidates_have_dust_endpoint_bridge(
                    left,
                    right,
                    endpoint_degrees,
                )
                && !source_segment_candidates_have_projection_endpoint_bridge(
                    left,
                    right,
                    endpoint_degrees,
                )
            {
                continue;
            }
            let Some(left_projection_index) = projections
                .iter()
                .position(|projection| *projection == left.canonical_point)
            else {
                continue;
            };
            let Some(right_projection_index) = projections
                .iter()
                .position(|projection| *projection == right.canonical_point)
            else {
                continue;
            };
            if left_projection_index == right_projection_index {
                continue;
            }
            adjacency[left_projection_index].push(right_projection_index);
            adjacency[right_projection_index].push(left_projection_index);
        }
    }
    let mut visited = vec![false; projections.len()];
    let mut pending = vec![0usize];
    while let Some(index) = pending.pop() {
        if visited[index] {
            continue;
        }
        visited[index] = true;
        pending.extend(adjacency[index].iter().copied());
    }
    visited.into_iter().all(|entry| entry)
}

fn source_segment_authority_endpoint_degrees(
    authorities: &[NodeRailSourceSegmentAuthority],
) -> BTreeMap<NodeOwnershipPointKey, usize> {
    let mut endpoint_degrees = BTreeMap::new();
    for authority in authorities {
        *endpoint_degrees.entry(authority.segment.start).or_default() += 1;
        *endpoint_degrees.entry(authority.segment.end).or_default() += 1;
    }
    endpoint_degrees
}

fn projection_index_near_key(
    projections: &[NodeOwnershipPointKey],
    key: NodeOwnershipPointKey,
) -> Option<usize> {
    if let Some(index) = projections.iter().position(|projection| *projection == key) {
        return Some(index);
    }
    let dust_budget_sq = SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS
        * SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS;
    projections
        .iter()
        .enumerate()
        .filter_map(|(index, projection)| {
            let distance_sq = key_distance_sq(*projection, key)?;
            (distance_sq <= dust_budget_sq).then_some((distance_sq, index))
        })
        .min()
        .map(|(_, index)| index)
}

fn source_segment_candidates_share_endpoint(
    left: &NodeSourceSegmentAuthorizationCandidate,
    right: &NodeSourceSegmentAuthorizationCandidate,
) -> bool {
    left.segment_start == right.segment_start
        || left.segment_start == right.segment_end
        || left.segment_end == right.segment_start
        || left.segment_end == right.segment_end
}

fn source_segment_candidates_have_dust_endpoint_bridge(
    left: &NodeSourceSegmentAuthorizationCandidate,
    right: &NodeSourceSegmentAuthorizationCandidate,
    endpoint_degrees: &BTreeMap<NodeOwnershipPointKey, usize>,
) -> bool {
    [
        (left.segment_start, right.segment_start),
        (left.segment_start, right.segment_end),
        (left.segment_end, right.segment_start),
        (left.segment_end, right.segment_end),
    ]
    .iter()
    .any(|(left_endpoint, right_endpoint)| {
        left_endpoint != right_endpoint
            && endpoint_degrees
                .get(left_endpoint)
                .copied()
                .unwrap_or_default()
                > 1
            && endpoint_degrees
                .get(right_endpoint)
                .copied()
                .unwrap_or_default()
                > 1
            && key_distance_sq(*left_endpoint, *right_endpoint).is_some_and(|distance_sq| {
                distance_sq
                    <= SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS
                        * SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS
            })
    })
}

fn source_segment_candidates_have_projection_endpoint_bridge(
    left: &NodeSourceSegmentAuthorizationCandidate,
    right: &NodeSourceSegmentAuthorizationCandidate,
    endpoint_degrees: &BTreeMap<NodeOwnershipPointKey, usize>,
) -> bool {
    source_segment_candidate_projection_near_fragmented_endpoint(left, right, endpoint_degrees)
        || source_segment_candidate_projection_near_fragmented_endpoint(
            right,
            left,
            endpoint_degrees,
        )
}

fn source_segment_candidate_projection_near_fragmented_endpoint(
    endpoint_candidate: &NodeSourceSegmentAuthorizationCandidate,
    projection_candidate: &NodeSourceSegmentAuthorizationCandidate,
    endpoint_degrees: &BTreeMap<NodeOwnershipPointKey, usize>,
) -> bool {
    [
        endpoint_candidate.segment_start,
        endpoint_candidate.segment_end,
    ]
    .into_iter()
    .any(|endpoint| {
        endpoint_degrees.get(&endpoint).copied().unwrap_or_default() > 1
            && point_is_within_source_segment_dust(endpoint, projection_candidate.canonical_point)
            && closest_key_on_segment(
                endpoint,
                projection_candidate.segment_start,
                projection_candidate.segment_end,
            )
            .is_some_and(|projection| point_is_within_source_segment_dust(endpoint, projection))
    })
}

fn point_is_within_source_segment_dust(
    first: NodeOwnershipPointKey,
    second: NodeOwnershipPointKey,
) -> bool {
    let dust_budget_sq = SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS
        * SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS;
    key_distance_sq(first, second).is_some_and(|distance_sq| distance_sq <= dust_budget_sq)
}

fn source_segment_origin(
    candidate: NodeSourceSegmentAuthorizationCandidate,
) -> NodeCarrierProvenanceOrigin {
    NodeCarrierProvenanceOrigin::SourceSegment {
        source_segment_id: candidate.source_segment_id,
        canonical_point: NodeOwnedRegionArrangementKey::from_ownership_key(
            candidate.canonical_point,
        ),
        segment_start: NodeOwnedRegionArrangementKey::from_ownership_key(candidate.segment_start),
        segment_end: NodeOwnedRegionArrangementKey::from_ownership_key(candidate.segment_end),
        distance_key_units_sq: candidate.distance_key_units_sq,
        dust_budget_key_units: candidate.dust_budget_key_units,
    }
}

fn generated_carrier_vertex_origin(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    claim_priority: NodeGeneratedContourClaimPriority,
    point: NodeOwnershipPointKey,
    rails: &NodeRailContourSet,
) -> Option<NodeCarrierProvenanceOrigin> {
    rails
        .contours
        .iter()
        .enumerate()
        .filter(|(_, contour)| {
            generated_contour_matches_context(owner, source, claim_priority, contour)
        })
        .filter(|(_, contour)| generated_contour_emits_point(contour, point))
        .map(
            |(contour_index, contour)| NodeCarrierProvenanceOrigin::GeneratedCarrierVertex {
                contour_index,
                purpose: contour.purpose,
                claim_priority: contour.claim_priority,
            },
        )
        .min()
}

fn generated_carrier_surface_origin(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    claim_priority: NodeGeneratedContourClaimPriority,
    point: NodeOwnershipPointKey,
    rails: &NodeRailContourSet,
) -> Option<NodeCarrierProvenanceOrigin> {
    rails
        .contours
        .iter()
        .enumerate()
        .filter(|(_, contour)| {
            generated_contour_matches_context(owner, source, claim_priority, contour)
        })
        .filter(|(_, contour)| generated_contour_contains_point(contour, point))
        .map(
            |(contour_index, contour)| NodeCarrierProvenanceOrigin::GeneratedCarrierSurface {
                contour_index,
                purpose: contour.purpose,
                claim_priority: contour.claim_priority,
            },
        )
        .min()
}

fn generated_contour_matches_context(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    claim_priority: NodeGeneratedContourClaimPriority,
    contour: &NodeGeneratedContour,
) -> bool {
    contour.claim_priority == claim_priority
        && generated_contour_matches_source(owner, source, contour)
}

fn generated_contour_matches_source(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    contour: &NodeGeneratedContour,
) -> bool {
    contour.owner == Some(owner)
        && contour.source_band_index == Some(source.2)
        && contour.source_mouth_order_index == source.1
        && contour.height_points_world.is_some()
        && matches!(contour.kind, NodeGeneratedContourKind::Band { kind } if kind == source.0)
}

fn generated_contour_emits_point(
    contour: &NodeGeneratedContour,
    point: NodeOwnershipPointKey,
) -> bool {
    contour
        .points_xz
        .iter()
        .copied()
        .map(ownership_key_from_road_point)
        .any(|candidate| candidate == point)
        || contour.height_points_world.as_ref().is_some_and(|points| {
            points
                .iter()
                .copied()
                .map(|point| ownership_key_from_road_point(road_vec3_xz(point)))
                .any(|candidate| candidate == point)
        })
}

fn source_intersection_origin(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    claim_priority: NodeGeneratedContourClaimPriority,
    point: NodeOwnershipPointKey,
    final_point_contexts: &BTreeMap<NodeOwnershipPointKey, Vec<NodeCarrierPointContext>>,
    rails: &NodeRailContourSet,
) -> Option<NodeCarrierProvenanceOrigin> {
    let peer_count = final_point_contexts
        .get(&point)?
        .iter()
        .filter(|context| {
            context.owner != owner
                || context.source != source
                || context.height_field_id
                    != NodeBandHeightFieldId::new(source.1, source.2, source.0)
        })
        .count();
    if peer_count == 0
        || !generated_source_surface_contains_point(owner, source, claim_priority, point, rails)
    {
        return None;
    }
    Some(NodeCarrierProvenanceOrigin::SourceIntersection { peer_count })
}

fn generated_source_surface_contains_point(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    claim_priority: NodeGeneratedContourClaimPriority,
    point: NodeOwnershipPointKey,
    rails: &NodeRailContourSet,
) -> bool {
    rails.contours.iter().any(|contour| {
        generated_contour_matches_context(owner, source, claim_priority, contour)
            && generated_contour_contains_point(contour, point)
    })
}

fn generated_contour_contains_point(
    contour: &NodeGeneratedContour,
    point: NodeOwnershipPointKey,
) -> bool {
    key_polygon_contains_point(
        &contour
            .points_xz
            .iter()
            .copied()
            .map(ownership_key_from_road_point)
            .collect::<Vec<_>>(),
        point,
    )
}

fn key_polygon_contains_point(
    polygon: &[NodeOwnershipPointKey],
    point: NodeOwnershipPointKey,
) -> bool {
    if key_simple_polygon_contains_point(polygon, point) {
        return true;
    }
    key_repeated_vertex_cycles(polygon)
        .iter()
        .any(|cycle| key_simple_polygon_contains_point(cycle, point))
}

fn key_simple_polygon_contains_point(
    polygon: &[NodeOwnershipPointKey],
    point: NodeOwnershipPointKey,
) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    for edge in polygon.windows(2) {
        if point_key_lies_on_segment(point, edge[0], edge[1]) {
            return true;
        }
    }
    if let (Some(first), Some(last)) = (polygon.first().copied(), polygon.last().copied())
        && point_key_lies_on_segment(point, last, first)
    {
        return true;
    }

    let px = point.0 as f64;
    let pz = point.1 as f64;
    let mut inside = false;
    for index in 0..polygon.len() {
        let start = polygon[index];
        let end = polygon[(index + 1) % polygon.len()];
        let start_z = start.1 as f64;
        let end_z = end.1 as f64;
        if (start_z > pz) == (end_z > pz) {
            continue;
        }
        let edge_x_at_point_z =
            (end.0 - start.0) as f64 * (pz - start_z) / (end_z - start_z) + start.0 as f64;
        if px < edge_x_at_point_z {
            inside = !inside;
        }
    }
    inside
}

fn key_repeated_vertex_cycles(
    polygon: &[NodeOwnershipPointKey],
) -> Vec<Vec<NodeOwnershipPointKey>> {
    let mut cycles = Vec::new();
    for start_index in 0..polygon.len() {
        for end_index in start_index + 2..polygon.len() {
            if polygon[start_index] != polygon[end_index] {
                continue;
            }
            cycles.push(polygon[start_index..end_index].to_vec());
            let mut complement = polygon[end_index..].to_vec();
            complement.extend_from_slice(&polygon[..start_index]);
            cycles.push(complement);
        }
    }
    cycles
        .into_iter()
        .filter(|cycle| {
            let mut distinct = cycle.clone();
            distinct.sort_unstable();
            distinct.dedup();
            distinct.len() >= 3
        })
        .collect()
}

fn source_segment_authorization_candidate(
    point: NodeOwnershipPointKey,
    authority: NodeRailSourceSegmentAuthority,
) -> Option<NodeSourceSegmentAuthorizationCandidate> {
    let canonical_point =
        closest_key_on_segment(point, authority.segment.start, authority.segment.end)?;
    if !point_key_lies_on_segment(
        canonical_point,
        authority.segment.start,
        authority.segment.end,
    ) {
        return None;
    }
    if ownership_mm_key(canonical_point) != ownership_mm_key(point) {
        return None;
    }
    let distance_key_units_sq = key_distance_sq(point, canonical_point)?;
    let dust_budget_sq = SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS
        * SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS;
    if distance_key_units_sq > dust_budget_sq {
        return None;
    }
    Some(NodeSourceSegmentAuthorizationCandidate {
        source_segment_id: authority.source_segment_id,
        source_kind: authority.source.0,
        source_mouth_order_index: authority.source.1,
        source_band_index: authority.source.2,
        canonical_point,
        segment_start: authority.segment.start,
        segment_end: authority.segment.end,
        distance_key_units_sq,
        dust_budget_key_units: SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS,
    })
}

fn closest_key_on_segment(
    point: NodeOwnershipPointKey,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
) -> Option<NodeOwnershipPointKey> {
    if start == end {
        return None;
    }
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    let length_sq = dx * dx + dz * dz;
    if length_sq == 0 {
        return None;
    }
    let px = i128::from(point.0 - start.0);
    let pz = i128::from(point.1 - start.1);
    let numerator = (px * dx + pz * dz).clamp(0, length_sq);
    Some((
        round_div_i128(i128::from(start.0) * length_sq + dx * numerator, length_sq),
        round_div_i128(i128::from(start.1) * length_sq + dz * numerator, length_sq),
    ))
}

fn key_distance_sq(first: NodeOwnershipPointKey, second: NodeOwnershipPointKey) -> Option<i64> {
    let dx = i128::from(first.0 - second.0);
    let dz = i128::from(first.1 - second.1);
    let distance_sq = dx * dx + dz * dz;
    i64::try_from(distance_sq).ok()
}

fn round_div_i128(numerator: i128, denominator: i128) -> i64 {
    debug_assert!(denominator > 0);
    let half = denominator / 2;
    let rounded = if numerator >= 0 {
        (numerator + half) / denominator
    } else {
        (numerator - half) / denominator
    };
    rounded as i64
}

const SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS: i64 = 256;
