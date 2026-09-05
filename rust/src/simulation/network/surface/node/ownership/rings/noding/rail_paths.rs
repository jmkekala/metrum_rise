//! Rail-path assisted edge noding helpers.

use super::*;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::Range;

pub(super) struct PreparedRailPaths<'a> {
    paths: &'a [Vec<NodeOwnershipPointKey>],
    path_has_consecutive_duplicates: Vec<bool>,
    occurrences: Vec<(NodeOwnershipPointKey, usize, usize)>,
    occurrence_ranges: HashMap<NodeOwnershipPointKey, Range<usize>>,
}

impl<'a> PreparedRailPaths<'a> {
    pub(super) fn new(paths: &'a [Vec<NodeOwnershipPointKey>]) -> Self {
        let occurrence_capacity = paths
            .iter()
            .filter(|points| points.len() >= 3)
            .map(Vec::len)
            .sum();
        let mut occurrences = Vec::with_capacity(occurrence_capacity);
        let mut path_has_consecutive_duplicates = vec![false; paths.len()];
        for (path_index, points) in paths.iter().enumerate() {
            if points.len() < 3 {
                continue;
            }
            path_has_consecutive_duplicates[path_index] =
                points.windows(2).any(|pair| pair[0] == pair[1]);
            occurrences.extend(
                points
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(point_index, point)| (point, path_index, point_index)),
            );
        }
        occurrences.sort_unstable();
        let mut occurrence_ranges = HashMap::new();
        let mut start = 0;
        while start < occurrences.len() {
            let point = occurrences[start].0;
            let end = start
                + occurrences[start..].partition_point(|(candidate, _, _)| *candidate == point);
            occurrence_ranges.insert(point, start..end);
            start = end;
        }
        Self {
            paths,
            path_has_consecutive_duplicates,
            occurrences,
            occurrence_ranges,
        }
    }

    fn occurrences_for(
        &self,
        point: NodeOwnershipPointKey,
    ) -> &[(NodeOwnershipPointKey, usize, usize)] {
        self.occurrence_ranges
            .get(&point)
            .map(|range| &self.occurrences[range.clone()])
            .unwrap_or(&[])
    }
}

pub(super) fn rail_path_points_between_into(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_paths: &PreparedRailPaths<'_>,
    best: &mut Vec<NodeOwnershipPointKey>,
    candidate: &mut Vec<NodeOwnershipPointKey>,
) -> bool {
    best.clear();
    candidate.clear();
    if start == end {
        return false;
    }
    let mut found = false;
    let start_occurrences = rail_paths.occurrences_for(start);
    if start_occurrences.is_empty() {
        return false;
    }
    let end_occurrences = rail_paths.occurrences_for(end);
    let (mut start_offset, mut end_offset) = (0, 0);
    while start_offset < start_occurrences.len() && end_offset < end_occurrences.len() {
        let start_path_index = start_occurrences[start_offset].1;
        let end_path_index = end_occurrences[end_offset].1;
        match start_path_index.cmp(&end_path_index) {
            Ordering::Less => {
                start_offset += 1;
            }
            Ordering::Greater => {
                end_offset += 1;
            }
            Ordering::Equal => {
                let start_group_end = start_offset
                    + start_occurrences[start_offset..]
                        .partition_point(|(_, path_index, _)| *path_index == start_path_index);
                let end_group_end = end_offset
                    + end_occurrences[end_offset..]
                        .partition_point(|(_, path_index, _)| *path_index == end_path_index);
                for &(_, _, start_index) in &start_occurrences[start_offset..start_group_end] {
                    for &(_, _, end_index) in &end_occurrences[end_offset..end_group_end] {
                        if start_index.abs_diff(end_index) <= 1 {
                            continue;
                        }
                        let points = &rail_paths.paths[start_path_index];
                        let path_slice = if start_index < end_index {
                            &points[start_index..=end_index]
                        } else {
                            &points[end_index..=start_index]
                        };
                        if !rail_paths.path_has_consecutive_duplicates[start_path_index] {
                            if !rail_path_candidate_can_node_owned_edge(path_slice) {
                                continue;
                            }
                            let candidate_len = path_slice.len();
                            let lexicographically_before_best = if start_index < end_index {
                                path_slice < best.as_slice()
                            } else {
                                path_slice
                                    .iter()
                                    .rev()
                                    .copied()
                                    .cmp(best.iter().copied())
                                    .is_lt()
                            };
                            let should_replace = !found
                                || candidate_len > best.len()
                                || (candidate_len == best.len() && lexicographically_before_best);
                            if should_replace {
                                best.clear();
                                if start_index < end_index {
                                    best.extend_from_slice(path_slice);
                                } else {
                                    best.extend(path_slice.iter().rev().copied());
                                }
                                found = true;
                            }
                            continue;
                        }
                        candidate.clear();
                        if start_index < end_index {
                            candidate.extend_from_slice(path_slice);
                        } else {
                            candidate.extend(path_slice.iter().rev().copied());
                        }
                        dedup_consecutive_ownership_keys(candidate);
                        if !rail_path_candidate_can_node_owned_edge(candidate) {
                            continue;
                        }
                        let should_replace = !found
                            || candidate.len() > best.len()
                            || (candidate.len() == best.len() && candidate < best);
                        if should_replace {
                            std::mem::swap(best, candidate);
                            found = true;
                        }
                    }
                }
                start_offset = start_group_end;
                end_offset = end_group_end;
            }
        }
    }
    found
}

fn rail_path_candidate_can_node_owned_edge(candidate: &[NodeOwnershipPointKey]) -> bool {
    if candidate.len() < 3 {
        return false;
    }
    if candidate.len() == 3 {
        return true;
    }
    let start = candidate[0];
    let end = *candidate
        .last()
        .expect("candidate length was checked above");
    candidate[1..candidate.len() - 1]
        .iter()
        .all(|point| point_key_lies_exactly_on_segment(*point, start, end))
}

fn dedup_consecutive_ownership_keys(points: &mut Vec<NodeOwnershipPointKey>) {
    points.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared_path_points_between(
        start: NodeOwnershipPointKey,
        end: NodeOwnershipPointKey,
        paths: &[Vec<NodeOwnershipPointKey>],
    ) -> Option<Vec<NodeOwnershipPointKey>> {
        let mut points = Vec::new();
        let mut candidate = Vec::new();
        rail_path_points_between_into(
            start,
            end,
            &PreparedRailPaths::new(paths),
            &mut points,
            &mut candidate,
        )
        .then_some(points)
    }

    #[test]
    pub(super) fn rail_path_points_between_preserves_multiple_interior_source_vertices() {
        let path = vec![(0, 0), (1, 0), (2, 0), (3, 0)];

        assert_eq!(
            prepared_path_points_between((0, 0), (3, 0), &[path]),
            Some(vec![(0, 0), (1, 0), (2, 0), (3, 0)])
        );
    }

    #[test]
    pub(super) fn rail_path_points_between_prefers_longest_then_lexicographic_candidate() {
        let short = vec![(0, 0), (2, 0), (4, 0)];
        let long = vec![(0, 0), (1, 0), (2, 0), (4, 0)];
        let lexicographic = vec![(0, 0), (1, -1), (2, 0), (4, 0)];

        assert_eq!(
            prepared_path_points_between((0, 0), (4, 0), &[short, long, lexicographic]),
            Some(vec![(0, 0), (1, 0), (2, 0), (4, 0)])
        );
    }

    #[test]
    pub(super) fn rail_path_points_between_rejects_multi_point_detours_off_owned_edge() {
        let detour = vec![(0, 0), (1, 1), (2, 0), (4, 0)];
        let direct = vec![(0, 0), (2, 0), (4, 0)];

        assert_eq!(
            prepared_path_points_between((0, 0), (4, 0), &[detour, direct]),
            Some(vec![(0, 0), (2, 0), (4, 0)])
        );
    }

    #[test]
    fn strict_rail_path_noding_does_not_use_global_points_as_join_or_cap_substitute() {
        let global_points = vec![(2, 0)];

        assert_eq!(
            noded_owned_region_edge_points_with_rail_paths(
                (0, 0),
                (4, 0),
                &global_points,
                &[],
                true
            ),
            vec![(0, 0), (4, 0)]
        );
    }

    #[test]
    fn non_strict_rail_path_noding_still_uses_canonical_global_points() {
        let global_points = vec![(2, 0)];

        assert_eq!(
            noded_owned_region_edge_points_with_rail_paths(
                (0, 0),
                (4, 0),
                &global_points,
                &[],
                false
            ),
            vec![(0, 0), (2, 0), (4, 0)]
        );
    }

    #[test]
    fn non_strict_point_index_preserves_overlay_grid_tolerance_on_axis_edges() {
        assert_eq!(
            noded_owned_region_edge_points_with_rail_paths((0, 0), (0, 10), &[(1, 5)], &[], false),
            vec![(0, 0), (1, 5), (0, 10)]
        );
        assert_eq!(
            noded_owned_region_edge_points_with_rail_paths((0, 0), (10, 0), &[(5, 1)], &[], false),
            vec![(0, 0), (5, 1), (10, 0)]
        );
    }
}
