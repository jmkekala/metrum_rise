#[cfg(test)]
mod tests {
    use crate::simulation::network::render::road::RoadRenderer;
    use crate::simulation::network::render::TransitRenderer;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::types::{TransitType, NodeType, EdgeClass};
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::core::config::MapConfig;
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
        assert_eq!(mesh_data.vertices.len() % 3, 0, "Vertex count must be a multiple of 3");

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

            // 3. Winding order: cross(d1, d2) in XZ must be <= 0 (CCW when viewed from above
            //    in Godot Y-up). A positive cross means a CW / back-facing triangle that
            //    renders black.
            let d1 = v1 - v0;
            let d2 = v2 - v0;
            let cross = d1.x * d2.z - d1.z * d2.x;
            assert!(
                cross <= 0.001, // small positive tolerance for numerical edge cases
                "tri {tri_idx}: CW (back-facing) winding detected (cross={cross:.4}). \
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
            let n2 = graph.add_node(Vector3::new(rad.cos() * 10.0, 0.0, rad.sin() * 10.0), NodeType::Junction);

            graph.add_edge(create_test_edge(n0, n1, Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0), 10.0, 0.0));
            graph.add_edge(create_test_edge(n0, n2, Vector3::ZERO, Vector3::new(rad.cos() * 10.0, 0.0, rad.sin() * 10.0), 10.0, 0.0));

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
            let n2 = graph.add_node(Vector3::new(rad.cos() * 20.0, 0.0, rad.sin() * 20.0), NodeType::Junction);

            graph.add_edge(create_test_edge(n0, n1, Vector3::ZERO, Vector3::new(20.0, 0.0, 0.0), 10.0, 0.0));
            graph.add_edge(create_test_edge(n0, n2, Vector3::ZERO, Vector3::new(rad.cos() * 20.0, 0.0, rad.sin() * 20.0), 10.0, 0.0));

            graph.rebuild_adjacency_list();
            let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
            validate_mesh(&graph, &mesh_data, 60.0);
        }
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
            let n2 = graph.add_node(Vector3::new(rad.cos() * 20.0, 0.0, rad.sin() * 20.0), NodeType::Junction);
            let n3 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction); // back leg

            graph.add_edge(create_test_edge(n0, n1, Vector3::ZERO, Vector3::new(20.0, 0.0, 0.0), 10.0, 0.0));
            graph.add_edge(create_test_edge(n0, n2, Vector3::ZERO, Vector3::new(rad.cos() * 20.0, 0.0, rad.sin() * 20.0), 10.0, 0.0));
            graph.add_edge(create_test_edge(n0, n3, Vector3::ZERO, Vector3::new(-20.0, 0.0, 0.0), 10.0, 0.0));

            graph.rebuild_adjacency_list();
            let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
            validate_mesh(&graph, &mesh_data, 60.0);
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

        graph.add_edge(create_test_edge(n0, n1, Vector3::ZERO, Vector3::new(20.0, 0.0, 0.0), 16.0, 0.0));
        graph.add_edge(create_test_edge(n0, n2, Vector3::ZERO, Vector3::new(-20.0, 0.0, 0.0), 16.0, 0.0));
        graph.add_edge(create_test_edge(n0, n3, Vector3::ZERO, Vector3::new(0.0, 0.0, 20.0), 7.0, 0.0));

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

        graph.add_edge(create_test_edge(n0, n1, Vector3::ZERO, Vector3::new(10.0, 2.0, 0.0), 10.0, 0.0));
        graph.add_edge(create_test_edge(n0, n2, Vector3::ZERO, Vector3::new(-10.0, -2.0, 0.0), 10.0, 0.0));
        graph.add_edge(create_test_edge(n0, n3, Vector3::ZERO, Vector3::new(0.0, 0.0, 10.0), 10.0, 0.0));
        graph.add_edge(create_test_edge(n0, n4, Vector3::ZERO, Vector3::new(0.0, 0.0, -10.0), 10.0, 0.0));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 30.0);

        // 3. Highway off-ramp (25° acute exit, highway 16m wide vs 6m ramp)
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let ramp_rad = (25.0f32).to_radians();
        let n3 = graph.add_node(Vector3::new(ramp_rad.cos() * 20.0, 0.0, ramp_rad.sin() * 20.0), NodeType::Junction);

        graph.add_edge(create_test_edge(n0, n1, Vector3::ZERO, Vector3::new(20.0, 0.0, 0.0), 16.0, 0.0));
        graph.add_edge(create_test_edge(n0, n2, Vector3::ZERO, Vector3::new(-20.0, 0.0, 0.0), 16.0, 0.0));
        graph.add_edge(create_test_edge(n0, n3, Vector3::ZERO, Vector3::new(ramp_rad.cos() * 20.0, 0.0, ramp_rad.sin() * 20.0), 6.0, 0.0));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 60.0);

        // 4. Clipped T-Junction (verify gap filling with realistic clip values)
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);

        graph.add_edge(create_test_edge(n0, n1, Vector3::ZERO, Vector3::new(20.0, 0.0, 0.0), 10.0, 3.0));
        graph.add_edge(create_test_edge(n0, n2, Vector3::ZERO, Vector3::new(-20.0, 0.0, 0.0), 10.0, 3.0));
        graph.add_edge(create_test_edge(n0, n3, Vector3::ZERO, Vector3::new(0.0, 0.0, 20.0), 10.0, 3.0));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &terrain);
        validate_mesh(&graph, &mesh_data, 60.0);
    }
}
