//! Network-specific rendering logic for Godot interaction.
//!
//! Handles road mesh generation and road connection utility calculations.

use crate::nodes::sim::core::SimCore;
use godot::prelude::*;

impl SimCore {
    // ── Network Renderer ──

    /// Returns dictionary of road/intersection mesh data.
    pub fn get_road_mesh_data_internal(&mut self) -> VarDictionary {
        let mesh_data = self
            .transit_network
            .generate_mesh_data(&self.region_graph, &self.heightmap);
        let mut dict = VarDictionary::new();
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

    /// Returns temporary compiled preview-surface data for the road tool.
    pub fn get_preview_road_surface_internal(
        &self,
        points: PackedVector3Array,
        fwd_lanes: u8,
        bkw_lanes: u8,
    ) -> VarDictionary {
        let preview = self.transit_network.road_surface.compile_preview_surface(
            &points.to_vec(),
            fwd_lanes,
            bkw_lanes,
            &self.heightmap,
        );

        let mut dict = VarDictionary::new();
        dict.set(
            "prepared_points",
            PackedVector3Array::from_iter(preview.prepared_points),
        );
        dict.set(
            "surface_vertices",
            PackedVector3Array::from_iter(preview.surface_vertices),
        );
        dict.set("is_valid", preview.is_valid);
        dict
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
            "node_patch_lines",
            PackedVector3Array::from_iter(debug.node_patch_lines),
        );
        dict.set(
            "earthwork_chunk_lines",
            PackedVector3Array::from_iter(debug.earthwork_chunk_lines),
        );
        dict
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
