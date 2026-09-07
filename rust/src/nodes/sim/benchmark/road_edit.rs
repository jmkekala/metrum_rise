// SPDX-License-Identifier: GPL-2.0-only

//! Fixed-size, allocation-free diagnostics for the latest road command's core stages.

use godot::prelude::*;

/// Latest road command measurements, matched to its authoritative generation by the harness.
#[derive(Default)]
pub(crate) struct RoadEditMetrics {
    pub(crate) generation: u64,
    pub(crate) committed: bool,
    pub(crate) lock_wait_ms: f64,
    pub(crate) core_work_ms: f64,
    pub(crate) add_ms: f64,
    // Finalization includes the agent/lane, building, and routing child stages, but not surface_ms.
    pub(crate) finalize_ms: f64,
    pub(crate) surface_ms: f64,
    pub(crate) agent_invalidate_ms: f64,
    pub(crate) lanes_and_reattach_ms: f64,
    pub(crate) buildings_ms: f64,
    pub(crate) routing_ms: f64,
    pub(crate) mesh_ms: f64,
    pub(crate) snapshot_ms: f64,
    pub(crate) refined_state_ms: f64,
    pub(crate) dirty_edges: usize,
    pub(crate) rebuilt_surface_chunks: usize,
    pub(crate) rebuilt_terrain_chunks: usize,
}

impl RoadEditMetrics {
    pub(crate) fn to_dictionary(&self) -> VarDictionary {
        let mut result = VarDictionary::new();
        macro_rules! fields {
            ($($field:ident),* $(,)?) => { $(result.set(stringify!($field), self.$field);)* };
        }
        fields!(
            generation,
            committed,
            lock_wait_ms,
            core_work_ms,
            add_ms,
            finalize_ms,
            surface_ms,
            agent_invalidate_ms,
            lanes_and_reattach_ms,
            buildings_ms,
            routing_ms,
            mesh_ms,
            snapshot_ms,
            refined_state_ms
        );
        result.set("dirty_edges", self.dirty_edges as i64);
        result.set("rebuilt_surface_chunks", self.rebuilt_surface_chunks as i64);
        result.set("rebuilt_terrain_chunks", self.rebuilt_terrain_chunks as i64);
        result
    }
}
