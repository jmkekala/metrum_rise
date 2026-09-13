// SPDX-License-Identifier: GPL-2.0-only

//! Spatial and simulation inspection bridges. Network selection is owned by the graph.

pub mod lanes;
pub mod network;
pub mod terrain;

#[cfg(test)]
mod tests {
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::interaction::get_closest_node_xz;
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    #[test]
    fn merged_alias_nodes_are_not_treated_as_live_query_nodes() {
        let mut graph = RegionGraph::new();
        let keep = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let remove = graph.add_node(Vector3::new(10.2, 0.0, 0.0), NodeType::Junction);
        let far = graph.add_node(Vector3::new(40.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(keep, far));
        graph.unite_nodes(keep, remove);

        assert!(graph.is_live_canonical_node(keep));
        assert!(!graph.is_live_canonical_node(remove));
        assert_eq!(
            get_closest_node_xz(&graph, Vector3::new(10.1, 0.0, 0.0), 2.0),
            Some(keep)
        );
    }

    #[test]
    fn deleted_edge_only_nodes_are_not_treated_as_live_query_nodes() {
        let mut graph = RegionGraph::new();
        let a = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let b = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(a, b));

        assert!(graph.is_live_canonical_node(a));
        assert_eq!(
            get_closest_node_xz(&graph, Vector3::new(0.5, 0.0, 0.0), 3.0),
            Some(a)
        );

        graph.edge_mut(edge_idx).deleted = true;

        assert!(!graph.is_live_canonical_node(a));
        assert_eq!(
            get_closest_node_xz(&graph, Vector3::new(0.5, 0.0, 0.0), 3.0),
            None
        );
    }

    #[test]
    fn canonical_node_query_uses_xz_distance_for_editor_hits() {
        let mut graph = RegionGraph::new();
        let node = graph.add_node(Vector3::new(0.0, 50.0, 0.0), NodeType::Junction);
        let far = graph.add_node(Vector3::new(20.0, 50.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(node, far));

        assert_eq!(
            get_closest_node_xz(&graph, Vector3::new(0.5, 0.0, 0.0), 3.0),
            Some(node)
        );
        assert_eq!(
            crate::simulation::network::interaction::get_closest_node(
                &graph,
                Vector3::new(0.5, 0.0, 0.0),
                3.0
            ),
            None
        );
    }

    #[test]
    fn closest_node_preserves_radius_and_lowest_id_ties() {
        let mut graph = RegionGraph::new();
        let low = graph.add_node(Vector3::new(1.0, 0.0, 0.0), NodeType::Junction);
        let high = graph.add_node(Vector3::new(-1.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(low, high));
        assert_eq!(get_closest_node_xz(&graph, Vector3::ZERO, 1.0), None);
        assert_eq!(get_closest_node_xz(&graph, Vector3::ZERO, 2.0), Some(low));
        // Moving out and back changes bucket insertion order without changing final geometry.
        graph.move_node(low, Vector3::new(40.0, 0.0, 0.0));
        graph.move_node(low, Vector3::new(1.0, 0.0, 0.0));
        assert_eq!(get_closest_node_xz(&graph, Vector3::ZERO, 2.0), Some(low));
    }

    #[test]
    fn node_grid_bounds_match_independent_point_filter() {
        let mut graph = RegionGraph::new();
        for x in [-32.0, -16.0, -0.5, 0.0, 16.0, 32.0] {
            for z in [-17.0, 0.0, 17.0] {
                graph.add_node(Vector3::new(x, x + z, z), NodeType::Junction);
            }
        }
        for (low, high) in [
            (-16.0, 16.0),
            (-0.25, 0.25),
            (16.0, -16.0),
            (-f32::MAX, f32::MAX),
            (f32::NEG_INFINITY, f32::INFINITY),
        ] {
            let min = Vector3::new(low, 0.0, low);
            let max = Vector3::new(high, 0.0, high);
            let expected: Vec<_> = graph
                .nodes()
                .iter()
                .enumerate()
                .filter_map(|(id, node)| {
                    (node.pos.x >= low
                        && node.pos.x <= high
                        && node.pos.z >= low
                        && node.pos.z <= high)
                        .then_some(id as u32)
                })
                .collect();
            assert_eq!(graph.get_nodes_near_aabb(min, max), expected);
        }
    }

    #[test]
    #[ignore = "matched release node-query locality measurement"]
    fn benchmark_editor_node_query_locality() {
        use crate::simulation::network::interaction;
        use std::{hint::black_box, time::Instant};
        const QUERIES: usize = 64;
        for background in [0, 1_000, 10_000, 100_000] {
            let setup = Instant::now();
            let mut graph = RegionGraph::new();
            for i in 0..=background {
                let x = if i == 0 {
                    0.0
                } else {
                    1_000.0 + (i % 512) as f32 * 64.0
                };
                let z = if i == 0 {
                    0.0
                } else {
                    1_000.0 + (i / 512) as f32 * 64.0
                };
                let a = graph.add_node(Vector3::new(x, 0.0, z), NodeType::Junction);
                let b = graph.add_node(Vector3::new(x + 24.0, 0.0, z), NodeType::Junction);
                let mut edge = test_edge(a, b);
                edge.geometry = vec![graph.node(a).pos, graph.node(b).pos];
                edge.physical_geometry = edge.geometry.clone();
                graph.add_edge(edge);
            }
            let setup_us = setup.elapsed().as_secs_f64() * 1e6;
            for operation in ["editor", "spatial_3d", "snap"] {
                let mut samples = Vec::new();
                for sample in 0..10 {
                    let mut outputs = [-2; QUERIES];
                    let start = Instant::now();
                    for (i, output) in outputs.iter_mut().enumerate() {
                        let position = black_box(match i % 4 {
                            0 => Vector3::new(0.25, 0.0, 0.5),
                            1 => Vector3::new(23.5, 0.0, 0.25),
                            2 => Vector3::new(-0.25, 0.0, -0.5),
                            _ => Vector3::new(-50.0, 0.0, -50.0),
                        });
                        let graph = black_box(&graph);
                        *output = match operation {
                            "editor" => get_closest_node_xz(graph, position, black_box(5.0))
                                .map_or(-1, |id| id as i32),
                            "spatial_3d" => {
                                interaction::get_closest_node(graph, position, black_box(5.0))
                                    .map_or(-1, |id| id as i32)
                            }
                            _ => interaction::get_closest_network_snap_xz(
                                graph,
                                position,
                                black_box(5.0),
                            )
                            .map_or(-1, |snap| match snap.target {
                                interaction::NetworkSnapTarget::Node(id) => id as i32,
                                interaction::NetworkSnapTarget::Edge(_) => -3,
                            }),
                        };
                    }
                    let elapsed_us = start.elapsed().as_secs_f64() * 1e6 / QUERIES as f64;
                    for (i, output) in outputs.into_iter().enumerate() {
                        assert_eq!(output, [0, 1, 0, -1][i % 4]);
                    }
                    if sample > 0 {
                        samples.push(elapsed_us);
                    }
                }
                samples.sort_by(f64::total_cmp);
                println!(
                    "NODE_QUERY_BENCH {}",
                    serde_json::json!({
                        "background":background,"nodes":graph.node_count(),"operation":operation,
                        "median_us":samples[samples.len()/2],"setup_us":setup_us,
                        "queries_per_sample":QUERIES,"fingerprint":[0,1,0,-1],
                    })
                );
            }
        }
    }

    fn test_edge(start_node: u32, end_node: u32) -> crate::simulation::network::graph::Edge {
        crate::simulation::network::graph::Edge {
            start_node,
            end_node,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            physical_length: 20.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            ..Default::default()
        }
    }
}
