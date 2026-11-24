// SPDX-License-Identifier: GPL-3.0-or-later

//! The convex hull of the resident water patches' bounds on the ground plane, kept by their
//! residency events, for the planar water reflection's screen rectangle (`RENDER-76`).
//!
//! The reflection crops its mirror to the screen rectangle the resident water patches cover once
//! laid on the mirror plane (`water_reflection.gd`). The patches' corners project through a
//! perspective that is a projective map on the half-plane in front of the camera, and a projective
//! map's screen coordinates take their extremes over a convex region at its vertices. So whenever
//! the camera sees the whole of the hull, the hull's boundary points give that rectangle exactly,
//! and the script projects only those instead of every resident patch. Where the near plane cuts
//! the hull, the script checks what the cut can add and walks the patches only when the hull
//! cannot settle a side (see `_water_screen_rect`).
//!
//! The book holds each patch's rectangle and the multiset of their corners; an event updates both
//! in `O(log corners)` and marks the hull stale only when the corner it adds or takes is not
//! strictly inside the current hull (`O(hull)` to test). A stale hull is rebuilt on the next read
//! by a monotone chain over the corners, already sorted: `O(corners)`.

use godot::prelude::*;
use std::collections::{BTreeMap, HashMap};

// A corner's position, ordered by x then z by the bit patterns' total order.
type CornerKey = (i32, i32);

/// Per resident water patch, its bounds on the ground plane, and the hull of their corners.
#[derive(Default)]
pub(crate) struct ScreenHullBook {
    // Patch key -> [min x, min z, max x, max z].
    rects: HashMap<(i32, i32), [f32; 4]>,
    // Every distinct corner, with the number of patches that have it.
    corners: BTreeMap<CornerKey, ([f32; 2], u32)>,
    // The hull's boundary, counter-clockwise in (x, z), collinear boundary corners included: a
    // point on a hull edge ties with the edge's ends for some views, and the rectangle must take
    // the same float the patch walk takes.
    hull: Vec<[f32; 2]>,
    stale: bool,
    revision: i64,
}

// The total order of f32 bit patterns as an i32 (negative numbers reversed).
fn order_bits(value: f32) -> i32 {
    let bits = value.to_bits() as i32;
    bits ^ (((bits >> 31) as u32) >> 1) as i32
}

fn corner_key(point: [f32; 2]) -> CornerKey {
    (order_bits(point[0]), order_bits(point[1]))
}

// Twice the signed area of (a, b, c): positive for a left (counter-clockwise) turn. In f64 the
// differences and products of f32 inputs are exact, and the rounded difference keeps the sign.
fn cross(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f64 {
    let (ax, az) = (a[0] as f64, a[1] as f64);
    ((b[0] as f64 - ax) * (c[1] as f64 - az)) - ((b[1] as f64 - az) * (c[0] as f64 - ax))
}

fn rect_corners(rect: [f32; 4]) -> [[f32; 2]; 4] {
    [
        [rect[0], rect[1]],
        [rect[2], rect[1]],
        [rect[2], rect[3]],
        [rect[0], rect[3]],
    ]
}

impl ScreenHullBook {
    /// Sets `key`'s bounds (`[min x, min z, max x, max z]`), adding the patch if new.
    pub(crate) fn set(&mut self, key: (i32, i32), rect: [f32; 4]) {
        if self.rects.get(&key) == Some(&rect) {
            return;
        }
        self.remove(key);
        self.rects.insert(key, rect);
        for corner in rect_corners(rect) {
            let entry = self.corners.entry(corner_key(corner)).or_insert((corner, 0));
            entry.1 += 1;
            if entry.1 == 1 {
                self.touch(corner);
            }
        }
    }

    /// Removes `key`'s bounds; nothing when it holds none.
    pub(crate) fn remove(&mut self, key: (i32, i32)) {
        let Some(rect) = self.rects.remove(&key) else {
            return;
        };
        for corner in rect_corners(rect) {
            let ordered = corner_key(corner);
            let Some(entry) = self.corners.get_mut(&ordered) else {
                continue;
            };
            entry.1 -= 1;
            if entry.1 == 0 {
                self.corners.remove(&ordered);
                self.touch(corner);
            }
        }
    }

    /// Forgets every patch.
    pub(crate) fn clear(&mut self) {
        self.rects.clear();
        self.corners.clear();
        self.hull.clear();
        self.stale = false;
        self.revision += 1;
    }

    /// Changes whenever the hull may have changed.
    pub(crate) fn revision(&self) -> i64 {
        self.revision
    }

    /// The patches held.
    pub(crate) fn len(&self) -> usize {
        self.rects.len()
    }

    /// The hull's boundary corners, counter-clockwise, collinear ones included; every corner when
    /// they are all on one line. Rebuilt here when an event made it stale.
    pub(crate) fn hull(&mut self) -> &[[f32; 2]] {
        if self.stale {
            self.rebuild();
        }
        &self.hull
    }

    // A corner came or went. Strictly inside the hull it changes nothing; otherwise the hull is
    // rebuilt on its next read.
    fn touch(&mut self, corner: [f32; 2]) {
        if self.stale || !self.strictly_inside(corner) {
            self.stale = true;
            self.revision += 1;
        }
    }

    fn strictly_inside(&self, point: [f32; 2]) -> bool {
        let count = self.hull.len();
        count >= 3
            && (0..count)
                .all(|index| cross(self.hull[index], self.hull[(index + 1) % count], point) > 0.0)
    }

    // Andrew's monotone chain over the corners in (x, z) order, popping only on a right turn so
    // that collinear boundary corners stay.
    fn rebuild(&mut self) {
        self.stale = false;
        self.hull.clear();
        let points = || self.corners.values().map(|(point, _)| *point);
        let mut lower: Vec<[f32; 2]> = Vec::with_capacity(self.hull.capacity());
        for point in points() {
            while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], point) < 0.0 {
                lower.pop();
            }
            lower.push(point);
        }
        // All on one line (or fewer than three corners): the chain holds every corner, in order.
        if lower.len() == self.corners.len()
            && lower.windows(3).all(|w| cross(w[0], w[1], w[2]) == 0.0)
        {
            self.hull = lower;
            return;
        }
        let mut upper: Vec<[f32; 2]> = Vec::new();
        for point in points().rev() {
            while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], point) < 0.0 {
                upper.pop();
            }
            upper.push(point);
        }
        lower.pop();
        upper.pop();
        lower.extend(upper);
        self.hull = lower;
    }
}

/// The water reflection's hull of the resident water patches' bounds (`RENDER-76`), fed by the
/// water renderer's residency and mesh events; see the module notes.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct WaterScreenHull {
    book: ScreenHullBook,
}

#[godot_api]
impl WaterScreenHull {
    /// Sets the bounds on the ground plane (x, z) of resident patch `key`, adding it if new.
    /// `O(log corners)`, plus `O(hull)` when a new corner is not already a corner.
    #[func]
    pub fn set_patch(&mut self, key: Vector2i, low: Vector2, high: Vector2) {
        self.book.set((key.x, key.y), [low.x, low.y, high.x, high.y]);
    }

    /// Removes patch `key` (no longer resident, hidden or freed); nothing when not held.
    #[func]
    pub fn remove_patch(&mut self, key: Vector2i) {
        self.book.remove((key.x, key.y));
    }

    /// Forgets every patch.
    #[func]
    pub fn clear(&mut self) {
        self.book.clear();
    }

    /// Changes whenever the hull may have changed; a caller keeps the last `boundary` until then.
    #[func]
    pub fn revision(&self) -> i64 {
        self.book.revision()
    }

    /// The patches held.
    #[func]
    pub fn patch_count(&self) -> i64 {
        self.book.len() as i64
    }

    /// The hull's boundary corners (x, z), counter-clockwise, collinear ones included.
    #[func]
    pub fn boundary(&mut self) -> PackedVector2Array {
        self.book.hull().iter().map(|point| Vector2::new(point[0], point[1])).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(points: &[[f32; 2]]) -> Vec<(i32, i32)> {
        let mut keys: Vec<_> = points.iter().map(|point| corner_key(*point)).collect();
        keys.sort();
        keys
    }

    // Brute force: a corner is on the hull's boundary when a line through it and another corner
    // has every corner on one closed side (a supporting line turned about it meets a corner).
    fn brute_boundary(corners: &[[f32; 2]]) -> Vec<(i32, i32)> {
        let mut keep: Vec<_> = corners
            .iter()
            .filter(|&&p| {
                corners.iter().filter(|&&a| a != p).any(|&a| {
                    corners.iter().all(|&q| cross(p, a, q) >= 0.0)
                        || corners.iter().all(|&q| cross(p, a, q) <= 0.0)
                })
            })
            .map(|p| corner_key(*p))
            .collect();
        keep.sort();
        keep
    }

    fn corners_of(book: &ScreenHullBook) -> Vec<[f32; 2]> {
        book.corners.values().map(|(point, _)| *point).collect()
    }

    #[test]
    fn the_hull_keeps_the_boundary_corners_through_events() {
        let mut book = ScreenHullBook::default();
        // A diamond of 256 m patches, some dry (a point at the centre), some wet (a footprint).
        for x in -6..=6 {
            for z in -6..=6 {
                if i32::abs(x) + i32::abs(z) > 7 {
                    continue;
                }
                let (cx, cz) = (x as f32 * 256.0 + 128.0, z as f32 * 256.0 + 128.0);
                let rect = if (x + z) % 3 == 0 {
                    [cx - 128.0, cz - 128.0, cx + 128.0, cz + 128.0]
                } else {
                    [cx, cz, cx, cz]
                };
                book.set((x, z), rect);
            }
        }
        let hull = book.hull().to_vec();
        assert_eq!(sorted(&hull), brute_boundary(&corners_of(&book)));
        // Consecutive boundary points turn left or run straight on.
        for index in 0..hull.len() {
            let n = hull.len();
            assert!(cross(hull[index], hull[(index + 1) % n], hull[(index + 2) % n]) >= 0.0);
        }
        // An interior patch changing leaves the hull as it was.
        let revision = book.revision();
        book.set((0, 0), [100.0, 100.0, 120.0, 120.0]);
        book.remove((1, 1));
        assert_eq!(book.revision(), revision, "interior events must not stale the hull");
        // An edge patch leaving changes it.
        book.remove((7, 0));
        book.remove((6, 1));
        assert_ne!(book.revision(), revision);
        let hull = book.hull().to_vec();
        assert_eq!(sorted(&hull), brute_boundary(&corners_of(&book)));
        // Collinear and single corners.
        let mut line = ScreenHullBook::default();
        for x in 0..5 {
            line.set((x, 0), [x as f32, 0.0, x as f32, 0.0]);
        }
        assert_eq!(line.hull().len(), 5);
        line.clear();
        assert!(line.hull().is_empty());
        line.set((0, 0), [3.0, 4.0, 3.0, 4.0]);
        assert_eq!(line.hull(), &[[3.0, 4.0]]);
    }

    #[test]
    fn shared_corners_stay_while_a_patch_still_has_them() {
        let mut book = ScreenHullBook::default();
        book.set((0, 0), [0.0, 0.0, 1.0, 1.0]);
        book.set((1, 0), [1.0, 0.0, 2.0, 1.0]);
        book.remove((0, 0));
        assert_eq!(sorted(book.hull()), sorted(&[[1.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0]]));
        assert_eq!(book.len(), 1);
    }
}
