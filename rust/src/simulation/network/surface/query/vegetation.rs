// SPDX-License-Identifier: GPL-2.0-only

//! Allocation-free vegetation clearance against compiled visible top surfaces.

use super::super::{RoadSurfaceBandKind, RoadSurfaceSystem, RoadVec2, RoadVec3};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;

impl RoadSurfaceSystem {
    /// Tests a disc against visible top-surface bands whose kind passes `kinds`, including
    /// node corners. Uses existing query chunks and owner-local triangle grids; work follows
    /// touched cells and local entries, without collecting owners or scanning whole edges.
    pub(crate) fn surface_near_disc(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        point: RoadVec2,
        radius: f64,
        kinds: impl Fn(RoadSurfaceBandKind) -> bool,
    ) -> bool {
        let bounds = [
            point.x - radius,
            point.y - radius,
            point.x + radius,
            point.y + radius,
        ];
        let first = Self::query_chunk_coords_for_world(bounds[0], bounds[1]);
        let last = Self::query_chunk_coords_for_world(bounds[2], bounds[3]);
        for x in first.0..=last.0 {
            for z in first.1..=last.1 {
                // Duplicate owners across neighboring chunks are harmless; no query scratch set.
                for &edge in self
                    .query_chunk_spans
                    .get(&(x, z))
                    .into_iter()
                    .flat_map(|owners| owners.iter())
                {
                    let Some(piece) = self.compiled_visual_span_pieces.get(&edge) else {
                        continue;
                    };
                    if piece.surface_query.items_in_bounds(bounds).any(|item| {
                        kinds(piece.span_owned_regions[(item >> 1) as usize].owner().kind)
                            && triangle_near_disc(
                                piece.surface_query_triangle(item).0,
                                point,
                                radius,
                            )
                    }) {
                        return true;
                    }
                }
                for &node in self
                    .query_chunk_nodes
                    .get(&(x, z))
                    .into_iter()
                    .flat_map(|owners| owners.iter())
                {
                    if !self.node_uses_visible_surface(graph, terrain, node) {
                        continue;
                    }
                    let Some(piece) = self.compiled_visual_node_pieces.get(&node) else {
                        continue;
                    };
                    if piece.surface_query.items_in_bounds(bounds).any(|item| {
                        kinds(
                            piece.owned_regions
                                [(item >> piece.surface_query_triangle_bits) as usize]
                                .kind,
                        ) && triangle_near_disc(piece.surface_query_triangle(item).0, point, radius)
                    }) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

fn triangle_near_disc(triangle: [RoadVec3; 3], point: RoadVec2, radius: f64) -> bool {
    if !RoadSurfaceSystem::top_surface_triangle_is_renderable_xz(triangle) {
        return false;
    }
    if RoadSurfaceSystem::triangle_height_at_xz(&triangle, point).is_some() {
        return true;
    }
    let points = triangle.map(|p| RoadVec2::new(p.x, p.z));
    (0..3).any(|i| {
        let a = points[i];
        let ab = points[(i + 1) % 3] - a;
        let t = if ab.length_squared() > 0.0 {
            ((point - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        point.distance_squared(a + ab * t) <= radius * radius
    })
}
