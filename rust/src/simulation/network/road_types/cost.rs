// SPDX-License-Identifier: GPL-2.0-only

//! What a road costs (`ROAD-101`): one per-metre weight in 2-lane-road units, so the built-in
//! 2-Lane Road at 50 km/h on the ground weighs exactly 1.0, builds for exactly
//! [`BUILD_COST_PER_M`] and costs exactly [`UPKEEP_PER_M_DAY`] a day (the constants every road
//! cost before road types). Lanes and medians scale with speed, sidewalks add, a structure
//! multiplies the whole weight, zoning costs nothing:
//!
//! ```text
//! weight = structure x (lanes x 0.4 x f(v) + median x f(v) + sidewalks)
//!   f(v)      = 1 + (v_kmh - 50) / 50
//!   median    = painted 0.05; kerbed grass or paving 0.1 + 0.1 x width_m
//!   sidewalks = 0.2 (both) or 0
//!   structure = standard 1, bridge 3, tunnel 5
//!   walkway   = structure x 0.25
//! ```
//!
//! Stroke previews and commits, in-place type changes, the road builder's display and the
//! weighted daily upkeep roll all read it. Engine-free; `O(1)`, allocation-free.

use super::{RoadAttributes, RoadCrossSection};
use crate::simulation::network::graph::{Edge, MedianStyle, RoadMedian};
use crate::simulation::network::types::EdgeClass;

/// Build cost per metre of a weight-1.0 road (the 2-Lane Road at 50 km/h on the ground).
pub const BUILD_COST_PER_M: f64 = 15.0;
/// Upkeep per metre and day of a weight-1.0 road.
pub const UPKEEP_PER_M_DAY: f64 = 0.1;

// Each car lane's weight at 50 km/h.
const LANE_WEIGHT: f64 = 0.4;
// Both sidewalks together.
const SIDEWALKS_WEIGHT: f64 = 0.2;
// A walkway's footpath, before its structure factor.
const WALKWAY_WEIGHT: f64 = 0.25;
// A painted median, whatever its width.
const PAINTED_MEDIAN_WEIGHT: f64 = 0.05;
// A kerbed median: its kerbs, then each metre of its width.
const RAISED_MEDIAN_WEIGHT: f64 = 0.1;
const RAISED_MEDIAN_WEIGHT_PER_M: f64 = 0.1;

/// How a road's speed scales its lanes' and median's weight: `1 + (v - 50) / 50` (30 km/h 0.6,
/// 50 exactly 1.0, 80 1.6, 120 2.4).
pub fn speed_factor(speed_kmh: u16) -> f64 {
    1.0 + (f64::from(speed_kmh) - 50.0) / 50.0
}

/// How a structure multiplies a road's whole weight: 1 on the ground, 3 on a bridge, 5 in a
/// tunnel.
pub fn structure_factor(class: EdgeClass) -> f64 {
    match class {
        EdgeClass::Standard => 1.0,
        EdgeClass::Bridge => 3.0,
        EdgeClass::Tunnel => 5.0,
    }
}

// A median's weight at 50 km/h.
fn median_weight(median: RoadMedian) -> f64 {
    match median.style {
        MedianStyle::Painted => PAINTED_MEDIAN_WEIGHT,
        MedianStyle::KerbedGrass | MedianStyle::KerbedPaving => {
            RAISED_MEDIAN_WEIGHT + RAISED_MEDIAN_WEIGHT_PER_M * f64::from(median.width_m)
        }
    }
}

/// The per-metre weight of a road of `section` at `speed_kmh` (ignored for a walkway) and of
/// `class`, by the rule in the module docs.
pub fn road_weight(section: RoadCrossSection, speed_kmh: u16, class: EdgeClass) -> f64 {
    let structure = structure_factor(class);
    if section.is_walkway() {
        return structure * WALKWAY_WEIGHT;
    }
    let speed = speed_factor(speed_kmh);
    let lanes = f64::from(section.fwd_lanes + section.bkw_lanes) * LANE_WEIGHT * speed;
    let median = section
        .median
        .map_or(0.0, |median| median_weight(median) * speed);
    let sidewalks = if section.sidewalks {
        SIDEWALKS_WEIGHT
    } else {
        0.0
    };
    structure * (lanes + median + sidewalks)
}

/// The weight of a stroke of `fwd` + `bkw` lanes, `median` and `attributes`, as its prepared
/// `class` builds it: the edge [`Edge::cost_weight`] then reads back.
pub fn stroke_weight(
    (fwd, bkw): (u8, u8),
    median: Option<RoadMedian>,
    attributes: RoadAttributes,
    class: EdgeClass,
) -> f64 {
    let section = RoadCrossSection {
        fwd_lanes: fwd,
        bkw_lanes: bkw,
        median,
        sidewalks: attributes.sidewalks && fwd + bkw > 0,
    };
    road_weight(
        section,
        RoadAttributes::speed_kmh_of(attributes.speed_ms),
        class,
    )
}

/// Build cost per metre of a road of `weight`.
pub fn build_per_m(weight: f64) -> f64 {
    BUILD_COST_PER_M * weight
}

/// Upkeep per metre and day of a road of `weight`.
pub fn upkeep_per_m_day(weight: f64) -> f64 {
    UPKEEP_PER_M_DAY * weight
}

impl Edge {
    /// This road's per-metre cost weight ([`road_weight`]) from its own lanes, median, sidewalks
    /// (`FOOT` on a car road), speed limit (whole km/h) and class. `O(1)`.
    pub fn cost_weight(&self) -> f64 {
        road_weight(
            RoadCrossSection::of_edge(self),
            RoadAttributes::speed_kmh_of(self.speed_limit),
            self.class,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::road_types::BUILTIN_ROAD_TYPES;

    fn section(
        fwd: u8,
        bkw: u8,
        median: Option<(MedianStyle, f32)>,
        sidewalks: bool,
    ) -> RoadCrossSection {
        RoadCrossSection {
            fwd_lanes: fwd,
            bkw_lanes: bkw,
            median: median.map(|(style, width_m)| RoadMedian { style, width_m }),
            sidewalks,
        }
    }

    // DESIGN-RB3 section D's table with the structure factor on the whole weight.
    #[test]
    fn the_cost_table_rows() {
        use EdgeClass::{Bridge, Standard, Tunnel};
        use MedianStyle::{KerbedGrass, KerbedPaving, Painted};
        let rows: &[(&str, RoadCrossSection, u16, EdgeClass, f64)] = &[
            ("walkway", RoadCrossSection::WALKWAY, 0, Standard, 0.25),
            ("4-lane", section(2, 2, None, true), 50, Standard, 1.8),
            ("one-way 1L", section(1, 0, None, true), 50, Standard, 0.6),
            ("one-way 2L", section(2, 0, None, true), 50, Standard, 1.0),
            ("1+1 at 30", section(1, 1, None, true), 30, Standard, 0.68),
            (
                "1+1 painted 1 m",
                section(1, 1, Some((Painted, 1.0)), true),
                50,
                Standard,
                1.05,
            ),
            (
                "boulevard",
                section(2, 2, Some((KerbedGrass, 3.0)), true),
                50,
                Standard,
                2.2,
            ),
            ("rural", section(1, 1, None, false), 80, Standard, 1.28),
            (
                "highway-like",
                section(2, 2, Some((KerbedGrass, 3.0)), false),
                100,
                Standard,
                4.0,
            ),
            (
                "2+2 paving 2 m at 120",
                section(2, 2, Some((KerbedPaving, 2.0)), false),
                120,
                Standard,
                4.56,
            ),
            ("2-lane bridge", section(1, 1, None, true), 50, Bridge, 3.0),
            ("2-lane tunnel", section(1, 1, None, true), 50, Tunnel, 5.0),
            // The structure multiplies the sidewalks too: 3 x (0.8 + 0.2), 5 x (0.8 x 1.6 + 0.2).
            ("4-lane bridge", section(2, 2, None, true), 50, Bridge, 5.4),
            (
                "1+1 at 80 in a tunnel",
                section(1, 1, None, true),
                80,
                Tunnel,
                7.4,
            ),
            ("walkway bridge", RoadCrossSection::WALKWAY, 0, Bridge, 0.75),
            ("walkway tunnel", RoadCrossSection::WALKWAY, 0, Tunnel, 1.25),
        ];
        for &(name, section, speed, class, weight) in rows {
            let got = road_weight(section, speed, class);
            assert!((got - weight).abs() < 1e-12, "{name}: {got} != {weight}");
            assert!(
                (build_per_m(got) - 15.0 * weight).abs() < 1e-9,
                "{name} build"
            );
            assert!(
                (upkeep_per_m_day(got) - 0.1 * weight).abs() < 1e-12,
                "{name} upkeep"
            );
        }
        // A walkway has no speed.
        assert_eq!(
            road_weight(RoadCrossSection::WALKWAY, 120, Standard),
            road_weight(RoadCrossSection::WALKWAY, 0, Standard)
        );
    }

    // The anchor: the 2-Lane Road at 50 km/h is bitwise today's constants, so a city of them
    // builds and upkeeps exactly as before road types.
    #[test]
    fn the_two_lane_road_at_fifty_is_bitwise_the_old_constants() {
        let two_lane = BUILTIN_ROAD_TYPES[1].cross_section;
        let weight = road_weight(two_lane, 50, EdgeClass::Standard);
        assert_eq!(weight.to_bits(), 1.0f64.to_bits());
        assert_eq!(build_per_m(weight).to_bits(), 15.0f64.to_bits());
        assert_eq!(upkeep_per_m_day(weight).to_bits(), 0.1f64.to_bits());
        let stroke = stroke_weight((1, 1), None, RoadAttributes::DEFAULT, EdgeClass::Standard);
        assert_eq!(stroke.to_bits(), 1.0f64.to_bits());
        // A walkway stroke ignores the tool's sidewalks and speed.
        assert_eq!(
            stroke_weight(
                (0, 0),
                None,
                RoadAttributes::new(120, true, true),
                EdgeClass::Bridge
            ),
            0.75
        );
    }

    #[test]
    fn an_edge_reads_its_own_weight() {
        use crate::simulation::network::types::{TransitFlags, TransitType};
        let mut edge = Edge {
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: crate::config::DEFAULT_URBAN_ROAD_SPEED_MS,
            ..Default::default()
        };
        assert_eq!(edge.cost_weight().to_bits(), 1.0f64.to_bits());
        edge.allowed_types = TransitFlags::CAR;
        edge.speed_limit = RoadAttributes::new(80, false, true).speed_ms;
        assert!((edge.cost_weight() - 1.28).abs() < 1e-12);
        edge.class = EdgeClass::Bridge;
        assert!((edge.cost_weight() - 3.84).abs() < 1e-12);
    }
}
