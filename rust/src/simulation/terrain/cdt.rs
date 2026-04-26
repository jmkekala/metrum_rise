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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerrainCdtError {
    InvalidPatch,
    InvalidRoadLoop,
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
    let cdt = SpadeCdt::bulk_load_cdt(spade_vertices, canonical.constraints.clone())
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
    let patch_indices = patch_corners
        .iter()
        .map(|&vertex| insert_vertex(vertex, &mut vertices, &mut vertex_lookup))
        .collect::<Vec<_>>();
    push_loop_constraints(&patch_indices, &mut constraint_set, None);

    input
        .road_loops
        .sort_by_key(|road_loop| (road_loop.stable_piece_id, road_loop.local_loop_index));
    for road_loop in input.road_loops {
        let points = simplified_loop(road_loop.vertices);
        if points.len() < 3 || signed_area(&points).abs() <= CDT_EPSILON_M * CDT_EPSILON_M {
            return Err(TerrainCdtError::InvalidRoadLoop);
        }
        let points = ensure_ccw(points);
        let loop_indices = points
            .iter()
            .map(|&vertex| insert_vertex(vertex, &mut vertices, &mut vertex_lookup))
            .collect::<Vec<_>>();
        push_loop_constraints(
            &loop_indices,
            &mut constraint_set,
            Some(&mut road_constraint_edges),
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
        if road_loops
            .iter()
            .any(|road_loop| point_in_polygon(sample, road_loop))
        {
            continue;
        }
        insert_vertex(sample, &mut vertices, &mut vertex_lookup);
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

fn push_loop_constraints(
    indices: &[usize],
    constraint_set: &mut BTreeSet<[usize; 2]>,
    mut road_constraint_edges: Option<&mut Vec<[usize; 2]>>,
) {
    for index in 0..indices.len() {
        let edge = [indices[index], indices[(index + 1) % indices.len()]];
        if edge[0] == edge[1] {
            continue;
        }
        constraint_set.insert(edge);
        if let Some(edges) = road_constraint_edges.as_deref_mut() {
            edges.push(edge);
        }
    }
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
    if deduplicated.len() > 1
        && same_xz(deduplicated[0], *deduplicated.last().unwrap())
    {
        deduplicated.pop();
    }
    deduplicated
}

fn ensure_ccw(mut points: Vec<TerrainCdtVertex>) -> Vec<TerrainCdtVertex> {
    if signed_area(&points) < 0.0 {
        points.reverse();
    }
    points
}

fn same_xz(a: TerrainCdtVertex, b: TerrainCdtVertex) -> bool {
    quantized_coord(a.x) == quantized_coord(b.x)
        && quantized_coord(a.z) == quantized_coord(b.z)
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

    fn diagonal_road_loop() -> Vec<TerrainCdtVertex> {
        let start = TerrainCdtVertex::new(8.0, 0.0, 12.0);
        let end = TerrainCdtVertex::new(32.0, 0.0, 28.0);
        let dx = end.x - start.x;
        let dz = end.z - start.z;
        let length = (dx * dx + dz * dz).sqrt();
        let normal_x = -dz / length;
        let normal_z = dx / length;
        let mut road = vec![
            TerrainCdtVertex::new(start.x + normal_x * 3.0, 0.0, start.z + normal_z * 3.0),
            TerrainCdtVertex::new(end.x + normal_x * 3.0, 0.0, end.z + normal_z * 3.0),
            TerrainCdtVertex::new(end.x - normal_x * 3.0, 0.0, end.z - normal_z * 3.0),
            TerrainCdtVertex::new(start.x - normal_x * 3.0, 0.0, start.z - normal_z * 3.0),
        ];
        if signed_area(&road) < 0.0 {
            road.reverse();
        }
        road
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
