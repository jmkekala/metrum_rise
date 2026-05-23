//! Rail-path assisted edge noding helpers.

use super::*;

pub(super) fn rail_path_points_between(
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
                retain_best_rail_path_candidate(&mut best, candidate);
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
                retain_best_rail_path_candidate(&mut best, candidate);
            }
        }
    }
    best
}

fn retain_best_rail_path_candidate(
    best: &mut Option<Vec<NodeOwnershipPointKey>>,
    candidate: Vec<NodeOwnershipPointKey>,
) {
    if !rail_path_candidate_can_node_owned_edge(&candidate) {
        return;
    }
    let should_replace = best.as_ref().is_none_or(|best| {
        candidate.len() > best.len() || (candidate.len() == best.len() && candidate < *best)
    });
    if should_replace {
        *best = Some(candidate);
    }
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
        .all(|point| point_key_lies_on_segment(*point, start, end))
}

fn dedup_consecutive_ownership_keys(points: &mut Vec<NodeOwnershipPointKey>) {
    points.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub(super) fn rail_path_points_between_preserves_multiple_interior_source_vertices() {
        let path = vec![(0, 0), (1, 0), (2, 0), (3, 0)];

        assert_eq!(
            rail_path_points_between((0, 0), (3, 0), &[path]),
            Some(vec![(0, 0), (1, 0), (2, 0), (3, 0)])
        );
    }

    #[test]
    pub(super) fn rail_path_points_between_prefers_longest_then_lexicographic_candidate() {
        let short = vec![(0, 0), (2, 0), (4, 0)];
        let long = vec![(0, 0), (1, 0), (2, 0), (4, 0)];
        let lexicographic = vec![(0, 0), (1, -1), (2, 0), (4, 0)];

        assert_eq!(
            rail_path_points_between((0, 0), (4, 0), &[short, long, lexicographic]),
            Some(vec![(0, 0), (1, 0), (2, 0), (4, 0)])
        );
    }

    #[test]
    pub(super) fn rail_path_points_between_rejects_multi_point_detours_off_owned_edge() {
        let detour = vec![(0, 0), (1, 1), (2, 0), (4, 0)];
        let direct = vec![(0, 0), (2, 0), (4, 0)];

        assert_eq!(
            rail_path_points_between((0, 0), (4, 0), &[detour, direct]),
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
}
