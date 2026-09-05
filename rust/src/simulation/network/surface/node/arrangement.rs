//! Canonical node-arrangement identity and ownership data model.

mod build;
mod edges;
mod faces;
mod model;
mod regions;
mod seams;
mod steps;

#[cfg(test)]
mod tests;

pub(crate) use build::source_authorities_form_side_join_asphalt_sidewalk_split;
pub(crate) use model::{
    NodeArrangement, NodeArrangementAttachProfile, NodeArrangementBuildProfile,
    NodeArrangementDiagnostic, NodeArrangementEdge, NodeArrangementEdgeId, NodeArrangementError,
    NodeArrangementFace, NodeArrangementFaceId, NodeArrangementKey, NodeArrangementVertex,
    NodeArrangementVertexId, NodeBandHeightFieldId, NodeBandOwner, NodeOwnedRegion,
    NodeOwnedRegionId,
};
pub(crate) use seams::{NodeRegionSeamConstraint, NodeSeamSource, seam_constraints_are_ambiguous};
pub(in crate::simulation::network::surface::node) use seams::{
    PreparedSeamConstraintCoverages, SeamConstraintCoverageScratch,
    prepared_seam_constraints_covering_surface_key_edge_as_fragments_into,
};
pub(crate) use steps::{
    NodeExplicitVerticalStepSegment, NodeFinalExplicitStepTopologyCache,
    explicit_vertical_step_segments_authorize_height_side_at_key,
    owners_form_explicit_vertical_step_pair,
};
