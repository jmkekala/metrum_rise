//! Boolean domain claiming and overlay helpers for node ownership.

use super::super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint};
use super::super::backend::road_vec2_to_overlay_point;
use super::super::rails::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeRailConstraint, NodeRailConstraintKind, NodeRailContourSet,
};
use super::super::{NodeOverlayContour, NodeOverlayShapes, RoadSurfaceBandKind, RoadSurfaceSystem};
use super::seams::ConstraintOverlapMode;
use super::seams::{owned_shape_is_discardable_numeric_dust, seam_constraints_for_shape};
use super::{NodeBooleanOwnedRegion, NodeBooleanOwnershipError};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::BTreeMap;

pub(super) struct OwnedDomainResult {
    pub(super) regions: Vec<NodeBooleanOwnedRegion>,
    pub(super) claimed_shapes: NodeOverlayShapes,
}

struct OwnedDomainGroup<'a> {
    owner: NodeBandOwner,
    kind: RoadSurfaceBandKind,
    claim_priority: NodeGeneratedContourClaimPriority,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    domains: Vec<&'a NodeGeneratedContour>,
}

#[derive(Clone, Copy)]
pub(super) enum ResidualKind {
    Asphalt,
    Band(RoadSurfaceBandKind),
    NonRoad,
}

impl ResidualKind {
    fn requires_explicit_profile_seam_rail(self) -> bool {
        match self {
            ResidualKind::Band(kind) => band_kind_requires_explicit_profile_seam_rail(kind),
            ResidualKind::Asphalt | ResidualKind::NonRoad => false,
        }
    }
}

mod claiming;
mod domain_sources;
mod overlay_ops;
mod residual;

pub(super) use claiming::{owned_regions_from_domains, split_non_road_regions};
pub(super) use domain_sources::{
    asphalt_authority_domains, asphalt_owner_domains, overlay_contours_for_domains,
};
use domain_sources::{band_kind, non_road_band_order, non_road_domains_for_band_kind};
use overlay_ops::overlay_union_shape_sets;
pub(super) use overlay_ops::{
    overlay_contour_from_domain, overlay_difference, overlay_intersect, overlay_union,
};
use residual::{
    band_kind_requires_explicit_profile_seam_rail, region_has_explicit_profile_seam_rail,
};
pub(super) use residual::{
    reject_residual, sort_boolean_owned_regions,
    validate_non_road_regions_have_explicit_profile_seam_rails,
};
