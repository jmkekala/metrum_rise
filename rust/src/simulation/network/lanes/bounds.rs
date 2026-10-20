// SPDX-License-Identifier: GPL-2.0-only

//! Planar boxes of lane geometry, for exact conservative render culling (`SIM-02-K`).
//!
//! A pose sampled on a lane with at least two geometry points is a clamped interpolation
//! between two consecutive points, so its x/z lie in the box of the lane's points up to the
//! rounding of that interpolation. A lane with fewer than two points, or with a non-finite
//! coordinate, gets an unbounded box that no cull rejects: its samples can fall back to the
//! agent's stored position or stop early, and are then posed as before.

use rayon::prelude::*;

use super::{Lane, LaneSystem};

/// `[min_x, min_z, max_x, max_z]` of one lane's geometry.
pub(crate) type LanePlanarBox = [f32; 4];

const UNBOUNDED: LanePlanarBox = [
    f32::NEG_INFINITY,
    f32::NEG_INFINITY,
    f32::INFINITY,
    f32::INFINITY,
];

/// Lane boxes as last built, with the lane revision and lane count they were built for.
#[derive(Default)]
pub(super) struct LanePlanarBounds {
    built_for: Option<(u64, usize)>,
    boxes: Vec<LanePlanarBox>,
}

fn lane_box(lane: &Lane) -> LanePlanarBox {
    if lane.geometry.len() < 2 {
        return UNBOUNDED;
    }
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for point in &lane.geometry {
        if !point.x.is_finite() || !point.z.is_finite() {
            return UNBOUNDED;
        }
        bounds[0] = bounds[0].min(point.x);
        bounds[1] = bounds[1].min(point.z);
        bounds[2] = bounds[2].max(point.x);
        bounds[3] = bounds[3].max(point.z);
    }
    bounds
}

impl LaneSystem {
    /// Rebuilds the planar lane boxes when the lane revision or count changed since the last
    /// build: O(lane points) on the shared pool, else O(1). Every rebuild of lane geometry
    /// bumps the revision; the visible-surface height sync changes only heights.
    pub(crate) fn refresh_planar_bounds(&mut self) {
        let key = (self.revision, self.lanes.len());
        if self.planar_bounds.built_for == Some(key) {
            return;
        }
        let lanes = &self.lanes;
        let boxes = &mut self.planar_bounds.boxes;
        crate::thread_pools::install(|| {
            lanes
                .par_iter()
                .with_min_len(2_048)
                .map(lane_box)
                .collect_into_vec(boxes);
        });
        self.planar_bounds.built_for = Some(key);
    }

    /// Planar lane boxes by lane id, as of the last [`Self::refresh_planar_bounds`].
    pub(crate) fn planar_bounds(&self) -> &[LanePlanarBox] {
        debug_assert_eq!(
            self.planar_bounds.built_for,
            Some((self.revision, self.lanes.len()))
        );
        &self.planar_bounds.boxes
    }
}
