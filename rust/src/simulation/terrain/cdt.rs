//! Deterministic constrained triangulation for road-touched terrain patches.
//!
//! This module owns the Rust-side CDT kernel used by terrain patches. It deliberately
//! does not depend on Godot types: callers adapt road-piece loops and terrain samples
//! into this small data model, then convert the returned indexed mesh to renderer
//! buffers at the boundary.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet, HashSet};

use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

const CDT_EPSILON_M: f64 = 0.001;
const MAX_INVALID_CONSTRAINT_SAMPLES: usize = 8;
const MAX_ROAD_SEAM_FACE_SAMPLES: usize = 8;
const MAX_TERRAIN_TIE_IN_SLOPE_RATIO: f32 = 0.5;
const MIN_TIE_IN_HEIGHT_DELTA_M: f32 = 0.01;

type SpadeCdt = ConstrainedDelaunayTriangulation<Point2<f64>>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtVertex {
    pub(crate) x: f64,
    pub(crate) height_m: f32,
    pub(crate) z: f64,
}

impl TerrainCdtVertex {
    pub(crate) fn new(x: f64, height_m: f32, z: f64) -> Self {
        Self { x, height_m, z }
    }

    fn point2(self) -> Point2<f64> {
        Point2::new(self.x, self.z)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtPatch {
    pub(crate) min_x: f64,
    pub(crate) min_z: f64,
    pub(crate) max_x: f64,
    pub(crate) max_z: f64,
    pub(crate) corner_heights_m: [f32; 4],
}

impl TerrainCdtPatch {
    pub(crate) fn new(
        min_x: f64,
        min_z: f64,
        max_x: f64,
        max_z: f64,
        corner_heights_m: [f32; 4],
    ) -> Self {
        Self {
            min_x,
            min_z,
            max_x,
            max_z,
            corner_heights_m,
        }
    }

    fn is_valid(self) -> bool {
        self.max_x > self.min_x + CDT_EPSILON_M && self.max_z > self.min_z + CDT_EPSILON_M
    }

    fn corners_cw(self) -> [TerrainCdtVertex; 4] {
        [
            TerrainCdtVertex::new(self.min_x, self.corner_heights_m[0], self.min_z),
            TerrainCdtVertex::new(self.min_x, self.corner_heights_m[1], self.max_z),
            TerrainCdtVertex::new(self.max_x, self.corner_heights_m[2], self.max_z),
            TerrainCdtVertex::new(self.max_x, self.corner_heights_m[3], self.min_z),
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainCdtRoadLoop {
    pub(crate) stable_piece_id: u64,
    pub(crate) local_loop_index: u32,
    pub(crate) vertices: Vec<TerrainCdtVertex>,
}

impl TerrainCdtRoadLoop {
    pub(crate) fn new(
        stable_piece_id: u64,
        local_loop_index: u32,
        vertices: Vec<TerrainCdtVertex>,
    ) -> Self {
        Self {
            stable_piece_id,
            local_loop_index,
            vertices,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainCdtInput {
    pub(crate) patch: TerrainCdtPatch,
    pub(crate) road_loops: Vec<TerrainCdtRoadLoop>,
    pub(crate) source_samples: Vec<TerrainCdtVertex>,
}

impl TerrainCdtInput {
    pub(crate) fn new(
        patch: TerrainCdtPatch,
        road_loops: Vec<TerrainCdtRoadLoop>,
        source_samples: Vec<TerrainCdtVertex>,
    ) -> Self {
        Self {
            patch,
            road_loops,
            source_samples,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainCdtMesh {
    pub(crate) vertices: Vec<TerrainCdtVertex>,
    pub(crate) triangles: Vec<[usize; 3]>,
    pub(crate) stats: TerrainCdtStats,
    pub(crate) invalid_constraint_samples: Vec<TerrainCdtInvalidConstraintSample>,
    pub(crate) road_seam_face_samples: Vec<TerrainCdtFaceSample>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtStats {
    pub(crate) input_vertices: usize,
    pub(crate) constraint_edges: usize,
    pub(crate) road_constraint_edges: usize,
    pub(crate) accepted_faces: usize,
    pub(crate) rejected_road_faces: usize,
    pub(crate) preserved_road_constraint_edges: usize,
    pub(crate) invalid_constraint_edges: usize,
    pub(crate) max_face_y_delta_m: f32,
    pub(crate) max_face_slope_ratio: f32,
    pub(crate) road_seam_faces: usize,
    pub(crate) road_seam_steep_faces: usize,
    pub(crate) road_seam_max_y_delta_m: f32,
    pub(crate) road_seam_max_slope_ratio: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtFaceSample {
    pub(crate) vertices: [TerrainCdtVertex; 3],
    pub(crate) centroid: TerrainCdtVertex,
    pub(crate) min_x: f64,
    pub(crate) min_z: f64,
    pub(crate) max_x: f64,
    pub(crate) max_z: f64,
    pub(crate) min_y_m: f32,
    pub(crate) max_y_m: f32,
    pub(crate) max_y_delta_m: f32,
    pub(crate) max_slope_ratio: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtInvalidConstraintSample {
    pub(crate) start: TerrainCdtVertex,
    pub(crate) end: TerrainCdtVertex,
    pub(crate) road_owned: bool,
    pub(crate) stable_piece_id: u64,
    pub(crate) local_loop_index: u32,
    pub(crate) local_edge_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerrainCdtError {
    InvalidPatch,
    TriangulationFailed,
}

pub(crate) fn build_road_touched_terrain_patch(
    input: TerrainCdtInput,
) -> Result<TerrainCdtMesh, TerrainCdtError> {
    if !input.patch.is_valid() {
        return Err(TerrainCdtError::InvalidPatch);
    }

    let canonical = canonicalize_input(input)?;
    let spade_vertices = canonical
        .vertices
        .iter()
        .map(|vertex| vertex.point2())
        .collect::<Vec<_>>();
    let mut invalid_constraint_edges = 0usize;
    let mut invalid_constraint_samples = Vec::new();
    let cdt = SpadeCdt::try_bulk_load_cdt(spade_vertices, canonical.constraints.clone(), |edge| {
        invalid_constraint_edges += 1;
        insert_invalid_constraint_sample(
            &mut invalid_constraint_samples,
            normalize_edge_array(edge[0], edge[1]),
            &canonical.vertices,
            &canonical.road_constraint_sources,
        );
    })
    .map_err(|_| TerrainCdtError::TriangulationFailed)?;

    let mut triangles = Vec::new();
    let mut rejected_road_faces = 0usize;
    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let triangle = [a.fix().index(), b.fix().index(), c.fix().index()];
        let center = centroid([
            canonical.vertices[triangle[0]],
            canonical.vertices[triangle[1]],
            canonical.vertices[triangle[2]],
        ]);
        if canonical
            .road_loops
            .iter()
            .any(|road_loop| point_in_polygon(center, road_loop))
        {
            rejected_road_faces += 1;
            continue;
        }
        triangles.push(triangle);
    }

    let accepted_edges = emitted_triangle_edges(&triangles);
    let preserved_road_constraint_edges = canonical
        .road_constraint_edges
        .iter()
        .filter(|edge| accepted_edges.contains(&normalize_edge(edge[0], edge[1])))
        .count();
    let diagnostics = terrain_face_diagnostics(
        &canonical.vertices,
        &triangles,
        &canonical.road_constraint_edges,
    );

    Ok(TerrainCdtMesh {
        stats: TerrainCdtStats {
            input_vertices: canonical.vertices.len(),
            constraint_edges: canonical.constraints.len(),
            road_constraint_edges: canonical.road_constraint_edges.len(),
            accepted_faces: triangles.len(),
            rejected_road_faces,
            preserved_road_constraint_edges,
            invalid_constraint_edges,
            max_face_y_delta_m: diagnostics.max_face_y_delta_m,
            max_face_slope_ratio: diagnostics.max_face_slope_ratio,
            road_seam_faces: diagnostics.road_seam_faces,
            road_seam_steep_faces: diagnostics.road_seam_steep_faces,
            road_seam_max_y_delta_m: diagnostics.road_seam_max_y_delta_m,
            road_seam_max_slope_ratio: diagnostics.road_seam_max_slope_ratio,
        },
        vertices: canonical.vertices,
        triangles,
        invalid_constraint_samples,
        road_seam_face_samples: diagnostics.road_seam_face_samples,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TerrainCdtRoadConstraintSource {
    stable_piece_id: u64,
    local_loop_index: u32,
    local_edge_index: u32,
}

struct TerrainCdtDiagnostics {
    max_face_y_delta_m: f32,
    max_face_slope_ratio: f32,
    road_seam_faces: usize,
    road_seam_steep_faces: usize,
    road_seam_max_y_delta_m: f32,
    road_seam_max_slope_ratio: f32,
    road_seam_face_samples: Vec<TerrainCdtFaceSample>,
}

struct CanonicalTerrainCdtInput {
    vertices: Vec<TerrainCdtVertex>,
    constraints: Vec<[usize; 2]>,
    road_constraint_edges: Vec<[usize; 2]>,
    road_constraint_sources: BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
    road_loops: Vec<Vec<TerrainCdtVertex>>,
}

fn canonicalize_input(
    mut input: TerrainCdtInput,
) -> Result<CanonicalTerrainCdtInput, TerrainCdtError> {
    let mut vertices = Vec::new();
    let mut vertex_lookup = BTreeMap::new();
    let mut constraint_set = BTreeSet::new();
    let mut road_constraint_edges = Vec::new();
    let mut road_constraint_sources = BTreeMap::new();
    let mut road_loops = Vec::new();

    let patch_corners = input.patch.corners_cw();
    for &vertex in &patch_corners {
        insert_vertex(vertex, &mut vertices, &mut vertex_lookup);
    }

    input
        .road_loops
        .sort_by_key(|road_loop| (road_loop.stable_piece_id, road_loop.local_loop_index));
    for road_loop in input.road_loops {
        let original_points = simplified_loop(road_loop.vertices);
        if original_points.len() < 3
            || signed_area(&original_points).abs() <= CDT_EPSILON_M * CDT_EPSILON_M
        {
            continue;
        }
        let points = simplified_loop(clip_loop_to_patch(original_points, input.patch));
        if points.len() < 3 {
            continue;
        }
        if signed_area(&points).abs() <= CDT_EPSILON_M * CDT_EPSILON_M {
            continue;
        }
        let points = ensure_ccw(points);
        let loop_indices = points
            .iter()
            .map(|&vertex| insert_vertex(vertex, &mut vertices, &mut vertex_lookup))
            .collect::<Vec<_>>();
        push_road_loop_constraints(
            &loop_indices,
            &vertices,
            input.patch,
            road_loop.stable_piece_id,
            road_loop.local_loop_index,
            &mut road_constraint_edges,
            &mut road_constraint_sources,
        );
        road_loops.push(points);
    }

    input.source_samples.sort_by_key(|sample| {
        (
            quantized_coord(sample.x),
            quantized_coord(sample.z),
            quantized_coord(f64::from(sample.height_m)),
        )
    });
    for sample in input.source_samples {
        if !patch_contains(sample, input.patch) {
            continue;
        }
        if road_loops
            .iter()
            .any(|road_loop| point_in_polygon(sample, road_loop))
        {
            continue;
        }
        if road_loops
            .iter()
            .any(|road_loop| source_sample_would_make_oversteep_tie_in(sample, road_loop))
        {
            continue;
        }
        insert_vertex(sample, &mut vertices, &mut vertex_lookup);
    }

    node_road_constraint_edges(
        &mut vertices,
        &mut vertex_lookup,
        input.patch,
        &mut road_constraint_edges,
        &mut road_constraint_sources,
    );
    push_patch_boundary_constraints(input.patch, &vertices, &mut constraint_set);
    for edge in &road_constraint_edges {
        insert_constraint(*edge, &mut constraint_set);
    }

    Ok(CanonicalTerrainCdtInput {
        vertices,
        constraints: constraint_set.into_iter().collect(),
        road_constraint_edges,
        road_constraint_sources,
        road_loops,
    })
}

fn insert_vertex(
    vertex: TerrainCdtVertex,
    vertices: &mut Vec<TerrainCdtVertex>,
    vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
) -> usize {
    let key = (quantized_coord(vertex.x), quantized_coord(vertex.z));
    if let Some(index) = vertex_lookup.get(&key) {
        return *index;
    }
    let index = vertices.len();
    vertices.push(vertex);
    vertex_lookup.insert(key, index);
    index
}

fn push_road_loop_constraints(
    indices: &[usize],
    vertices: &[TerrainCdtVertex],
    patch: TerrainCdtPatch,
    stable_piece_id: u64,
    local_loop_index: u32,
    road_constraint_edges: &mut Vec<[usize; 2]>,
    road_constraint_sources: &mut BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) {
    for index in 0..indices.len() {
        let edge = normalize_edge_array(indices[index], indices[(index + 1) % indices.len()]);
        if edge[0] == edge[1] {
            continue;
        }
        if !edge_lies_on_patch_boundary(vertices[edge[0]], vertices[edge[1]], patch) {
            road_constraint_edges.push(edge);
            road_constraint_sources
                .entry(edge)
                .or_insert(TerrainCdtRoadConstraintSource {
                    stable_piece_id,
                    local_loop_index,
                    local_edge_index: u32::try_from(index).unwrap_or(u32::MAX),
                });
        }
    }
}

fn push_patch_boundary_constraints(
    patch: TerrainCdtPatch,
    vertices: &[TerrainCdtVertex],
    constraint_set: &mut BTreeSet<[usize; 2]>,
) {
    let mut left = Vec::new();
    let mut top = Vec::new();
    let mut right = Vec::new();
    let mut bottom = Vec::new();

    for (index, vertex) in vertices.iter().copied().enumerate() {
        if same_coord(vertex.x, patch.min_x)
            && vertex.z >= patch.min_z - CDT_EPSILON_M
            && vertex.z <= patch.max_z + CDT_EPSILON_M
        {
            left.push((quantized_coord(vertex.z), index));
        }
        if same_coord(vertex.z, patch.max_z)
            && vertex.x >= patch.min_x - CDT_EPSILON_M
            && vertex.x <= patch.max_x + CDT_EPSILON_M
        {
            top.push((quantized_coord(vertex.x), index));
        }
        if same_coord(vertex.x, patch.max_x)
            && vertex.z >= patch.min_z - CDT_EPSILON_M
            && vertex.z <= patch.max_z + CDT_EPSILON_M
        {
            right.push((-quantized_coord(vertex.z), index));
        }
        if same_coord(vertex.z, patch.min_z)
            && vertex.x >= patch.min_x - CDT_EPSILON_M
            && vertex.x <= patch.max_x + CDT_EPSILON_M
        {
            bottom.push((-quantized_coord(vertex.x), index));
        }
    }

    push_sorted_boundary_side(&mut left, constraint_set);
    push_sorted_boundary_side(&mut top, constraint_set);
    push_sorted_boundary_side(&mut right, constraint_set);
    push_sorted_boundary_side(&mut bottom, constraint_set);
}

fn push_sorted_boundary_side(
    side: &mut Vec<(i64, usize)>,
    constraint_set: &mut BTreeSet<[usize; 2]>,
) {
    side.sort_unstable();
    side.dedup_by_key(|(_, index)| *index);
    for pair in side.windows(2) {
        insert_constraint([pair[0].1, pair[1].1], constraint_set);
    }
}

fn insert_constraint(edge: [usize; 2], constraint_set: &mut BTreeSet<[usize; 2]>) {
    let edge = normalize_edge_array(edge[0], edge[1]);
    if edge[0] != edge[1] {
        constraint_set.insert(edge);
    }
}

// Spade accepts a constrained graph but does not node crossing or T-touching
// constraints for us. i_overlay owns roadbed area union; this patch-local pass
// only canonicalizes the final CDT constraint graph. Determinism comes from
// sorted original road loops, quantized XZ vertex lookup, and BTreeSet edge
// emission. Complexity is O(E^2) with bbox rejection over one dirty terrain
// patch's roadbed constraints, outside the per-tick simulation hot path.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TerrainCdtRoadConstraintSplit {
    t: f64,
    vertex_index: usize,
}

fn node_road_constraint_edges(
    vertices: &mut Vec<TerrainCdtVertex>,
    vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
    patch: TerrainCdtPatch,
    road_constraint_edges: &mut Vec<[usize; 2]>,
    road_constraint_sources: &mut BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) {
    if road_constraint_edges.len() < 2 {
        return;
    }

    let original_edges = road_constraint_edges.clone();
    let mut split_points = original_edges
        .iter()
        .map(|edge| {
            vec![
                TerrainCdtRoadConstraintSplit {
                    t: 0.0,
                    vertex_index: edge[0],
                },
                TerrainCdtRoadConstraintSplit {
                    t: 1.0,
                    vertex_index: edge[1],
                },
            ]
        })
        .collect::<Vec<_>>();

    for first_index in 0..original_edges.len() {
        for second_index in first_index + 1..original_edges.len() {
            let first_edge = original_edges[first_index];
            let second_edge = original_edges[second_index];
            if first_edge == second_edge {
                continue;
            }
            let first_start = vertices[first_edge[0]];
            let first_end = vertices[first_edge[1]];
            let second_start = vertices[second_edge[0]];
            let second_end = vertices[second_edge[1]];
            if !segment_bounds_overlap(first_start, first_end, second_start, second_end) {
                continue;
            }

            for intersection in
                segment_intersections(first_start, first_end, second_start, second_end)
            {
                let first_t =
                    segment_parameter(first_start, first_end, intersection.x, intersection.z);
                let second_t =
                    segment_parameter(second_start, second_end, intersection.x, intersection.z);
                if !unit_interval_contains(first_t) || !unit_interval_contains(second_t) {
                    continue;
                }
                let first_height =
                    interpolated_segment_height(first_start, first_end, clamp_unit(first_t));
                let second_height =
                    interpolated_segment_height(second_start, second_end, clamp_unit(second_t));
                let vertex_index = insert_road_constraint_vertex(
                    TerrainCdtVertex::new(
                        intersection.x,
                        first_height.max(second_height),
                        intersection.z,
                    ),
                    vertices,
                    vertex_lookup,
                );
                split_points[first_index].push(TerrainCdtRoadConstraintSplit {
                    t: clamp_unit(first_t),
                    vertex_index,
                });
                split_points[second_index].push(TerrainCdtRoadConstraintSplit {
                    t: clamp_unit(second_t),
                    vertex_index,
                });
            }
        }
    }

    let original_sources = road_constraint_sources.clone();
    let mut noded_edges = BTreeSet::new();
    road_constraint_sources.clear();

    for (edge, splits) in original_edges.iter().copied().zip(split_points.iter_mut()) {
        sort_dedup_constraint_splits(splits);
        let source = original_sources.get(&edge).copied();
        for pair in splits.windows(2) {
            let noded_edge = normalize_edge_array(pair[0].vertex_index, pair[1].vertex_index);
            if noded_edge[0] == noded_edge[1]
                || edge_lies_on_patch_boundary(
                    vertices[noded_edge[0]],
                    vertices[noded_edge[1]],
                    patch,
                )
            {
                continue;
            }
            noded_edges.insert(noded_edge);
            if let Some(source) = source {
                road_constraint_sources
                    .entry(noded_edge)
                    .and_modify(|existing| *existing = (*existing).min(source))
                    .or_insert(source);
            }
        }
    }

    *road_constraint_edges = noded_edges.into_iter().collect();
}

fn insert_road_constraint_vertex(
    vertex: TerrainCdtVertex,
    vertices: &mut Vec<TerrainCdtVertex>,
    vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
) -> usize {
    let key = (quantized_coord(vertex.x), quantized_coord(vertex.z));
    if let Some(index) = vertex_lookup.get(&key) {
        vertices[*index].height_m = vertices[*index].height_m.max(vertex.height_m);
        return *index;
    }
    let index = vertices.len();
    vertices.push(vertex);
    vertex_lookup.insert(key, index);
    index
}

fn sort_dedup_constraint_splits(splits: &mut Vec<TerrainCdtRoadConstraintSplit>) {
    splits.sort_by(|a, b| {
        a.t.total_cmp(&b.t)
            .then_with(|| a.vertex_index.cmp(&b.vertex_index))
    });
    let mut deduped = Vec::with_capacity(splits.len());
    for split in splits.iter().copied() {
        if let Some(last) = deduped.last_mut() {
            let last: &mut TerrainCdtRoadConstraintSplit = last;
            if (split.t - last.t).abs() <= CDT_EPSILON_M || split.vertex_index == last.vertex_index
            {
                if split.vertex_index < last.vertex_index {
                    *last = split;
                }
                continue;
            }
        }
        deduped.push(split);
    }
    *splits = deduped;
}

fn segment_intersections(
    first_start: TerrainCdtVertex,
    first_end: TerrainCdtVertex,
    second_start: TerrainCdtVertex,
    second_end: TerrainCdtVertex,
) -> Vec<TerrainCdtVertex> {
    let first_dx = first_end.x - first_start.x;
    let first_dz = first_end.z - first_start.z;
    let second_dx = second_end.x - second_start.x;
    let second_dz = second_end.z - second_start.z;
    let first_len_sq = first_dx * first_dx + first_dz * first_dz;
    let second_len_sq = second_dx * second_dx + second_dz * second_dz;
    if first_len_sq <= CDT_EPSILON_M * CDT_EPSILON_M
        || second_len_sq <= CDT_EPSILON_M * CDT_EPSILON_M
    {
        return Vec::new();
    }

    let cross = cross_xz(first_dx, first_dz, second_dx, second_dz);
    let start_delta_x = second_start.x - first_start.x;
    let start_delta_z = second_start.z - first_start.z;
    if cross.abs() > CDT_EPSILON_M * first_len_sq.sqrt().max(second_len_sq.sqrt()) {
        let first_t = cross_xz(start_delta_x, start_delta_z, second_dx, second_dz) / cross;
        let second_t = cross_xz(start_delta_x, start_delta_z, first_dx, first_dz) / cross;
        if unit_interval_contains(first_t) && unit_interval_contains(second_t) {
            return vec![TerrainCdtVertex::new(
                first_start.x + first_dx * clamp_unit(first_t),
                0.0,
                first_start.z + first_dz * clamp_unit(first_t),
            )];
        }
        return Vec::new();
    }

    if cross_xz(start_delta_x, start_delta_z, first_dx, first_dz).abs()
        > CDT_EPSILON_M * first_len_sq.sqrt()
    {
        return Vec::new();
    }

    let first_t0 = segment_parameter(first_start, first_end, second_start.x, second_start.z);
    let first_t1 = segment_parameter(first_start, first_end, second_end.x, second_end.z);
    let overlap_start = first_t0.min(first_t1).max(0.0);
    let overlap_end = first_t0.max(first_t1).min(1.0);
    if overlap_start > overlap_end + CDT_EPSILON_M {
        return Vec::new();
    }

    let mut intersections = vec![TerrainCdtVertex::new(
        first_start.x + first_dx * clamp_unit(overlap_start),
        0.0,
        first_start.z + first_dz * clamp_unit(overlap_start),
    )];
    if (overlap_end - overlap_start).abs() > CDT_EPSILON_M {
        intersections.push(TerrainCdtVertex::new(
            first_start.x + first_dx * clamp_unit(overlap_end),
            0.0,
            first_start.z + first_dz * clamp_unit(overlap_end),
        ));
    }
    intersections
}

fn segment_bounds_overlap(
    first_start: TerrainCdtVertex,
    first_end: TerrainCdtVertex,
    second_start: TerrainCdtVertex,
    second_end: TerrainCdtVertex,
) -> bool {
    first_start.x.min(first_end.x) <= second_start.x.max(second_end.x) + CDT_EPSILON_M
        && second_start.x.min(second_end.x) <= first_start.x.max(first_end.x) + CDT_EPSILON_M
        && first_start.z.min(first_end.z) <= second_start.z.max(second_end.z) + CDT_EPSILON_M
        && second_start.z.min(second_end.z) <= first_start.z.max(first_end.z) + CDT_EPSILON_M
}

fn segment_parameter(start: TerrainCdtVertex, end: TerrainCdtVertex, x: f64, z: f64) -> f64 {
    let dx = end.x - start.x;
    let dz = end.z - start.z;
    let length_squared = dx * dx + dz * dz;
    if length_squared <= CDT_EPSILON_M * CDT_EPSILON_M {
        return 0.0;
    }
    ((x - start.x) * dx + (z - start.z) * dz) / length_squared
}

fn interpolated_segment_height(start: TerrainCdtVertex, end: TerrainCdtVertex, t: f64) -> f32 {
    (f64::from(start.height_m) + f64::from(end.height_m - start.height_m) * t) as f32
}

fn unit_interval_contains(value: f64) -> bool {
    value >= -CDT_EPSILON_M && value <= 1.0 + CDT_EPSILON_M
}

fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn cross_xz(ax: f64, az: f64, bx: f64, bz: f64) -> f64 {
    ax * bz - az * bx
}

fn edge_lies_on_patch_boundary(
    a: TerrainCdtVertex,
    b: TerrainCdtVertex,
    patch: TerrainCdtPatch,
) -> bool {
    (same_coord(a.x, patch.min_x) && same_coord(b.x, patch.min_x))
        || (same_coord(a.x, patch.max_x) && same_coord(b.x, patch.max_x))
        || (same_coord(a.z, patch.min_z) && same_coord(b.z, patch.min_z))
        || (same_coord(a.z, patch.max_z) && same_coord(b.z, patch.max_z))
}

fn simplified_loop(points: Vec<TerrainCdtVertex>) -> Vec<TerrainCdtVertex> {
    let mut deduplicated = Vec::with_capacity(points.len());
    for point in points {
        if deduplicated
            .last()
            .is_some_and(|last: &TerrainCdtVertex| same_xz(*last, point))
        {
            continue;
        }
        deduplicated.push(point);
    }
    if deduplicated.len() > 1 && same_xz(deduplicated[0], *deduplicated.last().unwrap()) {
        deduplicated.pop();
    }
    deduplicated
}

fn clip_loop_to_patch(
    points: Vec<TerrainCdtVertex>,
    patch: TerrainCdtPatch,
) -> Vec<TerrainCdtVertex> {
    let points = clip_loop_to_boundary(
        points,
        |point| point.x >= patch.min_x - CDT_EPSILON_M,
        |a, b| intersect_at_x(a, b, patch.min_x),
    );
    let points = clip_loop_to_boundary(
        points,
        |point| point.x <= patch.max_x + CDT_EPSILON_M,
        |a, b| intersect_at_x(a, b, patch.max_x),
    );
    let points = clip_loop_to_boundary(
        points,
        |point| point.z >= patch.min_z - CDT_EPSILON_M,
        |a, b| intersect_at_z(a, b, patch.min_z),
    );
    let points = clip_loop_to_boundary(
        points,
        |point| point.z <= patch.max_z + CDT_EPSILON_M,
        |a, b| intersect_at_z(a, b, patch.max_z),
    );
    points
        .into_iter()
        .map(|point| clamp_to_patch(point, patch))
        .collect()
}

fn clip_loop_to_boundary(
    points: Vec<TerrainCdtVertex>,
    inside: impl Fn(TerrainCdtVertex) -> bool,
    intersection: impl Fn(TerrainCdtVertex, TerrainCdtVertex) -> TerrainCdtVertex,
) -> Vec<TerrainCdtVertex> {
    if points.is_empty() {
        return points;
    }

    let mut clipped = Vec::new();
    let mut previous = *points.last().unwrap();
    let mut previous_inside = inside(previous);
    for current in points {
        let current_inside = inside(current);
        if current_inside {
            if !previous_inside {
                clipped.push(intersection(previous, current));
            }
            clipped.push(current);
        } else if previous_inside {
            clipped.push(intersection(previous, current));
        }
        previous = current;
        previous_inside = current_inside;
    }
    clipped
}

fn intersect_at_x(a: TerrainCdtVertex, b: TerrainCdtVertex, x: f64) -> TerrainCdtVertex {
    let denominator = b.x - a.x;
    if denominator.abs() <= CDT_EPSILON_M {
        return TerrainCdtVertex::new(x, a.height_m, a.z);
    }
    interpolate_vertex(a, b, (x - a.x) / denominator)
}

fn intersect_at_z(a: TerrainCdtVertex, b: TerrainCdtVertex, z: f64) -> TerrainCdtVertex {
    let denominator = b.z - a.z;
    if denominator.abs() <= CDT_EPSILON_M {
        return TerrainCdtVertex::new(a.x, a.height_m, z);
    }
    interpolate_vertex(a, b, (z - a.z) / denominator)
}

fn interpolate_vertex(a: TerrainCdtVertex, b: TerrainCdtVertex, t: f64) -> TerrainCdtVertex {
    let t = t.clamp(0.0, 1.0);
    TerrainCdtVertex::new(
        a.x + (b.x - a.x) * t,
        (f64::from(a.height_m) + f64::from(b.height_m - a.height_m) * t) as f32,
        a.z + (b.z - a.z) * t,
    )
}

fn clamp_to_patch(vertex: TerrainCdtVertex, patch: TerrainCdtPatch) -> TerrainCdtVertex {
    TerrainCdtVertex::new(
        vertex.x.clamp(patch.min_x, patch.max_x),
        vertex.height_m,
        vertex.z.clamp(patch.min_z, patch.max_z),
    )
}

fn patch_contains(vertex: TerrainCdtVertex, patch: TerrainCdtPatch) -> bool {
    vertex.x >= patch.min_x - CDT_EPSILON_M
        && vertex.x <= patch.max_x + CDT_EPSILON_M
        && vertex.z >= patch.min_z - CDT_EPSILON_M
        && vertex.z <= patch.max_z + CDT_EPSILON_M
}

fn ensure_ccw(mut points: Vec<TerrainCdtVertex>) -> Vec<TerrainCdtVertex> {
    if signed_area(&points) < 0.0 {
        points.reverse();
    }
    points
}

fn same_xz(a: TerrainCdtVertex, b: TerrainCdtVertex) -> bool {
    quantized_coord(a.x) == quantized_coord(b.x) && quantized_coord(a.z) == quantized_coord(b.z)
}

fn same_coord(a: f64, b: f64) -> bool {
    quantized_coord(a) == quantized_coord(b)
}

fn quantized_coord(value: f64) -> i64 {
    (value / CDT_EPSILON_M).round() as i64
}

fn signed_area(points: &[TerrainCdtVertex]) -> f64 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        area += points[index].x * points[next].z - points[next].x * points[index].z;
    }
    area * 0.5
}

fn point_in_polygon(point: TerrainCdtVertex, polygon: &[TerrainCdtVertex]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let a = polygon[current];
        let b = polygon[previous];
        if (a.z > point.z) != (b.z > point.z) {
            let intersection_x = (b.x - a.x) * (point.z - a.z) / (b.z - a.z) + a.x;
            if point.x < intersection_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

fn centroid(points: [TerrainCdtVertex; 3]) -> TerrainCdtVertex {
    TerrainCdtVertex::new(
        (points[0].x + points[1].x + points[2].x) / 3.0,
        (points[0].height_m + points[1].height_m + points[2].height_m) / 3.0,
        (points[0].z + points[1].z + points[2].z) / 3.0,
    )
}

fn emitted_triangle_edges(triangles: &[[usize; 3]]) -> HashSet<(usize, usize)> {
    let mut edges = HashSet::new();
    for [a, b, c] in triangles {
        edges.insert(normalize_edge(*a, *b));
        edges.insert(normalize_edge(*b, *c));
        edges.insert(normalize_edge(*c, *a));
    }
    edges
}

fn terrain_face_diagnostics(
    vertices: &[TerrainCdtVertex],
    triangles: &[[usize; 3]],
    road_constraint_edges: &[[usize; 2]],
) -> TerrainCdtDiagnostics {
    let road_edge_set = road_constraint_edges
        .iter()
        .map(|edge| normalize_edge(edge[0], edge[1]))
        .collect::<HashSet<_>>();
    let mut diagnostics = TerrainCdtDiagnostics {
        max_face_y_delta_m: 0.0,
        max_face_slope_ratio: 0.0,
        road_seam_faces: 0,
        road_seam_steep_faces: 0,
        road_seam_max_y_delta_m: 0.0,
        road_seam_max_slope_ratio: 0.0,
        road_seam_face_samples: Vec::new(),
    };

    for triangle in triangles {
        let points = [
            vertices[triangle[0]],
            vertices[triangle[1]],
            vertices[triangle[2]],
        ];
        let metrics = terrain_face_sample(points);
        diagnostics.max_face_y_delta_m = diagnostics.max_face_y_delta_m.max(metrics.max_y_delta_m);
        diagnostics.max_face_slope_ratio = diagnostics
            .max_face_slope_ratio
            .max(metrics.max_slope_ratio);

        let touches_road_seam = triangle_edges(triangle)
            .iter()
            .any(|edge| road_edge_set.contains(edge));
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
        if metrics.max_slope_ratio > MAX_TERRAIN_TIE_IN_SLOPE_RATIO {
            diagnostics.road_seam_steep_faces += 1;
        }
        insert_road_seam_face_sample(&mut diagnostics.road_seam_face_samples, metrics);
    }

    diagnostics
}

fn source_sample_would_make_oversteep_tie_in(
    sample: TerrainCdtVertex,
    road_loop: &[TerrainCdtVertex],
) -> bool {
    let Some((distance_m, seam_height_m)) =
        closest_loop_edge_distance_and_height(sample, road_loop)
    else {
        return false;
    };
    let height_delta_m = (sample.height_m - seam_height_m).abs();
    if height_delta_m <= MIN_TIE_IN_HEIGHT_DELTA_M {
        return false;
    }
    let min_tie_in_distance_m = f64::from(height_delta_m / MAX_TERRAIN_TIE_IN_SLOPE_RATIO);
    distance_m < min_tie_in_distance_m - CDT_EPSILON_M
}

fn closest_loop_edge_distance_and_height(
    point: TerrainCdtVertex,
    road_loop: &[TerrainCdtVertex],
) -> Option<(f64, f32)> {
    if road_loop.len() < 2 {
        return None;
    }

    let mut closest_distance_m = f64::INFINITY;
    let mut closest_height_m = 0.0_f32;
    for index in 0..road_loop.len() {
        let start = road_loop[index];
        let end = road_loop[(index + 1) % road_loop.len()];
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
        if distance_m < closest_distance_m {
            closest_distance_m = distance_m;
            closest_height_m =
                (f64::from(start.height_m) + f64::from(end.height_m - start.height_m) * t) as f32;
        }
    }

    closest_distance_m
        .is_finite()
        .then_some((closest_distance_m, closest_height_m))
}

fn triangle_edges(triangle: &[usize; 3]) -> [(usize, usize); 3] {
    [
        normalize_edge(triangle[0], triangle[1]),
        normalize_edge(triangle[1], triangle[2]),
        normalize_edge(triangle[2], triangle[0]),
    ]
}

fn terrain_face_sample(points: [TerrainCdtVertex; 3]) -> TerrainCdtFaceSample {
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
        vertices: points,
        centroid: centroid(points),
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
            .then_with(|| a.centroid.x.total_cmp(&b.centroid.x))
            .then_with(|| a.centroid.z.total_cmp(&b.centroid.z))
    });
    samples.truncate(MAX_ROAD_SEAM_FACE_SAMPLES);
}

fn insert_invalid_constraint_sample(
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
    });
    samples.sort_by(|a, b| {
        b.road_owned
            .cmp(&a.road_owned)
            .then_with(|| a.stable_piece_id.cmp(&b.stable_piece_id))
            .then_with(|| a.local_loop_index.cmp(&b.local_loop_index))
            .then_with(|| a.local_edge_index.cmp(&b.local_edge_index))
            .then_with(|| a.start.x.total_cmp(&b.start.x))
            .then_with(|| a.start.z.total_cmp(&b.start.z))
            .then_with(|| a.end.x.total_cmp(&b.end.x))
            .then_with(|| a.end.z.total_cmp(&b.end.z))
    });
    samples.truncate(MAX_INVALID_CONSTRAINT_SAMPLES);
}

fn normalize_edge(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

fn normalize_edge_array(a: usize, b: usize) -> [usize; 2] {
    if a < b { [a, b] } else { [b, a] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spade_cdt_preserves_road_constraints_and_omits_road_faces() {
        let road = diagonal_road_loop();
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]),
            vec![TerrainCdtRoadLoop::new(7, 0, road.clone())],
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 5.0),
                TerrainCdtVertex::new(6.0, 0.0, 30.0),
                TerrainCdtVertex::new(18.0, 0.0, 34.0),
                TerrainCdtVertex::new(20.0, 0.0, 6.0),
                TerrainCdtVertex::new(34.0, 0.0, 10.0),
                TerrainCdtVertex::new(35.0, 0.0, 35.0),
            ],
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("Spade should triangulate a road-touched terrain patch");

        assert!(!mesh.triangles.is_empty());
        assert_eq!(mesh.stats.road_constraint_edges, 4);
        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges,
            mesh.stats.road_constraint_edges
        );
        assert!(mesh.stats.rejected_road_faces > 0);
        for triangle in &mesh.triangles {
            let center = centroid([
                mesh.vertices[triangle[0]],
                mesh.vertices[triangle[1]],
                mesh.vertices[triangle[2]],
            ]);
            assert!(
                !point_in_polygon(center, &road),
                "accepted terrain triangle leaked into the road footprint"
            );
        }
    }

    #[test]
    fn spade_cdt_face_set_is_deterministic_for_canonical_input() {
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]),
            vec![TerrainCdtRoadLoop::new(7, 0, diagonal_road_loop())],
            vec![
                TerrainCdtVertex::new(35.0, 0.0, 35.0),
                TerrainCdtVertex::new(5.0, 0.0, 5.0),
                TerrainCdtVertex::new(34.0, 0.0, 10.0),
                TerrainCdtVertex::new(20.0, 0.0, 6.0),
                TerrainCdtVertex::new(18.0, 0.0, 34.0),
                TerrainCdtVertex::new(6.0, 0.0, 30.0),
            ],
        );

        let first = build_road_touched_terrain_patch(input.clone()).unwrap();
        let second = build_road_touched_terrain_patch(input).unwrap();

        assert_eq!(
            canonical_triangle_set(&first.triangles),
            canonical_triangle_set(&second.triangles)
        );
        assert_eq!(first.stats, second.stats);
    }

    #[test]
    fn cdt_skips_source_samples_that_would_make_oversteep_road_tie_ins() {
        let road = vec![
            TerrainCdtVertex::new(3.0, 0.12, 3.0),
            TerrainCdtVertex::new(7.0, 0.12, 3.0),
            TerrainCdtVertex::new(7.0, 0.12, 7.0),
            TerrainCdtVertex::new(3.0, 0.12, 7.0),
        ];
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![TerrainCdtRoadLoop::new(3, 0, road)],
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 2.99),
                TerrainCdtVertex::new(2.99, 0.0, 5.0),
                TerrainCdtVertex::new(7.01, 0.0, 5.0),
                TerrainCdtVertex::new(5.0, 0.0, 7.01),
            ],
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("Spade should triangulate a raised road seam");

        assert_eq!(
            mesh.stats.input_vertices, 8,
            "near-road source samples should be omitted from the tie-in input"
        );
        assert!(mesh.stats.road_seam_faces > 0);
        assert_eq!(mesh.stats.road_seam_steep_faces, 0);
        assert!(mesh.stats.road_seam_max_y_delta_m >= 0.12);
        assert!(
            mesh.stats.road_seam_max_slope_ratio <= MAX_TERRAIN_TIE_IN_SLOPE_RATIO + 0.0001,
            "terrain tie-in should not exceed the configured slope budget; stats={:?}",
            mesh.stats
        );
        assert!(!mesh.road_seam_face_samples.is_empty());
        assert!(
            mesh.road_seam_face_samples[0].max_slope_ratio
                >= mesh.stats.road_seam_max_slope_ratio - 0.0001
        );
    }

    #[test]
    fn road_loop_crossing_one_patch_edge_is_clipped_to_shared_boundary_vertices() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road = road_loop_from_centerline(
            TerrainCdtVertex::new(-10.0, 0.0, 20.0),
            TerrainCdtVertex::new(20.0, 0.0, 20.0),
            6.0,
        );

        let mesh = build_crossing_patch(patch, road.clone());
        assert_valid_clipped_mesh(&mesh, patch, &road);
        assert!(
            mesh.vertices
                .iter()
                .any(|vertex| same_coord(vertex.x, patch.min_x))
        );
    }

    #[test]
    fn road_loop_crossing_two_patch_edges_splits_both_patch_boundary_constraints() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road = road_loop_from_centerline(
            TerrainCdtVertex::new(-10.0, 0.0, 20.0),
            TerrainCdtVertex::new(50.0, 0.0, 20.0),
            6.0,
        );

        let mesh = build_crossing_patch(patch, road.clone());
        assert_valid_clipped_mesh(&mesh, patch, &road);
        assert!(
            mesh.vertices
                .iter()
                .any(|vertex| same_coord(vertex.x, patch.min_x))
        );
        assert!(
            mesh.vertices
                .iter()
                .any(|vertex| same_coord(vertex.x, patch.max_x))
        );
    }

    #[test]
    fn road_loop_crossing_patch_corner_uses_corner_as_constraint_endpoint() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road = road_loop_from_centerline(
            TerrainCdtVertex::new(-10.0, 0.0, -10.0),
            TerrainCdtVertex::new(20.0, 0.0, 20.0),
            6.0,
        );

        let mesh = build_crossing_patch(patch, road.clone());
        assert_valid_clipped_mesh(&mesh, patch, &road);
        assert!(
            mesh.vertices.iter().any(
                |vertex| same_coord(vertex.x, patch.min_x) && same_coord(vertex.z, patch.min_z)
            )
        );
    }

    #[test]
    fn multiple_road_loops_in_one_patch_preserve_all_seam_constraints_deterministically() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road_a = road_loop_from_centerline(
            TerrainCdtVertex::new(8.0, 0.0, 10.0),
            TerrainCdtVertex::new(18.0, 0.0, 18.0),
            4.0,
        );
        let road_b = road_loop_from_centerline(
            TerrainCdtVertex::new(22.0, 0.0, 28.0),
            TerrainCdtVertex::new(34.0, 0.0, 28.0),
            4.0,
        );
        let input = TerrainCdtInput::new(
            patch,
            vec![
                TerrainCdtRoadLoop::new(99, 0, road_b.clone()),
                TerrainCdtRoadLoop::new(7, 0, road_a.clone()),
            ],
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 5.0),
                TerrainCdtVertex::new(5.0, 0.0, 35.0),
                TerrainCdtVertex::new(20.0, 0.0, 5.0),
                TerrainCdtVertex::new(20.0, 0.0, 35.0),
                TerrainCdtVertex::new(35.0, 0.0, 5.0),
                TerrainCdtVertex::new(35.0, 0.0, 35.0),
            ],
        );

        let first = build_road_touched_terrain_patch(input.clone())
            .expect("Spade should triangulate multiple road loops");
        let second = build_road_touched_terrain_patch(input)
            .expect("Spade should deterministically triangulate multiple road loops");

        assert_eq!(first.stats.road_constraint_edges, 8);
        assert_eq!(first.stats.invalid_constraint_edges, 0);
        assert_eq!(
            first.stats.preserved_road_constraint_edges,
            first.stats.road_constraint_edges
        );
        assert_eq!(
            canonical_triangle_set(&first.triangles),
            canonical_triangle_set(&second.triangles)
        );
        for triangle in &first.triangles {
            let center = centroid([
                first.vertices[triangle[0]],
                first.vertices[triangle[1]],
                first.vertices[triangle[2]],
            ]);
            assert!(!point_in_polygon(center, &road_a));
            assert!(!point_in_polygon(center, &road_b));
        }
    }

    #[test]
    fn bend_footprint_loop_preserves_piece_owned_constraints() {
        let patch = piece_test_patch();
        let road = vec![
            test_vertex(10.0, 10.0),
            test_vertex(26.0, 10.0),
            test_vertex(26.0, 20.0),
            test_vertex(42.0, 20.0),
            test_vertex(42.0, 34.0),
            test_vertex(10.0, 34.0),
        ];

        let mesh = build_piece_patch(patch, 11, road.clone());

        assert_valid_piece_footprint_mesh(&mesh, patch, &road);
    }

    #[test]
    fn terminal_footprint_loop_preserves_piece_owned_constraints() {
        let patch = piece_test_patch();
        let road = vec![
            test_vertex(22.0, 8.0),
            test_vertex(38.0, 8.0),
            test_vertex(38.0, 36.0),
            test_vertex(44.0, 40.0),
            test_vertex(38.0, 44.0),
            test_vertex(22.0, 44.0),
            test_vertex(16.0, 40.0),
            test_vertex(22.0, 36.0),
        ];

        let mesh = build_piece_patch(patch, 12, road.clone());

        assert_valid_piece_footprint_mesh(&mesh, patch, &road);
    }

    #[test]
    fn junction_n_footprint_loop_preserves_piece_owned_constraints() {
        let patch = piece_test_patch();
        let road = vec![
            test_vertex(24.0, 4.0),
            test_vertex(36.0, 4.0),
            test_vertex(36.0, 24.0),
            test_vertex(56.0, 24.0),
            test_vertex(56.0, 36.0),
            test_vertex(36.0, 36.0),
            test_vertex(36.0, 56.0),
            test_vertex(24.0, 56.0),
            test_vertex(24.0, 36.0),
            test_vertex(4.0, 36.0),
            test_vertex(4.0, 24.0),
            test_vertex(24.0, 24.0),
        ];

        let first = build_piece_patch(patch, 13, road.clone());
        let second = build_piece_patch(patch, 13, road.clone());

        assert_valid_piece_footprint_mesh(&first, patch, &road);
        assert_eq!(
            canonical_triangle_set(&first.triangles),
            canonical_triangle_set(&second.triangles)
        );
        assert_eq!(first.stats, second.stats);
    }

    #[test]
    fn crossing_road_constraints_are_noded_before_triangulation() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road_a = road_loop_from_centerline(
            TerrainCdtVertex::new(4.0, 0.0, 20.0),
            TerrainCdtVertex::new(36.0, 0.0, 20.0),
            5.0,
        );
        let road_b = road_loop_from_centerline(
            TerrainCdtVertex::new(20.0, 0.0, 4.0),
            TerrainCdtVertex::new(20.0, 0.0, 36.0),
            5.0,
        );

        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![
                TerrainCdtRoadLoop::new(21, 0, road_a),
                TerrainCdtRoadLoop::new(22, 0, road_b),
            ],
            piece_source_samples(),
        ))
        .expect("crossing road loops must not panic the terrain bridge");

        assert_eq!(
            mesh.stats.invalid_constraint_edges, 0,
            "road constraints must be split at deterministic intersections before Spade sees them"
        );
        assert!(
            mesh.stats.road_constraint_edges > 8,
            "crossing road loops should gain noded roadbed constraints"
        );
        for vertex in &mesh.vertices {
            assert!(patch_contains(*vertex, patch));
        }
    }

    #[test]
    fn road_loop_endpoint_on_another_loop_edge_splits_the_roadbed_constraint() {
        let patch = TerrainCdtPatch::new(-96.0, -32.0, 64.0, 64.0, [0.0; 4]);
        let horizontal = vec![
            TerrainCdtVertex::new(-83.390, 0.12, -18.916),
            TerrainCdtVertex::new(49.610, 0.12, -18.916),
            TerrainCdtVertex::new(49.610, 0.12, -8.916),
            TerrainCdtVertex::new(-83.390, 0.12, -8.916),
        ];
        let incoming = vec![
            TerrainCdtVertex::new(-16.818, 0.12, -8.916),
            TerrainCdtVertex::new(-9.747, 0.12, -1.845),
            TerrainCdtVertex::new(-16.818, 0.12, 5.226),
            TerrainCdtVertex::new(-23.889, 0.12, -1.845),
        ];

        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![
                TerrainCdtRoadLoop::new(0, 0, horizontal),
                TerrainCdtRoadLoop::new(1, 0, incoming),
            ],
            Vec::new(),
        ))
        .expect("T-touching terrain roadbed constraints must be accepted");

        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert!(
            mesh.stats.road_constraint_edges > 8,
            "the horizontal roadbed edge must be split at the incoming mouth vertex"
        );
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges,
            mesh.stats.road_constraint_edges
        );
        assert!(mesh.vertices.iter().any(|vertex| {
            same_coord(vertex.x, -16.818)
                && same_coord(vertex.z, -8.916)
                && (vertex.height_m - 0.12).abs() <= 0.0001
        }));
    }

    fn diagonal_road_loop() -> Vec<TerrainCdtVertex> {
        road_loop_from_centerline(
            TerrainCdtVertex::new(8.0, 0.0, 12.0),
            TerrainCdtVertex::new(32.0, 0.0, 28.0),
            6.0,
        )
    }

    fn road_loop_from_centerline(
        start: TerrainCdtVertex,
        end: TerrainCdtVertex,
        width: f64,
    ) -> Vec<TerrainCdtVertex> {
        let dx = end.x - start.x;
        let dz = end.z - start.z;
        let length = (dx * dx + dz * dz).sqrt();
        let normal_x = -dz / length;
        let normal_z = dx / length;
        let half_width = width * 0.5;
        let mut road = vec![
            TerrainCdtVertex::new(
                start.x + normal_x * half_width,
                0.0,
                start.z + normal_z * half_width,
            ),
            TerrainCdtVertex::new(
                end.x + normal_x * half_width,
                0.0,
                end.z + normal_z * half_width,
            ),
            TerrainCdtVertex::new(
                end.x - normal_x * half_width,
                0.0,
                end.z - normal_z * half_width,
            ),
            TerrainCdtVertex::new(
                start.x - normal_x * half_width,
                0.0,
                start.z - normal_z * half_width,
            ),
        ];
        if signed_area(&road) < 0.0 {
            road.reverse();
        }
        road
    }

    fn piece_test_patch() -> TerrainCdtPatch {
        TerrainCdtPatch::new(0.0, 0.0, 60.0, 60.0, [0.0; 4])
    }

    fn test_vertex(x: f64, z: f64) -> TerrainCdtVertex {
        TerrainCdtVertex::new(x, 0.0, z)
    }

    fn build_piece_patch(
        patch: TerrainCdtPatch,
        stable_piece_id: u64,
        road: Vec<TerrainCdtVertex>,
    ) -> TerrainCdtMesh {
        build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![TerrainCdtRoadLoop::new(stable_piece_id, 0, road)],
            piece_source_samples(),
        ))
        .expect("Spade should triangulate a piece-owned road footprint")
    }

    fn piece_source_samples() -> Vec<TerrainCdtVertex> {
        vec![
            test_vertex(6.0, 6.0),
            test_vertex(6.0, 20.0),
            test_vertex(6.0, 40.0),
            test_vertex(6.0, 54.0),
            test_vertex(20.0, 6.0),
            test_vertex(20.0, 54.0),
            test_vertex(40.0, 6.0),
            test_vertex(40.0, 54.0),
            test_vertex(54.0, 6.0),
            test_vertex(54.0, 20.0),
            test_vertex(54.0, 40.0),
            test_vertex(54.0, 54.0),
        ]
    }

    fn build_crossing_patch(patch: TerrainCdtPatch, road: Vec<TerrainCdtVertex>) -> TerrainCdtMesh {
        build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![TerrainCdtRoadLoop::new(7, 0, road)],
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 5.0),
                TerrainCdtVertex::new(5.0, 0.0, 35.0),
                TerrainCdtVertex::new(20.0, 0.0, 5.0),
                TerrainCdtVertex::new(20.0, 0.0, 35.0),
                TerrainCdtVertex::new(35.0, 0.0, 5.0),
                TerrainCdtVertex::new(35.0, 0.0, 35.0),
            ],
        ))
        .expect("Spade should triangulate a clipped road footprint")
    }

    fn assert_valid_clipped_mesh(
        mesh: &TerrainCdtMesh,
        patch: TerrainCdtPatch,
        original_road: &[TerrainCdtVertex],
    ) {
        let clipped_road = ensure_ccw(simplified_loop(clip_loop_to_patch(
            original_road.to_vec(),
            patch,
        )));
        assert!(clipped_road.len() >= 3);
        assert!(!mesh.triangles.is_empty());
        assert!(mesh.stats.rejected_road_faces > 0);
        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges,
            mesh.stats.road_constraint_edges
        );
        for vertex in &mesh.vertices {
            assert!(patch_contains(*vertex, patch));
        }
        for triangle in &mesh.triangles {
            let center = centroid([
                mesh.vertices[triangle[0]],
                mesh.vertices[triangle[1]],
                mesh.vertices[triangle[2]],
            ]);
            assert!(
                !point_in_polygon(center, &clipped_road),
                "accepted terrain triangle leaked into the clipped road footprint"
            );
        }
    }

    fn assert_valid_piece_footprint_mesh(
        mesh: &TerrainCdtMesh,
        patch: TerrainCdtPatch,
        road: &[TerrainCdtVertex],
    ) {
        let road = ensure_ccw(simplified_loop(road.to_vec()));
        assert!(road.len() >= 3);
        assert!(!mesh.triangles.is_empty());
        assert_eq!(mesh.stats.road_constraint_edges, road.len());
        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert!(mesh.stats.rejected_road_faces > 0);
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges,
            mesh.stats.road_constraint_edges
        );
        for vertex in &mesh.vertices {
            assert!(patch_contains(*vertex, patch));
        }
        for triangle in &mesh.triangles {
            let center = centroid([
                mesh.vertices[triangle[0]],
                mesh.vertices[triangle[1]],
                mesh.vertices[triangle[2]],
            ]);
            assert!(
                !point_in_polygon(center, &road),
                "accepted terrain triangle leaked into a piece-owned road footprint"
            );
        }
    }

    fn canonical_triangle_set(triangles: &[[usize; 3]]) -> Vec<[usize; 3]> {
        let mut canonical = triangles
            .iter()
            .map(|triangle| {
                let mut sorted = *triangle;
                sorted.sort_unstable();
                sorted
            })
            .collect::<Vec<_>>();
        canonical.sort_unstable();
        canonical
    }
}
