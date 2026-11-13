// SPDX-License-Identifier: GPL-2.0-only

//! Straight guide segments.

use glam::DVec2;

/// A straight guide segment from `a` to `b`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    /// The start.
    pub a: DVec2,
    /// The end.
    pub b: DVec2,
}

impl Line {
    /// The line from `a` to `b`.
    pub fn new(a: DVec2, b: DVec2) -> Self {
        Self { a, b }
    }

    /// Length in metres.
    pub fn length(&self) -> f64 {
        (self.b - self.a).length()
    }

    /// The unit direction from `a` to `b` (zero for a zero-length line).
    pub fn direction(&self) -> DVec2 {
        (self.b - self.a).normalize_or_zero()
    }

    /// The point at distance `s` from `a`, clamped to the line; the ends are returned exactly.
    pub fn point_at(&self, s: f64) -> DVec2 {
        let length = self.length();
        if s <= 0.0 || length == 0.0 {
            self.a
        } else if s >= length {
            self.b
        } else {
            self.a + (self.b - self.a) * (s / length)
        }
    }

    // Split at `0 < s < length`; both halves share the split point.
    pub(super) fn split_at(&self, s: f64) -> (Self, Self) {
        let split = self.point_at(s);
        (Self::new(self.a, split), Self::new(split, self.b))
    }

    /// The line from `b` to `a`.
    pub fn reversed(&self) -> Self {
        Self::new(self.b, self.a)
    }

    /// The parallel line `d` metres to the left of the direction of travel (right for negative
    /// `d`).
    pub fn offset(&self, d: f64) -> Self {
        let shift = self.direction().perp() * d;
        Self::new(self.a + shift, self.b + shift)
    }
}
