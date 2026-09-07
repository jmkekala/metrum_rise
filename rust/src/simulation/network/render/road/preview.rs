// SPDX-License-Identifier: GPL-2.0-only

//! Local junction preview export and reversible replacement of cached source owners.

use super::{
    MeshLayer, NetworkMeshData, NetworkMeshOwner, RoadRenderer, earthwork_color, push_triangle,
};
use crate::config::{HEIGHT_SCALE, ROAD_DECAL_RENDER_Z_BIAS_M};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::surface::{
    CURB_STEP_HEIGHT_M, RoadSurfaceSystem, RoadVec3, SurfaceChunkKey,
};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

mod contacts;

/// The already-compiled validation neighborhood, moved rather than compiled a second time.
pub(crate) struct RoadPreviewRenderInput {
    graph: RegionGraph,
    surface: RoadSurfaceSystem,
    removed: HashSet<NetworkMeshOwner>,
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
}

impl RoadPreviewRenderInput {
    /// Resolves local validation IDs back to the existing mesh owners before any render export.
    pub(crate) fn new(
        graph: RegionGraph,
        surface: RoadSurfaceSystem,
        source_edges: HashMap<usize, usize>,
        removed_nodes: HashSet<u32>,
    ) -> Self {
        let mut removed = HashSet::new();
        for (source, local) in source_edges {
            if graph.edge(local).deleted || surface.compiled_visual_span_pieces.contains_key(&local)
            {
                removed.insert(NetworkMeshOwner::Edge(source));
            }
        }
        removed.extend(removed_nodes.into_iter().map(NetworkMeshOwner::Node));
        Self {
            graph,
            surface,
            removed,
        }
    }

    /// Exports only the local compiled scene. Lane reconstruction is confined to this excerpt;
    /// it supplies the same crosswalks and lane-divider clipping as the committed renderer.
    pub(crate) fn render(
        mut self,
        terrain: &TerrainSystem,
        existing_graph: &RegionGraph,
        existing: &RoadSurfaceSystem,
    ) -> Option<RoadJunctionPreview> {
        let infill = existing.preview_vacated_ground(
            existing_graph,
            terrain,
            self.removed.iter().filter_map(|owner| match owner {
                NetworkMeshOwner::Edge(edge) => Some(*edge),
                _ => None,
            }),
            self.removed.iter().filter_map(|owner| match owner {
                NetworkMeshOwner::Node(node) => Some(*node),
                _ => None,
            }),
            &self.graph,
            &self.surface,
        )?;
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
            chunks.extend(surface_chunks.into_iter().flatten().copied());
            chunks.extend(earthwork_chunks.into_iter().flatten().copied());
        }
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut self.graph);
        let mut planned = RoadRenderer.generate_mesh_chunks_with_surface(
            &self.graph,
            &mut lanes,
            terrain,
            &self.surface,
            &chunks,
        );
        let (origin_x, origin_z) = self.surface.chunk_origin_m();
        // Committed terrain is cut out beneath existing roads and stitched to their edges.
        // Those contacts must not hover. Only new coverage needs terrain clearance; adjacent
        // layers share one decision at equal world-XZ positions.
        planned.par_iter_mut().for_each(|(chunk, mesh)| {
            lift_display_mesh(
                mesh,
                terrain,
                existing_graph,
                existing,
                &infill.boundaries,
                origin_x + chunk.0 as f32 * self.surface.chunk_span_m(),
                origin_z + chunk.1 as f32 * self.surface.chunk_span_m(),
            );
        });
        // A bend can shrink the former terminal's footprint. The old terrain cutout remains
        // resident, so close its vacated area with temporary ground, not the old road markings.
        // Append after lifting: this infill stays at the committed road/terrain contact height.
        for triangle in infill.triangles {
            let center = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
            let span = self.surface.chunk_span_m();
            let key = (
                ((center.x - f64::from(origin_x)) / f64::from(span)).floor() as i32,
                ((center.z - f64::from(origin_z)) / f64::from(span)).floor() as i32,
            );
            chunks.insert(key);
            let vertices = triangle.map(|point| {
                Vector3::new(
                    point.x as f32 - origin_x - key.0 as f32 * span,
                    point.y as f32,
                    point.z as f32 - origin_z - key.1 as f32 * span,
                )
            });
            push_triangle(
                planned.entry(key).or_insert_with(NetworkMeshData::new),
                MeshLayer::Earthwork,
                vertices,
                triangle.map(|point| Vector2::new(point.x as f32, point.z as f32)),
                earthwork_color(),
            );
        }
        Some(RoadJunctionPreview {
            source_mesh_generation: 0,
            planned: planned
                .into_iter()
                .map(|(key, mesh)| (key, Arc::new(mesh)))
                .collect(),
            retained: Arc::new(BTreeMap::new()),
            retained_revision: 0,
            replacement_chunks: chunks,
            chunk_span_m: self.surface.chunk_span_m(),
            chunk_origin_x_m: origin_x,
            chunk_origin_z_m: origin_z,
            removed: self.removed,
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
                    let retained = mesh.without_owners(&self.removed);
                    (!retained.is_empty()).then(|| (*key, Arc::new(retained)))
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
            meshes: Arc::clone(&scene.retained),
        });
    }
}

fn lift_display_mesh(
    mesh: &mut NetworkMeshData,
    terrain: &TerrainSystem,
    existing_graph: &RegionGraph,
    existing: &RoadSurfaceSystem,
    boundaries: &[[RoadVec3; 2]],
    origin_x: f32,
    origin_z: f32,
) {
    let key = |point: &Vector3| (point.x.to_bits(), point.z.to_bits());
    let mut base_heights: HashMap<(u32, u32), f32> = HashMap::new();
    for (vertices, band_height) in [
        (&mesh.road_vertices, 0.0),
        (&mesh.sidewalk_vertices, CURB_STEP_HEIGHT_M),
        (&mesh.curb_vertices, 0.0),
        (&mesh.raised_step_vertices, 0.0),
        (&mesh.marking_vertices, ROAD_DECAL_RENDER_Z_BIAS_M),
    ] {
        for point in vertices {
            base_heights
                .entry(key(point))
                .and_modify(|height| *height = height.min(point.y - band_height))
                .or_insert(point.y - band_height);
        }
    }
    // Subgrade support faces must not lift the pavement at the same XZ coordinate.
    for point in mesh
        .earthwork_vertices
        .iter()
        .chain(&mesh.concrete_vertices)
    {
        base_heights.entry(key(point)).or_insert(point.y);
    }
    for ((x, z), lift) in &mut base_heights {
        let world_x = f32::from_bits(*x) + origin_x;
        let world_z = f32::from_bits(*z) + origin_z;
        // Reuse the committed surface's query chunks and owner-local triangle grids. Work is
        // bounded by local query contributors/cells, not the number of roads in the city.
        if existing
            .sample_visible_surface_height(existing_graph, terrain, world_x, world_z)
            .is_some()
        {
            *lift = 0.0;
            continue;
        }
        let terrain_height = terrain.sample_visual_height_world(world_x, world_z) * HEIGHT_SCALE;
        *lift = (terrain_height - *lift).max(0.0) + 0.15;
    }
    // Add explicit contact vertices before applying the offset. Merely pinning old vertices
    // lets a triangle straddling the cutout interpolate a positive lift across the terrain seam.
    contacts::split_mesh(mesh, &mut base_heights, boundaries, origin_x, origin_z);
    // Retessellation also creates interior vertices, not just boundary contacts. An
    // interpolated offset is not a coverage decision: pin new vertices over the old road too.
    for (&(x, z), offset) in &mut base_heights {
        if *offset != 0.0
            && existing
                .sample_visible_surface_height(
                    existing_graph,
                    terrain,
                    f32::from_bits(x) + origin_x,
                    f32::from_bits(z) + origin_z,
                )
                .is_some()
        {
            *offset = 0.0;
        }
    }
    macro_rules! lift {
        ($vertices:ident, $normals:ident) => {{
            for point in &mut mesh.$vertices {
                let offset = base_heights[&key(point)];
                if offset != 0.0 {
                    point.y += offset;
                }
            }
            for (triangle, normals) in mesh
                .$vertices
                .chunks_exact(3)
                .zip(mesh.$normals.chunks_exact_mut(3))
            {
                if triangle
                    .iter()
                    .all(|point| base_heights[&key(point)] == 0.0)
                {
                    continue;
                }
                if let Some(mut normal) = (triangle[1] - triangle[0])
                    .cross(triangle[2] - triangle[0])
                    .try_normalized()
                {
                    if normal.dot(normals[0]) < 0.0 {
                        normal = -normal;
                    }
                    normals.fill(normal);
                }
            }
        }};
    }
    lift!(earthwork_vertices, earthwork_normals);
    lift!(curb_vertices, curb_normals);
    lift!(raised_step_vertices, raised_step_normals);
    lift!(sidewalk_vertices, sidewalk_normals);
    lift!(road_vertices, road_normals);
    lift!(marking_vertices, marking_normals);
    lift!(concrete_vertices, concrete_normals);
}

#[cfg(test)]
mod tests;
