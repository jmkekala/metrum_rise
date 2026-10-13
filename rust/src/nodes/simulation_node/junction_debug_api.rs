// SPDX-License-Identifier: GPL-2.0-only

//! Junction connector debug view and stuck-at-junction detector (`junction_connectors`).
//!
//! Called only by the overlay while the visual mode is on; nothing here runs on the
//! simulation tick.

use super::*;
use crate::nodes::sim::render::junctions::{
    JunctionArea, JunctionView, describe_connectors, pick_junction,
};
use crate::simulation::economy::agents::junction_stalls::JunctionStallDetector;

#[godot_api(secondary)]
impl SimulationNode {
    /// Junction overlay lines for `view`, the `junction_connectors` index (`0` is off). Covers
    /// the XZ square of half size `half_extent_m` around `center`, or only `focus_node` when
    /// it is not negative. Returns an empty dictionary while the simulation core is busy or
    /// the view is off, so the caller keeps its last mesh; otherwise `points` and `colors` as
    /// a line list plus the counts `junctions`, `connectors`, `pass_throughs`, `occupied` and
    /// `blocked`.
    #[func]
    pub fn get_junction_debug_lines(
        &self,
        view: i64,
        center: Vector3,
        half_extent_m: f32,
        focus_node: i64,
    ) -> VarDictionary {
        let Some(view) = JunctionView::from_index(view) else {
            return VarDictionary::new();
        };
        let area = match u32::try_from(focus_node) {
            Ok(node_id) => JunctionArea::Node(node_id),
            Err(_) => {
                let half = Vector3::new(half_extent_m, 0.0, half_extent_m);
                JunctionArea::Rect {
                    min: center - half,
                    max: center + half,
                }
            }
        };
        let Some(core) = self.try_lock_core() else {
            return VarDictionary::new();
        };
        let lines = core.junction_debug_lines(view, area);
        drop(core);
        vdict! {
            "points": PackedVector3Array::from(lines.points.as_slice()),
            "colors": PackedColorArray::from(lines.colors.as_slice()),
            "junctions": lines.junctions as i64,
            "connectors": lines.connectors as i64,
            "pass_throughs": lines.pass_throughs as i64,
            "occupied": lines.occupied as i64,
            "blocked": lines.blocked as i64,
        }
    }

    /// The nearest junction the overlay draws within `radius_m` of `pos` (XZ), or `-1`.
    #[func]
    pub fn pick_debug_junction(&self, pos: Vector3, radius_m: f32) -> i64 {
        let core = self.lock_core();
        pick_junction(
            &core.region_graph,
            &core.transit_network.lane_system,
            pos,
            radius_m,
        )
        .map_or(-1, i64::from)
    }

    /// Logs the node's vehicle connectors on `events`: lane id, from and to edge/lane, length,
    /// turn speed cap and whether the node whitelists its turns. Returns the lines logged.
    #[func]
    pub fn log_junction_connectors(&self, node_id: i64) -> i64 {
        let Ok(node_id) = u32::try_from(node_id) else {
            return 0;
        };
        let core = self.lock_core();
        if node_id as usize >= core.region_graph.node_count() {
            return 0;
        }
        let lines = describe_connectors(
            &core.region_graph,
            &core.transit_network.lane_system,
            node_id,
        );
        drop(core);
        for line in &lines {
            debug_log!("events", "{line}");
        }
        lines.len() as i64
    }

    /// Runs the stuck-at-junction detector, at most one scan per simulated second, and logs
    /// each car newly stopped on a connector for 10 simulated seconds on `events`. Returns the
    /// number reported; `0` while the core is busy. Call it only while the view is on.
    #[func]
    pub fn scan_junction_stalls(&mut self) -> i64 {
        let Some(core) = Self::try_lock_shared_core(&self.core) else {
            return 0;
        };
        let reports = self
            .junction_stalls
            .get_or_insert_with(JunctionStallDetector::default)
            .scan(&core.agents, &core.transit_network.lane_system);
        drop(core);
        for report in &reports {
            debug_log!("events", "{report}");
        }
        reports.len() as i64
    }

    /// Drops the detector and everything it tracked; the overlay calls this when it turns off.
    #[func]
    pub fn clear_junction_stalls(&mut self) {
        self.junction_stalls = None;
    }
}
