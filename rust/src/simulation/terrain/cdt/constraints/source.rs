// SPDX-License-Identifier: GPL-2.0-only

//! Provenance attached to one canonical road constraint.

use super::super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::simulation::terrain::cdt) struct TerrainCdtRoadConstraintSource {
    pub(in crate::simulation::terrain::cdt) stable_piece_id: u64,
    pub(in crate::simulation::terrain::cdt) local_loop_index: u32,
    pub(in crate::simulation::terrain::cdt) local_edge_index: u32,
    pub(in crate::simulation::terrain::cdt) boundary_source: TerrainCdtRoadBoundarySource,
}
