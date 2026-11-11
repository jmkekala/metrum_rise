// SPDX-License-Identifier: GPL-2.0-only

//! The view a streaming layer measures its chunks from (`RENDER-32`): the camera and how far it
//! has travelled. For the graded tiles the camera is the renderer's (`terrain.gd`): its XZ with,
//! as `y`, its height above the visible ground under it (`docs/terrain.md` "LOD and distant
//! tiles"), so a camera high over a tile takes the level of its distance, not of its offset
//! along the ground. Values are Godot's `f32` position widened to `f64`, as GDScript measured
//! them, so a distance here equals the one the GDScript renderer computed bit for bit.

use crate::simulation::terrain::graded::{TILE_M, TileKey};

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

#[cfg(test)]
mod tests {
    use super::*;

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
