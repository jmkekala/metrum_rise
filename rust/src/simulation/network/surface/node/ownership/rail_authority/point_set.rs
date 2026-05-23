//! Canonical rail point-set lookup methods.

use super::*;

impl NodeRailCanonicalPointSet {
    pub(in crate::simulation::network::surface::node::ownership) fn owner_source_authorizes_point(
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

    pub(in crate::simulation::network::surface::node::ownership) fn canonical_conflict_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Result<Option<NodeOwnershipPointKey>, NodeBooleanOwnershipError> {
        let Some(canonical) = self.canonical_point_for_owner(owner, point)? else {
            return Ok(None);
        };
        Ok((canonical != point).then_some(canonical))
    }

    pub(in crate::simulation::network::surface::node::ownership) fn canonicalized_point_for_owner(
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
