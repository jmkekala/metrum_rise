// SPDX-License-Identifier: GPL-2.0-only

//! All simulation subsystems. Driven by [`crate::nodes::simulation_node::SimulationNode`].
//!
//! Subsystems communicate by passing references to each other in `simulate_tick` —
//! there is no shared global state. The canonical tick order is:
//! time → terrain (passive) → water → network (road edits) → pathing → grid (env) → buildings → agents.

pub(crate) mod agriculture;
pub mod buildings;
pub mod core;
pub mod economy;
pub(crate) mod extraction;
pub mod grid;
pub mod network;
pub mod pathing;
pub(crate) mod resources;
pub(crate) mod save;
pub mod terrain;
pub mod water;
pub(crate) mod work_area;
pub(crate) mod world_definition;
pub mod zoning;

/// Shared road-configuration table used by parametrized tests across all subsystems.
///
/// Each entry is `(fwd_lanes, bkw_lanes, label)`. Every scenario test that exercises
/// road behaviour should iterate this slice so new road types are automatically covered
/// without duplicating test bodies. **Add a row here whenever a new road configuration
/// is introduced; that is the only change required to extend all parametrized tests.**
#[cfg(test)]
pub const LANE_CONFIGS: &[(u8, u8, &str)] = &[
    (1, 0, "1-way 1-lane"),
    (2, 0, "1-way 2-lane"),
    (1, 1, "2-way 1+1"),
    (2, 2, "2-way 2+2"),
];
