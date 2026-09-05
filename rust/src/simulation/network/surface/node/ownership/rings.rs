// SPDX-License-Identifier: GPL-2.0-only

//! Canonical ring noding and cleanup helpers for node boolean ownership.

mod cleanup;
mod noding;

pub(super) use cleanup::clean_canonical_owned_region_shapes_with_reuse;
pub(super) use noding::{NodeOwnershipPointIndex, canonicalize_owned_region_rings};
#[cfg(test)]
pub(super) use noding::{
    canonicalize_final_owned_region_boundary_edges,
    canonicalize_owned_region_rings_with_rail_point_set,
};
