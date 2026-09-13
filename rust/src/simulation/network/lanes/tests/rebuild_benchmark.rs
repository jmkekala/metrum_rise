// SPDX-License-Identifier: GPL-2.0-only

//! Matched incremental lane-rebuild locality measurements; setup and checks are not timed.

use super::*;
use crate::simulation::network::lanes::Lane;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::hint::black_box;
use std::time::Instant;

fn lane_role(lane: &Lane) -> (usize, bool, i8, bool) {
    (
        lane.edge_id,
        lane.is_fwd,
        lane.lane_idx,
        lane.lane_type == LaneType::Foot,
    )
}

fn hash_lane_shape(lane: &Lane, hash: &mut DefaultHasher) {
    lane_role(lane).hash(hash);
    lane.node_id.hash(hash);
    lane.length.to_bits().hash(hash);
    lane.frontage_delay_penalty_s.to_bits().hash(hash);
    lane.crosswalk_edge_id.hash(hash);
    lane.crosswalk_marking
        .map(|marking| {
            (
                marking.edge_id,
                [
                    marking.start.x.to_bits(),
                    marking.start.y.to_bits(),
                    marking.start.z.to_bits(),
                ],
                [
                    marking.end.x.to_bits(),
                    marking.end.y.to_bits(),
                    marking.end.z.to_bits(),
                ],
            )
        })
        .hash(hash);
    lane.geometry.len().hash(hash);
    for point in &lane.geometry {
        [point.x.to_bits(), point.y.to_bits(), point.z.to_bits()].hash(hash);
    }
    lane.cum_dist.len().hash(hash);
    for distance in &lane.cum_dist {
        distance.to_bits().hash(hash);
    }
}

fn local_products(lanes: &LaneSystem, edges: [usize; 3]) -> u64 {
    let mut hash = DefaultHasher::new();
    for edge in edges {
        for &lane_id in &lanes.edge_lanes[&edge] {
            let lane = &lanes.lanes[lane_id];
            hash_lane_shape(lane, &mut hash);
            lane.next_lanes.len().hash(&mut hash);
            for &next_id in &lane.next_lanes {
                let next = &lanes.lanes[next_id];
                hash_lane_shape(next, &mut hash);
                if next.edge_id == usize::MAX {
                    // Compare connector destinations by physical role, since appended lane IDs
                    // depend on the size of the deliberately unrelated background fixture.
                    next.next_lanes.len().hash(&mut hash);
                    for &target in &next.next_lanes {
                        lane_role(&lanes.lanes[target]).hash(&mut hash);
                    }
                }
            }
        }
    }
    hash.finish()
}

#[test]
#[ignore = "matched release lane-rebuild locality; excludes background construction and checks"]
fn incremental_lane_rebuild_scaling() {
    let mut expected_products = None;
    for background_edges in [0, 1_000, 10_000, 100_000] {
        let setup = Instant::now();
        let (mut graph, changed, expanded, preserved) = lane_rebuild_frontier();
        for i in 0..background_edges {
            let position = Vector3::new(
                10_000.0 + (i % 1_000) as f32 * 100.0,
                0.0,
                10_000.0 + (i / 1_000) as f32 * 100.0,
            );
            let start = graph.add_node(position, NodeType::Junction);
            let end = graph.add_node(position + Vector3::RIGHT * 40.0, NodeType::Junction);
            let edge = add_road_edge(&mut graph, start, end);
            graph.edge_mut(edge).allowed_types = TransitFlags::CAR;
        }
        graph.rebuild_adjacency_list();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        let setup_ms = setup.elapsed().as_secs_f64() * 1e3;
        let affected = HashSet::from([changed]);
        let local_edges = [changed, expanded, preserved];
        lanes.rebuild_edges_incremental(&mut graph, &affected);
        let products = local_products(&lanes, local_edges);
        if let Some(expected) = expected_products {
            assert_eq!(
                products, expected,
                "background roads must not change local products"
            );
        } else {
            expected_products = Some(products);
        }
        let mut samples = Vec::with_capacity(21);
        for _ in 0..21 {
            let start = Instant::now();
            lanes.rebuild_edges_incremental(black_box(&mut graph), black_box(&affected));
            samples.push(start.elapsed().as_secs_f64() * 1e3);
            assert_eq!(local_products(&lanes, local_edges), products);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "LANE_REBUILD_BENCH {}",
            serde_json::json!({
                "background_edges": background_edges, "graph_edges": graph.edge_count(),
                "local_edges": local_edges.len(), "samples": samples.len(),
                "median_ms": samples[10], "p95_ms": samples[19], "setup_ms": setup_ms,
                "local_products": format!("{products:016x}"),
            })
        );
    }
}
