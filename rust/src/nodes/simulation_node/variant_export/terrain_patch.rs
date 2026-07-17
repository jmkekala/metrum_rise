//! Terrain patch and refined terrain variant export helpers.

use super::super::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn terrain_patch_dict(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
    ) -> VarDictionary {
        let mut dict = Self::terrain_patch_metadata_dict(patch);
        dict.set(
            "height_data",
            PackedFloat32Array::from_iter(patch.height_data.iter().copied()),
        );
        dict.set("height_bytes", Self::packed_f32_bytes(&patch.height_data));
        dict
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_metadata_dict(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
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
        dict
    }

    pub(in crate::nodes::simulation_node) fn f32_bytes_vec(values: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(values.len().saturating_mul(std::mem::size_of::<f32>()));
        for value in values {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        bytes
    }

    pub(in crate::nodes::simulation_node) fn packed_f32_bytes(values: &[f32]) -> PackedByteArray {
        let bytes = Self::f32_bytes_vec(values);
        PackedByteArray::from_iter(bytes)
    }

    pub(in crate::nodes::simulation_node) fn refined_patch_cache_key(
        patch_x: usize,
        patch_z: usize,
        render_step_m: f32,
    ) -> RefinedTerrainPatchCacheKey {
        RefinedTerrainPatchCacheKey {
            patch_x,
            patch_z,
            render_step_mm: (render_step_m.max(f32::EPSILON) * 1000.0).round() as u32,
        }
    }

    pub(in crate::nodes::simulation_node) fn cached_refined_terrain_patch_dict(
        cached: &CachedRefinedTerrainPatch,
        include_debug: bool,
    ) -> VarDictionary {
        let mut dict = Self::terrain_patch_dict(&cached.patch);
        let road_clip_query = RoadClipLoopQuery {
            cdt_road_loops: Vec::new(),
            source_count: cached.clip_source_count,
            road_source_count: cached.road_clip_source_count,
            road_loop_count: cached.road_clip_loop_count,
            site_loop_count: cached.site_clip_loop_count,
            clip_error_label: cached.clip_error_label,
        };
        Self::append_road_clip_status(&mut dict, &road_clip_query);
        Self::append_cached_cdt_terrain_mesh(&mut dict, cached, include_debug);
        dict
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_payload_dict(
        payload: &TerrainPatchPayload,
    ) -> VarDictionary {
        let requires_engineered_refinement = match &payload.data {
            TerrainPatchPayloadData::Regular { .. } => false,
            TerrainPatchPayloadData::Refined { patch } => patch.requires_engineered_refinement,
        };
        let mut dict = match &payload.data {
            TerrainPatchPayloadData::Regular {
                patch,
                height_bytes,
            } => {
                let mut dict = Self::terrain_patch_metadata_dict(patch);
                dict.set(
                    "height_bytes",
                    PackedByteArray::from_iter(height_bytes.iter().copied()),
                );
                dict
            }
            TerrainPatchPayloadData::Refined { patch } => {
                Self::cached_refined_terrain_patch_dict(patch, false)
            }
        };
        dict.set("render_step_mm", i64::from(payload.key.render_step_mm));
        dict.set(
            "terrain_requires_engineered_refinement",
            requires_engineered_refinement,
        );
        dict.set(
            "surface_generation",
            i64::try_from(payload.surface_generation).unwrap_or(i64::MAX),
        );
        dict
    }
}
