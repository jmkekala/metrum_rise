//! Boundary vertex source lookup.

use super::*;

pub(in crate::simulation::network::surface::node::boundary) fn node_footprint_boundary_vertex_source_at_point(
    point_key: ArrangementBoundaryPointKey,
    direct_vertex_sources: &BTreeMap<
        ArrangementBoundaryPointKey,
        NodeFootprintBoundaryDirectVertex,
    >,
) -> Option<NodeFootprintBoundaryDirectVertex> {
    direct_vertex_sources.get(&point_key).copied()
}

pub(in crate::simulation::network::surface::node::boundary) fn node_footprint_boundary_vertex_source_for_edge_point(
    source_edge: &NodeEarthworkBoundarySourceEdge,
    point_key: ArrangementBoundaryPointKey,
) -> Option<NodeFootprintBoundaryVertexSource> {
    if point_key == source_edge.start_point_key {
        return Some(NodeFootprintBoundaryVertexSource::Direct(
            source_edge.start_source,
        ));
    }
    if point_key == source_edge.end_point_key {
        return Some(NodeFootprintBoundaryVertexSource::Direct(
            source_edge.end_source,
        ));
    }
    if !arrangement_key_lies_exactly_on_segment(
        point_key.xz_key(),
        source_edge.start_key,
        source_edge.end_key,
    ) {
        return None;
    }
    let parameter = arrangement_key_segment_parameter_xz(
        point_key.xz_key(),
        source_edge.start_key,
        source_edge.end_key,
    )?;
    let expected_height_mm = interpolated_segment_height_mm(
        source_edge.start_point_key,
        source_edge.end_point_key,
        parameter,
    );
    if (expected_height_mm - point_key.y_mm).abs() > 1 {
        return None;
    }
    Some(NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
        owning_segment_start: source_edge.start_source,
        owning_segment_end: source_edge.end_source,
        height_mm: point_key.y_mm,
    })
}
