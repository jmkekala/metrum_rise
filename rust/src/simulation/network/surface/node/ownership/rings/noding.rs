//! Canonical owned-region ring noding helpers.

use super::super::super::NodeOverlayContour;
use super::super::super::NodeOverlayShapes;
use super::super::super::RoadSurfaceVisualNodePieceKind;
use super::super::super::arrangement::NodeBandOwner;
use super::super::super::rails::NodeGeneratedContourClaimPriority;
use super::super::rail_authority::NodeRailCanonicalPointSet;
use super::super::topology_keys::{
    NodeOwnershipPointKey, overlay_point_from_key, ownership_key_from_overlay_point,
    ownership_mm_key, point_key_lies_exactly_on_segment, point_key_lies_on_segment,
    segment_parameter_key,
};
use super::super::{NodeBooleanOwnedRegion, NodeBooleanOwnershipError};

mod canonicalization;
mod edges;
mod rail_paths;

pub(in crate::simulation::network::surface::node::ownership) use canonicalization::{
    canonicalize_final_join_or_cap_owned_region_boundary_edges,
    canonicalize_final_owned_region_boundary_edges_for_piece_kind, canonicalize_owned_region_rings,
    canonicalize_owned_region_rings_with_rail_point_set_for_piece_kind,
};
#[cfg(test)]
pub(in crate::simulation::network::surface::node::ownership) use canonicalization::{
    canonicalize_final_owned_region_boundary_edges,
    canonicalize_owned_region_rings_with_rail_point_set,
};
pub(in crate::simulation::network::surface::node::ownership) use edges::dedup_consecutive_overlay_points;
#[cfg(test)]
use edges::noded_owned_region_edge_points_with_rail_paths;
use edges::{noded_owned_region_contour, noded_owned_region_contour_with_rail_paths};
use rail_paths::rail_path_points_between;
