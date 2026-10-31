// SPDX-License-Identifier: GPL-2.0-only

//! The gridlock report (`TRAFFIC-14`): the junctions of the wait-for cycles the last gridlock
//! check confirmed and could not detour, for the player's notice and overlay. Read-only; the
//! check itself runs in the simulation's `gridlock check` slot.

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// The last gridlock check's report: `junctions` (node ids, ascending) and `positions`
    /// (their world positions, same order) of the saturated cycles nothing could leave, plus the
    /// counts `checks`, `waiting_lanes`, `cycles`, `confirmed`, `saturated`, `detours` and
    /// `saturation_reports`. Empty while the simulation core is busy, so the caller keeps its
    /// last report.
    #[func]
    pub fn get_gridlock_report(&self) -> VarDictionary {
        let Some(core) = self.try_lock_core() else {
            return VarDictionary::new();
        };
        let report = core.agents.gridlock.report();
        let mut junctions = PackedInt32Array::new();
        let mut positions = PackedVector3Array::new();
        for &node in &report.junctions {
            if (node as usize) < core.region_graph.node_count() {
                junctions.push(node as i32);
                positions.push(core.region_graph.node(node).pos);
            }
        }
        vdict! {
            "junctions": junctions,
            "positions": positions,
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
