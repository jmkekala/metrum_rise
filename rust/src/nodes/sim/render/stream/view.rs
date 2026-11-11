// SPDX-License-Identifier: GPL-2.0-only

//! The view a streaming layer measures its chunks from (`RENDER-32`): the camera and how far it
//! has travelled, and the agents' ground rectangle from the camera's corner rays. For the graded tiles the camera is the renderer's (`terrain.gd`): its XZ with,
//! as `y`, its height above the visible ground under it (`docs/terrain.md` "LOD and distant
//! tiles"), so a camera high over a tile takes the level of its distance, not of its offset
//! along the ground. Values are Godot's `f32` position widened to `f64`, as GDScript measured
//! them, so a distance here equals the one the GDScript renderer computed bit for bit.

use crate::simulation::terrain::graded::{TILE_M, TileKey};
use godot::prelude::Vector3;

/// A camera and the distance it travelled since it was first set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct StreamView {
    camera: [f64; 3],
    set: bool,
    // The sum of the straight moves between the cameras set, metres.
    travel: f64,
}

impl StreamView {
    /// The metres travelled since the first camera.
    pub fn travel(&self) -> f64 {
        self.travel
    }

    /// Moves to `camera`. Returns the straight distance moved, `None` for the first camera and
    /// for an unchanged one. `O(1)`.
    pub fn move_to(&mut self, camera: [f64; 3]) -> Option<f64> {
        if !self.set {
            self.camera = camera;
            self.set = true;
            return None;
        }
        let [dx, dy, dz] = [0, 1, 2].map(|axis| camera[axis] - self.camera[axis]);
        let moved = (dx * dx + dy * dy + dz * dz).sqrt();
        if moved == 0.0 {
            return None;
        }
        self.camera = camera;
        self.travel += moved;
        Some(moved)
    }

    /// The camera's distance to the centre of `tile` on the ground under the camera, in the
    /// GDScript renderer's operation order (`_tile_distance`).
    pub fn tile_distance(&self, tile: TileKey) -> f64 {
        let [cx, height, cz] = self.camera;
        let dx = (f64::from(tile.x) + 0.5) * TILE_M - cx;
        let dz = (f64::from(tile.z) + 0.5) * TILE_M - cz;
        (dx * dx + height * height + dz * dz).sqrt()
    }

    /// The squared ground distance from the camera to the centre of `tile`: nearest first by it
    /// is nearest first by [`Self::tile_distance`].
    pub fn tile_ground_distance_sq(&self, tile: TileKey) -> f64 {
        let dx = (f64::from(tile.x) + 0.5) * TILE_M - self.camera[0];
        let dz = (f64::from(tile.z) + 0.5) * TILE_M - self.camera[2];
        dx * dx + dz * dz
    }

    /// The squared ground distance from the camera to the nearest point of the rectangle
    /// `[min_x, min_z, max_x, max_z]` (0 inside it): a lower bound of
    /// [`Self::tile_ground_distance_sq`] for every tile centre in it.
    pub fn rect_ground_distance_sq(&self, rect: [f64; 4]) -> f64 {
        let [cx, _, cz] = self.camera;
        let dx = (rect[0] - cx).max(cx - rect[2]).max(0.0);
        let dz = (rect[1] - cz).max(cz - rect[3]).max(0.0);
        dx * dx + dz * dz
    }
}

/// The ground rectangle `[x_min, x_max, z_min, z_max]` the agents are culled to (`RENDER-28`):
/// each of the viewport's four corner rays `(origin, direction)` meets the ground plane `y = 0`
/// where it comes down onto it, at most `far` along the ray, and the rectangle around the four
/// points is padded by `pad`. In the operation order of the GDScript it replaced
/// (`agents.gd` `_update_camera_aabb`: `f64` distances and bounds, `f32` points), so the
/// rectangle is the one it computed bit for bit. O(1).
pub(crate) fn agent_ground_rect(rays: [(Vector3, Vector3); 4], far: f32, pad: f32) -> [f32; 4] {
    let mut bounds = [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for (origin, direction) in rays {
        // A ray that does not come down steeply enough would meet the ground beyond any sane
        // distance, or never.
        let distance = if f64::from(direction.y) < -1e-3 {
            (-f64::from(origin.y) / f64::from(direction.y)).min(f64::from(far))
        } else {
            f64::from(far)
        };
        let point = origin + direction * distance as f32;
        bounds[0] = bounds[0].min(f64::from(point.x));
        bounds[1] = bounds[1].max(f64::from(point.x));
        bounds[2] = bounds[2].min(f64::from(point.z));
        bounds[3] = bounds[3].max(f64::from(point.z));
    }
    let pad = f64::from(pad);
    [
        (bounds[0] - pad) as f32,
        (bounds[1] + pad) as f32,
        (bounds[2] - pad) as f32,
        (bounds[3] + pad) as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_rectangle_meets_the_ground_or_stops_at_the_far_distance() {
        let eye = Vector3::new(100.0, 50.0, -20.0);
        let down = |x: f32, z: f32| Vector3::new(x, -1.0, z).normalized();
        // Two corners come down 50 m below the eye; two look over the horizon and stop at 4 km.
        let rays = [
            (eye, down(-1.0, -1.0)),
            (eye, down(1.0, -1.0)),
            (eye, Vector3::new(0.6, 0.0, 0.8)),
            (eye, Vector3::new(-0.6, 0.0005, 0.8)),
        ];
        let [x_min, x_max, z_min, z_max] = agent_ground_rect(rays, 4000.0, 200.0);
        assert!((x_min - (100.0 - 2400.0 - 200.0)).abs() < 1e-2, "{x_min}");
        assert!((x_max - (100.0 + 2400.0 + 200.0)).abs() < 1e-2, "{x_max}");
        assert!((z_min - (-20.0 - 50.0 - 200.0)).abs() < 1e-3, "{z_min}");
        assert!((z_max - (-20.0 + 3200.0 + 200.0)).abs() < 1e-2, "{z_max}");
        // The GDScript's order: the f64 distance rounded once to f32, then the f32 point.
        let z = rays[..2].iter().map(|&(origin, direction)| {
            let distance = -f64::from(origin.y) / f64::from(direction.y);
            f64::from((origin + direction * distance as f32).z)
        });
        assert_eq!(z_min, (z.fold(f64::INFINITY, f64::min) - 200.0) as f32);
    }

    #[test]
    fn travel_sums_the_moves_and_skips_the_first_and_still_cameras() {
        let mut view = StreamView::default();
        assert_eq!(view.move_to([10.0, 40.0, 10.0]), None);
        assert_eq!(view.travel(), 0.0);
        assert_eq!(view.move_to([13.0, 40.0, 14.0]), Some(5.0));
        assert_eq!(view.move_to([13.0, 40.0, 14.0]), None);
        assert_eq!(view.move_to([13.0, 52.0, 14.0]), Some(12.0));
        assert_eq!(view.travel(), 17.0);
    }

    #[test]
    fn the_rectangle_bound_never_exceeds_a_tile_centre_inside_it() {
        let mut view = StreamView::default();
        view.move_to([700.3, 55.0, -129.9]);
        for x in -12..12 {
            for z in -12..12 {
                let tile = TileKey { x, z };
                let [ox, oz] = tile.origin();
                let rect = [ox, oz, ox + TILE_M, oz + TILE_M];
                assert!(view.rect_ground_distance_sq(rect) <= view.tile_ground_distance_sq(tile));
            }
        }
    }
}
