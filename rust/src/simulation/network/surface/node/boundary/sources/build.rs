//! Footprint boundary export-source construction.

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
            let owner = edge.owner();
            let Some(NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::Direct(start_source),
                ..
            }) = self.unique_direct_vertex_source_for_owner_at_point(
                start_point_key,
                owner.kind(),
                owner.owner_index(),
            )?
            else {
                continue;
            };
            let Some(NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::Direct(end_source),
                ..
            }) = self.unique_direct_vertex_source_for_owner_at_point(
                end_point_key,
                owner.kind(),
                owner.owner_index(),
            )?
            else {
                continue;
            };
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
                height_field_id: edge.height_field_id(),
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

impl NodeFootprintBoundaryExportSources {
    fn unique_direct_vertex_source_for_owner_at_point(
        &self,
        point_key: ArrangementBoundaryPointKey,
        owner_kind: RoadSurfaceBandKind,
        owner_index: usize,
    ) -> Result<Option<NodeFootprintBoundaryDirectVertex>, NodeBoundaryExportError> {
        let mut source = None;
        for candidate in self
            .direct_vertex_candidates_at_point(point_key)
            .into_iter()
            .filter(|candidate| {
                candidate.owner_kind == owner_kind
                    && candidate.owner_index == owner_index
                    && matches!(
                        candidate.source,
                        NodeFootprintBoundaryVertexSource::Direct(_)
                    )
            })
        {
            merge_node_footprint_boundary_point_source(point_key, &mut source, candidate)?;
        }
        Ok(source)
    }

    pub(in crate::simulation::network::surface::node::boundary) fn direct_vertex_candidates_at_point(
        &self,
        point_key: ArrangementBoundaryPointKey,
    ) -> Vec<NodeFootprintBoundaryDirectVertex> {
        if let Some(candidates) = self.direct_vertex_source_candidates.get(&point_key) {
            return candidates.clone();
        }
        if let Some(conflict) = self.direct_vertex_source_conflicts.get(&point_key).copied() {
            return vec![conflict.existing, conflict.incoming];
        }
        node_footprint_boundary_vertex_source_at_point(point_key, &self.direct_vertex_sources)
            .into_iter()
            .collect()
    }
}
