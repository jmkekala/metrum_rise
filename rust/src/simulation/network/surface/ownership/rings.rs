//! Canonical ring noding and cleanup helpers for node boolean ownership.

mod cleanup;
mod noding;

pub(super) use cleanup::clean_canonical_owned_region_shapes;
#[cfg(test)]
pub(super) use noding::{
    canonicalize_final_owned_region_boundary_edges,
    canonicalize_owned_region_rings_with_rail_point_set,
};
pub(super) use noding::{
    canonicalize_owned_region_rings, noded_owned_region_edge_points, owned_region_global_points,
};
