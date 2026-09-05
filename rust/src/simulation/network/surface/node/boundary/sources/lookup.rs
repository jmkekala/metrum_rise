// SPDX-License-Identifier: GPL-2.0-only

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
    let parameter = boundary_segment_parameter_xz_on_segment(
        point_key,
        source_edge.start_point_key,
        source_edge.end_point_key,
    )
    .or_else(|| {
        if source_edge.final_footprint_boundary {
            final_boundary_segment_parameter_xz_on_segment(
                point_key,
                source_edge.start_point_key,
                source_edge.end_point_key,
            )
        } else {
            None
        }
    })
    .or_else(|| {
        endpoint_dust_segment_parameter(
            boundary_point_surface_key(point_key),
            boundary_point_surface_key(source_edge.start_point_key),
            boundary_point_surface_key(source_edge.end_point_key),
        )
    })?;
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
