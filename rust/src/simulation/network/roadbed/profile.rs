// SPDX-License-Identifier: GPL-2.0-only

//! Cross-section profiles: ordered lateral bands with widths and levels, and the roadbed
//! half-width rule derived from them.
//!
//! The band rule is the legacy one, unchanged: `docs/roads.md` "Cross-section profile".

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
// A footpath is at least this wide.
const FOOTPATH_MIN_WIDTH_M: f32 = 2.0;
// Sidewalk, curb, two carriageway halves, curb, sidewalk.
pub(super) const MAX_PROFILE_BANDS: usize = 6;

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

/// Vertical level of a band relative to the section's base surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Level {
    /// On the base surface: carriageway and footpath.
    Ground,
    /// [`CURB_STEP_HEIGHT_M`] above the base surface: curb, shoulder and sidewalk.
    Raised,
}

impl Level {
    /// Height of this level above the base surface, in metres.
    pub fn offset_m(self) -> f32 {
        match self {
            Self::Ground => 0.0,
            Self::Raised => CURB_STEP_HEIGHT_M,
        }
    }
}

/// One band of a [`RoadProfile`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileBand {
    /// Band classification.
    pub kind: BandKind,
    /// Signed lateral start offset from the centerline, in metres; negative is left.
    pub start_m: f32,
    /// Signed lateral end offset from the centerline, in metres; greater than `start_m`.
    pub end_m: f32,
    /// Vertical level of the whole band.
    pub level: Level,
}

/// Ordered lateral bands of an edge's cross-section, left to right in the edge's forward frame.
///
/// Fixed capacity, so building a profile never allocates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadProfile {
    bands: [ProfileBand; MAX_PROFILE_BANDS],
    len: u8,
}

impl RoadProfile {
    /// The profile of an edge at its full carriageway width.
    pub fn for_edge(edge: &Edge) -> Self {
        Self::with_carriageway_half_width(edge, None)
    }

    /// The profile of an edge with its carriageway half-width overridden, as width-change tapers
    /// do; the override is floored at half a lane. Footpaths ignore it.
    pub(crate) fn with_carriageway_half_width(
        edge: &Edge,
        carriageway_half_width_override_m: Option<f32>,
    ) -> Self {
        let mut profile = Self {
            bands: [ProfileBand {
                kind: BandKind::Carriageway,
                start_m: 0.0,
                end_m: 0.0,
                level: Level::Ground,
            }; MAX_PROFILE_BANDS],
            len: 0,
        };
        let mut push = |kind, start_m, end_m, level| {
            profile.bands[usize::from(profile.len)] = ProfileBand {
                kind,
                start_m,
                end_m,
                level,
            };
            profile.len += 1;
        };

        if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
            let half_width = edge.width.max(FOOTPATH_MIN_WIDTH_M) * 0.5;
            push(BandKind::Footpath, -half_width, half_width, Level::Ground);
            return profile;
        }

        let half_carriageway = carriageway_half_width_override_m
            .unwrap_or_else(|| edge.width.max(config::LANE_WIDTH) * 0.5)
            .max(config::LANE_WIDTH * 0.5);
        let has_sidewalk = edge.allowed_types & TransitFlags::FOOT != 0;
        let sidewalk_total = if has_sidewalk {
            config::SIDEWALK_WIDTH
        } else {
            0.0
        };
        let curb_width = if has_sidewalk {
            CURB_BAND_WIDTH_M.min(sidewalk_total)
        } else {
            SHOULDER_BAND_WIDTH_M
        };
        let sidewalk_width = (sidewalk_total - curb_width).max(0.0);
        // A zero-width curb would leave the raised bands at ground level (legacy rule).
        let raised = if curb_width > 0.0 {
            Level::Raised
        } else {
            Level::Ground
        };
        if sidewalk_width > 0.0 {
            push(
                BandKind::Sidewalk,
                -(half_carriageway + curb_width + sidewalk_width),
                -(half_carriageway + curb_width),
                raised,
            );
        }
        push(
            BandKind::CurbOrShoulder,
            -(half_carriageway + curb_width),
            -half_carriageway,
            raised,
        );
        push(BandKind::Carriageway, -half_carriageway, 0.0, Level::Ground);
        push(BandKind::Carriageway, 0.0, half_carriageway, Level::Ground);
        push(
            BandKind::CurbOrShoulder,
            half_carriageway,
            half_carriageway + curb_width,
            raised,
        );
        if sidewalk_width > 0.0 {
            push(
                BandKind::Sidewalk,
                half_carriageway + curb_width,
                half_carriageway + curb_width + sidewalk_width,
                raised,
            );
        }
        profile
    }

    /// Bands, left to right.
    pub fn bands(&self) -> &[ProfileBand] {
        &self.bands[..usize::from(self.len)]
    }

    /// Distance from the centerline to the outermost band boundary, in metres.
    pub fn half_width_m(&self) -> f32 {
        self.bands()
            .iter()
            .map(|band| band.start_m.abs().max(band.end_m.abs()))
            .fold(0.0, f32::max)
    }

    /// Distance from the centerline to the outer carriageway boundary, in metres; zero for a
    /// footpath.
    pub fn carriageway_half_width_m(&self) -> f32 {
        self.bands()
            .iter()
            .filter(|band| band.kind == BandKind::Carriageway)
            .map(|band| band.start_m.abs().max(band.end_m.abs()))
            .fold(0.0, f32::max)
    }
}

/// Returns the complete visible roadbed half-width, including curb or shoulder and sidewalk.
pub(crate) fn visual_roadbed_half_width_m(edge: &Edge) -> f32 {
    RoadProfile::for_edge(edge).half_width_m()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(width: f32, allowed_types: u8, primary_type: TransitType) -> Edge {
        Edge {
            width,
            allowed_types,
            primary_type,
            ..Edge::default()
        }
    }

    #[test]
    fn profile_table_matches_the_legacy_band_rule() {
        let road = edge(
            7.0,
            TransitFlags::CAR | TransitFlags::FOOT,
            TransitType::Road,
        );
        let bands = RoadProfile::for_edge(&road);
        let expected = [
            (BandKind::Sidewalk, -5.0, -3.65, Level::Raised),
            (BandKind::CurbOrShoulder, -3.65, -3.5, Level::Raised),
            (BandKind::Carriageway, -3.5, 0.0, Level::Ground),
            (BandKind::Carriageway, 0.0, 3.5, Level::Ground),
            (BandKind::CurbOrShoulder, 3.5, 3.65, Level::Raised),
            (BandKind::Sidewalk, 3.65, 5.0, Level::Raised),
        ];
        assert_eq!(bands.bands().len(), expected.len());
        for (band, (kind, start, end, level)) in bands.bands().iter().zip(expected) {
            assert_eq!((band.kind, band.level), (kind, level));
            assert!((band.start_m - start).abs() < 1e-6 && (band.end_m - end).abs() < 1e-6);
        }
        assert_eq!(bands.carriageway_half_width_m(), 3.5);

        let rural = edge(7.0, TransitFlags::CAR, TransitType::Road);
        let kinds: Vec<_> = RoadProfile::for_edge(&rural)
            .bands()
            .iter()
            .map(|band| (band.kind, band.level))
            .collect();
        assert_eq!(
            kinds,
            [
                (BandKind::CurbOrShoulder, Level::Raised),
                (BandKind::Carriageway, Level::Ground),
                (BandKind::Carriageway, Level::Ground),
                (BandKind::CurbOrShoulder, Level::Raised),
            ]
        );
        assert_eq!(RoadProfile::for_edge(&rural).half_width_m(), 4.25);

        let path = edge(1.0, TransitFlags::FOOT, TransitType::Foot);
        let path = RoadProfile::for_edge(&path);
        assert_eq!(path.bands().len(), 1);
        assert_eq!(path.bands()[0].kind, BandKind::Footpath);
        assert_eq!(path.half_width_m(), 1.0);
        assert_eq!(path.carriageway_half_width_m(), 0.0);
    }

    #[test]
    fn half_width_keeps_the_legacy_sum_bit_for_bit() {
        for width in [0.0, 3.5, 7.0, 7.3, 10.5, 14.0] {
            let road = edge(
                width,
                TransitFlags::CAR | TransitFlags::FOOT,
                TransitType::Road,
            );
            let half_carriageway =
                (road.width.max(config::LANE_WIDTH) * 0.5).max(config::LANE_WIDTH * 0.5);
            let curb = CURB_BAND_WIDTH_M.min(config::SIDEWALK_WIDTH);
            let legacy = half_carriageway + curb + (config::SIDEWALK_WIDTH - curb).max(0.0);
            assert_eq!(
                visual_roadbed_half_width_m(&road).to_bits(),
                legacy.to_bits()
            );
        }
    }
}
