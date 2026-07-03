//! Footprint boundary export-source construction.

use super::*;

impl NodeFootprintBoundaryExportSources {
    pub(in crate::simulation::network::surface) fn from_owned_regions(
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        owned_regions: &[NodeOwnedRegion],
        node_top_surface_sources: &[NodeTopSurfacePolygonSource],
        node_grade_authorities: &[height::NodeGradeVertexAuthority],
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
            final_height_edges: Vec::new(),
            final_vertex_sources: BTreeMap::new(),
            direct_vertex_sources,
            direct_vertex_source_candidates,
            direct_vertex_source_conflicts,
            grade_authority_source_provenance: node_grade_authorities
                .iter()
                .map(|authority| authority.source_provenance)
                .collect(),
            explicit_vertical_step_segments: explicit_vertical_step_segments.to_vec(),
        })
    }

    pub(in crate::simulation::network::surface) fn extend_arrangement_exposed_boundary_edges(
        &mut self,
        arrangement: &arrangement::NodeArrangement,
        top_height_context: &super::super::super::NodeExportTopHeightContext,
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
            let owner = edge.owner();
            let start_source_kind = start_vertex
                .grade_authority()
                .source_provenance
                .map_or(start_vertex.height_field_id().kind(), |provenance| {
                    provenance.source_kind
                });
            let end_source_kind = end_vertex
                .grade_authority()
                .source_provenance
                .map_or(end_vertex.height_field_id().kind(), |provenance| {
                    provenance.source_kind
                });
            let start_source_kind = super::super::super::node_export_top_source_kind(
                owner,
                start_source_kind,
                start_vertex.key(),
                start_vertex.height_mm(),
                top_height_context,
            );
            let end_source_kind = super::super::super::node_export_top_source_kind(
                owner,
                end_source_kind,
                end_vertex.key(),
                end_vertex.height_mm(),
                top_height_context,
            );
            let height_mm = [
                super::super::super::node_export_top_height_mm(
                    owner,
                    start_source_kind,
                    start_vertex.key(),
                    start_vertex.height_mm(),
                    top_height_context,
                ),
                super::super::super::node_export_top_height_mm(
                    owner,
                    end_source_kind,
                    end_vertex.key(),
                    end_vertex.height_mm(),
                    top_height_context,
                ),
            ];
            let start_point_key = ArrangementBoundaryPointKey {
                x_key: start_vertex.key().x_key(),
                z_key: start_vertex.key().z_key(),
                y_mm: height_mm[0],
            };
            let end_point_key = ArrangementBoundaryPointKey {
                x_key: end_vertex.key().x_key(),
                z_key: end_vertex.key().z_key(),
                y_mm: height_mm[1],
            };
            self.ensure_arrangement_exposed_boundary_vertex_source(start_point_key, owner)?;
            self.ensure_arrangement_exposed_boundary_vertex_source(end_point_key, owner)?;
            self.final_height_edges
                .push(NodeFinalFootprintBoundaryHeightEdge {
                    start_point_key,
                    end_point_key,
                    owner_kind: owner.kind(),
                    owner_index: owner.owner_index(),
                });
            self.insert_final_footprint_boundary_vertex_source(start_point_key, owner)?;
            self.insert_final_footprint_boundary_vertex_source(end_point_key, owner)?;
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
        self.final_height_edges
            .sort_by(|a, b| final_height_edge_ordering(a, b));
        Ok(())
    }
}

fn final_height_edge_ordering(
    a: &NodeFinalFootprintBoundaryHeightEdge,
    b: &NodeFinalFootprintBoundaryHeightEdge,
) -> std::cmp::Ordering {
    band_kind_sort_key(a.owner_kind)
        .cmp(&band_kind_sort_key(b.owner_kind))
        .then(a.owner_index.cmp(&b.owner_index))
        .then(a.start_point_key.cmp(&b.start_point_key))
        .then(a.end_point_key.cmp(&b.end_point_key))
}

impl NodeFootprintBoundaryExportSources {
    fn ensure_arrangement_exposed_boundary_vertex_source(
        &mut self,
        point_key: ArrangementBoundaryPointKey,
        owner: arrangement::NodeBandOwner,
    ) -> Result<(), NodeBoundaryExportError> {
        if self
            .direct_vertex_candidates_at_point(point_key)
            .into_iter()
            .any(|candidate| {
                candidate.owner_kind == owner.kind() && candidate.owner_index == owner.owner_index()
            })
        {
            return Ok(());
        }
        insert_node_footprint_boundary_direct_vertex_source(
            &mut self.direct_vertex_sources,
            &mut self.direct_vertex_source_candidates,
            &mut self.direct_vertex_source_conflicts,
            point_key,
            NodeFootprintBoundaryDirectVertex {
                source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: point_key.x_key,
                    z_key: point_key.z_key,
                    y_mm: point_key.y_mm,
                },
                owner_kind: owner.kind(),
                owner_index: owner.owner_index(),
            },
        );
        Ok(())
    }

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

    fn insert_final_footprint_boundary_vertex_source(
        &mut self,
        point_key: ArrangementBoundaryPointKey,
        owner: arrangement::NodeBandOwner,
    ) -> Result<(), NodeBoundaryExportError> {
        let Some(candidate) = self.unique_vertex_source_for_owner_at_point(
            point_key,
            owner.kind(),
            owner.owner_index(),
        )?
        else {
            return Ok(());
        };
        let candidates = self.final_vertex_sources.entry(point_key).or_default();
        if !candidates.iter().copied().any(|existing| {
            node_footprint_direct_vertices_share_source_identity(existing, candidate)
        }) {
            candidates.push(candidate);
        }
        Ok(())
    }

    fn unique_vertex_source_for_owner_at_point(
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
                candidate.owner_kind == owner_kind && candidate.owner_index == owner_index
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
