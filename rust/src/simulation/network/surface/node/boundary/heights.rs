// SPDX-License-Identifier: GPL-2.0-only

//! Footprint boundary height resolution and conflict rejection.

use super::super::band_semantics::{raised_step_band_rank, raised_step_kinds_can_contact};
use super::sources::node_footprint_boundary_vertex_source_for_edge_point;
use super::*;

mod candidates;
mod raised_steps;
mod source_conflicts;
