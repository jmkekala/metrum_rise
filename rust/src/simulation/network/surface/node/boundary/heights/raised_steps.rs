// SPDX-License-Identifier: GPL-2.0-only

//! Raised-step boundary height authorization.

use super::*;

mod candidate;
mod explicit;
mod rank_pairs;
mod terminal;

#[cfg(test)]
pub(super) use candidate::raised_step_footprint_height_candidate;
pub(super) use candidate::raised_step_footprint_height_mm;
use explicit::explicit_same_kind_vertical_step_authorizes_footprint_height_pair;
use explicit::explicit_vertical_step_authorizes_footprint_height_pair;
#[cfg(test)]
use rank_pairs::ordered_raised_step_footprint_candidates;
use rank_pairs::raised_step_footprint_authorized_height_mm;
use rank_pairs::same_kind_explicit_vertical_step_footprint_height_mm;
use terminal::terminal_source_edge_endpoints_authorize_footprint_height_pair;
