//! Authored resource overlay rendering bridge.

use crate::nodes::sim::core::SimCore;
use crate::simulation::resources::{COAL_RESOURCE_ID, RESOURCE_RICHNESS_MAX};
use godot::prelude::*;
use rayon::prelude::*;

const COAL_PIT_OVERLAY_TARGET_CELL_M: f32 = 2.0;
const COAL_PIT_EMPTY_OVERLAY_SIZE_PX: usize = 2;
const COAL_PIT_OVERLAY_MIN_SIZE_PX: usize = 16;
const COAL_PIT_OVERLAY_MAX_SIZE_PX: usize = 4096;
const COAL_PIT_EDGE_FEATHER_M: f32 = 5.0;
const COAL_PIT_MASK_SUPERSAMPLE_AXIS: usize = 2;

impl SimCore {
    /// Returns authored coal deposit richness as terrain-sized RGBA8 overlay data.
    pub fn get_world_coal_deposit_overlay_data_internal(&self) -> PackedByteArray {
        let target_w = self.heightmap.width;
        let target_h = self.heightmap.height;
        let (resource_w, resource_h) = self.resource_deposits.grid_dimensions();
        if target_w == 0 || target_h == 0 || resource_w == 0 || resource_h == 0 {
            return PackedByteArray::new();
        }

        let mut pixels = Vec::with_capacity(target_w * target_h * 4);
        for z in 0..target_h {
            let sample_z = scaled_index(z, target_h, resource_h);
            for x in 0..target_w {
                let sample_x = scaled_index(x, target_w, resource_w);
                let richness = self.resource_deposits.coal_richness_at(sample_x, sample_z);
                if richness == 0 {
                    pixels.extend_from_slice(&[0, 0, 0, 0]);
                    continue;
                }
                let normalized =
                    (f32::from(richness) / f32::from(RESOURCE_RICHNESS_MAX)).clamp(0.0, 1.0);
                let channel = (normalized * 255.0).round() as u8;
                pixels.extend_from_slice(&[channel, channel, channel, channel]);
            }
        }
        PackedByteArray::from_iter(pixels)
    }

    /// Returns committed coal extraction polygons as a high-resolution L8 pit mask.
    pub fn get_coal_pit_overlay_data_internal(&self) -> PackedByteArray {
        let Some((mask_sites, bounds)) = self.coal_pit_mask_sites_and_bounds() else {
            return PackedByteArray::new();
        };
        let (target_w, target_h) = coal_pit_overlay_dimensions(bounds.width(), bounds.height());
        if target_w == 0 || target_h == 0 {
            return PackedByteArray::new();
        }
        let texel_w_m = overlay_texel_step_m(target_w, bounds.width());
        let texel_h_m = overlay_texel_step_m(target_h, bounds.height());

        let pixels: Vec<u8> = (0..target_h)
            .into_par_iter()
            .flat_map_iter(|z| {
                let sites = &mask_sites;
                let world_z = pixel_world_coord_in_bounds(z, target_h, bounds.min_y, bounds.max_y);
                (0..target_w).map(move |x| {
                    let world_pos = Vector2::new(
                        pixel_world_coord_in_bounds(x, target_w, bounds.min_x, bounds.max_x),
                        world_z,
                    );
                    let mut alpha = 0.0_f32;
                    for site in sites {
                        if !site.contains(world_pos) {
                            continue;
                        }
                        alpha = alpha.max(pixel_soft_mask_alpha(
                            world_pos,
                            texel_w_m,
                            texel_h_m,
                            site.polygon,
                            COAL_PIT_EDGE_FEATHER_M,
                        ));
                        if alpha >= 1.0 {
                            break;
                        }
                    }
                    (alpha * 255.0).round() as u8
                })
            })
            .collect();
        PackedByteArray::from_iter(pixels)
    }

    /// Returns the pixel dimensions used by the coal pit L8 mask payload.
    pub fn get_coal_pit_overlay_size_internal(&self) -> Vector2 {
        let Some((_, bounds)) = self.coal_pit_mask_sites_and_bounds() else {
            return Vector2::new(
                COAL_PIT_EMPTY_OVERLAY_SIZE_PX as f32,
                COAL_PIT_EMPTY_OVERLAY_SIZE_PX as f32,
            );
        };
        let (target_w, target_h) = coal_pit_overlay_dimensions(bounds.width(), bounds.height());
        Vector2::new(target_w as f32, target_h as f32)
    }

    /// Returns `(min_x, min_z, width, height)` for the cropped coal pit mask in world metres.
    pub fn get_coal_pit_overlay_world_bounds_internal(&self) -> Vector4 {
        let Some((_, bounds)) = self.coal_pit_mask_sites_and_bounds() else {
            return Vector4::ZERO;
        };
        Vector4::new(bounds.min_x, bounds.min_y, bounds.width(), bounds.height())
    }

    /// Returns the current extractor visual revision for cheap renderer polling.
    pub fn get_coal_pit_overlay_revision_internal(&self) -> u64 {
        self.resource_extraction.visual_revision()
    }

    fn coal_pit_mask_sites_and_bounds(
        &self,
    ) -> Option<(Vec<CoalPitMaskSite<'_>>, CoalPitOverlayBounds)> {
        let (world_w, world_h) = self.heightmap.world_size();
        if world_w <= 0.0 || world_h <= 0.0 {
            return None;
        }
        let half_w = world_w * 0.5;
        let half_h = world_h * 0.5;
        let bounds_padding_m = COAL_PIT_EDGE_FEATHER_M + COAL_PIT_OVERLAY_TARGET_CELL_M;
        let mut mask_sites = Vec::new();
        let mut bounds: Option<CoalPitOverlayBounds> = None;
        for site in self.resource_extraction.sites() {
            if site.resource_id != COAL_RESOURCE_ID || site.polygon_world.len() < 3 {
                continue;
            }
            let Some(mask_site) =
                CoalPitMaskSite::from_polygon(&site.polygon_world, bounds_padding_m)
                    .and_then(|site| site.clipped_to_world(half_w, half_h))
            else {
                continue;
            };
            bounds = Some(match bounds {
                Some(existing) => existing.union(mask_site.bounds()),
                None => mask_site.bounds(),
            });
            mask_sites.push(mask_site);
        }
        Some((mask_sites, bounds?))
    }
}

#[derive(Clone, Copy)]
struct CoalPitMaskSite<'a> {
    polygon: &'a [Vector2],
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

impl<'a> CoalPitMaskSite<'a> {
    fn from_polygon(polygon: &'a [Vector2], padding_m: f32) -> Option<Self> {
        let mut points = polygon.iter();
        let first = *points.next()?;
        let mut min_x = first.x;
        let mut max_x = first.x;
        let mut min_y = first.y;
        let mut max_y = first.y;
        for point in points {
            min_x = min_x.min(point.x);
            max_x = max_x.max(point.x);
            min_y = min_y.min(point.y);
            max_y = max_y.max(point.y);
        }
        Some(Self {
            polygon,
            min_x: min_x - padding_m,
            max_x: max_x + padding_m,
            min_y: min_y - padding_m,
            max_y: max_y + padding_m,
        })
    }

    fn clipped_to_world(mut self, half_w: f32, half_h: f32) -> Option<Self> {
        self.min_x = self.min_x.clamp(-half_w, half_w);
        self.max_x = self.max_x.clamp(-half_w, half_w);
        self.min_y = self.min_y.clamp(-half_h, half_h);
        self.max_y = self.max_y.clamp(-half_h, half_h);
        if self.max_x <= self.min_x || self.max_y <= self.min_y {
            return None;
        }
        Some(self)
    }

    fn bounds(&self) -> CoalPitOverlayBounds {
        CoalPitOverlayBounds {
            min_x: self.min_x,
            max_x: self.max_x,
            min_y: self.min_y,
            max_y: self.max_y,
        }
    }

    fn contains(&self, point: Vector2) -> bool {
        point.x >= self.min_x
            && point.x <= self.max_x
            && point.y >= self.min_y
            && point.y <= self.max_y
    }
}

#[derive(Clone, Copy)]
struct CoalPitOverlayBounds {
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

impl CoalPitOverlayBounds {
    fn union(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            max_x: self.max_x.max(other.max_x),
            min_y: self.min_y.min(other.min_y),
            max_y: self.max_y.max(other.max_y),
        }
    }

    fn width(self) -> f32 {
        (self.max_x - self.min_x).max(0.0)
    }

    fn height(self) -> f32 {
        (self.max_y - self.min_y).max(0.0)
    }
}

fn coal_pit_overlay_dimensions(world_w: f32, world_h: f32) -> (usize, usize) {
    (
        coal_pit_overlay_axis_len(world_w),
        coal_pit_overlay_axis_len(world_h),
    )
}

fn coal_pit_overlay_axis_len(world_size_m: f32) -> usize {
    if !world_size_m.is_finite() || world_size_m <= 0.0 {
        return 0;
    }
    ((world_size_m / COAL_PIT_OVERLAY_TARGET_CELL_M).ceil() as usize + 1)
        .clamp(COAL_PIT_OVERLAY_MIN_SIZE_PX, COAL_PIT_OVERLAY_MAX_SIZE_PX)
}

fn overlay_texel_step_m(target_len: usize, world_size_m: f32) -> f32 {
    if target_len <= 1 {
        return world_size_m.max(0.0);
    }
    world_size_m / target_len.saturating_sub(1) as f32
}

fn pixel_world_coord_in_bounds(index: usize, target_len: usize, min: f32, max: f32) -> f32 {
    min + overlay_texel_step_m(target_len, max - min) * index as f32
}

fn pixel_soft_mask_alpha(
    point: Vector2,
    texel_w_m: f32,
    texel_h_m: f32,
    polygon: &[Vector2],
    feather_m: f32,
) -> f32 {
    let mut total = 0.0;
    let mut samples = 0;
    for sample_y in 0..COAL_PIT_MASK_SUPERSAMPLE_AXIS {
        let offset_y =
            supersample_offset(sample_y, COAL_PIT_MASK_SUPERSAMPLE_AXIS) * texel_h_m.max(0.0);
        for sample_x in 0..COAL_PIT_MASK_SUPERSAMPLE_AXIS {
            let offset_x =
                supersample_offset(sample_x, COAL_PIT_MASK_SUPERSAMPLE_AXIS) * texel_w_m.max(0.0);
            total += polygon_soft_mask_alpha(
                Vector2::new(point.x + offset_x, point.y + offset_y),
                polygon,
                feather_m,
            );
            samples += 1;
        }
    }
    total / samples as f32
}

fn supersample_offset(sample: usize, sample_count: usize) -> f32 {
    (sample as f32 + 0.5) / sample_count.max(1) as f32 - 0.5
}

fn polygon_soft_mask_alpha(point: Vector2, polygon: &[Vector2], feather_m: f32) -> f32 {
    if !point_in_polygon(point, polygon) {
        return 0.0;
    }
    smoothstep(
        0.0,
        feather_m.max(f32::EPSILON),
        point_distance_to_polygon_edges(point, polygon),
    )
}

fn point_distance_to_polygon_edges(point: Vector2, polygon: &[Vector2]) -> f32 {
    if polygon.is_empty() {
        return 0.0;
    }

    let mut min_distance = f32::INFINITY;
    let mut prev = polygon[polygon.len() - 1];
    for &curr in polygon {
        min_distance = min_distance.min(point_segment_distance(point, prev, curr));
        prev = curr;
    }
    min_distance
}

fn point_segment_distance(point: Vector2, a: Vector2, b: Vector2) -> f32 {
    let ab_x = b.x - a.x;
    let ab_y = b.y - a.y;
    let ap_x = point.x - a.x;
    let ap_y = point.y - a.y;
    let len_sq = ab_x * ab_x + ab_y * ab_y;
    if len_sq <= f32::EPSILON {
        let dx = point.x - a.x;
        let dy = point.y - a.y;
        return (dx * dx + dy * dy).sqrt();
    }

    let t = ((ap_x * ab_x + ap_y * ab_y) / len_sq).clamp(0.0, 1.0);
    let closest_x = a.x + ab_x * t;
    let closest_y = a.y + ab_y * t;
    let dx = point.x - closest_x;
    let dy = point.y - closest_y;
    (dx * dx + dy * dy).sqrt()
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn scaled_index(index: usize, target_len: usize, source_len: usize) -> usize {
    if target_len <= 1 || source_len <= 1 {
        return 0;
    }
    (index.saturating_mul(source_len) / target_len).min(source_len - 1)
}

fn point_in_polygon(point: Vector2, polygon: &[Vector2]) -> bool {
    let mut inside = false;
    let mut prev = polygon[polygon.len() - 1];
    for &curr in polygon {
        let crosses = (curr.y > point.y) != (prev.y > point.y);
        if crosses {
            let denom = prev.y - curr.y;
            if denom.abs() > f32::EPSILON {
                let x_at_y = (prev.x - curr.x) * (point.y - curr.y) / denom + curr.x;
                if point.x < x_at_y {
                    inside = !inside;
                }
            }
        }
        prev = curr;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coal_pit_soft_mask_does_not_bleed_outside_polygon() {
        let polygon = [
            Vector2::new(-10.0, -10.0),
            Vector2::new(10.0, -10.0),
            Vector2::new(10.0, 10.0),
            Vector2::new(-10.0, 10.0),
        ];

        assert_eq!(
            polygon_soft_mask_alpha(Vector2::new(10.5, 0.0), &polygon, 4.0),
            0.0
        );
        assert_eq!(
            polygon_soft_mask_alpha(Vector2::new(11.0, 11.0), &polygon, 4.0),
            0.0
        );
        assert!(
            polygon_soft_mask_alpha(Vector2::new(8.0, 0.0), &polygon, 4.0) > 0.0,
            "inside edge feather should still soften the boundary"
        );
        assert_eq!(
            polygon_soft_mask_alpha(Vector2::new(0.0, 0.0), &polygon, 4.0),
            1.0
        );
    }
}
