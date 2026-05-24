//! Owned-region edge noding helpers.

use super::*;

pub(super) fn noded_owned_region_contour(
    contour: &NodeOverlayContour,
    global_points: &[NodeOwnershipPointKey],
) -> NodeOverlayContour {
    noded_owned_region_contour_with_edge_points(contour, |start, end| {
        noded_owned_region_edge_points(start, end, global_points)
    })
}

pub(super) fn noded_owned_region_contour_with_rail_paths(
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

pub(super) fn noded_owned_region_contour_with_edge_points(
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

pub(super) fn noded_owned_region_edge_points_with_rail_paths(
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
    noded_owned_region_source_edge_points(start, end, global_points)
}

pub(in crate::simulation::network::surface::node::ownership) fn dedup_consecutive_overlay_points(
    points: &mut NodeOverlayContour,
) {
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

fn noded_owned_region_source_edge_points(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    source_points: &[NodeOwnershipPointKey],
) -> Vec<NodeOwnershipPointKey> {
    let mut split_points = source_points
        .iter()
        .copied()
        .filter(|point| *point != start && *point != end)
        .filter(|point| point_key_lies_on_segment(*point, start, end))
        .collect::<Vec<_>>();
    split_points.sort_by_key(|point| segment_parameter_key(start, end, *point));
    split_points.dedup();

    let mut points = Vec::with_capacity(split_points.len() + 2);
    points.push(start);
    points.extend(split_points);
    points.push(end);
    points
}
