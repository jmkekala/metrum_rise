// SPDX-License-Identifier: GPL-2.0-only

//! Rectangular cell coverage owned by a derived building lot, independently of its road access.

use super::{CELL_DEPTH, CellKey, CellStore, GridFrame};
use crate::simulation::zoning::ParcelGeometry;
use glam::DVec2;
use godot::prelude::Vector2;
use serde::{Deserialize, Serialize};

/// Front boundary in the canonical grid; the inward normal points into the covered rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(crate) enum CellFrontage {
    /// Minimum grid Y, growing toward increasing Y.
    MinY,
    /// Maximum grid X, growing toward decreasing X.
    MaxX,
    /// Maximum grid Y, growing toward decreasing Y.
    MaxY,
    /// Minimum grid X, growing toward increasing X.
    MinX,
}

/// Constant-size authority for the cells claimed by one parcel, including redevelopment grace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CellLot {
    origin: CellKey,
    width: u16,
    depth: u8,
    frontage: CellFrontage,
}

impl CellLot {
    /// Places a lot behind its lowest-coordinate frontage cell without shifting the frontage.
    pub(crate) fn from_frontage(
        first: CellKey,
        width: u16,
        depth: u8,
        frontage: CellFrontage,
    ) -> Option<Self> {
        let mut origin = first;
        let offset = i32::from(depth).checked_sub(1)?;
        match frontage {
            CellFrontage::MaxX => origin.x = origin.x.checked_sub(offset)?,
            CellFrontage::MaxY => origin.y = origin.y.checked_sub(offset)?,
            _ => {}
        }
        Self::new(origin, width, depth, frontage)
    }

    /// Visits the road-facing row in increasing canonical lattice coordinates.
    pub(crate) fn frontage_cells(self) -> impl Iterator<Item = CellKey> {
        (0..i32::from(self.width)).map(move |offset| {
            let mut key = self.origin;
            match self.frontage {
                CellFrontage::MinY => key.x += offset,
                CellFrontage::MaxY => {
                    key.x += offset;
                    key.y += i32::from(self.depth) - 1;
                }
                CellFrontage::MinX => key.y += offset,
                CellFrontage::MaxX => {
                    key.y += offset;
                    key.x += i32::from(self.depth) - 1;
                }
            }
            key
        })
    }

    /// Creates nonempty coverage with at most six depth rows and representable lattice bounds.
    pub(crate) fn new(
        origin: CellKey,
        width: u16,
        depth: u8,
        frontage: CellFrontage,
    ) -> Option<Self> {
        let lot = Self {
            origin,
            width,
            depth,
            frontage,
        };
        let (nx, ny) = lot.extent();
        (origin.grid != 0
            && width != 0
            && depth != 0
            && usize::from(depth) <= CELL_DEPTH
            && origin.x.checked_add(nx).is_some()
            && origin.y.checked_add(ny).is_some())
        .then_some(lot)
    }

    /// Minimum lattice address of the rectangle, independent of the frontage direction.
    pub(crate) fn origin(self) -> CellKey {
        self.origin
    }

    /// Canonical boundary on which every frontage cell must retain road access.
    pub(crate) fn frontage(self) -> CellFrontage {
        self.frontage
    }

    /// Visits exactly the covered cells without allocating a per-lot address vector.
    pub(crate) fn cells(self) -> impl Iterator<Item = CellKey> {
        let (nx, ny) = self.extent();
        (0..ny).flat_map(move |y| {
            (0..nx).map(move |x| CellKey {
                grid: self.origin.grid,
                x: self.origin.x + x,
                y: self.origin.y + y,
            })
        })
    }

    /// Reconstructs the canonical footprint; road metadata never rotates or translates it.
    pub(crate) fn geometry(
        self,
        frame: GridFrame,
        edge_idx: usize,
        side: i8,
        frontage_center_t: f32,
    ) -> ParcelGeometry {
        let (nx, ny) = self.extent();
        let x = f64::from(self.origin.x);
        let y = f64::from(self.origin.y);
        let (u, v) = frame.axes();
        let (tangent, normal) = match self.frontage {
            CellFrontage::MinY => (u, v),
            CellFrontage::MaxX => (v, -u),
            CellFrontage::MaxY => (-u, -v),
            CellFrontage::MinX => (-v, u),
        };
        let center = frame.world(x + f64::from(nx) * 0.5, y + f64::from(ny) * 0.5);
        let width = f64::from(self.width) * frame.cell_m();
        let depth = f64::from(self.depth) * frame.cell_m();
        let front = center - normal * (depth * 0.5);
        // Construct each corner directly in lattice coordinates. Independent vector sums
        // could round a shared boundary differently from the individual covered cells.
        let corners = self.corners(frame);
        let first = match self.frontage {
            CellFrontage::MinY => 0,
            CellFrontage::MaxX => 1,
            CellFrontage::MaxY => 2,
            CellFrontage::MinX => 3,
        };
        let corners = std::array::from_fn(|i| vector(corners[(first + i) % 4]));
        let aabb_min = corners.iter().copied().fold(Vector2::INF, |a, b| {
            Vector2::new(a.x.min(b.x), a.y.min(b.y))
        });
        let aabb_max = corners.iter().copied().fold(-Vector2::INF, |a, b| {
            Vector2::new(a.x.max(b.x), a.y.max(b.y))
        });
        ParcelGeometry {
            edge_idx,
            side,
            frontage_center_t,
            frontage_m: width as f32,
            depth_m: depth as f32,
            front_center: vector(front),
            center: vector(center),
            tangent: vector(tangent),
            normal: vector(normal),
            corners,
            aabb_min,
            aabb_max,
        }
    }

    /// Exact counterclockwise footprint before conversion to the engine's float coordinates.
    pub(crate) fn corners(self, frame: GridFrame) -> [DVec2; 4] {
        let (nx, ny) = self.extent();
        let x = f64::from(self.origin.x);
        let y = f64::from(self.origin.y);
        frame.rectangle_corners(x, y, f64::from(nx), f64::from(ny))
    }

    /// Rechecks deserialized coverage using the same bounds as normal construction.
    pub(super) fn valid(self) -> bool {
        Self::new(self.origin, self.width, self.depth, self.frontage) == Some(self)
    }

    fn extent(self) -> (i32, i32) {
        match self.frontage {
            CellFrontage::MinY | CellFrontage::MaxY => {
                (i32::from(self.width), i32::from(self.depth))
            }
            CellFrontage::MinX | CellFrontage::MaxX => {
                (i32::from(self.depth), i32::from(self.width))
            }
        }
    }
}

impl CellStore {
    /// Returns one nonzero profile only when every covered cell still has that designation.
    pub(crate) fn lot_profile(&self, lot: CellLot) -> Option<u16> {
        if !lot.valid() {
            return None;
        }
        let profile = self.profile(lot.origin)?;
        (profile != 0 && lot.cells().all(|key| self.profile(key) == Some(profile)))
            .then_some(profile)
    }

    /// Checks all coverage before an atomic lot claim; missing, mixed and claimed cells fail.
    pub(crate) fn can_claim_lot(&self, lot: CellLot, profile: u16) -> bool {
        self.lot_profile(lot) == Some(profile) && lot.cells().all(|key| self.lot(key) == Some(0))
    }

    /// Claims a validated rectangular footprint in one revision, with no partial writes.
    pub(crate) fn claim_lot(&mut self, lot: CellLot, parcel_id: u64, profile: u16) -> bool {
        if parcel_id == 0 || !self.can_claim_lot(lot, profile) {
            return false;
        }
        self.assign_lot(lot, parcel_id);
        true
    }

    /// Checks coverage for restoring an existing lot, whose paint may have changed in grace.
    pub(crate) fn can_restore_lot(&self, lot: CellLot) -> bool {
        lot.valid() && lot.cells().all(|key| self.lot(key) == Some(0))
    }

    /// Restores an existing owner's reservation independently of the current paint profile.
    pub(crate) fn restore_lot(&mut self, lot: CellLot, parcel_id: u64) -> bool {
        if parcel_id == 0 || !self.can_restore_lot(lot) {
            return false;
        }
        self.assign_lot(lot, parcel_id);
        true
    }

    /// Releases only the named owner, retaining all cell paint for subsequent lot derivation.
    pub(crate) fn release_lot(&mut self, lot: CellLot, parcel_id: u64) -> bool {
        if parcel_id == 0
            || !lot.valid()
            || !lot.cells().all(|key| self.lot(key) == Some(parcel_id))
        {
            return false;
        }
        self.assign_lot(lot, 0);
        true
    }
}

fn vector(point: DVec2) -> Vector2 {
    Vector2::new(point.x as f32, point.y as f32)
}
