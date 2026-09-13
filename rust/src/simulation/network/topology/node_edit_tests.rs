// SPDX-License-Identifier: GPL-2.0-only

//! Node-edit geometry, canonical ownership and spatial-index regressions.

use super::*;

#[test]
fn node_move_accepts_independent_control_and_physical_supports() {
    for (control, physical) in [
        (vec![0.0, 25.0, 100.0], vec![0.0, 50.0, 75.0, 100.0]),
        (vec![0.0, 25.0, 50.0, 100.0], vec![0.0, 100.0]),
    ] {
        for moved_node in [0, 1] {
            let mut graph = RegionGraph::new();
            let point = |x| Vector3::new(x, x * 0.1, 0.0);
            let start = graph.add_node(point(0.0), NodeType::Junction);
            let end = graph.add_node(point(100.0), NodeType::Junction);
            let mut edge = super::super::build_surface_edge(
                start,
                end,
                control.iter().copied().map(point).collect(),
                1,
                1,
                EdgeClass::Standard,
            );
            edge.physical_geometry = physical.iter().copied().map(point).collect();
            let edge_id = graph.add_edge(edge);
            let destination = graph.node(moved_node).pos + Vector3::new(0.0, 3.0, 0.0);

            graph.move_node(moved_node, destination);

            let edge = graph.edge(edge_id);
            for geometry in [&edge.geometry, &edge.physical_geometry] {
                assert_eq!(geometry.first(), Some(&graph.node(start).pos));
                assert_eq!(geometry.last(), Some(&graph.node(end).pos));
                assert!(geometry.iter().all(|point| point.is_finite()));
            }
        }
    }
}

#[test]
fn node_move_translates_self_loop_once() {
    let mut graph = RegionGraph::new();
    let node = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let geometry = vec![Vector3::ZERO, Vector3::new(20.0, 2.0, 10.0), Vector3::ZERO];
    let edge_id = graph.add_edge(super::super::build_surface_edge(
        node,
        node,
        geometry.clone(),
        1,
        1,
        EdgeClass::Standard,
    ));
    let delta = Vector3::new(10.0, 3.0, 5.0);

    graph.move_node(node, delta);

    let expected: Vec<_> = geometry.iter().map(|point| *point + delta).collect();
    assert_eq!(graph.edge(edge_id).geometry, expected);
    assert_eq!(graph.edge(edge_id).physical_geometry, expected);
}

#[test]
fn node_move_preserves_independent_profile_heights() {
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let end = graph.add_node(Vector3::new(100.0, 10.0, 0.0), NodeType::Junction);
    let mut edge = super::super::build_surface_edge(
        start,
        end,
        vec![
            Vector3::ZERO,
            Vector3::new(50.0, 8.0, 0.0),
            graph.node(end).pos,
        ],
        1,
        1,
        EdgeClass::Standard,
    );
    edge.physical_geometry = vec![
        Vector3::ZERO,
        Vector3::new(25.0, 2.0, 0.0),
        Vector3::new(75.0, 12.0, 0.0),
        graph.node(end).pos,
    ];
    let edge_id = graph.add_edge(edge);

    graph.move_node(start, Vector3::new(8.0, 4.0, 16.0));

    let expected_controls = vec![
        Vector3::new(8.0, 4.0, 16.0),
        Vector3::new(31.75, 7.375, 13.5),
        Vector3::new(54.0, 10.0, 8.0),
        Vector3::new(76.25, 9.625, 2.5),
        Vector3::new(100.0, 10.0, 0.0),
    ];
    let mut expected_physical = expected_controls.clone();
    expected_physical[1].y = 5.375;
    expected_physical[2].y = 9.0;
    expected_physical[3].y = 12.625;
    assert_eq!(graph.edge(edge_id).geometry, expected_controls);
    assert_eq!(graph.edge(edge_id).physical_geometry, expected_physical);
}

#[test]
fn node_edits_refresh_physical_length_and_routing_cost() {
    for merge in [false, true] {
        let mut graph = RegionGraph::new();
        let high = graph.add_node(Vector3::new(0.0, 20.0, 0.0), NodeType::Junction);
        let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let end = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let mut edge = super::super::build_surface_edge(
            start,
            end,
            vec![graph.node(start).pos, graph.node(end).pos],
            1,
            1,
            EdgeClass::Standard,
        );
        edge.speed_limit = 10.0;
        edge.base_cost = 10.0;
        let edge_id = graph.add_edge(edge);
        if merge {
            graph.unite_nodes(high, start);
        } else {
            graph.move_node(start, graph.node(high).pos);
        }

        let edge = graph.edge(edge_id);
        assert!((edge.physical_length - 101.98039).abs() < 0.001);
        assert!((edge.base_cost - 20.396078).abs() < 0.001);
    }
}

#[test]
fn node_merge_replaces_its_original_spatial_entry() {
    let mut graph = RegionGraph::new();
    let keep = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let remove = graph.add_node(Vector3::new(30.0, 0.0, 0.0), NodeType::Junction);
    let far = graph.add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
    let edge_id = graph.add_edge(super::super::build_surface_edge(
        remove,
        far,
        vec![graph.node(remove).pos, graph.node(far).pos],
        1,
        1,
        EdgeClass::Standard,
    ));

    graph.unite_nodes(keep, remove);

    assert_eq!(graph.node_adjacency(keep), &[edge_id]);
    assert!(graph.node_adjacency(remove).is_empty());
    assert_eq!(
        graph.get_edges_near_aabb(Vector3::new(-1.0, 0.0, -1.0), Vector3::new(61.0, 0.0, 1.0)),
        vec![edge_id],
        "the spatial index must contain one current entry per live edge"
    );
    assert!(
        graph
            .find_node_within(Vector3::new(30.0, 0.0, 0.0), 1.0)
            .is_none()
    );
    graph.add_node_to_spatial_index(remove);
    assert!(
        graph
            .get_nodes_near_aabb(Vector3::new(29.0, 0.0, -1.0), Vector3::new(31.0, 0.0, 1.0))
            .is_empty()
    );
}

#[test]
fn node_merge_uses_lowest_canonical_parent() {
    let mut graph = RegionGraph::new();
    for x in [0.0, 10.0, 20.0] {
        graph.add_node(Vector3::new(x, 0.0, 0.0), NodeType::Junction);
    }
    graph.unite_nodes(0, 2);
    graph.unite_nodes(1, 2);

    for node in 0..3 {
        assert_eq!(graph.get_valid_node(node), 0);
    }
}

#[test]
#[ignore = "matched release node-edit benchmark; graph setup, cloning and checks are outside timing"]
fn benchmark_node_edit_locality() {
    use std::hint::black_box;
    use std::time::Instant;

    for background in [0, 1_000, 10_000, 100_000] {
        let setup = Instant::now();
        let mut graph = RegionGraph::new();
        graph.add_node(Vector3::ZERO, NodeType::Junction);
        graph.add_node(Vector3::ZERO, NodeType::Junction);
        graph.add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
        let add_edge = |graph: &mut RegionGraph, start, end| {
            let mut edge = super::super::build_surface_edge(
                start,
                end,
                vec![graph.node(start).pos, graph.node(end).pos],
                1,
                1,
                EdgeClass::Standard,
            );
            edge.base_cost = edge.physical_length / edge.speed_limit;
            graph.add_edge(edge);
        };
        add_edge(&mut graph, 1, 2);
        for id in 0..background {
            let origin = Vector3::new(
                10_000.0 + (id % 1000) as f32 * 100.0,
                0.0,
                10_000.0 + (id / 1000) as f32 * 100.0,
            );
            let start = graph.add_node(origin, NodeType::Junction);
            let end = graph.add_node(origin + Vector3::new(40.0, 0.0, 0.0), NodeType::Junction);
            add_edge(&mut graph, start, end);
        }
        let setup_us = setup.elapsed().as_secs_f64() * 1e6;
        for operation in ["merge", "move"] {
            let mut samples = Vec::new();
            let mut clones = Vec::new();
            let mut product = None;
            for sample in 0..10 {
                let begin = Instant::now();
                let mut local = graph.clone();
                let clone_us = begin.elapsed().as_secs_f64() * 1e6;
                let begin = Instant::now();
                if operation == "merge" {
                    black_box(&mut local).unite_nodes(0, 1);
                } else {
                    black_box(&mut local).move_node(1, Vector3::new(0.0, 3.0, 0.0));
                }
                let update_us = begin.elapsed().as_secs_f64() * 1e6;
                if sample != 0 {
                    samples.push(update_us);
                    clones.push(clone_us);
                }
                let edge = local.edge(0);
                assert_eq!(edge.start_node, if operation == "merge" { 0 } else { 1 });
                assert_eq!(
                    edge.physical_geometry.first(),
                    Some(&local.node(edge.start_node).pos)
                );
                assert_eq!(edge.physical_geometry.last(), Some(&local.node(2).pos));
                assert_eq!(
                    local.get_edges_near_point(Vector3::new(30.0, 0.0, 0.0), 1.0),
                    vec![0]
                );
                let mut hash = 0xcbf29ce484222325_u64;
                for point in edge.geometry.iter().chain(&edge.physical_geometry) {
                    for component in [point.x, point.y, point.z] {
                        hash = (hash ^ u64::from(component.to_bits())).wrapping_mul(0x100000001b3);
                    }
                }
                if let Some(previous) = product {
                    assert_eq!(hash, previous);
                }
                product = Some(hash);
            }
            samples.sort_by(f64::total_cmp);
            clones.sort_by(f64::total_cmp);
            println!(
                "NODE_EDIT_BENCH {}",
                serde_json::json!({"operation":operation,"background":background,"samples":samples.len(),"median_us":samples[samples.len()/2],"clone_us":clones[clones.len()/2],"setup_us":setup_us,"fingerprint":format!("{:016x}",product.unwrap())})
            );
        }
    }

    // The old mismatched-profile path fails its regression, so this phase records the
    // corrected path's absolute scaling rather than inventing a before/after comparison.
    for control_points in [16, 256, 4096] {
        let length = (control_points - 1) as f32 * 2.0;
        let height = |x: f32| x.min(length - x) * 0.125;
        let mut edge = super::super::build_surface_edge(
            0,
            1,
            (0..control_points)
                .map(|i| Vector3::new(i as f32 * 2.0, 0.0, 0.0))
                .collect(),
            1,
            1,
            EdgeClass::Standard,
        );
        edge.physical_geometry.clear();
        edge.physical_geometry.push(Vector3::ZERO);
        for i in 1..control_points {
            let x = i as f32 * 2.0 - 1.0;
            edge.physical_geometry.push(Vector3::new(x, height(x), 0.0));
        }
        edge.physical_geometry.push(Vector3::new(length, 0.0, 0.0));
        let mut samples = Vec::with_capacity(21);
        let mut product = None;
        for sample in 0..22 {
            let mut local = edge.clone();
            let begin = Instant::now();
            super::node_geometry::deform_edge(
                black_box(&mut local),
                true,
                Vector3::new(0.0, 4.0, 0.0),
            );
            let elapsed_us = begin.elapsed().as_secs_f64() * 1e6;
            if sample != 0 {
                samples.push(elapsed_us);
            }
            assert_eq!(local.geometry.len(), control_points * 2 - 1);
            assert_eq!(local.geometry.len(), local.physical_geometry.len());
            assert_eq!(local.physical_geometry[0], Vector3::new(0.0, 4.0, 0.0));
            assert_eq!(
                local.physical_geometry.last(),
                Some(&Vector3::new(length, 0.0, 0.0))
            );
            let mut hash = 0xcbf29ce484222325_u64;
            for (control, physical) in local.geometry.iter().zip(&local.physical_geometry) {
                assert_eq!((control.x, control.z), (physical.x, physical.z));
                assert!((physical.y - control.y - height(physical.x)).abs() < 0.0001);
                for component in [
                    control.x, control.y, control.z, physical.x, physical.y, physical.z,
                ] {
                    hash = (hash ^ u64::from(component.to_bits())).wrapping_mul(0x100000001b3);
                }
            }
            if let Some(previous) = product {
                assert_eq!(hash, previous);
            }
            product = Some(hash);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "NODE_PROFILE_BENCH {}",
            serde_json::json!({"control_points":control_points,"physical_points":edge.physical_geometry.len(),"aligned_points":control_points*2-1,"samples":samples.len(),"median_us":samples[samples.len()/2],"fingerprint":format!("{:016x}",product.unwrap())})
        );
    }
}
