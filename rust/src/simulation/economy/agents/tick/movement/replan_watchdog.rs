//! Network replan failure watchdog and deterministic recovery fallbacks.

use super::super::slices::MovementSlices;
use super::NETWORK_REPLAN_DELAY_S;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{
    ACCESS_IMMIGRATION_ORIGIN, ACTIVITY_HOME, MODE_CAR, MODE_WALK, TRANSIT_IMMIGRATING,
    TRANSIT_IN_BUILDING, TRANSIT_NETWORK,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::NodeType;
use crate::traffic_log;
use godot::prelude::Vector2;

const NETWORK_REPLAN_WATCHDOG_FAILURES: u8 = 3;

/// Clears consecutive network-replan failure state after a route proves usable again.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[inline(always)]
pub(super) unsafe fn reset_network_replan_watchdog(i: usize, slices: &MovementSlices) {
    unsafe {
        *slices.network_replan_failures.get_mut(i) = 0;
    }
}

/// Returns whether a route-less network actor has enough trip context for watchdog recovery.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[inline(always)]
pub(super) unsafe fn has_recoverable_network_trip(i: usize, slices: &MovementSlices) -> bool {
    unsafe {
        *slices.tgt_b.get(i) != usize::MAX
            || *slices.planned_tgt_b.get(i) != usize::MAX
            || *slices.freight_target_border_node.get(i) != u32::MAX
            || *slices.pending_household_size.get(i) > 0
            || *slices.freight_shipment_id.get(i) != u64::MAX
            || *slices.transit.get(i) == TRANSIT_IMMIGRATING
    }
}

/// Records one failed live network replan and recovers the agent once the watchdog threshold is hit.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn delay_or_recover_after_network_replan_failure(
    i: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    graph: &RegionGraph,
    reason: &'static str,
    slices: &MovementSlices,
) {
    unsafe {
        clear_failed_replan_attempt(i, slices);

        let failures = slices.network_replan_failures.get(i).saturating_add(1);
        let failures = failures.min(NETWORK_REPLAN_WATCHDOG_FAILURES);
        *slices.network_replan_failures.get_mut(i) = failures;

        traffic_log!(
            "[NETWORK_REPLAN_FAILED] agent={} failures={}/{} reason={} transit={} mode={} home_bldg={} target_bldg={} current_node={} current_edge={} lane={} flags=0x{:02x}",
            i,
            failures,
            NETWORK_REPLAN_WATCHDOG_FAILURES,
            reason,
            *slices.transit.get(i),
            *slices.tmode.get(i),
            *slices.home.get(i),
            *slices.tgt_b.get(i),
            *slices.cur_n.get(i),
            *slices.cur_e.get(i),
            *slices.lane_id.get(i),
            *slices.access_flags.get(i),
        );

        if failures < NETWORK_REPLAN_WATCHDOG_FAILURES {
            *slices.next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
            return;
        }

        if use_border_recovery(i, allocator, slices) {
            recover_to_border(i, sim_time, allocator, graph, reason, slices);
        } else if !recover_to_home(i, sim_time, allocator, reason, slices) {
            recover_to_border(i, sim_time, allocator, graph, reason, slices);
        }
    }
}

unsafe fn clear_failed_replan_attempt(i: usize, slices: &MovementSlices) {
    unsafe {
        slices.path.get_mut(i).clear();
        *slices.path_idx.get_mut(i) = 0;
        *slices.speed.get_mut(i) = 0.0;
    }
}

unsafe fn use_border_recovery(
    i: usize,
    allocator: &BuildingAllocator,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let home = *slices.home.get(i);
        *slices.pending_household_size.get(i) > 0
            || *slices.freight_shipment_id.get(i) != u64::MAX
            || *slices.transit.get(i) == TRANSIT_IMMIGRATING
            || (*slices.access_flags.get(i) & ACCESS_IMMIGRATION_ORIGIN) != 0
            || home >= allocator.buildings.len()
    }
}

unsafe fn recover_to_home(
    i: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    reason: &'static str,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let home = *slices.home.get(i);
        if home >= allocator.buildings.len() {
            return false;
        }

        let pos = home_recovery_position(home, allocator);
        clear_access_and_network_state(i, slices);
        *slices.pos_x.get_mut(i) = pos.x;
        *slices.pos_y.get_mut(i) = pos.y;
        *slices.cur_b.get_mut(i) = home;
        *slices.tgt_b.get_mut(i) = usize::MAX;
        *slices.planned_tgt_b.get_mut(i) = usize::MAX;
        *slices.activity.get_mut(i) = ACTIVITY_HOME;
        *slices.planned_activity.get_mut(i) = ACTIVITY_HOME;
        *slices.jstart.get_mut(i) = sim_time;
        *slices.transit.get_mut(i) = TRANSIT_IN_BUILDING;
        *slices.tmode.get_mut(i) = MODE_WALK;
        *slices.next_replan_time.get_mut(i) = 0.0;
        *slices.network_replan_failures.get_mut(i) = 0;

        traffic_log!(
            "[NETWORK_REPLAN_WATCHDOG] agent={} action=home reason={} home_bldg={} pos=({:.2},{:.2})",
            i,
            reason,
            home,
            pos.x,
            pos.y,
        );
        true
    }
}

unsafe fn recover_to_border(
    i: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    graph: &RegionGraph,
    reason: &'static str,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let Some(border_node) = choose_recovery_border_node(i, graph, slices) else {
            *slices.next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
            traffic_log!(
                "[NETWORK_REPLAN_WATCHDOG] agent={} action=delay_no_border reason={} home_bldg={} target_bldg={} current_node={}",
                i,
                reason,
                *slices.home.get(i),
                *slices.tgt_b.get(i),
                *slices.cur_n.get(i),
            );
            return false;
        };

        let pos = graph.node(border_node).pos;
        let current_target = *slices.tgt_b.get(i);
        let home = *slices.home.get(i);
        let recovered_target = if current_target < allocator.buildings.len() {
            current_target
        } else if (*slices.pending_household_size.get(i) > 0
            || *slices.transit.get(i) == TRANSIT_IMMIGRATING)
            && home < allocator.buildings.len()
        {
            home
        } else {
            usize::MAX
        };

        clear_access_and_network_state(i, slices);
        *slices.pos_x.get_mut(i) = pos.x;
        *slices.pos_y.get_mut(i) = pos.z;
        *slices.cur_b.get_mut(i) = usize::MAX;
        *slices.cur_n.get_mut(i) = border_node;
        *slices.tgt_b.get_mut(i) = recovered_target;
        *slices.planned_tgt_b.get_mut(i) = usize::MAX;
        *slices.planned_activity.get_mut(i) = 0;
        *slices.jstart.get_mut(i) = sim_time;
        *slices.transit.get_mut(i) = if recovered_target < allocator.buildings.len() {
            TRANSIT_IMMIGRATING
        } else {
            TRANSIT_NETWORK
        };
        *slices.tmode.get_mut(i) = MODE_CAR;
        *slices.next_replan_time.get_mut(i) = 0.0;
        *slices.network_replan_failures.get_mut(i) = 0;

        traffic_log!(
            "[NETWORK_REPLAN_WATCHDOG] agent={} action=border reason={} border_node={} target_bldg={} freight_target_border_node={} pos=({:.2},{:.2})",
            i,
            reason,
            border_node,
            recovered_target,
            *slices.freight_target_border_node.get(i),
            pos.x,
            pos.z,
        );
        true
    }
}

fn home_recovery_position(home: usize, allocator: &BuildingAllocator) -> Vector2 {
    allocator
        .entrances
        .get(home)
        .map(|entrance| entrance.door_pos)
        .unwrap_or_else(|| {
            let building = &allocator.buildings[home];
            Vector2::new(building.center_x, building.center_y)
        })
}

unsafe fn choose_recovery_border_node(
    i: usize,
    graph: &RegionGraph,
    slices: &MovementSlices,
) -> Option<u32> {
    unsafe {
        for candidate in [
            *slices.freight_target_border_node.get(i),
            *slices.cur_n.get(i),
            *slices.planned_attach_n.get(i),
            *slices.planned_detach_n.get(i),
        ] {
            if let Some(border_node) = usable_border_node(candidate, graph) {
                return Some(border_node);
            }
        }
    }

    graph
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(node_id, node)| {
            let node_id = u32::try_from(node_id).ok()?;
            (node.node_type == NodeType::Border && graph.node_has_live_incident_edge(node_id))
                .then_some(node_id)
        })
}

fn usable_border_node(node_id: u32, graph: &RegionGraph) -> Option<u32> {
    if node_id == u32::MAX {
        return None;
    }
    let valid = graph.get_valid_node(node_id);
    if valid as usize >= graph.node_count() {
        return None;
    }
    let node = graph.node(valid);
    (node.node_type == NodeType::Border && graph.node_has_live_incident_edge(valid))
        .then_some(valid)
}

unsafe fn clear_access_and_network_state(i: usize, slices: &MovementSlices) {
    unsafe {
        *slices.planned_attach_n.get_mut(i) = u32::MAX;
        *slices.planned_detach_n.get_mut(i) = u32::MAX;
        *slices.planned_attach_lane.get_mut(i) = u32::MAX;
        *slices.planned_detach_lane.get_mut(i) = u32::MAX;
        *slices.planned_attach_lane_d.get_mut(i) = 0.0;
        *slices.planned_detach_lane_d.get_mut(i) = 0.0;
        *slices.access_flags.get_mut(i) = 0;
        *slices.cur_n.get_mut(i) = u32::MAX;
        *slices.cur_e.get_mut(i) = usize::MAX;
        *slices.lane_id.get_mut(i) = usize::MAX;
        *slices.lane_d.get_mut(i) = 0.0;
        *slices.lane_change_from_lane.get_mut(i) = u32::MAX;
        *slices.lane_change_start_d.get_mut(i) = 0.0;
        *slices.lane_change_length.get_mut(i) = 0.0;
        *slices.overtake_blocked_time.get_mut(i) = 0.0;
        *slices.overtake_cooldown.get_mut(i) = 0.0;
        *slices.speed.get_mut(i) = 0.0;
        slices.path.get_mut(i).clear();
        *slices.path_idx.get_mut(i) = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_URBAN_ROAD_SPEED_MS;
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator, BuildingEntrance};
    use crate::simulation::economy::agents::tick::slices::{MovementSlices, RawSlice};
    use crate::simulation::economy::agents::{AgentSystem, TRANSIT_NETWORK};
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::types::{
        EdgeClass, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::zoning::ZoneType;
    use godot::prelude::{Vector2, Vector3};

    #[test]
    fn housed_citizen_watchdog_recovers_to_home_building() {
        let mut allocator = BuildingAllocator::new();
        allocator.buildings.push(test_building(7.0, 9.0));
        allocator
            .entrances
            .push(test_entrance(Vector2::new(12.0, 14.0)));
        let graph = RegionGraph::new();
        let mut agents = AgentSystem::new();
        let agent_idx = agents.spawn_housed_agent(0, -1.0, -2.0);
        agents.transit[agent_idx] = TRANSIT_NETWORK;
        agents.current_building[agent_idx] = usize::MAX;
        agents.target_building[agent_idx] = 99;
        agents.current_node[agent_idx] = 2;
        agents.current_edge[agent_idx] = 4;
        agents.current_lane_id[agent_idx] = 6;
        agents.current_path[agent_idx] = vec![2, 3];
        agents.current_path_index[agent_idx] = 1;
        agents.speed[agent_idx] = 5.0;

        let slices = movement_slices(&mut agents);
        unsafe {
            delay_or_recover_after_network_replan_failure(
                agent_idx, 10.0, &allocator, &graph, "test", &slices,
            );
            assert_eq!(*slices.network_replan_failures.get(agent_idx), 1);
            assert_eq!(*slices.transit.get(agent_idx), TRANSIT_NETWORK);
            assert_eq!(*slices.next_replan_time.get(agent_idx), 15.0);

            delay_or_recover_after_network_replan_failure(
                agent_idx, 15.0, &allocator, &graph, "test", &slices,
            );
            assert_eq!(*slices.network_replan_failures.get(agent_idx), 2);
            assert_eq!(*slices.transit.get(agent_idx), TRANSIT_NETWORK);

            delay_or_recover_after_network_replan_failure(
                agent_idx, 20.0, &allocator, &graph, "test", &slices,
            );
        }

        assert_eq!(agents.transit[agent_idx], TRANSIT_IN_BUILDING);
        assert_eq!(agents.current_building[agent_idx], 0);
        assert_eq!(agents.target_building[agent_idx], usize::MAX);
        assert_eq!(agents.activity[agent_idx], ACTIVITY_HOME);
        assert_eq!(agents.pos_x[agent_idx], 12.0);
        assert_eq!(agents.pos_y[agent_idx], 14.0);
        assert_eq!(agents.current_node[agent_idx], u32::MAX);
        assert_eq!(agents.current_edge[agent_idx], usize::MAX);
        assert_eq!(agents.current_lane_id[agent_idx], usize::MAX);
        assert!(agents.current_path[agent_idx].is_empty());
        assert_eq!(agents.network_replan_failures[agent_idx], 0);
    }

    #[test]
    fn freight_watchdog_recovers_to_target_border_node() {
        let allocator = BuildingAllocator::new();
        let mut graph = RegionGraph::new();
        let border_node = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Border);
        let city_node = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(border_node, city_node));

        let mut agents = AgentSystem::new();
        let agent_idx = agents.spawn_border_arrival_agent(
            usize::MAX,
            border_node,
            0.0,
            0.0,
            city_node,
            0.0,
            0.0,
        );
        agents.transit[agent_idx] = TRANSIT_NETWORK;
        agents.freight_shipment_id[agent_idx] = 42;
        agents.freight_target_border_node[agent_idx] = border_node;
        agents.current_node[agent_idx] = city_node;
        agents.current_path[agent_idx] = vec![city_node, border_node];
        agents.current_path_index[agent_idx] = 1;
        agents.speed[agent_idx] = DEFAULT_URBAN_ROAD_SPEED_MS;

        let slices = movement_slices(&mut agents);
        unsafe {
            delay_or_recover_after_network_replan_failure(
                agent_idx, 10.0, &allocator, &graph, "test", &slices,
            );
            delay_or_recover_after_network_replan_failure(
                agent_idx, 15.0, &allocator, &graph, "test", &slices,
            );
            delay_or_recover_after_network_replan_failure(
                agent_idx, 20.0, &allocator, &graph, "test", &slices,
            );
        }

        assert_eq!(agents.transit[agent_idx], TRANSIT_NETWORK);
        assert_eq!(agents.current_building[agent_idx], usize::MAX);
        assert_eq!(agents.current_node[agent_idx], border_node);
        assert_eq!(agents.current_edge[agent_idx], usize::MAX);
        assert_eq!(agents.current_lane_id[agent_idx], usize::MAX);
        assert_eq!(agents.freight_target_border_node[agent_idx], border_node);
        assert!(agents.current_path[agent_idx].is_empty());
        assert_eq!(agents.pos_x[agent_idx], 100.0);
        assert_eq!(agents.pos_y[agent_idx], 0.0);
        assert_eq!(agents.network_replan_failures[agent_idx], 0);
    }

    fn movement_slices(agents: &mut AgentSystem) -> MovementSlices {
        MovementSlices {
            home: RawSlice::new(&mut agents.agents.home_building),
            work: RawSlice::new(&mut agents.agents.work_building),
            age_group: RawSlice::new(&mut agents.agents.age_group),
            pos_x: RawSlice::new(&mut agents.agents.pos_x),
            pos_y: RawSlice::new(&mut agents.agents.pos_y),
            activity: RawSlice::new(&mut agents.agents.activity),
            transit: RawSlice::new(&mut agents.agents.transit),
            happiness: RawSlice::new(&mut agents.agents.happiness),
            jstart: RawSlice::new(&mut agents.agents.journey_start_time),
            schedule_seed: RawSlice::new(&mut agents.agents.schedule_seed),
            cached_commute_minutes: RawSlice::new(&mut agents.agents.cached_commute_minutes),
            next_commute_refresh_time: RawSlice::new(&mut agents.agents.next_commute_refresh_time),
            next_departure_day: RawSlice::new(&mut agents.agents.next_departure_day),
            next_departure_minute: RawSlice::new(&mut agents.agents.next_departure_minute),
            next_departure_origin: RawSlice::new(&mut agents.agents.next_departure_origin_building),
            next_departure_target: RawSlice::new(&mut agents.agents.next_departure_target_building),
            next_departure_activity: RawSlice::new(&mut agents.agents.next_departure_activity),
            cached_schedule_work_building: RawSlice::new(
                &mut agents.agents.cached_schedule_work_building,
            ),
            cached_work_profile_index: RawSlice::new(&mut agents.agents.cached_work_profile_index),
            pending_household_size: RawSlice::new(&mut agents.agents.pending_household_size),
            freight_shipment_id: RawSlice::new(&mut agents.agents.freight_shipment_id),
            cur_b: RawSlice::new(&mut agents.agents.current_building),
            tgt_b: RawSlice::new(&mut agents.agents.target_building),
            planned_tgt_b: RawSlice::new(&mut agents.agents.planned_target_building),
            freight_target_border_node: RawSlice::new(
                &mut agents.agents.freight_target_border_node,
            ),
            cur_n: RawSlice::new(&mut agents.agents.current_node),
            planned_attach_n: RawSlice::new(&mut agents.agents.planned_attach_node),
            planned_detach_n: RawSlice::new(&mut agents.agents.planned_detach_node),
            planned_attach_lane: RawSlice::new(&mut agents.agents.planned_attach_lane_id),
            planned_detach_lane: RawSlice::new(&mut agents.agents.planned_detach_lane_id),
            planned_attach_lane_d: RawSlice::new(&mut agents.agents.planned_attach_lane_d),
            planned_detach_lane_d: RawSlice::new(&mut agents.agents.planned_detach_lane_d),
            access_flags: RawSlice::new(&mut agents.agents.access_flags),
            next_replan_time: RawSlice::new(&mut agents.agents.next_replan_time),
            network_replan_failures: RawSlice::new(&mut agents.agents.network_replan_failures),
            cur_e: RawSlice::new(&mut agents.agents.current_edge),
            lane_id: RawSlice::new(&mut agents.agents.current_lane_id),
            lane_d: RawSlice::new(&mut agents.agents.lane_distance),
            lane_change_from_lane: RawSlice::new(&mut agents.agents.lane_change_from_lane_id),
            lane_change_start_d: RawSlice::new(&mut agents.agents.lane_change_start_d),
            lane_change_length: RawSlice::new(&mut agents.agents.lane_change_length_m),
            overtake_blocked_time: RawSlice::new(&mut agents.agents.overtake_blocked_time_s),
            overtake_cooldown: RawSlice::new(&mut agents.agents.overtake_cooldown_s),
            tmode: RawSlice::new(&mut agents.agents.transit_mode),
            planned_activity: RawSlice::new(&mut agents.agents.planned_activity),
            path: RawSlice::new(&mut agents.agents.current_path),
            path_idx: RawSlice::new(&mut agents.agents.current_path_index),
            has_car: RawSlice::new(&mut agents.agents.has_car),
            speed: RawSlice::new(&mut agents.agents.speed),
            walk_phase: RawSlice::new(&mut agents.agents.walk_phase),
        }
    }

    fn test_entrance(door_pos: Vector2) -> BuildingEntrance {
        BuildingEntrance {
            door_pos,
            ..BuildingEntrance::default()
        }
    }

    fn test_edge(start_node: u32, end_node: u32) -> Edge {
        Edge {
            start_node,
            end_node,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: DEFAULT_URBAN_ROAD_SPEED_MS,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        }
    }

    fn test_building(center_x: f32, center_y: f32) -> Building {
        Building {
            center_x,
            center_y,
            support_height_m: 0.0,
            width_cells: 1,
            depth_cells: 1,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(1.0, 0.0),
            frontage_t: 0.5,
            side_offset: 5.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            service_funding_override: -1.0,
            asset_id: "test:placeholder".to_owned(),
            level: 1,
            construction_total_hours: 0,
            construction_remaining_hours: 0,
            broken: false,
            economy_profile_runtime_id: 0,
            economy_broken: false,
            resource_inventory: Vec::new(),
            revenue: 0.0,
            operating_budget: 500.0,
            profit_tax_budget_baseline: 500.0,
            last_day_profit: 0.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            daily_city_funded_input_cost: 0.0,
            daily_household_sales_value: 0.0,
            daily_power_service_units: 0.0,
            daily_power_served_units: 0.0,
            recent_power_service_units: 0.0,
            recent_power_served_units: 0.0,
            recent_household_sales_value: 0.0,
            commercial_activity_floor_scale: 0.0,
            work_area_scale: 1.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        }
    }
}
