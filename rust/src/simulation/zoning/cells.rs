// SPDX-License-Identifier: GPL-2.0-only

//! Sparse road-generated zoning cells, canonical geometry and shared paint/erase selection.
//!
//! Cell coordinates address a persistent oriented grid, independently of road array indices.
//! Building lots are consumers of cell paint; an unpainted cell is not a land reservation.

mod generation;
mod geometry;
mod lots;
mod queries;
mod selection;
mod sources;
mod store;

pub(crate) use generation::{CellGeneration, overlaps_road_corridors};
pub(crate) use geometry::{CellBounds, GridFrame, interiors_overlap, road_contact_interior};
pub(crate) use lots::{CellFrontage, CellLot};
pub(crate) use selection::{CellEdit, CellSelection, CellSelectionShape};
pub(crate) use sources::CellCurveSource;
pub(crate) use store::{CellStore, RoadCellAlignment};

use serde::{Deserialize, Serialize};

/// Number of full square rows offered on each eligible road side.
pub(crate) const CELL_DEPTH: usize = 6;

/// Frontage columns in one rigid curved group or retained straight-grid constraint strip.
pub(crate) const CELL_GROUP_COLUMNS: usize = 4;

/// Fixed fraction denominator for coverage along one cell's increasing lattice coordinate.
pub(crate) const CELL_FRONTAGE_UNITS: u32 = 1_000_000_000;

/// Current road support for one front-row cell, rebuilt with the local road geometry cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CellRoadFrontage {
    /// Graph edge supplying this particular frontage, independent of competing nearby roads.
    pub(crate) edge: usize,
    /// Signed road side used by the existing building attachment contract.
    pub(crate) side: i8,
    /// Boundary of the cell adjoining the supplying road.
    pub(crate) boundary: CellFrontage,
    /// Inclusive beginning of road coverage, in billionths of this cell's front boundary.
    pub(crate) start: u32,
    /// Inclusive end of road coverage; a whole frontage ends at `CELL_FRONTAGE_UNITS`.
    pub(crate) end: u32,
}

/// One compatible initial asset footprint considered when deriving painted building lots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CellLotSize {
    /// Profile whose asset rules admit this footprint.
    pub(crate) profile: u16,
    /// Number of cells along its road-facing boundary.
    pub(crate) width: u16,
    /// Number of cells extending away from the road, at most six.
    pub(crate) depth: u8,
}
// Storage tiles are independent of roadside groups and must never create fill boundaries.
const BLOCK_SIDE: i32 = 8;
const BLOCK_CELLS: usize = (BLOCK_SIDE * BLOCK_SIDE) as usize;

/// Stable address in an oriented lattice; unrelated road splits cannot renumber its cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(crate) struct CellKey {
    /// Persistent, nonzero grid identity in this world.
    pub(crate) grid: u64,
    /// Signed column in the grid's canonical frame.
    pub(crate) x: i32,
    /// Signed row in the grid's canonical frame.
    pub(crate) y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct BlockKey {
    grid: u64,
    x: i32,
    y: i32,
}

impl CellKey {
    fn block(self) -> BlockKey {
        BlockKey {
            grid: self.grid,
            x: self.x.div_euclid(BLOCK_SIDE),
            y: self.y.div_euclid(BLOCK_SIDE),
        }
    }

    fn offset(self) -> usize {
        (self.y.rem_euclid(BLOCK_SIDE) * BLOCK_SIDE + self.x.rem_euclid(BLOCK_SIDE)) as usize
    }
}

impl BlockKey {
    fn cell(self, offset: usize) -> CellKey {
        CellKey {
            grid: self.grid,
            x: self.x * BLOCK_SIDE + offset as i32 % BLOCK_SIDE,
            y: self.y * BLOCK_SIDE + offset as i32 / BLOCK_SIDE,
        }
    }
}

#[cfg(test)]
mod tests;
