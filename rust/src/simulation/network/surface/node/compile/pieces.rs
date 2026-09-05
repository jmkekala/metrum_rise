// SPDX-License-Identifier: GPL-2.0-only

//! Visual node-piece assembly entry points.

use super::*;
use crate::simulation::network::types::EdgeClass;
use std::sync::Arc;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn visual_node_compile_input(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Option<RoadSurfaceVisualNodeCompileInput> {
        let valid = graph.get_valid_node(node_id);
        let incidents = self.sorted_incident_surface_edges(graph, valid);
        match self.classify_visual_node_kind(&incidents) {
            CompiledNodeKind::Terminal => {
                let incident = incidents.first()?;
                let mouths = self.build_ordered_piece_mouths(graph, &[*incident])?;
                let mouth_edge_classes = Self::node_mouth_edge_classes(graph, &mouths)?;
                Some(RoadSurfaceVisualNodeCompileInput {
                    kind: RoadSurfaceVisualNodePieceKind::Terminal,
                    mouths,
                    mouth_edge_classes,
                })
            }
            CompiledNodeKind::PassThrough => None,
            CompiledNodeKind::Bend => {
                if incidents.len() != 2 {
                    return None;
                }
                let mouths = self.build_ordered_piece_mouths(graph, &incidents)?;
                let mouth_edge_classes = Self::node_mouth_edge_classes(graph, &mouths)?;
                Some(RoadSurfaceVisualNodeCompileInput {
                    kind: RoadSurfaceVisualNodePieceKind::Bend,
                    mouths,
                    mouth_edge_classes,
                })
            }
            CompiledNodeKind::JunctionN => {
                if incidents.len() < 3 {
                    return None;
                }
                let mouths = self.build_ordered_piece_mouths(graph, &incidents)?;
                let mouth_edge_classes = Self::node_mouth_edge_classes(graph, &mouths)?;
                Some(RoadSurfaceVisualNodeCompileInput {
                    kind: RoadSurfaceVisualNodePieceKind::JunctionN,
                    mouths,
                    mouth_edge_classes,
                })
            }
        }
    }

    fn node_mouth_edge_classes(
        graph: &RegionGraph,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<Vec<EdgeClass>> {
        mouths
            .iter()
            .map(|mouth| {
                (mouth.edge_idx < graph.edge_count()).then(|| graph.edge(mouth.edge_idx).class)
            })
            .collect()
    }

    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn compile_visual_node_piece_from_input(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        input: &RoadSurfaceVisualNodeCompileInput,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        self.compile_visual_node_piece_with_earthwork_boundaries(
            graph, terrain, node_id, input, None,
        )
        .map(|result| Arc::unwrap_or_clone(result.piece))
    }

    pub(in crate::simulation::network::surface) fn compile_visual_node_piece_with_earthwork_boundaries(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        input: &RoadSurfaceVisualNodeCompileInput,
        previous_topology: Option<&NodeCanonicalTopologyCache>,
    ) -> Option<NodeVisualCompileResult> {
        self.compile_visual_node_piece_with_earthwork_boundaries_once(
            graph,
            terrain,
            node_id,
            input,
            previous_topology,
        )
        .or_else(|| {
            previous_topology.and_then(|_| {
                self.compile_visual_node_piece_with_earthwork_boundaries_once(
                    graph, terrain, node_id, input, None,
                )
            })
        })
    }

    fn compile_visual_node_piece_with_earthwork_boundaries_once(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        input: &RoadSurfaceVisualNodeCompileInput,
        previous_topology: Option<&NodeCanonicalTopologyCache>,
    ) -> Option<NodeVisualCompileResult> {
        let canonical = self.compile_canonical_node_surface_regions_with_topology_cache(
            node_id,
            input.kind,
            &input.mouths,
            previous_topology,
        )?;
        let node_regions = canonical.regions;
        let mut earthwork_boundary_segments = node_regions.earthwork_boundary_segments.clone();
        for segment in earthwork_boundary_segments.iter_mut().flatten() {
            segment.source = segment.source.with_node_identity(node_id, input.kind);
        }
        let top_surface_shapes = Self::top_surface_overlay_shapes(
            node_regions
                .road_surface_polygons
                .iter()
                .chain(node_regions.curb_surface_polygons.iter())
                .chain(node_regions.sidewalk_surface_polygons.iter()),
        );
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_segments(
                &node_regions.earthwork_boundary_segments,
                terrain,
                top_surface_shapes.as_ref(),
            )
            .ok()?;
        let earthwork_owner_sources = Self::node_earthwork_owner_sources_from_regions(
            graph,
            &input.mouths,
            &node_regions.owned_regions,
            &node_regions.node_top_surface_sources,
        );

        let piece = self.assemble_explicit_node_piece(
            node_id,
            input.kind,
            node_regions.outer_boundary_loops,
            node_regions.terrain_clip_boundary_loops,
            node_regions.road_surface_polygons,
            node_regions.curb_surface_polygons,
            node_regions.raised_step_faces,
            node_regions.sidewalk_surface_polygons,
            node_regions.explicit_vertical_step_segments,
            node_regions.node_grade_authorities,
            node_regions.node_top_surface_sources,
            node_regions.owned_regions,
            node_regions.boolean_debug,
            earthwork_owner_sources,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )?;
        Some(NodeVisualCompileResult {
            piece: Arc::new(piece),
            earthwork_boundaries: Arc::new(earthwork_boundary_segments),
            topology_cache: canonical.topology_cache.map(Arc::new),
            rail_topology_reused: canonical.rail_topology_reused,
            ownership_reused: canonical.ownership_reused,
            preview_artifact_zero_copy: false,
            #[cfg(test)]
            export_reuse_stats: canonical.export_reuse_stats,
        })
    }

    pub(in crate::simulation::network::surface) fn replay_exact_preview_node_piece(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        input: &RoadSurfaceVisualNodeCompileInput,
        preview_piece: Arc<RoadSurfaceVisualNodePiece>,
        preview_earthwork_boundaries: Arc<Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>>,
        preview_topology: Arc<NodeCanonicalTopologyCache>,
        exact_identity: bool,
    ) -> NodeVisualCompileResult {
        if exact_identity && preview_piece.boolean_debug.is_none() {
            let topology_cache = if input.kind == RoadSurfaceVisualNodePieceKind::JunctionN {
                preview_topology
            } else {
                Arc::new(
                    Arc::try_unwrap(preview_topology)
                        .unwrap_or_else(|topology| topology.as_ref().clone())
                        .into_for_committed_node(input.kind),
                )
            };
            return NodeVisualCompileResult {
                piece: preview_piece,
                earthwork_boundaries: preview_earthwork_boundaries,
                topology_cache: Some(topology_cache),
                rail_topology_reused: true,
                ownership_reused: true,
                preview_artifact_zero_copy: true,
                #[cfg(test)]
                export_reuse_stats: Default::default(),
            };
        }

        let (mut piece, piece_zero_copy) = match Arc::try_unwrap(preview_piece) {
            Ok(piece) => (piece, true),
            Err(piece) => (piece.as_ref().clone(), false),
        };
        piece.node_id = node_id;
        piece.boolean_debug = None;
        piece.earthwork_owner_sources = Self::node_earthwork_owner_sources_from_regions(
            graph,
            &input.mouths,
            &piece.owned_regions,
            &piece.node_top_surface_sources,
        );
        for face in &mut piece.render_earthwork_faces {
            face.source = face.source.with_node_identity(node_id, input.kind);
        }
        let (mut earthwork_boundaries, boundaries_zero_copy) =
            match Arc::try_unwrap(preview_earthwork_boundaries) {
                Ok(boundaries) => (boundaries, true),
                Err(boundaries) => (boundaries.as_ref().clone(), false),
            };
        for segment in earthwork_boundaries.iter_mut().flatten() {
            segment.source = segment.source.with_node_identity(node_id, input.kind);
        }
        let (topology, topology_zero_copy) = match Arc::try_unwrap(preview_topology) {
            Ok(topology) => (topology, true),
            Err(topology) => (topology.as_ref().clone(), false),
        };
        NodeVisualCompileResult {
            piece: Arc::new(piece),
            earthwork_boundaries: Arc::new(earthwork_boundaries),
            topology_cache: Some(Arc::new(topology.into_for_committed_node(input.kind))),
            rail_topology_reused: true,
            ownership_reused: true,
            preview_artifact_zero_copy: piece_zero_copy
                && boundaries_zero_copy
                && topology_zero_copy,
            #[cfg(test)]
            export_reuse_stats: Default::default(),
        }
    }

    pub(in crate::simulation::network::surface) fn refresh_visual_node_piece_earthwork_from_cached_top(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        input: &RoadSurfaceVisualNodeCompileInput,
        cached_piece: &RoadSurfaceVisualNodePiece,
        earthwork_boundary_segments: &[Vec<RoadSurfaceEarthworkBoundarySegment>],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if cached_piece.node_id != node_id || cached_piece.kind != input.kind {
            return None;
        }
        let top_surface_shapes = Self::top_surface_overlay_shapes(
            cached_piece
                .road_surface_polygons
                .iter()
                .chain(&cached_piece.curb_surface_polygons)
                .chain(&cached_piece.sidewalk_surface_polygons),
        );
        let (
            mut earthwork_surface_polygons,
            mut earthwork_outer_boundary_loops,
            mut render_earthwork_faces,
        ) = self
            .build_closed_earthwork_geometry_from_boundary_segments(
                earthwork_boundary_segments,
                terrain,
                top_surface_shapes.as_ref(),
            )
            .ok()?;
        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);

        let mut piece = cached_piece.clone();
        piece.earthwork_owner_sources = Self::node_earthwork_owner_sources_from_regions(
            graph,
            &input.mouths,
            &piece.owned_regions,
            &piece.node_top_surface_sources,
        );
        piece.earthwork_surface_polygons = earthwork_surface_polygons;
        piece.earthwork_outer_boundary_loops = earthwork_outer_boundary_loops;
        piece.render_earthwork_faces = render_earthwork_faces;
        Some(piece)
    }

    fn node_earthwork_owner_sources_from_regions(
        graph: &RegionGraph,
        mouths: &[OrderedIncidentPieceMouth],
        owned_regions: &[NodeOwnedRegion],
        node_top_surface_sources: &[NodeTopSurfacePolygonSource],
    ) -> Vec<NodeEarthworkOwnerSource> {
        let mut sources = Vec::new();
        for (region, top_source) in owned_regions.iter().zip(node_top_surface_sources) {
            let mouth_order_index = top_source.height_field_id.mouth_order_index();
            let Some(mouth) = mouths.get(mouth_order_index) else {
                continue;
            };
            if mouth.edge_idx >= graph.edge_count() {
                continue;
            }
            sources.push(NodeEarthworkOwnerSource {
                owner_kind: region.kind,
                owner_index: region.owner_index,
                mouth_order_index,
                edge_idx: mouth.edge_idx,
                edge_class: graph.edge(mouth.edge_idx).class,
            });
        }
        sources.sort_by(|a, b| {
            a.owner_kind
                .cmp(&b.owner_kind)
                .then(a.owner_index.cmp(&b.owner_index))
                .then(a.mouth_order_index.cmp(&b.mouth_order_index))
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(edge_class_sort_key(a.edge_class).cmp(&edge_class_sort_key(b.edge_class)))
        });
        sources.dedup_by(|a, b| {
            a.owner_kind == b.owner_kind
                && a.owner_index == b.owner_index
                && a.mouth_order_index == b.mouth_order_index
                && a.edge_idx == b.edge_idx
                && a.edge_class == b.edge_class
        });
        sources
    }
}
