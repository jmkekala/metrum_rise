// SPDX-License-Identifier: GPL-2.0-only

//! Reversible ground coverage for committed cutouts vacated by an editor preview.

use super::super::{RoadSurfaceSystem, RoadSurfaceTerrainClipLoop, backend::RoadVec3};
use crate::config::HEIGHT_SCALE;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use i_overlay::core::{fill_rule::FillRule, overlay_rule::OverlayRule};
use rayon::prelude::*;

/// Local cutout boundaries and the ground exposed by a preview footprint change.
pub(crate) struct RoadPreviewGround {
    /// Temporary coverage for old cutouts outside the planned footprint, in world coordinates.
    pub(crate) triangles: Vec<[RoadVec3; 3]>,
    /// Original owner-loop segments, including collinear terrain seam breakpoints.
    pub(crate) boundaries: Vec<[RoadVec3; 2]>,
}

impl RoadSurfaceSystem {
    /// Covers only old cutouts no longer occupied by the planned road, without editing terrain.
    /// Reuses the compiler's deterministic overlay/CDT backend on the replaced local owners.
    pub(crate) fn preview_vacated_ground(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        replaced_edges: impl Iterator<Item = usize>,
        replaced_nodes: impl Iterator<Item = u32>,
        planned_graph: &RegionGraph,
        planned: &Self,
    ) -> Option<RoadPreviewGround> {
        let mut previous = Vec::new();
        for edge in replaced_edges {
            if let Some(piece) = self.compiled_visual_span_pieces.get(&edge) {
                previous.extend(piece.terrain_clip_boundary_loops.iter());
            }
        }
        for node in replaced_nodes {
            if self.node_has_terrain_clip_surface_edges(graph, node)
                && let Some(piece) = self.compiled_visual_node_pieces.get(&node)
            {
                previous.extend(piece.terrain_clip_boundary_loops.iter());
            }
        }
        // With no old cutout there is nothing to uncover, regardless of the planned footprint.
        // First/isolated strokes must not union their entire new outline just to subtract it
        // from an empty set. They also have no old terrain contacts to preserve during lifting.
        if previous.is_empty() {
            return Some(RoadPreviewGround {
                triangles: Vec::new(),
                boundaries: Vec::new(),
            });
        }
        let mut next: Vec<&RoadSurfaceTerrainClipLoop> = planned
            .compiled_visual_span_pieces
            .values()
            .flat_map(|piece| piece.terrain_clip_boundary_loops.iter())
            .chain(
                planned
                    .compiled_visual_node_pieces
                    .values()
                    .flat_map(|piece| piece.terrain_clip_boundary_loops.iter()),
            )
            .collect();
        // Hash iteration order must not influence boolean input or triangulation ordering.
        let order = |a: &&RoadSurfaceTerrainClipLoop, b: &&RoadSurfaceTerrainClipLoop| {
            let key = |point: &RoadVec3| [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()];
            a.points_world
                .iter()
                .map(key)
                .cmp(b.points_world.iter().map(key))
        };
        previous.sort_by(order);
        next.sort_by(order);
        let old_shapes = Self::overlay_union_contours_with_fill_rule(
            &Self::overlay_contours_from_terrain_clip_boundary_loops(&previous),
            FillRule::NonZero,
        )?;
        let new_shapes = Self::overlay_union_contours_with_fill_rule(
            &Self::overlay_contours_from_terrain_clip_boundary_loops(&next),
            FillRule::NonZero,
        )?;
        let mut vacant =
            Self::overlay_binary_shapes(&old_shapes, &new_shapes, OverlayRule::Difference)?;
        Self::sort_overlay_shapes(&mut vacant);
        let triangles = vacant
            .par_iter()
            .map(|shape| {
                let rings: Vec<Vec<RoadVec3>> = shape
                    .iter()
                    .map(|contour| {
                        contour
                            .iter()
                            .map(|point| {
                                let x = point[0] as f32;
                                let z = point[1] as f32;
                                // The inner boundary joins the new road; the outer boundary
                                // joins committed terrain. Old curb heights must not leak onto
                                // the newly shaped bend's material boundary.
                                let height = planned
                                    .sample_visible_surface_height(planned_graph, terrain, x, z)
                                    .or_else(|| {
                                        self.sample_visible_surface_height(graph, terrain, x, z)
                                    })
                                    .unwrap_or_else(|| {
                                        terrain.sample_visual_height_world(x, z) * HEIGHT_SCALE
                                    });
                                RoadVec3::new(point[0], f64::from(height), point[1])
                            })
                            .collect()
                    })
                    .collect();
                Self::triangulate_constrained_rings_xz(&rings)
            })
            .collect::<Option<Vec<_>>>()
            .map(|pieces| pieces.into_iter().flatten().collect())?;
        let boundaries = previous
            .iter()
            .flat_map(|boundary| {
                let points = &boundary.points_world;
                (0..points.len()).map(|i| [points[i], points[(i + 1) % points.len()]])
            })
            .collect();
        Some(RoadPreviewGround {
            triangles,
            boundaries,
        })
    }
}
