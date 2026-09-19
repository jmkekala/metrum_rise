// SPDX-License-Identifier: GPL-2.0-only

//! Engine-owned authored-mesh LOD selection, independent of Godot and simulation state.
//! Bounds are always LOD0's local geometry bounds. No distance bands, culling or shadows
//! participate. Projection and selection have constant work and allocate no memory.

use glam::{Mat4, Vec2, Vec3};

/// Provisional calibration threshold: LOD0 gives way below this many effective pixels.
pub const FIRST_SWITCH_PIXELS: f32 = 512.0;
const FIRST_EXPONENT: i32 = (FIRST_SWITCH_PIXELS.to_bits() >> 23) as i32 - 127;
/// Relative dead band on either side of a transition; not a cross-fade.
pub const HYSTERESIS: f32 = 0.1;

/// Global presentation quality, never an asset-authored override.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LodQuality {
    /// Earlier transitions, at half the effective projected size.
    Performance,
    /// Reference transition thresholds used during authoring review.
    #[default]
    Balanced,
    /// Later transitions, at twice the effective projected size.
    Quality,
}

impl LodQuality {
    /// Scale applied to projected pixels, with larger values retaining detail longer.
    pub fn pixel_scale(self) -> f32 {
        match self {
            Self::Performance => 0.5,
            Self::Balanced => 1.0,
            Self::Quality => 2.0,
        }
    }
}

/// Nominal render-pixel boundary from tier `index` to `index + 1`, before hysteresis.
pub fn switch_pixels(index: usize, quality: LodQuality) -> f32 {
    2.0_f32.powi(FIRST_EXPONENT - index.min(i32::MAX as usize) as i32) / quality.pixel_scale()
}

/// Maximum width/height of the projected local AABB in actual render pixels.
/// `local_to_clip` includes placement scale, view transform and the camera projection.
/// Clip coordinates use Godot's camera/OpenGL convention (near plane at z = -w).
/// Near-plane intersections and invalid inputs return infinity, retaining LOD0;
/// callers handle visibility separately. Extents are deliberately not viewport-clipped.
pub fn projected_size_pixels(min: Vec3, max: Vec3, local_to_clip: Mat4, viewport: Vec2) -> f32 {
    if !min.is_finite()
        || !max.is_finite()
        || !max.cmpge(min).all()
        || !local_to_clip.is_finite()
        || !viewport.is_finite()
        || viewport.min_element() <= 0.0
    {
        return f32::INFINITY;
    }
    let mut screen_min = Vec2::splat(f32::INFINITY);
    let mut screen_max = Vec2::splat(f32::NEG_INFINITY);
    for corner in 0..8 {
        let point = Vec3::new(
            if corner & 1 == 0 { min.x } else { max.x },
            if corner & 2 == 0 { min.y } else { max.y },
            if corner & 4 == 0 { min.z } else { max.z },
        );
        let clip = local_to_clip * point.extend(1.0);
        if !clip.is_finite() || clip.w <= 0.0 || clip.z <= -clip.w {
            return f32::INFINITY;
        }
        let screen = clip.truncate().truncate() / clip.w;
        screen_min = screen_min.min(screen);
        screen_max = screen_max.max(screen);
    }
    ((screen_max - screen_min) * viewport * 0.5).max_element()
}

/// Choose an available tier using 512/256/128/... effective-pixel boundaries.
/// Equality retains the finer tier. `previous` supplies hysteresis history; omit it
/// on first evaluation or an explicit quality/mode change. Empty chains return None.
/// Missing coarse tiers retain the last available mesh, never hide the asset.
pub fn select_lod(
    pixels: f32,
    tiers: usize,
    previous: Option<usize>,
    quality: LodQuality,
) -> Option<usize> {
    let last = tiers.checked_sub(1)?;
    if pixels.is_nan() || pixels < 0.0 || pixels == f32::INFINITY {
        return Some(0);
    }
    let effective = pixels * quality.pixel_scale();
    let raw = |size: f32| -> usize {
        if size >= FIRST_SWITCH_PIXELS {
            0
        } else if size <= 0.0 {
            last
        } else {
            // Boundaries are powers of two. Extract floor(log2(size)) exactly instead
            // of rounding log2 at a boundary or overflowing a division for tiny sizes.
            let bits = size.to_bits();
            let exponent = (bits >> 23) as i32;
            let floor_log2 = if exponent == 0 {
                -149 + (31 - bits.leading_zeros()) as i32
            } else {
                exponent - 127
            };
            (FIRST_EXPONENT - floor_log2).max(0) as usize
        }
        .min(last)
    };
    let target = raw(effective);
    let Some(previous) = previous.filter(|&index| index <= last) else {
        return Some(target);
    };
    Some(if target > previous {
        raw(effective / (1.0 - HYSTERESIS)).max(previous)
    } else if target < previous {
        raw(effective / (1.0 + HYSTERESIS)).min(previous)
    } else {
        previous
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_variable_chains_and_quality() {
        assert_eq!(
            FIRST_SWITCH_PIXELS.to_bits() & 0x7fffff,
            0,
            "policy requires power-of-two boundaries"
        );
        for index in 0..10 {
            let boundary = switch_pixels(index, LodQuality::Balanced);
            assert_eq!(
                select_lod(boundary, 20, None, LodQuality::Balanced),
                Some(index)
            );
            assert_eq!(
                select_lod(
                    f32::from_bits(boundary.to_bits() - 1),
                    20,
                    None,
                    LodQuality::Balanced
                ),
                Some(index + 1)
            );
        }
        assert_eq!(
            select_lod(f32::from_bits(1), 200, None, LodQuality::Balanced),
            Some(158)
        );
        for (pixels, expected) in [
            (1024.0, 0),
            (512.0, 0),
            (511.0, 1),
            (256.0, 1),
            (255.0, 2),
            (128.0, 2),
            (127.0, 3),
            (0.0, 3),
        ] {
            assert_eq!(
                select_lod(pixels, 4, None, LodQuality::Balanced),
                Some(expected)
            );
        }
        assert_eq!(select_lod(1.0, 0, None, LodQuality::Balanced), None);
        assert_eq!(select_lod(0.0, 1, None, LodQuality::Balanced), Some(0));
        assert_eq!(select_lod(1.0, 12, None, LodQuality::Balanced), Some(9));
        assert_eq!(select_lod(300.0, 4, None, LodQuality::Performance), Some(2));
        assert_eq!(select_lod(300.0, 4, None, LodQuality::Quality), Some(0));
        for bad in [f32::NAN, f32::INFINITY, -1.0] {
            assert_eq!(select_lod(bad, 4, Some(3), LodQuality::Balanced), Some(0));
        }
    }

    #[test]
    fn hysteresis_holds_and_large_jumps_do_not_walk_tiers() {
        for pixels in [480.0, 510.0, 530.0, 560.0] {
            assert_eq!(
                select_lod(pixels, 4, Some(1), LodQuality::Balanced),
                Some(1)
            );
        }
        assert_eq!(select_lod(470.0, 4, Some(0), LodQuality::Balanced), Some(0));
        assert_eq!(select_lod(450.0, 4, Some(0), LodQuality::Balanced), Some(1));
        assert_eq!(select_lod(570.0, 4, Some(1), LodQuality::Balanced), Some(0));
        assert_eq!(select_lod(10.0, 4, Some(0), LodQuality::Balanced), Some(3));
        assert_eq!(
            select_lod(2000.0, 4, Some(3), LodQuality::Balanced),
            Some(0)
        );
        assert_eq!(select_lod(100.0, 2, Some(3), LodQuality::Balanced), Some(1));
    }

    fn measure(projection: Mat4, distance: f32, scale: f32, viewport: Vec2) -> f32 {
        let model = Mat4::from_translation(Vec3::new(0.0, 0.0, -distance))
            * Mat4::from_scale(Vec3::splat(scale));
        projected_size_pixels(
            Vec3::splat(-1.0),
            Vec3::splat(1.0),
            projection * model,
            viewport,
        )
    }

    #[test]
    fn perspective_tracks_distance_scale_fov_and_resolution() {
        let viewport = Vec2::new(1920.0, 1080.0);
        let projection = Mat4::perspective_rh_gl(60_f32.to_radians(), 16.0 / 9.0, 0.1, 5000.0);
        let base = measure(projection, 20.0, 1.0, viewport);
        assert!(base > measure(projection, 40.0, 1.0, viewport));
        assert!(base < measure(projection, 20.0, 2.0, viewport));
        assert_eq!(base * 2.0, measure(projection, 20.0, 1.0, viewport * 2.0));
        let wider = Mat4::perspective_rh_gl(90_f32.to_radians(), 16.0 / 9.0, 0.1, 5000.0);
        assert!(base > measure(wider, 20.0, 1.0, viewport));
        assert_eq!(measure(projection, 0.5, 1.0, viewport), f32::INFINITY);
        assert_eq!(measure(projection, -20.0, 1.0, viewport), f32::INFINITY);
    }

    #[test]
    fn orthographic_uses_size_not_distance_and_bounds_are_not_screen_clipped() {
        let projection = Mat4::orthographic_rh_gl(-10.0, 10.0, -10.0, 10.0, 0.1, 5000.0);
        let viewport = Vec2::splat(1000.0);
        assert!((measure(projection, 20.0, 1.0, viewport) - 100.0).abs() < 0.001);
        assert_eq!(
            measure(projection, 20.0, 1.0, viewport),
            measure(projection, 200.0, 1.0, viewport)
        );
        assert_eq!(measure(projection, 20.0, 2.0, viewport), 200.0);
        let offscreen = projection * Mat4::from_translation(Vec3::new(100.0, 0.0, -20.0));
        assert!(
            (projected_size_pixels(-Vec3::ONE, Vec3::ONE, offscreen, viewport) - 100.0).abs()
                < 0.01
        );
        assert_eq!(measure(projection, 20.0, 1.0, Vec2::ZERO), f32::INFINITY);
    }
}
