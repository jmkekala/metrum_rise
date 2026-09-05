// SPDX-License-Identifier: GPL-2.0-only

//! Footprint boundary source-edge extraction.

use super::*;

pub(super) fn node_earthwork_boundary_source_edges_from_owned_regions(
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
    owned_regions: &[NodeOwnedRegion],
    node_top_surface_sources: &[NodeTopSurfacePolygonSource],
) -> Result<Vec<NodeEarthworkBoundarySourceEdge>, NodeBoundaryExportError> {
    if owned_regions.len() != node_top_surface_sources.len() {
        return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
    }
    let mut source_edges = Vec::new();
    for (region_index, region) in owned_regions.iter().enumerate() {
        let Some(top_source) = node_top_surface_sources.get(region_index) else {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        };
        let points = &region.polygon.points_world;
        if points.len() < 3 {
            continue;
        }
        if top_source.vertex_sources.len() != points.len()
            || top_source.vertex_keys.len() != points.len()
            || top_source.vertex_height_mm.len() != points.len()
        {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        }
        for index in 0..points.len() {
            let start_point_key = top_source_boundary_point_key(top_source, index);
            let end_point_key =
                top_source_boundary_point_key(top_source, (index + 1) % points.len());
            let start_key = start_point_key.xz_key();
            let end_key = end_point_key.xz_key();
            if start_key == end_key {
                continue;
            }
            source_edges.push(NodeEarthworkBoundarySourceEdge {
                start_point_key,
                end_point_key,
                start_key,
                end_key,
                final_footprint_boundary: false,
                node_id,
                kind,
                owner_kind: region.kind,
                owner_index: region.owner_index,
                height_field_id: top_source.height_field_id,
                start_source: NodeFootprintBoundaryDirectSource {
                    top_surface_source_index: region_index,
                    grade_authority_index: top_source.vertex_sources[index].grade_authority_index,
                },
                end_source: NodeFootprintBoundaryDirectSource {
                    top_surface_source_index: region_index,
                    grade_authority_index: top_source.vertex_sources[(index + 1) % points.len()]
                        .grade_authority_index,
                },
            });
        }
    }
    source_edges.sort_by(|a, b| {
        node_earthwork_source_edge_ordering(a, b)
            .then(a.start_key.cmp(&b.start_key))
            .then(a.end_key.cmp(&b.end_key))
    });
    Ok(source_edges)
}

pub(super) fn node_earthwork_source_edge_ordering(
    a: &NodeEarthworkBoundarySourceEdge,
    b: &NodeEarthworkBoundarySourceEdge,
) -> std::cmp::Ordering {
    a.node_id
        .cmp(&b.node_id)
        .then(a.kind.sort_key().cmp(&b.kind.sort_key()))
        .then(band_kind_sort_key(a.owner_kind).cmp(&band_kind_sort_key(b.owner_kind)))
        .then(a.owner_index.cmp(&b.owner_index))
        .then(a.start_source.cmp(&b.start_source))
        .then(a.end_source.cmp(&b.end_source))
}
