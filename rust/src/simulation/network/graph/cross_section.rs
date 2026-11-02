// SPDX-License-Identifier: GPL-2.0-only

//! An edge's cross-section: where its two directional lane groups sit inside the centred
//! carriageway (`ROAD-100`), and its lane counts edited in place (`ROAD-80`).

use super::{Edge, RegionGraph};
use crate::config;
use crate::simulation::network::types::TransitType;

/// Most vehicle lanes one direction of a road carries, as the road tool offers them.
pub(crate) const MAX_LANES_PER_DIRECTION: u8 = 4;

impl Edge {
    /// Lateral offset, in the edge's forward frame, of the line between its backward and forward
    /// lane groups: zero when both directions carry equally many lanes, else half a lane per
    /// extra lane towards the direction with fewer, so the lanes fill the carriageway the
    /// profile centres on the centreline (`ROAD-100`), where the markings already divide it.
    /// A 2+1 road's lanes sit at -3.5, 0 and 3.5 m, a one-way lane on the centreline.
    /// `O(1)`, allocation-free.
    pub fn lane_group_offset(&self) -> f32 {
        let side = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
        (f32::from(self.bkw_lanes) - f32::from(self.fwd_lanes)) * config::LANE_WIDTH * 0.5 * side
    }

    /// The carriageway width a road of `fwd` + `bkw` lanes has, as a stroke builds it
    /// (`network::build_surface_edge`): a lane width each, at least 2 m.
    pub(crate) fn lanes_width_m(fwd: u8, bkw: u8) -> f32 {
        (f32::from(fwd + bkw) * config::LANE_WIDTH).max(2.0)
    }

    /// Whether this edge may carry `fwd` forward and `bkw` backward lanes after an in-place edit
    /// (`ROAD-80`): it must be a live road (a footpath stays a footpath), keep at least one lane
    /// and stay within [`MAX_LANES_PER_DIRECTION`] per direction. The error is the stable
    /// refusal key.
    pub(crate) fn validate_lanes(&self, fwd: u8, bkw: u8) -> Result<(), &'static str> {
        if self.deleted || self.primary_type != TransitType::Road {
            return Err("cross_section_not_road");
        }
        if fwd > MAX_LANES_PER_DIRECTION || bkw > MAX_LANES_PER_DIRECTION {
            return Err("too_many_lanes");
        }
        if fwd == 0 && bkw == 0 {
            return Err("cross_section_cannot_change_transit_mode");
        }
        Ok(())
    }
}

impl RegionGraph {
    /// Gives edge `id` `fwd` forward and `bkw` backward lanes and the width they take, keeping
    /// its centreline, id, class and everything else (`ROAD-80`). The counts must pass
    /// [`Edge::validate_lanes`]. The spatial index reads the centreline only, so it is kept.
    /// Callers re-lay out both end nodes and rebuild their lanes, as `SimCore`'s in-place edit
    /// does. `O(1)`.
    pub(crate) fn set_edge_lanes(&mut self, id: usize, fwd: u8, bkw: u8) {
        let edge = self.edge_mut(id);
        debug_assert_eq!(edge.validate_lanes(fwd, bkw), Ok(()));
        edge.fwd_lanes = fwd;
        edge.bkw_lanes = bkw;
        edge.width = Edge::lanes_width_m(fwd, bkw);
    }
}
