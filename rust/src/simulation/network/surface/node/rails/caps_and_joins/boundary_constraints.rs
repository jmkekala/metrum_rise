// SPDX-License-Identifier: GPL-2.0-only

//! Terminal-cap and side-join rail boundary constraints.

use super::*;

mod owners;
mod side_join;
mod terminal;

pub(super) use side_join::push_side_join_band_boundary_constraints;
pub(super) use terminal::push_terminal_cap_band_boundary_constraints;
