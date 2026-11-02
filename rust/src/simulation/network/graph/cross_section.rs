// SPDX-License-Identifier: GPL-2.0-only

//! An edge's cross-section as the lanes read it: where its two directional lane groups sit
//! inside the centred carriageway (`ROAD-100`).

use super::Edge;
use crate::config;

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
}
