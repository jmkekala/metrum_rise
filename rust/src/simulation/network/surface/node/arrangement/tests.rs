// SPDX-License-Identifier: GPL-2.0-only

//! Canonical node arrangement construction and export tests.

use super::super::backend::RoadVec2;
use super::super::height::NodeGradeCarrierDecision;
use super::super::height::NodeGradeVertexAuthority;
use super::super::height::{NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex};
use super::super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use super::*;

mod edges;
mod seams;
mod steps;
mod support;
mod vertices;

use support::*;
