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
use crate::simulation::economy::agents::tick::TripQueryScratch;
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
    /// Marks the flow fields of the zones whose buildings the allocator placed, completed,
    /// changed or removed since the last call, so the next rebuild sees them: a loaded game
    /// rebuilds every field, and the running game must route on the same fields. O(zones).
    pub(crate) fn mark_changed_zone_flow_fields(&mut self) {
        use crate::simulation::buildings::allocator::BASELINE_PRIVATE_ZONES;
        for (zone_idx, zone) in BASELINE_PRIVATE_ZONES.iter().enumerate() {
            if std::mem::take(&mut self.allocator.dirty_zones[zone_idx]) {
                self.transit_network.flow_fields.mark_zone_dirty(*zone);
            }
        }
    }

    // The frame's flow-field drain reads the zone index and the entrance cache, so like every
    // reader of the index it rebuilds a stale one first (`SIM-02-I`). The index is rebuilt at
    // every frame that starts with it stale, whether or not a field is marked: the flow-field
    // marks are not saved (a load marks every field), while the stale flag is, so a game
    // loaded from a save between frames rebuilds the index at the same frame as the game it
    // was saved from, from the same buildings, and its vacancy order and admission order
    // continue identically. A stale entrance cache is rebuilt too; it has no order to keep.
    // O(B) when stale, O(1) otherwise.
    fn repair_flow_field_sources(&mut self) {
        if self.allocator.vacancy_index_stale() {
            self.allocator.rebuild_zone_index();
        }
        let allocator = &mut self.allocator;
        if allocator.entrances_dirty || allocator.entrances.len() != allocator.buildings.len() {
            allocator.rebuild_entrance_cache(&self.region_graph, &self.transit_network.lane_system);
        }
    }

    /// Builds at load what the first frame would otherwise build (`SIM-02-K`): the routing
    /// hierarchy if dirty, the flow fields loading marked dirty, and the route scratch of the
    /// movement pass, the slots' trips and the job search with its network-leg storage for
    /// that network, and admission's border arrival costs. Each is a function of the loaded
    /// network, buildings and entrances, so the first frame (and the first admission batch)
    /// finds them current and has nothing left to build. The fields wait for the first frame
    /// when the zone index or entrance cache is stale here.
    pub(crate) fn prepare_loaded_pathing(&mut self) {
        self.transit_network
            .rebuild_pathing_if_dirty(&mut self.region_graph);
        self.allocator
            .prepare_border_arrival_costs(&self.transit_network, &self.region_graph);
        let allocator = &self.allocator;
        if !allocator.vacancy_index_stale()
            && !allocator.entrances_dirty
            && allocator.entrances.len() == allocator.buildings.len()
        {
            self.mark_changed_zone_flow_fields();
            let revision = self.transit_network.flow_graph_revision();
            let alloc = &self.allocator;
            let graph = &self.region_graph;
            self.transit_network.flow_fields.rebuild_dirty(
                graph,
                revision,
                |zone, mode_flags, sources| {
                    alloc.collect_sources_for_zone(zone, graph, mode_flags, sources)
                },
            );
        }
        self.agents
            .prepare_movement_scratch(&self.transit_network, &self.region_graph);
        self.slot_route_scratch
            .prepare_routes(&self.transit_network, &self.region_graph);
        self.households
            .prepare_job_search_scratch(&self.transit_network, &self.region_graph);
    }

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
        observer: &mut (impl EconomyObserver + Send),
    ) -> FrameStepReport {
        // The sim thread, tests and benchmarks all step in the shared pool (CODE-17).
        crate::thread_pools::install(|| self.step_frame_in_pool(frame_dt_s, observer))
    }

    fn step_frame_in_pool(
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

        // Rebuild CCH if dirty, then rebuild any dirty flow fields, the zones whose buildings
        // changed since the last frame among them.
        let pathing_start = Instant::now();
        record_crash_phase_for_core(self, "lane surface sync");
        self.transit_network
            .sync_new_lane_heights(&self.region_graph, &self.heightmap);
        record_crash_phase_for_core(self, "pathing rebuild");
        self.transit_network
            .rebuild_pathing_if_dirty(&mut self.region_graph);
        self.repair_flow_field_sources();
        self.mark_changed_zone_flow_fields();
        {
            let revision = self.transit_network.flow_graph_revision();
            let alloc = &self.allocator;
            let graph = &self.region_graph;
            self.transit_network.flow_fields.rebuild_dirty(
                graph,
                revision,
                |zone, mode_flags, sources| {
                    alloc.collect_sources_for_zone(zone, graph, mode_flags, sources)
                },
            );
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
        self.weather
            .advance_scheduled(frame_dt_s * speed as f64, self.time.seconds_per_day / 24.0);
        self.weather
            .advance_wind(frame_dt_s * f64::from(speed.min(1.0)));
        report.elapsed_minutes = time_advance.elapsed_minutes;
        // The slots' serial trip pricing and planning use one owned route scratch, whichever
        // pool worker runs this frame: each worker's own scratch cost about 8 ms at 100k to
        // prepare the first time it ran a slot (`SIM-02-K`). Scratch never changes a result.
        let mut route_scratch = std::mem::take(&mut self.slot_route_scratch);
        TripQueryScratch::with_owned(&mut route_scratch, || {
            for slot in time_advance.crossed_slots() {
                self.step_slot(slot, &mut report, observer);
            }
        });
        self.slot_route_scratch = route_scratch;
        report.minute_ms = minute_start.elapsed().as_secs_f64() * 1000.0;
        report
    }
}
