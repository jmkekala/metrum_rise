// SPDX-License-Identifier: GPL-2.0-only

//! The gridlock report (`TRAFFIC-14`): the junctions of the wait-for cycles the last gridlock
//! check confirmed and could not detour, for the player's notice and overlay. Read-only and read
//! from the render snapshot, so it never waits for the core; the check itself runs in the
//! simulation's `gridlock check` slot.

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// The last gridlock check's report, as of the latest render snapshot: `junctions` (node
    /// ids, ascending) and `positions` (their world positions, same order) of the saturated
    /// cycles nothing could leave, plus the counts `checks`, `waiting_lanes`, `cycles`,
    /// `confirmed`, `saturated`, `detours` and `saturation_reports`.
    #[func]
    pub fn get_gridlock_report(&self) -> VarDictionary {
        let snapshot = self.snapshot.read().unwrap();
        let report = &snapshot.gridlock;
        let junctions = &snapshot.gridlock_junctions;
        vdict! {
            "junctions": junctions.iter().map(|&(node, _)| node as i32).collect::<PackedInt32Array>(),
            "positions": junctions.iter().map(|&(_, pos)| pos).collect::<PackedVector3Array>(),
            "checks": report.checks as i64,
            "waiting_lanes": i64::from(report.waiting_lanes),
            "cycles": i64::from(report.cycles),
            "confirmed": i64::from(report.confirmed),
            "saturated": i64::from(report.saturated),
            "detours": report.detours as i64,
            "saturation_reports": report.saturation_reports as i64,
        }
    }
}
