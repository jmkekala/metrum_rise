// SPDX-License-Identifier: GPL-2.0-only

//! Cross-section band kinds, the raised-band step and the roadbed half-width rule.
//!
//! The half-width helpers keep the legacy band rule bit for bit; `ROAD-68-B` replaces them with
//! the full profile table in `docs/roads.md`.

use crate::config;
use crate::simulation::network::build_surface_edge;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
use godot::prelude::Vector3;

/// Height of raised bands (curb, shoulder, sidewalk) above the carriageway, in metres.
pub const CURB_STEP_HEIGHT_M: f32 = 0.12;

// Lateral widths of the raised bands beside the carriageway.
const CURB_BAND_WIDTH_M: f32 = 0.15;
const SHOULDER_BAND_WIDTH_M: f32 = 0.75;

/// Lateral surface-band kinds of a road cross-section.
///
/// Only `Carriageway`, `CurbOrShoulder`, `Sidewalk` and `Footpath` are emitted; the others are
/// reserved for explicit future bands.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BandKind {
    /// Drivable carriageway at ground level.
    Carriageway,
    /// Raised curb, or the raised shoulder of a road without sidewalks.
    CurbOrShoulder,
    /// Raised roadside sidewalk.
    Sidewalk,
    /// Pedestrian path that is not a roadside sidewalk, at ground level.
    Footpath,
    /// Reserved central median or separator.
    Median,
    /// Reserved parking band.
    Parking,
    /// Reserved bicycle band.
    CycleTrack,
    /// Reserved tram corridor.
    TramReservation,
}

/// Returns the complete visible roadbed half-width, including curb or shoulder and sidewalk.
pub(crate) fn visual_roadbed_half_width_m(edge: &Edge) -> f32 {
    if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
        return edge.width.max(2.0) * 0.5;
    }
    let half_carriageway = (edge.width.max(config::LANE_WIDTH) * 0.5).max(config::LANE_WIDTH * 0.5);
    if edge.allowed_types & TransitFlags::FOOT == 0 {
        return half_carriageway + SHOULDER_BAND_WIDTH_M;
    }
    let curb_width = CURB_BAND_WIDTH_M.min(config::SIDEWALK_WIDTH);
    let sidewalk_width = (config::SIDEWALK_WIDTH - curb_width).max(0.0);
    // The outermost band boundary, summed in the legacy band order.
    if sidewalk_width > 0.0 {
        half_carriageway + curb_width + sidewalk_width
    } else {
        half_carriageway + curb_width
    }
}

/// Returns the visible roadbed half-width for one not-yet-committed road candidate.
pub(crate) fn preview_candidate_roadbed_half_width_m(
    prepared_points: &[Vector3],
    edge_class: EdgeClass,
    fwd_lanes: u8,
    bkw_lanes: u8,
) -> f32 {
    let points = match (prepared_points.first(), prepared_points.last()) {
        (Some(first), Some(last)) => vec![*first, *last],
        _ => Vec::new(),
    };
    let edge = build_surface_edge(0, 1, points, fwd_lanes, bkw_lanes, edge_class);
    visual_roadbed_half_width_m(&edge)
}
