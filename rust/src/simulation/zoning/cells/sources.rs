// SPDX-License-Identifier: GPL-2.0-only

//! Persistent bounded curve guides for painted groups, independent of mutable road edge IDs.

use super::{CELL_DEPTH, CELL_GROUP_COLUMNS, CellBounds, CellFrontage, CellKey, GridFrame};
use glam::DVec2;
use serde::{Deserialize, Serialize};

/// Bounded lattice strip identity; replacing its guide does not retain old geometry versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(super) struct CurveSourceKey {
    /// Canonical lattice, independent of the store's numeric grid ID.
    pub(super) frame: GridFrame,
    /// Inclusive low cell address.
    pub(super) min: [i32; 2],
    /// Exclusive high cell address.
    pub(super) max: [i32; 2],
    /// Road-facing edge of the strip.
    pub(super) frontage: CellFrontage,
}

/// Original centreline interval supplying one rigid curved group; never stores graph indices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct CellCurveSource {
    /// Original strip covered by this guide.
    pub(super) key: CurveSourceKey,
    /// Horizontal road points clipped to the original group's station interval.
    #[serde(with = "point_bits")]
    pub(super) points: Vec<[f64; 2]>,
    /// Road half-width including the sidewalk used to place the strip.
    #[serde(with = "width_bits")]
    pub(super) half_width: f64,
    /// Side relative to the ordered guide points.
    pub(super) side: i8,
    /// Terminal groups require their final whole cell to fit before the road endpoint.
    pub(super) terminal: bool,
}

mod point_bits {
    //! Lossless guide coordinates without changing JSON parsing for unrelated systems.

    use serde::{Deserialize, Deserializer, Serializer};

    /// Stores IEEE bits as integer pairs, avoiding decimal parsing changes to the grid basis.
    pub(super) fn serialize<S: Serializer>(
        points: &[[f64; 2]],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(points.iter().map(|point| point.map(f64::to_bits)))
    }

    /// Restores the exact coordinates; source validation subsequently rejects nonfinite values.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<[f64; 2]>, D::Error> {
        Ok(Vec::<[u64; 2]>::deserialize(deserializer)?
            .into_iter()
            .map(|point| point.map(f64::from_bits))
            .collect())
    }
}

mod width_bits {
    //! Preserve the source width's exact equality with the current road width.

    use serde::{Deserialize, Deserializer, Serializer};

    /// Stores the original computed width without decimal conversion.
    pub(super) fn serialize<S: Serializer>(width: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(width.to_bits())
    }

    /// Restores the exact width; source validation subsequently enforces its physical range.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        u64::deserialize(deserializer).map(f64::from_bits)
    }
}

impl CurveSourceKey {
    /// Along axis, depth axis, and sign pointing away from the road.
    pub(super) fn axes(self) -> (usize, usize, i32) {
        match self.frontage {
            CellFrontage::MinY => (0, 1, 1),
            CellFrontage::MaxY => (0, 1, -1),
            CellFrontage::MinX => (1, 0, 1),
            CellFrontage::MaxX => (1, 0, -1),
        }
    }

    /// Enumerates at most twenty-four cell addresses in canonical order.
    pub(super) fn cells(self, grid: u64) -> impl Iterator<Item = CellKey> {
        (self.min[0]..self.max[0])
            .flat_map(move |x| (self.min[1]..self.max[1]).map(move |y| CellKey { grid, x, y }))
    }

    /// World envelope used by the existing cell chunk index.
    pub(super) fn bounds(self) -> CellBounds {
        CellBounds::from_points([
            self.frame
                .world(f64::from(self.min[0]), f64::from(self.min[1])),
            self.frame
                .world(f64::from(self.max[0]), f64::from(self.min[1])),
            self.frame
                .world(f64::from(self.max[0]), f64::from(self.max[1])),
            self.frame
                .world(f64::from(self.min[0]), f64::from(self.max[1])),
        ])
    }
}

impl CellCurveSource {
    /// Columns of a non-terminal curved group, counted from its lattice-rounded origin.
    /// Generation and validation share this so micrometre phase rounding cannot add a sliver.
    pub(super) fn open_columns(chord_along: f64, cell_m: f64) -> i32 {
        ((chord_along - 1e-7) / cell_m).ceil() as i32
    }

    /// Validates the complete guide/strip relationship, preventing unrelated saved geometry
    /// from granting frontage to a rectangle elsewhere in the world.
    pub(super) fn is_valid(&self) -> bool {
        let key = self.key;
        let (along, depth, depth_sign) = key.axes();
        let width = i64::from(key.max[along]) - i64::from(key.min[along]);
        if !key.frame.is_valid()
            || !(1..=CELL_GROUP_COLUMNS as i64).contains(&width)
            || i64::from(key.max[depth]) - i64::from(key.min[depth]) != CELL_DEPTH as i64
            || !key.bounds().is_valid()
            || !self.half_width.is_finite()
            || !(0.0..=128.0).contains(&self.half_width)
            || ![-1, 1].contains(&self.side)
            || self.points.len() < 2
            || !self
                .points
                .iter()
                .all(|p| super::geometry::valid_point(DVec2::from_array(*p)))
        {
            return false;
        }
        let start = DVec2::from_array(self.points[0]);
        let end = DVec2::from_array(self.points[self.points.len() - 1]);
        let chord = end - start;
        let cell_m = key.frame.cell_m();
        let epsilon = super::geometry::road_contact_precision(key.bounds());
        let length: f64 = self
            .points
            .windows(2)
            .map(|p| DVec2::from_array(p[0]).distance(DVec2::from_array(p[1])))
            .sum();
        if chord.length() < 1e-6
            || (self.terminal && chord.length() < cell_m - epsilon)
            || length > CELL_GROUP_COLUMNS as f64 * cell_m + epsilon
            || self.points.windows(2).any(|p| p[0] == p[1])
        {
            return false;
        }
        let direction = chord.normalize();
        let Some(basis) = GridFrame::new(DVec2::ZERO, direction, cell_m) else {
            return false;
        };
        let (u, v) = basis.axes();
        let axes = [u, v];
        if usize::from(direction.dot(v).abs() > direction.dot(u).abs()) != along {
            return false;
        }
        let along_sign = direction.dot(axes[along]).signum();
        let tangent = axes[along] * along_sign;
        let normal = DVec2::new(tangent.y, -tangent.x) * f64::from(self.side);
        if normal.dot(axes[depth]) * f64::from(depth_sign) < 0.5 {
            return false;
        }
        let support = self
            .points
            .iter()
            .map(|&p| normal.dot(DVec2::from_array(p)))
            .fold(f64::NEG_INFINITY, f64::max)
            + self.half_width;
        let origin = start + normal * (support - normal.dot(start));
        if GridFrame::new(origin, tangent, cell_m) != Some(key.frame) {
            return false;
        }
        let local = key.frame.local(origin).round();
        let count = if self.terminal {
            ((tangent.dot(chord) + epsilon) / cell_m).floor() as i32
        } else {
            Self::open_columns(tangent.dot(chord), cell_m)
        };
        let along_min = local[along] as i32 - if along_sign < 0.0 { count } else { 0 };
        let depth_min = local[depth] as i32 - if depth_sign < 0 { CELL_DEPTH as i32 } else { 0 };
        key.min[along] == along_min && key.min[depth] == depth_min && width == i64::from(count)
    }
}
