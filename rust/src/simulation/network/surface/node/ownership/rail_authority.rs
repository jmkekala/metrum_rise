//! Rail-source authority indexing for node boolean ownership.

use super::super::RoadSurfaceBandKind;
use super::super::arrangement::NodeBandOwner;
use super::super::backend::{RoadVec3, road_vec3_xz};
use super::super::rails::{
    NodeGeneratedContour, NodeGeneratedContourKind, NodeRailConstraint, NodeRailContourSet,
    NodeRailHeightCarrierPaths,
};
use super::topology_keys::{
    NodeOwnershipPointKey, OwnedRegionEdgeKey, ownership_key_from_road_point, ownership_mm_key,
};
#[cfg(test)]
use super::{NodeBooleanOwnedRegion, NodeCarrierProvenanceClosure};
use super::{NodeBooleanOwnershipError, NodeSourceCarrierSegmentId, source_carrier_segment_id};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type NodeRailHeightSourceKey = (RoadSurfaceBandKind, usize, usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeRailSourceSegmentAuthority {
    pub(super) owner: NodeBandOwner,
    pub(super) source: NodeRailHeightSourceKey,
    pub(super) source_segment_id: NodeSourceCarrierSegmentId,
    pub(super) segment: OwnedRegionEdgeKey,
    pub(super) materialization: NodeRailSourceSegmentMaterialization,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum NodeRailSourceSegmentMaterialization {
    DirectHeight,
    GeneratedCarrierSurface,
}

impl NodeRailSourceSegmentAuthority {
    pub(super) fn new(
        owner: NodeBandOwner,
        source: NodeRailHeightSourceKey,
        segment: OwnedRegionEdgeKey,
    ) -> Self {
        Self {
            owner,
            source,
            source_segment_id: source_carrier_segment_id(owner, source, segment),
            segment,
            materialization: NodeRailSourceSegmentMaterialization::DirectHeight,
        }
    }

    pub(super) fn generated_surface(
        owner: NodeBandOwner,
        source: NodeRailHeightSourceKey,
        segment: OwnedRegionEdgeKey,
    ) -> Self {
        Self {
            owner,
            source,
            source_segment_id: source_carrier_segment_id(owner, source, segment),
            segment,
            materialization: NodeRailSourceSegmentMaterialization::GeneratedCarrierSurface,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct NodeSourceCarrierRegistry {
    pub(crate) source_segments_by_owner:
        BTreeMap<NodeBandOwner, Vec<NodeRailSourceSegmentAuthority>>,
    pub(crate) height_points_by_source:
        BTreeMap<NodeRailHeightSourceKey, Vec<NodeOwnershipPointKey>>,
}

impl NodeSourceCarrierRegistry {
    pub(super) fn height_points(
        &self,
        source: NodeRailHeightSourceKey,
    ) -> Option<&Vec<NodeOwnershipPointKey>> {
        self.height_points_by_source.get(&source)
    }

    pub(super) fn has_source_carrier(
        &self,
        owner: NodeBandOwner,
        source: NodeRailHeightSourceKey,
    ) -> bool {
        self.height_points_by_source
            .get(&source)
            .is_some_and(|points| !points.is_empty())
            || self
                .source_segments_by_owner
                .get(&owner)
                .is_some_and(|segments| segments.iter().any(|segment| segment.source == source))
    }
}

pub(super) struct NodeRailCanonicalPointSet {
    pub(super) all_points: Vec<NodeOwnershipPointKey>,
    pub(super) points_by_owner: BTreeMap<NodeBandOwner, Vec<NodeOwnershipPointKey>>,
    pub(super) source_carriers: NodeSourceCarrierRegistry,
    pub(super) canonical_points_by_mm_key_by_owner:
        BTreeMap<NodeBandOwner, BTreeMap<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>>,
    pub(super) paths_by_owner: BTreeMap<NodeBandOwner, Vec<Vec<NodeOwnershipPointKey>>>,
}

mod collection;
mod point_set;
mod validation;

pub(super) use collection::canonical_points_for_rail_set;
#[cfg(test)]
pub(super) use validation::validate_owned_region_vertices_against_carrier_closure;
#[cfg(test)]
pub(super) use validation::{canonical_points_by_mm_key_by_owner, constraint_authority_owners};
#[cfg(not(test))]
use validation::{canonical_points_by_mm_key_by_owner, constraint_authority_owners};
