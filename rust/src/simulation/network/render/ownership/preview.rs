// SPDX-License-Identifier: GPL-2.0-only

//! Attribute-preserving preview partitions at the bounded junction/profile edit footprint.

use super::{NetworkMeshData, NetworkMeshOwner};
use godot::prelude::{Color, Vector2, Vector3};
use std::collections::HashSet;

#[derive(Clone, Copy, Default)]
struct Vertex {
    p: Vector3,
    n: Vector3,
    uv: Vector2,
    c: Color,
}

impl Vertex {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            p: self.p.lerp(other.p, t),
            n: self.n.lerp(other.n, t),
            uv: self.uv.lerp(other.uv, t),
            c: Color::from_rgba(
                self.c.r + (other.c.r - self.c.r) * t,
                self.c.g + (other.c.g - self.c.g) * t,
                self.c.b + (other.c.b - self.c.b) * t,
                self.c.a + (other.c.a - self.c.a) * t,
            ),
        }
    }
}

// Convex triangle/rectangle intersections have at most seven vertices. Fixed buffers avoid
// allocating per triangle; the two reusable fragment vectors handle unions of edit bounds.
#[derive(Clone, Copy)]
struct Polygon {
    vertices: [Vertex; 8],
    len: usize,
}

impl Polygon {
    fn split(self, axis: usize, boundary: f32) -> (Self, Self) {
        let mut low = Self {
            vertices: [Vertex::default(); 8],
            len: 0,
        };
        let mut high = low;
        let coordinate = |v: &Vertex| if axis == 0 { v.p.x } else { v.p.z };
        if self.vertices[..self.len]
            .iter()
            .all(|v| coordinate(v) <= boundary)
        {
            return (self, high);
        }
        if self.vertices[..self.len]
            .iter()
            .all(|v| coordinate(v) >= boundary)
        {
            return (low, self);
        }
        for i in 0..self.len {
            let a = self.vertices[i];
            let b = self.vertices[(i + 1) % self.len];
            let av = if axis == 0 { a.p.x } else { a.p.z };
            let bv = if axis == 0 { b.p.x } else { b.p.z };
            if av <= boundary {
                low.vertices[low.len] = a;
                low.len += 1;
            }
            if av >= boundary {
                high.vertices[high.len] = a;
                high.len += 1;
            }
            if (av < boundary && bv > boundary) || (av > boundary && bv < boundary) {
                let mut v = a.lerp(b, (boundary - av) / (bv - av));
                if axis == 0 {
                    v.p.x = boundary;
                } else {
                    v.p.z = boundary;
                }
                low.vertices[low.len] = v;
                low.len += 1;
                high.vertices[high.len] = v;
                high.len += 1;
            }
        }
        (low, high)
    }
}

impl NetworkMeshData {
    /// Partitions only selected owners. Entirely untouched triangles keep all attributes bitwise.
    /// Bounds are the solver's junction/profile support envelope, in world XZ coordinates.
    /// O(local mesh triangles × local edit bounds), with reusable clipping scratch buffers.
    pub(crate) fn preview_partition(
        &self,
        whole: &HashSet<NetworkMeshOwner>,
        bounded: &HashSet<NetworkMeshOwner>,
        bounds: &[[f32; 4]],
        origin: Vector2,
        inside: bool,
    ) -> Self {
        let mut output = Self::new();
        let mut pending = Vec::new();
        let mut remaining = Vec::new();
        let mut selected = Vec::new();
        macro_rules! layer {
            ($index:expr, $p:ident, $n:ident, $uv:ident, $c:ident) => {{
                let mut range_index = 0;
                // Owner membership is constant within a range, so hash once per range, not per
                // triangle.
                let mut classified_range = usize::MAX;
                let (mut full, mut partial) = (false, false);
                for start in (0..self.$p.len()).step_by(3) {
                    while self.owner_ranges[$index]
                        .get(range_index)
                        .is_some_and(|r| r.end <= start)
                    {
                        range_index += 1;
                    }
                    let owner = self.owner_ranges[$index]
                        .get(range_index)
                        .and_then(|r| r.owner);
                    if classified_range != range_index {
                        classified_range = range_index;
                        full = owner.is_some_and(|o| whole.contains(&o));
                        partial = owner.is_some_and(|o| bounded.contains(&o));
                    }
                    selected.clear();
                    let mut triangle = Polygon {
                        vertices: [Vertex::default(); 8],
                        len: 3,
                    };
                    for i in 0..3 {
                        triangle.vertices[i] = Vertex {
                            p: self.$p[start + i],
                            n: self.$n[start + i],
                            uv: self.$uv[start + i],
                            c: self.$c[start + i],
                        };
                    }
                    if full || !partial {
                        if full == inside {
                            selected.push(triangle);
                        }
                    } else {
                        pending.clear();
                        pending.push(triangle);
                        for bound in bounds {
                            remaining.clear();
                            for polygon in pending.drain(..) {
                                let mut inner = polygon;
                                for (axis, boundary, keep_high) in [
                                    (0, bound[0] - origin.x, true),
                                    (0, bound[2] - origin.x, false),
                                    (1, bound[1] - origin.y, true),
                                    (1, bound[3] - origin.y, false),
                                ] {
                                    if inner.len < 3 {
                                        break;
                                    }
                                    let (low, high) = inner.split(axis, boundary);
                                    let outer;
                                    (inner, outer) =
                                        if keep_high { (high, low) } else { (low, high) };
                                    if outer.len >= 3 {
                                        remaining.push(outer);
                                    }
                                }
                                if inside && inner.len >= 3 {
                                    selected.push(inner);
                                }
                            }
                            std::mem::swap(&mut pending, &mut remaining);
                        }
                        if !inside {
                            selected.extend_from_slice(&pending);
                        }
                    }
                    for polygon in &selected {
                        for i in 1..polygon.len.saturating_sub(1) {
                            let vertices = [
                                polygon.vertices[0],
                                polygon.vertices[i],
                                polygon.vertices[i + 1],
                            ];
                            if (vertices[1].p - vertices[0].p)
                                .cross(vertices[2].p - vertices[0].p)
                                .length_squared()
                                == 0.0
                            {
                                continue;
                            }
                            for v in vertices {
                                output.$p.push(v.p);
                                output.$n.push(v.n);
                                output.$uv.push(v.uv);
                                output.$c.push(v.c);
                            }
                            output.current_owner = owner;
                            output.record_owner_triangle($index, output.$p.len());
                        }
                    }
                }
            }};
        }
        layer!(
            0,
            earthwork_vertices,
            earthwork_normals,
            earthwork_uvs,
            earthwork_colors
        );
        layer!(1, curb_vertices, curb_normals, curb_uvs, curb_colors);
        layer!(
            2,
            raised_step_vertices,
            raised_step_normals,
            raised_step_uvs,
            raised_step_colors
        );
        layer!(
            3,
            sidewalk_vertices,
            sidewalk_normals,
            sidewalk_uvs,
            sidewalk_colors
        );
        layer!(4, road_vertices, road_normals, road_uvs, road_colors);
        layer!(
            5,
            marking_vertices,
            marking_normals,
            marking_uvs,
            marking_colors
        );
        layer!(
            6,
            concrete_vertices,
            concrete_normals,
            concrete_uvs,
            concrete_colors
        );
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_edit_bounds_partition_triangles_without_loss_or_duplicate_area() {
        let owner = NetworkMeshOwner::Edge(7);
        let mut mesh = NetworkMeshData::new();
        mesh.set_owner(owner);
        for p in [
            Vector3::new(-8.0, -4.0, -8.0),
            Vector3::new(8.0, 4.0, -8.0),
            Vector3::new(0.0, 0.0, 8.0),
        ] {
            mesh.road_vertices.push(p);
            mesh.road_normals.push(Vector3::UP);
            mesh.road_uvs.push(Vector2::new(p.x, p.z));
            mesh.road_colors.push(Color::WHITE);
        }
        mesh.record_owner_triangle(4, 3);
        let bounded = HashSet::from([owner]);
        let bounds = [[97.0, 197.0, 102.0, 202.0], [100.0, 199.0, 105.0, 205.0]];
        let origin = Vector2::new(100.0, 200.0);
        let inside = mesh.preview_partition(&HashSet::new(), &bounded, &bounds, origin, true);
        let outside = mesh.preview_partition(&HashSet::new(), &bounded, &bounds, origin, false);
        let area = |mesh: &NetworkMeshData| {
            mesh.road_vertices
                .chunks_exact(3)
                .map(|t| {
                    ((t[1].x - t[0].x) * (t[2].z - t[0].z) - (t[1].z - t[0].z) * (t[2].x - t[0].x))
                        .abs()
                        * 0.5
                })
                .sum::<f32>()
        };
        assert!((area(&inside) + area(&outside) - area(&mesh)).abs() < 0.0001);
        assert!(area(&inside) > 0.0 && area(&outside) > 0.0);
        for part in [&inside, &outside] {
            for (p, uv) in part.road_vertices.iter().zip(&part.road_uvs) {
                assert!((p.y - p.x * 0.5).abs() < 0.00001);
                assert!((uv.x - p.x).abs() < 0.00001 && (uv.y - p.z).abs() < 0.00001);
            }
        }
        let untouched = mesh.preview_partition(
            &HashSet::new(),
            &bounded,
            &[[200.0, 300.0, 210.0, 310.0]],
            origin,
            false,
        );
        assert_eq!(mesh.road_vertices, untouched.road_vertices);
        assert_eq!(mesh.road_uvs, untouched.road_uvs);
    }
}
