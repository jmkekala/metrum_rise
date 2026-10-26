// SPDX-License-Identifier: GPL-2.0-only

//! Graded terrain surface (`TERRAIN-07`, approach B): the base heightmap plus a grading and
//! footprint distance layer per 64 m tile near roads and building sites, sampled on four level
//! lattices, with material classes, conflict walls, and the point and ray queries every
//! consumer moves to. `docs/terrain.md` "Graded terrain surface"
//! owns the contract.
//!
//! Every value is a pure function of its world position, the base heightmap, the grounded road
//! seams and top triangles and the building sites within the cap: `f64` geometry, exact pruning,
//! seams and sites in canonical order, so a tile's bytes never depend on the batch, the order,
//! the worker count or the edit history. Nothing here is cached; baked tiles are the caller's to keep. `TERRAIN-07-A` runs
//! beside the legacy exact cut and no consumer reads it yet.

mod coverage;
mod grade;
mod portal;
mod prune;
mod query;
mod sample;
mod seams;
mod select;
mod sites;
mod tile;
mod walls;

pub use grade::{CAP_M, FILL_ARM_MAX_M, INSIDE_REACH_M, TIE_IN_SLOPE, overlap};
pub use portal::{PORTAL_CLEARANCE_M, PORTAL_REACH_M};
pub use query::{GradedSurface, GroundClass, QueryScratch};
pub use seams::{Seam, SeamKind};
pub use select::{
    SelectScratch, dirty_tiles_for_base_change, dirty_tiles_for_road_changes,
    dirty_tiles_for_site_changes,
};
pub use sites::{NoSites, SiteBuffer, SiteSource};
pub use tile::{BakeError, BakeStats, Level, TILE_M, TileInputs, TileKey, TilePayload, bake_tiles};
pub use walls::WallFace;

#[cfg(test)]
mod portal_tests;
#[cfg(test)]
pub(crate) mod site_tests;
#[cfg(test)]
pub(crate) mod tests;
