//! Authoritative simulation state and deterministic cadence transitions.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use super::budget::{
    CityServicePolicy, CityTreasury, DailyBudgetLedgerEntry, ROAD_UPKEEP_PER_METER_PER_DAY,
};
use super::snapshot::SimulationSnapshot;
use super::terrain_payloads::{
    CachedRefinedTerrainPatch, ROAD_LOCKED_TERRAIN_RENDER_STEP_M, RefinedTerrainAssemblyLedger,
    RefinedTerrainPatchCacheKey,
};
use super::water_preview::{AuthoredWaterPatchFillDebug, WorldLakeFillPreview};
use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::demand::{
    DemandBuildingActionPlan, DemandSpawnAction, DemandSystem,
};
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::render::NetworkMeshData;
use crate::simulation::network::surface::RoadSurfaceCompileReason;
use crate::simulation::terrain::{TerrainSystem, terrain_cdt_local_sample_margin_m};
use crate::simulation::water::WaterSystem;
use crate::simulation::world_definition::{AuthoredLakeFill, AuthoredOpenWaterFill};
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use godot::prelude::Vector3;

const MINUTES_PER_DAY_U64: u64 = 24 * 60;
const DEMAND_SPAWN_ACTIONS_PER_MINUTE: usize = 1;

/// Demand-planned private building spawn delayed to a later authored minute.
#[derive(Clone, Debug)]
pub(crate) struct PendingDemandSpawnAction {
    /// Absolute authored minute when this spawn may be released.
    pub(crate) due_minute: u64,
    /// Demand bucket that selected this spawn.
    pub(crate) zone_type: ZoneType,
    /// Final parcel and asset placement request.
    pub(crate) action: DemandSpawnAction,
    /// Operational day when demand originally planned the spawn.
    pub(crate) planned_day_index: u32,
    /// Minute of day when demand originally planned the spawn.
    pub(crate) planned_minute_of_day: u16,
}

#[derive(Debug)]
pub(super) struct BulkRoadGeometryFinalize {
    pub(super) dirty_edges: HashSet<usize>,
    pub(super) affected_nodes: HashSet<u32>,
    pub(super) profile_us: u128,
    pub(super) regrade_us: u128,
    pub(super) clips_us: u128,
}

pub(super) fn absolute_operational_minute(day_index: u32, minute_of_day: u16) -> u64 {
    u64::from(day_index.saturating_sub(1)) * MINUTES_PER_DAY_U64 + u64::from(minute_of_day)
}

pub(super) fn demand_plan_has_non_spawn_actions(plan: &DemandBuildingActionPlan) -> bool {
    [&plan.residential, &plan.commercial, &plan.industrial]
        .iter()
        .any(|use_plan| {
            !use_plan.despawns.is_empty()
                || !use_plan.downgrades.is_empty()
                || !use_plan.upgrades.is_empty()
        })
}

pub(super) fn demand_plan_without_spawns(
    plan: &DemandBuildingActionPlan,
) -> DemandBuildingActionPlan {
    let mut immediate_plan = plan.clone();
    immediate_plan.residential.spawns.clear();
    immediate_plan.commercial.spawns.clear();
    immediate_plan.industrial.spawns.clear();
    immediate_plan
}

/// All simulation state — owned exclusively by the background sim thread when running.
///
/// The main thread accesses this via `Arc<Mutex<SimCore>>`. The lock is held for at
/// most one tick duration (~7 ms at 100 k agents) per mutation.
pub struct SimCore {
    /// Simulation clock and day counter.
    pub time: TimeSystem,
    /// Terrain heightmap.
    pub heightmap: TerrainSystem,
    /// Shallow-water simulation.
    pub watermap: WaterSystem,
    /// Road topology graph.
    pub region_graph: crate::simulation::network::graph::RegionGraph,
    /// Lane system, CCH pathfinder, and road mutation helpers.
    pub transit_network: TransitNetwork,
    /// Road-aligned parcel zoning.
    pub zoning: ZoningSystem,
    /// Pollution diffusion grid.
    pub pollution: PollutionSystem,
    /// Traffic noise grid.
    pub noise: NoiseSystem,
    /// Composite desirability grid.
    pub desirability: DesirabilitySystem,
    /// Global R/C/I demand counters.
    pub demand: DemandSystem,
    /// Demand building spawns scheduled across authored minutes to avoid hourly bursts.
    pub(crate) pending_demand_spawns: VecDeque<PendingDemandSpawnAction>,
    /// Building placement and vacancy index.
    pub allocator: BuildingAllocator,
    /// Agent FSM in Structure-of-Arrays layout.
    pub agents: AgentSystem,
    /// Explicit household runtime records and first-pass daily economy logic.
    pub households: HouseholdSystem,
    /// Active building-level freight reservations and delayed deliveries.
    pub logistics: ShipmentSystem,
    /// World configuration (extent, chunk metadata, cell sizes).
    pub config: WorldConfig,
    /// City-level fiscal ledger tracking infrastructure build cost and daily upkeep.
    pub treasury: CityTreasury,
    /// Player-controlled live service funding policies.
    pub(crate) service_policy: CityServicePolicy,
    /// Completed daily budget ledger entries for overview windows and trend graphs.
    pub(crate) budget_history: VecDeque<DailyBudgetLedgerEntry>,
    /// Lifetime build cost observed when the most recent budget ledger entry was recorded.
    pub(crate) budget_last_lifetime_build_cost: f64,
    /// Runtime-only economy debug counter reset after each daily diagnostic line.
    pub(crate) debug_household_admissions_since_daily: u32,
    /// Undo history stack — kept in SimCore so all mutations are co-located.
    pub(crate) undo_stack: VecDeque<SimulationSnapshot>,
    /// Authored-world lake fill records when editing or playing from a `WorldDefinition`.
    pub(crate) world_lake_fills: Vec<AuthoredLakeFill>,
    /// Authored-world edge-connected open-water fills when editing or playing from a `WorldDefinition`.
    pub(crate) world_open_water_fills: Vec<AuthoredOpenWaterFill>,
    /// Transient world-editor lake-fill preview. Never saved into `WorldDefinition`.
    pub(crate) world_lake_fill_preview: Option<WorldLakeFillPreview>,
    /// Cached authored-water fill debug summaries keyed by water render patch.
    pub(crate) authored_water_patch_fill_debug_cache:
        HashMap<(usize, usize), Vec<AuthoredWaterPatchFillDebug>>,
    /// True while the world editor is accumulating one terrain brush stroke.
    pub(crate) terrain_stroke_active: bool,
    /// True once the active terrain brush stroke has applied at least one terrain mutation.
    pub(crate) terrain_stroke_has_changes: bool,
    /// Set by terrain mutations until Godot acknowledges the exact dirty patch revisions.
    pub terrain_dirty: bool,
    /// Set by water mutations until Godot acknowledges the exact dirty patch revisions.
    pub water_dirty: bool,
    /// Set by any network mutation until `NetworkRenderer` acknowledges the published generation.
    pub network_dirty: bool,
    /// True when running in benchmark mode (skips undo stack on road placement).
    pub benchmark_mode: bool,
    /// Duration of the last daily economy tick in milliseconds.
    pub last_tick_duration: f64,
    /// Duration of the last agent movement tick in microseconds.
    pub last_agent_tick_us: u64,
    /// Per-phase timing breakdown from the last road placement, for profiling.
    pub last_road_timing: String,
    /// Edge ids touched by the most recent committed network edit and queued for one focused
    /// road-surface debug dump after the next terrain/mesh rebuild.
    pub(crate) last_surface_debug_edges: Vec<usize>,
    /// Production refined terrain patches precomputed by the sim thread for Godot upload.
    pub(crate) refined_terrain_patch_cache:
        HashMap<RefinedTerrainPatchCacheKey, Arc<CachedRefinedTerrainPatch>>,
    /// Sorted terrain render patches that must use road-locked refined terrain meshes.
    pub(crate) road_locked_terrain_patch_keys: Vec<(usize, usize)>,
    /// Required grading margin for each road-locked terrain render patch.
    pub(crate) road_locked_terrain_patch_margins: BTreeMap<(usize, usize), f32>,
    /// Terrain patches owned by at least one indexed building site.
    pub(crate) building_site_owned_terrain_patch_keys: HashSet<(usize, usize)>,
    /// Sorted terrain patches owned by roads, building sites, or both.
    pub(crate) engineered_terrain_patch_keys: Vec<(usize, usize)>,
    /// Required CDT query margin for every engineered-owned terrain patch.
    pub(crate) engineered_terrain_patch_margins: BTreeMap<(usize, usize), f32>,
    /// Monotonic source revision allocator for asynchronous terrain payloads.
    pub(crate) terrain_payload_generation_counter: u64,
    /// Latest source revision that invalidates every terrain render patch.
    pub(crate) terrain_payload_global_generation: u64,
    /// Latest building-site source revision for individually affected render patches.
    pub(crate) terrain_payload_patch_generations: HashMap<(usize, usize), u64>,
    /// Generation-stamped full/local refined-terrain assembly scopes by render patch.
    pub(crate) refined_terrain_assembly_ledgers:
        HashMap<(usize, usize), RefinedTerrainAssemblyLedger>,
    /// Latest full road mesh generated by the sim thread after a network edit.
    pub(crate) cached_road_mesh_data: Option<Arc<NetworkMeshData>>,
    /// Road-tool surface generation represented by the cached road mesh.
    pub(crate) cached_road_mesh_generation: u64,
    /// Cached world-space positions of live canonical network nodes for render snapshots.
    pub(crate) cached_network_node_positions: Arc<Vec<Vector3>>,
    /// True when network topology changed and the cached node-position snapshot must rebuild.
    pub(crate) cached_network_node_positions_dirty: bool,
    /// Monotonic stamp for road-tool terrain/surface snapshots.
    pub(crate) road_tool_surface_generation: u64,
    /// World-space AABB for frustum culling: (x_min, x_max, z_min, z_max).
    /// Agents outside this rect are excluded from `RenderSnapshot` transforms.
    /// Updated each frame via `SimCommand::SetCameraAabb`. Defaults to "show all".
    pub camera_aabb: (f32, f32, f32, f32),
}

impl SimCore {
    /// Invalidates immutable road-tool query snapshots without forcing terrain payload rebuilds.
    pub(crate) fn bump_road_tool_query_generation(&mut self) {
        self.road_tool_surface_generation =
            self.road_tool_surface_generation.wrapping_add(1).max(1);
    }

    fn next_terrain_payload_generation(&mut self) -> u64 {
        self.terrain_payload_generation_counter = self
            .terrain_payload_generation_counter
            .wrapping_add(1)
            .max(1);
        self.terrain_payload_generation_counter
    }

    pub(crate) fn bump_global_terrain_payload_generation(&mut self) {
        self.terrain_payload_global_generation = self.next_terrain_payload_generation();
    }

    pub(crate) fn bump_terrain_payload_patch_generations(&mut self, patch_keys: &[(usize, usize)]) {
        if patch_keys.is_empty() {
            return;
        }
        let generation = self.next_terrain_payload_generation();
        for &key in patch_keys {
            self.terrain_payload_patch_generations
                .insert(key, generation);
            self.refined_terrain_assembly_ledgers
                .entry(key)
                .or_default()
                .full_dirty_at = Some(generation);
        }
    }

    /// Advances patch revisions while retaining fixed query-chunk scope for local road edits.
    pub(crate) fn bump_local_road_terrain_payload_generations(
        &mut self,
        patch_keys: &[(usize, usize)],
        query_chunks: &[(i32, i32)],
    ) {
        if patch_keys.is_empty() {
            return;
        }
        if query_chunks.is_empty() {
            self.bump_terrain_payload_patch_generations(patch_keys);
            return;
        }
        let generation = self.next_terrain_payload_generation();
        for &key in patch_keys {
            self.terrain_payload_patch_generations
                .insert(key, generation);
            let ledger = self
                .refined_terrain_assembly_ledgers
                .entry(key)
                .or_default();
            for &chunk in query_chunks {
                ledger.road_query_chunk_dirty_at.insert(chunk, generation);
            }
        }
    }

    pub(crate) fn terrain_payload_generation_for_patch(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> u64 {
        self.terrain_payload_patch_generations
            .get(&(patch_x, patch_z))
            .copied()
            .unwrap_or(0)
            .max(self.terrain_payload_global_generation)
    }

    pub(crate) fn terrain_dirty_patch_states(&self) -> Vec<(usize, usize, u64)> {
        let mut states = self
            .heightmap
            .dirty_render_patches()
            .iter()
            .map(|&(patch_x, patch_z)| {
                (
                    patch_x,
                    patch_z,
                    self.terrain_payload_generation_for_patch(patch_x, patch_z),
                )
            })
            .collect::<Vec<_>>();
        states.sort_unstable();
        states
    }

    /// Acknowledges one terrain upload only when it matches the current patch revision.
    pub(crate) fn acknowledge_terrain_render_patch(
        &mut self,
        patch_x: usize,
        patch_z: usize,
        generation: u64,
    ) -> bool {
        if self.terrain_payload_generation_for_patch(patch_x, patch_z) != generation {
            return false;
        }
        self.heightmap.clear_render_patch_dirty(patch_x, patch_z);
        let key = (patch_x, patch_z);
        if let Some(ledger) = self.refined_terrain_assembly_ledgers.get_mut(&key) {
            if ledger
                .full_dirty_at
                .is_some_and(|stamp| stamp <= generation)
            {
                ledger.full_dirty_at = None;
            }
            ledger
                .road_query_chunk_dirty_at
                .retain(|_, stamp| *stamp > generation);
            if ledger.full_dirty_at.is_none() && ledger.road_query_chunk_dirty_at.is_empty() {
                self.refined_terrain_assembly_ledgers.remove(&key);
            }
        }
        true
    }

    fn mark_network_visuals_dirty(&mut self) {
        self.bump_road_tool_query_generation();
        self.network_dirty = true;
        self.cached_network_node_positions_dirty = true;
    }

    /// Marks network visuals and every terrain payload dirty after a world-wide reset.
    pub(crate) fn mark_network_render_dirty(&mut self) {
        self.bump_global_terrain_payload_generation();
        self.cached_road_mesh_data = None;
        self.cached_road_mesh_generation = 0;
        self.mark_network_visuals_dirty();
    }

    /// Marks network visuals dirty while terrain payload revisions remain patch-local.
    pub(crate) fn mark_local_network_render_dirty(&mut self) {
        self.mark_network_visuals_dirty();
    }

    /// Acknowledges one network upload only when no newer surface mutation exists.
    pub(crate) fn acknowledge_network_render_generation(&mut self, generation: u64) -> bool {
        if self.road_tool_surface_generation != generation {
            return false;
        }
        self.network_dirty = false;
        true
    }

    pub(super) fn finalize_bulk_road_geometry_for_dirty_edges(
        &mut self,
    ) -> BulkRoadGeometryFinalize {
        let mut dirty_edges = std::mem::take(&mut self.transit_network.bulk_dirty_edges);
        let mut affected_nodes: HashSet<u32> = self
            .transit_network
            .road_surface
            .dirty_nodes()
            .iter()
            .copied()
            .map(|node_id| self.region_graph.get_valid_node(node_id))
            .collect();

        for &edge_idx in &dirty_edges {
            if edge_idx >= self.region_graph.edge_count()
                || self.region_graph.edge(edge_idx).deleted
            {
                continue;
            }
            let edge = self.region_graph.edge(edge_idx);
            affected_nodes.insert(self.region_graph.get_valid_node(edge.start_node));
            affected_nodes.insert(self.region_graph.get_valid_node(edge.end_node));
        }

        let profile_start = Instant::now();
        let profile_changed_edges = self.transit_network.solve_dirty_junction_endpoint_profiles(
            &mut self.region_graph,
            &affected_nodes,
            &dirty_edges,
        );
        let profile_us = profile_start.elapsed().as_micros();
        dirty_edges.extend(profile_changed_edges);

        let regrade_start = Instant::now();
        let regrade_changed_edges = self
            .transit_network
            .regrade_dirty_junction_endpoint_profiles(
                &mut self.region_graph,
                &affected_nodes,
                &dirty_edges,
            );
        let regrade_us = regrade_start.elapsed().as_micros();
        dirty_edges.extend(regrade_changed_edges);

        self.transit_network.mark_surface_dirty_from_sets(
            &self.region_graph,
            &dirty_edges,
            &affected_nodes,
        );

        let clips_start = Instant::now();
        self.region_graph
            .rebuild_intersection_clips_for_nodes(&affected_nodes);
        let clips_us = clips_start.elapsed().as_micros();

        BulkRoadGeometryFinalize {
            dirty_edges,
            affected_nodes,
            profile_us,
            regrade_us,
            clips_us,
        }
    }
}

impl SimCore {
    /// Executes one coarse operational-hour economy step before the daily settlement boundary.
    pub fn simulate_operational_hour_internal(&mut self, day_index: u32, minute_of_day: u16) {
        let absolute_hour = day_index
            .saturating_sub(1)
            .saturating_mul(24)
            .saturating_add(u32::from(minute_of_day / 60));
        self.allocator.advance_construction_hour();
        let service_funding_by_building = self.electricity_funding_by_building();
        let fiscal_revenue = self.households.operational_hour_tick(
            &mut self.agents,
            &mut self.allocator,
            &mut self.logistics,
            &self.transit_network,
            &self.region_graph,
            absolute_hour,
            minute_of_day,
            &mut self.treasury.balance,
            &service_funding_by_building,
        );
        self.collect_fiscal_revenue(fiscal_revenue);
        if minute_of_day != 0 {
            self.execute_hourly_demand_pass(day_index, minute_of_day, &service_funding_by_building);
        }
    }

    fn enqueue_hourly_demand_spawns(&mut self, day_index: u32, minute_of_day: u16) -> usize {
        let now = absolute_operational_minute(day_index, minute_of_day);
        let first_due_minute = self
            .pending_demand_spawns
            .back()
            .map(|pending| pending.due_minute.saturating_add(1))
            .unwrap_or_else(|| now.saturating_add(1))
            .max(now.saturating_add(1));
        let mut queued = 0_usize;

        for (zone_type, spawns) in [
            (
                ZoneType::Residential,
                &self.demand.building_actions.residential.spawns,
            ),
            (
                ZoneType::Commercial,
                &self.demand.building_actions.commercial.spawns,
            ),
            (
                ZoneType::Industrial,
                &self.demand.building_actions.industrial.spawns,
            ),
        ] {
            for action in spawns {
                let due_minute = first_due_minute
                    .saturating_add((queued / DEMAND_SPAWN_ACTIONS_PER_MINUTE) as u64);
                self.pending_demand_spawns
                    .push_back(PendingDemandSpawnAction {
                        due_minute,
                        zone_type,
                        action: action.clone(),
                        planned_day_index: day_index,
                        planned_minute_of_day: minute_of_day,
                    });
                queued += 1;
            }
        }

        if queued > 0 {
            let last_due_minute = first_due_minute
                .saturating_add(((queued - 1) / DEMAND_SPAWN_ACTIONS_PER_MINUTE) as u64);
            debug_log!(
                "economy",
                "queued demand spawns: day={} minute={} queued={} pending_total={} first_due={} last_due={}",
                day_index,
                minute_of_day,
                queued,
                self.pending_demand_spawns.len(),
                first_due_minute,
                last_due_minute,
            );
        }
        queued
    }

    pub(super) fn execute_pending_demand_spawns_for_minute(
        &mut self,
        day_index: u32,
        minute_of_day: u16,
    ) -> usize {
        let now = absolute_operational_minute(day_index, minute_of_day);
        let mut executed_this_minute = 0_usize;
        while executed_this_minute < DEMAND_SPAWN_ACTIONS_PER_MINUTE {
            let Some(pending) = self.pending_demand_spawns.front() else {
                return executed_this_minute;
            };
            if pending.due_minute > now {
                return executed_this_minute;
            }
            let Some(pending) = self.pending_demand_spawns.pop_front() else {
                return executed_this_minute;
            };
            self.execute_pending_demand_spawn(pending, day_index, minute_of_day);
            executed_this_minute += 1;
        }
        executed_this_minute
    }

    fn execute_pending_demand_spawn(
        &mut self,
        pending: PendingDemandSpawnAction,
        day_index: u32,
        minute_of_day: u16,
    ) {
        let total_start = Instant::now();
        let compile_start = Instant::now();
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::SimCommit,
        );
        let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;
        let execute_start = Instant::now();
        let execution = self.allocator.execute_single_demand_spawn_action(
            pending.zone_type,
            &pending.action,
            &mut self.zoning,
            &self.region_graph,
            &self.transit_network.lane_system,
            &self.transit_network.road_surface,
            &self.heightmap,
            self.demand.runtime_catalog(),
            self.demand.runtime_tuning(),
        );
        let execute_ms = execute_start.elapsed().as_secs_f64() * 1000.0;
        self.treasury
            .collect_property_tax(execution.property_tax_paid as f64);
        if let Some(bounds) = execution.site_dirty_bounds {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
        let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;

        let use_execution = match pending.zone_type {
            ZoneType::Residential => execution.residential,
            ZoneType::Commercial => execution.commercial,
            ZoneType::Industrial => execution.industrial,
            _ => return,
        };
        debug_log!(
            "economy",
            "queued demand spawn execution: planned_day={} planned_minute={} executed_day={} executed_minute={} zone={:?} attempted={} placed={} failed={} pending_remaining={} compile_ms={:.3} execute_ms={:.3} total_ms={:.3}",
            pending.planned_day_index,
            pending.planned_minute_of_day,
            day_index,
            minute_of_day,
            pending.zone_type,
            use_execution.spawn_attempted,
            use_execution.spawn_executed,
            use_execution.spawn_rejections.total(),
            self.pending_demand_spawns.len(),
            compile_ms,
            execute_ms,
            total_ms,
        );
    }

    /// Executes one full economy / daily tick (called once per in-game day).
    pub fn simulate_tick_internal(&mut self, day_index: u32) {
        let tick_start = Instant::now();

        debug_log!(
            "economy",
            "daily tick start: buildings={} households={} agents={}",
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|h| h.member_count > 0)
                .count(),
            self.agents.len(),
        );
        self.allocator.tick(
            &mut self.zoning,
            &mut self.agents,
            &mut self.households,
            &mut self.logistics,
            &mut self.transit_network,
            &mut self.region_graph,
        );
        if let Some(bounds) = self.allocator.take_pending_site_dirty_bounds() {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
        // Drain building dirty-zone flags → mark matching flow fields for rebuild.
        {
            use crate::simulation::buildings::allocator::BASELINE_PRIVATE_ZONES;
            for (zone_idx, zone) in BASELINE_PRIVATE_ZONES.iter().enumerate() {
                if self.allocator.dirty_zones[zone_idx] {
                    self.allocator.dirty_zones[zone_idx] = false;
                    self.transit_network.flow_fields.mark_zone_dirty(*zone);
                }
            }
        }

        self.pollution.tick(&self.allocator, &self.config);
        self.noise
            .tick(&self.allocator, &self.region_graph, &self.config);
        self.desirability
            .tick(&self.zoning, &self.pollution, &self.noise);
        let service_funding_by_building = self.electricity_funding_by_building();
        let fiscal_revenue = self.households.daily_settlement_tick(
            &mut self.agents,
            &mut self.allocator,
            &self.logistics,
            &self.transit_network,
            &self.region_graph,
            &mut self.treasury.balance,
            &service_funding_by_building,
        );
        self.collect_fiscal_revenue(fiscal_revenue);
        // City treasury: settle daily road upkeep on the fiscal cadence.
        let road_length_m: f64 = self
            .region_graph
            .edges()
            .iter()
            .filter(|e| !e.deleted)
            .map(|e| e.physical_length as f64)
            .sum();
        self.treasury
            .settle_daily_upkeep(road_length_m * ROAD_UPKEEP_PER_METER_PER_DAY);
        self.demand.run_daily_pass_with_service_funding(
            &self.allocator,
            &self.households,
            &self.region_graph,
            &self.zoning,
            self.treasury.balance,
            &service_funding_by_building,
        );
        let removed_households = self.households.execute_demand_household_removal(
            self.demand.households_to_remove_today,
            &mut self.agents,
            &mut self.allocator,
            &mut self.logistics,
        );
        self.demand
            .record_household_removal_execution(removed_households);
        self.demand
            .log_daily_household_action_diagnostics(day_index);
        self.execute_hourly_demand_pass(day_index, 0, &service_funding_by_building);
        // Minute 0 is the deterministic closing boundary: operational-hour work,
        // daily settlement, and midnight demand all post before the daily tax
        // buckets roll into the report.
        self.treasury.finalize_daily_tax_window();
        self.record_daily_budget_ledger(day_index);
        self.log_daily_city_flow_diagnostics(day_index, removed_households);
        self.debug_household_admissions_since_daily = 0;
        // Reset OWA/local input accumulators after the daily and midnight demand snapshots have
        // been taken.
        self.allocator.reset_daily_input_accumulators();
        debug_log!(
            "economy",
            "daily tick end: buildings={} households={} agents={} demand=(R {:+.0}%, C {:+.0}%, I {:+.0}%) admit={} remove={} planned_spawns=({}/{}/{}) upgrades=({}/{}/{}) downgrades=({}/{}/{}) despawns=({}/{}/{}) treasury={:.0}",
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|h| h.member_count > 0)
                .count(),
            self.agents.len(),
            self.demand.net_residential_pressure() * 100.0,
            self.demand.net_commercial_pressure() * 100.0,
            self.demand.net_industrial_pressure() * 100.0,
            self.demand.households_to_admit_today,
            self.demand.households_to_remove_today,
            self.demand.building_actions.residential.spawns.len(),
            self.demand.building_actions.commercial.spawns.len(),
            self.demand.building_actions.industrial.spawns.len(),
            self.demand.building_actions.residential.upgrades.len(),
            self.demand.building_actions.commercial.upgrades.len(),
            self.demand.building_actions.industrial.upgrades.len(),
            self.demand.building_actions.residential.downgrades.len(),
            self.demand.building_actions.commercial.downgrades.len(),
            self.demand.building_actions.industrial.downgrades.len(),
            self.demand.building_actions.residential.despawns.len(),
            self.demand.building_actions.commercial.despawns.len(),
            self.demand.building_actions.industrial.despawns.len(),
            self.treasury.balance,
        );
        self.agents.daily_update(&self.pollution, &self.config);
        self.agents
            .pathfind_count
            .store(0, std::sync::atomic::Ordering::Relaxed);

        self.last_tick_duration = tick_start.elapsed().as_secs_f64() * 1000.0;
    }

    pub(super) fn execute_hourly_demand_pass(
        &mut self,
        day_index: u32,
        minute_of_day: u16,
        service_funding_by_building: &[f32],
    ) {
        self.demand.run_hourly_pass_with_service_funding(
            &self.allocator,
            &self.households,
            &self.region_graph,
            &self.zoning,
            self.treasury.balance,
            service_funding_by_building,
        );
        let launched_households = self.allocator.execute_demand_household_admission(
            self.demand.households_to_admit_today,
            &mut self.agents,
            &self.transit_network,
            &self.region_graph,
        );
        self.debug_household_admissions_since_daily = self
            .debug_household_admissions_since_daily
            .saturating_add(launched_households);
        self.demand
            .record_household_admission_execution(launched_households);
        let queued_spawn_count = self.enqueue_hourly_demand_spawns(day_index, minute_of_day);
        let immediate_building_plan = demand_plan_without_spawns(&self.demand.building_actions);
        let building_action_execution =
            if demand_plan_has_non_spawn_actions(&immediate_building_plan) {
                self.allocator.execute_demand_building_actions(
                    &immediate_building_plan,
                    &mut self.zoning,
                    &mut self.agents,
                    &mut self.households,
                    &mut self.logistics,
                    &self.region_graph,
                    &self.transit_network.lane_system,
                    &self.transit_network.road_surface,
                    &self.heightmap,
                    self.demand.runtime_catalog(),
                    self.demand.runtime_tuning(),
                )
            } else {
                Default::default()
            };
        self.treasury
            .collect_property_tax(building_action_execution.property_tax_paid as f64);
        if let Some(bounds) = building_action_execution.site_dirty_bounds {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
        self.demand
            .log_hourly_household_action_diagnostics(day_index, minute_of_day);
        self.demand
            .log_hourly_building_action_diagnostics(day_index, minute_of_day);
        for (use_label, execution) in [
            ("Residential", &building_action_execution.residential),
            ("Commercial", &building_action_execution.commercial),
            ("Industrial", &building_action_execution.industrial),
        ] {
            let rejections = execution.spawn_rejections;
            if execution.spawn_attempted == 0 && rejections.total() == 0 {
                continue;
            }
            debug_log!(
                "economy",
                "building action execution: day={} minute={} use={} \
                 spawn_attempted={} spawn_placed={} spawn_failed={} \
                 spawn_failed_geometry={} fail_asset={} fail_parcel={} fail_slot={} \
                 fail_driveway_surface={} fail_driveway_height={} fail_driveway_connection={} \
                 fail_frontage_surface={} fail_neighbor_height={} fail_site_tie_in={}",
                day_index,
                minute_of_day,
                use_label,
                execution.spawn_attempted,
                execution.spawn_executed,
                rejections.total(),
                rejections.geometry_total(),
                rejections.asset_unavailable,
                rejections.parcel_unavailable,
                rejections.slot_unavailable,
                rejections.driveway_road_surface_missing,
                rejections.driveway_height_conflict,
                rejections.driveway_connection_missing,
                rejections.frontage_road_surface_missing,
                rejections.neighbor_site_height_conflict,
                rejections.site_support_tie_in_invalid,
            );
        }
        debug_log!(
            "economy",
            "hourly demand: day={} minute={} demand=(R {:+.0}%, C {:+.0}%, I {:+.0}%) admit={} planned_spawns=({}/{}/{}) queued_spawns={} pending_spawns={} placed_spawns=({}/{}/{}) upgrades=({}/{}/{}) downgrades=({}/{}/{}) despawns=({}/{}/{})",
            day_index,
            minute_of_day,
            self.demand.net_residential_pressure() * 100.0,
            self.demand.net_commercial_pressure() * 100.0,
            self.demand.net_industrial_pressure() * 100.0,
            self.demand.households_to_admit_today,
            self.demand.building_actions.residential.spawns.len(),
            self.demand.building_actions.commercial.spawns.len(),
            self.demand.building_actions.industrial.spawns.len(),
            queued_spawn_count,
            self.pending_demand_spawns.len(),
            building_action_execution.residential.spawn_executed,
            building_action_execution.commercial.spawn_executed,
            building_action_execution.industrial.spawn_executed,
            self.demand.building_actions.residential.upgrades.len(),
            self.demand.building_actions.commercial.upgrades.len(),
            self.demand.building_actions.industrial.upgrades.len(),
            self.demand.building_actions.residential.downgrades.len(),
            self.demand.building_actions.commercial.downgrades.len(),
            self.demand.building_actions.industrial.downgrades.len(),
            self.demand.building_actions.residential.despawns.len(),
            self.demand.building_actions.commercial.despawns.len(),
            self.demand.building_actions.industrial.despawns.len(),
        );
    }

    pub(crate) fn mark_building_site_terrain_dirty_bounds(&mut self, bounds: (f32, f32, f32, f32)) {
        let margin_m =
            terrain_cdt_local_sample_margin_m(&self.heightmap, ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
        self.allocator.mark_building_site_terrain_bounds_dirty(
            &mut self.heightmap,
            bounds,
            margin_m,
        );
        let (min_x, min_z, max_x, max_z) = bounds;
        let dirty_patch_keys = self.heightmap.render_patch_keys_for_world_bounds(
            min_x - margin_m,
            min_z - margin_m,
            max_x + margin_m,
            max_z + margin_m,
        );
        self.refresh_engineered_terrain_patch_ownership_for_keys(
            ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
            &dirty_patch_keys,
        );
        self.bump_terrain_payload_patch_generations(&dirty_patch_keys);
        self.terrain_dirty = true;
    }
}
