// SPDX-License-Identifier: GPL-2.0-only

//! Attribute-preserving convex-polygon splits, used to clip crosswalk quads at band boundaries.

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
