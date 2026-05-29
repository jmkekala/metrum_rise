//! Network-specific rendering logic for Godot interaction.
//!
//! Handles road mesh generation and road connection utility calculations.

use crate::debug_log;
use crate::nodes::sim::core::{RoadPreviewSnapshot, SimCore};
use godot::prelude::*;
use std::time::Instant;

impl SimCore {
    // ── Network Renderer ──

    /// Returns dictionary of road/intersection mesh data.
    pub fn get_road_mesh_data_internal(&mut self) -> VarDictionary {
        let mesh_data = self
            .transit_network
            .generate_mesh_data(&self.region_graph, &self.heightmap);
        let mut dict = VarDictionary::new();
        dict.set(
            "earthwork_vertices",
            PackedVector3Array::from_iter(mesh_data.earthwork_vertices),
        );
        dict.set(
            "earthwork_normals",
            PackedVector3Array::from_iter(mesh_data.earthwork_normals),
        );
        dict.set(
            "earthwork_uvs",
            PackedVector2Array::from_iter(mesh_data.earthwork_uvs),
        );
        dict.set(
            "earthwork_colors",
            PackedColorArray::from_iter(mesh_data.earthwork_colors),
        );
        dict.set(
            "curb_vertices",
            PackedVector3Array::from_iter(mesh_data.curb_vertices),
        );
        dict.set(
            "curb_normals",
            PackedVector3Array::from_iter(mesh_data.curb_normals),
        );
        dict.set(
            "curb_uvs",
            PackedVector2Array::from_iter(mesh_data.curb_uvs),
        );
        dict.set(
            "curb_colors",
            PackedColorArray::from_iter(mesh_data.curb_colors),
        );
        dict.set(
            "raised_step_vertices",
            PackedVector3Array::from_iter(mesh_data.raised_step_vertices),
        );
        dict.set(
            "raised_step_normals",
            PackedVector3Array::from_iter(mesh_data.raised_step_normals),
        );
        dict.set(
            "raised_step_uvs",
            PackedVector2Array::from_iter(mesh_data.raised_step_uvs),
        );
        dict.set(
            "raised_step_colors",
            PackedColorArray::from_iter(mesh_data.raised_step_colors),
        );
        dict.set(
            "sidewalk_vertices",
            PackedVector3Array::from_iter(mesh_data.sidewalk_vertices),
        );
        dict.set(
            "sidewalk_normals",
            PackedVector3Array::from_iter(mesh_data.sidewalk_normals),
        );
        dict.set(
            "sidewalk_uvs",
            PackedVector2Array::from_iter(mesh_data.sidewalk_uvs),
        );
        dict.set(
            "sidewalk_colors",
            PackedColorArray::from_iter(mesh_data.sidewalk_colors),
        );
        dict.set(
            "road_vertices",
            PackedVector3Array::from_iter(mesh_data.road_vertices),
        );
        dict.set(
            "road_normals",
            PackedVector3Array::from_iter(mesh_data.road_normals),
        );
        dict.set(
            "road_uvs",
            PackedVector2Array::from_iter(mesh_data.road_uvs),
        );
        dict.set(
            "road_colors",
            PackedColorArray::from_iter(mesh_data.road_colors),
        );

        dict.set(
            "marking_vertices",
            PackedVector3Array::from_iter(mesh_data.marking_vertices),
        );
        dict.set(
            "marking_normals",
            PackedVector3Array::from_iter(mesh_data.marking_normals),
        );
        dict.set(
            "marking_uvs",
            PackedVector2Array::from_iter(mesh_data.marking_uvs),
        );
        dict.set(
            "marking_colors",
            PackedColorArray::from_iter(mesh_data.marking_colors),
        );

        dict.set(
            "concrete_vertices",
            PackedVector3Array::from_iter(mesh_data.concrete_vertices),
        );
        dict.set(
            "concrete_normals",
            PackedVector3Array::from_iter(mesh_data.concrete_normals),
        );
        dict.set(
            "concrete_uvs",
            PackedVector2Array::from_iter(mesh_data.concrete_uvs),
        );
        dict.set(
            "concrete_colors",
            PackedColorArray::from_iter(mesh_data.concrete_colors),
        );
        dict
    }

    /// Compiles temporary road-tool preview data without touching Godot objects.
    pub(crate) fn compile_road_preview_snapshot(
        &self,
        request_id: u64,
        points: Vec<Vector3>,
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> RoadPreviewSnapshot {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let point_count = points.len();
        let compile_start = road_debug.then(Instant::now);
        let preview = self
            .transit_network
            .road_surface
            .compile_preview_surface_mesh_only(
                &points,
                fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                &self.heightmap,
            );
        let compile_ms = compile_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let prepared_count = preview.prepared_points.len();
        let surface_vertex_count = preview.surface_vertices.len();
        let is_valid = preview.is_valid;
        let total_ms = total_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug && total_ms >= 50.0 {
            debug_log!(
                "road",
                "preview_surface_rust request_id={} points={} prepared_points={} surface_vertices={} valid={} compile_ms={:.3} total_ms={:.3}",
                request_id,
                point_count,
                prepared_count,
                surface_vertex_count,
                is_valid,
                compile_ms,
                total_ms
            );
        }
        RoadPreviewSnapshot {
            request_id,
            prepared_points: preview.prepared_points,
            surface_vertices: preview.surface_vertices,
            is_valid,
        }
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

    /// Returns terrain render-patch keys that must keep full mesh resolution over compiled road ownership.
    pub fn get_road_locked_terrain_patches_internal(&mut self) -> PackedInt32Array {
        self.transit_network
            .road_surface
            .compile_dirty(&self.region_graph, &self.heightmap);
        let keys = self
            .transit_network
            .road_surface
            .terrain_render_patch_keys_with_visible_road(&self.region_graph, &self.heightmap);

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
