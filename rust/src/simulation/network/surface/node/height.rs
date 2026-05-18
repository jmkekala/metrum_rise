//! Explicit height-carrier evaluation for canonical node-owned regions.

use super::arrangement::{NodeBandHeightFieldId, NodeBandOwner, NodeRegionSeamConstraint};
use super::backend::{
    RoadVec2, RoadVec3, overlay_point_to_road, quantize_road_vec2_to_overlay_grid,
    road_vec3_xz as xz,
};
use super::input::{NodeArrangementInput, NodeInputBandInterval};
use super::keys::{SURFACE_XZ_KEY_SCALE, SurfaceHeightMmKey, SurfaceXzKey};
use super::ownership::{NodeBooleanOwnedRegion, NodeBooleanOwnership};
use super::rails::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeGeneratedContourPurpose, NodeRailContourSet, NodeRailGenerationError,
};
use super::segments::raw_tuple_key_lies_exactly_on_segment;
use super::terminal::{
    NodeTerminalCapBand, TerminalCapGenerationError, terminal_cap_bands_by_mouth,
};
use super::{
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind, SurfaceCdt,
};
use spade::{Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet};

mod authority;
mod build;
mod carriers;
mod evaluate;
mod field;
mod grade;
mod handoff;
mod model;
mod patch;
mod seams;
mod source_edges;
mod triangles;
mod vertices;

#[cfg(test)]
mod tests;

pub(crate) use grade::{NodeGradeCarrierDecision, NodeGradeVertexAuthority};
pub(crate) use model::{
    NodeHeightAuthoritySource, NodeHeightFieldError, NodeHeightSolution, NodeHeightedRegion,
    NodeHeightedVertex,
};
