// SPDX-License-Identifier: GPL-2.0-only

//! An edge's cross-section: where its two directional lane groups sit inside the centred
//! carriageway (`ROAD-100`), the median between them (`ROAD-80`), a turn pocket's extra lane and
//! the kerb it moves (`ROAD-95`), and the lane counts, median and road-type attributes edited in
//! place (`ROAD-80`, `ROAD-101`). The median's geometry is the roadbed's (`roadbed::profile`).

use super::{Edge, RegionGraph};
use crate::config;
use crate::simulation::network::road_types::RoadAttributes;
use crate::simulation::network::roadbed::EdgeEnd;
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::pathing::cost::CostCalculator;
use godot::prelude::Vector3;

/// Most vehicle lanes one direction of a road carries, as the road tool offers them.
pub(crate) const MAX_LANES_PER_DIRECTION: u8 = 4;

/// What a left turn pocket leaves of a raised median it narrows (`ROAD-95`, D3): a kerb stub
/// this wide at least, which may lie below the style's minimum width on the pocket edge only.
pub(crate) const POCKET_MEDIAN_STUB_M: f32 = 0.3;

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

    /// The style a tool or save code names (1 painted, 2 kerbed grass, 3 kerbed paving); `None`
    /// for any other code, 0 (no median) included.
    pub fn from_code(code: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|style| *style as i32 == code)
    }
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
        if style == 0 {
            return Ok(None);
        }
        let style = MedianStyle::from_code(style).ok_or("unknown_median_style")?;
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

/// A turn pocket at one end of an edge (`ROAD-95`): extra lanes of the direction that approaches
/// `end` (the forward lanes when `end` is [`EdgeEnd::End`]), each serving one turn at the node
/// there. The lane counts and `Edge::width` include the pocket lanes. `left` is the lane left of
/// the through lanes as its drivers see it (rank 0, `lanes::lane_rank`), `right` the lane on
/// their right. Under right-hand traffic a right pocket moves the kerb out by one lane and only
/// that kerb ([`Edge::kerb_shift_m`]); a left pocket does the same on the other kerb of a road
/// without a median, where on a two-way road the oncoming lanes move out by a lane with it (round
/// R5c1), and on a divided road takes a lane of the raised median, keeping both kerbs
/// (`Edge::median_pocket_lanes`). The edge is the short pocket edge, split off its base road at a
/// degree-two [`NodeType::LaneTransition`] node; the taper between them is the roadbed's (D4). Lengths are
/// never stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoadPocket {
    /// The edge end whose node the pocket lanes turn at.
    pub end: EdgeEnd,
    /// A pocket lane on the drivers' left.
    pub left: bool,
    /// A pocket lane on the drivers' right.
    pub right: bool,
}

impl RoadPocket {
    /// The pocket a save names: `end` 0 none, 1 start, 2 end; `roles` bit 1 left, bit 2 right.
    /// `None` for no pocket; an error for an unknown code or a pocket with neither lane.
    pub fn from_codes(end: i64, roles: i64) -> Result<Option<Self>, &'static str> {
        let end = match end {
            0 if roles == 0 => return Ok(None),
            1 => EdgeEnd::Start,
            2 => EdgeEnd::End,
            _ => return Err("invalid_pocket_dimensions"),
        };
        if !(1..=3).contains(&roles) {
            return Err("invalid_pocket_dimensions");
        }
        Ok(Some(Self {
            end,
            left: roles & 1 != 0,
            right: roles & 2 != 0,
        }))
    }

    /// [`Self::from_codes`]'s inverse: `(end, roles)`, `(0, 0)` for no pocket.
    pub fn codes(pocket: Option<Self>) -> (i64, i64) {
        pocket.map_or((0, 0), |pocket| {
            let end = match pocket.end {
                EdgeEnd::Start => 1,
                EdgeEnd::End => 2,
            };
            (end, i64::from(pocket.left) | i64::from(pocket.right) << 1)
        })
    }

    /// Whether the lanes of one direction (`is_fwd`) approach this pocket's end: forward lanes
    /// reach an edge's end, backward lanes its start.
    pub fn serves(self, is_fwd: bool) -> bool {
        is_fwd == (self.end == EdgeEnd::End)
    }

    // The pocket lanes `(inner, outer)` in lane-index order: the lane next to the line between
    // the lane groups (index 0 or -1) and the one at the kerb. Under right-hand traffic the
    // drivers' left is the inner side.
    pub(crate) fn inner_outer(self) -> (bool, bool) {
        if config::DRIVE_ON_LEFT {
            (self.right, self.left)
        } else {
            (self.left, self.right)
        }
    }
}

impl Edge {
    /// Lateral offset, in the edge's forward frame, of the line between its backward and forward
    /// lane groups: zero when both directions carry equally many lanes, else half a lane per
    /// extra lane towards the direction with fewer, so the lanes fill the carriageway the
    /// profile centres on the centreline (`ROAD-100`), where the markings already divide it.
    /// A 2+1 road's lanes sit at -3.5, 0 and 3.5 m, a one-way lane on the centreline. A median
    /// is centred on this line. On a pocket edge (`ROAD-95`) the carriageway is centred on
    /// [`Self::kerb_shift_m`] instead, so a kerb pocket's lane lies outside the base road's
    /// lanes, which keep their offsets: a 1+1 road with a forward right pocket has its lanes at
    /// -1.75, 1.75 and 5.25 m. `O(1)`, allocation-free.
    pub fn lane_group_offset(&self) -> f32 {
        let side = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
        let centred = (f32::from(self.bkw_lanes) - f32::from(self.fwd_lanes))
            * config::LANE_WIDTH
            * 0.5
            * side;
        if self.pocket.is_none() {
            return centred;
        }
        self.kerb_shift_m() + centred
    }

    /// Lateral offset, in the edge's forward frame, of the carriageway's centre from the
    /// centreline (`ROAD-95`): half a lane towards the kerb a pocket lane moves out, so the
    /// roadbed profile moves that kerb by a lane and keeps the other. Zero without a pocket.
    /// `O(1)`.
    pub fn kerb_shift_m(&self) -> f32 {
        let Some(pocket) = self.pocket else {
            return 0.0;
        };
        // Lanes run outwards from the line between the groups on the driving side: forward
        // lanes towards +x under right-hand traffic, backward lanes towards -x.
        let side = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
        let direction = if pocket.end == EdgeEnd::End {
            1.0
        } else {
            -1.0
        };
        let (inner, outer) = pocket.inner_outer();
        // An inner pocket moves its kerb on a road without a median (on a two-way road the
        // oncoming lanes move out with it, round R5c1); on a divided road it takes the median's
        // room (`ROAD-95` round R5b1), so the footprint keeps both kerbs.
        let inner_kerb = inner && self.median_pocket_lanes() == 0;
        let lanes = f32::from(u8::from(outer)) - f32::from(u8::from(inner_kerb));
        lanes * config::LANE_WIDTH * 0.5 * side * direction
    }

    /// `points` (this edge's polyline) shifted sideways by [`Self::kerb_shift_m`], mitred at
    /// interior points as lane polylines are: the carriageway's axis, about which the roadbed
    /// spans `width / 2` plus its side band on both sides exactly. A pocket edge's corridor checks
    /// and zoning cells read it (`ROAD-95`, D17); without a pocket it is `points`. `O(points)`,
    /// one allocation.
    pub(crate) fn carriageway_axis(&self, points: &[Vector3]) -> Vec<Vector3> {
        let shift = self.kerb_shift_m();
        if shift == 0.0 || points.len() < 2 {
            return points.to_vec();
        }
        // The lanes' normal (`lanes::geometry::build_one_lane`): positive offsets lie right of
        // the forward direction under right-hand traffic.
        let normal = |a: Vector3, b: Vector3| {
            let (dx, dz) = (b.x - a.x, b.z - a.z);
            let length = dx.hypot(dz);
            if length > 1e-6 {
                Vector3::new(-dz / length, 0.0, dx / length)
            } else {
                Vector3::ZERO
            }
        };
        let last = points.len() - 1;
        (0..=last)
            .map(|index| {
                let offset = if index == 0 {
                    normal(points[0], points[1])
                } else if index == last {
                    normal(points[last - 1], points[last])
                } else {
                    let (n1, n2) = (
                        normal(points[index - 1], points[index]),
                        normal(points[index], points[index + 1]),
                    );
                    let bisector = n1 + n2;
                    let length = bisector.length();
                    if length > 1e-6 {
                        let bisector = bisector / length;
                        bisector / bisector.dot(n1).max(0.1)
                    } else {
                        n1
                    }
                };
                points[index] + offset * shift
            })
            .collect()
    }

    /// The pocket lanes among the lanes of one direction (`is_fwd`): one per pocket role when
    /// those lanes approach the pocket's end, else zero. `O(1)`.
    pub fn pocket_lanes(&self, is_fwd: bool) -> u8 {
        self.pocket
            .filter(|pocket| pocket.serves(is_fwd))
            .map_or(0, |pocket| u8::from(pocket.left) + u8::from(pocket.right))
    }

    /// Gives this edge the turn pocket `pocket`, or removes its pocket: the lanes of the
    /// direction a pocket serves change by the difference in pocket lanes and the width with
    /// them ([`RegionGraph::set_edge_pocket`] on a record outside the graph, such as the pocket
    /// edit's planned piece). Unvalidated; see [`Self::validate_pocket`]. `O(1)`.
    pub(crate) fn set_pocket(&mut self, pocket: Option<RoadPocket>) {
        let (fwd, bkw) = (self.pocket_lanes(true), self.pocket_lanes(false));
        let narrowed = self.median_pocket_lanes();
        self.fwd_lanes -= fwd;
        self.bkw_lanes -= bkw;
        if let Some(median) = self.median.as_mut() {
            median.width_m += f32::from(narrowed) * config::LANE_WIDTH;
        }
        self.pocket = pocket;
        self.fwd_lanes += self.pocket_lanes(true);
        self.bkw_lanes += self.pocket_lanes(false);
        let narrowed = self.median_pocket_lanes();
        if let Some(median) = self.median.as_mut() {
            median.width_m -= f32::from(narrowed) * config::LANE_WIDTH;
        }
        self.width = Edge::cross_section_width_m(self.fwd_lanes, self.bkw_lanes, self.median);
    }

    /// The pocket lanes that take the raised median's room (`ROAD-95` round R5b1): the inner
    /// pocket lane (the drivers' left under right-hand traffic) of a road with a raised median
    /// and lanes both ways, whose median is a lane narrower than the base road's, so the
    /// footprint and both kerbs stay. Zero otherwise. `O(1)`.
    pub(crate) fn median_pocket_lanes(&self) -> u8 {
        let Some(pocket) = self.pocket else {
            return 0;
        };
        let two_way = self.fwd_lanes > 0 && self.bkw_lanes > 0;
        let raised = self.median.is_some_and(RoadMedian::is_raised);
        u8::from(pocket.inner_outer().0 && two_way && raised)
    }

    /// The median as the base road carries it (`ROAD-95`): this edge's, a lane wider for each
    /// lane of a left pocket that narrows it ([`Self::median_pocket_lanes`]). `O(1)`.
    pub(crate) fn base_median(&self) -> Option<RoadMedian> {
        let narrowed = f32::from(self.median_pocket_lanes()) * config::LANE_WIDTH;
        self.median.map(|median| RoadMedian {
            width_m: median.width_m + narrowed,
            ..median
        })
    }

    /// The width as the base road carries it (`ROAD-95`): this edge's width without its pocket
    /// lanes and with the median they narrowed, the bits removing the pocket gives the road
    /// back ([`Self::set_pocket`]); the stored width itself without a pocket. Zoning's curb
    /// frames read it, so a pocket edit leaves the base road's grid (round R5d1). `O(1)`.
    pub(crate) fn base_width_m(&self) -> f32 {
        if self.pocket.is_none() {
            return self.width;
        }
        Edge::cross_section_width_m(
            self.fwd_lanes - self.pocket_lanes(true),
            self.bkw_lanes - self.pocket_lanes(false),
            self.base_median(),
        )
    }

    /// Whether this edge's pocket is one a pocket edit could have built (`ROAD-95`): a car road
    /// at ground level (a bridge's or tunnel's taper would lie on a structure piece), at least one
    /// through lane besides the pocket lanes in the approaching direction, at most
    /// [`MAX_LANES_PER_DIRECTION`] lanes there, and an inner pocket (the drivers' left under
    /// right-hand traffic) as a kerb pocket on a road without a median, where it moves the kerb
    /// on its side (on a two-way road the oncoming lanes move with it, `ROAD-95` round R5c1), or
    /// inside a raised median at least [`config::LANE_WIDTH`] + [`POCKET_MEDIAN_STUB_M`] wide on
    /// the base road, which it narrows by a lane (round R5b1, D3): `pocket_requires_wide_median`
    /// beside a painted or narrower median. Edits and loads share it; the error is the stable
    /// refusal key. `O(1)`.
    pub(crate) fn validate_pocket(&self) -> Result<(), &'static str> {
        let Some(pocket) = self.pocket else {
            return Ok(());
        };
        if self.deleted
            || self.primary_type != TransitType::Road
            || self.allowed_types & TransitFlags::CAR == 0
        {
            return Err("invalid_road");
        }
        if self.class != EdgeClass::Standard {
            return Err("pocket_requires_surface_road");
        }
        if !pocket.left && !pocket.right {
            return Err("invalid_pocket_dimensions");
        }
        let is_fwd = pocket.end == EdgeEnd::End;
        let (lanes, opposite) = if is_fwd {
            (self.fwd_lanes, self.bkw_lanes)
        } else {
            (self.bkw_lanes, self.fwd_lanes)
        };
        if lanes > MAX_LANES_PER_DIRECTION {
            return Err("pocket_lane_limit");
        }
        if lanes <= self.pocket_lanes(is_fwd) {
            return Err("invalid_pocket_dimensions");
        }
        if pocket.inner_outer().0
            && opposite > 0
            && let Some(median) = self.base_median()
        {
            // Inside a divided road the pocket takes a lane of a raised median and leaves at
            // least a kerb stub (D3). On an undivided road it moves the kerb on the oncoming
            // side by a lane (round R5c1), as on a one-way road.
            if !median.is_raised() || median.width_m < config::LANE_WIDTH + POCKET_MEDIAN_STUB_M {
                return Err("pocket_requires_wide_median");
            }
        }
        Ok(())
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

    /// A car road's `allowed_types` with its sidewalks (`FOOT`) on or off (`ROAD-101`). `O(1)`.
    pub(crate) fn sidewalk_flags(allowed_types: u8, sidewalks: bool) -> u8 {
        if sidewalks {
            allowed_types | TransitFlags::FOOT
        } else {
            allowed_types & !TransitFlags::FOOT
        }
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
    /// Whether `node` is a lane transition (`ROAD-95`): typed [`NodeType::LaneTransition`] and
    /// joining exactly two live edges. Every rule that treats the node specially reads this, so
    /// the type is inert once a third arm or a removal changes the degree. `O(degree)`.
    pub(crate) fn is_lane_transition(&self, node: u32) -> bool {
        let node = self.get_valid_node(node);
        self.node(node).node_type == NodeType::LaneTransition
            && self.live_node_connection_count(node) == 2
    }

    /// Resets each of `nodes` that is typed [`NodeType::LaneTransition`] but no longer joins two
    /// live edges to [`NodeType::Junction`] (`ROAD-95`): a bulldozed pocket pair leaves a road
    /// end, not a hidden node. Not a border role, so the role revision is kept. `O(sum of
    /// degrees)`.
    pub(crate) fn reset_stale_lane_transitions(&mut self, nodes: impl IntoIterator<Item = u32>) {
        for node in nodes {
            let node = self.get_valid_node(node);
            if self.node(node).node_type == NodeType::LaneTransition
                && !self.is_lane_transition(node)
            {
                self.nodes[node as usize].node_type = NodeType::Junction;
            }
        }
    }

    /// Gives edge `id` the turn pocket `pocket` (`ROAD-95`), or removes its pocket: the lanes of
    /// the direction a pocket serves change by the difference in pocket lanes and the width with
    /// them; the centreline, id and everything else are kept. The result must pass
    /// [`Edge::validate_pocket`]. The pocket's transition node and the split are the pocket edit's;
    /// callers re-lay out both end nodes and rebuild their lanes. `O(1)`.
    pub fn set_edge_pocket(&mut self, id: usize, pocket: Option<RoadPocket>) {
        let edge = self.edge_mut(id);
        edge.set_pocket(pocket);
        debug_assert_eq!(edge.validate_pocket(), Ok(()));
    }

    /// Gives edge `id` `fwd` forward and `bkw` backward lanes, `median`, and the width they take,
    /// and its road type's `attributes` (`ROAD-101`): the `FOOT` flag from `sidewalks`, the speed
    /// limit (the routing `base_cost` recomputed from it when it changes),
    /// `no_building_spawn` from `zoning` and `no_street_lights` from `street_lights`. Keeps its centreline, id, class and everything else
    /// (`ROAD-80`). The cross-section must pass [`Edge::validate_cross_section`], so the edge is
    /// a car road. The spatial index reads the centreline only, so it is kept. Callers re-lay out
    /// both end nodes and rebuild their lanes, as `SimCore`'s in-place edit does. `O(1)`, or
    /// `O(points)` for a speed change.
    pub(crate) fn set_edge_cross_section(
        &mut self,
        id: usize,
        fwd: u8,
        bkw: u8,
        median: Option<RoadMedian>,
        attributes: RoadAttributes,
    ) {
        let edge = self.edge_mut(id);
        debug_assert_eq!(edge.validate_cross_section(fwd, bkw, median), Ok(()));
        edge.fwd_lanes = fwd;
        edge.bkw_lanes = bkw;
        edge.median = median;
        edge.width = Edge::cross_section_width_m(fwd, bkw, median);
        edge.allowed_types = Edge::sidewalk_flags(edge.allowed_types, attributes.sidewalks);
        edge.no_building_spawn = !attributes.zoning;
        edge.no_street_lights = !attributes.street_lights;
        if edge.speed_limit.to_bits() != attributes.speed_ms.to_bits() {
            edge.speed_limit = attributes.speed_ms;
            edge.base_cost = CostCalculator::calculate_costs(edge).0;
        }
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
