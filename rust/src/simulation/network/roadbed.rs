// SPDX-License-Identifier: GPL-2.0-only

//! Constructive roadbed (`ROAD-68`): road cross-section profiles, immutable road pieces, the
//! road chunk index and visible-surface queries, all derived from the road graph.
//!
//! Junction, bend and terminal nodes get node pieces from their layouts (`ROAD-68-C`), and span
//! pieces run between the mouth sections those node pieces evaluate. `docs/roads.md` owns the
//! contract and the gate plan.

mod arms;
#[cfg(test)]
pub(crate) mod fixtures;
mod geometry;
mod index;
mod layout;
mod mesh;
mod node;
mod profile;
mod query;
mod section;
mod span;
mod system;
mod terrain;
mod walkways;

pub(crate) use arms::{NodeArms, directions_pass_through, edges_pass_through};
pub(crate) use geometry::CoverageTriangle;
#[cfg(test)]
pub(crate) use geometry::{RoadSurfaceTriangleQueryIndex, triangulate_constrained_rings_xz};
pub use layout::{
    ArmBand, ArmInput, ArmKey, ArmKnot, ArmLayout, ArmSide, BoundaryBand, BoundaryTaper, CoreVertex,
    CoreVertexSource, Corner, CornerTangent, Crosswalk, EdgeEnd, JunctionLayout, LayoutError,
    NodeKind, SectionLine, SectorBoundary, SectorKind, SectorLayout, Side,
};
pub use mesh::{BRIDGE_DECK_DEPTH_M, OuterChain, PieceMesh, SKIRT_DEPTH_M, StripKind};
pub(crate) use node::{check_node, terminal_plane};
pub use node::{NodeError, NodePiece};
pub use profile::{BandKind, CURB_STEP_HEIGHT_M, Level, ProfileBand, RoadProfile};
pub(crate) use profile::{preview_candidate_roadbed_half_width_m, visual_roadbed_half_width_m};
pub(crate) use query::{GroundedPieceScratch, LocalSurfaceQuery};
pub use query::{LaneSurfaceQuery, PieceId, PortalPart, WallSegment};
pub use section::{RoadBand, RoadSection};
pub(crate) use span::incident_direction;
pub use span::{MouthSection, SpanPiece};
pub(crate) use system::RoadbedScratch;
pub use system::{RebuiltOwners, RoadbedDelta, RoadbedRestore, RoadbedSystem};
pub use terrain::TerrainChanges;
pub(crate) use walkways::{FootpathJoin, MouthFrame, NodeWalkways, SidewalkEnd};

/// Road chunk key `(x, z)` in the terrain-aligned render grid of a [`RoadbedSystem`].
pub type SurfaceChunkKey = (i32, i32);
/// Roadbed 2-D geometry in world XZ metres.
pub type RoadVec2 = glam::DVec2;
/// Roadbed 3-D geometry in world XYZ metres.
pub type RoadVec3 = glam::DVec3;
