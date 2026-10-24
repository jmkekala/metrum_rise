// SPDX-License-Identifier: GPL-2.0-only

//! Exact material partition of existing support triangles; never chooses new ground heights.

use super::model::{BuildingSiteClient, BuildingSiteSurfaceClient};
use crate::assets::SiteSurfaceMaterial;
use crate::simulation::network::roadbed::{RoadVec3, triangulate_constrained_rings_xz};
use godot::prelude::{Vector2, Vector3};
use rstar::{AABB, RTree, RTreeObject};

// One convex material region with its interior on the left of every edge: a whole convex
// surface, or one triangle of a non-convex surface.
struct PavingRegion {
    polygon: Vec<RoadVec3>,
    order: usize,
    material: SiteSurfaceMaterial,
}

impl RTreeObject for PavingRegion {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        bounds(&self.polygon)
    }
}

/// Reuses the road polygon CDT and R-tree for one bounded set of site materials.
/// Construction is O(paving vertices + triangulation + T log T); queries visit
/// only overlapping material regions, not every building or every site polygon.
pub(crate) struct SitePavingPartition {
    regions: RTree<PavingRegion>,
}

impl SitePavingPartition {
    /// First authored region wins overlaps; caller supplies stable building/asset order.
    /// A convex surface is clipped against as a whole: splitting it into triangles would add
    /// internal edges whose split points two neighbouring pieces compute apart (`ROAD-68-D`).
    pub(crate) fn new(surfaces: &[BuildingSiteSurfaceClient]) -> Option<Self> {
        let mut regions = Vec::new();
        for surface in surfaces {
            let mut ring: Vec<_> = surface
                .vertices_world
                .iter()
                .map(|p| RoadVec3::new(f64::from(p.x), 0.0, f64::from(p.y)))
                .collect();
            if signed_area(&ring) < 0.0 {
                ring.reverse();
            }
            if strictly_convex(&ring) {
                regions.push(PavingRegion {
                    polygon: ring,
                    order: regions.len(),
                    material: surface.material,
                });
                continue;
            }
            for mut triangle in triangulate_constrained_rings_xz(&[ring])? {
                if cross(triangle[0], triangle[1], triangle[2]) < 0.0 {
                    triangle.swap(1, 2);
                }
                regions.push(PavingRegion {
                    polygon: triangle.to_vec(),
                    order: regions.len(),
                    material: surface.material,
                });
            }
        }
        Some(Self {
            regions: RTree::bulk_load(regions),
        })
    }

    /// Splits geometry only at material boundaries. Every emitted point lies on
    /// the original triangle, so paving/grass and road seams keep identical heights.
    pub(crate) fn partition(
        &self,
        triangle: [Vector3; 3],
        mut emit: impl FnMut(Option<SiteSurfaceMaterial>, [Vector3; 3]),
    ) {
        let triangle =
            triangle.map(|p| RoadVec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)));
        let mut candidates: Vec<_> = self
            .regions
            .locate_in_envelope_intersecting(&bounds(&triangle))
            .collect();
        if candidates.is_empty() {
            emit_triangle(None, triangle, &mut emit);
            return;
        }
        candidates.sort_unstable_by_key(|candidate| candidate.order);
        let mut remaining = vec![triangle.to_vec()];
        for candidate in candidates {
            let mut next = Vec::new();
            for polygon in remaining {
                let mut inside = polygon;
                let corners = candidate.polygon.len();
                for edge in 0..corners {
                    let (kept, outside) = split(
                        &inside,
                        candidate.polygon[edge],
                        candidate.polygon[(edge + 1) % corners],
                    );
                    if outside.len() >= 3 {
                        next.push(outside);
                    }
                    inside = kept;
                    if inside.len() < 3 {
                        break;
                    }
                }
                emit_fan(Some(candidate.material), &inside, &mut emit);
            }
            remaining = next;
            if remaining.is_empty() {
                break;
            }
        }
        for polygon in remaining {
            emit_fan(None, &polygon, &mut emit);
        }
    }
}

impl BuildingSiteClient {
    /// Foundation only. Paving beyond the foundation is part of the terrain mesh.
    pub(crate) fn foundation_mesh(&self) -> &[(Option<SiteSurfaceMaterial>, [Vector3; 3])] {
        self.foundation_mesh.get_or_init(|| {
            let Some(triangles) =
                site_foundation_triangles(&self.footprint_world, self.support_height_m)
            else {
                return Vec::new();
            };
            let Some(partition) = SitePavingPartition::new(&self.surfaces) else {
                return Vec::new();
            };
            let mut result = Vec::new();
            for triangle in triangles {
                partition.partition(triangle, |material, mut triangle| {
                    // Godot's front face uses clockwise winding viewed from above.
                    if (triangle[1] - triangle[0])
                        .cross(triangle[2] - triangle[0])
                        .y
                        > 0.0
                    {
                        triangle.swap(1, 2);
                    }
                    result.push((material, triangle));
                });
            }
            result
        })
    }
}

// Foundation topology uses the same canonical triangulator as roads and material regions.
fn site_foundation_triangles(points: &[Vector2], height: f32) -> Option<Vec<[Vector3; 3]>> {
    let ring: Vec<_> = points
        .iter()
        .map(|p| RoadVec3::new(f64::from(p.x), f64::from(height), f64::from(p.y)))
        .collect();
    Some(
        triangulate_constrained_rings_xz(&[ring])?
            .into_iter()
            .map(|triangle| triangle.map(to_vector3))
            .collect(),
    )
}

fn bounds(points: &[RoadVec3]) -> AABB<[f64; 2]> {
    let (low, high) = points.iter().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(low, high), p| {
            (
                [low[0].min(p.x), low[1].min(p.z)],
                [high[0].max(p.x), high[1].max(p.z)],
            )
        },
    );
    AABB::from_corners(low, high)
}

fn cross(a: RoadVec3, b: RoadVec3, p: RoadVec3) -> f64 {
    (b.x - a.x) * (p.z - a.z) - (b.z - a.z) * (p.x - a.x)
}

fn signed_area(ring: &[RoadVec3]) -> f64 {
    (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            a.x * b.z - b.x * a.z
        })
        .sum()
}

// Every corner turns left: no repeated, collinear or reflex corner.
fn strictly_convex(ring: &[RoadVec3]) -> bool {
    let n = ring.len();
    n >= 3 && (0..n).all(|i| cross(ring[i], ring[(i + 1) % n], ring[(i + 2) % n]) > 0.0)
}

// The input and both outputs are convex. The same intersection value goes to
// both sides; there is no tolerance strip, discarded sliver, or sampled height.
fn split(points: &[RoadVec3], a: RoadVec3, b: RoadVec3) -> (Vec<RoadVec3>, Vec<RoadVec3>) {
    let mut inside = Vec::new();
    let mut outside = Vec::new();
    for (index, &point) in points.iter().enumerate() {
        let next = points[(index + 1) % points.len()];
        let d = cross(a, b, point);
        let next_d = cross(a, b, next);
        if d >= 0.0 {
            inside.push(point);
        }
        if d <= 0.0 {
            outside.push(point);
        }
        if (d < 0.0 && next_d > 0.0) || (d > 0.0 && next_d < 0.0) {
            // From the segment's lexicographically lower end, so the two faces that share the
            // segment (it runs the other way in one of them) split it at the same point.
            let (from, to, from_d, to_d) = if (point.x, point.z) <= (next.x, next.z) {
                (point, next, d, next_d)
            } else {
                (next, point, next_d, d)
            };
            let hit = from + (to - from) * (from_d / (from_d - to_d));
            inside.push(hit);
            outside.push(hit);
        }
    }
    (inside, outside)
}

fn emit_fan(
    material: Option<SiteSurfaceMaterial>,
    points: &[RoadVec3],
    emit: &mut impl FnMut(Option<SiteSurfaceMaterial>, [Vector3; 3]),
) {
    for i in 1..points.len().saturating_sub(1) {
        emit_triangle(material, [points[0], points[i], points[i + 1]], emit);
    }
}

fn emit_triangle(
    material: Option<SiteSurfaceMaterial>,
    triangle: [RoadVec3; 3],
    emit: &mut impl FnMut(Option<SiteSurfaceMaterial>, [Vector3; 3]),
) {
    if cross(triangle[0], triangle[1], triangle[2]) != 0.0 {
        emit(material, triangle.map(to_vector3));
    }
}

fn to_vector3(p: RoadVec3) -> Vector3 {
    Vector3::new(p.x as f32, p.y as f32, p.z as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paving_partitions_sloped_ground_without_gaps_or_height_changes() {
        let paving = SitePavingPartition::new(&[BuildingSiteSurfaceClient {
            material: SiteSurfaceMaterial::Asphalt,
            name: String::new(),
            vertices_world: vec![
                Vector2::new(1.0, -1.0),
                Vector2::new(3.0, -1.0),
                Vector2::new(3.0, 5.0),
                Vector2::new(1.0, 5.0),
            ],
        }])
        .unwrap();
        for grade in [-0.4, 0.4] {
            let mut area = 0.0;
            let mut paved_area = 0.0;
            paving.partition(
                [
                    Vector3::new(0.0, 0.0, 0.0),
                    Vector3::new(4.0, grade * 4.0, 0.0),
                    Vector3::new(0.0, 0.0, 4.0),
                ],
                |material, triangle| {
                    let [a, b, c] = triangle;
                    let value = ((b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)).abs() * 0.5;
                    area += value;
                    if material.is_some() {
                        paved_area += value;
                    }
                    for p in triangle {
                        assert!((p.y - grade * p.x).abs() < 0.00001);
                    }
                },
            );
            assert!((area - 8.0).abs() < 0.00001, "coverage area {area}");
            assert!(
                (paved_area - 4.0).abs() < 0.00001,
                "paving area {paved_area}"
            );
        }
    }
}
