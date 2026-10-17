// SPDX-License-Identifier: GPL-2.0-only

//! Finnish zebra masks partition the existing asphalt triangles, never an offset paint plane.

use super::super::ownership::{Polygon, Vertex};
use super::*;

// Finnish L3_2 at urban speeds: planar metres, preserved from the existing marking contract.
pub(super) const CROSSWALK_STRIPE_WIDTH: f32 = 0.5;
pub(super) const CROSSWALK_STRIPE_LEN: f32 = 2.5;
const CROSSWALK_STRIPE_GAP: f32 = 0.5;

/// Local paint frame shared by the span and junction owning a crossing.
#[derive(Clone, Copy, Debug)]
pub(in crate::simulation::network::render) struct Crossing {
    origin: Vector3,
    across: Vector3,
    along: Vector3,
    width: f32,
    margin: f32,
    age: f32,
}

impl Crossing {
    fn new(marking: crate::simulation::network::lanes::CrosswalkMarking, age: f32) -> Option<Self> {
        let delta = marking.end - marking.start;
        let length = Vector2::new(delta.x, delta.z).length();
        let step = CROSSWALK_STRIPE_WIDTH + CROSSWALK_STRIPE_GAP;
        let count = ((length + CROSSWALK_STRIPE_GAP) / step).floor();
        if count < 1.0 {
            return None;
        }
        let width = count * step - CROSSWALK_STRIPE_GAP;
        let margin = (length - width) * 0.5;
        let across = Vector3::new(delta.x / length, 0.0, delta.z / length);
        Some(Self {
            origin: marking.start + across * margin,
            across,
            along: Vector3::new(-across.z, 0.0, across.x),
            width,
            margin,
            age,
        })
    }

    fn uv(self, p: Vector3) -> Vector2 {
        let d = p - self.origin;
        Vector2::new(d.dot(self.across), d.dot(self.along))
    }
}

/// Registers local crossing frames before emitting asphalt triangles.
pub(super) fn prepare_crosswalks(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    lane_system: &crate::simulation::network::lanes::LaneSystem,
    coverage: &super::standard_surface::CompiledSurfaceCoverage,
) {
    // A crossing may sit in a span chunk whose junction mesh is in the neighboring chunk.
    let mut nodes: BTreeSet<_> = coverage.node_ids.iter().copied().collect();
    for &edge_id in &coverage.edge_indices {
        let edge = graph.edge(edge_id);
        nodes.extend([edge.start_node, edge.end_node]);
    }
    for node_id in nodes {
        let Some(lane_ids) = lane_system.node_lanes.get(&(node_id as usize)) else {
            continue;
        };
        for &lane_id in lane_ids {
            let Some(marking) = lane_system.crosswalk_marking(lane_id) else {
                continue;
            };
            let Some(crossing) =
                Crossing::new(marking, cosmetic_road_age(graph.edge(marking.edge_id)))
            else {
                continue;
            };
            for owner in [
                NetworkMeshOwner::Node(node_id),
                NetworkMeshOwner::Edge(marking.edge_id),
            ] {
                mesh.crosswalks.entry(owner).or_default().push(crossing);
            }
        }
    }
}

/// Partitions an asphalt triangle while preserving its original surface attributes.
/// O(K * (T + V)) for T input and V output triangles of the touched owner and K local
/// crossings, never city-wide. Span K is at most two; node K follows its incident arms.
/// Each rectangle cuts a triangle into at most nine triangles. Fixed polygon scratch avoids
/// per-triangle allocations; recursion advances through local crossings only.
pub(super) fn emit_triangle(mesh: &mut NetworkMeshData, triangle: [Vertex; 3], next: usize) {
    let crossing = mesh
        .current_owner
        .and_then(|owner| mesh.crosswalks.get(&owner))
        .and_then(|crossings| crossings.get(next))
        .copied();
    let Some(crossing) = crossing else {
        push_triangle_to_layer_with_crossing(
            mesh,
            MeshLayer::Road,
            triangle.map(|v| v.p),
            triangle.map(|v| v.uv),
            triangle[0].c,
            triangle[0].n,
            [Vector4::ZERO; 3],
        );
        return;
    };
    let coords = triangle.map(|v| crossing.uv(v.p));
    let half = CROSSWALK_STRIPE_LEN * 0.5;
    if coords.iter().all(|uv| uv.x <= 0.0)
        || coords.iter().all(|uv| uv.x >= crossing.width)
        || coords.iter().all(|uv| uv.y <= -half)
        || coords.iter().all(|uv| uv.y >= half)
    {
        emit_triangle(mesh, triangle, next + 1);
        return;
    }
    let mut inner = Polygon {
        vertices: [Vertex::default(); 8],
        len: 3,
    };
    inner.vertices[..3].copy_from_slice(&triangle);
    for (axis, boundary, keep_high) in [
        (0, 0.0, true),
        (0, crossing.width, false),
        (1, -half, true),
        (1, half, false),
    ] {
        if inner.len < 3 {
            break;
        }
        let (low, high) = inner.split_by(
            |v| {
                let uv = crossing.uv(v.p);
                if axis == 0 { uv.x } else { uv.y }
            },
            boundary,
        );
        let outer;
        (inner, outer) = if keep_high { (high, low) } else { (low, high) };
        for i in 1..outer.len.saturating_sub(1) {
            emit_triangle(
                mesh,
                [outer.vertices[0], outer.vertices[i], outer.vertices[i + 1]],
                next + 1,
            );
        }
    }
    for i in 1..inner.len.saturating_sub(1) {
        let vertices = [inner.vertices[0], inner.vertices[i], inner.vertices[i + 1]];
        let paint = vertices.map(|v| {
            let uv = crossing.uv(v.p);
            // CUSTOM0: stripe metres, longitudinal metres, age+1 (zero means no crossing),
            // distance from the kerb for wheel-path phase, including the symmetric margin.
            Vector4::new(uv.x, uv.y, crossing.age + 1.0, uv.x + crossing.margin)
        });
        push_triangle_to_layer_with_crossing(
            mesh,
            MeshLayer::Road,
            vertices.map(|v| v.p),
            vertices.map(|v| v.uv),
            vertices[0].c,
            vertices[0].n,
            paint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::lanes::CrosswalkMarking;

    #[test]
    fn zebra_partitions_preserve_surface_and_planar_dimensions() {
        // A diagonal, sloping road: crossing geometry must not inherit the crossfall tilt.
        let across = Vector3::new(0.6, 0.0, 0.8);
        let along = Vector3::new(-0.8, 0.0, 0.6);
        let point = |s: f32, l: f32| {
            let p = along * s + across * l;
            Vector3::new(p.x, 2.0 + s * 0.1 + l * 0.05, p.z)
        };
        let crossing = Crossing::new(
            CrosswalkMarking {
                edge_id: 0,
                start: point(0.0, -3.5),
                end: point(0.0, 3.5),
            },
            0.7,
        )
        .unwrap();
        assert!((crossing.width - 6.5).abs() < 1e-5);
        assert!((crossing.margin - 0.25).abs() < 1e-5);
        let mut mesh = NetworkMeshData::new();
        let owner = NetworkMeshOwner::Edge(0);
        mesh.set_owner(owner);
        mesh.crosswalks.insert(owner, vec![crossing]);
        push_quad(
            &mut mesh,
            MeshLayer::Road,
            [
                point(-4.0, -3.5),
                point(-4.0, 3.5),
                point(4.0, 3.5),
                point(4.0, -3.5),
            ],
            [
                Vector2::new(-4.0, -3.5),
                Vector2::new(-4.0, 3.5),
                Vector2::new(4.0, 3.5),
                Vector2::new(4.0, -3.5),
            ],
            road_color(),
        );
        assert!(mesh.marking_vertices.is_empty());
        assert_eq!(mesh.road_crossings.len(), mesh.road_vertices.len());
        let mut area = 0.0;
        let mut painted_area = 0.0;
        for (tri, paint) in mesh
            .road_vertices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(mesh.road_crossings.as_chunks::<3>().0.iter())
        {
            let a = ((tri[1] - tri[0]).cross(tri[2] - tri[0])).y.abs() * 0.5;
            area += a;
            if paint[0].z > 0.0 {
                painted_area += a;
                for v in paint {
                    assert!((v.z - 1.7).abs() < 1e-5);
                    assert!(v.x >= -1e-5 && v.x <= 6.5 + 1e-5);
                    assert!(v.y.abs() <= CROSSWALK_STRIPE_LEN * 0.5 + 1e-5);
                    assert!((v.w - v.x - 0.25).abs() < 1e-5);
                }
            }
        }
        assert!((area - 56.0).abs() < 1e-4);
        assert!((painted_area - 6.5 * 2.5).abs() < 1e-4);
        for (p, uv) in mesh.road_vertices.iter().zip(&mesh.road_uvs) {
            assert!((p.y - (2.0 + uv.x * 0.1 + uv.y * 0.05)).abs() < 1e-5);
            assert!(
                Vector2::new(p.x - point(uv.x, uv.y).x, p.z - point(uv.x, uv.y).z).length() < 1e-5
            );
        }
    }
}
