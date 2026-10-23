// SPDX-License-Identifier: GPL-2.0-only

//! Constructive roadbed (`ROAD-68`): road cross-section profiles, immutable road pieces, the
//! road chunk index and visible-surface queries, all derived from the road graph.
//!
//! `ROAD-68-B` builds span pieces between the mouth stations the graph clips give; junction,
//! bend and terminal pieces arrive in `ROAD-68-C`. `docs/roads.md` owns the contract and the gate
//! plan.

#[cfg(test)]
pub(crate) mod fixtures;
mod geometry;
mod index;
mod mesh;
mod profile;
mod query;
mod section;
mod span;
mod system;

pub(crate) use geometry::{
    RoadSurfaceTriangleQueryIndex, ray_xz_interval_for_bounds, road_ray_triangle_intersection_t,
    triangulate_constrained_rings_xz,
};
pub use mesh::{OuterChain, PieceMesh};
pub use profile::{BandKind, CURB_STEP_HEIGHT_M, Level, ProfileBand, RoadProfile};
pub(crate) use profile::{preview_candidate_roadbed_half_width_m, visual_roadbed_half_width_m};
pub use query::{LaneSurfaceQuery, PieceId};
pub use section::{RoadBand, RoadSection};
pub use span::{MouthSection, SpanPiece};
pub use system::RoadbedSystem;

/// Road chunk key `(x, z)` in the terrain-aligned render grid of a [`RoadbedSystem`].
pub type SurfaceChunkKey = (i32, i32);
/// Roadbed 2-D geometry in world XZ metres.
pub type RoadVec2 = glam::DVec2;
/// Roadbed 3-D geometry in world XYZ metres.
pub type RoadVec3 = glam::DVec3;
