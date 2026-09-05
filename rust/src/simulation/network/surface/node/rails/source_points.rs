// SPDX-License-Identifier: GPL-2.0-only

//! Source height-carrier point collection for generated node rails.

mod collection;
mod materialization;
mod paths;

use super::super::RoadSurfaceBandKind;

type NodeRailHeightSourceKey = (RoadSurfaceBandKind, usize, usize);

pub(super) use collection::push_band_height_carrier_points;
pub(super) use materialization::push_owned_region_height_carrier_points;
pub(super) use paths::{interval_height_carrier_paths, interval_height_carrier_points};
