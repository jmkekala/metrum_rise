//! Source-owned roadbed grading-envelope generation for terrain CDT inputs.

use super::*;

const TERRAIN_CDT_GRADING_SAMPLE_KEY_SCALE: f64 = 1000.0;
const TERRAIN_CDT_GRADING_RING_MULTIPLIERS: [f32; 4] = [1.0, 2.0, 4.0, 8.0];

impl RoadSurfaceSystem {
    /// Emits deterministic terrain-CDT guide samples for the roadbed grading envelope.
    pub(crate) fn append_terrain_cdt_roadbed_grading_envelope(
        terrain: &TerrainSystem,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
        tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
        tie_in_guide_constraints: &mut Vec<TerrainCdtTieInGuideConstraint>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let safe_step_m = render_step_m.max(f32::EPSILON);
        let max_distance_m = terrain_cdt_local_sample_margin_m(terrain, safe_step_m);
        let constrain_guide_rails_for_input = road_loops.len() == 1
            && road_loops.first().is_some_and(|road_loop| {
                !road_loop.is_hole
                    && road_loop.vertices.len() >= 3
                    && Self::road_loop_uses_clean_grounded_tie_in(road_loop)
                    && Self::terrain_cdt_road_loop_signed_area_xz(road_loop).abs() > f64::EPSILON
                    && Self::terrain_cdt_road_loop_is_convex(road_loop)
            });

        for road_loop in road_loops {
            if road_loop.is_hole
                || road_loop.vertices.len() < 3
                || !Self::road_loop_uses_clean_grounded_tie_in(road_loop)
            {
                continue;
            }
            let signed_area = Self::terrain_cdt_road_loop_signed_area_xz(road_loop);
            if signed_area.abs() <= f64::EPSILON {
                continue;
            }
            let loop_is_ccw = signed_area > 0.0;
            let mut edge_outward_directions = Vec::with_capacity(road_loop.vertices.len());

            for index in 0..road_loop.vertices.len() {
                let start = road_loop.vertices[index];
                let end = road_loop.vertices[(index + 1) % road_loop.vertices.len()];
                let dx = end.x - start.x;
                let dz = end.z - start.z;
                let length_m = dx.hypot(dz);
                if length_m <= f64::EPSILON {
                    continue;
                }
                let outward_x = if loop_is_ccw { dz } else { -dz } / length_m;
                let outward_z = if loop_is_ccw { -dx } else { dx } / length_m;
                edge_outward_directions.push((outward_x, outward_z));
                let sample_count = ((length_m as f32 / safe_step_m).ceil() as u32).max(1);
                let mut previous_ring_vertices = Vec::new();
                for sample_index in 0..=sample_count {
                    let t = f64::from(sample_index) / f64::from(sample_count);
                    let seam_x = start.x + dx * t;
                    let seam_z = start.z + dz * t;
                    let seam_height_m = start.height_m + (end.height_m - start.height_m) * t as f32;
                    let ring_vertices = Self::terrain_cdt_grading_ring_vertices(
                        terrain,
                        seam_x,
                        seam_z,
                        seam_height_m,
                        outward_x,
                        outward_z,
                        safe_step_m,
                        max_distance_m,
                    );
                    for vertex in &ring_vertices {
                        Self::push_terrain_cdt_grading_guide_sample(
                            *vertex,
                            tie_in_guide_samples,
                            sample_keys,
                        );
                    }
                    if constrain_guide_rails_for_input {
                        for (previous, current) in
                            previous_ring_vertices.iter().zip(ring_vertices.iter())
                        {
                            Self::push_terrain_cdt_grading_guide_constraint(
                                *previous,
                                *current,
                                tie_in_guide_constraints,
                            );
                        }
                    }
                    previous_ring_vertices = ring_vertices;
                }
            }

            if edge_outward_directions.len() != road_loop.vertices.len() {
                continue;
            }
            for index in 0..road_loop.vertices.len() {
                let (previous_outward_x, previous_outward_z) = edge_outward_directions
                    [(index + road_loop.vertices.len() - 1) % road_loop.vertices.len()];
                let (next_outward_x, next_outward_z) = edge_outward_directions[index];
                let bisector_x = previous_outward_x + next_outward_x;
                let bisector_z = previous_outward_z + next_outward_z;
                let bisector_length_m = bisector_x.hypot(bisector_z);
                if bisector_length_m <= f64::EPSILON {
                    continue;
                }
                let vertex = road_loop.vertices[index];
                let ring_vertices = Self::terrain_cdt_grading_ring_vertices(
                    terrain,
                    vertex.x,
                    vertex.z,
                    vertex.height_m,
                    bisector_x / bisector_length_m,
                    bisector_z / bisector_length_m,
                    safe_step_m,
                    max_distance_m,
                );
                for guide_vertex in &ring_vertices {
                    Self::push_terrain_cdt_grading_guide_sample(
                        *guide_vertex,
                        tie_in_guide_samples,
                        sample_keys,
                    );
                }
            }
        }
    }

    fn terrain_cdt_grading_ring_vertices(
        terrain: &TerrainSystem,
        seam_x: f64,
        seam_z: f64,
        seam_height_m: f32,
        direction_x: f64,
        direction_z: f64,
        safe_step_m: f32,
        max_distance_m: f32,
    ) -> Vec<TerrainCdtVertex> {
        let mut vertices = Vec::new();
        let mut previous_distance_m = 0.0_f32;
        for multiplier in TERRAIN_CDT_GRADING_RING_MULTIPLIERS {
            let distance_m = (safe_step_m * multiplier).min(max_distance_m);
            if distance_m <= previous_distance_m + f32::EPSILON {
                continue;
            }
            previous_distance_m = distance_m;
            let world_x = seam_x + direction_x * f64::from(distance_m);
            let world_z = seam_z + direction_z * f64::from(distance_m);
            let terrain_height_m = terrain
                .sample_visual_height_world(world_x as f32, world_z as f32)
                * crate::config::HEIGHT_SCALE;
            let guide_height_m = Self::terrain_cdt_grade_limited_tie_in_height(
                seam_height_m,
                terrain_height_m,
                distance_m,
            );
            vertices.push(TerrainCdtVertex::new(world_x, guide_height_m, world_z));
        }
        vertices
    }

    fn terrain_cdt_road_loop_is_convex(road_loop: &TerrainCdtRoadLoop) -> bool {
        if road_loop.vertices.len() < 3 {
            return false;
        }
        let signed_area = Self::terrain_cdt_road_loop_signed_area_xz(road_loop);
        if signed_area.abs() <= f64::EPSILON {
            return false;
        }
        let expected_sign = signed_area.signum();
        for index in 0..road_loop.vertices.len() {
            let previous = road_loop.vertices
                [(index + road_loop.vertices.len() - 1) % road_loop.vertices.len()];
            let current = road_loop.vertices[index];
            let next = road_loop.vertices[(index + 1) % road_loop.vertices.len()];
            let ax = current.x - previous.x;
            let az = current.z - previous.z;
            let bx = next.x - current.x;
            let bz = next.z - current.z;
            let cross = ax * bz - az * bx;
            if cross.abs() <= f64::EPSILON {
                continue;
            }
            if cross.signum() != expected_sign {
                return false;
            }
        }
        true
    }

    fn road_loop_uses_clean_grounded_tie_in(road_loop: &TerrainCdtRoadLoop) -> bool {
        road_loop.source_edges.is_empty()
            || road_loop
                .source_edges
                .iter()
                .all(|edge| Self::boundary_source_uses_clean_grounded_tie_in(edge.source))
    }

    fn boundary_source_uses_clean_grounded_tie_in(source: TerrainCdtRoadBoundarySource) -> bool {
        match source {
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_class,
                support_policy,
                ..
            } => {
                edge_class == TerrainCdtEdgeClass::Standard
                    && support_policy == TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan
            }
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary { .. }
            | TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff { .. }
            | TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. } => true,
            TerrainCdtRoadBoundarySource::BuildingSiteBoundary { .. } => false,
        }
    }

    fn terrain_cdt_road_loop_signed_area_xz(road_loop: &TerrainCdtRoadLoop) -> f64 {
        let mut area = 0.0;
        for index in 0..road_loop.vertices.len() {
            let start = road_loop.vertices[index];
            let end = road_loop.vertices[(index + 1) % road_loop.vertices.len()];
            area += start.x * end.z - end.x * start.z;
        }
        area * 0.5
    }

    fn terrain_cdt_grade_limited_tie_in_height(
        seam_height_m: f32,
        terrain_height_m: f32,
        distance_m: f32,
    ) -> f32 {
        let max_delta_m = distance_m.max(0.0) * MAX_TERRAIN_TIE_IN_SLOPE_RATIO;
        let delta_m = terrain_height_m - seam_height_m;
        if delta_m.abs() <= max_delta_m {
            terrain_height_m
        } else {
            seam_height_m + delta_m.signum() * max_delta_m
        }
    }

    fn push_terrain_cdt_grading_guide_sample(
        vertex: TerrainCdtVertex,
        tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        if !vertex.x.is_finite() || !vertex.z.is_finite() || !vertex.height_m.is_finite() {
            return;
        }
        let key = Self::terrain_cdt_grading_sample_key(vertex.x, vertex.z);
        if sample_keys.insert(key, ()).is_some() {
            return;
        }
        tie_in_guide_samples.push(TerrainCdtTieInGuideSample { vertex });
    }

    fn push_terrain_cdt_grading_guide_constraint(
        start: TerrainCdtVertex,
        end: TerrainCdtVertex,
        tie_in_guide_constraints: &mut Vec<TerrainCdtTieInGuideConstraint>,
    ) {
        if !start.x.is_finite()
            || !start.z.is_finite()
            || !start.height_m.is_finite()
            || !end.x.is_finite()
            || !end.z.is_finite()
            || !end.height_m.is_finite()
        {
            return;
        }
        if Self::terrain_cdt_grading_sample_key(start.x, start.z)
            == Self::terrain_cdt_grading_sample_key(end.x, end.z)
        {
            return;
        }
        tie_in_guide_constraints.push(TerrainCdtTieInGuideConstraint { start, end });
    }

    fn terrain_cdt_grading_sample_key(x: f64, z: f64) -> (i64, i64) {
        (
            (x * TERRAIN_CDT_GRADING_SAMPLE_KEY_SCALE).round() as i64,
            (z * TERRAIN_CDT_GRADING_SAMPLE_KEY_SCALE).round() as i64,
        )
    }
}
