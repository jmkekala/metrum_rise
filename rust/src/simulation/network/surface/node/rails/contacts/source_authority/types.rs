//! Shared source-authority contact data.

use super::super::{
    GeneratedContourDirectedEdge, GeneratedContourEdgeKey, NodeBandOwner,
    NodeGeneratedContourClaimPriority, NodeOverlayShapes, NodeRailConstraint,
    NodeRailConstraintKind, NodeRailPointKey, RoadSurfaceBandKind,
};

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

#[derive(Clone, Copy)]
pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) struct RaisedStepSourceConstraint<
    'a,
> {
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) source:
        GeneratedRaisedStepEndpointSource,
    pub(in crate::simulation::network::surface::node::rails::contacts::source_authority) constraint:
        &'a NodeRailConstraint,
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
