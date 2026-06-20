//! Network-specific rendering logic for Godot interaction.
//!
//! Handles road mesh generation and road connection utility calculations.

use crate::nodes::sim::core::{ROAD_LOCKED_TERRAIN_RENDER_STEP_M, SimCore};
use crate::simulation::network::render::NetworkMeshData;
use crate::simulation::terrain::terrain_cdt_local_sample_margin_m;
use crate::{debug, debug_log};
use godot::prelude::*;
use std::time::Instant;

impl SimCore {
    // ── Network Renderer ──

    /// Returns dictionary of road/intersection mesh data.
    pub fn get_road_mesh_data_internal(&mut self) -> VarDictionary {
        let road_debug = debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let cache_hit = self.cached_road_mesh_data.is_some();
        if self.cached_road_mesh_data.is_none() {
            self.cached_road_mesh_data = Some(
                self.transit_network
                    .generate_mesh_data(&self.region_graph, &self.heightmap),
            );
        }
        let Some(mesh_data) = self.cached_road_mesh_data.as_ref() else {
            return VarDictionary::new();
        };
        let vertex_count = Self::network_mesh_vertex_count(mesh_data);
        let dict = Self::network_mesh_data_dict(mesh_data);
        if road_debug {
            debug_log!(
                "road",
                "road_mesh_data cache_hit={} vertices={} total_ms={:.3}",
                cache_hit,
                vertex_count,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
        dict
    }

    /// Regenerates the full road mesh cache after a network edit on the sim thread.
    pub(crate) fn precompute_road_mesh_data(&mut self) {
        let road_debug = debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        self.cached_road_mesh_data = Some(
            self.transit_network
                .generate_mesh_data(&self.region_graph, &self.heightmap),
        );
        if road_debug {
            let vertex_count = self
                .cached_road_mesh_data
                .as_ref()
                .map(Self::network_mesh_vertex_count)
                .unwrap_or(0);
            debug_log!(
                "road",
                "road_mesh_precompute vertices={} total_ms={:.3}",
                vertex_count,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }

    fn network_mesh_data_dict(mesh_data: &NetworkMeshData) -> VarDictionary {
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

    fn network_mesh_vertex_count(mesh_data: &NetworkMeshData) -> usize {
        mesh_data.earthwork_vertices.len()
            + mesh_data.curb_vertices.len()
            + mesh_data.raised_step_vertices.len()
            + mesh_data.sidewalk_vertices.len()
            + mesh_data.road_vertices.len()
            + mesh_data.marking_vertices.len()
            + mesh_data.concrete_vertices.len()
    }

    /// Returns compiled road-surface debug line data for editor visualization.
    pub fn get_road_surface_debug_data_internal(&mut self) -> VarDictionary {
        self.transit_network
            .road_surface
            .compile_dirty(&self.region_graph, &self.heightmap);
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
        self.transit_network
            .road_surface
            .compile_dirty(&self.region_graph, &self.heightmap);
        let dump = self
            .transit_network
            .road_surface
            .build_road_surface_probe_debug_dump(&self.region_graph, &self.heightmap, world_pos);
        GString::from(dump.as_str())
    }

    /// Returns terrain render-patch keys that must keep full mesh resolution over compiled road ownership.
    pub fn get_road_locked_terrain_patches_internal(&self) -> PackedInt32Array {
        let margin_m =
            terrain_cdt_local_sample_margin_m(&self.heightmap, ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
        let mut keys = self.road_locked_terrain_patch_keys.clone();
        keys.extend(
            self.allocator
                .terrain_render_patch_keys_with_building_site_margin(&self.heightmap, margin_m),
        );
        keys.sort_unstable();
        keys.dedup();
        let mut packed = PackedInt32Array::new();
        for (patch_x, patch_z) in keys {
            packed.push(i32::try_from(patch_x).unwrap_or(i32::MAX));
            packed.push(i32::try_from(patch_z).unwrap_or(i32::MAX));
        }
        packed
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
