//! Canonical owned-region ring noding helpers.

use super::super::super::NodeOverlayContour;
use super::super::super::NodeOverlayShapes;
use super::super::super::arrangement::NodeBandOwner;
use super::super::super::rails::NodeGeneratedContourClaimPriority;
use super::super::rail_authority::NodeRailCanonicalPointSet;
use super::super::topology_keys::{
    NodeOwnershipPointKey, overlay_point_from_key, ownership_key_from_overlay_point,
    point_key_lies_exactly_on_segment, point_key_lies_on_segment, segment_parameter_key,
};
use super::super::{NodeBooleanOwnedRegion, NodeBooleanOwnershipError};

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_owned_region_rings(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    for region in regions.iter_mut() {
        for contour in &mut region.shape {
            *contour = noded_owned_region_contour(contour, &global_points);
        }
    }
}

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_final_owned_region_boundary_edges(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_canonical_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_owned_region_rings_with_rail_point_set(regions, rail_canonical_points)?;
    node_owned_region_rings_to_global_points(regions, footprint_shapes);
    canonicalize_owned_region_rings_with_rail_point_set(regions, rail_canonical_points)?;
    Ok(())
}

fn node_owned_region_rings_to_global_points(
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

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_owned_region_rings_with_rail_point_set(
    regions: &mut [NodeBooleanOwnedRegion],
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    if rail_points.all_points.is_empty() {
        return Ok(());
    }

    for region in regions {
        let owner_points = rail_points
            .points_by_owner
            .get(&region.owner)
            .map(Vec::as_slice)
            .unwrap_or(&rail_points.all_points);
        let source_height_points = region.source_band_index.and_then(|source_band_index| {
            rail_points.height_points_by_source.get(&(
                region.kind,
                region.source_mouth_order_index,
                source_band_index,
            ))
        });
        let mut preserved_points = source_height_points.cloned().unwrap_or_default();
        preserved_points.sort_unstable();
        preserved_points.dedup();
        let authority_points = source_height_points
            .map(Vec::as_slice)
            .unwrap_or(owner_points);
        let mut source_points = preserved_points.clone();
        for point in authority_points.iter().copied() {
            source_points.push(rail_points.canonicalized_point_for_owner(region.owner, point)?);
        }
        for point in rail_points.all_points.iter().copied() {
            source_points.push(rail_points.canonicalized_point_for_owner(region.owner, point)?);
        }
        source_points.sort_unstable();
        source_points.dedup();
        let owner_paths = if region.claim_priority == NodeGeneratedContourClaimPriority::JoinOrCap {
            rail_points
                .paths_by_owner
                .get(&region.owner)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        } else {
            &[]
        };

        for contour in &mut region.shape {
            canonicalize_owned_region_contour_to_owner_source_points(
                contour,
                region.owner,
                &preserved_points,
                rail_points,
            )?;
            *contour = noded_owned_region_contour_with_rail_paths(
                contour,
                &source_points,
                owner_paths,
                region.claim_priority == NodeGeneratedContourClaimPriority::JoinOrCap,
            );
        }
    }
    Ok(())
}

fn canonicalize_owned_region_contour_to_owner_source_points(
    contour: &mut NodeOverlayContour,
    owner: NodeBandOwner,
    source_points: &[NodeOwnershipPointKey],
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    for point in contour.iter_mut() {
        let key = ownership_key_from_overlay_point(*point);
        if source_points.binary_search(&key).is_ok() {
            continue;
        }
        let canonical = rail_points.canonicalized_point_for_owner(owner, key)?;
        if canonical == key {
            continue;
        }
        *point = overlay_point_from_key(canonical);
    }
    dedup_consecutive_overlay_points(contour);
    if contour.len() >= 2
        && ownership_key_from_overlay_point(contour[0])
            == ownership_key_from_overlay_point(*contour.last().expect("contour has last"))
    {
        contour.pop();
    }
    Ok(())
}

pub(in crate::simulation::network::surface::node::ownership) fn owned_region_global_points(
    regions: &[NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) -> Vec<NodeOwnershipPointKey> {
    let mut global_points = regions
        .iter()
        .flat_map(|region| region.shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .map(ownership_key_from_overlay_point)
        .chain(
            footprint_shapes
                .iter()
                .flat_map(|shape| shape.iter())
                .flat_map(|contour| contour.iter().copied())
                .map(ownership_key_from_overlay_point),
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
    noded_owned_region_contour_with_edge_points(contour, |start, end| {
        noded_owned_region_edge_points(start, end, global_points)
    })
}

fn noded_owned_region_contour_with_rail_paths(
    contour: &NodeOverlayContour,
    global_points: &[NodeOwnershipPointKey],
    rail_paths: &[Vec<NodeOwnershipPointKey>],
    require_rail_path: bool,
) -> NodeOverlayContour {
    noded_owned_region_contour_with_edge_points(contour, |start, end| {
        noded_owned_region_edge_points_with_rail_paths(
            start,
            end,
            global_points,
            rail_paths,
            require_rail_path,
        )
    })
}

fn noded_owned_region_contour_with_edge_points(
    contour: &NodeOverlayContour,
    mut edge_points: impl FnMut(
        NodeOwnershipPointKey,
        NodeOwnershipPointKey,
    ) -> Vec<NodeOwnershipPointKey>,
) -> NodeOverlayContour {
    if contour.len() < 2 {
        return contour.clone();
    }

    let mut noded = Vec::with_capacity(contour.len());
    for edge_index in 0..contour.len() {
        let start = ownership_key_from_overlay_point(contour[edge_index]);
        let end = ownership_key_from_overlay_point(contour[(edge_index + 1) % contour.len()]);
        if start == end {
            continue;
        }
        let points = edge_points(start, end);
        let limit = points.len().saturating_sub(1);
        noded.extend(points.into_iter().take(limit).map(overlay_point_from_key));
    }
    dedup_consecutive_overlay_points(&mut noded);
    if noded.len() >= 2
        && ownership_key_from_overlay_point(noded[0])
            == ownership_key_from_overlay_point(
                *noded.last().expect("noded contour has last point"),
            )
    {
        noded.pop();
    }
    noded
}

fn noded_owned_region_edge_points_with_rail_paths(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    global_points: &[NodeOwnershipPointKey],
    rail_paths: &[Vec<NodeOwnershipPointKey>],
    require_rail_path: bool,
) -> Vec<NodeOwnershipPointKey> {
    if let Some(points) = rail_path_points_between(start, end, rail_paths) {
        return points;
    }
    if require_rail_path {
        return vec![start, end];
    }
    noded_owned_region_edge_points(start, end, global_points)
}

pub(super) fn dedup_consecutive_overlay_points(points: &mut NodeOverlayContour) {
    points.dedup_by(|a, b| {
        ownership_key_from_overlay_point(*a) == ownership_key_from_overlay_point(*b)
    });
}

pub(in crate::simulation::network::surface::node::ownership) fn noded_owned_region_edge_points(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    global_points: &[NodeOwnershipPointKey],
) -> Vec<NodeOwnershipPointKey> {
    let mut split_points = global_points
        .iter()
        .copied()
        .filter(|point| *point != start && *point != end)
        .filter(|point| point_key_lies_exactly_on_segment(*point, start, end))
        .collect::<Vec<_>>();
    split_points.sort_by_key(|point| segment_parameter_key(start, end, *point));
    split_points.dedup();

    let mut points = Vec::with_capacity(split_points.len() + 2);
    points.push(start);
    points.extend(split_points);
    points.push(end);
    points
}

fn rail_path_points_between(
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
    fn rail_path_points_between_preserves_multiple_interior_source_vertices() {
        let path = vec![(0, 0), (1, 0), (2, 0), (3, 0)];

        assert_eq!(
            rail_path_points_between((0, 0), (3, 0), &[path]),
            Some(vec![(0, 0), (1, 0), (2, 0), (3, 0)])
        );
    }

    #[test]
    fn rail_path_points_between_prefers_longest_then_lexicographic_candidate() {
        let short = vec![(0, 0), (2, 0), (4, 0)];
        let long = vec![(0, 0), (1, 0), (2, 0), (4, 0)];
        let lexicographic = vec![(0, 0), (1, -1), (2, 0), (4, 0)];

        assert_eq!(
            rail_path_points_between((0, 0), (4, 0), &[short, long, lexicographic]),
            Some(vec![(0, 0), (1, 0), (2, 0), (4, 0)])
        );
    }

    #[test]
    fn rail_path_points_between_rejects_multi_point_detours_off_owned_edge() {
        let detour = vec![(0, 0), (1, 1), (2, 0), (4, 0)];
        let direct = vec![(0, 0), (2, 0), (4, 0)];

        assert_eq!(
            rail_path_points_between((0, 0), (4, 0), &[detour, direct]),
            Some(vec![(0, 0), (2, 0), (4, 0)])
        );
    }

    #[test]
    fn strict_rail_path_noding_does_not_use_global_points_as_join_or_cap_fallback() {
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
