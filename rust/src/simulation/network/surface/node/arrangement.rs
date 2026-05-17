//! Canonical node-arrangement identity and ownership data model.

mod build;
mod model;
mod seams;
mod steps;

#[cfg(test)]
mod tests;

pub(crate) use model::{
    NodeArrangement, NodeArrangementDiagnostic, NodeArrangementEdge, NodeArrangementEdgeId,
    NodeArrangementError, NodeArrangementFace, NodeArrangementFaceId, NodeArrangementKey,
    NodeArrangementVertex, NodeArrangementVertexId, NodeBandHeightFieldId, NodeBandOwner,
    NodeOwnedRegion, NodeOwnedRegionId,
};
pub(crate) use seams::{NodeRegionSeamConstraint, NodeSeamSource, seam_constraints_are_ambiguous};
pub(crate) use steps::{NodeExplicitVerticalStepSegment, owners_form_explicit_vertical_step_pair};
