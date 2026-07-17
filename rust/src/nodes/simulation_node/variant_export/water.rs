//! Water patch, water mesh, and authored-water variant export helpers.

use super::super::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn water_patch_metadata_dict(
        patch: &crate::simulation::water::WaterPatchSnapshot,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("patch_x", i64::try_from(patch.patch_x).unwrap_or(0));
        dict.set("patch_z", i64::try_from(patch.patch_z).unwrap_or(0));
        dict.set(
            "sample_width",
            i64::try_from(patch.sample_width).unwrap_or(0),
        );
        dict.set(
            "sample_height",
            i64::try_from(patch.sample_height).unwrap_or(0),
        );
        dict.set(
            "texture_width",
            i64::try_from(patch.texture_width).unwrap_or(0),
        );
        dict.set(
            "texture_height",
            i64::try_from(patch.texture_height).unwrap_or(0),
        );
        dict.set(
            "inner_offset_x",
            i64::try_from(patch.inner_offset_x).unwrap_or(0),
        );
        dict.set(
            "inner_offset_z",
            i64::try_from(patch.inner_offset_z).unwrap_or(0),
        );
        dict.set("world_origin_x", f64::from(patch.world_origin_x));
        dict.set("world_origin_z", f64::from(patch.world_origin_z));
        dict.set("world_size_x", f64::from(patch.world_size_x));
        dict.set("world_size_z", f64::from(patch.world_size_z));
        dict.set(
            "depth_nonzero_count",
            i64::try_from(patch.depth_nonzero_count).unwrap_or(0),
        );
        dict.set(
            "depth_signature",
            i64::from_ne_bytes(water_patch_depth_signature(patch).to_ne_bytes()),
        );
        dict
    }

    pub(in crate::nodes::simulation_node) fn water_patch_payload_dict(
        payload: &WaterPatchPayload,
    ) -> VarDictionary {
        let mut dict = Self::water_patch_metadata_dict(&payload.patch);
        dict.set(
            "depth_bytes",
            PackedByteArray::from_iter(payload.depth_bytes.iter().copied()),
        );
        dict.set(
            "source_generation",
            i64::try_from(payload.source_generation).unwrap_or(i64::MAX),
        );
        dict.set(
            "surface_generation",
            i64::try_from(payload.surface_generation).unwrap_or(i64::MAX),
        );
        Self::append_road_clip_query(&mut dict, &payload.road_clip_query);
        dict
    }

    pub(in crate::nodes::simulation_node) fn water_patch_mesh_dict(
        mesh: &CachedWaterPatchMesh,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("patch_x", i64::try_from(mesh.key.patch_x).unwrap_or(0));
        dict.set("patch_z", i64::try_from(mesh.key.patch_z).unwrap_or(0));
        dict.set("lod_step", i64::try_from(mesh.key.lod_step).unwrap_or(1));
        dict.set("mesh_world_size_x", f64::from(mesh.world_size_x));
        dict.set("mesh_world_size_z", f64::from(mesh.world_size_z));
        dict.set("road_clip_signature", mesh.key.road_clip_signature);
        dict.set(
            "depth_signature",
            i64::from_ne_bytes(mesh.key.depth_signature.to_ne_bytes()),
        );
        dict.set(
            "vertices",
            PackedVector3Array::from_iter(mesh.vertices.iter().copied()),
        );
        dict.set(
            "normals",
            PackedVector3Array::from_iter(mesh.normals.iter().copied()),
        );
        dict.set(
            "uvs",
            PackedVector2Array::from_iter(mesh.uvs.iter().copied()),
        );
        dict.set(
            "indices",
            PackedInt32Array::from_iter(mesh.indices.iter().copied()),
        );
        dict.set(
            "mesh_cells",
            i64::try_from(mesh.stats.cells_total).unwrap_or(i64::MAX),
        );
        dict.set(
            "mesh_full_cells",
            i64::try_from(mesh.stats.full_cells).unwrap_or(i64::MAX),
        );
        dict.set(
            "mesh_partial_cells",
            i64::try_from(mesh.stats.partial_cells).unwrap_or(i64::MAX),
        );
        dict.set(
            "mesh_conservative_cells",
            i64::try_from(mesh.stats.conservative_cells).unwrap_or(i64::MAX),
        );
        dict.set(
            "mesh_dry_cells",
            i64::try_from(mesh.stats.dry_cells).unwrap_or(i64::MAX),
        );
        dict.set(
            "mesh_road_clipped_cells",
            i64::try_from(mesh.stats.road_clipped_cells).unwrap_or(i64::MAX),
        );
        dict.set(
            "mesh_emitted_vertices",
            i64::try_from(mesh.stats.emitted_vertices).unwrap_or(i64::MAX),
        );
        dict.set(
            "mesh_emitted_triangles",
            i64::try_from(mesh.stats.emitted_triangles).unwrap_or(i64::MAX),
        );
        let estimated_bytes = mesh
            .vertices
            .len()
            .saturating_mul(std::mem::size_of::<Vector3>())
            .saturating_add(
                mesh.normals
                    .len()
                    .saturating_mul(std::mem::size_of::<Vector3>()),
            )
            .saturating_add(
                mesh.uvs
                    .len()
                    .saturating_mul(std::mem::size_of::<Vector2>()),
            )
            .saturating_add(
                mesh.indices
                    .len()
                    .saturating_mul(std::mem::size_of::<i32>()),
            );
        dict.set(
            "mesh_estimated_bytes",
            i64::try_from(estimated_bytes).unwrap_or(i64::MAX),
        );
        dict
    }

    pub(in crate::nodes::simulation_node) fn water_patch_layer_debug_dict(
        stats: &crate::simulation::water::WaterPatchLayerStats,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set(
            "total_samples",
            i64::try_from(stats.total_samples).unwrap_or(0),
        );
        dict.set(
            "baseline_nonzero",
            i64::try_from(stats.baseline_nonzero).unwrap_or(0),
        );
        dict.set("baseline_max", f64::from(stats.baseline_max));
        dict.set("baseline_sum", f64::from(stats.baseline_sum));
        dict.set(
            "visible_nonzero",
            i64::try_from(stats.visible_nonzero).unwrap_or(0),
        );
        dict.set("visible_max", f64::from(stats.visible_max));
        dict.set("visible_sum", f64::from(stats.visible_sum));
        dict
    }

    pub(in crate::nodes::simulation_node) fn authored_water_patch_fill_debug_dict(
        fill: &crate::nodes::sim::core::AuthoredWaterPatchFillDebug,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set(
            "kind",
            GString::from(match fill.kind {
                WorldWaterFillKind::Lake => "lake",
                WorldWaterFillKind::OpenWater => "open_water",
            }),
        );
        dict.set("fill_index", i64::from(fill.fill_index));
        dict.set("preview", fill.preview);
        dict.set("world_x", f64::from(fill.world_x));
        dict.set("world_z", f64::from(fill.world_z));
        dict.set("surface_elevation_m", f64::from(fill.surface_elevation_m));
        dict.set(
            "filled_cells",
            i64::try_from(fill.filled_cells).unwrap_or(0),
        );
        dict.set("touches_world_edge", fill.touches_world_edge);
        dict.set(
            "patch_nonzero_samples",
            i64::try_from(fill.patch_nonzero_samples).unwrap_or(0),
        );
        dict.set("patch_max_depth_m", f64::from(fill.patch_max_depth_m));
        dict.set("patch_sum_depth_m", f64::from(fill.patch_sum_depth_m));
        dict
    }
}
