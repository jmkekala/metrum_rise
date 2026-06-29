//! Source-owned roadbed grading-envelope generation for terrain CDT inputs.

use super::*;

const TERRAIN_CDT_GRADING_SAMPLE_KEY_SCALE: f64 = 1000.0;
const TERRAIN_CDT_GRADING_RING_MULTIPLIERS: [f32; 4] = [1.0, 2.0, 4.0, 8.0];
const TERRAIN_CDT_REQUIRED_GRADING_MAX_PROBES: usize = 8;

impl RoadSurfaceSystem {
    /// Returns the support distance required for road terrain-CDT grading guides.
    pub(crate) fn terrain_cdt_required_grading_margin_m(
        terrain: &TerrainSystem,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
    ) -> f32 {
        let base_margin_m = terrain_cdt_local_sample_margin_m(terrain, render_step_m);
        let mut margin_m = base_margin_m;
        for road_loop in road_loops {
            margin_m = margin_m.max(Self::terrain_cdt_required_grading_margin_for_loop_vertices(
                terrain,
                &road_loop.vertices,
                road_loop.is_hole,
                Self::road_loop_uses_clean_grounded_tie_in(road_loop),
                render_step_m,
                base_margin_m,
            ));
        }
        margin_m
    }

    /// Returns the largest support distance required by visible grounded roads.
    pub(crate) fn terrain_cdt_required_grading_margin_for_visible_roads(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        render_step_m: f32,
    ) -> f32 {
        let base_margin_m = terrain_cdt_local_sample_margin_m(terrain, render_step_m);
        let mut margin_m = base_margin_m;

        for (_, piece) in self.compiled_visual_span_pieces.iter() {
            if piece.edge_class != EdgeClass::Standard {
                continue;
            }
            for boundary_loop in &piece.terrain_clip_boundary_loops {
                margin_m = margin_m.max(Self::terrain_cdt_required_grading_margin_for_clip_loop(
                    terrain,
                    boundary_loop,
                    render_step_m,
                    base_margin_m,
                ));
            }
        }

        for (&node_id, piece) in self.compiled_visual_node_pieces.iter() {
            if !self.node_has_standard_surface_edges(graph, node_id) {
                continue;
            }
            for boundary_loop in &piece.terrain_clip_boundary_loops {
                margin_m = margin_m.max(Self::terrain_cdt_required_grading_margin_for_clip_loop(
                    terrain,
                    boundary_loop,
                    render_step_m,
                    base_margin_m,
                ));
            }
        }

        margin_m
    }

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
        let max_distance_m =
            Self::terrain_cdt_required_grading_margin_m(terrain, road_loops, safe_step_m);
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

    pub(super) fn terrain_cdt_required_grading_margin_for_clip_loop(
        terrain: &TerrainSystem,
        boundary_loop: &RoadSurfaceTerrainClipLoop,
        render_step_m: f32,
        base_margin_m: f32,
    ) -> f32 {
        Self::terrain_cdt_required_grading_margin_for_loop_vertices(
            terrain,
            &boundary_loop
                .points_world
                .iter()
                .map(|point| TerrainCdtVertex::new(point.x, point.y as f32, point.z))
                .collect::<Vec<_>>(),
            false,
            true,
            render_step_m,
            base_margin_m,
        )
    }

    fn terrain_cdt_required_grading_margin_for_loop_vertices(
        terrain: &TerrainSystem,
        vertices: &[TerrainCdtVertex],
        is_hole: bool,
        uses_clean_grounded_tie_in: bool,
        render_step_m: f32,
        base_margin_m: f32,
    ) -> f32 {
        let safe_step_m = render_step_m.max(f32::EPSILON);
        let mut margin_m = base_margin_m.max(0.0);

        if is_hole || vertices.len() < 3 || !uses_clean_grounded_tie_in {
            return Self::terrain_cdt_required_grading_margin_for_seam_vertices(
                terrain,
                vertices.iter().copied(),
                safe_step_m,
                margin_m,
            );
        }

        let signed_area = Self::terrain_cdt_vertices_signed_area_xz(vertices);
        if signed_area.abs() <= f64::EPSILON {
            return Self::terrain_cdt_required_grading_margin_for_seam_vertices(
                terrain,
                vertices.iter().copied(),
                safe_step_m,
                margin_m,
            );
        }
        let loop_is_ccw = signed_area > 0.0;
        let mut edge_outward_directions = Vec::with_capacity(vertices.len());

        for index in 0..vertices.len() {
            let start = vertices[index];
            let end = vertices[(index + 1) % vertices.len()];
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
            for sample_index in 0..=sample_count {
                let t = f64::from(sample_index) / f64::from(sample_count);
                let seam_x = start.x + dx * t;
                let seam_z = start.z + dz * t;
                let seam_height_m = start.height_m + (end.height_m - start.height_m) * t as f32;
                margin_m = margin_m.max(Self::terrain_cdt_required_grading_margin_for_ray(
                    terrain,
                    seam_x,
                    seam_z,
                    seam_height_m,
                    outward_x,
                    outward_z,
                    safe_step_m,
                    margin_m,
                ));
            }
        }

        if edge_outward_directions.len() != vertices.len() {
            return margin_m;
        }
        for index in 0..vertices.len() {
            let (previous_outward_x, previous_outward_z) =
                edge_outward_directions[(index + vertices.len() - 1) % vertices.len()];
            let (next_outward_x, next_outward_z) = edge_outward_directions[index];
            let bisector_x = previous_outward_x + next_outward_x;
            let bisector_z = previous_outward_z + next_outward_z;
            let bisector_length_m = bisector_x.hypot(bisector_z);
            if bisector_length_m <= f64::EPSILON {
                continue;
            }
            let vertex = vertices[index];
            margin_m = margin_m.max(Self::terrain_cdt_required_grading_margin_for_ray(
                terrain,
                vertex.x,
                vertex.z,
                vertex.height_m,
                bisector_x / bisector_length_m,
                bisector_z / bisector_length_m,
                safe_step_m,
                margin_m,
            ));
        }

        margin_m
    }

    fn terrain_cdt_required_grading_margin_for_seam_vertices(
        terrain: &TerrainSystem,
        vertices: impl IntoIterator<Item = TerrainCdtVertex>,
        safe_step_m: f32,
        base_margin_m: f32,
    ) -> f32 {
        let mut margin_m = base_margin_m.max(0.0);
        for vertex in vertices {
            if !vertex.x.is_finite() || !vertex.z.is_finite() || !vertex.height_m.is_finite() {
                continue;
            }
            let terrain_height_m = terrain
                .sample_visual_height_world(vertex.x as f32, vertex.z as f32)
                * crate::config::HEIGHT_SCALE;
            let height_delta_m = (vertex.height_m - terrain_height_m).abs();
            let required_distance_m =
                height_delta_m / MAX_TERRAIN_TIE_IN_SLOPE_RATIO.max(f32::EPSILON);
            margin_m = margin_m.max(required_distance_m + safe_step_m * 2.0);
        }
        margin_m
    }

    pub(super) fn terrain_cdt_required_grading_margin_for_ray(
        terrain: &TerrainSystem,
        seam_x: f64,
        seam_z: f64,
        seam_height_m: f32,
        direction_x: f64,
        direction_z: f64,
        safe_step_m: f32,
        base_margin_m: f32,
    ) -> f32 {
        if !seam_x.is_finite()
            || !seam_z.is_finite()
            || !seam_height_m.is_finite()
            || !direction_x.is_finite()
            || !direction_z.is_finite()
        {
            return base_margin_m.max(0.0);
        }
        let (world_w, world_h) = terrain.world_size();
        let max_probe_distance_m = world_w.hypot(world_h).max(base_margin_m).max(safe_step_m);
        let mut margin_m = base_margin_m.max(0.0);
        let mut probe_distance_m = safe_step_m.min(max_probe_distance_m).max(f32::EPSILON);

        for _ in 0..TERRAIN_CDT_REQUIRED_GRADING_MAX_PROBES {
            let terrain_height_m = terrain.sample_visual_height_world(
                (seam_x + direction_x * f64::from(probe_distance_m)) as f32,
                (seam_z + direction_z * f64::from(probe_distance_m)) as f32,
            ) * crate::config::HEIGHT_SCALE;
            let height_delta_m = (seam_height_m - terrain_height_m).abs();
            let required_distance_m = height_delta_m
                / MAX_TERRAIN_TIE_IN_SLOPE_RATIO.max(f32::EPSILON)
                + safe_step_m * 2.0;
            margin_m = margin_m.max(required_distance_m);
            if probe_distance_m + f32::EPSILON >= margin_m
                || probe_distance_m >= max_probe_distance_m
            {
                break;
            }
            probe_distance_m = margin_m
                .max(probe_distance_m * 2.0)
                .min(max_probe_distance_m);
        }

        margin_m.min(max_probe_distance_m)
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
            Self::push_terrain_cdt_grading_ring_vertex(
                terrain,
                seam_x,
                seam_z,
                seam_height_m,
                direction_x,
                direction_z,
                distance_m,
                &mut previous_distance_m,
                &mut vertices,
            );
        }
        Self::push_terrain_cdt_grading_ring_vertex(
            terrain,
            seam_x,
            seam_z,
            seam_height_m,
            direction_x,
            direction_z,
            max_distance_m,
            &mut previous_distance_m,
            &mut vertices,
        );
        vertices
    }

    fn push_terrain_cdt_grading_ring_vertex(
        terrain: &TerrainSystem,
        seam_x: f64,
        seam_z: f64,
        seam_height_m: f32,
        direction_x: f64,
        direction_z: f64,
        distance_m: f32,
        previous_distance_m: &mut f32,
        vertices: &mut Vec<TerrainCdtVertex>,
    ) {
        if distance_m <= *previous_distance_m + f32::EPSILON {
            return;
        }
        *previous_distance_m = distance_m;
        let world_x = seam_x + direction_x * f64::from(distance_m);
        let world_z = seam_z + direction_z * f64::from(distance_m);
        let terrain_height_m = terrain.sample_visual_height_world(world_x as f32, world_z as f32)
            * crate::config::HEIGHT_SCALE;
        let guide_height_m = Self::terrain_cdt_grade_limited_tie_in_height(
            seam_height_m,
            terrain_height_m,
            distance_m,
        );
        vertices.push(TerrainCdtVertex::new(world_x, guide_height_m, world_z));
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
        Self::terrain_cdt_vertices_signed_area_xz(&road_loop.vertices)
    }

    pub(super) fn terrain_cdt_vertices_signed_area_xz(vertices: &[TerrainCdtVertex]) -> f64 {
        let mut area = 0.0;
        for index in 0..vertices.len() {
            let start = vertices[index];
            let end = vertices[(index + 1) % vertices.len()];
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
