//! Undo/Redo system for simulation state.

use crate::nodes::sim::core::{
    BuildingRemovalUndo, SimCore, SimulationRuntimeSnapshot, SimulationSnapshot,
    WaterRuntimeSnapshot,
};
use crate::simulation::network::graph::RegionGraphUndoDelta;
use crate::simulation::zoning::ZoningParcelRemovalUndo;
use godot::prelude::Vector3;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

impl SimCore {
    /// Pushes a new state snapshot onto the undo stack.
    ///
    /// Parameters select which dense authoring systems are included in the snapshot.
    pub fn push_undo_state(&mut self, inc_terrain: bool, inc_water: bool, inc_trans_graph: bool) {
        self.push_undo_state_internal(inc_terrain, inc_water, inc_trans_graph, false);
    }

    /// Pushes an undo snapshot that may include building/economy runtime state.
    #[cfg(test)]
    pub(crate) fn push_undo_state_with_runtime(
        &mut self,
        inc_terrain: bool,
        inc_water: bool,
        inc_trans_graph: bool,
        inc_runtime: bool,
    ) {
        self.push_undo_state_internal(inc_terrain, inc_water, inc_trans_graph, inc_runtime);
    }

    fn push_undo_state_internal(
        &mut self,
        inc_terrain: bool,
        inc_water: bool,
        inc_trans_graph: bool,
        inc_runtime: bool,
    ) {
        self.push_undo_snapshot(SimulationSnapshot {
            terrain: if inc_terrain {
                Some(self.heightmap.clone_visual_dense())
            } else {
                None
            },
            water: if inc_water {
                Some(WaterRuntimeSnapshot {
                    baseline_depth: self.watermap.clone_baseline_depth_dense(),
                })
            } else {
                None
            },
            trans_graph: if inc_trans_graph {
                Some(self.region_graph.capture_full_undo())
            } else {
                None
            },
            zoning: None,
            runtime: if inc_runtime {
                Some(SimulationRuntimeSnapshot::PendingDemandSpawns(
                    self.pending_demand_spawns.clone(),
                ))
            } else {
                None
            },
        });
    }

    /// Captures only graph records that a road polyline and its junction solve can mutate.
    pub(crate) fn push_network_undo_for_polyline(&mut self, points: &[Vector3], margin_m: f32) {
        let graph = self
            .region_graph
            .capture_undo_for_polyline(points, margin_m);
        self.push_network_undo_delta(graph, None);
    }

    /// Captures a known local graph mutation set plus its incident junction ring.
    pub(crate) fn push_network_undo_for_local_topology(
        &mut self,
        edge_ids: HashSet<usize>,
        node_ids: HashSet<u32>,
    ) {
        let graph = self
            .region_graph
            .capture_undo_for_local_topology(edge_ids, node_ids);
        self.push_network_undo_delta(graph, None);
    }

    /// Captures a local road-removal inverse plus only parcels attached to that road.
    pub(crate) fn push_road_removal_undo(
        &mut self,
        edge_ids: HashSet<usize>,
        node_ids: HashSet<u32>,
        removed_edge_idx: usize,
    ) {
        let graph = self
            .region_graph
            .capture_undo_for_local_topology(edge_ids, node_ids);
        let zoning = self.zoning.capture_parcel_removal_undo(removed_edge_idx);
        self.push_network_undo_delta(graph, (!zoning.is_empty()).then_some(zoning));
    }

    fn push_network_undo_delta(
        &mut self,
        graph: RegionGraphUndoDelta,
        zoning: Option<ZoningParcelRemovalUndo>,
    ) {
        self.push_undo_snapshot(SimulationSnapshot {
            terrain: None,
            water: None,
            trans_graph: Some(graph),
            zoning,
            runtime: None,
        });
    }

    fn push_undo_snapshot(&mut self, snapshot: SimulationSnapshot) {
        if self.undo_stack.len() >= 30 {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(snapshot);
    }

    /// Captures a bounded inverse journal for one building deletion.
    pub(crate) fn push_building_removal_undo(&mut self, building_idx: usize) -> bool {
        let Some(runtime) = self.capture_building_removal_undo(building_idx) else {
            return false;
        };
        self.push_undo_snapshot(SimulationSnapshot {
            terrain: None,
            water: None,
            trans_graph: None,
            zoning: None,
            runtime: Some(SimulationRuntimeSnapshot::BuildingRemoval(runtime)),
        });
        true
    }

    fn capture_building_removal_undo(
        &mut self,
        building_idx: usize,
    ) -> Option<BuildingRemovalUndo> {
        let original_building_count = self.allocator.buildings.len();
        let last_idx = original_building_count.checked_sub(1)?;
        if building_idx >= original_building_count {
            return None;
        }

        let households = self.households.capture_building_undo(building_idx);
        let logistics = self.logistics.capture_building_undo(building_idx);
        let rehoused_household_count = households
            .records
            .iter()
            .filter(|(_, household, _)| household.home_building_id == building_idx)
            .count();

        let mut building_ids = BTreeSet::from([building_idx, last_idx]);
        building_ids.extend(
            self.allocator
                .preview_vacancy_claims_except(building_idx, rehoused_household_count),
        );
        building_ids.extend(households.mutated_building_ids.iter().copied());
        building_ids.extend(logistics.mutated_building_ids.iter().copied());
        let buildings = building_ids
            .into_iter()
            .filter_map(|idx| {
                self.allocator
                    .buildings
                    .get(idx)
                    .cloned()
                    .map(|building| (idx, building))
            })
            .collect();

        let sites = [building_idx, last_idx]
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|idx| {
                self.allocator
                    .building_sites
                    .get(idx)
                    .cloned()
                    .map(|site| (idx, site))
            })
            .collect();

        let carrier_ids = logistics
            .carrier_agent_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>();
        let agent_store = &self.agents.agents;
        let agents = (0..agent_store.len())
            .into_par_iter()
            .filter(|&idx| {
                !carrier_ids.contains(&idx)
                    && Self::agent_record_touches_building(agent_store, idx, building_idx)
            })
            .map(|idx| (idx, agent_store.get(idx).expect("agent index").to_owned()))
            .collect();
        let removed_carriers = logistics
            .carrier_agent_ids
            .iter()
            .filter_map(|&idx| agent_store.get(idx).map(|agent| (idx, agent.to_owned())))
            .collect();

        Some(BuildingRemovalUndo {
            building_idx,
            original_building_count,
            expected_post_building_ref_revision: self
                .allocator
                .building_ref_revision
                .wrapping_add(1),
            original_site_count: self.allocator.building_sites.len(),
            original_agent_count: self.agents.agents.len(),
            original_household_count: self.households.households.len(),
            original_shipment_count: self.logistics.shipments.len(),
            original_request_failure_count: self.logistics.request_failures.len(),
            buildings,
            sites,
            agents,
            removed_carriers,
            households,
            logistics,
            dirty_bounds: self.allocator.site_world_bounds(building_idx),
        })
    }

    /// Seals the latest building-removal journal after all deletion-side maintenance completes.
    pub(crate) fn seal_building_removal_undo(&mut self) {
        let Some(SimulationSnapshot {
            runtime: Some(SimulationRuntimeSnapshot::BuildingRemoval(undo)),
            ..
        }) = self.undo_stack.back_mut()
        else {
            return;
        };
        undo.expected_post_building_ref_revision = self.allocator.building_ref_revision;
    }

    fn agent_record_touches_building(
        agents: &crate::simulation::economy::agents::AgentVec,
        idx: usize,
        building_idx: usize,
    ) -> bool {
        agents.home_building[idx] == building_idx
            || agents.work_building[idx] == building_idx
            || agents.current_building[idx] == building_idx
            || agents.target_building[idx] == building_idx
            || agents.planned_target_building[idx] == building_idx
            || agents.next_departure_origin_building[idx] == building_idx
            || agents.next_departure_target_building[idx] == building_idx
            || agents.cached_schedule_work_building[idx] == building_idx
    }

    /// Pops the last state snapshot from the undo stack and restores simulation state.
    /// Returns true if an action was undone.
    pub fn undo_action_internal(&mut self) -> bool {
        if let Some(state) = self.undo_stack.pop_back() {
            if let Some(SimulationRuntimeSnapshot::BuildingRemoval(removal)) = &state.runtime
                && !self.can_restore_building_removal_undo(removal)
            {
                self.undo_stack.push_back(state);
                return false;
            }
            if let Some(zoning) = &state.zoning
                && !self.zoning.can_restore_parcel_removal_undo(zoning)
            {
                self.undo_stack.push_back(state);
                return false;
            }
            let SimulationSnapshot {
                terrain,
                water,
                trans_graph,
                zoning,
                runtime,
            } = state;
            let mut sync_trans_graph = false;
            let old_engineered_patch_keys = self.engineered_terrain_patch_keys.clone();

            if let Some(t_data) = terrain {
                self.heightmap
                    .replace_visual_from_dense(&t_data)
                    .expect("undo terrain snapshot must match the live terrain dimensions");
                sync_trans_graph = true;
            }
            if let Some(w_data) = water {
                self.watermap
                    .replace_baseline_depth_from_dense(&w_data.baseline_depth)
                    .expect("undo baseline water snapshot must match the live water dimensions");
                self.water_dirty = true;
                self.bump_road_tool_query_generation();
            }
            if let Some(tr_graph) = trans_graph {
                self.region_graph.restore_undo_delta(tr_graph);
                sync_trans_graph = true;
            }
            if let Some(zoning) = zoning {
                self.zoning.restore_parcel_removal_undo(zoning);
            }
            if let Some(runtime) = runtime {
                match runtime {
                    SimulationRuntimeSnapshot::PendingDemandSpawns(pending) => {
                        self.pending_demand_spawns = pending;
                    }
                    SimulationRuntimeSnapshot::BuildingRemoval(removal) => {
                        self.restore_building_removal_undo(removal);
                    }
                }
            }

            if sync_trans_graph {
                // Rebuild lane topology from the restored graph so crosswalk geometry
                // and junction connections match the reverted road network.
                self.transit_network
                    .lane_system
                    .rebuild(&mut self.region_graph);
                self.transit_network
                    .rebuild_cch_and_check(&self.region_graph);
                self.transit_network.cch_dirty_chunks.clear();
                self.transit_network.flow_fields.mark_all_dirty();
                self.reset_network_render_state(old_engineered_patch_keys);
            }
            return true;
        }
        false
    }

    fn reset_network_render_state(&mut self, old_engineered_patch_keys: Vec<(usize, usize)>) {
        for (patch_x, patch_z) in old_engineered_patch_keys {
            if let Some((min_x, min_z, max_x, max_z)) =
                self.heightmap.render_patch_world_bounds(patch_x, patch_z)
            {
                self.heightmap
                    .reset_visual_region_from_source_world(min_x, min_z, max_x, max_z);
            } else {
                self.heightmap.mark_render_patch_dirty(patch_x, patch_z);
            }
        }

        self.transit_network.road_surface.clear();
        self.refined_terrain_patch_cache.clear();
        self.road_locked_terrain_patch_keys.clear();
        self.road_locked_terrain_patch_margins.clear();
        self.building_site_owned_terrain_patch_keys.clear();
        self.engineered_terrain_patch_keys.clear();
        self.engineered_terrain_patch_margins.clear();
        self.cached_road_mesh_data = None;
        self.terrain_dirty = true;
        self.mark_network_render_dirty();
    }

    fn can_restore_building_removal_undo(&self, undo: &BuildingRemovalUndo) -> bool {
        if undo.original_building_count == 0
            || undo.building_idx >= undo.original_building_count
            || self.allocator.buildings.len().saturating_add(1) != undo.original_building_count
            || self.allocator.building_ref_revision != undo.expected_post_building_ref_revision
            || self
                .agents
                .agents
                .len()
                .saturating_add(undo.removed_carriers.len())
                != undo.original_agent_count
            || self.households.households.len() != undo.original_household_count
            || self
                .logistics
                .shipments
                .len()
                .saturating_add(undo.logistics.shipments.len())
                != undo.original_shipment_count
            || self
                .logistics
                .request_failures
                .len()
                .saturating_add(undo.logistics.request_failures.len())
                != undo.original_request_failure_count
        {
            return false;
        }

        let expected_site_count =
            undo.original_site_count - usize::from(undo.building_idx < undo.original_site_count);
        if self.allocator.building_sites.len() != expected_site_count {
            return false;
        }

        let last_building_idx = undo.original_building_count - 1;
        if !undo
            .buildings
            .iter()
            .any(|(idx, _)| *idx == undo.building_idx)
            || (undo.building_idx < last_building_idx
                && !undo
                    .buildings
                    .iter()
                    .any(|(idx, _)| *idx == last_building_idx))
            || undo
                .buildings
                .iter()
                .any(|(idx, _)| *idx >= undo.original_building_count)
        {
            return false;
        }

        if undo.building_idx < undo.original_site_count {
            let last_site_idx = undo.original_site_count - 1;
            if !undo.sites.iter().any(|(idx, _)| *idx == undo.building_idx)
                || (undo.building_idx < last_site_idx
                    && !undo.sites.iter().any(|(idx, _)| *idx == last_site_idx))
                || undo
                    .sites
                    .iter()
                    .any(|(idx, _)| *idx >= undo.original_site_count)
            {
                return false;
            }
        }

        let current_agent_count = self.agents.agents.len();
        if undo
            .removed_carriers
            .iter()
            .enumerate()
            .any(|(offset, (idx, _))| {
                *idx > current_agent_count + offset || *idx >= undo.original_agent_count
            })
            || undo
                .removed_carriers
                .windows(2)
                .any(|pair| pair[0].0 >= pair[1].0)
            || undo
                .agents
                .iter()
                .any(|(idx, _)| *idx >= undo.original_agent_count)
            || undo
                .households
                .records
                .iter()
                .any(|(idx, _, _)| *idx >= undo.original_household_count)
        {
            return false;
        }

        true
    }

    fn restore_building_removal_undo(&mut self, undo: BuildingRemovalUndo) {
        let building_idx = undo.building_idx;
        let last_idx = undo.original_building_count - 1;
        let mut buildings = undo.buildings.into_iter().collect::<BTreeMap<_, _>>();
        let removed_building = buildings
            .remove(&building_idx)
            .expect("building undo journal was prevalidated");
        let removed_parcel_id = removed_building.parcel_id;
        if building_idx < last_idx {
            let moved_building = buildings
                .remove(&last_idx)
                .expect("moved building undo record was prevalidated");
            self.allocator.buildings.push(moved_building);
            self.allocator.buildings[building_idx] = removed_building;
        } else {
            self.allocator.buildings.push(removed_building);
        }
        for (idx, building) in buildings {
            if let Some(target) = self.allocator.buildings.get_mut(idx) {
                *target = building;
            }
        }

        if building_idx < undo.original_site_count {
            let last_site_idx = undo.original_site_count - 1;
            let mut sites = undo.sites.into_iter().collect::<BTreeMap<_, _>>();
            let removed_site = sites
                .remove(&building_idx)
                .expect("building-site undo journal was prevalidated");
            if building_idx < last_site_idx {
                let moved_site = sites
                    .remove(&last_site_idx)
                    .expect("moved building-site undo record was prevalidated");
                self.allocator.building_sites.push(moved_site);
                self.allocator.building_sites[building_idx] = removed_site;
            } else {
                self.allocator.building_sites.push(removed_site);
            }
        }
        if self.allocator.building_sites.len() != undo.original_site_count {
            self.allocator
                .rebuild_building_site_clients(self.zoning.config.zone_cell_m);
        } else {
            self.allocator.recompute_max_site_radius_m();
        }

        if building_idx < last_idx {
            let inverse = HashMap::from([(building_idx, last_idx)]);
            self.agents.remap_building_indices(&inverse);
            self.households.remap_building_indices(&inverse);
            self.logistics.remap_building_indices(&inverse);
            self.zoning.remap_parcel_occupancy(building_idx, last_idx);
        }

        for (carrier_idx, carrier) in undo.removed_carriers {
            let current_len = self.agents.agents.len();
            if carrier_idx == current_len {
                self.agents.agents.push(carrier);
                continue;
            }
            let moved = self
                .agents
                .agents
                .get(carrier_idx)
                .expect("restored carrier slot")
                .to_owned();
            self.agents.agents.push(moved);
            self.logistics
                .remap_carrier_agent_index(carrier_idx, current_len);
            self.agents.agents.replace(carrier_idx, carrier);
        }
        for (agent_idx, agent) in undo.agents {
            self.agents.agents.replace(agent_idx, agent);
        }
        self.agents.invalidate_lane_bucket_snapshot();

        self.households.restore_building_undo(undo.households);
        self.logistics.restore_building_undo(undo.logistics);
        if removed_parcel_id != 0 {
            self.zoning.occupy_parcel(removed_parcel_id, building_idx);
        }

        self.allocator.rebuild_zone_index();
        self.allocator.dirty = true;
        self.allocator.dirty_index = false;
        self.allocator.entrances_dirty = true;
        self.allocator.building_ref_revision = self.allocator.building_ref_revision.wrapping_add(1);
        self.allocator.entrance_ref_revision = self.allocator.entrance_ref_revision.wrapping_add(1);
        self.rebuild_building_entrances_internal();
        if let Some(bounds) = undo.dirty_bounds {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
        self.transit_network.flow_fields.mark_all_dirty();
        self.terrain_dirty = true;
    }
}
