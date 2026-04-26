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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TerrainCdtStats {
    pub(crate) input_vertices: usize,
    pub(crate) constraint_edges: usize,
    pub(crate) road_constraint_edges: usize,
    pub(crate) accepted_faces: usize,
    pub(crate) rejected_road_faces: usize,
    pub(crate) preserved_road_constraint_edges: usize,
    pub(crate) invalid_constraint_edges: usize,
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
    let cdt = SpadeCdt::try_bulk_load_cdt(spade_vertices, canonical.constraints.clone(), |_| {
        invalid_constraint_edges += 1
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

    Ok(TerrainCdtMesh {
        stats: TerrainCdtStats {
            input_vertices: canonical.vertices.len(),
            constraint_edges: canonical.constraints.len(),
            road_constraint_edges: canonical.road_constraint_edges.len(),
            accepted_faces: triangles.len(),
            rejected_road_faces,
            preserved_road_constraint_edges,
            invalid_constraint_edges,
        },
        vertices: canonical.vertices,
        triangles,
    })
}

struct CanonicalTerrainCdtInput {
    vertices: Vec<TerrainCdtVertex>,
    constraints: Vec<[usize; 2]>,
    road_constraint_edges: Vec<[usize; 2]>,
    road_loops: Vec<Vec<TerrainCdtVertex>>,
}

fn canonicalize_input(
    mut input: TerrainCdtInput,
) -> Result<CanonicalTerrainCdtInput, TerrainCdtError> {
    let mut vertices = Vec::new();
    let mut vertex_lookup = BTreeMap::new();
    let mut constraint_set = BTreeSet::new();
    let mut road_constraint_edges = Vec::new();
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
            &mut road_constraint_edges,
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
        insert_vertex(sample, &mut vertices, &mut vertex_lookup);
    }

    push_patch_boundary_constraints(input.patch, &vertices, &mut constraint_set);
    for edge in &road_constraint_edges {
        insert_constraint(*edge, &mut constraint_set);
    }

    Ok(CanonicalTerrainCdtInput {
        vertices,
        constraints: constraint_set.into_iter().collect(),
        road_constraint_edges,
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
    road_constraint_edges: &mut Vec<[usize; 2]>,
) {
    for index in 0..indices.len() {
        let edge = normalize_edge_array(indices[index], indices[(index + 1) % indices.len()]);
        if edge[0] == edge[1] {
            continue;
        }
        if !edge_lies_on_patch_boundary(vertices[edge[0]], vertices[edge[1]], patch) {
            road_constraint_edges.push(edge);
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
    fn conflicting_road_constraints_are_reported_without_panicking() {
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
        .expect("conflicting road loops must not panic the terrain bridge");

        assert!(
            mesh.stats.invalid_constraint_edges > 0,
            "overlapping road loops must be reported as skipped CDT constraints"
        );
        for vertex in &mesh.vertices {
            assert!(patch_contains(*vertex, patch));
        }
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
