// SPDX-License-Identifier: GPL-2.0-only

//! Local junction preview export and reversible replacement of cached source owners.

use super::{NetworkMeshData, NetworkMeshOwner, RoadRenderer};
use crate::simulation::network::graph::{RegionGraph, rebuild::JUNCTION_PROFILE_BLEND_ZONE_M};
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::surface::{RoadSurfaceSystem, SurfaceChunkKey};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector2;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// The already-compiled validation neighborhood, moved rather than compiled a second time.
pub(crate) struct RoadPreviewRenderInput {
    graph: RegionGraph,
    surface: RoadSurfaceSystem,
    removed: HashSet<NetworkMeshOwner>,
    bounded: HashSet<NetworkMeshOwner>,
    bounds: Vec<[f32; 4]>,
    replaced_nodes: HashSet<NetworkMeshOwner>,
    planned_whole: HashSet<NetworkMeshOwner>,
    planned_bounded: HashSet<NetworkMeshOwner>,
}

/// Immutable render-only meshes; retained chunks exclude precisely the replaced source owners.
#[derive(Clone, Debug)]
pub(crate) struct RoadJunctionPreview {
    pub(crate) source_mesh_generation: u64,
    pub(crate) planned: BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>,
    pub(crate) retained: Arc<BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>>,
    pub(crate) retained_revision: u64,
    pub(crate) replacement_chunks: BTreeSet<SurfaceChunkKey>,
    pub(crate) chunk_span_m: f32,
    pub(crate) chunk_origin_x_m: f32,
    pub(crate) chunk_origin_z_m: f32,
    removed: HashSet<NetworkMeshOwner>,
    bounded: HashSet<NetworkMeshOwner>,
    bounds: Vec<[f32; 4]>,
}

impl RoadPreviewRenderInput {
    /// Resolves local validation IDs back to the existing mesh owners before any render export.
    pub(crate) fn new(
        graph: RegionGraph,
        surface: RoadSurfaceSystem,
        source_edges: HashMap<usize, usize>,
        removed_nodes: HashSet<u32>,
        replaced_nodes: HashSet<u32>,
        preview_edges: HashSet<usize>,
        preview_nodes: HashSet<u32>,
    ) -> Self {
        let mut removed = HashSet::new();
        for (source, local) in source_edges {
            if graph.edge(local).deleted || surface.compiled_visual_span_pieces.contains_key(&local)
            {
                removed.insert(NetworkMeshOwner::Edge(source));
            }
        }
        removed.extend(removed_nodes.into_iter().map(NetworkMeshOwner::Node));
        let bounded = removed
            .iter()
            .filter(|owner| matches!(owner, NetworkMeshOwner::Edge(_)))
            .copied()
            .collect();
        let planned_whole = preview_edges
            .iter()
            .copied()
            .map(NetworkMeshOwner::Edge)
            .chain(preview_nodes.iter().copied().map(NetworkMeshOwner::Node))
            .collect();
        let planned_bounded = surface
            .compiled_visual_span_pieces
            .keys()
            .filter(|id| !preview_edges.contains(id))
            .copied()
            .map(NetworkMeshOwner::Edge)
            .collect();
        let mut nodes: Vec<_> = preview_nodes.iter().copied().collect();
        nodes.sort_unstable();
        let bounds = nodes
            .into_iter()
            .filter_map(|node| {
                let incidents = graph.node_adjacency(node);
                // Isolated new terminals have no source road to partition.
                if incidents.len() < 2 {
                    return None;
                }
                let pos = graph.node(node).pos;
                let radius = incidents
                    .iter()
                    .map(|&id| {
                        let edge = graph.edge(id);
                        graph.junction_profile_crossing_core_m(
                            id,
                            graph.get_valid_node(edge.start_node) == node,
                        ) + JUNCTION_PROFILE_BLEND_ZONE_M
                            + RoadSurfaceSystem::visual_roadbed_half_width_m(edge)
                    })
                    .fold(0.0_f32, f32::max);
                Some([
                    pos.x - radius,
                    pos.z - radius,
                    pos.x + radius,
                    pos.z + radius,
                ])
            })
            .collect();
        Self {
            graph,
            surface,
            removed,
            bounded,
            bounds,
            planned_whole,
            planned_bounded,
            replaced_nodes: replaced_nodes
                .into_iter()
                .map(NetworkMeshOwner::Node)
                .collect(),
        }
    }

    /// Exports only the local compiled scene. Lane reconstruction is confined to this excerpt;
    /// it supplies the same crosswalks and lane-divider clipping as the committed renderer.
    pub(crate) fn render(
        mut self,
        terrain: &TerrainSystem,
        existing: &RoadSurfaceSystem,
    ) -> Option<RoadJunctionPreview> {
        let mut chunks = BTreeSet::new();
        chunks.extend(self.surface.surface_chunk_cache().keys().copied());
        chunks.extend(self.surface.earthwork_chunk_cache().keys().copied());
        for owner in &self.removed {
            let (surface_chunks, earthwork_chunks) = match owner {
                NetworkMeshOwner::Edge(edge) => (
                    existing.surface_span_chunks.get(edge),
                    existing.earthwork_span_chunks.get(edge),
                ),
                NetworkMeshOwner::Node(node) => (
                    existing.surface_node_chunks.get(node),
                    existing.earthwork_node_chunks.get(node),
                ),
            };
            chunks.extend(
                surface_chunks
                    .into_iter()
                    .flat_map(|chunks| chunks.iter())
                    .copied(),
            );
            chunks.extend(
                earthwork_chunks
                    .into_iter()
                    .flat_map(|chunks| chunks.iter())
                    .copied(),
            );
        }
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut self.graph);
        let planned = RoadRenderer.generate_mesh_chunks_with_surface(
            &self.graph,
            &mut lanes,
            terrain,
            &self.surface,
            &chunks,
        );
        let (origin_x, origin_z) = self.surface.chunk_origin_m();
        // Display the canonical road geometry at its solved terrain-aware heights. Terrain
        // replacement and cutout repair belong to placement, so hover does no clearance solve.
        Some(RoadJunctionPreview {
            source_mesh_generation: 0,
            planned: planned
                .into_iter()
                .filter_map(|(key, mesh)| {
                    let origin = Vector2::new(
                        origin_x + key.0 as f32 * self.surface.chunk_span_m(),
                        origin_z + key.1 as f32 * self.surface.chunk_span_m(),
                    );
                    let mesh = mesh.preview_partition(
                        &self.planned_whole,
                        &self.planned_bounded,
                        &self.bounds,
                        origin,
                        true,
                    );
                    (!mesh.is_empty()).then(|| (key, mesh.seal()))
                })
                .collect(),
            retained: Arc::new(BTreeMap::new()),
            retained_revision: 0,
            replacement_chunks: chunks,
            chunk_span_m: self.surface.chunk_span_m(),
            chunk_origin_x_m: origin_x,
            chunk_origin_z_m: origin_z,
            removed: self.replaced_nodes,
            bounded: self.bounded,
            bounds: self.bounds,
        })
    }
}

impl RoadJunctionPreview {
    /// Filters a bounded snapshot of existing chunks off-lock. Unchanged attributes are copied
    /// exactly, including crosswalks, markings and roads unrelated to the edited junction.
    pub(crate) fn retain_existing(
        &mut self,
        meshes: &BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>,
    ) {
        self.retained = Arc::new(
            meshes
                .par_iter()
                .filter_map(|(key, mesh)| {
                    let origin = Vector2::new(
                        self.chunk_origin_x_m + key.0 as f32 * self.chunk_span_m,
                        self.chunk_origin_z_m + key.1 as f32 * self.chunk_span_m,
                    );
                    let retained = mesh.preview_partition(
                        &self.removed,
                        &self.bounded,
                        &self.bounds,
                        origin,
                        false,
                    );
                    (!retained.is_empty()).then(|| (*key, retained.seal()))
                })
                .collect(),
        );
    }
}

/// One bounded retained-mesh entry shared across a drag, keyed by exact source ownership.
#[derive(Default)]
pub(crate) struct RoadPreviewRetainedCache {
    revision: u64,
    entry: Option<RetainedEntry>,
}

struct RetainedEntry {
    source_generation: u64,
    grid: (f32, f32, f32),
    chunks: BTreeSet<SurfaceChunkKey>,
    removed: HashSet<NetworkMeshOwner>,
    bounded: HashSet<NetworkMeshOwner>,
    bounds: Vec<[f32; 4]>,
    meshes: Arc<BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>>,
}

impl RoadPreviewRetainedCache {
    /// Reuses filtering only when the mesh revision, chunk grid, keys and excluded owners match.
    /// O(affected chunks + owners), independent of resident city size; payload reuse is O(1).
    pub(crate) fn reuse(&self, scene: &mut RoadJunctionPreview) -> bool {
        let Some(entry) = &self.entry else {
            return false;
        };
        if entry.source_generation != scene.source_mesh_generation
            || entry.grid
                != (
                    scene.chunk_span_m,
                    scene.chunk_origin_x_m,
                    scene.chunk_origin_z_m,
                )
            || entry.chunks != scene.replacement_chunks
            || entry.removed != scene.removed
            || entry.bounded != scene.bounded
            || entry.bounds != scene.bounds
        {
            return false;
        }
        scene.retained = Arc::clone(&entry.meshes);
        scene.retained_revision = self.revision;
        true
    }

    /// Publishes one newly filtered entry; replaces rather than accumulating drag history.
    pub(crate) fn store(&mut self, scene: &mut RoadJunctionPreview) {
        self.revision = self.revision.wrapping_add(1).max(1);
        scene.retained_revision = self.revision;
        self.entry = Some(RetainedEntry {
            source_generation: scene.source_mesh_generation,
            grid: (
                scene.chunk_span_m,
                scene.chunk_origin_x_m,
                scene.chunk_origin_z_m,
            ),
            chunks: scene.replacement_chunks.clone(),
            removed: scene.removed.clone(),
            bounded: scene.bounded.clone(),
            bounds: scene.bounds.clone(),
            meshes: Arc::clone(&scene.retained),
        });
    }
}

#[cfg(test)]
mod tests;
