//! Terrain-clip union orchestration.

use super::super::{NodeOverlayContour, NodeOverlayShape, RoadSurfaceSystem};
use super::model::*;
use super::output::TerrainClipOutputSourceError;
use super::recovery::TerrainClipSourceChainRecovery;

mod api;
mod contours;
mod loop_build;
