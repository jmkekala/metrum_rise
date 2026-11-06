// SPDX-License-Identifier: GPL-2.0-only

//! The roundabout edit on a city (`ROAD-62`, round R7b): what a ring on empty ground and a
//! conversion build, the preview being the commit, the refusals changing nothing, undo restoring
//! the graph, pieces and lanes, the cars and walkers in the circle, the dependents of a trimmed
//! arm, a stroke joined to a bare ring and the bulldozed ring. Ported from Veeti's road-chain
//! snapshot `efd58129` (`nodes/sim/roundabouts.rs`), adapted to the constructive roadbed and the
//! pocket's transaction.

use super::road_crosswalks::{live_arms, products};
use super::road_in_place::house_fronting;
use super::*;
use crate::simulation::economy::agents::MODE_WALK;
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::roundabouts::{RoundaboutRequest, RoundaboutSite, ring_at};

mod joins;

// A four-way crossing at the origin of two roads 300 m long, `lanes` each way. Returns the core
// and the crossing node.
fn crossing(lanes: i32) -> (SimCore, u32) {
    let mut core = test_core();
    core.treasury.balance = 1.0e7;
    for points in [
        vec![Vector3::new(-150.0, 0.0, 0.0), Vector3::new(150.0, 0.0, 0.0)],
        vec![Vector3::new(0.0, 0.0, -150.0), Vector3::new(0.0, 0.0, 150.0)],
    ] {
        assert!(core.add_road_internal(points, lanes, lanes).committed);
    }
    core.precompute_road_mesh_data();
    let node = (0..core.region_graph.node_count() as u32)
        .find(|&node| live_arms(&core, node).len() == 4)
        .expect("the crossing");
    (core, node)
}

fn ground(centre: Vector3, lanes: u8, diameter_m: f32) -> RoundaboutRequest {
    RoundaboutRequest {
        site: RoundaboutSite::Ground { centre, lanes },
        diameter_m,
    }
}

fn junction(node: u32, diameter_m: f32) -> RoundaboutRequest {
    RoundaboutRequest {
        site: RoundaboutSite::Junction(node),
        diameter_m,
    }
}

fn counts(core: &SimCore) -> (usize, usize) {
    (
        core.region_graph.node_count(),
        core.region_graph.edge_count(),
    )
}

// The roundabout edit's verdict and its commit: `""` when applied, else the refusal's reason. The
// preview must give the same verdict and change nothing, and a refusal must change nothing.
fn ring_edit(core: &mut SimCore, request: RoundaboutRequest) -> String {
    let before = (products(core), core.undo_stack.len(), counts(core));
    let preview = core
        .preview_roundabout_internal(request)
        .err()
        .map(|refusal| {
            eprintln!("roundabout refusal {}: {}", refusal.reason, refusal.detail);
            refusal.reason
        })
        .unwrap_or_default();
    assert_eq!(products(core), before.0, "a preview changes nothing");
    assert_eq!(counts(core), before.2);
    let applied = core.place_roundabout_internal(request);
    assert_eq!(
        applied,
        preview.is_empty(),
        "the preview is the commit: {preview}"
    );
    if !applied {
        assert_eq!(
            (products(core), core.undo_stack.len(), counts(core)),
            before,
            "{preview}"
        );
    }
    preview
}

fn ring_edges(core: &SimCore) -> Vec<usize> {
    let edges = core.region_graph.edges().iter().enumerate();
    edges
        .filter(|(_, edge)| !edge.deleted && edge.roundabout.is_some())
        .map(|(id, _)| id)
        .collect()
}

// Every live road's record the edit may change, in ids, for bitwise comparison.
fn graph_state(core: &SimCore) -> String {
    let graph = &core.region_graph;
    let mut out = String::new();
    for (id, node) in graph.nodes().iter().enumerate() {
        let id = id as u32;
        let mut overrides: Vec<_> = node.crosswalk_overrides.iter().collect();
        overrides.sort();
        // Undo restores adjacency ascending, as a load builds it.
        let mut arms = graph.node_adjacency(id).to_vec();
        arms.sort_unstable();
        out += &format!(
            "{id} {:?} {:?} {overrides:?} {arms:?}\n",
            node.pos, node.node_type
        );
    }
    for (id, edge) in graph.edges().iter().enumerate() {
        out += &format!(
            "{id} {} {} {} {} {} {:?} {:?}\n",
            edge.deleted,
            edge.start_node,
            edge.end_node,
            edge.start_clip.to_bits(),
            edge.end_clip.to_bits(),
            edge.geometry,
            edge.physical_geometry
        );
    }
    out
}

// A ring on empty ground: three ring edges of three pass-through nodes, one undo, its build cost
// charged, and undo back to the empty graph bitwise. The preview's outline closes on itself.
#[test]
fn a_ring_on_empty_ground_is_one_edit_and_one_undo() {
    let mut core = test_core();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e7;
    core.precompute_road_mesh_data();
    let before = (graph_state(&core), core.treasury.balance);
    let request = ground(Vector3::new(40.0, 0.0, 60.0), 1, 36.0);
    let verdict = core.preview_roundabout_internal(request).unwrap();
    assert!(verdict.arms.is_empty());
    let (first, last) = (verdict.ring[0], *verdict.ring.last().unwrap());
    assert_eq!(first, last, "the outline is closed");
    assert!(verdict.cost > 0.0);
    assert!(core.place_roundabout_internal(request));
    assert_eq!(ring_edges(&core).len(), 3);
    assert_eq!(core.undo_stack.len(), 1);
    assert!((core.treasury.balance - (before.1 - verdict.cost)).abs() < 1e-6);
    let lanes = &core.transit_network.lane_system;
    assert_eq!(lanes.edge_lanes.len(), 3, "the ring has its lanes");
    core.precompute_road_mesh_data();
    assert!(core.transit_network.roadbed.failed_nodes().is_empty());
    assert!(core.undo_action_internal());
    assert_eq!(graph_state(&core), before.0, "undo restores the graph");
    assert!(core.transit_network.lane_system.edge_lanes.is_empty());
}

// Ported: `roundabout_replacement_retains_uneven_arms_and_original_edge_ids`. A conversion keeps
// the four arms' ids and far ends and retires the junction; undo restores graph, pieces and lanes
// bitwise.
#[test]
fn a_conversion_keeps_the_arms_and_undo_restores_the_city() {
    for (lanes, diameter_m, ring_lanes) in [(1, 36.0, 1), (2, 50.0, 2)] {
        let (mut core, node) = crossing(lanes);
        let arms = live_arms(&core, node);
        let far: Vec<u32> = arms
            .iter()
            .map(|&arm| {
                let edge = core.region_graph.edge(arm);
                if edge.start_node == node {
                    edge.end_node
                } else {
                    edge.start_node
                }
            })
            .collect();
        let (state, before, count) = (graph_state(&core), products(&mut core), counts(&core));
        assert_eq!(ring_edit(&mut core, junction(node, diameter_m)), "");
        let ring = ring_edges(&core);
        assert_eq!(ring.len(), 4);
        for &id in &ring {
            assert_eq!(core.region_graph.edge(id).fwd_lanes, ring_lanes);
        }
        assert!(live_arms(&core, node).is_empty(), "the junction is retired");
        assert_eq!(core.region_graph.find_node_within(Vector3::ZERO, 2.0), None);
        for (&arm, &far) in arms.iter().zip(&far) {
            let edge = core.region_graph.edge(arm);
            assert!(!edge.deleted);
            assert!(edge.start_node == far || edge.end_node == far, "{arm}");
            let at_ring = if edge.start_node == far {
                edge.end_node
            } else {
                edge.start_node
            };
            assert!(ring_at(&core.region_graph, at_ring).is_some());
        }
        let lanes_at_old = &core.transit_network.lane_system.node_lanes;
        assert!(!lanes_at_old.contains_key(&(node as usize)));
        products(&mut core);
        assert!(core.undo_action_internal());
        assert_eq!(counts(&core), count);
        assert_eq!(graph_state(&core), state, "undo restores the graph");
        assert_eq!(products(&mut core), before, "undo restores the pieces and lanes");
    }
}

fn lane_of(core: &SimCore, edge: usize, kind: LaneType) -> usize {
    let lanes = &core.transit_network.lane_system;
    lanes.edge_lanes[&edge]
        .iter()
        .copied()
        .find(|&id| lanes.lanes[id].lane_type == kind)
        .unwrap()
}

// Risk 3: a car at the old centre, on a connector of the junction, and a walker on a cut part of
// an arm: both inside the circle. Each reattaches to a lane within the 30 m reach and stays where
// it is on the map; undo reattaches them to a live lane too.
#[test]
fn a_car_and_a_walker_in_the_circle_reattach() {
    use crate::nodes::sim::render::lane_pose::sample_lane_pose;
    let (mut core, node) = crossing(1);
    let arm = live_arms(&core, node)[0];
    let edge = core.region_graph.edge(arm);
    let far = if edge.start_node == node {
        edge.end_node
    } else {
        edge.start_node
    };
    let lanes = &core.transit_network.lane_system;
    let connector = lanes.node_lanes[&(node as usize)]
        .iter()
        .copied()
        .find(|&id| lanes.lanes[id].lane_type == LaneType::Vehicle)
        .expect("a connector");
    let middle = 0.5 * lanes.lanes[connector].length;
    let was = sample_lane_pose(&lanes.lanes[connector], middle).unwrap().0;
    let car = core.agents.spawn_debug_car(
        vec![node, far],
        &core.transit_network.lane_system,
        connector,
        godot::prelude::Vector2::new(was.x, was.z),
    );
    core.agents.agents.lane_distance[car] = middle;
    let walk = lane_of(&core, arm, LaneType::Foot);
    let lanes = &core.transit_network.lane_system;
    let near_node = lanes.lanes[walk].geometry[0].length() < 75.0;
    let station = if near_node {
        5.0
    } else {
        lanes.lanes[walk].length - 5.0
    };
    let walked = sample_lane_pose(&lanes.lanes[walk], station).unwrap().0;
    let walker = core.agents.spawn_housed_agent(usize::MAX, walked.x, walked.z);
    core.agents.agents.transit_mode[walker] = MODE_WALK;
    core.agents
        .agents
        .enter_lane(walker, &core.transit_network.lane_system, walk, station, far);

    assert_eq!(ring_edit(&mut core, junction(node, 60.0)), "");
    let lanes = &core.transit_network.lane_system;
    let attached = |agent: usize, kind: LaneType| {
        let id = core.agents.agents.current_lane_id[agent];
        let lane = &lanes.lanes[id];
        assert_eq!(lane.lane_type, kind);
        assert!(!lane.geometry.is_empty(), "a live lane");
        sample_lane_pose(lane, core.agents.agents.lane_distance[agent])
            .unwrap()
            .0
    };
    assert!(attached(car, LaneType::Vehicle).distance_to(was) <= 30.0);
    assert!(
        core.agents.agents.current_path[car].is_empty(),
        "the retired junction leaves the live route"
    );
    assert!(attached(walker, LaneType::Foot).distance_to(walked) <= 30.0);
    assert!(core.undo_action_internal());
    let lanes = &core.transit_network.lane_system;
    for agent in [car, walker] {
        let lane = &lanes.lanes[core.agents.agents.current_lane_id[agent]];
        assert!(!lane.geometry.is_empty(), "a live lane after undo");
    }
}

// The refusal list (PLAN-R7 3.1): each changes nothing, and the preview says the same.
#[test]
fn refusals_change_nothing() {
    let (mut core, node) = crossing(1);
    for (request, reason) in [
        (junction(node, 12.0), "invalid_roundabout_dimensions"),
        (junction(node, f32::NAN), "invalid_roundabout_dimensions"),
        (junction(node, 80.0), "roundabout_replacement_too_large"),
        (junction(9999, 36.0), "invalid_roundabout_junction"),
        (
            ground(Vector3::new(16.0, 0.0, 16.0), 1, 36.0),
            "roundabout_overlaps_road",
        ),
    ] {
        assert_eq!(ring_edit(&mut core, request), reason);
    }
    // A road end and a bend are not junctions.
    let end = core.region_graph.find_node_within(Vector3::new(150.0, 0.0, 0.0), 1.0);
    assert_eq!(
        ring_edit(&mut core, junction(end.unwrap(), 36.0)),
        "roundabout_requires_three_to_six_arms"
    );
    // An arm too short for the ring: the cut reaches its far node.
    let (mut core, node) = crossing(1);
    let stub = [Vector3::new(25.0, 0.0, 0.0), Vector3::new(25.0, 0.0, 60.0)];
    assert!(core.add_road_internal(stub.to_vec(), 1, 1).committed);
    assert_eq!(
        ring_edit(&mut core, junction(node, 60.0)),
        "roundabout_arm_too_short"
    );
}

// Arms closer than their mouths, in the tool's words: six 1+1 arms, two of them 30 degrees apart,
// have 6.9 m of ring between them at 30 m and refuse; at 60 m they have 14.8 m and convert.
#[test]
fn six_crowded_arms_refuse_at_30_m_and_fit_at_60_m() {
    let six = |core: &mut SimCore| {
        for angle in [0f32, 30.0, 110.0, 180.0, 240.0, 300.0] {
            let (sin, cos) = angle.to_radians().sin_cos();
            let far = Vector3::new(cos * 150.0, 0.0, sin * 150.0);
            assert!(core.add_road_internal(vec![Vector3::ZERO, far], 1, 1).committed);
        }
        core.precompute_road_mesh_data();
        (0..core.region_graph.node_count() as u32)
            .find(|&node| live_arms(core, node).len() == 6)
            .expect("the hub")
    };
    let mut core = test_core();
    let node = six(&mut core);
    assert_eq!(ring_edit(&mut core, junction(node, 30.0)), "roundabout_arms_too_close");
    assert_eq!(ring_edit(&mut core, junction(node, 60.0)), "");
}

// A ring over water refuses as a road does; a whitelisted junction and a pocketed arm wait for
// the player; a road beside the site is in its way.
#[test]
fn the_rest_of_the_refusal_list_stands_on_a_city() {
    use crate::simulation::network::pockets::PocketRequest;
    use crate::simulation::network::roadbed::EdgeEnd;
    let (mut core, node) = crossing(1);
    let arm = live_arms(&core, node)[0];
    let end = if core.region_graph.edge(arm).start_node == node {
        EdgeEnd::Start
    } else {
        EdgeEnd::End
    };
    let pocket = PocketRequest {
        edge: arm,
        end,
        left: false,
        right: true,
        body_m: 40.0,
    };
    assert!(core.set_road_pocket_internal(pocket));
    // Its pocket lane makes two lanes in, so the ring would have two.
    assert_eq!(ring_edit(&mut core, junction(node, 50.0)), "roundabout_arm_has_pocket");
    assert!(core.undo_action_internal());
    products(&mut core);
    let lanes = &core.transit_network.lane_system;
    let arm_lane = lanes.edge_lanes[&arm]
        .iter()
        .copied()
        .find(|&id| lanes.lanes[id].lane_type == LaneType::Vehicle)
        .unwrap();
    let (from, lane) = (arm as i32, lanes.lanes[arm_lane].lane_idx as i32);
    let other = live_arms(&core, node)[1] as i32;
    core.set_lane_connection_internal(node, from, lane, other, 0);
    assert_eq!(
        ring_edit(&mut core, junction(node, 36.0)),
        "roundabout_has_lane_restrictions"
    );
}

// D20: a ring's street lights stand on its outer sidewalk, the island has none, on a bare ring and
// a converted one.
#[test]
fn a_rings_lights_stand_on_its_outer_sidewalk() {
    let outer_lights = |core: &mut SimCore, kerb_m: f32| {
        core.precompute_road_mesh_data();
        let mesh = core
            .transit_network
            .try_generate_mesh_data(&core.region_graph, &core.heightmap)
            .expect("current roadbed");
        assert!(!mesh.street_lights.is_empty());
        for light in &mesh.street_lights {
            let from_centre = light.position.x.hypot(light.position.z);
            assert!(from_centre > kerb_m, "a light {from_centre} m from the centre");
        }
    };
    let mut core = test_core();
    assert_eq!(ring_edit(&mut core, ground(Vector3::ZERO, 1, 36.0)), "");
    outer_lights(&mut core, 18.0);
    let (mut core, node) = crossing(2);
    assert_eq!(ring_edit(&mut core, junction(node, 50.0)), "");
    outer_lights(&mut core, 25.0);
}

// A ring reaching visible water refuses, as a road does.
#[test]
fn a_ring_refuses_water_under_it() {
    // A pond 32-47 m east of the origin.
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    let mut water = WaterSystem::with_chunking(257, 257, 1.0, 129);
    let mut depth = vec![0.0; 257 * 257];
    for z in 120..=136 {
        for x in 160..=175 {
            depth[x + z * 257] = 2.0;
        }
    }
    water.replace_baseline_depth_from_dense(&depth).unwrap();
    core.watermap = water;
    let request = ground(Vector3::new(35.0, 0.0, 0.0), 1, 36.0);
    assert_eq!(ring_edit(&mut core, request), "water_requires_bridge");
    let far = ground(Vector3::new(-60.0, 0.0, 0.0), 1, 36.0);
    assert_eq!(ring_edit(&mut core, far), "");
}

// Lots under the ring and on its island refuse; the same crossing with the house far off
// converts, the house's cell moves by what the cut took off a start, and undo puts it back.
#[test]
fn a_trimmed_arms_dependents_move_and_return() {
    let (mut core, node) = crossing(1);
    let arm = core
        .region_graph
        .find_node_within(Vector3::new(150.0, 0.0, 0.0), 1.0)
        .map(|end| live_arms(&core, end)[0])
        .unwrap();
    let (house, _) = house_fronting(&mut core, arm, (100.0, -12.0));
    let cell_m = core.config.zone_cell_m;
    let edge = core.region_graph.edge(arm);
    let starts_at_node = edge.start_node == node;
    let station: f32 = if starts_at_node { 100.0 } else { 50.0 };
    core.allocator.buildings[house].cell_x = (station / cell_m).floor() as usize;
    core.allocator.edge_occupancy.insert(
        arm,
        crate::simulation::buildings::allocator::EdgeOccupancy {
            cells_long: 12,
            left: vec![true; 12],
            right: vec![false; 12],
        },
    );
    let row = |core: &SimCore| {
        let building = &core.allocator.buildings[house];
        (
            building.edge_idx,
            building.cell_x,
            building.frontage_t.to_bits(),
            core.allocator.edge_occupancy[&arm].cells_long,
        )
    };
    let original = row(&core);
    let length_m = core.region_graph.edge(arm).physical_length;
    assert_eq!(ring_edit(&mut core, junction(node, 36.0)), "");
    let moved = row(&core);
    let cut_m = length_m - core.region_graph.edge(arm).physical_length;
    assert!(cut_m > 15.0);
    assert_eq!(moved.0, arm);
    let dropped = if starts_at_node {
        (cut_m / cell_m).floor() as usize
    } else {
        0
    };
    assert_eq!(moved.1, original.1 - dropped);
    assert_ne!(moved.2, original.2, "the frontage is along the shorter arm");
    assert!(core.undo_action_internal());
    assert_eq!(row(&core), original);

    // A house on the corner lot is on the ring's land.
    let (mut core, node) = crossing(1);
    let arm = core
        .region_graph
        .find_node_within(Vector3::new(150.0, 0.0, 0.0), 1.0)
        .map(|end| live_arms(&core, end)[0])
        .unwrap();
    house_fronting(&mut core, arm, (20.0, -12.0));
    assert_eq!(ring_edit(&mut core, junction(node, 60.0)), "parcel_overlap");
}

// D15: a ring goes whole. Its arms stay, ending on the ring's old line; one undo brings it back.
#[test]
fn bulldozing_a_ring_piece_removes_the_ring_and_leaves_the_arms_as_road_ends() {
    let (mut core, node) = crossing(1);
    assert_eq!(ring_edit(&mut core, junction(node, 36.0)), "");
    let ring = ring_edges(&core);
    let arms: Vec<usize> = core
        .region_graph
        .edges()
        .iter()
        .enumerate()
        .filter(|(id, edge)| !edge.deleted && !ring.contains(id))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(arms.len(), 4);
    let (state, before, undone) = (graph_state(&core), products(&mut core), core.undo_stack.len());
    assert!(core.bulldoze_road_edge_for_test(ring[1]));
    assert_eq!(core.undo_stack.len(), undone + 1, "one undo");
    assert!(ring_edges(&core).is_empty());
    for &arm in &arms {
        let edge = core.region_graph.edge(arm);
        assert!(!edge.deleted);
        for end in [edge.start_node, edge.end_node] {
            assert_eq!(live_arms(&core, end).len(), 1, "arm {arm} is a road end");
        }
    }
    products(&mut core);
    assert!(core.undo_action_internal());
    assert_eq!(graph_state(&core), state);
    assert_eq!(products(&mut core), before);
}

// Give-way is rebuilt with the lanes on a load, never saved (`ROAD-62`, round R7c): each entry
// again names a circulating connector into its own ring lane.
#[test]
fn a_rings_give_way_returns_with_the_lanes_on_a_load() {
    let entries = |core: &SimCore| {
        let system = &core.transit_network.lane_system;
        let lanes = &system.lanes;
        let mut found: Vec<(usize, usize)> = lanes
            .iter()
            .enumerate()
            .filter_map(|(id, lane)| {
                let entry = system.ring_entry(id)?;
                let target = lane.next_lanes[0];
                let circulating = &lanes[entry.circulating as usize];
                assert_eq!(circulating.next_lanes[0], target);
                assert_eq!(circulating.node_id, lane.node_id);
                Some((lanes[target].edge_id, lanes[entry.ring_in as usize].edge_id))
            })
            .collect();
        found.sort_unstable();
        found
    };
    let (mut core, node) = crossing(1);
    assert_eq!(ring_edit(&mut core, junction(node, 36.0)), "");
    let before = entries(&core);
    assert_eq!(before.len(), 4);
    let path = temp_save_path("road_ring_give_way");
    core.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    let mut loaded = test_core();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(entries(&loaded), before);
}

// Two rings a block apart and cars queued to cross both ways, as `METRUM_PERF_ROUNDABOUTS` builds
// on the street grid: a link of about 3 m between the rings, whose head enters one ring while the
// other ring's head waits to leave onto the opposite link. An entry that gave way to a head
// standing at its exit locked this pair for good (`ROAD-62`, round R7c).
fn crossing_pair_state(workers: usize) -> Vec<(usize, u32, u32)> {
    let mut core = test_core();
    core.treasury.balance = 1.0e7;
    for points in [
        vec![Vector3::new(-300.0, 0.0, 0.0), Vector3::new(300.0, 0.0, 0.0)],
        vec![Vector3::new(-25.0, 0.0, -150.0), Vector3::new(-25.0, 0.0, 150.0)],
        vec![Vector3::new(25.0, 0.0, -150.0), Vector3::new(25.0, 0.0, 150.0)],
    ] {
        assert!(core.add_road_internal(points, 1, 1).committed);
    }
    core.precompute_road_mesh_data();
    let at = |core: &SimCore, x: f32| {
        (0..core.region_graph.node_count() as u32)
            .find(|&node| {
                live_arms(core, node).len() == 4
                    && (core.region_graph.node(node).pos.x - x).abs() < 1.0
            })
            .expect("a crossing")
    };
    for x in [-25.0, 25.0] {
        let node = at(&core, x);
        assert_eq!(ring_edit(&mut core, junction(node, 36.0)), "");
    }
    // The node path between two nodes over the one-way graph, breadth first.
    let route = |core: &SimCore, from: u32, to: u32| {
        let graph = &core.region_graph;
        let mut came_from = vec![u32::MAX; graph.node_count()];
        let mut queue = std::collections::VecDeque::from([from]);
        came_from[from as usize] = from;
        while let Some(node) = queue.pop_front() {
            for &id in graph.node_adjacency(node) {
                let edge = graph.edge(id);
                let next = if edge.start_node == node && edge.fwd_lanes > 0 {
                    edge.end_node
                } else if edge.end_node == node && edge.bkw_lanes > 0 {
                    edge.start_node
                } else {
                    continue;
                };
                if !edge.deleted && came_from[next as usize] == u32::MAX {
                    came_from[next as usize] = node;
                    queue.push_back(next);
                }
            }
        }
        let mut path = vec![to];
        while *path.last().unwrap() != from {
            path.push(came_from[*path.last().unwrap() as usize]);
        }
        path.reverse();
        path
    };
    let end_at = |core: &SimCore, x: f32| {
        (0..core.region_graph.node_count() as u32)
            .find(|&node| (core.region_graph.node(node).pos.x - x).abs() < 1.0)
            .expect("a road end")
    };
    let (west, east) = (end_at(&core, -300.0), end_at(&core, 300.0));
    let mut targets = Vec::new();
    for (from, to) in [(west, east), (east, west)] {
        let path = route(&core, from, to);
        let edge = core
            .region_graph
            .get_edge_between_nodes(path[0], path[1])
            .unwrap();
        let forward = core.region_graph.edge(edge).start_node == from;
        let lanes = &core.transit_network.lane_system;
        let lane = lanes.edge_lanes[&edge]
            .iter()
            .copied()
            .find(|&id| lanes.lanes[id].lane_type == LaneType::Vehicle && lanes.lanes[id].is_fwd == forward)
            .unwrap();
        let length = lanes.lanes[lane].length;
        let last = core
            .region_graph
            .get_edge_between_nodes(path[path.len() - 2], to)
            .unwrap();
        for queue in 0..24 {
            let id = core.agents.spawn_debug_car(
                path.clone(),
                lanes,
                lane,
                godot::prelude::Vector2::ZERO,
            );
            core.agents.agents.current_node[id] = from;
            core.agents.agents.lane_distance[id] = length - 2.0 - queue as f32 * 6.0;
            targets.push((id, last));
        }
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap()
        .install(|| {
            let mut crossed = vec![false; targets.len()];
            for _ in 0..12_000 {
                core.agents.tick(
                    &core.allocator,
                    &mut core.transit_network,
                    &mut core.region_graph,
                    0.1,
                    &core.time,
                );
                for (k, &(id, last)) in targets.iter().enumerate() {
                    crossed[k] |= core.agents.agents.current_edge[id] == last;
                }
                if crossed.iter().all(|&crossed| crossed) {
                    break;
                }
            }
            let left = crossed.iter().filter(|&&crossed| !crossed).count();
            assert_eq!(left, 0, "cars still between the rings");
        });
    let agents = &core.agents.agents;
    (0..agents.len())
        .map(|i| {
            (
                agents.current_lane_id[i],
                agents.lane_distance[i].to_bits(),
                agents.speed[i].to_bits(),
            )
        })
        .collect()
}

#[test]
fn queues_crossing_two_neighbouring_rings_both_ways_get_through() {
    assert_eq!(crossing_pair_state(1), crossing_pair_state(4));
}
