// SPDX-License-Identifier: GPL-2.0-only

//! Compiled roadbed rendering split by ownership and visual layer.

mod bridge;
mod coverage;
mod earthwork;
mod geometry;
mod markings;
mod top_surface;

pub(super) use coverage::{
    CompiledSurfaceCoverage, build_compiled_surface_coverage,
    build_compiled_surface_coverage_for_chunks,
};
pub(super) use markings::emit_compiled_lane_markings;
pub(super) use top_surface::emit_compiled_surface_mesh;

#[cfg(test)]
mod tests;
