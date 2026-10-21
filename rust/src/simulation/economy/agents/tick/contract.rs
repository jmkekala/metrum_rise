// SPDX-License-Identifier: GPL-2.0-only

//! Short deterministic traffic work-count and timing contract (`PERF-06`).

use super::super::{
    ACCESS_PLAN_VALID, AgentSystem, MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS,
    TRANSIT_IN_BUILDING, TRANSIT_NETWORK,
};
use crate::assets::AssetManifest;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::zoning::ZoneType;
use godot::prelude::{Vector2, Vector3};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Actual loop work accumulated by traffic phases in serialized contract tests.
#[derive(Clone, Copy, Debug)]
pub(super) enum Phase {
    /// Agent rows inspected when rebuilding the occupancy snapshot.
    LaneBucketAgents,
    /// Occupancy entries processed across rebuilt dirty lane buckets.
    LaneBucketEntries,
    /// Agent rows visited by the active-speed or inactive-timer pass.
    SpeedAgents,
    /// Agent rows classified for lane handoff claims.
    ClaimAgents,
    /// Agent rows dispatched through movement or access states.
    MovementAgents,
    /// Agent rows contributing to the cadence-controlled frontage update.
    FrontageAgents,
    /// Lane rows updated when the frontage cadence fires.
    FrontageLanes,
}

const PHASE_COUNT: usize = Phase::FrontageLanes as usize + 1;
static COUNTS: [AtomicU64; PHASE_COUNT] = [const { AtomicU64::new(0) }; PHASE_COUNT];

/// Adds an exact loop length after or at dispatch; absent from gameplay builds.
pub(super) fn add(phase: Phase, amount: usize) {
    COUNTS[phase as usize].fetch_add(amount as u64, Ordering::Relaxed);
}

fn snapshot() -> [u64; PHASE_COUNT] {
    std::array::from_fn(|index| COUNTS[index].load(Ordering::Relaxed))
}

fn reset() {
    for count in &COUNTS {
        count.store(0, Ordering::Relaxed);
    }
}

fn fixture(count: usize) -> (AgentSystem, BuildingAllocator, TransitNetwork, RegionGraph) {
    let length = count as f32 * 16.0 + 1_000.0;
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let end = graph.add_node(Vector3::new(length, 0.0, 0.0), NodeType::Junction);
    let edge = graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 20.0,
        base_cost: length / 20.0,
        physical_length: length,
        geometry: vec![Vector3::ZERO, Vector3::new(length, 0.0, 0.0)],
        physical_geometry: vec![Vector3::ZERO, Vector3::new(length, 0.0, 0.0)],
        ..Default::default()
    });
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&graph);
    let lane = network.lane_system.edge_lanes[&edge]
        .iter()
        .copied()
        .find(|&lane_id| {
            let lane = &network.lane_system.lanes[lane_id];
            lane.is_fwd && lane.lane_type == LaneType::Vehicle
        })
        .expect("forward vehicle lane");

    let mut agents = AgentSystem::new();
    for index in 0..count {
        let agent = agents.spawn_debug_car(
            vec![start, end],
            &network.lane_system,
            lane,
            Vector2::new(index as f32 * 16.0, 0.0),
        );
        agents.lane_distance[agent] = index as f32 * 16.0;
        agents.speed[agent] = 10.0;
        agents.transit[agent] = TRANSIT_NETWORK;
        agents.transit_mode[agent] = MODE_CAR;
    }

    (agents, BuildingAllocator::new(), network, graph)
}

struct AccessPlan {
    lane_id: usize,
    lane_d: f32,
    attach_node: u32,
    detach_node: u32,
    door_pos: Vector2,
    lane_pos: Vector2,
}

fn register_access_asset(allocator: &mut BuildingAllocator) -> String {
    let manifest = AssetManifest::from_str(
        r#"
asset_id = "access_building"
display_name = "PERF access building"

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 0.5]
forward = [0.0, 0.0, 1.0]

[building]
zone_type = "commercial"
density = "low"
lot_width_cells = 1
lot_depth_cells = 1
min_zone_width_cells = 1
min_zone_depth_cells = 1
worker_capacity = 4
"#,
    )
    .expect("PERF-06 access asset manifest");
    allocator.registry.register("perf", manifest, String::new());
    "perf:access_building".to_owned()
}

fn access_plan(
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> AccessPlan {
    let entrance = allocator
        .entrances
        .first()
        .expect("PERF-06 access fixture entrance");
    assert_eq!(entrance.edge_idx, 0, "PERF-06 access entrance edge");
    assert!(entrance.door_pos.x.is_finite() && entrance.door_pos.y.is_finite());
    let lane_id = entrance.car_lane_fwd;
    assert!(
        lane_id < network.lane_system.lanes.len(),
        "PERF-06 access fixture requires a forward car lane"
    );
    let lane = &network.lane_system.lanes[lane_id];
    let edge = graph.edge(entrance.edge_idx);
    assert!(edge.physical_length > 0.0);
    let edge_pos = BuildingAllocator::sample_pos_on_edge(
        graph,
        entrance.edge_idx,
        entrance.entrance_s_m / edge.physical_length,
    );
    let lane_d = BuildingAllocator::project_point_to_polyline_s(&lane.geometry, edge_pos);
    assert!(lane_d.is_finite() && lane_d <= lane.length + 1.0e-4);
    let lane_pos = BuildingAllocator::sample_pos_on_lane(lane, lane_d);
    let (attach_node, detach_node) = if lane.is_fwd {
        (edge.end_node, edge.start_node)
    } else {
        (edge.start_node, edge.end_node)
    };
    AccessPlan {
        lane_id,
        lane_d,
        attach_node,
        detach_node,
        door_pos: entrance.door_pos,
        lane_pos,
    }
}

fn access_fixture(
    count: usize,
    transit: u8,
) -> (AgentSystem, BuildingAllocator, TransitNetwork, RegionGraph) {
    let (mut agents, mut allocator, network, graph) = fixture(count);
    let edge = 0;
    let asset_id = register_access_asset(&mut allocator);
    let building = crate::simulation::buildings::allocator::indexed_test_building(
        asset_id,
        ZoneType::Commercial,
        0,
    );
    let mut building = building;
    building.center_x = 250.0;
    building.center_y = -10.0;
    building.facing_dir = Vector2::new(0.0, -1.0);
    building.edge_idx = edge;
    building.side = 1;
    allocator.buildings.push(building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    reset_access_agents(&mut agents, &allocator, &network, &graph, transit);
    assert_access_fixture(&agents, &allocator, &network, &graph, transit);
    (agents, allocator, network, graph)
}

fn reset_access_agents(
    agents: &mut AgentSystem,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    transit: u8,
) {
    let plan = access_plan(allocator, network, graph);
    let egress = transit == TRANSIT_ACCESS_EGRESS;
    for agent in 0..agents.len() {
        agents.transit[agent] = transit;
        agents.transit_mode[agent] = MODE_CAR;
        agents.home_building[agent] = 0;
        agents.activity[agent] = if egress { 0 } else { 1 };
        agents.current_building[agent] = if egress { 0 } else { usize::MAX };
        agents.target_building[agent] = if egress { usize::MAX } else { 0 };
        agents.planned_target_building[agent] = usize::MAX;
        agents.current_node[agent] = u32::MAX;
        agents.current_lane_id[agent] = usize::MAX;
        agents.current_edge[agent] = usize::MAX;
        agents.lane_distance[agent] = 0.0;
        let pos = if egress { plan.door_pos } else { plan.lane_pos };
        agents.pos_x[agent] = pos.x;
        agents.pos_y[agent] = pos.y;
        agents.planned_attach_node[agent] = if egress { plan.attach_node } else { u32::MAX };
        agents.planned_attach_lane_id[agent] = if egress {
            plan.lane_id as u32
        } else {
            u32::MAX
        };
        agents.planned_attach_lane_d[agent] = if egress { plan.lane_d } else { 0.0 };
        agents.planned_detach_node[agent] = if egress { u32::MAX } else { plan.detach_node };
        agents.planned_detach_lane_id[agent] = if egress {
            u32::MAX
        } else {
            plan.lane_id as u32
        };
        agents.planned_detach_lane_d[agent] = if egress { 0.0 } else { plan.lane_d };
        agents.lane_change_from_lane_id[agent] = u32::MAX;
        agents.lane_change_start_d[agent] = 0.0;
        agents.lane_change_length_m[agent] = 0.0;
        agents.overtake_blocked_time_s[agent] = 0.0;
        agents.overtake_cooldown_s[agent] = 0.0;
        agents.speed[agent] = 0.0;
        agents.current_path[agent].clear();
        agents.current_path_index[agent] = 0;
        agents.next_replan_time[agent] = 0.0;
        agents.network_replan_failures[agent] = 0;
        agents.access_flags[agent] = ACCESS_PLAN_VALID;
    }
    agents.invalidate_lane_bucket_snapshot();
}

fn assert_access_fixture(
    agents: &AgentSystem,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    transit: u8,
) {
    let plan = access_plan(allocator, network, graph);
    let egress = transit == TRANSIT_ACCESS_EGRESS;
    for agent in 0..agents.len() {
        assert_eq!(agents.transit[agent], transit);
        assert_eq!(agents.transit_mode[agent], MODE_CAR);
        assert_eq!(agents.current_lane_id[agent], usize::MAX);
        assert_eq!(agents.current_edge[agent], usize::MAX);
        assert_eq!(agents.current_path[agent].len(), 0);
        assert_eq!(agents.current_path_index[agent], 0);
        assert_eq!(agents.access_flags[agent], ACCESS_PLAN_VALID);
        if egress {
            assert_eq!(agents.current_building[agent], 0);
            assert_eq!(agents.target_building[agent], usize::MAX);
            assert_eq!(agents.planned_attach_node[agent], plan.attach_node);
            assert_eq!(agents.planned_attach_lane_id[agent], plan.lane_id as u32);
            assert_eq!(agents.planned_attach_lane_d[agent], plan.lane_d);
        } else {
            assert_eq!(agents.current_building[agent], usize::MAX);
            assert_eq!(agents.target_building[agent], 0);
            assert_eq!(agents.planned_detach_node[agent], plan.detach_node);
            assert_eq!(agents.planned_detach_lane_id[agent], plan.lane_id as u32);
            assert_eq!(agents.planned_detach_lane_d[agent], plan.lane_d);
        }
    }
}

fn assert_access_tick(agents: &AgentSystem, transit: u8) {
    for agent in 0..agents.len() {
        match transit {
            TRANSIT_ACCESS_EGRESS => {
                assert!(matches!(
                    agents.transit[agent],
                    TRANSIT_ACCESS_EGRESS | TRANSIT_NETWORK
                ));
                if agents.transit[agent] == TRANSIT_ACCESS_EGRESS {
                    assert_eq!(agents.current_building[agent], 0);
                    assert_eq!(agents.target_building[agent], usize::MAX);
                    assert_eq!(agents.access_flags[agent], ACCESS_PLAN_VALID);
                } else {
                    assert_eq!(agents.current_building[agent], usize::MAX);
                    assert_eq!(agents.current_edge[agent], 0);
                    assert_ne!(agents.current_lane_id[agent], usize::MAX);
                }
            }
            TRANSIT_ACCESS_INGRESS => {
                assert!(matches!(
                    agents.transit[agent],
                    TRANSIT_ACCESS_INGRESS | TRANSIT_IN_BUILDING
                ));
                if agents.transit[agent] == TRANSIT_ACCESS_INGRESS {
                    assert_eq!(agents.current_building[agent], usize::MAX);
                    assert_eq!(agents.target_building[agent], 0);
                    assert_eq!(agents.current_lane_id[agent], usize::MAX);
                    assert_eq!(agents.current_edge[agent], usize::MAX);
                    assert_eq!(agents.access_flags[agent], ACCESS_PLAN_VALID);
                } else {
                    assert_eq!(agents.current_building[agent], 0);
                    assert_eq!(agents.target_building[agent], usize::MAX);
                    assert_eq!(agents.access_flags[agent], 0);
                }
            }
            _ => panic!("PERF-06 unsupported access mode {transit}"),
        }
    }
}

fn work_delta(before: [u64; PHASE_COUNT], after: [u64; PHASE_COUNT]) -> [u64; PHASE_COUNT] {
    std::array::from_fn(|index| after[index] - before[index])
}

fn work_json(work: [u64; PHASE_COUNT]) -> serde_json::Value {
    serde_json::json!({
        "lane_bucket_agents": work[Phase::LaneBucketAgents as usize],
        "lane_bucket_entries": work[Phase::LaneBucketEntries as usize],
        "speed_agents": work[Phase::SpeedAgents as usize],
        "claim_agents": work[Phase::ClaimAgents as usize],
        "movement_agents": work[Phase::MovementAgents as usize],
        "frontage_agents": work[Phase::FrontageAgents as usize],
        "frontage_lanes": work[Phase::FrontageLanes as usize],
    })
}

#[test]
#[ignore = "short PERF-06 agent tick contract; run release with --ignored"]
// Runtime guard: the ignored check must still compile in a debug test build.
#[allow(clippy::assertions_on_constants)]
fn benchmark_agent_tick_contract() {
    assert!(
        !cfg!(debug_assertions),
        "PERF-06 agent tick contract requires a release test binary"
    );
    const TIERS: [usize; 2] = [10_000, 30_000];
    const MAX_SCALE: f64 = 3.6;

    for mode in ["network", "access_egress", "access_ingress"] {
        let mut reports = Vec::new();
        for count in TIERS {
            let transit = match mode {
                "access_egress" => TRANSIT_ACCESS_EGRESS,
                "access_ingress" => TRANSIT_ACCESS_INGRESS,
                _ => TRANSIT_NETWORK,
            };
            let (mut agents, allocator, mut network, mut graph) = if mode == "network" {
                fixture(count)
            } else {
                access_fixture(count, transit)
            };
            let clock = TimeSystem::new();
            agents.tick(&allocator, &mut network, &mut graph, 0.1, &clock);
            if mode != "network" {
                reset_access_agents(&mut agents, &allocator, &network, &graph, transit);
            }
            network.frontage_delay_elapsed_s = 0.95;

            reset();
            let before = snapshot();
            let started = Instant::now();
            agents.tick(
                &allocator,
                &mut network,
                &mut graph,
                std::hint::black_box(0.1),
                std::hint::black_box(&clock),
            );
            let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
            let work = work_delta(before, snapshot());
            if mode != "network" {
                assert_access_tick(&agents, transit);
            }

            let (mut repeat_agents, repeat_allocator, mut repeat_network, mut repeat_graph) =
                if mode == "network" {
                    fixture(count)
                } else {
                    access_fixture(count, transit)
                };
            repeat_agents.tick(
                &repeat_allocator,
                &mut repeat_network,
                &mut repeat_graph,
                0.1,
                &clock,
            );
            if mode != "network" {
                reset_access_agents(
                    &mut repeat_agents,
                    &repeat_allocator,
                    &repeat_network,
                    &repeat_graph,
                    transit,
                );
            }
            repeat_network.frontage_delay_elapsed_s = 0.95;
            reset();
            let repeat_before = snapshot();
            repeat_agents.tick(
                &repeat_allocator,
                &mut repeat_network,
                &mut repeat_graph,
                0.1,
                &clock,
            );
            let repeat_work = work_delta(repeat_before, snapshot());
            if mode != "network" {
                assert_access_tick(&repeat_agents, transit);
            }
            assert_eq!(
                work, repeat_work,
                "PERF-06 work counts must be deterministic"
            );

            eprintln!(
                "PERF_AGENT {}",
                serde_json::json!({
                    "agents": count,
                    "mode": mode,
                    "elapsed_ms": elapsed_ms,
                    "work": work_json(work),
                })
            );
            reports.push((count, work));
        }
        let (small_count, small) = reports[0];
        let (large_count, large) = reports[1];
        let expected = large_count as f64 / small_count as f64;
        for (name, index) in [
            ("lane_bucket_agents", Phase::LaneBucketAgents),
            ("lane_bucket_entries", Phase::LaneBucketEntries),
            ("speed_agents", Phase::SpeedAgents),
            ("claim_agents", Phase::ClaimAgents),
            ("movement_agents", Phase::MovementAgents),
            ("frontage_agents", Phase::FrontageAgents),
            ("frontage_lanes", Phase::FrontageLanes),
        ] {
            let small = small[index as usize];
            let large = large[index as usize];
            let ratio = if small == 0 {
                if large != 0 { f64::INFINITY } else { 1.0 }
            } else {
                large as f64 / small as f64
            };
            let bounded = if small == 0 {
                large == 0
            } else {
                ratio <= MAX_SCALE
            };
            assert!(
                bounded,
                "PERF-06 {name} work ratio {ratio:.3} exceeds {MAX_SCALE:.3} for {expected:.1}x agents"
            );
        }
    }
}
