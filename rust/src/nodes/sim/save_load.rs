// SPDX-License-Identifier: GPL-2.0-only

//! SQLite save/load bridge methods for the Godot simulation node.

use crate::debug_log;
use crate::nodes::sim::core::SimCore;
use crate::nodes::sim::core::SimEvent;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::core::timing_wheel::{TimedEvent, TimingWheel};
use crate::simulation::save::{
    LoadedSimulation, SaveGameView, SavedCameraState, SavedScheduler, load_from_sqlite,
    save_to_sqlite,
};
use crate::simulation::vegetation::VegetationGenerator;
use std::path::PathBuf;

impl SimCore {
    /// Saves the current simulation state into a single-file SQLite snapshot.
    pub(crate) fn save_game_internal(
        &self,
        path: &str,
        camera: Option<SavedCameraState>,
    ) -> Result<(), String> {
        let path = PathBuf::from(path);
        let events = self.aligned_events();
        save_to_sqlite(
            &path,
            SaveGameView {
                camera,
                config: &self.config,
                vegetation: &self.vegetation.config,
                vegetation_edits: &self.vegetation_edits,
                time: &self.time,
                weather: &self.weather,
                terrain: &self.heightmap,
                water: &self.watermap,
                resource_deposits: &self.resource_deposits,
                graph: &self.region_graph,
                zoning: &self.zoning,
                pollution: &self.pollution,
                noise: &self.noise,
                demand: &self.demand,
                pending_demand_spawns: &self.pending_demand_spawns,
                scheduler: SavedScheduler {
                    next_slot: events.next_slot(),
                    next_event_seq: events.next_seq(),
                    next_spawn_id: self.next_spawn_id,
                    logistics_hour_ticks: self.logistics.hour_ticks,
                    logistics_next_shipment_id: self.logistics.next_shipment_id,
                    freight_departures: self.logistics.departure_minutes.clone(),
                    agent_settlement_days: self.agents.settlement_days,
                    road_length: {
                        let (edges, next_edge) =
                            self.road_length.saved_positions(&self.region_graph);
                        (
                            self.road_length.front_m,
                            self.road_length.back_mm,
                            edges,
                            next_edge,
                            self.road_length.measured,
                        )
                    },
                    environment: self.environment_sweep,
                    demand_snapshot: self.demand.saved_snapshot_sweep(),
                    events: events
                        .pending()
                        .into_iter()
                        .map(|timed| {
                            let (kind, a) = timed.event.to_saved();
                            (timed.slot, timed.seq, kind, a)
                        })
                        .collect(),
                },
                allocator: &self.allocator,
                households: &self.households,
                logistics: &self.logistics,
                resource_extraction: &self.resource_extraction,
                agriculture: &self.agriculture,
                agents: &self.agents,
                network: &self.transit_network,
                treasury: &self.treasury,
                service_policy: &self.service_policy,
                fiscal_policy: &self.fiscal_policy,
                budget_history: &self.budget_history,
            },
        )
        .map_err(|err| err.to_string())
    }

    /// Loads a full simulation snapshot from a SQLite save file and replaces the live world.
    pub(crate) fn load_game_internal(
        &mut self,
        path: &str,
    ) -> Result<Option<SavedCameraState>, String> {
        // The load's index, routing and economy rebuilds are parallel; Godot's main thread and
        // tests run them in the shared pool (CODE-17).
        crate::thread_pools::install(|| self.load_game_in_pool(path))
    }

    fn load_game_in_pool(&mut self, path: &str) -> Result<Option<SavedCameraState>, String> {
        let mut loaded = load_from_sqlite(&PathBuf::from(path), &self.allocator.registry)
            .map_err(|err| err.to_string())?;
        let events = restore_events(&loaded.scheduler, &loaded.time)?;
        // Vacancy capacity and admission eligibility require the live asset registry; the saved
        // vacancy order is restored over that rebuild (`SIM-02`).
        std::mem::swap(&mut loaded.allocator.registry, &mut self.allocator.registry);
        loaded.allocator.rebuild_zone_index();
        loaded
            .allocator
            .restore_vacancy_order(&loaded.vacancy_order);
        let stale_index = loaded.vacancy_order.stale;
        let camera = loaded.camera;
        self.apply_loaded_simulation(loaded, events);
        // Loading itself reads the index, which rebuilds a stale one; it stays stale for the
        // saved game's next reader.
        if stale_index {
            self.allocator.dirty_index = true;
        }
        Ok(camera)
    }

    fn apply_loaded_simulation(&mut self, loaded: LoadedSimulation, events: TimingWheel<SimEvent>) {
        self.config = loaded.config;
        self.vegetation = VegetationGenerator::resolve(loaded.vegetation, &self.config);
        self.vegetation_edits = loaded.vegetation_edits;
        self.time = loaded.time;
        self.weather = loaded.weather;
        self.world_epoch = self.world_epoch.wrapping_add(1);
        self.heightmap = loaded.terrain;
        self.watermap = loaded.water;
        self.resource_deposits = loaded.resource_deposits;
        self.region_graph = loaded.graph;
        self.transit_network = loaded.transit_network;
        self.zoning = loaded.zoning;
        self.pollution = loaded.pollution;
        self.noise = loaded.noise;
        self.desirability = loaded.desirability;
        self.demand = loaded.demand;
        self.pending_demand_spawns = loaded.pending_demand_spawns;
        self.next_spawn_id = loaded.scheduler.next_spawn_id;
        self.events = events;
        // The loaded allocator holds the live asset registry and its restored indices.
        self.allocator = loaded.allocator;
        self.allocator.now_slot = self.time.absolute_slot();
        // A live catalog with the saved lot sizes keeps the saved lots (`SIM-02-H`).
        let lot_sizes = self
            .allocator
            .cell_lot_sizes(&self.zoning, self.demand.runtime_catalog());
        self.zoning.restore_cell_lot_assets(lot_sizes);
        // Transforms and entrances were restored before agent lane references.
        self.allocator
            .rebuild_building_site_clients(self.zoning.config.zone_cell_m);
        self.households = loaded.households;
        self.logistics = loaded.logistics;
        self.logistics.hour_ticks = loaded.scheduler.logistics_hour_ticks;
        self.logistics.departure_minutes = loaded.scheduler.freight_departures.clone();
        // The saved supplier index labels components as the loaded graph does (`SIM-02-I`).
        self.logistics
            .adopt_loaded_topology(&self.transit_network, &self.region_graph);
        // So does the household destination index (`SIM-02-J`).
        self.households
            .adopt_loaded_topology(&self.transit_network, &self.region_graph);
        self.resource_extraction = loaded.resource_extraction;
        self.agriculture = loaded.agriculture;
        self.agents = loaded.agents;
        self.agents.settlement_days = loaded.scheduler.agent_settlement_days;
        let (front_m, back_mm, sweep_edges, next_edge, measured) = loaded.scheduler.road_length;
        self.road_length = super::core::RoadLengthRoll {
            front_m,
            back_mm,
            sweep_edges,
            next_edge,
            measured,
        };
        self.environment_sweep = loaded.scheduler.environment;
        self.agents.rebuild_arrival_carriers();
        self.treasury = loaded.treasury;
        self.service_policy = loaded.service_policy;
        self.fiscal_policy = loaded.fiscal_policy;
        self.apply_service_funding_staffing_policy();
        self.refresh_loaded_job_funding();
        self.refresh_demand_construction_queue(self.time.day_index, self.time.minute_of_day);
        self.refresh_loaded_demand_state_and_log();
        self.budget_history = loaded.budget_history;
        self.budget_last_lifetime_build_cost = self.treasury.lifetime_build_cost;
        self.debug_household_admissions_since_daily = 0;
        self.time.speed_multiplier = 0.0;
        self.transit_network.flow_fields.mark_all_dirty();
        self.clear_world_transient_state();
        self.heightmap.mark_all_render_patches_dirty();
        self.refresh_all_engineered_terrain_patch_state(
            crate::nodes::sim::core::ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
        );
        self.terrain_dirty = true;
        self.water_dirty = true;
        self.mark_network_render_dirty();
        // Loading advances the render-facing road generation after the persisted
        // surface was rebuilt. Stamp a matching mesh generation before water and
        // refined-terrain payload workers consume the refreshed snapshot.
        self.precompute_road_mesh_data();
        // The census input copy's pending row changes replace those the reconstruction above
        // recorded (`SIM-02-I`).
        self.households
            .census_inputs
            .finish_load(&mut self.allocator);
        // A census current at the save is recomputed whole from the loaded copy (`SIM-02-I`).
        self.households.complete_loaded_census(
            &self.allocator,
            &self.transit_network,
            &self.region_graph,
        );
        // Saves leave generated cells out; generate them now rather than in the first lot
        // regions served (`SIM-02-K`).
        self.warm_loaded_cells();
        // So are the flow fields and the movement scratch the first frame would build.
        self.prepare_loaded_pathing();
        // And what the first planning slot would prepare on the live books (`SIM-02-K`).
        self.warm_loaded_planning();
        // And the first agent tick's scrub and lane buckets.
        self.agents
            .prepare_loaded_tick(&self.allocator, &self.transit_network);
    }

    fn refresh_loaded_demand_state_and_log(&mut self) {
        self.demand
            .set_admission_candidate_hour(self.time.day_index, self.time.minute_of_day);
        // The household diagnostics are the saved ones, as the saved game showed them; a
        // recount here would differ from the uninterrupted game's (`SIM-02-K`).
        if !crate::debug::Channel::Spawn.enabled() {
            return;
        }
        let admission = self.demand.last_admission_debug_summary();
        debug_log!(
            "spawn",
            "load growth diagnostics: admit={} service_funding=electricity:{:.2} buildings={} households={} vacant_slots={} open_jobs={} move_in_jobs={} incoming_need={:.2}",
            self.demand.households_to_admit_today,
            self.service_policy.electricity_funding,
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|household| household.member_count > 0)
                .count(),
            admission.vacant_household_slots,
            admission.open_job_slots,
            admission.move_in_job_slots,
            admission.incoming_household_need,
        );
    }
}

// The saved wheel resumes at the slot after the saved clock: a game saves between frames, with
// the slot in progress already processed.
fn restore_events(
    scheduler: &SavedScheduler,
    time: &TimeSystem,
) -> Result<TimingWheel<SimEvent>, String> {
    if scheduler.next_slot != time.absolute_slot() + 1 {
        return Err("saved timing wheel does not match the saved clock".into());
    }
    let events = scheduler
        .events
        .iter()
        .map(|&(slot, seq, kind, a)| {
            SimEvent::from_saved(kind, a)
                .map(|event| TimedEvent { slot, seq, event })
                .ok_or_else(|| format!("unknown timed event kind {kind}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    TimingWheel::restore(scheduler.next_slot, scheduler.next_event_seq, events)
}
