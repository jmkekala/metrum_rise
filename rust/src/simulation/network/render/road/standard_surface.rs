// SPDX-License-Identifier: GPL-2.0-only

//! Roadbed rendering: piece surfaces, bridge structures and lane markings.

mod bridge;
mod coverage;
mod geometry;
mod markings;
mod top_surface;

pub(super) use bridge::emit_bridge_structures;
pub(super) use coverage::{
    CompiledSurfaceCoverage, build_compiled_surface_coverage,
    build_compiled_surface_coverage_for_chunks, coverage_of_pieces,
};
pub(super) use markings::emit_compiled_lane_markings;
pub(super) use top_surface::emit_piece_surfaces;
