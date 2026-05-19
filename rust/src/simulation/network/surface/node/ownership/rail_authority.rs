//! Rail-source authority indexing for node boolean ownership.

use super::super::RoadSurfaceBandKind;
use super::super::arrangement::NodeBandOwner;
use super::super::rails::{NodeGeneratedContourKind, NodeRailConstraint, NodeRailContourSet};
use super::topology_keys::{
    NodeOwnershipPointKey, OwnedRegionEdgeKey, ownership_key_from_overlay_point,
    ownership_key_from_road_point, ownership_mm_key, point_key_lies_on_segment,
};
use super::{NodeBooleanOwnedRegion, NodeBooleanOwnershipError};
use std::collections::{BTreeMap, BTreeSet};

const SOURCE_DUPLICATE_CLUSTER_MAX_SPAN_UNITS: i64 = 32;

pub(super) struct NodeRailCanonicalPointSet {
    pub(super) all_points: Vec<NodeOwnershipPointKey>,
    pub(super) points_by_owner: BTreeMap<NodeBandOwner, Vec<NodeOwnershipPointKey>>,
    pub(super) segments_by_owner: BTreeMap<NodeBandOwner, Vec<OwnedRegionEdgeKey>>,
    pub(super) canonical_points_by_mm_key_by_owner:
        BTreeMap<NodeBandOwner, BTreeMap<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>>,
    pub(super) height_points_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<NodeOwnershipPointKey>>,
    pub(super) paths_by_owner: BTreeMap<NodeBandOwner, Vec<Vec<NodeOwnershipPointKey>>>,
}

pub(super) fn canonical_points_for_rail_set(
    rails: &NodeRailContourSet,
) -> NodeRailCanonicalPointSet {
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
        .map(ownership_key_from_road_point)
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
            .map(|point| ownership_key_from_road_point(super::super::backend::road_vec3_xz(point)))
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
            .extend(
                constraint
                    .points_xz
                    .iter()
                    .copied()
                    .map(ownership_key_from_road_point),
            );
    }
    for contour in &rails.contours {
        let path = contour
            .points_xz
            .iter()
            .copied()
            .map(ownership_key_from_road_point)
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
            .map(ownership_key_from_road_point)
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

pub(super) fn insert_open_source_segments(
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

pub(super) fn canonical_points_by_mm_key_by_owner(
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

pub(super) fn validate_owned_region_vertices_against_source_authority(
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
            for point in contour
                .iter()
                .copied()
                .map(ownership_key_from_overlay_point)
            {
                if source_height_points.binary_search(&point).is_ok() {
                    continue;
                }
                if rail_points.owner_source_authorizes_point(region.owner, point)? {
                    continue;
                }
                let Some(canonical) =
                    rail_points.canonical_conflict_for_owner(region.owner, point)?
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
    ) -> Result<bool, NodeBooleanOwnershipError> {
        if self.owner_source_points_authorize_point(owner, point)
            || self.owner_source_segments_authorize_point(owner, point)
        {
            return Ok(true);
        }
        Ok(self.canonical_conflict_for_owner(owner, point)?.is_none())
    }

    fn owner_source_points_authorize_point(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> bool {
        self.points_by_owner
            .get(&owner)
            .is_some_and(|points| points.binary_search(&point).is_ok())
    }

    fn owner_source_segments_authorize_point(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> bool {
        self.segments_by_owner.get(&owner).is_some_and(|segments| {
            segments
                .iter()
                .any(|segment| point_key_lies_on_segment(point, segment.start, segment.end))
        })
    }

    fn canonical_conflict_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Result<Option<NodeOwnershipPointKey>, NodeBooleanOwnershipError> {
        let Some(canonical) = self.canonical_point_for_owner(owner, point)? else {
            return Ok(None);
        };
        Ok((canonical != point).then_some(canonical))
    }

    pub(super) fn canonicalized_point_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Result<NodeOwnershipPointKey, NodeBooleanOwnershipError> {
        if self.owner_source_points_authorize_point(owner, point) {
            return Ok(point);
        }
        match self.canonical_point_for_owner(owner, point) {
            Ok(Some(canonical)) => Ok(canonical),
            Ok(None) => Ok(point),
            Err(error) => {
                if self.owner_source_segments_authorize_point(owner, point) {
                    Ok(point)
                } else {
                    Err(error)
                }
            }
        }
    }

    fn canonical_point_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Result<Option<NodeOwnershipPointKey>, NodeBooleanOwnershipError> {
        let Some(candidates) = self.canonical_candidates_for_owner(owner, point) else {
            return Ok(None);
        };
        if candidates.contains(&point) {
            return Ok(Some(point));
        }
        if candidates.len() == 1 {
            return Ok(candidates.iter().copied().next());
        }
        if canonical_candidates_form_source_duplicate_cluster(point, candidates) {
            return Ok(Some(point));
        }
        Err(
            NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex {
                owner,
                point_x_key: point.0,
                point_z_key: point.1,
                candidates: candidates.iter().copied().collect(),
            },
        )
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

fn canonical_candidates_form_source_duplicate_cluster(
    point: NodeOwnershipPointKey,
    candidates: &BTreeSet<NodeOwnershipPointKey>,
) -> bool {
    if candidates.len() < 2 {
        return false;
    }
    let point_mm_key = ownership_mm_key(point);
    if candidates
        .iter()
        .any(|candidate| ownership_mm_key(*candidate) != point_mm_key)
    {
        return false;
    }
    let min_x = candidates
        .iter()
        .map(|candidate| candidate.0)
        .min()
        .unwrap_or(point.0);
    let max_x = candidates
        .iter()
        .map(|candidate| candidate.0)
        .max()
        .unwrap_or(point.0);
    let min_z = candidates
        .iter()
        .map(|candidate| candidate.1)
        .min()
        .unwrap_or(point.1);
    let max_z = candidates
        .iter()
        .map(|candidate| candidate.1)
        .max()
        .unwrap_or(point.1);
    max_x - min_x <= SOURCE_DUPLICATE_CLUSTER_MAX_SPAN_UNITS
        && max_z - min_z <= SOURCE_DUPLICATE_CLUSTER_MAX_SPAN_UNITS
}

pub(super) fn constraint_authority_owners(constraint: &NodeRailConstraint) -> Vec<NodeBandOwner> {
    let mut owners = [constraint.owner, constraint.opposite_owner]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners.dedup();
    owners
}
