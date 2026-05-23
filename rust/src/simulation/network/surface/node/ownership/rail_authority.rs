//! Rail-source authority indexing for node boolean ownership.

use super::super::RoadSurfaceBandKind;
use super::super::arrangement::NodeBandOwner;
use super::super::rails::{NodeGeneratedContourKind, NodeRailConstraint, NodeRailContourSet};
use super::topology_keys::{
    NodeOwnershipPointKey, OwnedRegionEdgeKey, ownership_key_from_overlay_point,
    ownership_key_from_road_point, ownership_mm_key, point_key_lies_on_segment,
};
use super::{NodeBooleanOwnedRegion, NodeBooleanOwnershipError};
use std::collections::{BTreeMap, BTreeSet};

// Same-owner source points inside this sub-quarter-millimeter span are one canonical
// source duplicate cluster; wider same-mm collisions remain blocking ambiguity.
const SOURCE_DUPLICATE_CLUSTER_MAX_SPAN_UNITS: i64 = 256;

pub(super) struct NodeRailCanonicalPointSet {
    pub(super) all_points: Vec<NodeOwnershipPointKey>,
    pub(super) points_by_owner: BTreeMap<NodeBandOwner, Vec<NodeOwnershipPointKey>>,
    pub(super) segments_by_owner: BTreeMap<NodeBandOwner, Vec<OwnedRegionEdgeKey>>,
    pub(super) canonical_points_by_mm_key_by_owner:
        BTreeMap<NodeBandOwner, BTreeMap<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>>,
    pub(super) height_points_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<NodeOwnershipPointKey>>,
    pub(super) paths_by_owner: BTreeMap<NodeBandOwner, Vec<Vec<NodeOwnershipPointKey>>>,
}

mod collection;
mod point_set;
mod validation;

pub(super) use collection::canonical_points_for_rail_set;
#[cfg(test)]
pub(super) use collection::insert_open_source_segments;
pub(super) use validation::validate_owned_region_vertices_against_source_authority;
#[cfg(test)]
pub(super) use validation::{canonical_points_by_mm_key_by_owner, constraint_authority_owners};
#[cfg(not(test))]
use validation::{canonical_points_by_mm_key_by_owner, constraint_authority_owners};
