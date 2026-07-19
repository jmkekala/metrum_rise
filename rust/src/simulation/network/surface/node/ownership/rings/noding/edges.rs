//! Owned-region edge noding helpers.

use super::*;

#[derive(Clone, Debug)]
pub(in crate::simulation::network::surface::node::ownership) struct NodeOwnershipPointIndex {
    points_by_x: Vec<NodeOwnershipPointKey>,
    points_by_z: Vec<NodeOwnershipPointKey>,
}

impl NodeOwnershipPointIndex {
    pub(in crate::simulation::network::surface::node::ownership) fn new(
        points: &[NodeOwnershipPointKey],
    ) -> Self {
        let mut points_by_x = points.to_vec();
        points_by_x.sort_unstable();
        points_by_x.dedup();
        let mut points_by_z = points_by_x.clone();
        points_by_z.sort_unstable_by_key(|point| (point.1, point.0));
        Self {
            points_by_x,
            points_by_z,
        }
    }

    pub(in crate::simulation::network::surface::node::ownership) fn candidates_between(
        &self,
        start: NodeOwnershipPointKey,
        end: NodeOwnershipPointKey,
    ) -> &[NodeOwnershipPointKey] {
        let x_candidates = self.x_candidates_between(start, end);
        let z_candidates = self.z_candidates_between(start, end);
        if x_candidates.len() <= z_candidates.len() {
            x_candidates
        } else {
            z_candidates
        }
    }

    pub(in crate::simulation::network::surface::node::ownership) fn tolerant_candidates_between(
        &self,
        start: NodeOwnershipPointKey,
        end: NodeOwnershipPointKey,
    ) -> &[NodeOwnershipPointKey] {
        if start.0 == end.0 {
            return self.z_candidates_between(start, end);
        }
        if start.1 == end.1 {
            return self.x_candidates_between(start, end);
        }
        self.candidates_between(start, end)
    }

    fn x_candidates_between(
        &self,
        start: NodeOwnershipPointKey,
        end: NodeOwnershipPointKey,
    ) -> &[NodeOwnershipPointKey] {
        let min_x = start.0.min(end.0);
        let max_x = start.0.max(end.0);
        let x_start = self.points_by_x.partition_point(|point| point.0 < min_x);
        let x_end = self.points_by_x.partition_point(|point| point.0 <= max_x);
        &self.points_by_x[x_start..x_end]
    }

    fn z_candidates_between(
        &self,
        start: NodeOwnershipPointKey,
        end: NodeOwnershipPointKey,
    ) -> &[NodeOwnershipPointKey] {
        let min_z = start.1.min(end.1);
        let max_z = start.1.max(end.1);
        let z_start = self.points_by_z.partition_point(|point| point.1 < min_z);
        let z_end = self.points_by_z.partition_point(|point| point.1 <= max_z);
        &self.points_by_z[z_start..z_end]
    }
}

pub(super) fn noded_owned_region_contour_with_point_index(
    contour: &NodeOverlayContour,
    point_index: &NodeOwnershipPointIndex,
) -> NodeOverlayContour {
    noded_owned_region_contour_with_edge_points(contour, |start, end| {
        noded_owned_region_edge_points_with_index(start, end, point_index, true)
    })
}

pub(super) fn noded_owned_region_contour_with_rail_paths_and_point_index(
    contour: &NodeOverlayContour,
    point_index: &NodeOwnershipPointIndex,
    rail_paths: &[Vec<NodeOwnershipPointKey>],
    require_rail_path: bool,
) -> NodeOverlayContour {
    noded_owned_region_contour_with_edge_points(contour, |start, end| {
        noded_owned_region_edge_points_with_rail_paths_and_point_index(
            start,
            end,
            point_index,
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

#[cfg(test)]
pub(super) fn noded_owned_region_edge_points_with_rail_paths(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    global_points: &[NodeOwnershipPointKey],
    rail_paths: &[Vec<NodeOwnershipPointKey>],
    require_rail_path: bool,
) -> Vec<NodeOwnershipPointKey> {
    let point_index = NodeOwnershipPointIndex::new(global_points);
    noded_owned_region_edge_points_with_rail_paths_and_point_index(
        start,
        end,
        &point_index,
        rail_paths,
        require_rail_path,
    )
}

fn noded_owned_region_edge_points_with_rail_paths_and_point_index(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    point_index: &NodeOwnershipPointIndex,
    rail_paths: &[Vec<NodeOwnershipPointKey>],
    require_rail_path: bool,
) -> Vec<NodeOwnershipPointKey> {
    if let Some(points) = rail_path_points_between(start, end, rail_paths) {
        return points;
    }
    if require_rail_path {
        return vec![start, end];
    }
    noded_owned_region_edge_points_with_index(start, end, point_index, false)
}

pub(in crate::simulation::network::surface::node::ownership) fn dedup_consecutive_overlay_points(
    points: &mut NodeOverlayContour,
) {
    points.dedup_by(|a, b| {
        ownership_key_from_overlay_point(*a) == ownership_key_from_overlay_point(*b)
    });
}

fn noded_owned_region_edge_points_with_index(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    point_index: &NodeOwnershipPointIndex,
    exact_segment: bool,
) -> Vec<NodeOwnershipPointKey> {
    let candidates = if exact_segment {
        point_index.candidates_between(start, end)
    } else {
        point_index.tolerant_candidates_between(start, end)
    };
    let mut split_points = candidates
        .iter()
        .copied()
        .filter(|point| *point != start && *point != end)
        .filter(|point| {
            if exact_segment {
                point_key_lies_exactly_on_segment(*point, start, end)
            } else {
                point_key_lies_on_segment(*point, start, end)
            }
        })
        .collect::<Vec<_>>();
    split_points.sort_by_key(|point| segment_parameter_key(start, end, *point));
    split_points.dedup();

    let mut points = Vec::with_capacity(split_points.len() + 2);
    points.push(start);
    points.extend(split_points);
    points.push(end);
    points
}
