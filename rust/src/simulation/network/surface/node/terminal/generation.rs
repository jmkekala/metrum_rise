//! Terminal-cap band generation and canonicalization.

use super::height_anchors::{
    terminal_end_band_inner_height_anchors, terminal_end_band_outer_height_anchors,
    terminal_side_band_height_anchors,
};
use super::paths::{
    clean_terminal_cap_path_world, normalized_terminal_cap_direction, terminal_cap_contour_world,
    terminal_offset_boundary_path, terminal_offset_boundary_path_with_linear_height,
};
use super::*;

mod build;
mod canonicalization;

pub(super) use build::terminal_cap_bands;
pub(super) use canonicalization::canonicalize_terminal_cap_bands;
