// SPDX-License-Identifier: GPL-2.0-only

//! Exact quantized point-in-contour predicates for generated contact geometry.

use super::super::NodeRailPointKey;
use super::overlay::GeneratedOverlayShapeKeys;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeneratedPointContourLocation {
    Outside,
    Boundary,
    Inside,
}

#[derive(Clone, Debug, Default)]
pub(in crate::simulation::network::surface::node::rails::contacts) struct PreparedGeneratedPointLocationContour
{
    edges: Vec<PreparedGeneratedPointLocationEdge>,
}

#[derive(Clone, Copy, Debug)]
struct PreparedGeneratedPointLocationEdge {
    start_x2: i128,
    start_z2: i128,
    end_z2: i128,
    dx: i128,
    dz: i128,
    min_x2: i128,
    min_z2: i128,
    max_x2: i128,
    max_z2: i128,
}

impl PreparedGeneratedPointLocationContour {
    pub(in crate::simulation::network::surface::node::rails::contacts) fn new(
        keys: &[NodeRailPointKey],
    ) -> Self {
        if keys.len() < 3 {
            return Self::default();
        }
        let edges = (0..keys.len())
            .map(|index| {
                PreparedGeneratedPointLocationEdge::new(keys[index], keys[(index + 1) % keys.len()])
            })
            .collect();
        Self { edges }
    }

    pub(in crate::simulation::network::surface::node::rails::contacts) fn contains_key(
        &self,
        point: NodeRailPointKey,
    ) -> bool {
        self.contains_doubled_point(i128::from(point.0) * 2, i128::from(point.1) * 2)
    }

    pub(super) fn contains_doubled_point(&self, point_x2: i128, point_z2: i128) -> bool {
        self.doubled_point_location(point_x2, point_z2) != GeneratedPointContourLocation::Outside
    }

    fn doubled_point_location(
        &self,
        point_x2: i128,
        point_z2: i128,
    ) -> GeneratedPointContourLocation {
        if self.edges.is_empty() {
            return GeneratedPointContourLocation::Outside;
        }
        let mut inside = false;
        for edge in &self.edges {
            let px = point_x2 - edge.start_x2;
            let pz = point_z2 - edge.start_z2;
            if point_x2 >= edge.min_x2
                && point_x2 <= edge.max_x2
                && point_z2 >= edge.min_z2
                && point_z2 <= edge.max_z2
                && px * edge.dz - pz * edge.dx == 0
            {
                return GeneratedPointContourLocation::Boundary;
            }
            if (edge.start_z2 > point_z2) == (edge.end_z2 > point_z2) {
                continue;
            }
            let lhs = px * edge.dz;
            let rhs = pz * edge.dx;
            let crosses = if edge.dz > 0 { lhs < rhs } else { lhs > rhs };
            if crosses {
                inside = !inside;
            }
        }
        if inside {
            GeneratedPointContourLocation::Inside
        } else {
            GeneratedPointContourLocation::Outside
        }
    }
}

impl PreparedGeneratedPointLocationEdge {
    fn new(start: NodeRailPointKey, end: NodeRailPointKey) -> Self {
        let start_x2 = i128::from(start.0) * 2;
        let start_z2 = i128::from(start.1) * 2;
        let end_x2 = i128::from(end.0) * 2;
        let end_z2 = i128::from(end.1) * 2;
        Self {
            start_x2,
            start_z2,
            end_z2,
            dx: end_x2 - start_x2,
            dz: end_z2 - start_z2,
            min_x2: start_x2.min(end_x2),
            min_z2: start_z2.min(end_z2),
            max_x2: start_x2.max(end_x2),
            max_z2: start_z2.max(end_z2),
        }
    }
}

pub(super) fn doubled_point_inside_or_on_overlay_shape_keys(
    point_x2: i128,
    point_z2: i128,
    shapes: &GeneratedOverlayShapeKeys,
) -> bool {
    for (shape, bounds) in shapes.prepared_shapes.iter().zip(&shapes.doubled_bounds) {
        let Some((min_x2, min_z2, max_x2, max_z2)) = *bounds else {
            continue;
        };
        if point_x2 < min_x2 || point_x2 > max_x2 || point_z2 < min_z2 || point_z2 > max_z2 {
            continue;
        }
        let Some((outer, holes)) = shape.split_first() else {
            continue;
        };
        let contained = match outer.doubled_point_location(point_x2, point_z2) {
            GeneratedPointContourLocation::Outside => false,
            GeneratedPointContourLocation::Boundary => true,
            GeneratedPointContourLocation::Inside => holes.iter().all(|hole| {
                hole.doubled_point_location(point_x2, point_z2)
                    != GeneratedPointContourLocation::Inside
            }),
        };
        if contained {
            return true;
        }
    }
    false
}
