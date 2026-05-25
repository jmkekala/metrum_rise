//! Node-local carrier provenance closure for boolean-owned region vertices.

use super::rail_authority::{NodeRailCanonicalPointSet, NodeRailSourceSegmentAuthority};
use super::topology_keys::{
    NodeOwnershipPointKey, OwnedRegionEdgeKey, ownership_key_from_overlay_point,
    ownership_key_from_road_point, ownership_mm_key, point_key_lies_on_segment,
};
use super::*;
use crate::simulation::network::surface::node::backend::road_vec3_xz;
use crate::simulation::network::surface::node::rails::{
    NodeGeneratedContourKind, NodeRailHeightCarrierPaths,
};
use i_overlay::core::fill_rule::FillRule;
use std::collections::{BTreeMap, BTreeSet};

type NodeCarrierSourceKey = (RoadSurfaceBandKind, usize, usize);
type NodeCarrierSurfaceKey = (NodeBandOwner, NodeCarrierSourceKey);

pub(super) struct NodeCarrierProvenanceContext<'a> {
    source_height_points: BTreeMap<NodeCarrierSourceKey, BTreeSet<NodeOwnershipPointKey>>,
    source_surface_shapes: BTreeMap<NodeCarrierSurfaceKey, NodeOverlayShapes>,
    rails: &'a NodeRailContourSet,
    paths_by_source: &'a BTreeMap<NodeCarrierSourceKey, NodeRailHeightCarrierPaths>,
    rail_points: &'a NodeRailCanonicalPointSet,
}

impl<'a> NodeCarrierProvenanceContext<'a> {
    pub(super) fn new(
        rails: &'a NodeRailContourSet,
        rail_points: &'a NodeRailCanonicalPointSet,
    ) -> Self {
        Self {
            source_height_points: source_height_points_by_key(rails),
            source_surface_shapes: source_surface_shapes_by_owner_source(rails),
            rails,
            paths_by_source: &rails.height_carrier_paths_by_source,
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
            point,
        )
    }

    pub(super) fn origin_for_owned_source_point(
        &self,
        owner: NodeBandOwner,
        kind: RoadSurfaceBandKind,
        source_mouth_order_index: usize,
        source_band_index: Option<usize>,
        point: NodeOwnershipPointKey,
    ) -> Result<Option<NodeCarrierProvenanceOrigin>, NodeBooleanOwnershipError> {
        let Some(source_band_index) = source_band_index else {
            return Ok(None);
        };
        provenance_origin_for_point(
            owner,
            (kind, source_mouth_order_index, source_band_index),
            point,
            &self.source_height_points,
            self.rails,
            self.paths_by_source,
            self.rail_points,
            &self.source_surface_shapes,
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
        let context = NodeCarrierProvenanceContext::new(rails, rail_points);
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

fn source_surface_shapes_by_owner_source(
    rails: &NodeRailContourSet,
) -> BTreeMap<NodeCarrierSurfaceKey, NodeOverlayShapes> {
    let mut contours_by_source = BTreeMap::<NodeCarrierSurfaceKey, Vec<NodeOverlayContour>>::new();
    for contour in &rails.contours {
        let (NodeGeneratedContourKind::Band { kind }, Some(owner), Some(source_band_index)) =
            (contour.kind, contour.owner, contour.source_band_index)
        else {
            continue;
        };
        if contour.height_points_world.is_none() {
            continue;
        }
        let source = (kind, contour.source_mouth_order_index, source_band_index);
        contours_by_source
            .entry((owner, source))
            .or_default()
            .push(overlay_contour_from_domain(contour));
    }
    contours_by_source
        .into_iter()
        .filter_map(|(key, contours)| {
            // Carrier surfaces are occupied generated bands. Self-touching generated contours can
            // contain legal lobes with mixed winding, so use the non-zero rule used for exported
            // occupied boundary loops rather than treating the source as one simple polygon.
            RoadSurfaceSystem::overlay_union_contours_with_fill_rule(&contours, FillRule::NonZero)
                .map(|mut shapes| {
                    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
                    (key, shapes)
                })
        })
        .collect()
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
    point: NodeOwnershipPointKey,
    source_height_points: &BTreeMap<NodeCarrierSourceKey, BTreeSet<NodeOwnershipPointKey>>,
    rails: &NodeRailContourSet,
    paths_by_source: &BTreeMap<NodeCarrierSourceKey, NodeRailHeightCarrierPaths>,
    rail_points: &NodeRailCanonicalPointSet,
    source_surface_shapes: &BTreeMap<NodeCarrierSurfaceKey, NodeOverlayShapes>,
) -> Result<Option<NodeCarrierProvenanceOrigin>, NodeBooleanOwnershipError> {
    if source_height_points
        .get(&source)
        .is_some_and(|points| points.contains(&point))
    {
        return Ok(Some(NodeCarrierProvenanceOrigin::SourceVertex));
    }
    match source_segment_provenance_for_point(owner, source, point, paths_by_source, rail_points) {
        Ok(Some(origin)) => return Ok(Some(origin)),
        Ok(None) => {}
        Err(_)
            if source_surface_contains_point(
                owner,
                source,
                point,
                rails,
                paths_by_source,
                source_surface_shapes,
            ) =>
        {
            return Ok(Some(NodeCarrierProvenanceOrigin::SourceSurface));
        }
        Err(error) => return Err(error),
    }
    Ok(source_surface_contains_point(
        owner,
        source,
        point,
        rails,
        paths_by_source,
        source_surface_shapes,
    )
    .then_some(NodeCarrierProvenanceOrigin::SourceSurface))
}

fn source_segment_provenance_for_point(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    point: NodeOwnershipPointKey,
    paths_by_source: &BTreeMap<NodeCarrierSourceKey, NodeRailHeightCarrierPaths>,
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<Option<NodeCarrierProvenanceOrigin>, NodeBooleanOwnershipError> {
    let mut candidates = rail_points
        .source_segments_by_owner
        .get(&owner)
        .into_iter()
        .flatten()
        .filter(|authority| authority.source == source)
        .filter_map(|authority| source_segment_authorization_candidate(point, *authority))
        .collect::<Vec<_>>();
    candidates.extend(source_path_segment_authorization_candidates(
        point,
        owner,
        source,
        paths_by_source,
    ));
    candidates.sort_unstable();
    candidates.dedup();
    if candidates.is_empty() {
        return Ok(None);
    }
    let mut canonical_points = candidates
        .iter()
        .map(|candidate| candidate.canonical_point)
        .collect::<Vec<_>>();
    canonical_points.sort_unstable();
    canonical_points.dedup();
    if let Some(candidate) = on_carrier_candidate_for_point(point, &candidates) {
        return Ok(Some(source_segment_origin(candidate)));
    }
    if canonical_points.len() == 1 {
        return Ok(Some(source_segment_origin(candidates[0])));
    }
    if let Some(candidate) = connected_endpoint_cluster_candidate(point, &candidates) {
        return Ok(Some(source_segment_origin(candidate)));
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

fn on_carrier_candidate_for_point(
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> Option<NodeSourceSegmentAuthorizationCandidate> {
    let mut on_carrier = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.canonical_point == point)
        .collect::<Vec<_>>();
    if on_carrier.is_empty()
        || !candidates
            .iter()
            .all(|candidate| projection_is_inside_same_dust_cluster(point, candidate))
    {
        return None;
    }
    on_carrier.sort_unstable();
    on_carrier.first().copied()
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

fn connected_endpoint_cluster_candidate(
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
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
        || !projection_cluster_has_connected_source_endpoint_path(&projections, candidates)
    {
        return None;
    }
    candidates
        .iter()
        .copied()
        .min_by_key(|candidate| (candidate.distance_key_units_sq, candidate.canonical_point))
}

fn projection_cluster_has_connected_source_endpoint_path(
    projections: &[NodeOwnershipPointKey],
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> bool {
    if projections.len() <= 1 {
        return true;
    }
    let mut adjacency = vec![Vec::<usize>::new(); projections.len()];
    for left_index in 0..candidates.len() {
        for right_index in left_index + 1..candidates.len() {
            let left = &candidates[left_index];
            let right = &candidates[right_index];
            if !source_segment_candidates_share_endpoint(left, right)
                && !source_segment_candidates_have_dust_endpoint_bridge(left, right)
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
            && key_distance_sq(*left_endpoint, *right_endpoint).is_some_and(|distance_sq| {
                distance_sq
                    <= SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS
                        * SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS
            })
    })
}

fn source_segment_origin(
    candidate: NodeSourceSegmentAuthorizationCandidate,
) -> NodeCarrierProvenanceOrigin {
    NodeCarrierProvenanceOrigin::SourceSegment {
        canonical_point: NodeOwnedRegionArrangementKey::from_ownership_key(
            candidate.canonical_point,
        ),
        segment_start: NodeOwnedRegionArrangementKey::from_ownership_key(candidate.segment_start),
        segment_end: NodeOwnedRegionArrangementKey::from_ownership_key(candidate.segment_end),
        distance_key_units_sq: candidate.distance_key_units_sq,
        dust_budget_key_units: candidate.dust_budget_key_units,
    }
}

fn source_path_segment_authorization_candidates(
    point: NodeOwnershipPointKey,
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    paths_by_source: &BTreeMap<NodeCarrierSourceKey, NodeRailHeightCarrierPaths>,
) -> Vec<NodeSourceSegmentAuthorizationCandidate> {
    let Some(paths) = paths_by_source.get(&source) else {
        return Vec::new();
    };
    let mut path = Vec::with_capacity(paths.start_path_world.len() + paths.end_path_world.len());
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
    let mut candidates = Vec::new();
    for segment in path.windows(2) {
        let authority = NodeRailSourceSegmentAuthority {
            owner,
            source,
            segment: OwnedRegionEdgeKey::new(segment[0], segment[1]),
        };
        if let Some(candidate) = source_segment_authorization_candidate(point, authority) {
            candidates.push(candidate);
        }
    }
    if path.len() > 2
        && let (Some(first), Some(last)) = (path.first().copied(), path.last().copied())
        && first != last
    {
        let authority = NodeRailSourceSegmentAuthority {
            owner,
            source,
            segment: OwnedRegionEdgeKey::new(first, last),
        };
        if let Some(candidate) = source_segment_authorization_candidate(point, authority) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn source_surface_contains_point(
    owner: NodeBandOwner,
    source: NodeCarrierSourceKey,
    point: NodeOwnershipPointKey,
    rails: &NodeRailContourSet,
    paths_by_source: &BTreeMap<NodeCarrierSourceKey, NodeRailHeightCarrierPaths>,
    source_surface_shapes: &BTreeMap<NodeCarrierSurfaceKey, NodeOverlayShapes>,
) -> bool {
    if paths_by_source.get(&source).is_some_and(|paths| {
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
        key_polygon_contains_point(&path, point)
    }) {
        return true;
    }
    if source_surface_shapes
        .get(&(owner, source))
        .is_some_and(|shapes| key_shapes_contain_point(shapes, point))
    {
        return true;
    }
    rails.contours.iter().any(|contour| {
        contour.owner == Some(owner)
            && contour.source_band_index == Some(source.2)
            && contour.source_mouth_order_index == source.1
            && matches!(contour.kind, NodeGeneratedContourKind::Band { kind } if kind == source.0)
            && contour.height_points_world.is_some()
            && key_polygon_contains_point(
                &contour
                    .points_xz
                    .iter()
                    .copied()
                    .map(ownership_key_from_road_point)
                    .collect::<Vec<_>>(),
                point,
            )
    })
}

fn key_shapes_contain_point(shapes: &NodeOverlayShapes, point: NodeOwnershipPointKey) -> bool {
    shapes
        .iter()
        .any(|shape| key_shape_contains_point(shape, point))
}

fn key_shape_contains_point(shape: &NodeOverlayShape, point: NodeOwnershipPointKey) -> bool {
    let Some((outer, holes)) = shape.split_first() else {
        return false;
    };
    key_polygon_contains_point(
        &outer
            .iter()
            .copied()
            .map(ownership_key_from_overlay_point)
            .collect::<Vec<_>>(),
        point,
    ) && !holes.iter().any(|hole| {
        key_polygon_contains_point(
            &hole
                .iter()
                .copied()
                .map(ownership_key_from_overlay_point)
                .collect::<Vec<_>>(),
            point,
        )
    })
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
