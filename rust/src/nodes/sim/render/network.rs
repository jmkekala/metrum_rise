//! Network-specific rendering logic for Godot interaction.
//!
//! Handles road mesh generation and road connection utility calculations.

use crate::nodes::sim::core::SimCore;
use crate::simulation::network::render::NetworkMeshData;
use crate::simulation::network::surface::{RoadSurfaceCompileReason, SurfaceChunkKey};
use crate::{debug, debug_log};
use godot::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

impl SimCore {
    // ── Network Renderer ──

    /// Regenerates only dirty road mesh chunks after a network edit on the sim thread.
    pub(crate) fn precompute_road_mesh_data(&mut self) {
        let road_debug = debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::MeshPrecompute,
        );
        if !self
            .transit_network
            .road_surface
            .published_generation_matches_source()
        {
            return;
        }
        let mut target_chunks = BTreeSet::new();
        if self.road_mesh_full_replace {
            target_chunks.extend(
                self.transit_network
                    .road_surface
                    .surface_chunk_cache()
                    .keys()
                    .copied(),
            );
            target_chunks.extend(
                self.transit_network
                    .road_surface
                    .earthwork_chunk_cache()
                    .keys()
                    .copied(),
            );
            target_chunks.extend(self.cached_road_mesh_chunks.keys().copied());
        } else {
            target_chunks.extend(
                self.transit_network
                    .road_surface
                    .last_rebuilt_surface_chunks
                    .iter()
                    .copied(),
            );
            target_chunks.extend(
                self.transit_network
                    .road_surface
                    .last_rebuilt_terrain_chunks
                    .iter()
                    .copied(),
            );
        }

        let mut rebuilt_vertex_count = 0;
        if let Some(mut generated_chunks) = self.transit_network.try_generate_mesh_chunks(
            &self.region_graph,
            &self.heightmap,
            &target_chunks,
        ) {
            let published_chunks = Arc::make_mut(&mut self.published_road_mesh_chunks);
            if self.road_mesh_full_replace {
                published_chunks.clear();
            }
            for chunk in &target_chunks {
                if let Some(mesh) = generated_chunks.remove(chunk) {
                    if mesh.is_empty() {
                        self.cached_road_mesh_chunks.remove(chunk);
                        published_chunks.remove(chunk);
                    } else {
                        rebuilt_vertex_count += mesh.vertex_count();
                        let mesh = Arc::new(mesh);
                        self.cached_road_mesh_chunks
                            .insert(*chunk, Arc::clone(&mesh));
                        published_chunks.insert(*chunk, mesh);
                    }
                } else {
                    self.cached_road_mesh_chunks.remove(chunk);
                    published_chunks.remove(chunk);
                }
            }
            Arc::make_mut(&mut self.pending_road_mesh_chunks).extend(target_chunks.iter().copied());
            self.cached_road_mesh_generation = self.road_tool_surface_generation;
        }
        if road_debug {
            debug_log!(
                "road",
                "road_mesh_precompute dirty_chunks={} cached_chunks={} rebuilt_vertices={} total_ms={:.3}",
                target_chunks.len(),
                self.cached_road_mesh_chunks.len(),
                rebuilt_vertex_count,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }

    pub(crate) fn road_mesh_chunks_dict(
        chunks: &BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>,
        pending_chunks: &BTreeSet<SurfaceChunkKey>,
        full_replace: bool,
        generation: u64,
        chunk_span_m: f32,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set(
            "surface_generation",
            i64::try_from(generation).unwrap_or(i64::MAX),
        );
        dict.set("full_replace", full_replace);
        dict.set("chunk_span_m", chunk_span_m);

        let mut updates = VarArray::new();
        if full_replace {
            for (&chunk, mesh) in chunks {
                updates.push(&Self::road_mesh_chunk_dict(chunk, mesh).to_variant());
            }
        } else {
            for &chunk in pending_chunks {
                let update = chunks
                    .get(&chunk)
                    .map(|mesh| Self::road_mesh_chunk_dict(chunk, mesh))
                    .unwrap_or_else(|| {
                        let mut removed = VarDictionary::new();
                        removed.set("chunk_x", chunk.0);
                        removed.set("chunk_z", chunk.1);
                        removed.set("removed", true);
                        removed
                    });
                updates.push(&update.to_variant());
            }
        }
        dict.set("chunks", updates);
        dict
    }

    fn road_mesh_chunk_dict(chunk: SurfaceChunkKey, mesh_data: &NetworkMeshData) -> VarDictionary {
        let mut dict = Self::network_mesh_data_dict(mesh_data);
        dict.set("chunk_x", chunk.0);
        dict.set("chunk_z", chunk.1);
        dict.set("removed", false);
        dict
    }

    pub(crate) fn network_mesh_data_dict(mesh_data: &NetworkMeshData) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set(
            "earthwork_vertices",
            PackedVector3Array::from_iter(mesh_data.earthwork_vertices.iter().copied()),
        );
        dict.set(
            "earthwork_normals",
            PackedVector3Array::from_iter(mesh_data.earthwork_normals.iter().copied()),
        );
        dict.set(
            "earthwork_uvs",
            PackedVector2Array::from_iter(mesh_data.earthwork_uvs.iter().copied()),
        );
        dict.set(
            "earthwork_colors",
            PackedColorArray::from_iter(mesh_data.earthwork_colors.iter().copied()),
        );
        dict.set(
            "curb_vertices",
            PackedVector3Array::from_iter(mesh_data.curb_vertices.iter().copied()),
        );
        dict.set(
            "curb_normals",
            PackedVector3Array::from_iter(mesh_data.curb_normals.iter().copied()),
        );
        dict.set(
            "curb_uvs",
            PackedVector2Array::from_iter(mesh_data.curb_uvs.iter().copied()),
        );
        dict.set(
            "curb_colors",
            PackedColorArray::from_iter(mesh_data.curb_colors.iter().copied()),
        );
        dict.set(
            "raised_step_vertices",
            PackedVector3Array::from_iter(mesh_data.raised_step_vertices.iter().copied()),
        );
        dict.set(
            "raised_step_normals",
            PackedVector3Array::from_iter(mesh_data.raised_step_normals.iter().copied()),
        );
        dict.set(
            "raised_step_uvs",
            PackedVector2Array::from_iter(mesh_data.raised_step_uvs.iter().copied()),
        );
        dict.set(
            "raised_step_colors",
            PackedColorArray::from_iter(mesh_data.raised_step_colors.iter().copied()),
        );
        dict.set(
            "sidewalk_vertices",
            PackedVector3Array::from_iter(mesh_data.sidewalk_vertices.iter().copied()),
        );
        dict.set(
            "sidewalk_normals",
            PackedVector3Array::from_iter(mesh_data.sidewalk_normals.iter().copied()),
        );
        dict.set(
            "sidewalk_uvs",
            PackedVector2Array::from_iter(mesh_data.sidewalk_uvs.iter().copied()),
        );
        dict.set(
            "sidewalk_colors",
            PackedColorArray::from_iter(mesh_data.sidewalk_colors.iter().copied()),
        );
        dict.set(
            "road_vertices",
            PackedVector3Array::from_iter(mesh_data.road_vertices.iter().copied()),
        );
        dict.set(
            "road_normals",
            PackedVector3Array::from_iter(mesh_data.road_normals.iter().copied()),
        );
        dict.set(
            "road_uvs",
            PackedVector2Array::from_iter(mesh_data.road_uvs.iter().copied()),
        );
        dict.set(
            "road_colors",
            PackedColorArray::from_iter(mesh_data.road_colors.iter().copied()),
        );

        dict.set(
            "marking_vertices",
            PackedVector3Array::from_iter(mesh_data.marking_vertices.iter().copied()),
        );
        dict.set(
            "marking_normals",
            PackedVector3Array::from_iter(mesh_data.marking_normals.iter().copied()),
        );
        dict.set(
            "marking_uvs",
            PackedVector2Array::from_iter(mesh_data.marking_uvs.iter().copied()),
        );
        dict.set(
            "marking_colors",
            PackedColorArray::from_iter(mesh_data.marking_colors.iter().copied()),
        );

        dict.set(
            "concrete_vertices",
            PackedVector3Array::from_iter(mesh_data.concrete_vertices.iter().copied()),
        );
        dict.set(
            "concrete_normals",
            PackedVector3Array::from_iter(mesh_data.concrete_normals.iter().copied()),
        );
        dict.set(
            "concrete_uvs",
            PackedVector2Array::from_iter(mesh_data.concrete_uvs.iter().copied()),
        );
        dict.set(
            "concrete_colors",
            PackedColorArray::from_iter(mesh_data.concrete_colors.iter().copied()),
        );
        dict
    }

    /// Returns compiled road-surface debug line data for editor visualization.
    pub fn get_road_surface_debug_data_internal(&mut self) -> VarDictionary {
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::MeshPrecompute,
        );
        let debug = self
            .transit_network
            .road_surface
            .build_debug_line_data(&self.region_graph, &self.heightmap);

        let mut dict = VarDictionary::new();
        dict.set(
            "section_lines",
            PackedVector3Array::from_iter(debug.section_lines),
        );
        dict.set(
            "band_lines",
            PackedVector3Array::from_iter(debug.band_lines),
        );
        dict.set(
            "piece_boundary_lines",
            PackedVector3Array::from_iter(debug.piece_boundary_lines),
        );
        dict.set(
            "earthwork_chunk_lines",
            PackedVector3Array::from_iter(debug.earthwork_chunk_lines),
        );
        dict
    }

    /// Returns a JSON debug dump of final road-surface triangles at one world-space probe point.
    pub fn get_road_surface_probe_debug_internal(&mut self, world_pos: Vector3) -> GString {
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::MeshPrecompute,
        );
        let dump = self
            .transit_network
            .road_surface
            .build_road_surface_probe_debug_dump(&self.region_graph, &self.heightmap, world_pos);
        GString::from(dump.as_str())
    }

    /// Calculates the normalized T-coordinates of the connection between two edges.
    pub fn get_connection_rust(&self, edge_a: usize, edge_b: usize) -> (f32, f32) {
        let (p_a0, _) = self.get_edge_pos_and_tangent(edge_a, 0.0);
        let (p_a1, _) = self.get_edge_pos_and_tangent(edge_a, 1.0);
        let (p_b0, _) = self.get_edge_pos_and_tangent(edge_b, 0.0);
        let (p_b1, _) = self.get_edge_pos_and_tangent(edge_b, 1.0);

        let thr = 400.0;
        if p_a1.distance_squared_to(p_b0) < thr {
            (1.0, 0.0)
        } else if p_a1.distance_squared_to(p_b1) < thr {
            (1.0, 1.0)
        } else if p_a0.distance_squared_to(p_b0) < thr {
            (0.0, 0.0)
        } else if p_a0.distance_squared_to(p_b1) < thr {
            (0.0, 1.0)
        } else {
            (1.0, 0.0)
        }
    }
}
