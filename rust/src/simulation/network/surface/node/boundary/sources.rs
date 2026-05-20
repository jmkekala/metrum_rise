//! Footprint boundary source-edge and direct-vertex provenance.

use super::super::band_semantics::band_kind_sort_key;
use super::*;

impl NodeFootprintBoundaryExportSources {
    pub(in crate::simulation::network::surface) fn from_owned_regions(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        owned_regions: &[NodeOwnedRegion],
        node_top_surface_sources: &[NodeTopSurfacePolygonSource],
        explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
    ) -> Result<Self, NodeBoundaryExportError> {
        let (
            direct_vertex_sources,
            direct_vertex_source_candidates,
            direct_vertex_source_conflicts,
        ) = node_footprint_boundary_direct_vertex_sources(owned_regions, node_top_surface_sources)?;
        Ok(Self {
            source_edges: node_earthwork_boundary_source_edges_from_owned_regions(
                node_id,
                kind,
                owned_regions,
                node_top_surface_sources,
            )?,
            direct_vertex_sources,
            direct_vertex_source_candidates,
            direct_vertex_source_conflicts,
            explicit_vertical_step_segments: explicit_vertical_step_segments.to_vec(),
        })
    }

    pub(in crate::simulation::network::surface) fn extend_arrangement_exposed_boundary_edges(
        &mut self,
        arrangement: &arrangement::NodeArrangement,
    ) -> Result<(), NodeBoundaryExportError> {
        for edge in arrangement
            .edges()
            .iter()
            .filter(|edge| edge.exposed_boundary())
        {
            let Some(start_vertex) = arrangement.vertices().get(edge.start().index()) else {
                return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
            };
            let Some(end_vertex) = arrangement.vertices().get(edge.end().index()) else {
                return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
            };
            if start_vertex.key() == end_vertex.key() {
                continue;
            }
            let start_point_key = ArrangementBoundaryPointKey {
                x_key: start_vertex.key().x_key(),
                z_key: start_vertex.key().z_key(),
                y_mm: start_vertex.height_mm(),
            };
            let end_point_key = ArrangementBoundaryPointKey {
                x_key: end_vertex.key().x_key(),
                z_key: end_vertex.key().z_key(),
                y_mm: end_vertex.height_mm(),
            };
            let Some(NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::Direct(start_source),
                ..
            }) = self.direct_vertex_sources.get(&start_point_key).copied()
            else {
                continue;
            };
            let Some(NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::Direct(end_source),
                ..
            }) = self.direct_vertex_sources.get(&end_point_key).copied()
            else {
                continue;
            };
            let owner = edge.owner();
            self.source_edges.push(NodeEarthworkBoundarySourceEdge {
                start_point_key,
                end_point_key,
                start_key: start_point_key.xz_key(),
                end_key: end_point_key.xz_key(),
                final_footprint_boundary: true,
                node_id: arrangement.node_id(),
                kind: arrangement.piece_kind(),
                owner_kind: owner.kind(),
                owner_index: owner.owner_index(),
                start_source,
                end_source,
            });
        }
        self.source_edges.sort_by(|a, b| {
            node_earthwork_source_edge_ordering(a, b)
                .then(a.start_key.cmp(&b.start_key))
                .then(a.end_key.cmp(&b.end_key))
        });
        Ok(())
    }
}

fn node_earthwork_boundary_source_edges_from_owned_regions(
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

fn node_footprint_boundary_direct_vertex_sources(
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

fn insert_node_footprint_boundary_direct_vertex_source(
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
    if node_footprint_direct_vertex_ordering(candidate, *current).is_gt() {
        *current = candidate;
    }
}

fn top_source_boundary_point_key(
    top_source: &NodeTopSurfacePolygonSource,
    point_index: usize,
) -> ArrangementBoundaryPointKey {
    ArrangementBoundaryPointKey {
        x_key: top_source.vertex_keys[point_index].x_key(),
        z_key: top_source.vertex_keys[point_index].z_key(),
        y_mm: top_source.vertex_height_mm[point_index],
    }
}

pub(super) fn node_footprint_boundary_vertex_source_at_point(
    point_key: ArrangementBoundaryPointKey,
    direct_vertex_sources: &BTreeMap<
        ArrangementBoundaryPointKey,
        NodeFootprintBoundaryDirectVertex,
    >,
) -> Option<NodeFootprintBoundaryDirectVertex> {
    direct_vertex_sources.get(&point_key).copied()
}

pub(super) fn node_footprint_boundary_vertex_source_for_edge_point(
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

fn node_earthwork_source_edge_ordering(
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
