//! Node boundary provenance and height-export tests.

use super::*;
use crate::simulation::network::surface::RoadVec3;

mod direct_segments;
mod duplicate_points;
mod height;
mod interpolation;
mod overlapping_sources;
mod raised_steps;
mod support;

use support::*;
