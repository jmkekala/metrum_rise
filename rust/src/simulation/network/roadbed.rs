// SPDX-License-Identifier: GPL-2.0-only

//! Constructive roadbed (`ROAD-68`): road cross-section profiles, immutable road pieces, the
//! road chunk index and visible-surface queries, all derived from the road graph.
//!
//! `ROAD-68-A` stands up this façade without geometry. No pieces are built, so every query
//! answers `None`, `false` or empty and nothing renders; consumers take their existing
//! "surface missing" paths. `docs/roads.md` owns the contract and the gate plan.

mod geometry;
mod profile;
mod query;
mod section;
mod system;

pub(crate) use geometry::{
    RoadSurfaceTriangleQueryIndex, ray_xz_interval_for_bounds, road_ray_triangle_intersection_t,
    triangulate_constrained_rings_xz,
};
pub use profile::{BandKind, CURB_STEP_HEIGHT_M};
pub(crate) use profile::{preview_candidate_roadbed_half_width_m, visual_roadbed_half_width_m};
pub use query::{LaneSurfaceQuery, PieceId};
pub use section::{RoadBand, RoadSection};
pub use system::RoadbedSystem;

/// Road chunk key `(x, z)` in the terrain-aligned render grid of a [`RoadbedSystem`].
pub type SurfaceChunkKey = (i32, i32);
/// Roadbed 2-D geometry in world XZ metres.
pub type RoadVec2 = glam::DVec2;
/// Roadbed 3-D geometry in world XYZ metres.
pub type RoadVec3 = glam::DVec3;
