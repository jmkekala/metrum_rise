//! Raised-step vertical face support checks against final owned top surfaces.

use super::super::{
    NodeOwnedRegion, RoadSurfaceVerticalFaceSource, arrangement::NodeBandOwner, keys, segments,
};
use crate::simulation::network::surface::{
    RoadSurfaceSystem, RoadSurfaceVisualPolygon, band_semantics::ordered_raised_step_kinds,
};
use godot::prelude::Vector3;
use std::collections::BTreeMap;

impl RoadSurfaceSystem {
    pub(super) fn retain_raised_step_faces_with_owned_top_support(
        raised_step_faces: &mut Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
        owned_regions: &[NodeOwnedRegion],
    ) {
        let top_edges = owned_top_boundary_edges(owned_regions);
        raised_step_faces.retain(|(face, source)| {
            raised_step_face_has_owned_top_support(face, *source, &top_edges)
        });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeTopSupportVertexKey {
    xz: keys::SurfaceXzKey,
    y_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeTopSupportEdgeKey {
    start: NodeTopSupportVertexKey,
    end: NodeTopSupportVertexKey,
}

#[derive(Clone, Copy, Debug)]
struct NodeTopSupportEdge {
    owner: NodeBandOwner,
    key: NodeTopSupportEdgeKey,
}

fn owned_top_boundary_edges(owned_regions: &[NodeOwnedRegion]) -> Vec<NodeTopSupportEdge> {
    let mut top_edges = Vec::new();
    for region in owned_regions {
        let owner = NodeBandOwner::new(region.kind, region.owner_index);
        let mut edge_counts = BTreeMap::<NodeTopSupportEdgeKey, usize>::new();
        for edge in visual_polygon_edges(&region.polygon) {
            if let Some(edge_key) = NodeTopSupportEdgeKey::from_points(edge[0], edge[1]) {
                *edge_counts.entry(edge_key).or_default() += 1;
            }
        }
        top_edges.extend(
            edge_counts.into_iter().filter_map(|(key, count)| {
                (count == 1).then_some(NodeTopSupportEdge { owner, key })
            }),
        );
    }
    top_edges
}

fn visual_polygon_edges(polygon: &RoadSurfaceVisualPolygon) -> Vec<[Vector3; 2]> {
    let mut edges = Vec::new();
    if polygon.triangles_world.is_empty() {
        let points = &polygon.points_world;
        if points.len() >= 2 {
            for index in 0..points.len() {
                edges.push([points[index], points[(index + 1) % points.len()]]);
            }
        }
        return edges;
    }
    for triangle in &polygon.triangles_world {
        for edge_index in 0..3 {
            edges.push([triangle[edge_index], triangle[(edge_index + 1) % 3]]);
        }
    }
    edges
}

fn raised_step_face_has_owned_top_support(
    face: &RoadSurfaceVisualPolygon,
    source: RoadSurfaceVerticalFaceSource,
    top_edges: &[NodeTopSupportEdge],
) -> bool {
    let Some((lower_owner, raised_owner)) = vertical_face_lower_and_raised_owners(source) else {
        return false;
    };
    let Some((lower_edge, upper_edge)) = vertical_face_horizontal_edge_keys(face) else {
        return false;
    };
    top_edges
        .iter()
        .any(|top_edge| top_edge.owner == lower_owner && top_edge.contains(lower_edge))
        && top_edges
            .iter()
            .any(|top_edge| top_edge.owner == raised_owner && top_edge.contains(upper_edge))
}

fn vertical_face_lower_and_raised_owners(
    source: RoadSurfaceVerticalFaceSource,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    let segment = source.segment();
    let owner = segment.owner();
    let opposite_owner = segment.opposite_owner();
    let (lower_kind, _) = ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
    Some(if owner.kind() == lower_kind {
        (owner, opposite_owner)
    } else {
        (opposite_owner, owner)
    })
}

fn vertical_face_horizontal_edge_keys(
    face: &RoadSurfaceVisualPolygon,
) -> Option<(NodeTopSupportEdgeKey, NodeTopSupportEdgeKey)> {
    let [a, b, c, d] = face.points_world.as_slice() else {
        return None;
    };
    let first = NodeTopSupportEdgeKey::from_points(*a, *d)?;
    let second = NodeTopSupportEdgeKey::from_points(*b, *c)?;
    let first_avg_y_mm = first.start.y_mm + first.end.y_mm;
    let second_avg_y_mm = second.start.y_mm + second.end.y_mm;
    Some(if first_avg_y_mm <= second_avg_y_mm {
        (first, second)
    } else {
        (second, first)
    })
}

impl NodeTopSupportEdgeKey {
    fn from_points(start: Vector3, end: Vector3) -> Option<Self> {
        let start = NodeTopSupportVertexKey::from_point(start);
        let end = NodeTopSupportVertexKey::from_point(end);
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }
}

impl NodeTopSupportVertexKey {
    fn from_point(point: Vector3) -> Self {
        Self {
            xz: keys::SurfaceXzKey::from_godot_world_xz(point),
            y_mm: keys::SurfaceHeightMmKey::from_m_f32(point.y).as_i64(),
        }
    }
}

impl NodeTopSupportEdge {
    fn contains(self, candidate: NodeTopSupportEdgeKey) -> bool {
        self.contains_vertex(candidate.start) && self.contains_vertex(candidate.end)
    }

    fn contains_vertex(self, vertex: NodeTopSupportVertexKey) -> bool {
        if !segments::key_lies_exactly_on_segment(vertex.xz, self.key.start.xz, self.key.end.xz) {
            return false;
        }
        let Some(parameter) =
            segments::exact_line_parameter(vertex.xz, self.key.start.xz, self.key.end.xz)
        else {
            return false;
        };
        let expected_y_mm =
            segments::interpolate_height_i64(self.key.start.y_mm, self.key.end.y_mm, parameter);
        vertex.y_mm == expected_y_mm
    }
}
