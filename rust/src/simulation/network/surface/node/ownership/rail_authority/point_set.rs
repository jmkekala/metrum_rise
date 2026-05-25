//! Canonical rail point-set lookup methods.

use super::*;

impl NodeRailCanonicalPointSet {
    #[cfg(test)]
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

    #[cfg(test)]
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

    #[cfg(test)]
    pub(in crate::simulation::network::surface::node::ownership) fn source_authorizes_same_mm_duplicate_cluster(
        &self,
        point: NodeOwnershipPointKey,
        source_points: &[NodeOwnershipPointKey],
    ) -> bool {
        source_points_same_mm_candidates(point, source_points).len() >= 2
    }

    pub(in crate::simulation::network::surface::node::ownership) fn source_canonicalized_point_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
        source: Option<NodeRailHeightSourceKey>,
        source_points: &[NodeOwnershipPointKey],
    ) -> Result<Option<NodeOwnershipPointKey>, NodeBooleanOwnershipError> {
        if !self.owner_has_ambiguous_same_mm_candidates(owner, point) {
            return Ok(None);
        }
        if let Some(source) = source {
            if let Some(canonical) =
                self.source_segment_canonicalized_point_for_owner(owner, point, source)?
            {
                return Ok(Some(canonical));
            }
        }
        let candidates = source_points_same_mm_candidates(point, source_points);
        if candidates.len() >= 2 {
            return Ok(Some(point));
        }
        Ok(None)
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

    fn owner_has_ambiguous_same_mm_candidates(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> bool {
        self.canonical_candidates_for_owner(owner, point)
            .is_some_and(|candidates| candidates.len() > 1 && !candidates.contains(&point))
    }

    fn source_segment_canonicalized_point_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
        source: NodeRailHeightSourceKey,
    ) -> Result<Option<NodeOwnershipPointKey>, NodeBooleanOwnershipError> {
        let Some(authorities) = self.source_segments_by_owner.get(&owner) else {
            return Ok(None);
        };
        let mut candidates = authorities
            .iter()
            .filter(|authority| authority.source == source)
            .filter_map(|authority| source_segment_authorization_candidate(point, *authority))
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        candidates.dedup();
        let mut canonical_points = candidates
            .iter()
            .map(|candidate| candidate.canonical_point)
            .collect::<Vec<_>>();
        canonical_points.sort_unstable();
        canonical_points.dedup();
        match canonical_points.as_slice() {
            [] => Ok(None),
            [canonical] => Ok(Some(*canonical)),
            _ if segment_projection_cluster_contains_point(point, &candidates)
                && projection_cluster_is_source_endpoint_backed(&candidates) =>
            {
                Ok(Some(point))
            }
            _ if let Some(canonical) =
                source_endpoint_projection_cluster_canonical_point(point, &candidates) =>
            {
                Ok(Some(canonical))
            }
            _ => Err(
                NodeBooleanOwnershipError::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
                    owner,
                    point_x_key: point.0,
                    point_z_key: point.1,
                    source_kind: source.0,
                    source_mouth_order_index: source.1,
                    source_band_index: source.2,
                    candidates,
                },
            ),
        }
    }
}

fn source_points_same_mm_candidates(
    point: NodeOwnershipPointKey,
    source_points: &[NodeOwnershipPointKey],
) -> Vec<NodeOwnershipPointKey> {
    let point_mm_key = ownership_mm_key(point);
    let mut candidates = source_points
        .iter()
        .copied()
        .filter(|candidate| ownership_mm_key(*candidate) == point_mm_key)
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    candidates
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
        source_segment_id: source_carrier_segment_id(
            authority.owner,
            authority.source,
            authority.segment,
        ),
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

fn segment_projection_cluster_contains_point(
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> bool {
    candidates
        .iter()
        .any(|candidate| candidate.canonical_point == point)
        && segment_projection_cluster_is_within_dust_budget(point, candidates)
}

fn segment_projection_cluster_is_within_dust_budget(
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> bool {
    let dust_budget_sq = SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS
        * SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS;
    candidates.iter().all(|candidate| {
        ownership_mm_key(candidate.canonical_point) == ownership_mm_key(point)
            && key_distance_sq(candidate.canonical_point, point)
                .is_some_and(|distance_sq| distance_sq <= dust_budget_sq)
    })
}

fn source_endpoint_projection_cluster_canonical_point(
    point: NodeOwnershipPointKey,
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> Option<NodeOwnershipPointKey> {
    if !segment_projection_cluster_is_within_dust_budget(point, candidates)
        || !projection_cluster_is_source_endpoint_backed(candidates)
    {
        return None;
    }
    candidates
        .iter()
        .map(|candidate| (candidate.distance_key_units_sq, candidate.canonical_point))
        .min()
        .map(|(_, canonical)| canonical)
}

fn projection_cluster_is_source_endpoint_backed(
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> bool {
    let mut projections = candidates
        .iter()
        .map(|candidate| candidate.canonical_point)
        .collect::<Vec<_>>();
    projections.sort_unstable();
    projections.dedup();
    if projections.is_empty() {
        return false;
    }
    projection_cluster_has_connected_source_endpoint_path(&projections, candidates)
}

fn projection_cluster_has_connected_source_endpoint_path(
    projections: &[NodeOwnershipPointKey],
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> bool {
    if projections.len() <= 1 {
        return true;
    }
    let mut adjacency = vec![Vec::<usize>::new(); projections.len()];
    let endpoint_degrees = source_segment_endpoint_degrees(candidates);
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
            if left_projection_index == right_projection_index
                || (!source_segment_candidates_share_endpoint(left, right)
                    && !source_segment_candidates_have_dust_endpoint_bridge(
                        left,
                        right,
                        &endpoint_degrees,
                    )
                    && !source_segment_candidates_have_projection_endpoint_bridge(
                        left,
                        right,
                        &endpoint_degrees,
                    ))
            {
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

fn source_segment_endpoint_degrees(
    candidates: &[NodeSourceSegmentAuthorizationCandidate],
) -> BTreeMap<NodeOwnershipPointKey, usize> {
    let mut endpoint_degrees = BTreeMap::new();
    for candidate in candidates {
        *endpoint_degrees.entry(candidate.segment_start).or_default() += 1;
        *endpoint_degrees.entry(candidate.segment_end).or_default() += 1;
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
    let endpoints = [
        (left.segment_start, right.segment_start),
        (left.segment_start, right.segment_end),
        (left.segment_end, right.segment_start),
        (left.segment_end, right.segment_end),
    ];
    let dust_budget_sq = SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS
        * SOURCE_SEGMENT_AUTHORIZATION_DUST_BUDGET_UNITS;
    endpoints.iter().any(|(left_endpoint, right_endpoint)| {
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
            && key_distance_sq(*left_endpoint, *right_endpoint)
                .is_some_and(|distance_sq| distance_sq <= dust_budget_sq)
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
