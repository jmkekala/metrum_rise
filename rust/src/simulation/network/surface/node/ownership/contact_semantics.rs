// SPDX-License-Identifier: GPL-2.0-only

//! Owner-pair contact semantics for ownership seam policy.

use super::super::RoadSurfaceBandKind;
use super::super::arrangement::NodeBandOwner;
use super::super::band_semantics::{
    raised_step_kinds_can_contact, raised_step_requires_exact_constraint_span,
};

pub(super) fn owners_form_raised_step_contact(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    raised_step_kinds_can_contact(owner.kind(), opposite_owner.kind())
}

pub(super) fn raised_step_contact_requires_exact_constraint_span(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    raised_step_requires_exact_constraint_span(owner.kind(), opposite_owner.kind())
}

pub(super) fn raised_step_contact_constrains_shared_height(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    owners_form_raised_step_contact(owner, opposite_owner)
        && !raised_step_contact_requires_exact_constraint_span(owner, opposite_owner)
}

pub(super) fn band_boundary_constrains_shared_height(
    left: RoadSurfaceBandKind,
    right: RoadSurfaceBandKind,
) -> bool {
    matches!(
        (left, right),
        (RoadSurfaceBandKind::Sidewalk, RoadSurfaceBandKind::Footpath)
            | (RoadSurfaceBandKind::Footpath, RoadSurfaceBandKind::Sidewalk)
    )
}
