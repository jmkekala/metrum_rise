//! Black-box renderer tests for the graph-dilation road mesher.
//!
//! These tests validate the visible road/sidewalk contract of the replacement renderer rather
//! than any specific internal junction contour implementation.

#[cfg(test)]
mod tests {
    use crate::config;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::render::road::RoadRenderer;
    use crate::simulation::network::render::{NetworkMeshData, TransitRenderer};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use crate::simulation::terrain::TerrainSystem;
    use godot::prelude::*;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum VisibleSurface {
        None,
        Road,
        Sidewalk,
    }

    fn create_test_edge(n1: u32, n2: u32, p1: Vector3, p2: Vector3, width: f32) -> Edge {
        Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width,
            fwd_lanes: ((width / config::LANE_WIDTH).round() as u8).max(1),
            bkw_lanes: 0,
            speed_limit: 13.0,
            base_cost: 0.0,
            physical_length: (p2 - p1).length(),
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![p1, p2],
            physical_geometry: vec![p1, p2],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        }
    }

    fn validate_mesh(mesh_data: &NetworkMeshData, max_dist: f32) {
        fn validate_triangles(vertices: &[Vector3], max_dist: f32, label: &str) {
            assert_eq!(
                vertices.len() % 3,
                0,
                "{label}: vertex count must be a multiple of 3"
            );
            for (tri_idx, chunk) in vertices.chunks_exact(3).enumerate() {
                let (v0, v1, v2) = (chunk[0], chunk[1], chunk[2]);
                for (vi, v) in [(0, v0), (1, v1), (2, v2)] {
                    assert!(
                        v.x.is_finite() && v.y.is_finite() && v.z.is_finite(),
                        "{label} tri {tri_idx} vert {vi}: non-finite coordinate {v:?}"
                    );
                    let dist = (v.x * v.x + v.z * v.z).sqrt();
                    assert!(
                        dist <= max_dist,
                        "{label} tri {tri_idx} vert {vi}: vertex {v:?} is {dist:.1}m from origin, expected <= {max_dist}"
                    );
                }
                let d1 = v1 - v0;
                let d2 = v2 - v0;
                let cross = d1.x * d2.z - d1.z * d2.x;
                assert!(
                    cross >= -0.001,
                    "{label} tri {tri_idx}: back-facing winding detected (cross={cross:.4})"
                );
                let area = cross.abs() * 0.5;
                assert!(
                    area >= 0.001,
                    "{label} tri {tri_idx}: degenerate triangle (area={area:.6})"
                );
            }
        }

        validate_triangles(&mesh_data.sidewalk_vertices, max_dist, "sidewalk");
        validate_triangles(&mesh_data.road_vertices, max_dist, "road");
        validate_triangles(&mesh_data.marking_vertices, max_dist, "marking");
        validate_triangles(&mesh_data.concrete_vertices, max_dist, "concrete");
    }

    fn main_triangles(mesh_data: &NetworkMeshData, surface: VisibleSurface) -> Vec<[Vector3; 3]> {
        match surface {
            VisibleSurface::Road => mesh_data
                .road_vertices
                .chunks_exact(3)
                .map(|triangle| [triangle[0], triangle[1], triangle[2]])
                .collect(),
            VisibleSurface::Sidewalk => mesh_data
                .sidewalk_vertices
                .chunks_exact(3)
                .map(|triangle| [triangle[0], triangle[1], triangle[2]])
                .collect(),
            VisibleSurface::None => Vec::new(),
        }
    }

    fn triangle_contains_point_xz(triangle: [Vector3; 3], point: Vector2) -> bool {
        let point = Vector3::new(point.x, 0.0, point.y);
        let [a, b, c] = triangle;
        let ab = (b.x - a.x) * (point.z - a.z) - (b.z - a.z) * (point.x - a.x);
        let bc = (c.x - b.x) * (point.z - b.z) - (c.z - b.z) * (point.x - b.x);
        let ca = (a.x - c.x) * (point.z - c.z) - (a.z - c.z) * (point.x - c.x);
        let epsilon = 0.001;
        let has_neg = ab < -epsilon || bc < -epsilon || ca < -epsilon;
        let has_pos = ab > epsilon || bc > epsilon || ca > epsilon;
        !(has_neg && has_pos)
    }

    fn visible_surface_at_point(
        road_triangles: &[[Vector3; 3]],
        sidewalk_triangles: &[[Vector3; 3]],
        point: Vector2,
    ) -> VisibleSurface {
        if road_triangles
            .iter()
            .copied()
            .any(|triangle| triangle_contains_point_xz(triangle, point))
        {
            VisibleSurface::Road
        } else if sidewalk_triangles
            .iter()
            .copied()
            .any(|triangle| triangle_contains_point_xz(triangle, point))
        {
            VisibleSurface::Sidewalk
        } else {
            VisibleSurface::None
        }
    }

    fn visible_coverage_ratio(
        mesh_data: &NetworkMeshData,
        min: Vector2,
        max: Vector2,
        step: f32,
        target: VisibleSurface,
    ) -> f32 {
        let road_triangles = main_triangles(mesh_data, VisibleSurface::Road);
        let sidewalk_triangles = main_triangles(mesh_data, VisibleSurface::Sidewalk);
        let mut covered = 0usize;
        let mut total = 0usize;
        let mut z = min.y;
        while z <= max.y {
            let mut x = min.x;
            while x <= max.x {
                total += 1;
                if visible_surface_at_point(
                    &road_triangles,
                    &sidewalk_triangles,
                    Vector2::new(x, z),
                ) == target
                {
                    covered += 1;
                }
                x += step;
            }
            z += step;
        }

        if total == 0 {
            0.0
        } else {
            covered as f32 / total as f32
        }
    }

    fn generate_editor_mesh(
        roads: &[(&[Vector3], u8, u8)],
    ) -> (RegionGraph, NetworkMeshData, TerrainSystem) {
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        for (road, fwd, bkw) in roads {
            network.add_road(
                &mut graph,
                road.to_vec(),
                *fwd,
                *bkw,
                EdgeClass::Standard,
                &mut zoning,
                &mut allocator,
            );
        }

        let terrain = TerrainSystem::new(256, 256);
        let mesh_data = network.generate_mesh_data(&graph, &terrain);
        (graph, mesh_data, terrain)
    }

    #[test]
    fn test_angle_sweep_produces_valid_meshes() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let lane_system = crate::simulation::network::lanes::LaneSystem::new();

        for angle_deg in (10..180).step_by(20) {
            let mut graph = RegionGraph::new();
            let rad = (angle_deg as f32).to_radians();

            let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
            let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
            let n2 = graph.add_node(
                Vector3::new(rad.cos() * 20.0, 0.0, rad.sin() * 20.0),
                NodeType::Junction,
            );

            graph.add_edge(create_test_edge(
                n0,
                n1,
                Vector3::ZERO,
                Vector3::new(20.0, 0.0, 0.0),
                10.0,
            ));
            graph.add_edge(create_test_edge(
                n0,
                n2,
                Vector3::ZERO,
                Vector3::new(rad.cos() * 20.0, 0.0, rad.sin() * 20.0),
                10.0,
            ));

            graph.rebuild_adjacency_list();
            let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
            validate_mesh(&mesh_data, 80.0);
        }
    }

    #[test]
    fn test_editor_path_straight_road_keeps_terminal_caps() {
        let road = [Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)];
        let (_graph, mesh_data, _terrain) = generate_editor_mesh(&[(&road, 1, 1)]);
        validate_mesh(&mesh_data, 40.0);

        let left_asphalt = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-20.0, -3.3),
            Vector2::new(-18.5, 3.3),
            0.25,
            VisibleSurface::Road,
        );
        let right_asphalt = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(18.5, -3.3),
            Vector2::new(20.0, 3.3),
            0.25,
            VisibleSurface::Road,
        );
        let left_sidewalk = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-20.0, 3.7),
            Vector2::new(-18.5, 4.9),
            0.25,
            VisibleSurface::Sidewalk,
        );
        let right_sidewalk = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(18.5, 3.7),
            Vector2::new(20.0, 4.9),
            0.25,
            VisibleSurface::Sidewalk,
        );

        assert!(left_asphalt >= 0.8 && right_asphalt >= 0.8);
        assert!(left_sidewalk >= 0.45 && right_sidewalk >= 0.45);
    }

    #[test]
    fn test_car_only_surface_road_has_no_sidewalk_band() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);

        let mut edge = create_test_edge(
            n0,
            n1,
            Vector3::new(-20.0, 0.0, 0.0),
            Vector3::new(20.0, 0.0, 0.0),
            10.0,
        );
        edge.allowed_types = TransitFlags::CAR;
        graph.add_edge(edge);

        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 60.0);

        let center_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-5.0, -4.5),
            Vector2::new(5.0, 4.5),
            0.25,
            VisibleSurface::Road,
        );
        let outer_sidewalk = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-5.0, 5.5),
            Vector2::new(5.0, 6.5),
            0.25,
            VisibleSurface::Sidewalk,
        );

        assert!(center_road >= 0.95);
        assert!(outer_sidewalk <= 0.05);
    }

    #[test]
    fn test_walkway_connection_keeps_road_core_owned_by_asphalt() {
        let main = [Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)];
        let walkway = [Vector3::new(-12.0, 0.0, -12.0), Vector3::ZERO];
        let (_graph, mesh_data, _terrain) =
            generate_editor_mesh(&[(&main, 1, 1), (&walkway, 0, 0)]);
        validate_mesh(&mesh_data, 80.0);

        let road_core = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-2.5, -2.0),
            Vector2::new(2.5, 2.0),
            0.25,
            VisibleSurface::Road,
        );
        let sidewalk_throat = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-5.5, -5.5),
            Vector2::new(-3.0, -3.0),
            0.25,
            VisibleSurface::Sidewalk,
        );

        assert!(road_core >= 0.85);
        assert!(sidewalk_throat >= 0.15);
    }

    #[test]
    fn test_walkway_connection_keeps_sidewalk_shoulders_filled() {
        let main = [Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)];
        let walkway = [Vector3::new(0.0, 0.0, -12.0), Vector3::ZERO];
        let (_graph, mesh_data, _terrain) =
            generate_editor_mesh(&[(&main, 1, 1), (&walkway, 0, 0)]);
        validate_mesh(&mesh_data, 80.0);

        let left_shoulder = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-4.75, -4.75),
            Vector2::new(-1.75, -3.5),
            0.25,
            VisibleSurface::Sidewalk,
        );
        let right_shoulder = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(1.75, -4.75),
            Vector2::new(4.75, -3.5),
            0.25,
            VisibleSurface::Sidewalk,
        );
        let left_apron = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-3.0, -4.9),
            Vector2::new(-1.0, -3.6),
            0.2,
            VisibleSurface::Sidewalk,
        );
        let right_apron = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(1.0, -4.9),
            Vector2::new(3.0, -3.6),
            0.2,
            VisibleSurface::Sidewalk,
        );
        let opposite_shoulder = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-2.0, 3.6),
            Vector2::new(2.0, 4.9),
            0.2,
            VisibleSurface::Sidewalk,
        );
        let split_core = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-0.5, -1.0),
            Vector2::new(0.5, 1.0),
            0.1,
            VisibleSurface::Road,
        );

        assert!(left_shoulder >= 0.8);
        assert!(right_shoulder >= 0.8);
        assert!(left_apron >= 0.9);
        assert!(right_apron >= 0.9);
        assert!(opposite_shoulder >= 0.9);
        assert!(split_core >= 0.9);
    }

    #[test]
    fn test_walkway_connection_on_dense_baked_road_keeps_sidewalks_filled() {
        let mut main = Vec::new();
        for step in 0..=30 {
            let x = -30.0 + step as f32 * 2.0;
            let z = if x.abs() < 4.0 { x * 0.08 } else { x * 0.02 };
            main.push(Vector3::new(x, 0.0, z));
        }
        let walkway = [Vector3::new(0.0, 0.0, -12.0), Vector3::new(0.0, 0.0, -0.2)];
        let (graph, mesh_data, _terrain) = generate_editor_mesh(&[(&main, 1, 1), (&walkway, 0, 0)]);
        validate_mesh(&mesh_data, 90.0);
        let road_core = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-0.8, -1.0),
            Vector2::new(0.8, 1.0),
            0.1,
            VisibleSurface::Road,
        );
        assert!(road_core >= 0.85);

        let foot_edge = graph
            .edges
            .iter()
            .find(|edge| !edge.deleted && edge.primary_type == TransitType::Foot)
            .unwrap();
        let split_node = foot_edge.end_node;
        let split_pos = graph.nodes[split_node as usize].pos;
        assert!(split_pos.distance_to(Vector3::ZERO) <= 0.01);

        let connected_road_edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| {
                !edge.deleted
                    && edge.primary_type == TransitType::Road
                    && (edge.start_node == split_node || edge.end_node == split_node)
            })
            .collect();
        assert_eq!(connected_road_edges.len(), 2);
        for road_edge in connected_road_edges {
            let endpoint = if road_edge.start_node == split_node {
                road_edge.geometry[0]
            } else {
                *road_edge.geometry.last().unwrap()
            };
            assert!(endpoint.distance_to(split_pos) <= 0.01);
        }
    }

    #[test]
    fn test_dense_baked_road_without_walkway_stays_valid() {
        let mut main = Vec::new();
        for step in 0..=30 {
            let x = -30.0 + step as f32 * 2.0;
            let z = if x.abs() < 4.0 { x * 0.08 } else { x * 0.02 };
            main.push(Vector3::new(x, 0.0, z));
        }

        let (_graph, mesh_data, _terrain) = generate_editor_mesh(&[(&main, 1, 1)]);
        validate_mesh(&mesh_data, 90.0);
    }

    #[test]
    fn test_straight_split_stays_connected() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(-30.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(30.0, 0.0, 0.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::new(-30.0, 0.0, 0.0),
            Vector3::ZERO,
            10.0,
        ));
        graph.add_edge(create_test_edge(
            n1,
            n2,
            Vector3::ZERO,
            Vector3::new(30.0, 0.0, 0.0),
            10.0,
        ));

        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 80.0);

        let center_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-2.0, -4.5),
            Vector2::new(2.0, 4.5),
            0.25,
            VisibleSurface::Road,
        );
        assert!(center_road >= 0.95);
    }

    #[test]
    fn test_obtuse_bend_keeps_junction_core_asphalt_owned() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-14.0, 0.0, 14.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(24.0, 0.0, 0.0),
            10.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-14.0, 0.0, 14.0),
            10.0,
        ));

        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 80.0);

        let throat_is_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-1.5, 0.5),
            Vector2::new(2.0, 4.0),
            0.25,
            VisibleSurface::Road,
        );
        assert!(throat_is_road >= 0.8);
    }

    #[test]
    fn test_t_junction_center_is_visibly_road() {
        let vertical = [Vector3::new(0.0, 0.0, -20.0), Vector3::new(0.0, 0.0, 0.0)];
        let horizontal = [Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)];
        let (_graph, mesh_data, _terrain) =
            generate_editor_mesh(&[(&vertical, 1, 1), (&horizontal, 1, 1)]);
        validate_mesh(&mesh_data, 60.0);

        let center_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-2.0, -2.0),
            Vector2::new(2.0, 2.0),
            0.25,
            VisibleSurface::Road,
        );
        let outer_sidewalk = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(4.0, -6.0),
            Vector2::new(6.0, -4.5),
            0.25,
            VisibleSurface::Sidewalk,
        );

        assert!(center_road >= 0.9);
        assert!(outer_sidewalk >= 0.35);
    }

    #[test]
    fn test_four_way_center_is_visibly_road() {
        let north = [Vector3::new(0.0, 0.0, -20.0), Vector3::new(0.0, 0.0, 0.0)];
        let south = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 20.0)];
        let west = [Vector3::new(-20.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)];
        let east = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)];
        let (_graph, mesh_data, _terrain) =
            generate_editor_mesh(&[(&north, 1, 1), (&south, 1, 1), (&west, 1, 1), (&east, 1, 1)]);
        validate_mesh(&mesh_data, 60.0);

        let center_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-2.0, -2.0),
            Vector2::new(2.0, 2.0),
            0.25,
            VisibleSurface::Road,
        );
        assert!(center_road >= 0.95);
    }

    #[test]
    fn test_editor_path_diagonal_branch_keeps_core_road_and_exterior_sidewalk() {
        let main = [Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)];
        let branch = [Vector3::new(-18.0, 0.0, 18.0), Vector3::new(0.0, 0.0, 0.0)];
        let (_graph, mesh_data, _terrain) = generate_editor_mesh(&[(&main, 1, 1), (&branch, 1, 1)]);
        validate_mesh(&mesh_data, 80.0);

        let junction_core = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-2.5, -1.5),
            Vector2::new(2.5, 2.5),
            0.25,
            VisibleSurface::Road,
        );
        let outer_band = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-2.0, -4.8),
            Vector2::new(2.0, -3.8),
            0.25,
            VisibleSurface::Sidewalk,
        );

        assert!(junction_core >= 0.8);
        assert!(outer_band >= 0.25);
    }

    #[test]
    fn test_width_change_node_stays_connected() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(-25.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::new(-25.0, 0.0, 0.0),
            Vector3::ZERO,
            7.0,
        ));
        graph.add_edge(create_test_edge(
            n1,
            n2,
            Vector3::ZERO,
            Vector3::new(25.0, 0.0, 0.0),
            14.0,
        ));

        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 80.0);

        let center_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-2.0, -3.0),
            Vector2::new(2.0, 3.0),
            0.25,
            VisibleSurface::Road,
        );
        assert!(center_road >= 0.9);
    }

    #[test]
    fn test_mixed_width_t_junction_does_not_grow_round_bubble() {
        let main = [Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)];
        let branch = [Vector3::new(0.0, 0.0, -24.0), Vector3::ZERO];
        let (_graph, mesh_data, _terrain) = generate_editor_mesh(&[(&main, 1, 1), (&branch, 3, 3)]);
        validate_mesh(&mesh_data, 80.0);

        let junction_core = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-3.0, -3.0),
            Vector2::new(3.0, 3.0),
            0.25,
            VisibleSurface::Road,
        );
        let bubble_zone = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-4.0, 4.5),
            Vector2::new(4.0, 8.0),
            0.25,
            VisibleSurface::Road,
        );

        assert!(junction_core >= 0.75);
        assert!(bubble_zone <= 0.15);
    }

    #[test]
    fn test_two_way_diagonal_width_change_stays_regular_junction() {
        let main = [Vector3::new(-40.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)];
        let branch = [Vector3::new(-18.0, 0.0, 18.0), Vector3::ZERO];
        let (graph, mesh_data, _terrain) = generate_editor_mesh(&[(&main, 3, 3), (&branch, 1, 1)]);
        validate_mesh(&mesh_data, 90.0);

        let junction_node = graph
            .nodes
            .iter()
            .position(|node| node.pos.distance_to(Vector3::ZERO) < 0.01)
            .expect("expected a junction node at the branch connection");
        assert_eq!(graph.nodes[junction_node].node_type, NodeType::Junction);

        let branch_throat = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-4.0, 1.0),
            Vector2::new(-0.5, 5.5),
            0.25,
            VisibleSurface::Road,
        );
        assert!(branch_throat >= 0.7);
    }

    #[test]
    fn test_junction_crosswalk_markings_generated() {
        let north = [Vector3::new(0.0, 0.0, -20.0), Vector3::new(0.0, 0.0, 0.0)];
        let east = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)];
        let west = [Vector3::new(-20.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)];

        // T-junction with sidewalks
        let (_graph, mesh_data, _terrain) =
            generate_editor_mesh(&[(&north, 1, 1), (&east, 1, 1), (&west, 1, 1)]);

        validate_mesh(&mesh_data, 60.0);

        // Marking vertices should include both road dividers and crosswalks.
        // A T-junction has 3 road arms and at least 2 crosswalks.
        // Each zebra bar is 6 vertices. 10m crossing = ~12 bars = 72 vertices per crosswalk.
        assert!(
            mesh_data.marking_vertices.len() >= 150,
            "Expected significant marking vertices for crosswalks, got {}",
            mesh_data.marking_vertices.len()
        );
    }

    #[test]
    fn test_two_way_node_only_one_crosswalk() {
        let north = [Vector3::new(0.0, 0.0, -20.0), Vector3::new(0.0, 0.0, 0.0)];
        let east = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)];

        // Bend (2 arms)
        let (_graph, mesh_data, _terrain) = generate_editor_mesh(&[(&north, 1, 1), (&east, 1, 1)]);

        validate_mesh(&mesh_data, 40.0);

        println!("Marking vertices: {}", mesh_data.marking_vertices.len());
        // ...

        // A 2-way junction should have:
        // - Dash markings on the 2 road arms
        // - exactly ONE crosswalk (zebra stripes)
        // Current rendering produces ~336 vertices for 2×20m arms + 1 crosswalk.
        // A "no crosswalk" result (dash markings only) would be well below 200.
        // Two crosswalks would be above 550.
        assert!(
            mesh_data.marking_vertices.len() > 140 && mesh_data.marking_vertices.len() < 550,
            "Expected one crosswalk's worth of marking vertices (plus road dashings) for 2-way node, got {}",
            mesh_data.marking_vertices.len()
        );
    }
}
