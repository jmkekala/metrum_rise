#[cfg(test)]
mod tests {
    use crate::config;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::render::road::RoadRenderer;
    use crate::simulation::network::render::TransitRenderer;
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitType};
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::terrain::TerrainSystem;
    use godot::prelude::*;

    fn create_test_edge(n1: u32, n2: u32, p1: Vector3, p2: Vector3, width: f32, clip: f32) -> Edge {
        Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: 1 | 2,
            class: EdgeClass::Standard,
            width,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 13.0,
            base_cost: 0.0,
            physical_length: (p2 - p1).length(),
            current_congestion: 0.0,
            start_clip: clip,
            end_clip: 0.0,
            geometry: vec![p1, p2],
            physical_geometry: vec![p1, p2],
            zoning_left: true,
            zoning_right: true,
            deleted: false,
        }
    }

    /// Validate geometry correctness for a generated road mesh.
    ///
    /// Checks performed:
    /// 1. No NaN/Inf vertices.
    /// 2. Correct winding: In Godot Y-up, front-facing triangles viewed from above have
    ///    cross(d1, d2) < 0 in XZ (right-hand rule with Y-up gives normal pointing up when
    ///    the XZ cross is negative). Violated triangles appear as black back-faces.
    /// 3. Non-degenerate: no triangle with area < 0.001 m² (collapsed/needle triangles
    ///    produce visual artifacts at junctions).
    /// 4. Vertex bounds: no vertex further than `max_dist` from the origin, which should be
    ///    set to the furthest reasonable world point in the test graph. Catches miter
    ///    intersections that shoot to infinity on near-parallel edges.
    fn validate_mesh(
        _graph: &RegionGraph,
        mesh_data: &crate::simulation::network::render::NetworkMeshData,
        max_dist: f32,
    ) {
        assert!(!mesh_data.vertices.is_empty(), "Mesh should have vertices");
        assert_eq!(
            mesh_data.vertices.len() % 3,
            0,
            "Vertex count must be a multiple of 3"
        );

        for (tri_idx, chunk) in mesh_data.vertices.chunks_exact(3).enumerate() {
            let (v0, v1, v2) = (chunk[0], chunk[1], chunk[2]);

            // 1. No NaN / Inf
            for (vi, v) in [(0, v0), (1, v1), (2, v2)] {
                assert!(
                    v.x.is_finite() && v.y.is_finite() && v.z.is_finite(),
                    "tri {tri_idx} vert {vi}: non-finite coordinate {v:?}"
                );
            }

            // 2. Vertex distance bound (catches exploding miter intersections)
            for (vi, v) in [(0, v0), (1, v1), (2, v2)] {
                let dist = (v.x * v.x + v.z * v.z).sqrt();
                assert!(
                    dist <= max_dist,
                    "tri {tri_idx} vert {vi}: vertex {v:?} is {dist:.1}m from origin, expected <= {max_dist}"
                );
            }

            // 3. Winding order: cross(d1, d2) in XZ must be >= 0 (CW when viewed from above).
            //    Godot+Vulkan treats CW (positive XZ cross) as front-facing.
            //    A negative cross means CCW / back-facing, which renders black.
            let d1 = v1 - v0;
            let d2 = v2 - v0;
            let cross = d1.x * d2.z - d1.z * d2.x;
            assert!(
                cross >= -0.001, // small negative tolerance for numerical edge cases
                "tri {tri_idx}: CCW (back-facing) winding detected (cross={cross:.4}). \
                 Verts: {v0:?}, {v1:?}, {v2:?}"
            );

            // 4. Non-degenerate: area = |cross| / 2 must be > threshold
            let area = cross.abs() * 0.5;
            assert!(
                area >= 0.001,
                "tri {tri_idx}: degenerate triangle (area={area:.6} m²). \
                 Verts: {v0:?}, {v1:?}, {v2:?}"
            );
        }
    }

    fn sidewalk_vertices(
        mesh_data: &crate::simulation::network::render::NetworkMeshData,
    ) -> Vec<Vector3> {
        mesh_data
            .vertices
            .iter()
            .zip(mesh_data.colors.iter())
            .filter_map(|(vertex, color)| if color.a > 0.9 { Some(*vertex) } else { None })
            .collect()
    }

    fn asphalt_patch_vertices(
        mesh_data: &crate::simulation::network::render::NetworkMeshData,
    ) -> Vec<Vector3> {
        mesh_data
            .vertices
            .iter()
            .zip(mesh_data.colors.iter())
            .filter_map(|(vertex, color)| {
                if color.a > 0.1 && color.a < 0.9 {
                    Some(*vertex)
                } else {
                    None
                }
            })
            .collect()
    }

    fn asphalt_patch_triangles(
        mesh_data: &crate::simulation::network::render::NetworkMeshData,
    ) -> Vec<[Vector3; 3]> {
        mesh_data
            .vertices
            .chunks_exact(3)
            .zip(mesh_data.colors.chunks_exact(3))
            .filter_map(|(triangle, colors)| {
                if colors[0].a > 0.1 && colors[0].a < 0.9 {
                    Some([triangle[0], triangle[1], triangle[2]])
                } else {
                    None
                }
            })
            .collect()
    }

    fn sidewalk_triangles(
        mesh_data: &crate::simulation::network::render::NetworkMeshData,
    ) -> Vec<[Vector3; 3]> {
        mesh_data
            .vertices
            .chunks_exact(3)
            .zip(mesh_data.colors.chunks_exact(3))
            .filter_map(|(triangle, colors)| {
                if colors[0].a > 0.9 {
                    Some([triangle[0], triangle[1], triangle[2]])
                } else {
                    None
                }
            })
            .collect()
    }

    fn has_patch_vertex_near(vertices: &[Vector3], target: Vector2, tolerance: f32) -> bool {
        vertices.iter().any(|vertex| {
            let delta = Vector2::new(vertex.x - target.x, vertex.z - target.y);
            delta.length() <= tolerance
        })
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

    fn asphalt_surface_triangles(
        mesh_data: &crate::simulation::network::render::NetworkMeshData,
    ) -> Vec<[Vector3; 3]> {
        mesh_data
            .vertices
            .chunks_exact(3)
            .zip(mesh_data.colors.chunks_exact(3))
            .filter_map(|(triangle, colors)| {
                if colors[0].a <= 0.9 {
                    Some([triangle[0], triangle[1], triangle[2]])
                } else {
                    None
                }
            })
            .collect()
    }

    fn region_coverage_ratio(
        triangles: &[[Vector3; 3]],
        min: Vector2,
        max: Vector2,
        step: f32,
    ) -> f32 {
        let mut covered = 0usize;
        let mut total = 0usize;
        let mut z = min.y;
        while z <= max.y {
            let mut x = min.x;
            while x <= max.x {
                total += 1;
                let point = Vector2::new(x, z);
                if triangles
                    .iter()
                    .copied()
                    .any(|triangle| triangle_contains_point_xz(triangle, point))
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
        roads: &[&[Vector3]],
    ) -> (
        RegionGraph,
        crate::simulation::network::render::NetworkMeshData,
        TerrainSystem,
    ) {
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        for road in roads {
            network.add_road(
                &mut graph,
                road.to_vec(),
                1,
                1,
                false,
                false,
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
    #[ignore]
    fn debug_editor_path_diagonal_merge_dump() {
        let main = [Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)];
        let branch = [Vector3::new(-18.0, 0.0, 18.0), Vector3::new(0.0, 0.0, 0.0)];
        let (graph, mesh_data, _terrain) = generate_editor_mesh(&[&main, &branch]);

        for (idx, node) in graph.nodes.iter().enumerate() {
            eprintln!("node {idx}: pos={:?} type={:?}", node.pos, node.node_type);
        }

        for (idx, edge) in graph.edges.iter().enumerate() {
            eprintln!(
                "edge {idx}: deleted={} start={} end={} len={:.3} width={:.3} clips=({:.3}, {:.3}) geom={:?}",
                edge.deleted,
                edge.start_node,
                edge.end_node,
                edge.physical_length,
                edge.width,
                edge.start_clip,
                edge.end_clip,
                edge.physical_geometry
            );
        }

        let asphalt = asphalt_surface_triangles(&mesh_data);
        let sidewalk = sidewalk_triangles(&mesh_data);
        eprintln!(
            "coverage core: asphalt={:.3} sidewalk={:.3}",
            region_coverage_ratio(
                &asphalt,
                Vector2::new(-4.0, -3.5),
                Vector2::new(4.0, 3.5),
                0.25,
            ),
            region_coverage_ratio(
                &sidewalk,
                Vector2::new(-4.0, -3.5),
                Vector2::new(4.0, 3.5),
                0.25,
            )
        );
        eprintln!(
            "coverage throat: sidewalk={:.3}",
            region_coverage_ratio(
                &sidewalk,
                Vector2::new(-6.0, 1.0),
                Vector2::new(-1.0, 6.0),
                0.25,
            )
        );

        for (idx, triangle) in sidewalk.iter().enumerate() {
            let min_x = triangle.iter().map(|v| v.x).fold(f32::INFINITY, f32::min);
            let max_x = triangle
                .iter()
                .map(|v| v.x)
                .fold(f32::NEG_INFINITY, f32::max);
            let min_z = triangle.iter().map(|v| v.z).fold(f32::INFINITY, f32::min);
            let max_z = triangle
                .iter()
                .map(|v| v.z)
                .fold(f32::NEG_INFINITY, f32::max);
            let overlaps_core = max_x >= -4.0 && min_x <= 4.0 && max_z >= -3.5 && min_z <= 3.5;
            let overlaps_throat = max_x >= -6.0 && min_x <= -1.0 && max_z >= 1.0 && min_z <= 6.0;
            if overlaps_core || overlaps_throat {
                eprintln!("sidewalk tri {idx}: {:?}", triangle);
            }
        }
    }

    #[test]
    fn test_junction_angles_sweep() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);

        // Sweep 10°–170° in 20° steps. Vertices must stay within 3× the leg length.
        for angle_deg in (10..180).step_by(20) {
            let mut graph = RegionGraph::new();
            let rad = (angle_deg as f32).to_radians();

            let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
            let n1 = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
            let n2 = graph.add_node(
                Vector3::new(rad.cos() * 10.0, 0.0, rad.sin() * 10.0),
                NodeType::Junction,
            );

            graph.add_edge(create_test_edge(
                n0,
                n1,
                Vector3::ZERO,
                Vector3::new(10.0, 0.0, 0.0),
                10.0,
                0.0,
            ));
            graph.add_edge(create_test_edge(
                n0,
                n2,
                Vector3::ZERO,
                Vector3::new(rad.cos() * 10.0, 0.0, rad.sin() * 10.0),
                10.0,
                0.0,
            ));

            graph.rebuild_adjacency_list();
            let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
            validate_mesh(&graph, &mesh_data, 30.0);
        }
    }

    #[test]
    fn test_acute_angle_2way_bend() {
        // Image 3 (right): 2-way bend at a sharp angle. The road makes an elbow — no
        // extra legs, just two edges meeting at < 45°. This commonly produced back-facing
        // junction fill triangles because the CCW sort flipped for small angles.
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);

        for angle_deg in [10u32, 20, 30, 40] {
            let rad = (angle_deg as f32).to_radians();
            let mut graph = RegionGraph::new();
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
                0.0,
            ));
            graph.add_edge(create_test_edge(
                n0,
                n2,
                Vector3::ZERO,
                Vector3::new(rad.cos() * 20.0, 0.0, rad.sin() * 20.0),
                10.0,
                0.0,
            ));

            graph.rebuild_adjacency_list();
            let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
            validate_mesh(&graph, &mesh_data, 60.0);
        }
    }

    #[test]
    fn test_standard_road_emits_sidewalk_geometry() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::new(-20.0, 0.0, 0.0),
            Vector3::new(20.0, 0.0, 0.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 40.0);

        let sidewalk = sidewalk_vertices(&mesh_data);
        assert!(
            !sidewalk.is_empty(),
            "standard roads should emit perimeter geometry"
        );

        let max_abs_z = sidewalk.iter().map(|v| v.z.abs()).fold(0.0f32, f32::max);
        let expected = 5.0 + config::SIDEWALK_WIDTH;
        assert!(
            (max_abs_z - expected).abs() < 0.2,
            "expected sidewalk strip at {:.2}m, found {:.2}m",
            expected,
            max_abs_z
        );
        assert!(
            sidewalk.iter().any(|v| v.x <= -19.5) && sidewalk.iter().any(|v| v.x >= 19.5),
            "degree-1 road endpoints should keep their sidewalk caps instead of being trimmed back like a junction"
        );
        assert!(
            mesh_data.vertices.iter().any(|v| v.x <= -19.5)
                && mesh_data.vertices.iter().any(|v| v.x >= 19.5),
            "degree-1 road endpoints should keep asphalt all the way to the terminal node"
        );
        assert!(
            asphalt_patch_vertices(&mesh_data).is_empty(),
            "a single road should not emit any junction asphalt patch"
        );
    }

    #[test]
    fn test_editor_path_straight_road_keeps_terminal_caps() {
        let road = [Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)];
        let (graph, mesh_data, _terrain) = generate_editor_mesh(&[&road]);
        validate_mesh(&graph, &mesh_data, 40.0);

        let asphalt = asphalt_surface_triangles(&mesh_data);
        let sidewalk = sidewalk_triangles(&mesh_data);
        let left_end = region_coverage_ratio(
            &asphalt,
            Vector2::new(-20.0, -4.0),
            Vector2::new(-18.5, 4.0),
            0.25,
        );
        let right_end = region_coverage_ratio(
            &asphalt,
            Vector2::new(18.5, -4.0),
            Vector2::new(20.0, 4.0),
            0.25,
        );
        let left_sidewalk = region_coverage_ratio(
            &sidewalk,
            Vector2::new(-20.0, 5.25),
            Vector2::new(-18.5, 6.75),
            0.25,
        );
        let right_sidewalk = region_coverage_ratio(
            &sidewalk,
            Vector2::new(18.5, 5.25),
            Vector2::new(20.0, 6.75),
            0.25,
        );

        assert!(
            left_end >= 0.85 && right_end >= 0.85,
            "editor-path straight roads must keep asphalt nearly all the way to both degree-1 endpoints; left={left_end:.3}, right={right_end:.3}"
        );
        assert!(
            left_sidewalk >= 0.6 && right_sidewalk >= 0.6,
            "editor-path straight roads must keep terminal sidewalk caps; left={left_sidewalk:.3}, right={right_sidewalk:.3}"
        );
        assert!(
            asphalt_patch_vertices(&mesh_data).is_empty(),
            "a single editor-path road should not create a node-owned junction patch"
        );
    }

    #[test]
    fn test_split_straight_road_is_coplanar() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);
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
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n1,
            n2,
            Vector3::ZERO,
            Vector3::new(30.0, 0.0, 0.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 80.0);

        let min_y = mesh_data
            .vertices
            .iter()
            .map(|v| v.y)
            .fold(f32::INFINITY, f32::min);
        let max_y = mesh_data
            .vertices
            .iter()
            .map(|v| v.y)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (max_y - min_y) <= 0.0001,
            "connected straight road segments should stay coplanar; found y range {:.6}",
            max_y - min_y
        );
        assert!(
            asphalt_patch_vertices(&mesh_data).is_empty(),
            "straight pass-through splits should not emit a junction asphalt island"
        );
    }

    #[test]
    fn test_acute_angle_3way_y_junction() {
        // Image 3 (left): 3-way Y-junction where two legs form a very shallow V. The third
        // leg exits the back. Acute inner angles here caused self-intersecting junction fills.
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);

        for angle_deg in [10u32, 20, 30] {
            let rad = (angle_deg as f32).to_radians();
            let mut graph = RegionGraph::new();
            let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
            let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
            let n2 = graph.add_node(
                Vector3::new(rad.cos() * 20.0, 0.0, rad.sin() * 20.0),
                NodeType::Junction,
            );
            let n3 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction); // back leg

            graph.add_edge(create_test_edge(
                n0,
                n1,
                Vector3::ZERO,
                Vector3::new(20.0, 0.0, 0.0),
                10.0,
                0.0,
            ));
            graph.add_edge(create_test_edge(
                n0,
                n2,
                Vector3::ZERO,
                Vector3::new(rad.cos() * 20.0, 0.0, rad.sin() * 20.0),
                10.0,
                0.0,
            ));
            graph.add_edge(create_test_edge(
                n0,
                n3,
                Vector3::ZERO,
                Vector3::new(-20.0, 0.0, 0.0),
                10.0,
                0.0,
            ));

            graph.rebuild_adjacency_list();
            let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
            validate_mesh(&graph, &mesh_data, 60.0);
        }
    }

    #[test]
    fn test_unclipped_two_way_bend_emits_local_patch() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);
        let mut graph = RegionGraph::new();
        let bend_rad = (35.0f32).to_radians();

        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(
            Vector3::new(bend_rad.cos() * 20.0, 0.0, bend_rad.sin() * 20.0),
            NodeType::Junction,
        );

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(20.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(bend_rad.cos() * 20.0, 0.0, bend_rad.sin() * 20.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 60.0);

        assert!(
            mesh_data
                .colors
                .iter()
                .any(|color| color.a > 0.1 && color.a < 0.9),
            "multi-road nodes should emit an asphalt patch in the area-based renderer"
        );
        assert!(
            !sidewalk_vertices(&mesh_data).is_empty(),
            "2-way bends should still emit perimeter geometry"
        );
    }

    #[test]
    fn test_two_way_bend_patch_does_not_collapse_to_node() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);
        let mut graph = RegionGraph::new();
        let bend_rad = (60.0f32).to_radians();

        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(
            Vector3::new(bend_rad.cos() * 20.0, 0.0, bend_rad.sin() * 20.0),
            NodeType::Junction,
        );

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(20.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(bend_rad.cos() * 20.0, 0.0, bend_rad.sin() * 20.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 60.0);

        let patch_vertices = asphalt_patch_vertices(&mesh_data);
        assert!(
            !patch_vertices.is_empty(),
            "angled 2-way bends should emit an asphalt patch"
        );
        assert!(
            patch_vertices
                .iter()
                .all(|vertex| Vector2::new(vertex.x, vertex.z).length() >= 4.0),
            "angled 2-way bends should stitch between trimmed caps instead of collapsing back to the node center"
        );
    }

    #[test]
    fn test_shallow_merge_emits_area_patch() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(-20.8, 0.0, 12.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(24.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-24.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n3,
            Vector3::ZERO,
            Vector3::new(-20.8, 0.0, 12.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 80.0);

        assert!(
            !sidewalk_vertices(&mesh_data).is_empty(),
            "shallow merges should still emit sidewalk geometry on exposed sides"
        );
        assert!(
            mesh_data
                .colors
                .iter()
                .any(|color| color.a > 0.1 && color.a < 0.9),
            "shallow merges should be resolved by a local asphalt patch, not edge-only stitching"
        );

        let patch_triangles = asphalt_patch_triangles(&mesh_data);
        assert!(
            patch_triangles
                .iter()
                .copied()
                .any(|triangle| triangle_contains_point_xz(triangle, Vector2::ZERO)),
            "shallow merge node patches should cover the junction center instead of leaving a hole between trimmed ribbons"
        );

        let sidewalk_triangles = sidewalk_triangles(&mesh_data);
        assert!(
            sidewalk_triangles
                .iter()
                .copied()
                .any(|triangle| triangle_contains_point_xz(triangle, Vector2::new(4.0, 6.0))),
            "shallow merges should keep a continuous sidewalk band on the exterior of the junction"
        );
        assert!(
            sidewalk_triangles
                .iter()
                .copied()
                .any(|triangle| triangle_contains_point_xz(triangle, Vector2::new(0.0, -6.0))),
            "shallow merges should keep the mainline sidewalk under the junction instead of leaving a gap"
        );
        assert!(
            sidewalk_triangles
                .iter()
                .copied()
                .all(|triangle| !triangle_contains_point_xz(triangle, Vector2::new(-3.0, 1.5))),
            "shallow merges should not leave sidewalk triangles inside the asphalt throat"
        );
    }

    #[test]
    fn test_diagonal_merge_keeps_node_sidewalk_band() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(-18.0, 0.0, 16.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(24.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-24.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n3,
            Vector3::ZERO,
            Vector3::new(-18.0, 0.0, 16.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 80.0);

        let sidewalk_triangles = sidewalk_triangles(&mesh_data);
        assert!(
            sidewalk_triangles
                .iter()
                .copied()
                .any(|triangle| triangle_contains_point_xz(triangle, Vector2::new(4.0, 6.0))),
            "diagonal merges should keep the node-owned sidewalk band on the exposed outer side of the junction"
        );
        assert!(
            sidewalk_triangles
                .iter()
                .copied()
                .all(|triangle| !triangle_contains_point_xz(triangle, Vector2::new(-3.0, 2.0))),
            "diagonal branch sidewalks should be cut before the node asphalt throat instead of extending under the junction"
        );
    }

    #[test]
    fn test_editor_path_diagonal_merge_has_no_sidewalk_in_junction_core() {
        let main = [Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)];
        let branch = [Vector3::new(-18.0, 0.0, 18.0), Vector3::new(0.0, 0.0, 0.0)];
        let (graph, mesh_data, _terrain) = generate_editor_mesh(&[&main, &branch]);
        validate_mesh(&graph, &mesh_data, 80.0);

        let asphalt = asphalt_surface_triangles(&mesh_data);
        let sidewalk = sidewalk_triangles(&mesh_data);
        let core_min = Vector2::new(-4.0, -3.5);
        let core_max = Vector2::new(4.0, 3.5);
        let sidewalk_core = region_coverage_ratio(&sidewalk, core_min, core_max, 0.25);
        let asphalt_core = region_coverage_ratio(&asphalt, core_min, core_max, 0.25);

        assert!(
            asphalt_core >= 0.95,
            "editor-path diagonal merges should keep the junction core asphalt-owned; asphalt coverage={asphalt_core:.3}"
        );
        assert!(
            sidewalk_core <= 0.02,
            "editor-path diagonal merges should not leak sidewalk ownership into the junction core; sidewalk coverage={sidewalk_core:.3}"
        );
    }

    #[test]
    fn test_editor_path_diagonal_merge_cuts_branch_sidewalk_before_throat() {
        let main = [Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)];
        let branch = [Vector3::new(-18.0, 0.0, 18.0), Vector3::new(0.0, 0.0, 0.0)];
        let (graph, mesh_data, _terrain) = generate_editor_mesh(&[&main, &branch]);
        validate_mesh(&graph, &mesh_data, 80.0);

        let sidewalk = sidewalk_triangles(&mesh_data);
        let throat_min = Vector2::new(-6.0, 1.0);
        let throat_max = Vector2::new(-1.0, 6.0);
        let sidewalk_throat = region_coverage_ratio(&sidewalk, throat_min, throat_max, 0.25);

        assert!(
            sidewalk_throat <= 0.05,
            "editor-path diagonal branch sidewalks should be cut before the junction throat; sidewalk coverage={sidewalk_throat:.3}"
        );
    }

    #[test]
    fn test_editor_path_diagonal_merge_keeps_outer_node_sidewalk_band() {
        let main = [Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)];
        let branch = [Vector3::new(-18.0, 0.0, 18.0), Vector3::new(0.0, 0.0, 0.0)];
        let (graph, mesh_data, _terrain) = generate_editor_mesh(&[&main, &branch]);
        validate_mesh(&graph, &mesh_data, 80.0);

        let sidewalk = sidewalk_triangles(&mesh_data);
        let outer_min = Vector2::new(2.0, 4.0);
        let outer_max = Vector2::new(7.0, 8.0);
        let outer_band = region_coverage_ratio(&sidewalk, outer_min, outer_max, 0.25);

        assert!(
            outer_band >= 0.15,
            "editor-path diagonal merges should keep a node-owned sidewalk band on the exposed outer junction side; sidewalk coverage={outer_band:.3}"
        );
    }

    #[test]
    fn test_clustered_multi_arm_patch_covers_junction_center() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(-16.0, 0.0, 18.0), NodeType::Junction);
        let n4 = graph.add_node(Vector3::new(-16.0, 0.0, -18.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(24.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-24.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n3,
            Vector3::ZERO,
            Vector3::new(-16.0, 0.0, 18.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n4,
            Vector3::ZERO,
            Vector3::new(-16.0, 0.0, -18.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 80.0);

        let patch_triangles = asphalt_patch_triangles(&mesh_data);
        assert!(
            patch_triangles
                .iter()
                .copied()
                .any(|triangle| triangle_contains_point_xz(triangle, Vector2::ZERO)),
            "clustered multi-arm junctions should keep a continuous asphalt patch at the node center"
        );
    }

    #[test]
    fn test_four_way_patch_stays_out_of_diagonal_corners() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
        let n4 = graph.add_node(Vector3::new(0.0, 0.0, -20.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(20.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-20.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n3,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, 20.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n4,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, -20.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 80.0);

        let patch_vertices = asphalt_patch_vertices(&mesh_data);
        assert!(
            !patch_vertices.is_empty(),
            "four-way nodes should still emit an asphalt patch"
        );

        let road_half = 5.0 + 0.35;
        assert!(
            patch_vertices
                .iter()
                .all(|vertex| vertex.x.abs() <= road_half || vertex.z.abs() <= road_half),
            "four-way asphalt patches should stay in the road cross, not inflate into diagonal lobes"
        );
    }

    #[test]
    fn test_four_way_patch_hits_trimmed_cap_endpoints() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
        let n4 = graph.add_node(Vector3::new(0.0, 0.0, -20.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(20.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-20.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n3,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, 20.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n4,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, -20.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        let trim = graph.edges[0].start_clip;
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 80.0);

        let patch_vertices = asphalt_patch_vertices(&mesh_data);
        let tolerance = 0.2;
        let expected = [
            Vector2::new(trim, -5.0),
            Vector2::new(trim, 5.0),
            Vector2::new(5.0, trim),
            Vector2::new(-5.0, trim),
            Vector2::new(-trim, 5.0),
            Vector2::new(-trim, -5.0),
            Vector2::new(-5.0, -trim),
            Vector2::new(5.0, -trim),
        ];

        for point in expected {
            assert!(
                has_patch_vertex_near(&patch_vertices, point, tolerance),
                "expected junction patch to include trimmed cap point near ({:.2}, {:.2})",
                point.x,
                point.y
            );
        }
    }

    #[test]
    fn test_complex_junctions() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);

        // 1. T-Junction: wide main road + narrow side street (4:2 width ratio)
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(20.0, 0.0, 0.0),
            16.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-20.0, 0.0, 0.0),
            16.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n3,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, 20.0),
            7.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 60.0);

        // 2. Sloped 4-way junction
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(10.0, 2.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-10.0, -2.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);
        let n4 = graph.add_node(Vector3::new(0.0, 0.0, -10.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(10.0, 2.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-10.0, -2.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n3,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, 10.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n4,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, -10.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 30.0);

        // 3. Highway off-ramp (25° acute exit, highway 16m wide vs 6m ramp)
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let ramp_rad = (25.0f32).to_radians();
        let n3 = graph.add_node(
            Vector3::new(ramp_rad.cos() * 20.0, 0.0, ramp_rad.sin() * 20.0),
            NodeType::Junction,
        );

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(20.0, 0.0, 0.0),
            16.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-20.0, 0.0, 0.0),
            16.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n3,
            Vector3::ZERO,
            Vector3::new(ramp_rad.cos() * 20.0, 0.0, ramp_rad.sin() * 20.0),
            6.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 60.0);

        // 4. Clipped T-Junction (verify gap filling with realistic clip values)
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(20.0, 0.0, 0.0),
            10.0,
            3.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-20.0, 0.0, 0.0),
            10.0,
            3.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n3,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, 20.0),
            10.0,
            3.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 60.0);
    }

    #[test]
    fn test_t_junction_emits_sidewalk_band() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(100, 100);
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::ZERO,
            Vector3::new(20.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n2,
            Vector3::ZERO,
            Vector3::new(-20.0, 0.0, 0.0),
            10.0,
            0.0,
        ));
        graph.add_edge(create_test_edge(
            n0,
            n3,
            Vector3::ZERO,
            Vector3::new(0.0, 0.0, 20.0),
            10.0,
            0.0,
        ));

        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 60.0);

        assert!(
            !sidewalk_vertices(&mesh_data).is_empty(),
            "multi-way intersections should emit perimeter band geometry"
        );
        assert!(
            mesh_data
                .colors
                .iter()
                .any(|color| color.a > 0.1 && color.a < 0.9),
            "multi-way intersections should still emit asphalt junction fill"
        );
    }
}
