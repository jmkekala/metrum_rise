//! Explicit height-carrier evaluation for canonical node-owned regions.

use super::arrangement::{NodeBandHeightFieldId, NodeBandOwner, NodeRegionSeamConstraint};
use super::backend::{
    RoadVec2, RoadVec3, overlay_point_to_road, quantize_road_vec2_to_overlay_grid,
    road_vec3_xz as xz,
};
use super::grade::{
    NodeGradeCarrierDecision, NodeGradeExplicitSeamHeightKey, NodeGradeVertexAuthority,
    apply_junctionn_node_grade_carrier, canonical_explicit_seam_owner_pair,
    material_height_constraints_for_vertex,
};
use super::input::{NodeArrangementInput, NodeInputBandInterval};
use super::keys::{SURFACE_XZ_KEY_SCALE, SurfaceHeightMmKey, SurfaceXzKey};
use super::ownership::{NodeBooleanOwnedRegion, NodeBooleanOwnership};
use super::rails::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeGeneratedContourPurpose, NodeRailContourSet,
};
use super::segments::{
    raw_tuple_key_lies_exactly_on_segment, raw_tuple_quantization_cell_intersects_segment,
};
use super::terminal::{
    NodeTerminalCapBand, TerminalCapGenerationError, terminal_cap_bands_by_mouth,
};
use super::{
    NodeOverlayContour, NodeOverlayPoint, NodeOverlayShape, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind, SurfaceCdt, WORLD_POINT_DEDUP_DISTANCE_M,
};
use spade::{Point2, Triangulation};
use std::collections::BTreeMap;

mod build;
mod carriers;
mod evaluate;
mod field;
mod model;
mod seams;
mod source_edges;
mod triangles;

#[cfg(test)]
mod tests;

pub(crate) use model::{
    NodeHeightAuthoritySource, NodeHeightFieldError, NodeHeightSolution, NodeHeightedRegion,
    NodeHeightedVertex,
};
