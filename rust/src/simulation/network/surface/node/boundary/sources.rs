//! Footprint boundary source-edge and direct-vertex provenance.

use super::super::band_semantics::band_kind_sort_key;
use super::*;

mod build;
mod direct_vertices;
mod lookup;
mod source_edges;

use direct_vertices::*;
pub(super) use lookup::{
    node_footprint_boundary_vertex_source_at_point,
    node_footprint_boundary_vertex_source_for_edge_point,
};
use source_edges::*;
