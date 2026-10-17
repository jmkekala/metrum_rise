// SPDX-License-Identifier: GPL-2.0-only

//! One fixed simulation frame: pathing refresh, agent movement and the authored-minute cadence.
//!
//! The sim thread calls [`SimCore::step_frame`] once per ~60 Hz tick while holding the core
//! lock; headless scenarios call it in a loop. Command handling, cell-chunk preparation and
//! render snapshots stay with the caller.

use std::sync::atomic::Ordering;
use std::time::Instant;

use super::state::SimCore;
use crate::debug::CrashSimSnapshot;
use godot::prelude::godot_error;

/// Real seconds per sim-thread frame (~60 Hz); agents move this times the speed multiplier.
pub(super) const FRAME_DT_S: f64 = 1.0 / 60.0;

/// Read-only boundaries used by the headless economy report. The live path uses `()`;
/// monomorphization removes its empty callbacks, without storing telemetry in `SimCore`.
pub(super) trait EconomyObserver {
    fn buildings_changed(&mut self, _core: &SimCore) {}
    fn hourly_economy(&mut self, _core: &SimCore) {}
    fn freight_delivered(
        &mut self,
        _shipment: &crate::simulation::economy::logistics::Shipment,
        _allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
    ) {
    }
    /// A worker's wage paid at its daily phase (`SIM-02-G`).
    fn wage_paid(
        &mut self,
        _core: &SimCore,
        _payment: crate::simulation::economy::households::WagePayment,
    ) {
    }
    /// A building is about to settle its 24-hour window at its daily phase.
    fn building_day_opened(&mut self, _core: &SimCore, _building_idx: usize) {}
    /// A building settled its 24-hour window; its accumulators still hold the window.
    fn building_day_closed(&mut self, _core: &SimCore, _building_idx: usize) {}
    /// A household settled its 24-hour window; its ledger still holds the window.
    fn household_day_closed(&mut self, _core: &SimCore, _household_id: usize) {}
    /// The calendar close: the city's day totals are the last day's.
    fn day_closed(&mut self, _core: &SimCore) {}
}

impl EconomyObserver for () {}

/// Runs one named simulation phase, flushing crash diagnostics before re-raising a panic.
pub(super) fn run_sim_phase<T>(phase: &str, run: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(value) => value,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("(non-string payload)");
            godot_error!("[sim] {} panicked: {}", phase, message);
            crate::debug::flush_crash_diagnostics(phase);
            std::panic::resume_unwind(payload);
        }
    }
}

pub(super) fn crash_summary_from_core(core: &SimCore) -> CrashSimSnapshot {
    CrashSimSnapshot {
        day_index: core.time.day_index,
        minute_of_day: core.time.minute_of_day,
        speed_multiplier: core.time.speed_multiplier,
        agent_count: core.agents.len(),
        pathfind_count: core.agents.pathfind_count.load(Ordering::Relaxed),
        building_count: core.allocator.buildings.len(),
        household_count: core.households.households.len(),
        road_node_count: core.region_graph.node_count(),
        road_edge_count: core.region_graph.edge_count(),
        road_generation: core.road_tool_surface_generation,
        pending_demand_spawns: core.pending_demand_spawns.len(),
        last_agent_tick_us: core.last_agent_tick_us,
        last_tick_duration_ms: core.last_tick_duration,
        terrain_dirty: core.terrain_dirty,
        water_dirty: core.water_dirty,
        network_dirty: core.network_dirty,
    }
}

pub(super) fn record_crash_phase_for_core(core: &SimCore, phase: &'static str) {
    if crate::debug::is_crash_diagnostics_enabled() {
        crate::debug::record_crash_phase(phase, crash_summary_from_core(core));
    }
}

/// Wall-clock cost and cadence counters of one [`SimCore::step_frame`] call.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrameStepReport {
    /// CCH and flow-field refresh.
    pub(crate) pathing_ms: f64,
    /// Agent movement tick.
    pub(crate) agent_ms: f64,
    /// Clock advance plus every authored-minute step it released.
    pub(crate) minute_ms: f64,
    /// Queued demand spawns, part of `minute_ms`.
    pub(crate) pending_spawn_ms: f64,
    /// Operational-hour economy steps, part of `minute_ms`.
    pub(crate) hourly_ms: f64,
    /// Daily settlement steps, part of `minute_ms`.
    pub(crate) daily_ms: f64,
    /// Authored minutes the clock crossed.
    pub(crate) elapsed_minutes: u16,
    /// Queued demand spawns placed.
    pub(crate) pending_spawns_executed: usize,
    /// Operational hours simulated.
    pub(crate) hourly_ticks: usize,
    /// Daily settlements simulated.
    pub(crate) daily_ticks: usize,
}

impl SimCore {
    /// Advances the simulation by one frame of `frame_dt_s` real seconds at the current speed.
    ///
    /// A paused clock (`speed_multiplier <= 0`) leaves all state untouched. Otherwise agents move
    /// `frame_dt_s × speed` seconds and every slot the clock crosses runs its due systems in
    /// order ([`SimCore::step_slot`]). Results depend only on the core state and `frame_dt_s`.
    pub(crate) fn step_frame(&mut self, frame_dt_s: f64) -> FrameStepReport {
        self.step_frame_observed(frame_dt_s, &mut ())
    }

    pub(super) fn step_frame_observed(
        &mut self,
        frame_dt_s: f64,
        observer: &mut impl EconomyObserver,
    ) -> FrameStepReport {
        let mut report = FrameStepReport::default();
        // Sites placed by tools since the last frame; demand places its own during slots.
        self.schedule_construction_completions();
        let speed = self.time.speed_multiplier;
        if speed <= 0.0 {
            return report;
        }

        // Rebuild CCH if dirty, then rebuild any dirty flow fields.
        let pathing_start = Instant::now();
        record_crash_phase_for_core(self, "pathing rebuild");
        self.transit_network
            .rebuild_pathing_if_dirty(&mut self.region_graph);
        {
            let alloc = &self.allocator;
            let graph = &self.region_graph;
            self.transit_network
                .flow_fields
                .rebuild_dirty(graph, |zone, mode_flags| {
                    alloc.get_sources_for_zone(zone, graph, mode_flags)
                });
        }
        report.pathing_ms = pathing_start.elapsed().as_secs_f64() * 1000.0;

        let dt = (frame_dt_s * speed as f64) as f32;
        let t_agent = Instant::now();
        record_crash_phase_for_core(self, "agent tick");
        run_sim_phase("agent tick", || {
            self.agents.tick(
                &self.allocator,
                &mut self.transit_network,
                &mut self.region_graph,
                dt,
                &self.time,
            );
        });
        self.last_agent_tick_us = t_agent.elapsed().as_micros() as u64;
        report.agent_ms = self.last_agent_tick_us as f64 / 1000.0;

        let minute_start = Instant::now();
        record_crash_phase_for_core(self, "time advance");
        self.align_events_to_clock();
        let time_advance = self.time.process_delta(frame_dt_s);
        report.elapsed_minutes = time_advance.elapsed_minutes;
        for slot in time_advance.crossed_slots() {
            self.step_slot(slot, &mut report, observer);
        }
        report.minute_ms = minute_start.elapsed().as_secs_f64() * 1000.0;
        report
    }
}
