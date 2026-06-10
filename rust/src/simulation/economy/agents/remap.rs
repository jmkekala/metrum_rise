//! Agent index repair after graph, lane, building, and household remaps.

use super::data::AgentSystem;
use crate::simulation::network::lanes::LaneSystem;
use std::collections::{HashMap, HashSet};

impl AgentSystem {
    /// Remaps the edge indices stored in all agents from old IDs to new IDs.
    pub fn update_edge_indices(&mut self, mapping: &HashMap<usize, usize>) {
        for i in 0..self.agents.len() {
            if self.agents.current_edge[i] != usize::MAX {
                if let Some(&new_id) = mapping.get(&self.agents.current_edge[i]) {
                    self.agents.current_edge[i] = new_id;
                    self.agents.current_path[i].clear();
                } else {
                    self.agents.current_edge[i] = usize::MAX;
                    self.agents.current_path[i].clear();
                }
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Remaps household indices after a `swap_remove` in `HouseholdSystem`. O(A).
    pub fn remap_household_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if mapping.is_empty() {
            return;
        }
        for i in 0..self.agents.len() {
            if let Some(&new_id) = mapping.get(&self.agents.household_id[i]) {
                self.agents.household_id[i] = new_id;
            }
        }
    }

    /// Remaps building indices after a `swap_remove` in `BuildingAllocator`. O(A).
    pub fn remap_building_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if mapping.is_empty() {
            return;
        }
        for i in 0..self.agents.len() {
            remap_usize(&mut self.agents.home_building[i], mapping);
            remap_usize(&mut self.agents.work_building[i], mapping);
            remap_usize(&mut self.agents.current_building[i], mapping);
            remap_usize(&mut self.agents.target_building[i], mapping);
            remap_usize(&mut self.agents.planned_target_building[i], mapping);
            remap_usize(&mut self.agents.next_departure_origin_building[i], mapping);
            remap_usize(&mut self.agents.next_departure_target_building[i], mapping);
            remap_usize(&mut self.agents.cached_schedule_work_building[i], mapping);
        }
    }

    /// Sets `current_lane_id = usize::MAX` for every agent whose active lane belongs to one
    /// of `affected_edges`, or whose connection lane leads directly into such a lane.
    ///
    /// Must be called **before** `LaneSystem::rebuild_edges_incremental` so the old lane IDs
    /// are still valid for lookup — old orphaned lanes retain their original `edge_id` even
    /// after `rebuild_edges_incremental` removes them from `edge_lanes`.
    pub fn invalidate_lane_ids_for_edges(
        &mut self,
        affected_edges: &HashSet<usize>,
        lane_system: &LaneSystem,
    ) {
        let mut affected_lane_ids: HashSet<usize> = HashSet::new();
        for &edge_id in affected_edges {
            if let Some(lane_ids) = lane_system.edge_lanes.get(&edge_id) {
                affected_lane_ids.extend(lane_ids);
            }
        }

        for i in 0..self.agents.len() {
            let lid = self.agents.current_lane_id[i];
            if lid == usize::MAX || lid >= lane_system.lanes.len() {
                continue;
            }
            let lane = &lane_system.lanes[lid];
            let should_invalidate = if lane.edge_id != usize::MAX {
                affected_edges.contains(&lane.edge_id)
            } else {
                lane.next_lanes
                    .first()
                    .map_or(false, |&next| affected_lane_ids.contains(&next))
            };
            if should_invalidate {
                self.agents.current_lane_id[i] = usize::MAX;
                self.agents.lane_change_from_lane_id[i] = u32::MAX;
                self.agents.lane_change_start_d[i] = 0.0;
                self.agents.lane_change_length_m[i] = 0.0;
                self.agents.overtake_blocked_time_s[i] = 0.0;
                self.agents.overtake_cooldown_s[i] = 0.0;
                // Preserve lane_distance so road edits do not visually snap agents to lane start
                // before the next tick can re-attach them.
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }
}

fn remap_usize(value: &mut usize, mapping: &HashMap<usize, usize>) {
    if let Some(&new_id) = mapping.get(value) {
        *value = new_id;
    }
}

#[cfg(test)]
mod tests {
    use super::super::data::{Agent, AgentSystem};
    use super::super::{AGE_ADULT, MODE_CAR, TRANSIT_IN_BUILDING, TRANSIT_NETWORK};
    use super::*;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::lanes::LaneSystem;
    use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    fn make_simple_lane_system() -> (RegionGraph, LaneSystem) {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(
            Vector3::new(0.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n1 = graph.add_node(
            Vector3::new(100.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n2 = graph.add_node(
            Vector3::new(200.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );

        let _e0 = graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            ..Default::default()
        });
        let _e1 = graph.add_edge(Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(200.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(200.0, 0.0, 0.0)],
            ..Default::default()
        });
        graph.rebuild_adjacency_list();

        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        (graph, lanes)
    }

    #[test]
    fn remaps_cached_schedule_building_ids() {
        let mut sys = AgentSystem::new();
        let idx = sys.spawn_housed_agent(3, 0.0, 0.0);
        sys.agents.work_building[idx] = 5;
        sys.agents.current_building[idx] = 3;
        sys.agents.target_building[idx] = 5;
        sys.agents.planned_target_building[idx] = 5;
        sys.agents.next_departure_origin_building[idx] = 3;
        sys.agents.next_departure_target_building[idx] = 5;
        sys.agents.cached_schedule_work_building[idx] = 5;

        let mapping = HashMap::from([(3usize, 8usize), (5usize, 9usize)]);
        sys.remap_building_indices(&mapping);

        assert_eq!(sys.agents.home_building[idx], 8);
        assert_eq!(sys.agents.work_building[idx], 9);
        assert_eq!(sys.agents.current_building[idx], 8);
        assert_eq!(sys.agents.target_building[idx], 9);
        assert_eq!(sys.agents.planned_target_building[idx], 9);
        assert_eq!(sys.agents.next_departure_origin_building[idx], 8);
        assert_eq!(sys.agents.next_departure_target_building[idx], 9);
        assert_eq!(sys.agents.cached_schedule_work_building[idx], 9);
    }

    #[test]
    fn test_invalidate_clears_agents_on_affected_edge() {
        let (_graph, lane_system) = make_simple_lane_system();
        let e0_lane = lane_system.edge_lanes[&0][0];
        let e1_lane = lane_system.edge_lanes[&1][0];

        let mut sys = AgentSystem::new();
        let render_id_0 = sys.allocate_render_id();
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            age_group: AGE_ADULT,
            pending_household_size: 0,
            work_building: usize::MAX,
            pos_x: 0.0,
            pos_y: 0.0,
            render_id: render_id_0,
            activity: 0,
            transit: TRANSIT_NETWORK,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            schedule_seed: 0,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: usize::MAX,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            current_node: 0,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            current_edge: 0,
            current_lane_id: e0_lane,
            lane_distance: 10.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: 10.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: true,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        });
        let render_id_1 = sys.allocate_render_id();
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            age_group: AGE_ADULT,
            pending_household_size: 0,
            work_building: usize::MAX,
            pos_x: 150.0,
            pos_y: 0.0,
            render_id: render_id_1,
            activity: 0,
            transit: TRANSIT_NETWORK,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            schedule_seed: 1,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: usize::MAX,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            current_node: 1,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            current_edge: 1,
            current_lane_id: e1_lane,
            lane_distance: 10.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: 10.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: true,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        });

        let mut affected = HashSet::new();
        affected.insert(0usize);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system);

        assert_eq!(sys.agents.current_lane_id[0], usize::MAX);
        assert_eq!(sys.agents.lane_distance[0], 10.0);
        assert_eq!(sys.agents.current_lane_id[1], e1_lane);
    }

    #[test]
    fn test_invalidate_skips_already_invalid_agents() {
        let (_graph, lane_system) = make_simple_lane_system();

        let mut sys = AgentSystem::new();
        let render_id = sys.allocate_render_id();
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            age_group: AGE_ADULT,
            pending_household_size: 0,
            work_building: usize::MAX,
            pos_x: 0.0,
            pos_y: 0.0,
            render_id,
            activity: 0,
            transit: TRANSIT_IN_BUILDING,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            schedule_seed: 0,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: 0,
            target_building: 0,
            planned_target_building: usize::MAX,
            current_node: 0,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: 0.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: false,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        });

        let mut affected = HashSet::new();
        affected.insert(0usize);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system);
        assert_eq!(sys.agents.current_lane_id[0], usize::MAX);
    }
}
