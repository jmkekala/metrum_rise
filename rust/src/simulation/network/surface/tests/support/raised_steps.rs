// SPDX-License-Identifier: GPL-2.0-only

//! Raised-step vertical-face helpers for road-surface tests.

use super::*;
use crate::simulation::network::surface::{
    backend::{RoadVec2, RoadVec3},
    keys as surface_keys, segments,
};

mod assertions;
mod extraction;
mod geometry;
mod keys;

pub(in crate::simulation::network::surface::tests) use assertions::*;
pub(in crate::simulation::network::surface::tests) use extraction::*;
pub(in crate::simulation::network::surface::tests) use geometry::*;
pub(in crate::simulation::network::surface::tests) use keys::*;
