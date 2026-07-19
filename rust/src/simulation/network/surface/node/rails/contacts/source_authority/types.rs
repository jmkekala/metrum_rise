//! Shared source-authority contact data.

use super::super::{
    GeneratedContourDirectedEdge, GeneratedContourEdgeKey, NodeBandOwner,
    NodeGeneratedContourClaimPriority, NodeOverlayShapes, NodeRailConstraint,
    NodeRailConstraintKind, NodeRailPointKey, RoadSurfaceBandKind,
};

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) const SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS: i64 =
    4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::node::rails::contacts) struct GeneratedSameBandContactConstraint
{
    pub(in crate::simulation::network::surface::node::rails::contacts) kind: NodeRailConstraintKind,
    pub(in crate::simulation::network::surface::node::rails::contacts) owner: NodeBandOwner,
    pub(in crate::simulation::network::surface::node::rails::contacts) opposite_owner:
        NodeBandOwner,
    pub(in crate::simulation::network::surface::node::rails::contacts) start: NodeRailPointKey,
    pub(in crate::simulation::network::surface::node::rails::contacts) end: NodeRailPointKey,
    pub(in crate::simulation::network::surface::node::rails::contacts) source_mouth_order_index:
        usize,
    pub(in crate::simulation::network::surface::node::rails::contacts) source_band_index:
        Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::node::rails::contacts) struct GeneratedSameBandContactConstraintKey
{
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) struct GeneratedRaisedStepEndpointSource
{
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) constraint_index:
        usize,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) source_mouth_order_index:
        usize,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) source_band_index:
        Option<usize>,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) owners:
        [NodeBandOwner; 2],
}

#[derive(Clone)]
pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) struct RaisedStepSourceConstraint<
    'a,
> {
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) source:
        GeneratedRaisedStepEndpointSource,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) constraint:
        &'a NodeRailConstraint,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) edges:
        Vec<GeneratedContourDirectedEdge>,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) min_x: i64,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) min_z: i64,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) max_x: i64,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) max_z: i64,
}

pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) struct RaisedStepSourceAuthority<
    'a,
> {
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) constraints:
        Vec<RaisedStepSourceConstraint<'a>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) struct SourceAuthorizedTargetGroupKey
{
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) owner:
        NodeBandOwner,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) kind:
        RoadSurfaceBandKind,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) claim_priority:
        NodeGeneratedContourClaimPriority,
}

#[derive(Clone, Debug)]
pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) struct SourceAuthorizedTargetGroup
{
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) key:
        SourceAuthorizedTargetGroupKey,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) contour_indices:
        Vec<usize>,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) shapes:
        NodeOverlayShapes,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) shape_edges:
        Vec<GeneratedContourDirectedEdge>,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) min_x: i64,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) min_z: i64,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) max_x: i64,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) max_z: i64,
}

impl GeneratedSameBandContactConstraint {
    pub(in crate::simulation::network::surface::node::rails::contacts) fn key(
        self,
    ) -> GeneratedSameBandContactConstraintKey {
        let edge = GeneratedContourEdgeKey::new(self.start, self.end);
        GeneratedSameBandContactConstraintKey {
            kind: self.kind,
            owner: self.owner,
            opposite_owner: self.opposite_owner,
            start: edge.start,
            end: edge.end,
        }
    }
}

impl SourceAuthorizedTargetGroup {
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn bounds_disjoint_edge(
        &self,
        edge: GeneratedContourDirectedEdge,
    ) -> bool {
        let min_x = edge.start.0.min(edge.end.0);
        let min_z = edge.start.1.min(edge.end.1);
        let max_x = edge.start.0.max(edge.end.0);
        let max_z = edge.start.1.max(edge.end.1);
        self.bounds_disjoint(min_x, min_z, max_x, max_z)
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn bounds_disjoint_group(
        &self,
        other: &Self,
    ) -> bool {
        self.bounds_disjoint(other.min_x, other.min_z, other.max_x, other.max_z)
    }

    fn bounds_disjoint(&self, min_x: i64, min_z: i64, max_x: i64, max_z: i64) -> bool {
        if self.min_x > self.max_x || self.min_z > self.max_z || min_x > max_x || min_z > max_z {
            return true;
        }
        self.max_x + 1 < min_x
            || max_x + 1 < self.min_x
            || self.max_z + 1 < min_z
            || max_z + 1 < self.min_z
    }
}

impl RaisedStepSourceConstraint<'_> {
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn bounds_disjoint_group(
        &self,
        group: &SourceAuthorizedTargetGroup,
    ) -> bool {
        self.bounds_disjoint(group.min_x, group.min_z, group.max_x, group.max_z)
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn bounds_disjoint_source(
        &self,
        other: &Self,
    ) -> bool {
        self.bounds_disjoint(other.min_x, other.min_z, other.max_x, other.max_z)
    }

    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) fn bounds_contains_key(
        &self,
        point: NodeRailPointKey,
    ) -> bool {
        self.min_x - SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS <= point.0
            && point.0 <= self.max_x + SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS
            && self.min_z - SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS <= point.1
            && point.1 <= self.max_z + SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS
    }

    fn bounds_disjoint(&self, min_x: i64, min_z: i64, max_x: i64, max_z: i64) -> bool {
        if self.min_x > self.max_x || self.min_z > self.max_z || min_x > max_x || min_z > max_z {
            return true;
        }
        self.max_x + SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS < min_x
            || max_x + SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS < self.min_x
            || self.max_z + SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS < min_z
            || max_z + SOURCE_AUTHORITY_BOUNDS_MARGIN_KEYS < self.min_z
    }
}
