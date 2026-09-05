// SPDX-License-Identifier: GPL-2.0-only

//! Error-to-validation-diagnostic mapping.

use super::super::RoadSurfaceVisualNodePieceKind;
use super::super::arrangement::{NodeArrangement, NodeArrangementDiagnostic, NodeArrangementError};
use super::super::height::NodeHeightFieldError;
use super::super::keys::SurfaceXzKey;
use super::super::ownership::{
    NodeBooleanOwnershipError, NodeOwnedRegionArrangement, NodeOwnedRegionArrangementDiagnostic,
};
use super::super::rails::NodeRailGenerationError;
use super::super::triangulation::NodeTriangulationError;
use super::report::{
    NodeCanonicalPointDiagnostic, NodeGeometryBackend, NodeGeometryDiagnostic,
    NodeGeometryDiagnosticKind, NodeGeometryStage, NodeInvalidConstraintReason,
    NodeRejectedResidualKind, NodeSeamConstraintFailureReason, NodeValidationReport,
};

mod arrangement;
mod boolean;
mod height;
mod rail;
mod reports;
mod triangulation;
