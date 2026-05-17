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
        Ok(Self {
            source_edges: node_earthwork_boundary_source_edges_from_owned_regions(
                node_id,
                kind,
                owned_regions,
                node_top_surface_sources,
            )?,
            direct_vertex_sources: node_footprint_boundary_direct_vertex_sources(
                owned_regions,
                node_top_surface_sources,
            )?,
            explicit_vertical_step_segments: explicit_vertical_step_segments.to_vec(),
        })
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
        if top_source.vertex_sources.len() != points.len() {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        }
        for index in 0..points.len() {
            let start_point_key = ArrangementBoundaryPointKey::from_world(points[index]);
            let end_point_key =
                ArrangementBoundaryPointKey::from_world(points[(index + 1) % points.len()]);
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
    BTreeMap<ArrangementBoundaryPointKey, NodeFootprintBoundaryDirectVertex>,
    NodeBoundaryExportError,
> {
    if owned_regions.len() != node_top_surface_sources.len() {
        return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
    }
    let mut sources = BTreeMap::new();
    for (region_index, region) in owned_regions.iter().enumerate() {
        let Some(top_source) = node_top_surface_sources.get(region_index) else {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        };
        if top_source.vertex_sources.len() != region.polygon.points_world.len() {
            return Err(NodeBoundaryExportError::MissingNodeTopSurfaceGradeAuthority);
        }
        for (point_index, point) in region.polygon.points_world.iter().copied().enumerate() {
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
            sources
                .entry(ArrangementBoundaryPointKey::from_world(point))
                .and_modify(|current| {
                    if node_footprint_direct_vertex_ordering(candidate, *current).is_gt() {
                        *current = candidate;
                    }
                })
                .or_insert(candidate);
        }
    }
    Ok(sources)
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

pub(super) fn node_footprint_boundary_vertex_source_for_edge_point_with_canonical_drift(
    source_edge: &NodeEarthworkBoundarySourceEdge,
    point_key: ArrangementBoundaryPointKey,
) -> Option<NodeFootprintBoundaryVertexSource> {
    let parameter = arrangement_key_segment_parameter_with_canonical_drift(
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
    Some(node_footprint_boundary_vertex_source_for_edge_parameter(
        source_edge,
        point_key.y_mm,
    ))
}

pub(super) fn node_footprint_boundary_vertex_source_for_edge_parameter(
    source_edge: &NodeEarthworkBoundarySourceEdge,
    height_mm: i64,
) -> NodeFootprintBoundaryVertexSource {
    NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
        owning_segment_start: source_edge.start_source,
        owning_segment_end: source_edge.end_source,
        height_mm,
    }
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
