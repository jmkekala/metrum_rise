//! Terrain CDT input, windowing, fingerprint, and sampling helpers.

use super::super::super::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn terrain_cdt_window_build_inputs(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
        site_grading: Option<TerrainCdtSiteGradingContext<'_>>,
        previous: Option<&CachedRefinedTerrainPatch>,
    ) -> Vec<RefinedTerrainCdtWindowBuildInput> {
        if road_loops.is_empty() {
            return Vec::new();
        }
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let previous_windows = previous
            .map(|cached| {
                cached
                    .windows
                    .iter()
                    .map(|window| (window.key, window.clone()))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();

        let mut loops_by_group = BTreeMap::<u64, Vec<TerrainCdtRoadLoop>>::new();
        for road_loop in road_loops {
            loops_by_group
                .entry(road_loop.footprint_group_id)
                .or_default()
                .push(road_loop.clone());
        }

        let mut drafts = loops_by_group
            .into_values()
            .filter_map(|loops| {
                Self::terrain_cdt_local_sample_bounds(terrain, patch, &loops, safe_render_step_m)
                    .map(|bounds| TerrainCdtLoopWindowDraft { bounds, loops })
            })
            .collect::<Vec<_>>();
        Self::merge_terrain_cdt_window_drafts(terrain, patch, safe_render_step_m, &mut drafts);

        drafts
            .into_iter()
            .filter_map(|draft| {
                let cdt_input = Self::terrain_cdt_input_for_bounds(
                    terrain,
                    patch,
                    &draft.loops,
                    safe_render_step_m,
                    draft.bounds,
                    site_grading,
                );
                if cdt_input.road_loops.is_empty() {
                    return None;
                }
                let key = Self::terrain_cdt_window_key(&cdt_input);
                Some(RefinedTerrainCdtWindowBuildInput {
                    key,
                    cdt_input,
                    previous: previous_windows.get(&key).cloned(),
                })
            })
            .collect()
    }

    pub(in crate::nodes::simulation_node) fn merge_terrain_cdt_window_drafts(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        render_step_m: f32,
        drafts: &mut Vec<TerrainCdtLoopWindowDraft>,
    ) {
        drafts.sort_by(|left, right| {
            left.bounds
                .0
                .total_cmp(&right.bounds.0)
                .then_with(|| left.bounds.1.total_cmp(&right.bounds.1))
                .then_with(|| left.bounds.2.total_cmp(&right.bounds.2))
                .then_with(|| left.bounds.3.total_cmp(&right.bounds.3))
        });
        let mut merged: Vec<TerrainCdtLoopWindowDraft> = Vec::new();
        'drafts: for mut draft in drafts.drain(..) {
            for existing in &mut merged {
                if Self::terrain_cdt_window_bounds_overlap(existing.bounds, draft.bounds) {
                    existing.loops.append(&mut draft.loops);
                    if let Some(bounds) = Self::terrain_cdt_local_sample_bounds(
                        terrain,
                        patch,
                        &existing.loops,
                        render_step_m,
                    ) {
                        existing.bounds = bounds;
                    }
                    continue 'drafts;
                }
            }
            merged.push(draft);
        }
        *drafts = merged;
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_window_bounds_overlap(
        left: (f32, f32, f32, f32),
        right: (f32, f32, f32, f32),
    ) -> bool {
        left.0 <= right.2 && left.2 >= right.0 && left.1 <= right.3 && left.3 >= right.1
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_input_for_bounds(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
        bounds: (f32, f32, f32, f32),
        site_grading: Option<TerrainCdtSiteGradingContext<'_>>,
    ) -> TerrainCdtInput {
        let (min_x, min_z, max_x, max_z) = bounds;
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let patch_model = Self::terrain_cdt_patch_for_bounds(terrain, min_x, min_z, max_x, max_z);
        let mut source_samples = Vec::new();
        let mut tie_in_guide_samples = Vec::new();
        let mut tie_in_guide_constraints = Vec::new();
        let mut sample_keys = BTreeMap::new();
        let grid_step_m =
            Self::terrain_cdt_grid_sample_step_m(min_x, min_z, max_x, max_z, safe_render_step_m);
        RoadSurfaceSystem::append_terrain_cdt_roadbed_grading_envelope(
            terrain,
            road_loops,
            safe_render_step_m,
            &mut tie_in_guide_samples,
            &mut tie_in_guide_constraints,
            &mut sample_keys,
        );
        if let Some(site_grading) = site_grading {
            site_grading.append_guides(
                terrain,
                (min_x, min_z, max_x, max_z),
                safe_render_step_m,
                &mut tie_in_guide_samples,
                &mut sample_keys,
            );
        }
        Self::append_terrain_cdt_grid_samples(
            terrain,
            patch,
            min_x,
            min_z,
            max_x,
            max_z,
            grid_step_m,
            &mut source_samples,
            &mut sample_keys,
        );
        Self::append_terrain_cdt_window_boundary_samples(
            terrain,
            min_x,
            min_z,
            max_x,
            max_z,
            safe_render_step_m,
            &mut source_samples,
            &mut sample_keys,
        );
        TerrainCdtInput::new(patch_model, road_loops.to_vec(), source_samples)
            .with_tie_in_guide_samples(tie_in_guide_samples)
            .with_tie_in_guide_constraints(tie_in_guide_constraints)
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_grid_sample_step_m(
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        render_step_m: f32,
    ) -> f32 {
        let safe_step_m = render_step_m.max(f32::EPSILON);
        let width_m = (max_x - min_x).max(0.0);
        let height_m = (max_z - min_z).max(0.0);
        let sample_x = (width_m / safe_step_m).ceil() + 1.0;
        let sample_z = (height_m / safe_step_m).ceil() + 1.0;
        let estimated_samples = sample_x * sample_z;
        if estimated_samples <= TERRAIN_CDT_MAX_LOCAL_GRID_SAMPLES {
            return safe_step_m;
        }

        let scale = (estimated_samples / TERRAIN_CDT_MAX_LOCAL_GRID_SAMPLES).sqrt();
        (safe_step_m * scale).max(safe_step_m)
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_window_key(
        input: &TerrainCdtInput,
    ) -> RefinedTerrainCdtWindowKey {
        RefinedTerrainCdtWindowKey {
            min_x_mm: Self::quantize_cdt_coord_mm(input.patch.min_x),
            min_z_mm: Self::quantize_cdt_coord_mm(input.patch.min_z),
            max_x_mm: Self::quantize_cdt_coord_mm(input.patch.max_x),
            max_z_mm: Self::quantize_cdt_coord_mm(input.patch.max_z),
            fingerprint: Self::terrain_cdt_input_fingerprint(input),
        }
    }

    pub(in crate::nodes::simulation_node) fn quantize_cdt_coord_mm(value: f64) -> i64 {
        (value * 1000.0).round() as i64
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_input_fingerprint(
        input: &TerrainCdtInput,
    ) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        Self::hash_i64(&mut hash, TERRAIN_CDT_CONTRACT_REVISION);
        Self::hash_u64(&mut hash, input.road_loops.len() as u64);
        for road_loop in &input.road_loops {
            Self::hash_u64(&mut hash, road_loop.stable_piece_id);
            Self::hash_u64(&mut hash, road_loop.footprint_group_id);
            Self::hash_u64(&mut hash, u64::from(road_loop.local_loop_index));
            Self::hash_u64(&mut hash, u64::from(road_loop.is_hole));
            Self::hash_u64(&mut hash, road_loop.vertices.len() as u64);
            for vertex in &road_loop.vertices {
                Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(vertex.x));
                Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(vertex.z));
                Self::hash_i64(
                    &mut hash,
                    (f64::from(vertex.height_m) * 1000.0).round() as i64,
                );
            }
        }
        Self::hash_u64(&mut hash, input.tie_in_guide_samples.len() as u64);
        for sample in &input.tie_in_guide_samples {
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(sample.vertex.x));
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(sample.vertex.z));
            Self::hash_i64(
                &mut hash,
                (f64::from(sample.vertex.height_m) * 1000.0).round() as i64,
            );
        }
        Self::hash_u64(&mut hash, input.tie_in_guide_constraints.len() as u64);
        for constraint in &input.tie_in_guide_constraints {
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(constraint.start.x));
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(constraint.start.z));
            Self::hash_i64(
                &mut hash,
                (f64::from(constraint.start.height_m) * 1000.0).round() as i64,
            );
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(constraint.end.x));
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(constraint.end.z));
            Self::hash_i64(
                &mut hash,
                (f64::from(constraint.end.height_m) * 1000.0).round() as i64,
            );
        }
        Self::hash_u64(&mut hash, input.source_samples.len() as u64);
        for sample in &input.source_samples {
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(sample.x));
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(sample.z));
            Self::hash_i64(
                &mut hash,
                (f64::from(sample.height_m) * 1000.0).round() as i64,
            );
        }
        hash
    }

    pub(in crate::nodes::simulation_node) fn hash_i64(hash: &mut u64, value: i64) {
        Self::hash_u64(hash, value as u64);
    }

    pub(in crate::nodes::simulation_node) fn hash_u64(hash: &mut u64, value: u64) {
        *hash ^= value;
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_patch_for_bounds(
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> TerrainCdtPatch {
        TerrainCdtPatch::new(
            f64::from(min_x),
            f64::from(min_z),
            f64::from(max_x),
            f64::from(max_z),
            [
                terrain.sample_visual_height_world(min_x, min_z) * config::HEIGHT_SCALE,
                terrain.sample_visual_height_world(min_x, max_z) * config::HEIGHT_SCALE,
                terrain.sample_visual_height_world(max_x, max_z) * config::HEIGHT_SCALE,
                terrain.sample_visual_height_world(max_x, min_z) * config::HEIGHT_SCALE,
            ],
        )
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_local_sample_bounds(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
    ) -> Option<(f32, f32, f32, f32)> {
        let mut min_x = f32::INFINITY;
        let mut min_z = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_z = f32::NEG_INFINITY;
        for road_loop in road_loops {
            for vertex in &road_loop.vertices {
                let x = vertex.x as f32;
                let z = vertex.z as f32;
                min_x = min_x.min(x);
                min_z = min_z.min(z);
                max_x = max_x.max(x);
                max_z = max_z.max(z);
            }
        }
        if !min_x.is_finite() || !min_z.is_finite() || !max_x.is_finite() || !max_z.is_finite() {
            return None;
        }

        let margin_m = RoadSurfaceSystem::terrain_cdt_required_grading_margin_m(
            terrain,
            road_loops,
            render_step_m,
        );
        let patch_min_x = patch.world_origin_x;
        let patch_min_z = patch.world_origin_z;
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        min_x = (min_x - margin_m).clamp(patch_min_x, patch_max_x);
        min_z = (min_z - margin_m).clamp(patch_min_z, patch_max_z);
        max_x = (max_x + margin_m).clamp(patch_min_x, patch_max_x);
        max_z = (max_z + margin_m).clamp(patch_min_z, patch_max_z);
        if min_x > max_x || min_z > max_z {
            None
        } else {
            Some((min_x, min_z, max_x, max_z))
        }
    }

    pub(in crate::nodes::simulation_node) fn append_terrain_cdt_grid_samples(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        step_m: f32,
        source_samples: &mut Vec<TerrainCdtVertex>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let safe_step_m = step_m.max(f32::EPSILON);
        let patch_min_x = patch.world_origin_x;
        let patch_min_z = patch.world_origin_z;
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        let start_x_index = (((min_x.clamp(patch_min_x, patch_max_x) - patch_min_x) / safe_step_m)
            .floor() as i64)
            .max(0);
        let start_z_index = (((min_z.clamp(patch_min_z, patch_max_z) - patch_min_z) / safe_step_m)
            .floor() as i64)
            .max(0);
        let end_x_index = (((max_x.clamp(patch_min_x, patch_max_x) - patch_min_x) / safe_step_m)
            .ceil() as i64)
            .max(start_x_index);
        let end_z_index = (((max_z.clamp(patch_min_z, patch_max_z) - patch_min_z) / safe_step_m)
            .ceil() as i64)
            .max(start_z_index);

        for sample_z_index in start_z_index..=end_z_index {
            let world_z = (patch_min_z + sample_z_index as f32 * safe_step_m).min(patch_max_z);
            for sample_x_index in start_x_index..=end_x_index {
                let world_x = (patch_min_x + sample_x_index as f32 * safe_step_m).min(patch_max_x);
                Self::push_terrain_cdt_source_sample(
                    terrain,
                    world_x,
                    world_z,
                    source_samples,
                    sample_keys,
                );
            }
        }
    }

    pub(in crate::nodes::simulation_node) fn append_terrain_cdt_window_boundary_samples(
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        step_m: f32,
        source_samples: &mut Vec<TerrainCdtVertex>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let safe_step_m = step_m.max(f32::EPSILON);
        let xs = Self::terrain_cdt_axis_samples(min_x, max_x, safe_step_m);
        let zs = Self::terrain_cdt_axis_samples(min_z, max_z, safe_step_m);
        for &x in &xs {
            Self::push_terrain_cdt_source_sample(terrain, x, min_z, source_samples, sample_keys);
            Self::push_terrain_cdt_source_sample(terrain, x, max_z, source_samples, sample_keys);
        }
        for &z in &zs {
            Self::push_terrain_cdt_source_sample(terrain, min_x, z, source_samples, sample_keys);
            Self::push_terrain_cdt_source_sample(terrain, max_x, z, source_samples, sample_keys);
        }
    }

    pub(in crate::nodes::simulation_node) fn terrain_cdt_axis_samples(
        min: f32,
        max: f32,
        step_m: f32,
    ) -> Vec<f32> {
        let safe_step_m = step_m.max(f32::EPSILON);
        let mut samples = vec![min];
        let mut next = min + safe_step_m;
        while next < max - 0.001 {
            samples.push(next);
            next += safe_step_m;
        }
        if samples
            .last()
            .is_none_or(|last| (*last - max).abs() > 0.001)
        {
            samples.push(max);
        }
        samples
    }

    pub(in crate::nodes::simulation_node) fn push_terrain_cdt_source_sample(
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
        source_samples: &mut Vec<TerrainCdtVertex>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let key = (
            (f64::from(world_x) * TERRAIN_CDT_SAMPLE_KEY_SCALE).round() as i64,
            (f64::from(world_z) * TERRAIN_CDT_SAMPLE_KEY_SCALE).round() as i64,
        );
        if sample_keys.insert(key, ()).is_some() {
            return;
        }
        source_samples.push(TerrainCdtVertex::new(
            f64::from(world_x),
            terrain.sample_visual_height_world(world_x, world_z) * config::HEIGHT_SCALE,
            f64::from(world_z),
        ));
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_sample_height_m(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        sample_x: usize,
        sample_z: usize,
    ) -> f32 {
        if patch.texture_width == 0 || patch.height_data.is_empty() {
            return 0.0;
        }
        let texture_x = patch
            .inner_offset_x
            .saturating_add(sample_x.min(patch.sample_width.saturating_sub(1)));
        let texture_z = patch
            .inner_offset_z
            .saturating_add(sample_z.min(patch.sample_height.saturating_sub(1)));
        let index = texture_z
            .saturating_mul(patch.texture_width)
            .saturating_add(texture_x)
            .min(patch.height_data.len().saturating_sub(1));
        patch.height_data[index] * config::HEIGHT_SCALE
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_height_at_world_m(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        world_x: f32,
        world_z: f32,
    ) -> f32 {
        if patch.sample_width == 0 || patch.sample_height == 0 {
            return 0.0;
        }
        let local_x = ((world_x - patch.world_origin_x) / patch.world_size_x.max(0.001))
            .clamp(0.0, 1.0)
            * patch.sample_width.saturating_sub(1) as f32;
        let local_z = ((world_z - patch.world_origin_z) / patch.world_size_z.max(0.001))
            .clamp(0.0, 1.0)
            * patch.sample_height.saturating_sub(1) as f32;

        let x0 = local_x.floor() as usize;
        let z0 = local_z.floor() as usize;
        let x1 = (x0 + 1).min(patch.sample_width.saturating_sub(1));
        let z1 = (z0 + 1).min(patch.sample_height.saturating_sub(1));
        let tx = local_x.fract();
        let tz = local_z.fract();

        let h00 = Self::terrain_patch_sample_height_m(patch, x0, z0);
        let h10 = Self::terrain_patch_sample_height_m(patch, x1, z0);
        let h01 = Self::terrain_patch_sample_height_m(patch, x0, z1);
        let h11 = Self::terrain_patch_sample_height_m(patch, x1, z1);
        let h0 = h00 * (1.0 - tx) + h10 * tx;
        let h1 = h01 * (1.0 - tx) + h11 * tx;
        h0 * (1.0 - tz) + h1 * tz
    }
}
