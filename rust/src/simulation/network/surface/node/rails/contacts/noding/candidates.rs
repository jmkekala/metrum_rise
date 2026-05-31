//! Contact contour noding candidate discovery.

use super::super::materialization::{
    GeneratedContactAuthorityIndex, generated_contact_point_has_explicit_roles,
};
use super::super::source_authority::generated_raised_step_contact_kind_for_owners;
use super::super::{
    GeneratedContourDirectedEdge, NodeGeneratedContour, NodeRailConstraint, NodeRailPointKey,
    generated_contour_band_kind, generated_contour_directed_edges, generated_contour_keys,
    generated_point_key_lies_on_segment, quantized_proper_segment_intersection,
};
use super::ContactNodingCandidate;
use std::collections::{BTreeMap, BTreeSet};

const CONTACT_NODING_BOUNDS_MARGIN_KEYS: i64 = 4096;
const CONTACT_NODING_CANDIDATE_TILE_KEYS: i64 = 8_000_000;

pub(super) fn generated_contact_contour_noding_candidates(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> Vec<ContactNodingCandidate> {
    let authority_index = GeneratedContactAuthorityIndex::new(constraints);
    let summaries = contours
        .iter()
        .map(ContactNodingContourSummary::from_contour)
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for (left_index, right_index) in contact_noding_candidate_pair_indices(&summaries) {
        let left = &contours[left_index];
        let right = &contours[right_index];
        let left_summary = &summaries[left_index];
        let right_summary = &summaries[right_index];
        candidates.extend(
            generated_contact_point_on_edge_noding_candidates(
                left,
                &left_summary.keys,
                right,
                &right_summary.keys,
                constraints,
                &authority_index,
            )
            .into_iter()
            .map(|(edge, insert_key)| (left_index, edge, insert_key)),
        );
        candidates.extend(
            generated_contact_point_on_edge_noding_candidates(
                right,
                &right_summary.keys,
                left,
                &left_summary.keys,
                constraints,
                &authority_index,
            )
            .into_iter()
            .map(|(edge, insert_key)| (right_index, edge, insert_key)),
        );
        candidates.extend(
            generated_contact_edge_intersection_noding_candidates(
                left,
                &left_summary.keys,
                right,
                &right_summary.keys,
                constraints,
                &authority_index,
            )
            .into_iter()
            .flat_map(|(left_edge, right_edge, insert_key)| {
                [
                    (left_index, left_edge, insert_key),
                    (right_index, right_edge, insert_key),
                ]
            }),
        );
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
}

fn contact_noding_candidate_pair_indices(
    summaries: &[ContactNodingContourSummary],
) -> Vec<(usize, usize)> {
    let mut indices_by_tile = BTreeMap::<ContactNodingCandidateTile, Vec<usize>>::new();
    for (summary_index, summary) in summaries.iter().enumerate() {
        if summary.owner.is_none() || !summary.bounds_valid() {
            continue;
        }
        for tile in ContactNodingCandidateTile::tiles_for_summary(summary) {
            indices_by_tile.entry(tile).or_default().push(summary_index);
        }
    }

    let mut tile_pairs = BTreeSet::<(usize, usize)>::new();
    for indices in indices_by_tile.values() {
        for left_position in 0..indices.len() {
            for right_index in indices.iter().copied().skip(left_position + 1) {
                let left_index = indices[left_position];
                let pair = if left_index <= right_index {
                    (left_index, right_index)
                } else {
                    (right_index, left_index)
                };
                tile_pairs.insert(pair);
            }
        }
    }

    tile_pairs
        .into_iter()
        .filter(|(left_index, right_index)| {
            contact_noding_summaries_can_contact(&summaries[*left_index], &summaries[*right_index])
        })
        .collect()
}

struct ContactNodingContourSummary {
    owner: Option<super::super::NodeBandOwner>,
    keys: Vec<NodeRailPointKey>,
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

impl ContactNodingContourSummary {
    fn from_contour(contour: &NodeGeneratedContour) -> Self {
        let mut keys = generated_contour_keys(contour);
        keys.sort_unstable();
        keys.dedup();
        let (mut min_x, mut min_z) = (i64::MAX, i64::MAX);
        let (mut max_x, mut max_z) = (i64::MIN, i64::MIN);
        for key in &keys {
            min_x = min_x.min(key.0);
            min_z = min_z.min(key.1);
            max_x = max_x.max(key.0);
            max_z = max_z.max(key.1);
        }
        if keys.is_empty() {
            min_x = 1;
            min_z = 1;
            max_x = 0;
            max_z = 0;
        }
        Self {
            owner: contour.owner,
            keys,
            min_x,
            min_z,
            max_x,
            max_z,
        }
    }

    fn bounds_valid(&self) -> bool {
        self.min_x <= self.max_x && self.min_z <= self.max_z
    }

    fn bounds_disjoint(&self, other: &Self) -> bool {
        self.max_x + CONTACT_NODING_BOUNDS_MARGIN_KEYS < other.min_x
            || other.max_x + CONTACT_NODING_BOUNDS_MARGIN_KEYS < self.min_x
            || self.max_z + CONTACT_NODING_BOUNDS_MARGIN_KEYS < other.min_z
            || other.max_z + CONTACT_NODING_BOUNDS_MARGIN_KEYS < self.min_z
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ContactNodingCandidateTile {
    x: i64,
    z: i64,
}

impl ContactNodingCandidateTile {
    fn tiles_for_summary(summary: &ContactNodingContourSummary) -> Vec<Self> {
        let min_tile_x = (summary.min_x - CONTACT_NODING_BOUNDS_MARGIN_KEYS)
            .div_euclid(CONTACT_NODING_CANDIDATE_TILE_KEYS);
        let max_tile_x = (summary.max_x + CONTACT_NODING_BOUNDS_MARGIN_KEYS)
            .div_euclid(CONTACT_NODING_CANDIDATE_TILE_KEYS);
        let min_tile_z = (summary.min_z - CONTACT_NODING_BOUNDS_MARGIN_KEYS)
            .div_euclid(CONTACT_NODING_CANDIDATE_TILE_KEYS);
        let max_tile_z = (summary.max_z + CONTACT_NODING_BOUNDS_MARGIN_KEYS)
            .div_euclid(CONTACT_NODING_CANDIDATE_TILE_KEYS);
        let mut tiles = Vec::new();
        for x in min_tile_x..=max_tile_x {
            for z in min_tile_z..=max_tile_z {
                tiles.push(Self { x, z });
            }
        }
        tiles
    }
}

fn contact_noding_summaries_can_contact(
    left: &ContactNodingContourSummary,
    right: &ContactNodingContourSummary,
) -> bool {
    let Some(left_owner) = left.owner else {
        return false;
    };
    let Some(right_owner) = right.owner else {
        return false;
    };
    if left_owner == right_owner || left.bounds_disjoint(right) {
        return false;
    }
    generated_raised_step_contact_kind_for_owners(left_owner, right_owner).is_some()
}

fn generated_contact_point_on_edge_noding_candidates(
    edge_contour: &NodeGeneratedContour,
    edge_keys: &[NodeRailPointKey],
    point_contour: &NodeGeneratedContour,
    point_keys: &[NodeRailPointKey],
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
) -> Vec<(GeneratedContourDirectedEdge, NodeRailPointKey)> {
    let mut candidates = Vec::new();
    for edge in generated_contour_directed_edges(edge_contour) {
        for point_key in point_keys.iter().copied() {
            if edge_keys.binary_search(&point_key).is_ok()
                || !generated_point_key_lies_on_segment(point_key, edge.start, edge.end)
                || !generated_contact_noding_point_has_explicit_roles(
                    edge_contour,
                    point_contour,
                    constraints,
                    authority_index,
                    point_key,
                )
            {
                continue;
            }
            candidates.push((edge, point_key));
        }
    }
    candidates
}

fn generated_contact_edge_intersection_noding_candidates(
    left: &NodeGeneratedContour,
    left_keys: &[NodeRailPointKey],
    right: &NodeGeneratedContour,
    right_keys: &[NodeRailPointKey],
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
) -> Vec<(
    GeneratedContourDirectedEdge,
    GeneratedContourDirectedEdge,
    NodeRailPointKey,
)> {
    let mut candidates = Vec::new();
    for left_edge in generated_contour_directed_edges(left) {
        for right_edge in generated_contour_directed_edges(right) {
            let Some(intersection) = quantized_proper_segment_intersection(
                left_edge.start,
                left_edge.end,
                right_edge.start,
                right_edge.end,
            ) else {
                continue;
            };
            if left_keys.binary_search(&intersection).is_ok()
                && right_keys.binary_search(&intersection).is_ok()
            {
                continue;
            }
            if !generated_contact_noding_point_has_explicit_roles(
                left,
                right,
                constraints,
                authority_index,
                intersection,
            ) {
                continue;
            }
            candidates.push((left_edge, right_edge, intersection));
        }
    }
    candidates
}

fn generated_contact_noding_point_has_explicit_roles(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    authority_index: &GeneratedContactAuthorityIndex<'_>,
    point: NodeRailPointKey,
) -> bool {
    let Some(left_kind) = generated_contour_band_kind(left) else {
        return false;
    };
    let Some(right_kind) = generated_contour_band_kind(right) else {
        return false;
    };
    let Some(left_owner) = left.owner else {
        return false;
    };
    let Some(right_owner) = right.owner else {
        return false;
    };
    let Some(contact_kind) = generated_raised_step_contact_kind_for_owners(left_owner, right_owner)
    else {
        return false;
    };
    generated_contact_point_has_explicit_roles(
        left_kind,
        right_kind,
        left,
        right,
        constraints,
        authority_index,
        point,
        contact_kind,
    )
}
