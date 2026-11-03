// SPDX-License-Identifier: GPL-2.0-only

//! Cross-section profiles: ordered lateral bands with widths and levels, and the roadbed
//! half-width rule derived from them.
//!
//! The band rule is the legacy one, unchanged: `docs/roads.md` "Cross-section profile".

use crate::config;
use crate::simulation::network::build_surface_edge;
use crate::simulation::network::graph::{Edge, RoadMedian};
use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
use godot::prelude::Vector3;

/// Height of raised bands (curb, shoulder, sidewalk) above the carriageway, in metres.
pub const CURB_STEP_HEIGHT_M: f32 = 0.12;

// Lateral widths of the raised bands beside the carriageway.
const CURB_BAND_WIDTH_M: f32 = 0.15;
const SHOULDER_BAND_WIDTH_M: f32 = 0.75;
// A footpath is at least this wide: a walkway's whole width (`Edge::cross_section_width_m`).
pub(crate) const FOOTPATH_MIN_WIDTH_M: f32 = 2.0;
/// Width of each kerb of a raised median at its authored width (`ROAD-80`), inside the median's
/// width; a nose scales the kerbs with the median.
pub const MEDIAN_KERB_WIDTH_M: f32 = 0.15;
// Sidewalk, curb, carriageway, kerb, median, kerb, carriageway, curb, sidewalk: a divided body
// (`RoadProfile::body_for_edge`); every other profile has at most six bands.
pub(super) const MAX_PROFILE_BANDS: usize = 9;

/// Lateral surface-band kinds of a road cross-section.
///
/// `Carriageway`, `CurbOrShoulder`, `Sidewalk` and `Footpath` are emitted, and `Median` on a
/// divided span's body (`ROAD-80`); the others are reserved for explicit future bands.
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
    /// The raised strip of a kerbed median between its two kerbs (`ROAD-80`).
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
#[derive(Clone, Copy, PartialEq)]
pub struct RoadProfile {
    bands: [ProfileBand; MAX_PROFILE_BANDS],
    len: u8,
}

// Lists at least six band slots, as the fixed array of six did before divided bodies
// (`ROAD-80`), so the text the product probes hash is unchanged for every other profile.
impl std::fmt::Debug for RoadProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let listed = usize::from(self.len).max(6);
        f.debug_struct("RoadProfile")
            .field("bands", &&self.bands[..listed])
            .field("len", &self.len)
            .finish()
    }
}

impl RoadProfile {
    /// The profile of an edge at its full carriageway width.
    pub fn for_edge(edge: &Edge) -> Self {
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

        let half_carriageway =
            (edge.width.max(config::LANE_WIDTH) * 0.5).max(config::LANE_WIDTH * 0.5);
        let (sidewalk_total, curb_width) = if edge.allowed_types & TransitFlags::FOOT != 0 {
            let total = config::SIDEWALK_WIDTH;
            (total, CURB_BAND_WIDTH_M.min(total))
        } else {
            (0.0, SHOULDER_BAND_WIDTH_M)
        };
        let sidewalk_width = (sidewalk_total - curb_width).max(0.0);
        // A zero-width curb would leave the raised bands at ground level (legacy rule).
        let raised = if curb_width > 0.0 {
            Level::Raised
        } else {
            Level::Ground
        };
        // Outer offsets of the curb or shoulder and of the sidewalk, mirrored on the left.
        let curb_m = half_carriageway + curb_width;
        let sidewalk_m = half_carriageway + curb_width + sidewalk_width;
        if sidewalk_width > 0.0 {
            push(BandKind::Sidewalk, -sidewalk_m, -curb_m, raised);
        }
        push(BandKind::CurbOrShoulder, -curb_m, -half_carriageway, raised);
        push(BandKind::Carriageway, -half_carriageway, 0.0, Level::Ground);
        push(BandKind::Carriageway, 0.0, half_carriageway, Level::Ground);
        push(BandKind::CurbOrShoulder, half_carriageway, curb_m, raised);
        if sidewalk_width > 0.0 {
            push(BandKind::Sidewalk, curb_m, sidewalk_m, raised);
        }
        profile
    }

    /// The body profile of a divided span (`ROAD-80`) whose raised median is `median_m` wide
    /// (from zero at a nose's tip up to the authored width): [`Self::for_edge`]'s bands with the
    /// split between the two carriageway bands opened into kerb, median and kerb, centred on the
    /// line between the lane groups ([`Edge::lane_group_offset`]). The kerbs are
    /// [`MEDIAN_KERB_WIDTH_M`] at the authored width and scale with the median, so every band
    /// has width for any width above zero and all three collapse onto the line at zero. The
    /// offsets mirror exactly: the edge reversed gives the mirrored profile bit for bit, so a
    /// pass-through pair's spans share their row. Only for an edge with a raised median.
    /// `O(B)`, no allocation.
    pub fn body_for_edge(edge: &Edge, median_m: f32) -> Self {
        let mouth = Self::for_edge(edge);
        let authored_m = edge.median.map_or(0.0, |median| median.width_m);
        debug_assert!(
            edge.median.is_some_and(|median| median.is_raised()),
            "a body profile needs a raised median"
        );
        let split = mouth
            .bands()
            .iter()
            .position(|band| band.kind == BandKind::Carriageway)
            .expect("a car road has a carriageway");
        let centre_m = edge.lane_group_offset();
        let half_m = median_m * 0.5;
        let kerb_m = if authored_m > 0.0 {
            MEDIAN_KERB_WIDTH_M * (median_m / authored_m)
        } else {
            0.0
        };
        // Left to right: the inner edges of the two carriageways, then the median's own edges.
        let (left_m, right_m) = (centre_m - half_m, centre_m + half_m);
        let (median_start_m, median_end_m) = (left_m + kerb_m, right_m - kerb_m);
        let mut profile = mouth;
        profile.len = 0;
        let mut push = |band: ProfileBand| {
            profile.bands[usize::from(profile.len)] = band;
            profile.len += 1;
        };
        for (index, band) in mouth.bands().iter().enumerate() {
            if index == split {
                push(ProfileBand {
                    end_m: left_m,
                    ..*band
                });
                for (kind, start_m, end_m) in [
                    (BandKind::CurbOrShoulder, left_m, median_start_m),
                    (BandKind::Median, median_start_m, median_end_m),
                    (BandKind::CurbOrShoulder, median_end_m, right_m),
                ] {
                    push(ProfileBand {
                        kind,
                        start_m,
                        end_m,
                        level: Level::Raised,
                    });
                }
            } else if index == split + 1 {
                push(ProfileBand {
                    start_m: right_m,
                    ..*band
                });
            } else {
                push(*band);
            }
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

/// Returns the visible roadbed half-width for one not-yet-committed road candidate with
/// `median` (`ROAD-80`: the median widens the corridor every stroke check reads).
pub(crate) fn preview_candidate_roadbed_half_width_m(
    prepared_points: &[Vector3],
    edge_class: EdgeClass,
    fwd_lanes: u8,
    bkw_lanes: u8,
    median: Option<RoadMedian>,
) -> f32 {
    let points = match (prepared_points.first(), prepared_points.last()) {
        (Some(first), Some(last)) => vec![*first, *last],
        _ => Vec::new(),
    };
    let edge = build_surface_edge(0, 1, points, fwd_lanes, bkw_lanes, edge_class, median);
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

    // `ROAD-80`: a kerbed median opens the split between the carriageways into kerb, median and
    // kerb around the line between the lane groups; its outer bands are the mouth profile's.
    #[test]
    fn a_body_profile_opens_the_split_into_kerbs_and_a_median() {
        let mut road = edge(
            17.5,
            TransitFlags::CAR | TransitFlags::FOOT,
            TransitType::Road,
        );
        road.fwd_lanes = 2;
        road.bkw_lanes = 1;
        road.median = crate::simulation::network::graph::RoadMedian::from_tool(2, 7.0).unwrap();
        let mouth = RoadProfile::for_edge(&road);
        let body = RoadProfile::body_for_edge(&road, 7.0);
        let centre = road.lane_group_offset();
        assert_eq!(centre, -1.75);
        let expected = [
            (BandKind::Sidewalk, -10.25, -8.9, Level::Raised),
            (BandKind::CurbOrShoulder, -8.9, -8.75, Level::Raised),
            (BandKind::Carriageway, -8.75, -5.25, Level::Ground),
            (BandKind::CurbOrShoulder, -5.25, -5.1, Level::Raised),
            (BandKind::Median, -5.1, 1.6, Level::Raised),
            (BandKind::CurbOrShoulder, 1.6, 1.75, Level::Raised),
            (BandKind::Carriageway, 1.75, 8.75, Level::Ground),
            (BandKind::CurbOrShoulder, 8.75, 8.9, Level::Raised),
            (BandKind::Sidewalk, 8.9, 10.25, Level::Raised),
        ];
        assert_eq!(body.bands().len(), expected.len());
        for (band, (kind, start, end, level)) in body.bands().iter().zip(expected) {
            assert_eq!((band.kind, band.level), (kind, level));
            assert!(
                (band.start_m - start).abs() < 1e-5 && (band.end_m - end).abs() < 1e-5,
                "{band:?}"
            );
        }
        assert_eq!(body.half_width_m().to_bits(), mouth.half_width_m().to_bits());
        assert_eq!(
            body.carriageway_half_width_m().to_bits(),
            mouth.carriageway_half_width_m().to_bits()
        );
        // At a nose's tip the three median bands collapse onto the split, bit for bit.
        let tip = RoadProfile::body_for_edge(&road, 0.0);
        for band in &tip.bands()[3..6] {
            assert_eq!((band.start_m, band.end_m), (centre, centre));
        }
        // The reversed edge gives the mirrored profile exactly (a pass-through pair's row).
        let mut reversed = road.clone();
        (reversed.fwd_lanes, reversed.bkw_lanes) = (1, 2);
        for width in [0.0, 1.75, 3.3, 7.0] {
            let forward = RoadProfile::body_for_edge(&road, width);
            let backward = RoadProfile::body_for_edge(&reversed, width);
            for (a, b) in forward.bands().iter().zip(backward.bands().iter().rev()) {
                assert_eq!(
                    (a.kind, a.level, a.start_m.to_bits(), a.end_m.to_bits()),
                    (b.kind, b.level, (-b.end_m).to_bits(), (-b.start_m).to_bits())
                );
            }
        }
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
