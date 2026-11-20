// SPDX-License-Identifier: GPL-3.0-or-later

//! Border promotion keeps the rest of the network's lane ids: it extends one road, so only that
//! road's lanes may change, and agents elsewhere keep the lanes they hold.

use super::*;

// Draws one stroke the way the road tool commits it.
fn commit(core: &mut SimCore, start: (f32, f32), end: (f32, f32)) {
    core.precompute_road_mesh_data();
    let outcome = core.commit_road(
        super::super::road_commit::RoadCommitRequest {
            points: vec![
                Vector3::new(start.0, 0.0, start.1),
                Vector3::new(end.0, 0.0, end.1),
            ],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
            median: None,
            parking: Default::default(),
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection_reason);
}

fn node_at(core: &SimCore, x: f32, z: f32) -> u32 {
    (0..core.region_graph.node_count() as u32)
        .find(|&node| {
            let pos = core.region_graph.node(node).pos;
            (pos.x - x).abs() < 0.5 && (pos.z - z).abs() < 0.5
        })
        .expect("road end node")
}

// The live lane ids of every edge outside `rebuilt`, by edge.
fn lanes_outside(core: &SimCore, rebuilt: &HashSet<usize>) -> Vec<(usize, Vec<usize>)> {
    let mut lanes: Vec<(usize, Vec<usize>)> = core
        .transit_network
        .lane_system
        .edge_lanes
        .iter()
        .filter(|(edge, _)| !rebuilt.contains(edge))
        .map(|(&edge, ids)| (edge, ids.clone()))
        .collect();
    lanes.sort_unstable();
    lanes
}

#[test]
fn border_connection_keeps_other_roads_lane_ids() {
    let mut core = test_core();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e7;
    commit(&mut core, (0.0, 0.0), (200.0, 0.0));
    // A tee splits that road, retiring its lanes and appending new ones, so the lane table has
    // the holes a full rebuild compacts; the road drawn after it holds lane ids past them.
    commit(&mut core, (100.0, -100.0), (100.0, 0.0));
    commit(&mut core, (-300.0, 0.0), (-100.0, 0.0));
    let border = node_at(&core, 200.0, 0.0);
    let extended: HashSet<usize> = core
        .region_graph
        .node_adjacency(border)
        .iter()
        .copied()
        .collect();
    let rebuilt = crate::simulation::network::lanes::LaneSystem::incremental_rebuild_edge_closure(
        &core.region_graph,
        &extended,
    );
    let before = lanes_outside(&core, &rebuilt);
    assert!(!before.is_empty());

    core.set_border_connection_internal(border as i32);

    assert_eq!(lanes_outside(&core, &rebuilt), before);
}

// The extension moves the border node and the road's end, so both spatial indices follow it, and
// an undo of the road leaves neither behind: a stale edge entry crashed the road cursor's snap
// once the undo had dropped the edge.
#[test]
fn border_connection_keeps_the_spatial_indices_through_an_undo() {
    let mut core = test_core();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e7;
    commit(&mut core, (0.0, 0.0), (200.0, 0.0));
    let border = node_at(&core, 200.0, 0.0);
    core.set_border_connection_internal(border as i32);
    let graph = &core.region_graph;
    let edge = graph.node_adjacency(border)[0];
    let points = &graph.edge(edge).physical_geometry;
    let max_x = points.iter().map(|p| p.x).fold(f32::MIN, f32::max);
    assert!(max_x > 200.0, "the road is extended");
    let entry = graph
        .spatial_edge_entries()
        .into_iter()
        .find(|(id, _)| *id == edge)
        .expect("the extended road is indexed");
    assert_eq!(entry.1[2], max_x, "its index entry reaches the new end");
    let node_pos = graph.node(border).pos;
    assert!(
        graph
            .get_nodes_near_aabb(node_pos - Vector3::ONE, node_pos + Vector3::ONE)
            .contains(&border),
        "the border node is indexed where it now stands"
    );

    assert!(core.undo_action_internal());
    let graph = &core.region_graph;
    assert!(
        graph
            .spatial_edge_entries()
            .iter()
            .all(|(id, _)| *id < graph.edge_count() && !graph.edge(*id).deleted),
        "no index entry outlives its road: {:?}",
        graph.spatial_edge_entries()
    );
}
