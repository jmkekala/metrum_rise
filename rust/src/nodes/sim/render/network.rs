// SPDX-License-Identifier: GPL-2.0-only

//! Network-specific rendering logic for Godot interaction.
//!
//! Handles road chunk generation, publication and debug exports.

use crate::nodes::sim::core::SimCore;
use crate::simulation::network::render::NetworkMeshData;
use crate::simulation::network::roadbed::SurfaceChunkKey;
use crate::{debug, debug_log};
use godot::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

impl SimCore {
    // ── Network Renderer ──

    /// Regenerates only dirty road mesh chunks after a network edit on the sim thread.
    pub(crate) fn precompute_road_mesh_data(&mut self) {
        let road_debug = debug::Channel::Road.enabled();
        let total_start = road_debug.then(Instant::now);
        self.transit_network
            .roadbed
            .compile_dirty(&self.region_graph);
        // A node piece that failed keeps the last good chunks published.
        let roadbed = &self.transit_network.roadbed;
        if !roadbed.is_current() || !roadbed.failed_nodes().is_empty() {
            return;
        }
        self.transit_network
            .sync_new_lane_heights(&self.region_graph);
        let mut target_chunks = BTreeSet::new();
        if self.road_mesh_full_replace {
            target_chunks.extend(self.transit_network.roadbed.occupied_chunks());
            target_chunks.extend(self.cached_road_mesh_chunks.keys().copied());
        } else {
            target_chunks.extend(self.transit_network.roadbed.changed_chunks());
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
                        let mesh = mesh.seal();
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
            self.transit_network.roadbed.clear_changed_chunks();
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
        chunk_origin_x_m: f32,
        chunk_origin_z_m: f32,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set(
            "surface_generation",
            i64::try_from(generation).unwrap_or(i64::MAX),
        );
        dict.set("full_replace", full_replace);
        dict.set("chunk_span_m", chunk_span_m);
        dict.set("chunk_origin_x_m", chunk_origin_x_m);
        dict.set("chunk_origin_z_m", chunk_origin_z_m);

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
        // Certified off the Godot main thread; Godot still checks Variant types and counts.
        dict.set(
            "road_mesh_payload_validated",
            mesh_data.render_payload_validated(),
        );
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
            "road_crossings",
            PackedFloat32Array::from_iter(
                mesh_data
                    .road_crossings
                    .iter()
                    .flat_map(|v| [v.x, v.y, v.z, v.w]),
            ),
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
        // Five floats per light: chunk-local pole base x, y, z, yaw, model kind.
        dict.set(
            "street_lights",
            PackedFloat32Array::from_iter(mesh_data.street_lights.iter().flat_map(|light| {
                [
                    light.position.x,
                    light.position.y,
                    light.position.z,
                    light.yaw,
                    f32::from(light.kind as u8),
                ]
            })),
        );
        // Same five-float pose contract as street lights, with RoadSignKind indices.
        dict.set(
            "road_signs",
            PackedFloat32Array::from_iter(mesh_data.road_signs.iter().flat_map(|sign| {
                [
                    sign.position.x,
                    sign.position.y,
                    sign.position.z,
                    sign.yaw,
                    f32::from(sign.kind as u8),
                ]
            })),
        );
        dict
    }

    /// Returns road-surface debug line data for editor visualization. `ROAD-68-A` has no pieces,
    /// so every line list is empty.
    pub fn get_road_surface_debug_data_internal(&mut self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        for key in [
            "section_lines",
            "band_lines",
            "piece_boundary_lines",
            "earthwork_chunk_lines",
        ] {
            dict.set(key, PackedVector3Array::new());
        }
        dict
    }

    /// Returns a JSON debug dump of final road-surface triangles at one world-space probe point.
    /// `ROAD-68-A` has no pieces, so the dump lists none.
    pub fn get_road_surface_probe_debug_internal(&mut self, world_pos: Vector3) -> GString {
        GString::from(
            format!(
                "{{\"probe\":[{},{},{}],\"triangles\":[]}}",
                world_pos.x, world_pos.y, world_pos.z
            )
            .as_str(),
        )
    }
}
