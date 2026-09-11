// SPDX-License-Identifier: GPL-2.0-only

//! Exact field reservations for placement, indexed with the existing 512 m world chunk layout.
//!
//! The allocator holds this derived cache so every building placement path sees committed fields.
//! Agriculture owns the saved polygons and updates the cache on commit, load, removal and undo.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::{RoadSurfaceSystem, RoadSurfaceVisualPolygon};
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::{fill_rule::FillRule, overlay_rule::OverlayRule};
use i_overlay::float::single::SingleFloatOverlay;
use std::collections::{HashMap, HashSet};

/// Prepared polygon used for exact positive-area overlap, including concave shapes and containment.
#[derive(Clone, Debug)]
pub(crate) struct PolygonFootprint {
    points: Vec<[f64; 2]>,
    /// Minimum world XZ bounds.
    pub(crate) min: Vector2,
    /// Maximum world XZ bounds.
    pub(crate) max: Vector2,
}

impl PolygonFootprint {
    /// Prepares a validated world polygon once per placement query or committed edit.
    pub(crate) fn new(points: &[Vector2]) -> Self {
        Self::from_points(points.iter().copied())
    }

    fn from_points(points: impl Iterator<Item = Vector2>) -> Self {
        let mut min = Vector2::new(f32::INFINITY, f32::INFINITY);
        let mut max = Vector2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        let points = points
            .map(|p| {
                min = Vector2::new(min.x.min(p.x), min.y.min(p.y));
                max = Vector2::new(max.x.max(p.x), max.y.max(p.y));
                [f64::from(p.x), f64::from(p.y)]
            })
            .collect();
        Self { points, min, max }
    }

    /// Tests occupied interiors; shared edges and vertices are permitted.
    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        self.overlaps_points(&other.points, other.min, other.max)
    }

    fn overlaps_points(&self, points: &[[f64; 2]], min: Vector2, max: Vector2) -> bool {
        self.min.x < max.x
            && self.max.x > min.x
            && self.min.y < max.y
            && self.max.y > min.y
            && !self
                .points
                .as_slice()
                .overlay(&points, OverlayRule::Intersect, FillRule::EvenOdd)
                .is_empty()
    }

    fn from_road(polygon: &RoadSurfaceVisualPolygon) -> Self {
        Self::from_points(
            polygon
                .points_world
                .iter()
                .map(|p| Vector2::new(p.x as f32, p.z as f32)),
        )
    }

    /// Tests compiled carriageway, sidewalk and junction surfaces through their existing query grid.
    pub(crate) fn overlaps_roads(&self, roads: &RoadSurfaceSystem) -> bool {
        let (_, cell_max) = RoadSurfaceSystem::query_chunk_world_bounds((0, 0));
        let cell_m = cell_max.x as f32;
        let min = (self.min / cell_m).floor();
        let max = (self.max / cell_m).floor();
        let mut tested_spans = HashSet::new();
        let mut tested_nodes = HashSet::new();
        for x in min.x as i32..=max.x as i32 {
            for z in min.y as i32..=max.y as i32 {
                if let Some(ids) = roads.query_chunk_spans.get(&(x, z)) {
                    for id in ids {
                        if !tested_spans.insert(*id) {
                            continue;
                        }
                        if let Some(piece) = roads.compiled_visual_span_pieces.get(id)
                            && piece
                                .road_surface_polygons
                                .iter()
                                .chain(&piece.curb_surface_polygons)
                                .chain(&piece.sidewalk_surface_polygons)
                                .any(|polygon| self.overlaps(&Self::from_road(polygon)))
                        {
                            return true;
                        }
                    }
                }
                if let Some(ids) = roads.query_chunk_nodes.get(&(x, z)) {
                    for id in ids {
                        if !tested_nodes.insert(*id) {
                            continue;
                        }
                        if let Some(piece) = roads.compiled_visual_node_pieces.get(id)
                            && piece
                                .road_surface_polygons
                                .iter()
                                .chain(&piece.curb_surface_polygons)
                                .chain(&piece.sidewalk_surface_polygons)
                                .any(|polygon| self.overlaps(&Self::from_road(polygon)))
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

/// Derived field footprints covering their full area, without inflating the building-center index.
#[derive(Clone, Debug, Default)]
pub(crate) struct FieldClearanceIndex {
    footprints: HashMap<usize, PolygonFootprint>,
    chunks: HashMap<(i32, i32), Vec<usize>>,
}

fn chunks(min: Vector2, max: Vector2) -> impl Iterator<Item = (i32, i32)> {
    let lower = (min / RegionGraph::CHUNK_SIZE).floor();
    let upper = (max / RegionGraph::CHUNK_SIZE).floor();
    (lower.x as i32..=upper.x as i32)
        .flat_map(move |x| (lower.y as i32..=upper.y as i32).map(move |z| (x, z)))
}

impl FieldClearanceIndex {
    /// Returns whether any committed field currently reserves land.
    pub(crate) fn is_empty(&self) -> bool {
        self.footprints.is_empty()
    }

    /// Replaces one owner's reservation, touching only the old and new field's chunks.
    pub(crate) fn set(&mut self, building_idx: usize, points: &[Vector2]) {
        self.remove(building_idx);
        if points.len() < 3 {
            return;
        }
        self.insert(building_idx, PolygonFootprint::new(points));
    }

    fn insert(&mut self, building_idx: usize, footprint: PolygonFootprint) {
        for key in chunks(footprint.min, footprint.max) {
            let bucket = self.chunks.entry(key).or_default();
            let position = bucket.partition_point(|&id| id < building_idx);
            bucket.insert(position, building_idx);
        }
        self.footprints.insert(building_idx, footprint);
    }

    fn remove(&mut self, building_idx: usize) -> Option<PolygonFootprint> {
        let footprint = self.footprints.remove(&building_idx)?;
        for key in chunks(footprint.min, footprint.max) {
            if let Some(bucket) = self.chunks.get_mut(&key) {
                bucket.retain(|&id| id != building_idx);
                if bucket.is_empty() {
                    self.chunks.remove(&key);
                }
            }
        }
        Some(footprint)
    }

    /// Removes a demolished field and rebinds a swap-moved owner's indexed footprint.
    pub(crate) fn remove_and_remap(&mut self, removed: usize, last: usize) {
        self.remove(removed);
        if removed != last
            && let Some(footprint) = self.remove(last)
        {
            self.insert(removed, footprint);
        }
    }

    /// Drops derived reservations before rebuilding them from saved field sites.
    pub(crate) fn clear(&mut self) {
        self.footprints.clear();
        self.chunks.clear();
    }

    /// Tests a placement polygon against committed fields without work on field-free maps.
    pub(crate) fn overlaps_polygon(&self, points: &[Vector2]) -> bool {
        if self.footprints.is_empty() {
            return false;
        }
        // Building/zoning candidates are rectangles: keep their broad-phase query on the stack.
        if points.len() == 4 {
            let vertices: [[f64; 2]; 4] =
                std::array::from_fn(|i| [f64::from(points[i].x), f64::from(points[i].y)]);
            let min = points
                .iter()
                .fold(points[0], |a, b| Vector2::new(a.x.min(b.x), a.y.min(b.y)));
            let max = points
                .iter()
                .fold(points[0], |a, b| Vector2::new(a.x.max(b.x), a.y.max(b.y)));
            self.overlaps_points(&vertices, min, max, None)
        } else {
            self.overlaps(&PolygonFootprint::new(points), None)
        }
    }

    /// Tests one compiled road polygon using its actual carrier geometry.
    pub(crate) fn overlaps_road_polygon(&self, polygon: &RoadSurfaceVisualPolygon) -> bool {
        !self.footprints.is_empty() && self.overlaps(&PolygonFootprint::from_road(polygon), None)
    }

    /// Queries only field references in touched chunks; `ignore` permits replacing one's own field.
    /// Cost follows touched chunks and local polygon geometry, independent of remote field count.
    pub(crate) fn overlaps(&self, footprint: &PolygonFootprint, ignore: Option<usize>) -> bool {
        self.overlaps_points(&footprint.points, footprint.min, footprint.max, ignore)
    }

    fn overlaps_points(
        &self,
        points: &[[f64; 2]],
        min: Vector2,
        max: Vector2,
        ignore: Option<usize>,
    ) -> bool {
        if self.footprints.is_empty() {
            return false;
        }
        chunks(min, max).any(|key| {
            self.chunks.get(&key).is_some_and(|bucket| {
                bucket.iter().any(|id| {
                    Some(*id) != ignore
                        && self.footprints.get(id).is_some_and(|field| {
                            // Test a multi-chunk field only in its first shared chunk, without a scratch set.
                            let first = chunks(
                                Vector2::new(min.x.max(field.min.x), min.y.max(field.min.y)),
                                Vector2::new(max.x.min(field.max.x), max.y.min(field.max.y)),
                            )
                            .next();
                            first == Some(key) && field.overlaps_points(points, min, max)
                        })
                })
            })
        })
    }

    /// Tests the full width of every candidate road segment against existing fields.
    pub(crate) fn overlaps_road_corridor(&self, points: &[Vector3], half_width_m: f32) -> bool {
        if self.footprints.is_empty() {
            return false;
        }
        points.windows(2).any(|pair| {
            let start = Vector2::new(pair[0].x, pair[0].z);
            let end = Vector2::new(pair[1].x, pair[1].z);
            let delta = end - start;
            if delta.length_squared() <= f32::EPSILON {
                return false;
            }
            let normal = Vector2::new(delta.y, -delta.x).normalized() * half_width_m;
            self.overlaps_polygon(&[start - normal, end - normal, end + normal, start + normal])
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(x: f32, z: f32, width: f32, depth: f32) -> [Vector2; 4] {
        [
            Vector2::new(x, z),
            Vector2::new(x + width, z),
            Vector2::new(x + width, z + depth),
            Vector2::new(x, z + depth),
        ]
    }

    #[test]
    fn overlap_handles_containment_concavity_and_boundary_contact() {
        let field = PolygonFootprint::new(&[
            Vector2::new(0.0, 0.0),
            Vector2::new(30.0, 0.0),
            Vector2::new(30.0, 10.0),
            Vector2::new(10.0, 10.0),
            Vector2::new(10.0, 30.0),
            Vector2::new(0.0, 30.0),
        ]);
        assert!(field.overlaps(&field));
        assert!(field.overlaps(&PolygonFootprint::new(&rectangle(1.0, 1.0, 2.0, 2.0))));
        assert!(field.overlaps(&PolygonFootprint::new(&rectangle(-1.0, -1.0, 40.0, 40.0))));
        assert!(!field.overlaps(&PolygonFootprint::new(&rectangle(15.0, 15.0, 5.0, 5.0))));
        assert!(!field.overlaps(&PolygonFootprint::new(&rectangle(30.0, 0.0, 5.0, 5.0))));
        assert!(field.overlaps(&PolygonFootprint::new(&rectangle(29.0, 0.0, 5.0, 5.0))));
    }

    #[test]
    fn reservations_cover_far_corners_and_follow_replacement_and_swap_remove() {
        let mut index = FieldClearanceIndex::default();
        index.set(5, &rectangle(-1000.0, -1000.0, 2000.0, 2000.0));
        let far = PolygonFootprint::new(&rectangle(800.0, 800.0, 10.0, 10.0));
        assert!(index.overlaps(&far, None));
        assert!(!index.overlaps(&far, Some(5)));
        index.set(5, &rectangle(-50.0, -50.0, 100.0, 100.0));
        assert!(!index.overlaps(&far, None));
        let local = PolygonFootprint::new(&rectangle(0.0, 0.0, 5.0, 5.0));
        index.remove_and_remap(2, 5);
        assert!(index.overlaps(&local, None));
        assert!(!index.overlaps(&local, Some(2)));
        index.remove_and_remap(2, 2);
        assert!(!index.overlaps(&local, None));
    }
}
