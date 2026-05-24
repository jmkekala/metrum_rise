//! Raised-step vertical face support checks against final owned top surfaces.

use super::super::{
    ArrangementBoundaryPointKey, NodeOwnedRegion, NodeTopSurfacePolygonSource,
    RoadSurfaceRaisedStepFace, RoadSurfaceVerticalFaceSource, arrangement::NodeBandOwner, keys,
    segments,
};
use crate::simulation::network::surface::{
    RoadSurfaceSystem, band_semantics::ordered_raised_step_kinds,
};
use std::collections::BTreeMap;

impl RoadSurfaceSystem {
    pub(super) fn retain_raised_step_faces_with_owned_top_support(
        raised_step_faces: &mut Vec<RoadSurfaceRaisedStepFace>,
        owned_regions: &[NodeOwnedRegion],
        node_top_surface_sources: &[NodeTopSurfacePolygonSource],
    ) {
        let top_edges = owned_top_boundary_edges(owned_regions, node_top_surface_sources);
        raised_step_faces.retain(|face| raised_step_face_has_owned_top_support(face, &top_edges));
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

fn owned_top_boundary_edges(
    owned_regions: &[NodeOwnedRegion],
    node_top_surface_sources: &[NodeTopSurfacePolygonSource],
) -> Vec<NodeTopSupportEdge> {
    let mut top_edges = Vec::new();
    for (region, source) in owned_regions.iter().zip(node_top_surface_sources) {
        let owner = NodeBandOwner::new(region.kind, region.owner_index);
        let mut edge_counts = BTreeMap::<NodeTopSupportEdgeKey, usize>::new();
        for edge_key in top_source_boundary_edges(source) {
            *edge_counts.entry(edge_key).or_default() += 1;
        }
        top_edges.extend(
            edge_counts.into_iter().filter_map(|(key, count)| {
                (count == 1).then_some(NodeTopSupportEdge { owner, key })
            }),
        );
    }
    top_edges
}

fn top_source_boundary_edges(source: &NodeTopSurfacePolygonSource) -> Vec<NodeTopSupportEdgeKey> {
    if source.vertex_keys.len() != source.vertex_height_mm.len() {
        return Vec::new();
    }
    let mut edges = Vec::new();
    for index in 0..source.vertex_keys.len() {
        let next = (index + 1) % source.vertex_keys.len();
        edges.push(NodeTopSupportEdgeKey::from_source_vertices(
            source.vertex_keys[index],
            source.vertex_height_mm[index],
            source.vertex_keys[next],
            source.vertex_height_mm[next],
        ));
    }
    edges
}

fn raised_step_face_has_owned_top_support(
    face: &RoadSurfaceRaisedStepFace,
    top_edges: &[NodeTopSupportEdge],
) -> bool {
    let Some((lower_owner, raised_owner)) = vertical_face_lower_and_raised_owners(face.source)
    else {
        return false;
    };
    let lower_edge = NodeTopSupportEdgeKey::from_boundary_points(face.lower_edge);
    let upper_edge = NodeTopSupportEdgeKey::from_boundary_points(face.upper_edge);
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

impl NodeTopSupportEdgeKey {
    fn from_source_vertices(
        start_key: super::super::arrangement::NodeArrangementKey,
        start_y_mm: i64,
        end_key: super::super::arrangement::NodeArrangementKey,
        end_y_mm: i64,
    ) -> Self {
        let start = NodeTopSupportVertexKey::from_source(start_key, start_y_mm);
        let end = NodeTopSupportVertexKey::from_source(end_key, end_y_mm);
        if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }

    fn from_boundary_points(
        edge: (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
    ) -> Self {
        let start = NodeTopSupportVertexKey::from_boundary_point(edge.0);
        let end = NodeTopSupportVertexKey::from_boundary_point(edge.1);
        if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }
}

impl NodeTopSupportVertexKey {
    fn from_source(key: super::super::arrangement::NodeArrangementKey, y_mm: i64) -> Self {
        Self {
            xz: keys::SurfaceXzKey::from_raw_keys(key.x_key(), key.z_key()),
            y_mm,
        }
    }

    fn from_boundary_point(point: ArrangementBoundaryPointKey) -> Self {
        Self {
            xz: keys::SurfaceXzKey::from_raw_keys(point.x_key, point.z_key),
            y_mm: point.y_mm,
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
