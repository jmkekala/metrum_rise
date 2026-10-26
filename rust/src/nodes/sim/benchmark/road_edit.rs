// SPDX-License-Identifier: GPL-2.0-only

//! Fixed-size, allocation-free diagnostics for the latest road command's core stages.

use godot::prelude::*;

/// Latest road command measurements, matched to its authoritative generation by the harness.
#[derive(Default)]
pub(crate) struct RoadEditMetrics {
    pub(crate) generation: u64,
    pub(crate) committed: bool,
    // Dispatch to dequeue, excluding subsequent core mutex contention.
    pub(crate) queue_wait_ms: f64,
    pub(crate) lock_wait_ms: f64,
    pub(crate) core_work_ms: f64,
    pub(crate) add_ms: f64,
    /// Whether the exact road geometry came from the worker's latest completed preview.
    pub(crate) preview_plan_reused: bool,
    /// Whether the commit installed the preview's terrain entries instead of building its own
    /// (`ROAD-68-E`).
    pub(crate) preview_terrain_adopted: bool,
    /// Reuse checks or a fresh local road solve; included in add_ms.
    pub(crate) road_plan_ms: f64,
    /// Deferred structural stamps, site capture and terrain compilation; included in add_ms.
    pub(crate) terrain_plan_ms: f64,
    // Finalization includes the agent/lane, building, and routing child stages, but not surface_ms.
    pub(crate) finalize_ms: f64,
    pub(crate) surface_ms: f64,
    // Children of surface_ms: roadbed compile or preview install with the undo capture, the
    // field and cell overlap test, and the terrain validation and its cache install.
    pub(crate) surface_roadbed_ms: f64,
    pub(crate) surface_overlap_ms: f64,
    pub(crate) surface_terrain_ms: f64,
    // The commit's graded tiles (`TERRAIN-07-E`), after the terrain validation: their
    // selection, input gathers and bakes, and how many tiles were adopted from the preview
    // and baked.
    pub(crate) surface_graded_ms: f64,
    pub(crate) graded_select_ms: f64,
    pub(crate) graded_inputs_ms: f64,
    pub(crate) graded_bake_ms: f64,
    pub(crate) graded_adopted: usize,
    pub(crate) graded_baked: usize,
    // Children of add_ms: undo record, topology mutation (graph and parcels), cell-lot marks.
    pub(crate) undo_ms: f64,
    pub(crate) topology_ms: f64,
    pub(crate) zoning_ms: f64,
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
            queue_wait_ms,
            lock_wait_ms,
            core_work_ms,
            add_ms,
            preview_plan_reused,
            preview_terrain_adopted,
            road_plan_ms,
            terrain_plan_ms,
            finalize_ms,
            surface_ms,
            surface_roadbed_ms,
            surface_overlap_ms,
            surface_terrain_ms,
            surface_graded_ms,
            graded_select_ms,
            graded_inputs_ms,
            graded_bake_ms,
            undo_ms,
            topology_ms,
            zoning_ms,
            agent_invalidate_ms,
            lanes_and_reattach_ms,
            buildings_ms,
            routing_ms,
            mesh_ms,
            snapshot_ms,
            refined_state_ms
        );
        result.set("graded_adopted", self.graded_adopted as i64);
        result.set("graded_baked", self.graded_baked as i64);
        result.set("dirty_edges", self.dirty_edges as i64);
        result.set("rebuilt_surface_chunks", self.rebuilt_surface_chunks as i64);
        result.set("rebuilt_terrain_chunks", self.rebuilt_terrain_chunks as i64);
        result
    }
}
