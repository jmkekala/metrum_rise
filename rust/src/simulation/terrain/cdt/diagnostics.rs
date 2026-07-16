//! Mesh diagnostics and source-provenance summaries.

use std::collections::{BTreeMap, HashSet};

use super::*;

pub(super) struct TerrainCdtDiagnostics {
    #[cfg(test)]
    pub(super) emitted_faces: Vec<TerrainCdtEmittedFace>,
    pub(super) terrain_triangles: Vec<[usize; 3]>,
    pub(super) terrain_triangle_sources: Vec<Vec<TerrainCdtRoadBoundarySource>>,
    pub(super) retaining_wall_triangles: Vec<[usize; 3]>,
    pub(super) retaining_wall_triangle_sources: Vec<Vec<TerrainCdtRoadBoundarySource>>,
    pub(super) max_face_y_delta_m: f32,
    pub(super) max_face_slope_ratio: f32,
    pub(super) longest_triangle_edge_m: f32,
    pub(super) road_seam_faces: usize,
    pub(super) road_seam_max_y_delta_m: f32,
    pub(super) road_seam_max_slope_ratio: f32,
    pub(super) retaining_wall_faces: usize,
    pub(super) retaining_wall_max_y_delta_m: f32,
    pub(super) retaining_wall_max_slope_ratio: f32,
    pub(super) road_seam_face_samples: Vec<TerrainCdtFaceSample>,
    pub(super) retaining_wall_face_samples: Vec<TerrainCdtFaceSample>,
}

pub(super) fn emitted_triangle_edges(triangles: &[[usize; 3]]) -> HashSet<(usize, usize)> {
    let mut edges = HashSet::new();
    for [a, b, c] in triangles {
        edges.insert(normalize_edge(*a, *b));
        edges.insert(normalize_edge(*b, *c));
        edges.insert(normalize_edge(*c, *a));
    }
    edges
}

pub(super) fn terrain_face_diagnostics(
    vertices: &[TerrainCdtVertex],
    triangles: &[[usize; 3]],
    road_constraint_sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
    retaining_wall_required_sources: &[TerrainCdtRoadBoundarySource],
) -> TerrainCdtDiagnostics {
    let mut diagnostics = TerrainCdtDiagnostics {
        #[cfg(test)]
        emitted_faces: Vec::new(),
        terrain_triangles: Vec::new(),
        terrain_triangle_sources: Vec::new(),
        retaining_wall_triangles: Vec::new(),
        retaining_wall_triangle_sources: Vec::new(),
        max_face_y_delta_m: 0.0,
        max_face_slope_ratio: 0.0,
        longest_triangle_edge_m: 0.0,
        road_seam_faces: 0,
        road_seam_max_y_delta_m: 0.0,
        road_seam_max_slope_ratio: 0.0,
        retaining_wall_faces: 0,
        retaining_wall_max_y_delta_m: 0.0,
        retaining_wall_max_slope_ratio: 0.0,
        road_seam_face_samples: Vec::new(),
        retaining_wall_face_samples: Vec::new(),
    };

    for triangle in triangles {
        let points = [
            vertices[triangle[0]],
            vertices[triangle[1]],
            vertices[triangle[2]],
        ];
        let sources = terrain_triangle_road_sources(triangle, road_constraint_sources);
        let touches_road_seam = !sources.is_empty();
        let kind = classify_terrain_tie_in_face(points, &sources, retaining_wall_required_sources);
        let metrics = terrain_face_sample(points, kind, sources);
        #[cfg(test)]
        diagnostics.emitted_faces.push(TerrainCdtEmittedFace {
            triangle: *triangle,
            kind,
            sources: metrics.sources.clone(),
        });
        diagnostics.max_face_y_delta_m = diagnostics.max_face_y_delta_m.max(metrics.max_y_delta_m);
        diagnostics.max_face_slope_ratio = diagnostics
            .max_face_slope_ratio
            .max(metrics.max_slope_ratio);
        diagnostics.longest_triangle_edge_m = diagnostics.longest_triangle_edge_m.max(
            edge_length_xz_m(points[0], points[1])
                .max(edge_length_xz_m(points[1], points[2]))
                .max(edge_length_xz_m(points[2], points[0])) as f32,
        );

        match kind {
            TerrainCdtTieInKind::OrdinaryTerrain => {
                diagnostics.terrain_triangles.push(*triangle);
                diagnostics
                    .terrain_triangle_sources
                    .push(metrics.sources.clone());
            }
            TerrainCdtTieInKind::RetainingWall => {
                diagnostics.retaining_wall_triangles.push(*triangle);
                diagnostics
                    .retaining_wall_triangle_sources
                    .push(metrics.sources.clone());
                diagnostics.retaining_wall_faces += 1;
                diagnostics.retaining_wall_max_y_delta_m = diagnostics
                    .retaining_wall_max_y_delta_m
                    .max(metrics.max_y_delta_m);
                diagnostics.retaining_wall_max_slope_ratio = diagnostics
                    .retaining_wall_max_slope_ratio
                    .max(metrics.max_slope_ratio);
                insert_road_seam_face_sample(
                    &mut diagnostics.retaining_wall_face_samples,
                    metrics.clone(),
                );
            }
        }

        if !touches_road_seam {
            continue;
        }

        diagnostics.road_seam_faces += 1;
        diagnostics.road_seam_max_y_delta_m = diagnostics
            .road_seam_max_y_delta_m
            .max(metrics.max_y_delta_m);
        diagnostics.road_seam_max_slope_ratio = diagnostics
            .road_seam_max_slope_ratio
            .max(metrics.max_slope_ratio);
        insert_road_seam_face_sample(&mut diagnostics.road_seam_face_samples, metrics);
    }

    diagnostics
}

fn terrain_triangle_road_sources(
    triangle: &[usize; 3],
    road_constraint_sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) -> Vec<TerrainCdtRoadBoundarySource> {
    let mut sources = triangle_edges(triangle)
        .iter()
        .filter_map(|edge| {
            road_constraint_sources
                .get(&[edge.0, edge.1])
                .map(|source| source.boundary_source)
        })
        .collect::<Vec<_>>();
    sort_dedup_terrain_cdt_boundary_sources(&mut sources);
    sources
}

fn classify_terrain_tie_in_face(
    points: [TerrainCdtVertex; 3],
    sources: &[TerrainCdtRoadBoundarySource],
    retaining_wall_required_sources: &[TerrainCdtRoadBoundarySource],
) -> TerrainCdtTieInKind {
    if sources.is_empty() {
        return TerrainCdtTieInKind::OrdinaryTerrain;
    }
    if !terrain_cdt_sources_allow_retaining_wall(sources) {
        return TerrainCdtTieInKind::OrdinaryTerrain;
    }
    if terrain_sources_include_retaining_wall_required_source(
        sources,
        retaining_wall_required_sources,
    ) {
        return TerrainCdtTieInKind::RetainingWall;
    }
    let metrics = terrain_face_sample(points, TerrainCdtTieInKind::OrdinaryTerrain, Vec::new());
    if metrics.max_slope_ratio > MAX_TERRAIN_TIE_IN_SLOPE_RATIO {
        TerrainCdtTieInKind::RetainingWall
    } else {
        TerrainCdtTieInKind::OrdinaryTerrain
    }
}

fn terrain_cdt_sources_allow_retaining_wall(sources: &[TerrainCdtRoadBoundarySource]) -> bool {
    sources
        .iter()
        .copied()
        .any(terrain_cdt_boundary_source_allows_retaining_wall)
}

pub(super) fn terrain_cdt_boundary_source_allows_retaining_wall(
    source: TerrainCdtRoadBoundarySource,
) -> bool {
    match source {
        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
            edge_class,
            support_policy,
            ..
        } => {
            edge_class != TerrainCdtEdgeClass::Standard
                || support_policy != TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan
        }
        TerrainCdtRoadBoundarySource::NodeFootprintBoundary { .. }
        | TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff { .. } => false,
        TerrainCdtRoadBoundarySource::BuildingSiteBoundary { .. } => false,
        TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. } => true,
    }
}

fn terrain_sources_include_retaining_wall_required_source(
    sources: &[TerrainCdtRoadBoundarySource],
    retaining_wall_required_sources: &[TerrainCdtRoadBoundarySource],
) -> bool {
    sources.iter().copied().any(|source| {
        retaining_wall_required_sources
            .binary_search_by(|required_source| {
                terrain_cdt_boundary_source_cmp(*required_source, source)
            })
            .is_ok()
    })
}

pub(super) fn widening_tie_in_sample_against_any_road_loop(
    sample: TerrainCdtVertex,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> Option<TerrainCdtTieInSample> {
    road_loops
        .iter()
        .filter_map(|road_loop| widening_tie_in_sample(sample, road_loop))
        .max_by(|a, b| {
            a.slope_ratio
                .total_cmp(&b.slope_ratio)
                .then_with(|| a.height_delta_m.total_cmp(&b.height_delta_m))
                .then_with(|| b.distance_m.total_cmp(&a.distance_m))
        })
}

pub(super) fn widening_tie_in_sample(
    sample: TerrainCdtVertex,
    road_loop: &CanonicalTerrainCdtRoadLoop,
) -> Option<TerrainCdtTieInSample> {
    let (distance_m, seam_point, seam_source) =
        closest_sourced_loop_edge_distance_point_and_source(sample, road_loop)?;
    let height_delta_m = (sample.height_m - seam_point.height_m).abs();
    if height_delta_m <= MIN_TIE_IN_HEIGHT_DELTA_M {
        return None;
    }
    if distance_m <= CDT_EPSILON_M {
        return Some(TerrainCdtTieInSample {
            source_sample: sample,
            seam_point,
            seam_source,
            distance_m: 0.0,
            required_distance_m: height_delta_m / MAX_TERRAIN_TIE_IN_SLOPE_RATIO,
            height_delta_m,
            slope_ratio: f32::INFINITY,
        });
    }
    let distance_m_f32 = distance_m as f32;
    let slope_ratio = height_delta_m / distance_m_f32;
    let required_distance_m = height_delta_m / MAX_TERRAIN_TIE_IN_SLOPE_RATIO;
    (distance_m < f64::from(required_distance_m) - CDT_EPSILON_M).then_some(TerrainCdtTieInSample {
        source_sample: sample,
        seam_point,
        seam_source,
        distance_m: distance_m_f32,
        required_distance_m,
        height_delta_m,
        slope_ratio,
    })
}

fn closest_sourced_loop_edge_distance_point_and_source(
    point: TerrainCdtVertex,
    road_loop: &CanonicalTerrainCdtRoadLoop,
) -> Option<(f64, TerrainCdtVertex, TerrainCdtRoadBoundarySource)> {
    if road_loop.vertices.len() < 2 {
        return None;
    }

    let mut closest_distance_m = f64::INFINITY;
    let mut closest_point = TerrainCdtVertex::new(0.0, 0.0, 0.0);
    let mut closest_source = None;
    for index in 0..road_loop.vertices.len() {
        let Some(source) = road_loop.edge_sources.get(index).copied().flatten() else {
            continue;
        };
        let start = road_loop.vertices[index];
        let end = road_loop.vertices[(index + 1) % road_loop.vertices.len()];
        let segment_x = end.x - start.x;
        let segment_z = end.z - start.z;
        let segment_len_sq = segment_x * segment_x + segment_z * segment_z;
        let t = if segment_len_sq <= CDT_EPSILON_M * CDT_EPSILON_M {
            0.0
        } else {
            (((point.x - start.x) * segment_x + (point.z - start.z) * segment_z) / segment_len_sq)
                .clamp(0.0, 1.0)
        };
        let closest_x = start.x + segment_x * t;
        let closest_z = start.z + segment_z * t;
        let dx = point.x - closest_x;
        let dz = point.z - closest_z;
        let distance_m = (dx * dx + dz * dz).sqrt();
        let height_m =
            (f64::from(start.height_m) + f64::from(end.height_m - start.height_m) * t) as f32;
        let candidate_point = TerrainCdtVertex::new(closest_x, height_m, closest_z);
        if terrain_cdt_closer_loop_point(
            distance_m,
            candidate_point,
            source,
            closest_distance_m,
            closest_point,
            closest_source,
        ) {
            closest_distance_m = distance_m;
            closest_point = candidate_point;
            closest_source = Some(source);
        }
    }

    closest_distance_m
        .is_finite()
        .then_some((closest_distance_m, closest_point, closest_source?))
}

fn terrain_cdt_closer_loop_point(
    candidate_distance_m: f64,
    candidate_point: TerrainCdtVertex,
    candidate_source: TerrainCdtRoadBoundarySource,
    current_distance_m: f64,
    current_point: TerrainCdtVertex,
    current_source: Option<TerrainCdtRoadBoundarySource>,
) -> bool {
    if candidate_distance_m < current_distance_m - CDT_EPSILON_M {
        return true;
    }
    if (candidate_distance_m - current_distance_m).abs() > CDT_EPSILON_M {
        return false;
    }
    let geometry_ordering = terrain_cdt_vertex_geometry_cmp(candidate_point, current_point);
    if !geometry_ordering.is_eq() {
        return geometry_ordering.is_lt();
    }
    current_source
        .is_none_or(|source| terrain_cdt_boundary_source_cmp(candidate_source, source).is_lt())
}

fn terrain_cdt_vertex_geometry_cmp(a: TerrainCdtVertex, b: TerrainCdtVertex) -> std::cmp::Ordering {
    terrain_cdt_vertex_key(a).cmp(&terrain_cdt_vertex_key(b))
}

pub(super) fn triangle_edges(triangle: &[usize; 3]) -> [(usize, usize); 3] {
    [
        normalize_edge(triangle[0], triangle[1]),
        normalize_edge(triangle[1], triangle[2]),
        normalize_edge(triangle[2], triangle[0]),
    ]
}

fn terrain_face_sample(
    points: [TerrainCdtVertex; 3],
    kind: TerrainCdtTieInKind,
    sources: Vec<TerrainCdtRoadBoundarySource>,
) -> TerrainCdtFaceSample {
    let mut min_x = points[0].x;
    let mut min_z = points[0].z;
    let mut max_x = points[0].x;
    let mut max_z = points[0].z;
    let mut min_y_m = points[0].height_m;
    let mut max_y_m = points[0].height_m;
    let mut max_y_delta_m = 0.0_f32;
    let max_slope_ratio = terrain_face_plane_slope_ratio(points);

    for point in points {
        min_x = min_x.min(point.x);
        min_z = min_z.min(point.z);
        max_x = max_x.max(point.x);
        max_z = max_z.max(point.z);
        min_y_m = min_y_m.min(point.height_m);
        max_y_m = max_y_m.max(point.height_m);
    }

    for edge_index in 0..3 {
        let start = points[edge_index];
        let end = points[(edge_index + 1) % 3];
        let y_delta_m = (end.height_m - start.height_m).abs();
        max_y_delta_m = max_y_delta_m.max(y_delta_m);
    }

    TerrainCdtFaceSample {
        kind,
        vertices: points,
        centroid: centroid(points),
        sources,
        min_x,
        min_z,
        max_x,
        max_z,
        min_y_m,
        max_y_m,
        max_y_delta_m,
        max_slope_ratio,
    }
}

fn terrain_face_plane_slope_ratio(points: [TerrainCdtVertex; 3]) -> f32 {
    let ax = points[1].x - points[0].x;
    let ay = f64::from(points[1].height_m - points[0].height_m);
    let az = points[1].z - points[0].z;
    let bx = points[2].x - points[0].x;
    let by = f64::from(points[2].height_m - points[0].height_m);
    let bz = points[2].z - points[0].z;

    let normal_x = ay * bz - az * by;
    let normal_y = az * bx - ax * bz;
    let normal_z = ax * by - ay * bx;
    let horizontal_normal = (normal_x * normal_x + normal_z * normal_z).sqrt();
    if horizontal_normal <= CDT_EPSILON_M * CDT_EPSILON_M {
        return 0.0;
    }
    if normal_y.abs() <= CDT_EPSILON_M * CDT_EPSILON_M {
        return 1_000_000.0;
    }
    (horizontal_normal / normal_y.abs()) as f32
}

fn insert_road_seam_face_sample(
    samples: &mut Vec<TerrainCdtFaceSample>,
    sample: TerrainCdtFaceSample,
) {
    samples.push(sample);
    samples.sort_by(|a, b| {
        b.max_slope_ratio
            .total_cmp(&a.max_slope_ratio)
            .then_with(|| b.max_y_delta_m.total_cmp(&a.max_y_delta_m))
            .then_with(|| terrain_cdt_boundary_sources_cmp(&a.sources, &b.sources))
            .then_with(|| a.centroid.x.total_cmp(&b.centroid.x))
            .then_with(|| a.centroid.z.total_cmp(&b.centroid.z))
    });
    samples.truncate(MAX_ROAD_SEAM_FACE_SAMPLES);
}

pub(super) fn insert_tie_in_widened_sample(
    samples: &mut Vec<TerrainCdtTieInSample>,
    sample: TerrainCdtTieInSample,
) {
    samples.push(sample);
    samples.sort_by(|a, b| {
        b.slope_ratio
            .total_cmp(&a.slope_ratio)
            .then_with(|| b.height_delta_m.total_cmp(&a.height_delta_m))
            .then_with(|| terrain_cdt_boundary_source_cmp(a.seam_source, b.seam_source))
            .then_with(|| a.source_sample.x.total_cmp(&b.source_sample.x))
            .then_with(|| a.source_sample.z.total_cmp(&b.source_sample.z))
    });
    samples.truncate(MAX_TIE_IN_SAMPLE_DIAGNOSTICS);
}

pub(super) fn insert_invalid_constraint_sample(
    samples: &mut Vec<TerrainCdtInvalidConstraintSample>,
    edge: [usize; 2],
    vertices: &[TerrainCdtVertex],
    road_constraint_sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) {
    let Some(&start) = vertices.get(edge[0]) else {
        return;
    };
    let Some(&end) = vertices.get(edge[1]) else {
        return;
    };
    let source = road_constraint_sources.get(&edge);
    samples.push(TerrainCdtInvalidConstraintSample {
        start,
        end,
        road_owned: source.is_some(),
        stable_piece_id: source.map_or(0, |source| source.stable_piece_id),
        local_loop_index: source.map_or(u32::MAX, |source| source.local_loop_index),
        local_edge_index: source.map_or(u32::MAX, |source| source.local_edge_index),
        source: source.map(|source| source.boundary_source),
    });
    samples.sort_by(|a, b| {
        b.road_owned
            .cmp(&a.road_owned)
            .then_with(|| a.stable_piece_id.cmp(&b.stable_piece_id))
            .then_with(|| a.local_loop_index.cmp(&b.local_loop_index))
            .then_with(|| a.local_edge_index.cmp(&b.local_edge_index))
            .then_with(|| terrain_cdt_optional_boundary_source_cmp(a.source, b.source))
            .then_with(|| a.start.x.total_cmp(&b.start.x))
            .then_with(|| a.start.z.total_cmp(&b.start.z))
            .then_with(|| a.end.x.total_cmp(&b.end.x))
            .then_with(|| a.end.z.total_cmp(&b.end.z))
    });
    samples.truncate(MAX_INVALID_CONSTRAINT_SAMPLES);
}

pub(super) fn unpreserved_road_constraint_samples(
    road_constraint_edges: &[[usize; 2]],
    accepted_edges: &HashSet<(usize, usize)>,
    vertices: &[TerrainCdtVertex],
    road_constraint_sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) -> Vec<TerrainCdtInvalidConstraintSample> {
    let mut samples = Vec::new();
    for edge in road_constraint_edges {
        if accepted_edges.contains(&normalize_edge(edge[0], edge[1])) {
            continue;
        }
        insert_invalid_constraint_sample(
            &mut samples,
            normalize_edge_array(edge[0], edge[1]),
            vertices,
            road_constraint_sources,
        );
    }
    samples
}

pub(super) fn sort_dedup_terrain_cdt_boundary_sources(
    sources: &mut Vec<TerrainCdtRoadBoundarySource>,
) {
    sources.sort_by(|a, b| terrain_cdt_boundary_source_cmp(*a, *b));
    sources.dedup_by(|a, b| terrain_cdt_boundary_source_cmp(*a, *b).is_eq());
}
