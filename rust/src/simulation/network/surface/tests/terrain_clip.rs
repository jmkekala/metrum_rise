//! Terrain clip and road-touched CDT stage contract tests.

use super::super::RoadSurfaceTerrainClipContourRole;
use super::super::keys::{SurfaceHeightMmKey, SurfaceXzKey};
use super::*;

mod ambiguity;
mod cdt;
mod loops;
mod road_locked;
mod source_chains;
mod support;
mod union;

use support::*;
