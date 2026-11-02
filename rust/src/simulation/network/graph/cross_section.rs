// SPDX-License-Identifier: GPL-2.0-only

//! An edge's cross-section: where its two directional lane groups sit inside the centred
//! carriageway (`ROAD-100`), the median between them (`ROAD-80`), and the lane counts and median
//! edited in place (`ROAD-80`). The median's geometry is the roadbed's (`roadbed::profile`).

use super::{Edge, RegionGraph};
use crate::config;
use crate::simulation::network::types::{TransitFlags, TransitType, VehicleFrontageAccess};

/// Most vehicle lanes one direction of a road carries, as the road tool offers them.
pub(crate) const MAX_LANES_PER_DIRECTION: u8 = 4;

/// Surface treatment of a road's median.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MedianStyle {
    /// Flush asphalt between the lane groups, marked with hatching; frontage turns may cross
    /// it. It adds no band to the roadbed.
    Painted = 1,
    /// A raised grass strip between kerbs.
    KerbedGrass = 2,
    /// A raised paved strip between kerbs.
    KerbedPaving = 3,
}

impl MedianStyle {
    /// Every style in tool-code order (1, 2, 3).
    pub const ALL: [Self; 3] = [Self::Painted, Self::KerbedGrass, Self::KerbedPaving];
}

/// A road's median: its style and its total width in metres, kerbs included. The lane groups sit
/// that far apart and `Edge::width` includes it. Noses, clearances and the lanes' offsets are
/// derived from it, never stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadMedian {
    /// Surface and access treatment.
    pub style: MedianStyle,
    /// Total separator width, its two kerbs included when raised.
    pub width_m: f32,
}

impl RoadMedian {
    /// The median a tool or save code names: style 0 is none, 1 painted, 2 kerbed grass, 3
    /// kerbed paving, at `width_m`, which must lie in the style's range (no clamping).
    pub fn from_tool(style: i32, width_m: f32) -> Result<Option<Self>, &'static str> {
        let style = match style {
            0 => return Ok(None),
            1 => MedianStyle::Painted,
            2 => MedianStyle::KerbedGrass,
            3 => MedianStyle::KerbedPaving,
            _ => return Err("unknown_median_style"),
        };
        let median = Self { style, width_m };
        median.validate()?;
        Ok(Some(median))
    }

    /// The style's save and tool code: [`Self::from_tool`]'s inverse; 0 for no median.
    pub fn style_code(median: Option<Self>) -> i32 {
        median.map_or(0, |median| median.style as i32)
    }

    /// The width range of each style, in metres: painted 0.5-2, kerbed grass 2-12, kerbed paving
    /// 1-3.
    pub fn width_range_m(style: MedianStyle) -> (f32, f32) {
        match style {
            MedianStyle::Painted => (0.5, 2.0),
            MedianStyle::KerbedGrass => (2.0, 12.0),
            MedianStyle::KerbedPaving => (1.0, 3.0),
        }
    }

    /// The width the road tool and the cross-section controls give a style when the player picks
    /// it, in metres: painted 1, kerbed grass 3, kerbed paving 2, each inside
    /// [`Self::width_range_m`].
    pub fn default_width_m(style: MedianStyle) -> f32 {
        match style {
            MedianStyle::Painted => 1.0,
            MedianStyle::KerbedGrass => 3.0,
            MedianStyle::KerbedPaving => 2.0,
        }
    }

    /// Checks a finite width inside the style's range.
    pub fn validate(self) -> Result<(), &'static str> {
        let (min, max) = Self::width_range_m(self.style);
        if !self.width_m.is_finite() || !(min..=max).contains(&self.width_m) {
            return Err("median_width_out_of_range");
        }
        Ok(())
    }

    /// Whether the median is a kerbed strip: a roadbed band that no vehicle crosses.
    pub fn is_raised(self) -> bool {
        self.style != MedianStyle::Painted
    }
}

impl Edge {
    /// Lateral offset, in the edge's forward frame, of the line between its backward and forward
    /// lane groups: zero when both directions carry equally many lanes, else half a lane per
    /// extra lane towards the direction with fewer, so the lanes fill the carriageway the
    /// profile centres on the centreline (`ROAD-100`), where the markings already divide it.
    /// A 2+1 road's lanes sit at -3.5, 0 and 3.5 m, a one-way lane on the centreline. A median
    /// is centred on this line. `O(1)`, allocation-free.
    pub fn lane_group_offset(&self) -> f32 {
        let side = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
        (f32::from(self.bkw_lanes) - f32::from(self.fwd_lanes)) * config::LANE_WIDTH * 0.5 * side
    }

    /// How far each lane group sits out from [`Self::lane_group_offset`]: half the median's
    /// width, painted or raised, zero without one. `O(1)`, allocation-free.
    pub fn median_half_width(&self) -> f32 {
        self.median.map_or(0.0, |median| median.width_m * 0.5)
    }

    /// The frontage access buildings along the edge get: a raised median keeps cars on their
    /// own carriageway, whatever the stored policy (which survives the median's removal).
    pub fn effective_vehicle_frontage_access(&self) -> VehicleFrontageAccess {
        if self.median.is_some_and(RoadMedian::is_raised) {
            VehicleFrontageAccess::SameSideOnly
        } else {
            self.vehicle_frontage_access
        }
    }

    /// The carriageway width a road of `fwd` + `bkw` lanes and `median` has, as a stroke builds
    /// it (`network::build_surface_edge`): a lane width each plus the median, at least 2 m.
    pub(crate) fn cross_section_width_m(fwd: u8, bkw: u8, median: Option<RoadMedian>) -> f32 {
        (f32::from(fwd + bkw) * config::LANE_WIDTH + median.map_or(0.0, |median| median.width_m))
            .max(2.0)
    }

    /// Whether this edge may carry `fwd` forward and `bkw` backward lanes and `median` after an
    /// in-place edit (`ROAD-80`): it must be a live road (a footpath stays a footpath), keep at
    /// least one lane and stay within [`MAX_LANES_PER_DIRECTION`] per direction; a median must
    /// be valid, on a car road, between two directions. The error is the stable refusal key.
    pub(crate) fn validate_cross_section(
        &self,
        fwd: u8,
        bkw: u8,
        median: Option<RoadMedian>,
    ) -> Result<(), &'static str> {
        if self.deleted || self.primary_type != TransitType::Road {
            return Err("cross_section_not_road");
        }
        if fwd > MAX_LANES_PER_DIRECTION || bkw > MAX_LANES_PER_DIRECTION {
            return Err("too_many_lanes");
        }
        if fwd == 0 && bkw == 0 {
            return Err("cross_section_cannot_change_transit_mode");
        }
        Self::validate_median(self.allowed_types, fwd, bkw, median)
    }

    /// The median rules alone, for an edge of `allowed_types` with `fwd` + `bkw` lanes: valid
    /// dimensions, a car road, both directions (`ROAD-92`: a walkway has no median). Strokes,
    /// in-place edits and loads share them.
    pub(crate) fn validate_median(
        allowed_types: u8,
        fwd: u8,
        bkw: u8,
        median: Option<RoadMedian>,
    ) -> Result<(), &'static str> {
        let Some(median) = median else {
            return Ok(());
        };
        median.validate()?;
        if allowed_types & TransitFlags::CAR == 0 {
            return Err("median_requires_vehicle_road");
        }
        if fwd == 0 || bkw == 0 {
            return Err("median_requires_both_directions");
        }
        Ok(())
    }
}

impl RegionGraph {
    /// Gives edge `id` `fwd` forward and `bkw` backward lanes, `median`, and the width they take,
    /// keeping its centreline, id, class and everything else (`ROAD-80`). The cross-section must
    /// pass [`Edge::validate_cross_section`]. The spatial index reads the centreline only, so it
    /// is kept. Callers re-lay out both end nodes and rebuild their lanes, as `SimCore`'s
    /// in-place edit does. `O(1)`.
    pub(crate) fn set_edge_cross_section(
        &mut self,
        id: usize,
        fwd: u8,
        bkw: u8,
        median: Option<RoadMedian>,
    ) {
        let edge = self.edge_mut(id);
        debug_assert_eq!(edge.validate_cross_section(fwd, bkw, median), Ok(()));
        edge.fwd_lanes = fwd;
        edge.bkw_lanes = bkw;
        edge.median = median;
        edge.width = Edge::cross_section_width_m(fwd, bkw, median);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_validation_refuses_one_way_footpaths_and_invalid_dimensions() {
        let edge = Edge {
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            ..Default::default()
        };
        let median = RoadMedian::from_tool(2, 5.0).unwrap();
        assert_eq!(
            edge.validate_cross_section(1, 0, median),
            Err("median_requires_both_directions")
        );
        assert_eq!(
            edge.validate_cross_section(0, 1, median),
            Err("median_requires_both_directions")
        );
        assert!(edge.validate_cross_section(1, 1, median).is_ok());
        assert_eq!(
            Edge::validate_median(TransitFlags::FOOT, 1, 1, median),
            Err("median_requires_vehicle_road")
        );
        let foot = Edge {
            primary_type: TransitType::Foot,
            allowed_types: TransitFlags::FOOT,
            ..Default::default()
        };
        assert_eq!(
            foot.validate_cross_section(0, 0, median),
            Err("cross_section_not_road")
        );
        assert!(RoadMedian::from_tool(1, f32::NAN).is_err());
        assert!(RoadMedian::from_tool(3, 0.5).is_err());
        assert_eq!(RoadMedian::from_tool(4, 1.0), Err("unknown_median_style"));
        assert_eq!(RoadMedian::from_tool(0, 99.0), Ok(None));
        for style in 1..=3 {
            let median = RoadMedian::from_tool(style, 2.0).unwrap();
            assert_eq!(RoadMedian::style_code(median), style);
        }
    }

    #[test]
    fn each_style_defaults_to_a_width_it_accepts() {
        for style in MedianStyle::ALL {
            let width_m = RoadMedian::default_width_m(style);
            let median = RoadMedian::from_tool(style as i32, width_m)
                .unwrap()
                .unwrap();
            assert_eq!(median.style, style);
        }
    }

    #[test]
    fn a_median_widens_the_road_and_parts_the_lane_groups() {
        let mut edge = Edge {
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            fwd_lanes: 2,
            bkw_lanes: 2,
            ..Default::default()
        };
        assert_eq!(edge.median_half_width(), 0.0);
        assert_eq!(Edge::cross_section_width_m(2, 2, None), 14.0);
        let grass = RoadMedian::from_tool(2, 4.0).unwrap();
        assert_eq!(Edge::cross_section_width_m(2, 2, grass), 18.0);
        edge.median = grass;
        assert_eq!(edge.median_half_width(), 2.0);
        assert_eq!(
            edge.effective_vehicle_frontage_access(),
            VehicleFrontageAccess::SameSideOnly
        );
        edge.median = RoadMedian::from_tool(1, 1.0).unwrap();
        assert_eq!(edge.median_half_width(), 0.5);
        assert_eq!(
            edge.effective_vehicle_frontage_access(),
            edge.vehicle_frontage_access
        );
    }
}
