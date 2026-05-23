//! Direct boundary vertex source collection.

use super::*;

pub(super) fn node_footprint_boundary_direct_vertex_sources(
    owned_regions: &[NodeOwnedRegion],
    node_top_surface_sources: &[NodeTopSurfacePolygonSource],
) -> Result<
    (
        BTreeMap<ArrangementBoundaryPointKey, NodeFootprintBoundaryDirectVertex>,
        BTreeMap<ArrangementBoundaryPointKey, Vec<NodeFootprintBoundaryDirectVertex>>,
        BTreeMap<ArrangementBoundaryPointKey, NodeFootprintBoundaryDirectVertexConflict>,
    ),
    NodeBoundaryExportError,
> {
    if owned_regions.len() != node_top_surface_sources.len() {
        return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
    }
    let mut sources = BTreeMap::new();
    let mut candidates = BTreeMap::new();
    let mut conflicts = BTreeMap::new();
    for (region_index, region) in owned_regions.iter().enumerate() {
        let Some(top_source) = node_top_surface_sources.get(region_index) else {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        };
        if top_source.vertex_sources.len() != region.polygon.points_world.len()
            || top_source.vertex_keys.len() != region.polygon.points_world.len()
            || top_source.vertex_height_mm.len() != region.polygon.points_world.len()
        {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        }
        for point_index in 0..region.polygon.points_world.len() {
            let candidate = NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::Direct(
                    NodeFootprintBoundaryDirectSource {
                        top_surface_source_index: region_index,
                        grade_authority_index: top_source.vertex_sources[point_index]
                            .grade_authority_index,
                    },
                ),
                owner_kind: region.kind,
                owner_index: region.owner_index,
            };
            insert_node_footprint_boundary_direct_vertex_source(
                &mut sources,
                &mut candidates,
                &mut conflicts,
                top_source_boundary_point_key(top_source, point_index),
                candidate,
            );
        }
    }
    Ok((sources, candidates, conflicts))
}

pub(super) fn insert_node_footprint_boundary_direct_vertex_source(
    sources: &mut BTreeMap<ArrangementBoundaryPointKey, NodeFootprintBoundaryDirectVertex>,
    candidates: &mut BTreeMap<ArrangementBoundaryPointKey, Vec<NodeFootprintBoundaryDirectVertex>>,
    conflicts: &mut BTreeMap<
        ArrangementBoundaryPointKey,
        NodeFootprintBoundaryDirectVertexConflict,
    >,
    point_key: ArrangementBoundaryPointKey,
    candidate: NodeFootprintBoundaryDirectVertex,
) {
    let point_candidates = candidates.entry(point_key).or_default();
    if !point_candidates
        .iter()
        .copied()
        .any(|existing| node_footprint_direct_vertices_share_source_identity(existing, candidate))
    {
        point_candidates.push(candidate);
    }
    let Some(current) = sources.get_mut(&point_key) else {
        sources.insert(point_key, candidate);
        return;
    };
    if !node_footprint_direct_vertices_share_source_identity(candidate, *current) {
        conflicts
            .entry(point_key)
            .or_insert(NodeFootprintBoundaryDirectVertexConflict {
                existing: *current,
                incoming: candidate,
            });
    }
}

pub(super) fn top_source_boundary_point_key(
    top_source: &NodeTopSurfacePolygonSource,
    point_index: usize,
) -> ArrangementBoundaryPointKey {
    ArrangementBoundaryPointKey {
        x_key: top_source.vertex_keys[point_index].x_key(),
        z_key: top_source.vertex_keys[point_index].z_key(),
        y_mm: top_source.vertex_height_mm[point_index],
    }
}
