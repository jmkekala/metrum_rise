// SPDX-License-Identifier: GPL-2.0-only

//! Stateless presentation bridge to the shared, engine-independent authored LOD policy.

use crate::assets::lod_policy::{self, LodQuality};
use godot::prelude::*;

/// Shared authored-LOD projection and selection for editor and runtime render callers.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct AssetLodPolicy;

#[godot_api]
impl AssetLodPolicy {
    /// Project LOD0 bounds with the actual local-to-clip matrix and render-pixel dimensions.
    #[func]
    pub fn projected_size_pixels(
        &self,
        bounds: Aabb,
        local_to_clip: Projection,
        viewport: Vector2,
    ) -> f32 {
        let columns = local_to_clip.cols.map(|v| [v.x, v.y, v.z, v.w]);
        let min = bounds.position;
        let max = bounds.end();
        lod_policy::projected_size_pixels(
            glam::Vec3::new(min.x, min.y, min.z),
            glam::Vec3::new(max.x, max.y, max.z),
            glam::Mat4::from_cols_array_2d(&columns),
            glam::Vec2::new(viewport.x, viewport.y),
        )
    }

    /// Select a tier; previous = -1 resets history. Empty chains return -1.
    /// Quality IDs are 0 performance, 1 balanced, 2 quality; unknown IDs use balanced.
    #[func]
    pub fn select_lod(&self, pixels: f32, tiers: i32, previous: i32, quality: i32) -> i32 {
        lod_policy::select_lod(
            pixels,
            tiers.max(0) as usize,
            usize::try_from(previous).ok(),
            quality_from_id(quality),
        )
        .map_or(-1, |index| index as i32)
    }

    /// Nominal boundary in actual pixels between tiers index and index + 1.
    /// This is inspection metadata; callers must use select_lod for hysteresis.
    #[func]
    pub fn switch_pixels(&self, index: i32, quality: i32) -> f32 {
        lod_policy::switch_pixels(index.max(0) as usize, quality_from_id(quality))
    }
}

/// Shared player/preview quality IDs; unknown values use Balanced.
pub(crate) fn quality_from_id(id: i32) -> LodQuality {
    match id {
        0 => LodQuality::Performance,
        2 => LodQuality::Quality,
        _ => LodQuality::Balanced,
    }
}
