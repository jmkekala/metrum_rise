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

    pub(in crate::nodes::simulation_node) fn refined_terrain_patch_dict(
        core: &crate::nodes::sim::core::SimCore,
        patch_x: usize,
        patch_z: usize,
        render_step_m: f32,
        include_debug: bool,
    ) -> VarDictionary {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let snapshot_start = road_debug.then(Instant::now);
        let Some(base_patch) = core.heightmap.visual_patch_snapshot(patch_x, patch_z) else {
            return VarDictionary::new();
        };
        let snapshot_ms = snapshot_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let dict_start = road_debug.then(Instant::now);
        let mut dict = Self::terrain_patch_dict(&base_patch);
        let base_dict_ms = dict_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let clip_start = road_debug.then(Instant::now);
        let road_clip_query = Self::road_clip_loop_query_for_bounds(
            core,
            base_patch.world_origin_x,
            base_patch.world_origin_z,
            base_patch.world_origin_x + base_patch.world_size_x,
            base_patch.world_origin_z + base_patch.world_size_z,
        );
        let clip_ms = clip_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let clip_loops = road_clip_query.cdt_road_loops.len();
        let clip_points: usize = road_clip_query
            .cdt_road_loops
            .iter()
            .map(|road_loop| road_loop.vertices.len())
            .sum();
        let clip_dict_start = road_debug.then(Instant::now);
        if include_debug {
            Self::append_road_clip_query(&mut dict, &road_clip_query);
        } else {
            Self::append_road_clip_status(&mut dict, &road_clip_query);
        }
        let clip_dict_ms = clip_dict_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let cdt_input_start = road_debug.then(Instant::now);
        let cdt_input = Self::terrain_cdt_input(
            &core.heightmap,
            &base_patch,
            &road_clip_query.cdt_road_loops,
            safe_render_step_m,
            Some(TerrainCdtSiteGradingContext {
                allocator: &core.allocator,
                graph: &core.region_graph,
                road_surface: &core.transit_network.road_surface,
            }),
        );
        let cdt_input_ms = cdt_input_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let cdt_source_samples = cdt_input.source_samples.len();
        let cdt_append_start = road_debug.then(Instant::now);
        Self::append_cdt_terrain_mesh(
            &mut dict,
            &base_patch,
            cdt_input,
            safe_render_step_m,
            road_clip_query.source_count > 0,
            true,
            road_clip_query.clip_error_label,
            include_debug,
        );
        let cdt_append_ms = cdt_append_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "refined_patch key=({},{}) include_debug={} snapshot_ms={:.3} base_dict_ms={:.3} clip_query_ms={:.3} clip_dict_ms={:.3} cdt_input_ms={:.3} cdt_append_ms={:.3} total_ms={:.3} clip_loops={} clip_points={} cdt_source_samples={}",
                patch_x,
                patch_z,
                include_debug,
                snapshot_ms,
                base_dict_ms,
                clip_ms,
                clip_dict_ms,
                cdt_input_ms,
                cdt_append_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0),
                clip_loops,
                clip_points,
                cdt_source_samples
            );
        }
        dict
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
            source_count: cached.road_clip_source_count,
            clip_error_label: cached.clip_error_label,
        };
        Self::append_road_clip_status(&mut dict, &road_clip_query);
        Self::append_cached_cdt_terrain_mesh(&mut dict, cached, include_debug);
        dict
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_payload_dict(
        payload: &TerrainPatchPayload,
    ) -> VarDictionary {
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
            "surface_generation",
            i64::try_from(payload.surface_generation).unwrap_or(i64::MAX),
        );
        dict
    }
}
