// SPDX-License-Identifier: GPL-2.0-only

//! Attribute-preserving preview partitions at the bounded junction/profile edit footprint.

use super::NetworkMeshData;
use godot::prelude::{Color, Vector2, Vector3, Vector4};

/// Attributes interpolated when a road triangle is partitioned.
#[derive(Clone, Copy, Default)]
pub(in crate::simulation::network::render) struct Vertex {
    pub(in crate::simulation::network::render) p: Vector3,
    pub(in crate::simulation::network::render) n: Vector3,
    pub(in crate::simulation::network::render) uv: Vector2,
    pub(in crate::simulation::network::render) c: Color,
    pub(in crate::simulation::network::render) crossing: Vector4,
}

impl Vertex {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            p: self.p.lerp(other.p, t),
            n: self.n.lerp(other.n, t),
            uv: self.uv.lerp(other.uv, t),
            crossing: self.crossing.lerp(other.crossing, t),
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
/// Fixed scratch polygon shared by preview and crossing partitions.
#[derive(Clone, Copy)]
pub(in crate::simulation::network::render) struct Polygon {
    pub(in crate::simulation::network::render) vertices: [Vertex; 8],
    pub(in crate::simulation::network::render) len: usize,
}

impl Polygon {
    /// Splits at a linear coordinate, preserving all surface attributes.
    pub(in crate::simulation::network::render) fn split_by(
        self,
        coordinate: impl Fn(&Vertex) -> f32,
        boundary: f32,
    ) -> (Self, Self) {
        self.split_impl(coordinate, boundary, |_| {})
    }

    fn split_impl(
        self,
        coordinate: impl Fn(&Vertex) -> f32,
        boundary: f32,
        snap: impl Fn(&mut Vertex),
    ) -> (Self, Self) {
        let mut low = Self {
            vertices: [Vertex::default(); 8],
            len: 0,
        };
        let mut high = low;
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
            let av = coordinate(&a);
            let bv = coordinate(&b);
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
                snap(&mut v);
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
            mesh.road_crossings.push(Vector4::new(p.x, p.z, 1.5, p.x));
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
                .as_chunks::<3>()
                .0
                .iter()
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
        for part in [&inside, &outside] {
            for (p, v) in part.road_vertices.iter().zip(&part.road_crossings) {
                assert!((v.x - p.x).abs() < 1e-5 && (v.y - p.z).abs() < 1e-5);
                assert!((v.z - 1.5).abs() < 1e-5 && (v.w - p.x).abs() < 1e-5);
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
        assert_eq!(mesh.road_crossings, untouched.road_crossings);
    }
}
