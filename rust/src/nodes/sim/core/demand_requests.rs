// SPDX-License-Identifier: GPL-2.0-only

//! The demand request queues (`SIM-02-J`, `docs/simulation.md#request-queues`): level changes,
//! building removals, household departures and admissions.
//!
//! The hour's actions slot (minute 3's first slot) proves the hour's level changes and decides
//! its admissions on the census published at slot 0; these queues carry them out over the hour,
//! after the household queues. Building and household swap-removes run only in the hour's
//! removal window, slots 64 (minute 4) to 479 ([`removals_allowed`]): earlier they would
//! invalidate the published books the later planning slots read, later they would restart the
//! census in its window. Over the window the census input copy is filling and mirrors each
//! swap-remove. A removal requested at any other slot waits for the window's next start.
//!
//! - `level changes` applies proved changes from the removal window's start on, oldest first,
//!   each only while its building still has the door, profile, level and asset its permit was
//!   proved for. An in-place change is a changed row for the census input copy, not a
//!   structural change, so it restarts no census.
//! - `building removals` removes planned clearances and buildings whose compatibility expired at
//!   their daily phase, each after checking again that it still should go, with every reference
//!   repair of a removal.
//! - `departures` removes the households whose daily phase claimed a departure, paced by the
//!   daily exit credit, each after checking again that it may still leave.
//! - `admissions` creates the hour's admitted households and their arrival carriers, paced by a
//!   credit over the hour; each carrier departs paced from its creation.

use super::frame::EconomyObserver;
use super::state::SimCore;
use crate::debug::work::{Work, add as add_work};
use crate::debug_log;
use crate::simulation::buildings::allocator::{
    BuildingRemovalKind, BuildingRemovalRequest, BuildingSiteEnvironment,
};
use crate::simulation::economy::demand::{DemandBuildingActionPlan, LevelChangeRefusal};
use crate::simulation::economy::households::QueuedAdmission;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::RoadSurfaceCompileReason;
use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};

#[cfg(doc)]
use crate::simulation::economy::hiring::removals_allowed;

/// Proved level changes the `level changes` queue applies per slot (`SIM-02-J`).
pub(super) const LEVEL_CHANGE_QUOTA: u32 = 8;

/// Building removals the `building removals` queue serves per slot of the removal window
/// (`SIM-02-J`).
pub(super) const BUILDING_REMOVAL_QUOTA: u32 = 8;

/// Household departures the `departures` queue serves per slot of the removal window
/// (`SIM-02-J`): the daily exit credit lets at most 48 a day leave.
pub(super) const DEPARTURE_QUOTA: u32 = 1;

// Whether `node` still accepts arrivals: a border with a car road on it, as admission chose it.
fn border_accepts_arrivals(graph: &RegionGraph, node: u32) -> bool {
    (node as usize) < graph.node_count()
        && graph.node(node).node_type == NodeType::Border
        && graph.node_adjacency(node).iter().any(|&edge_idx| {
            let edge = graph.edge(edge_idx);
            !edge.deleted
                && edge.primary_type == TransitType::Road
                && (edge.allowed_types & TransitFlags::CAR) != 0
        })
}

impl SimCore {
    /// Queues the hour plan's clearances for the `building removals` queue, in plan order
    /// (`SIM-02-J`): each planned building that still stands as planned. O(despawns).
    pub(super) fn queue_hour_clearances(&mut self, plan: &DemandBuildingActionPlan, slot: u64) {
        let allocator = &mut self.allocator;
        allocator
            .building_handles
            .register_appended(allocator.buildings.len());
        for use_plan in [&plan.residential, &plan.commercial, &plan.industrial] {
            for action in &use_plan.despawns {
                let Some(building_idx) =
                    allocator.resolve_demand_building_action(action, &self.zoning)
                else {
                    continue;
                };
                let building = allocator.building_handles.handle(building_idx);
                allocator
                    .removal_requests
                    .push_back(BuildingRemovalRequest {
                        slot,
                        building,
                        kind: BuildingRemovalKind::Clearance,
                    });
            }
        }
    }

    /// Decides the hour's admissions at the actions slot against the workforce calendar and
    /// queues them, in admission order, for the `admissions` queue (`SIM-02-J`). Each admitted
    /// family claims its home now and counts as a pending arrival until it is created. Returns
    /// how many were admitted. O(admission batch).
    pub(super) fn decide_hour_admissions(&mut self) -> u32 {
        let mut admitted = Vec::new();
        let demand = &mut self.demand;
        let fiscal_policy = &self.fiscal_policy;
        let count = self.allocator.decide_demand_household_admission(
            demand.households_to_admit_today,
            demand.admission_candidate_seed,
            &self.transit_network,
            &self.region_graph,
            |composition, members, home_level| {
                demand.claim_household_admission(composition, members, home_level, fiscal_policy)
            },
            &mut admitted,
        );
        let allocator = &mut self.allocator;
        allocator
            .building_handles
            .register_appended(allocator.buildings.len());
        for &(_, _, composition, _) in &admitted {
            self.households.record_pending_arrival(composition);
        }
        self.households.queue_admissions(admitted.into_iter().map(
            |(home, size, composition, border)| QueuedAdmission {
                home: allocator.building_handles.handle(home),
                size,
                composition,
                border_node: border,
            },
        ));
        count
    }

    /// One slot of the `admissions` queue (`SIM-02-J`): accrues the admission credit and
    /// creates the admissions it pays for, oldest first, each as an arrival carrier at its
    /// border; the carriers created in the slot depart paced over the hour after it. An
    /// admission whose home left is dropped; one whose border no longer takes arrivals releases
    /// its home. O(1) with nothing queued, else O(admissions created).
    pub(super) fn serve_admission_queue(&mut self, slot: u64) {
        if !self.households.accrue_admission_credit() {
            return;
        }
        let mut created = 0;
        while let Some(admission) = self.households.pop_due_admission() {
            created += 1;
            let Some(home) = self.allocator.building_handles.resolve(admission.home) else {
                add_work(Work::AdmissionsDropped, 1);
                continue;
            };
            let border = if (admission.border_node as usize) < self.region_graph.node_count() {
                self.region_graph.get_valid_node(admission.border_node)
            } else {
                admission.border_node
            };
            if !border_accepts_arrivals(&self.region_graph, border) {
                self.allocator.release_vacancy(home);
                add_work(Work::AdmissionsDropped, 1);
                continue;
            }
            let position = self.region_graph.node(border).pos;
            self.agents.spawn_household_arrival_carrier(
                home,
                admission.size,
                admission.composition,
                border,
                position.x,
                position.z,
            );
        }
        let waiting = self.households.admission_requests.len();
        self.households
            .admission_stats
            .record_slot(created, waiting);
        self.schedule_arrival_departures(slot);
    }

    /// One slot of the `departures` queue (`SIM-02-J`): up to `quota` claimed departures; the
    /// caller serves it only in the hour's removal window. O(1) with nothing queued.
    pub(super) fn serve_departure_queue(&mut self, quota: usize) {
        let removed = self.households.serve_departures(
            quota,
            self.demand.household_exit_policy(),
            &mut self.agents,
            &mut self.allocator,
        );
        if removed > 0 {
            self.demand.record_household_removal_execution(removed);
        }
    }

    /// One slot of the `building removals` queue (`SIM-02-J`): up to `quota` removals, oldest
    /// first, each checked again (a clearance that may no longer be cleared, or a building that
    /// recovered its compatibility, stays), then the derived caches are repaired once: as the
    /// actions slot repaired clearances, or as the daily phases repaired an incompatible
    /// building's removal. The caller serves it only in the hour's removal window. O(1) with
    /// nothing queued, else O(quota) removals with their parallel O(A + H) remaps plus the cache
    /// repair: O(V log V) for a zone index the clearances kept (`SIM-02-K`), else O(B).
    pub(super) fn serve_building_removal_queue(
        &mut self,
        quota: usize,
        observer: &mut impl EconomyObserver,
    ) {
        if self.allocator.removal_requests.is_empty() {
            return;
        }
        // Clearances name their changed rows, so building-indexed tables follow them
        // incrementally; an incompatible building's removal does not, so its slot bumps the
        // building revision and every such table rebuilds, as the daily phases did.
        let [mut cleared, mut expired_any] = [false; 2];
        let mut taken = 0;
        while taken < quota
            && let Some(request) = self.allocator.removal_requests.pop_front()
        {
            taken += 1;
            let Some(building_idx) = self.allocator.building_handles.resolve(request.building)
            else {
                continue;
            };
            match request.kind {
                BuildingRemovalKind::Clearance => {
                    cleared |= self.allocator.remove_cleared_building(
                        building_idx,
                        &mut self.zoning,
                        &mut self.agents,
                        &mut self.households,
                        &mut self.logistics,
                        &mut self.treasury.balance,
                        &self.region_graph,
                        &self.transit_network,
                    )
                }
                BuildingRemovalKind::Incompatible => {
                    let expired = self.allocator.compatibility_expired(
                        building_idx,
                        0,
                        &self.zoning,
                        &self.region_graph,
                    );
                    if expired {
                        self.allocator.remove_incompatible_building(
                            building_idx,
                            &mut self.zoning,
                            &mut self.agents,
                            &mut self.households,
                            &mut self.logistics,
                            &mut self.treasury.balance,
                        );
                    }
                    expired_any |= expired;
                }
            }
        }
        let waiting = self.allocator.removal_requests.len();
        self.allocator.removal_stats.record_slot(taken, waiting);
        if expired_any {
            self.finish_building_removals(observer);
        } else if cleared {
            // As the actions slot repaired its clearances: the zone index a rebuild gives, which
            // also returns the vacancy lists to building order (`ALLOC-03`); each removal kept
            // the lists of a current index in step, so only that order and the admission order
            // are recomputed, O(V log V), else it is rebuilt, O(B) (`SIM-02-K`). Each removal
            // kept a clean entrance cache in step too; only a stale one is rebuilt, O(B).
            self.allocator.rebuild_zone_index_after_removals();
            if self.allocator.entrances_dirty
                || self.allocator.entrances.len() != self.allocator.buildings.len()
            {
                self.allocator
                    .rebuild_entrance_cache(&self.region_graph, &self.transit_network.lane_system);
            }
            // The sites' terrain render patches follow in later slots (`SIM-02-K`).
            self.publish_pending_building_site_changes_deferring_terrain();
            observer.buildings_changed(self);
        }
    }

    /// One slot of the `level changes` queue (`SIM-02-J`): up to `quota` proved level changes,
    /// oldest first. Each is applied only while its building has the door, profile, level and
    /// asset its permit names and the target still fits; a stale permit is rejected and nothing
    /// is proved again. After a slot that applied any, the zone index and the entrance cache are
    /// rebuilt once, as the actions slot rebuilt them. The caller does not serve it in the
    /// hour's planning slots. O(1) with nothing queued, else O(quota) site checks plus O(B)
    /// for the repairs.
    pub(super) fn serve_level_change_queue(
        &mut self,
        quota: usize,
        observer: &mut impl EconomyObserver,
    ) {
        if self.demand.level_change_requests.is_empty() {
            return;
        }
        let cell_m = self.config.zone_cell_m;
        self.allocator.prepare_building_site_query_index(cell_m);
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::SimCommit,
        );
        let catalog = self.demand.shared_runtime_catalog();
        let mut applied_any = false;
        let mut taken = 0;
        while taken < quota
            && let Some(request) = self.demand.level_change_requests.pop_front()
        {
            taken += 1;
            let building_idx = match self.demand.check_level_change_permit(
                &request,
                &self.allocator,
                &self.region_graph,
                &self.transit_network.lane_system,
            ) {
                Ok(building_idx) => building_idx,
                Err(refusal) => {
                    if refusal == LevelChangeRefusal::StalePermit {
                        add_work(Work::LevelChangePermitRejections, 1);
                    }
                    debug_log!(
                        "economy",
                        "queued level change rejected: target={} reason={:?}",
                        request.target_asset_id,
                        refusal
                    );
                    continue;
                }
            };
            let applied = self
                .allocator
                .apply_level_change_action(
                    building_idx,
                    &request.target_asset_id,
                    &catalog,
                    cell_m,
                    &self.region_graph,
                    BuildingSiteEnvironment {
                        road_surface: &self.transit_network.road_surface,
                        terrain: &self.heightmap,
                    },
                )
                .is_some();
            if !applied {
                debug_log!(
                    "economy",
                    "queued level change no longer fits: building={} target={}",
                    building_idx,
                    request.target_asset_id
                );
            }
            applied_any |= applied;
        }
        let waiting = self.demand.level_change_requests.len();
        self.demand.level_change_stats.record_slot(taken, waiting);
        if applied_any {
            // Claims since the last rebuild swap-removed vacancies out of building order, and
            // admission breaks ties by vacancy position (`ALLOC-03`).
            self.allocator.rebuild_zone_index();
            self.allocator
                .rebuild_entrance_cache(&self.region_graph, &self.transit_network.lane_system);
            self.publish_pending_building_site_changes();
            observer.buildings_changed(self);
        }
    }

    /// Logs and resets the hour's level-change and building-removal queue service, at the
    /// hour's last slot (`SIM-02-J`).
    pub(super) fn close_demand_queue_hour(&mut self) {
        for (name, stats) in [
            (
                "level_change_requests_hour",
                std::mem::take(&mut self.demand.level_change_stats),
            ),
            (
                "building_removal_requests_hour",
                std::mem::take(&mut self.allocator.removal_stats),
            ),
        ] {
            if stats.taken > 0 {
                debug_log!(crate::debug::Channel::EconomyPhases, "{} {}", name, stats);
            }
        }
    }
}
