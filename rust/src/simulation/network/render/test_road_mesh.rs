//! Black-box renderer tests for the graph-dilation road mesher.
//!
//! These tests validate the visible road/sidewalk contract of the replacement renderer rather
//! than any specific internal junction contour implementation.

#[cfg(test)]
mod tests {
    use crate::config;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::render::road::RoadRenderer;
    use crate::simulation::network::render::{NetworkMeshData, TransitRenderer};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use crate::simulation::terrain::TerrainSystem;
    use godot::prelude::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum VisibleSurface {
        None,
        Road,
        Curb,
        Sidewalk,
        CurbOrSidewalk,
    }

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct RenderVertexKey {
        x_bits: u32,
        y_bits: u32,
        z_bits: u32,
    }

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct RenderEdgeKey {
        start: RenderVertexKey,
        end: RenderVertexKey,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CurbFaceRegion {
        LeftSpan,
        RightSpan,
        StartTerminalCap,
        EndTerminalCap,
    }

    impl RenderVertexKey {
        fn from_point(point: Vector3) -> Self {
            Self {
                x_bits: canonical_f32_bits(point.x),
                y_bits: canonical_f32_bits(point.y),
                z_bits: canonical_f32_bits(point.z),
            }
        }
    }

    impl RenderEdgeKey {
        fn new(start: Vector3, end: Vector3) -> Self {
            let start = RenderVertexKey::from_point(start);
            let end = RenderVertexKey::from_point(end);
            if end < start {
                Self {
                    start: end,
                    end: start,
                }
            } else {
                Self { start, end }
            }
        }
    }

    fn canonical_f32_bits(value: f32) -> u32 {
        if value == 0.0 {
            0.0_f32.to_bits()
        } else {
            value.to_bits()
        }
    }

    fn create_test_edge(n1: u32, n2: u32, p1: Vector3, p2: Vector3, width: f32) -> Edge {
        create_surface_edge(
            n1,
            n2,
            &[p1, p2],
            width,
            TransitType::Road,
            EdgeClass::Standard,
            TransitFlags::CAR | TransitFlags::FOOT,
            ((width / config::LANE_WIDTH).round() as u8).max(1),
            0,
        )
    }

    fn create_surface_edge(
        n1: u32,
        n2: u32,
        points: &[Vector3],
        width: f32,
        primary_type: TransitType,
        class: EdgeClass,
        allowed_types: u8,
        fwd_lanes: u8,
        bkw_lanes: u8,
    ) -> Edge {
        let physical_length = points
            .windows(2)
            .map(|segment| segment[0].distance_to(segment[1]))
            .sum();
        Edge {
            start_node: n1,
            end_node: n2,
            primary_type,
            allowed_types,
            class,
            width,
            fwd_lanes,
            bkw_lanes,
            speed_limit: 13.0,
            base_cost: 0.0,
            physical_length,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: points.to_vec(),
            physical_geometry: points.to_vec(),
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
                let projected_cross = d1.x * d2.z - d1.z * d2.x;
                if projected_cross.abs() >= 0.001 {
                    assert!(
                        projected_cross >= -0.001,
                        "{label} tri {tri_idx}: back-facing winding detected (cross={projected_cross:.4})"
                    );
                }
                let area = d1.cross(d2).length() * 0.5;
                assert!(
                    area >= 1.0e-6,
                    "{label} tri {tri_idx}: degenerate triangle (area={area:.6})"
                );
            }
        }

        validate_triangles(&mesh_data.sidewalk_vertices, max_dist, "sidewalk");
        validate_triangles(&mesh_data.curb_vertices, max_dist, "curb");
        validate_triangles(&mesh_data.curb_vertical_vertices, max_dist, "curb_vertical");
        validate_triangles(&mesh_data.road_vertices, max_dist, "road");
        validate_triangles(&mesh_data.marking_vertices, max_dist, "marking");
        validate_triangles(&mesh_data.concrete_vertices, max_dist, "concrete");
        validate_triangles(&mesh_data.earthwork_vertices, max_dist, "earthwork");
    }

    fn main_triangles(mesh_data: &NetworkMeshData, surface: VisibleSurface) -> Vec<[Vector3; 3]> {
        let triangles = match surface {
            VisibleSurface::Road => triangles_from_vertices(&mesh_data.road_vertices),
            VisibleSurface::Curb => triangles_from_vertices(&mesh_data.curb_vertices),
            VisibleSurface::Sidewalk => triangles_from_vertices(&mesh_data.sidewalk_vertices),
            VisibleSurface::CurbOrSidewalk => Vec::new(),
            VisibleSurface::None => Vec::new(),
        };
        triangles
            .into_iter()
            .filter(|triangle| triangle_projected_double_area(*triangle).abs() >= 0.001)
            .collect()
    }

    fn triangles_from_vertices(vertices: &[Vector3]) -> Vec<[Vector3; 3]> {
        vertices
            .chunks_exact(3)
            .map(|triangle| [triangle[0], triangle[1], triangle[2]])
            .collect()
    }

    fn triangle_projected_double_area(triangle: [Vector3; 3]) -> f32 {
        let [a, b, c] = triangle;
        (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)
    }

    fn triangle_y_delta(triangle: [Vector3; 3]) -> f32 {
        let [a, b, c] = triangle;
        let min_y = a.y.min(b.y).min(c.y);
        let max_y = a.y.max(b.y).max(c.y);
        max_y - min_y
    }

    fn triangle_edges(triangle: [Vector3; 3]) -> [[Vector3; 2]; 3] {
        [
            [triangle[0], triangle[1]],
            [triangle[1], triangle[2]],
            [triangle[2], triangle[0]],
        ]
    }

    fn top_surface_boundary_edges(vertices: &[Vector3]) -> BTreeSet<RenderEdgeKey> {
        let mut edge_counts = BTreeMap::new();
        for triangle in triangles_from_vertices(vertices) {
            if triangle_projected_double_area(triangle).abs() < 0.001 {
                continue;
            }
            for [start, end] in triangle_edges(triangle) {
                *edge_counts
                    .entry(RenderEdgeKey::new(start, end))
                    .or_insert(0usize) += 1;
            }
        }

        edge_counts
            .into_iter()
            .filter_map(|(edge, count)| (count == 1).then_some(edge))
            .collect()
    }

    fn top_surface_boundary_segments(vertices: &[Vector3]) -> Vec<[Vector3; 2]> {
        let mut edge_counts = BTreeMap::new();
        let mut edge_segments = BTreeMap::new();
        for triangle in triangles_from_vertices(vertices) {
            if triangle_projected_double_area(triangle).abs() < 0.001 {
                continue;
            }
            for [start, end] in triangle_edges(triangle) {
                let edge = RenderEdgeKey::new(start, end);
                *edge_counts.entry(edge).or_insert(0usize) += 1;
                edge_segments.entry(edge).or_insert([start, end]);
            }
        }

        edge_counts
            .into_iter()
            .filter_map(|(edge, count)| (count == 1).then(|| edge_segments[&edge]))
            .collect()
    }

    fn nearby_boundary_edges_debug(target: [Vector3; 2], segments: &[[Vector3; 2]]) -> String {
        let target_midpoint = (target[0] + target[1]) / 2.0;
        let mut nearby = segments
            .iter()
            .copied()
            .map(|segment| {
                let midpoint = (segment[0] + segment[1]) / 2.0;
                (
                    (midpoint - target_midpoint).length_squared(),
                    segment[0],
                    segment[1],
                )
            })
            .collect::<Vec<_>>();
        nearby.sort_by(|a, b| a.0.total_cmp(&b.0));
        nearby
            .into_iter()
            .take(4)
            .map(|(_, start, end)| format!("[{start:?}, {end:?}]"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn vertical_curb_face_horizontal_edges(vertices: &[Vector3]) -> Vec<[Vector3; 2]> {
        let mut edges = Vec::new();
        for triangle in triangles_from_vertices(vertices) {
            if triangle_projected_double_area(triangle).abs() >= 0.001
                || triangle_y_delta(triangle) < 0.05
            {
                continue;
            }
            for [start, end] in triangle_edges(triangle) {
                let xz_length = Vector2::new(end.x - start.x, end.z - start.z).length();
                if xz_length >= 0.001 && (end.y - start.y).abs() <= 0.001 {
                    edges.push([start, end]);
                }
            }
        }
        edges
    }

    fn curb_face_region(edge: [Vector3; 2]) -> Option<CurbFaceRegion> {
        let midpoint = (edge[0] + edge[1]) / 2.0;
        if midpoint.x.abs() < 12.0 && (midpoint.z + 5.0).abs() <= 0.2 {
            Some(CurbFaceRegion::LeftSpan)
        } else if midpoint.x.abs() < 12.0 && (midpoint.z - 5.0).abs() <= 0.2 {
            Some(CurbFaceRegion::RightSpan)
        } else if (midpoint.x + 20.0).abs() <= 0.2 && midpoint.z.abs() <= 4.9 {
            Some(CurbFaceRegion::StartTerminalCap)
        } else if (midpoint.x - 20.0).abs() <= 0.2 && midpoint.z.abs() <= 4.9 {
            Some(CurbFaceRegion::EndTerminalCap)
        } else {
            None
        }
    }

    fn triangle_normal(triangle: [Vector3; 3]) -> Vector3 {
        let [a, b, c] = triangle;
        (b - a).cross(c - a)
    }

    fn godot_cull_back_visible_direction(triangle: [Vector3; 3]) -> Vector3 {
        -triangle_normal(triangle)
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
        curb_triangles: &[[Vector3; 3]],
        sidewalk_triangles: &[[Vector3; 3]],
        point: Vector2,
    ) -> VisibleSurface {
        if road_triangles
            .iter()
            .copied()
            .any(|triangle| triangle_contains_point_xz(triangle, point))
        {
            VisibleSurface::Road
        } else if curb_triangles
            .iter()
            .copied()
            .any(|triangle| triangle_contains_point_xz(triangle, point))
        {
            VisibleSurface::Curb
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
        let curb_triangles = main_triangles(mesh_data, VisibleSurface::Curb);
        let sidewalk_triangles = main_triangles(mesh_data, VisibleSurface::Sidewalk);
        let mut covered = 0usize;
        let mut total = 0usize;
        let mut z = min.y;
        while z <= max.y {
            let mut x = min.x;
            while x <= max.x {
                total += 1;
                let visible_surface = visible_surface_at_point(
                    &road_triangles,
                    &curb_triangles,
                    &sidewalk_triangles,
                    Vector2::new(x, z),
                );
                if visible_surface == target
                    || (target == VisibleSurface::CurbOrSidewalk
                        && matches!(
                            visible_surface,
                            VisibleSurface::Curb | VisibleSurface::Sidewalk
                        ))
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

    fn triangle_coverage_ratio(
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
                if triangles
                    .iter()
                    .copied()
                    .any(|triangle| triangle_contains_point_xz(triangle, Vector2::new(x, z)))
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
        let config = WorldConfig::default();
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

    fn cross_slope_terrain(width: usize, height: usize) -> TerrainSystem {
        let mut terrain = TerrainSystem::with_chunking(width, height, 1.0, 8, 0.0);
        for z in 0..height {
            for x in 0..width {
                terrain.set_height(x, z, x as f32 * 0.005);
            }
        }
        terrain
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
    fn test_editor_path_straight_road_keeps_terminal_end_bands() {
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
        let left_terminal_end_band_sidewalk = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-21.5, -3.3),
            Vector2::new(-20.25, 3.3),
            0.25,
            VisibleSurface::Sidewalk,
        );
        let right_terminal_end_band_sidewalk = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(20.25, -3.3),
            Vector2::new(21.5, 3.3),
            0.25,
            VisibleSurface::Sidewalk,
        );
        let left_terminal_slab_leak = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-22.25, -3.3),
            Vector2::new(-21.75, 3.3),
            0.25,
            VisibleSurface::Sidewalk,
        );
        let right_terminal_slab_leak = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(21.75, -3.3),
            Vector2::new(22.25, 3.3),
            0.25,
            VisibleSurface::Sidewalk,
        );

        assert!(left_asphalt >= 0.8 && right_asphalt >= 0.8);
        assert!(left_sidewalk >= 0.45 && right_sidewalk >= 0.45);
        assert!(left_terminal_end_band_sidewalk >= 0.8 && right_terminal_end_band_sidewalk >= 0.8);
        assert!(left_terminal_slab_leak <= 0.05 && right_terminal_slab_leak <= 0.05);
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
    fn test_cross_slope_standard_road_uses_bounded_compiled_cross_section_heights() {
        let renderer = RoadRenderer;
        let terrain = cross_slope_terrain(128, 128);
        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::new(0.0, 0.0, -24.0),
            Vector3::new(0.0, 0.0, 24.0),
            10.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 80.0);

        let left_y = mesh_data
            .road_vertices
            .iter()
            .filter(|vertex| vertex.x <= -3.0)
            .map(|vertex| vertex.y)
            .fold(f32::INFINITY, f32::min);
        let right_y = mesh_data
            .road_vertices
            .iter()
            .filter(|vertex| vertex.x >= 3.0)
            .map(|vertex| vertex.y)
            .fold(f32::NEG_INFINITY, f32::max);
        let max_normal_x = mesh_data
            .road_normals
            .iter()
            .map(|normal| normal.x.abs())
            .fold(0.0_f32, f32::max);

        assert!(left_y.is_finite() && right_y.is_finite());
        assert!(
            right_y - left_y >= 0.29,
            "expected bounded cross-slope road surface to still span height across width, got left_y={left_y:.3} right_y={right_y:.3}"
        );
        assert!(
            right_y - left_y <= 0.5,
            "expected grounded-road cross-slope to stay within the bounded design profile, got left_y={left_y:.3} right_y={right_y:.3}"
        );
        assert!(
            max_normal_x >= 0.02,
            "expected compiled bounded cross-slope normals to tilt with the surface, got max_normal_x={max_normal_x:.3}"
        );
    }

    #[test]
    fn test_cross_slope_standard_road_uses_stitched_terrain_instead_of_visible_earthwork_mesh() {
        let renderer = RoadRenderer;
        let terrain = cross_slope_terrain(128, 128);
        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(0.0, 0.0, -24.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::new(0.0, 0.0, -24.0),
            Vector3::new(0.0, 0.0, 24.0),
            10.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 80.0);

        assert!(
            mesh_data.earthwork_vertices.is_empty(),
            "grounded standard roads must not emit a visible closure strip; Rust-generated stitched terrain owns the road/terrain boundary"
        );
    }

    #[test]
    fn test_flat_standard_road_emits_no_visible_earthwork_mesh() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::new(-24.0, 0.0, 0.0),
            Vector3::new(24.0, 0.0, 0.0),
            10.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 80.0);

        assert!(
            mesh_data.earthwork_vertices.is_empty(),
            "flat grounded standard roads must not render ordinary earthwork or closure geometry"
        );
    }

    #[test]
    fn test_compiled_top_surfaces_render_at_solved_physical_height() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::new(-20.0, 0.0, 0.0),
            Vector3::new(20.0, 0.0, 0.0),
            10.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 60.0);

        let road_clearance = mesh_data
            .road_vertices
            .iter()
            .map(|vertex| {
                vertex.y
                    - terrain.sample_visual_height_world(vertex.x, vertex.z) * config::HEIGHT_SCALE
            })
            .fold(f32::INFINITY, f32::min);
        let sidewalk_clearance = mesh_data
            .sidewalk_vertices
            .iter()
            .map(|vertex| {
                vertex.y
                    - terrain.sample_visual_height_world(vertex.x, vertex.z) * config::HEIGHT_SCALE
            })
            .fold(f32::INFINITY, f32::min);
        let curb_clearance = mesh_data
            .curb_vertices
            .iter()
            .map(|vertex| {
                vertex.y
                    - terrain.sample_visual_height_world(vertex.x, vertex.z) * config::HEIGHT_SCALE
            })
            .fold(f32::INFINITY, f32::min);

        assert!(road_clearance.abs() <= 0.001);
        assert!(
            curb_clearance.abs() <= 0.001,
            "expected compiled curb surface to use solved physical height, got clearance={curb_clearance:.4}"
        );
        assert!(
            (sidewalk_clearance - 0.12).abs() <= 0.001,
            "expected compiled sidewalk surface to use solved raised height, got clearance={sidewalk_clearance:.4}"
        );
    }

    #[test]
    fn test_curb_mesh_has_flat_top_and_explicit_vertical_face() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::new(-20.0, 0.0, 0.0),
            Vector3::new(20.0, 0.0, 0.0),
            10.0,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 60.0);

        let mut flat_top_triangles = 0usize;
        let mut vertical_face_triangles = 0usize;
        let mut asphalt_facing_left_curb_triangles = 0usize;
        let mut asphalt_facing_right_curb_triangles = 0usize;
        let mut asphalt_facing_start_cap_triangles = 0usize;
        let mut asphalt_facing_end_cap_triangles = 0usize;
        for triangle in triangles_from_vertices(&mesh_data.curb_vertices) {
            let projected_area = triangle_projected_double_area(triangle).abs();
            let y_delta = triangle_y_delta(triangle);
            assert!(
                projected_area >= 0.001,
                "curb top bucket must not contain vertical faces; triangle={triangle:?}"
            );
            assert!(
                y_delta <= 0.001,
                "curb top triangles must be flat; triangle={triangle:?}"
            );
            flat_top_triangles += 1;
        }

        for triangle in triangles_from_vertices(&mesh_data.curb_vertical_vertices) {
            let projected_area = triangle_projected_double_area(triangle).abs();
            let y_delta = triangle_y_delta(triangle);
            assert!(
                projected_area < 0.001 && y_delta >= 0.05,
                "curb vertical bucket must contain only vertical faces; triangle={triangle:?}"
            );
            let centroid = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
            let visible_direction = godot_cull_back_visible_direction(triangle);
            if centroid.x.abs() < 12.0 && (centroid.z + 5.0).abs() <= 0.2 {
                assert!(
                    visible_direction.z > 0.0,
                    "left curb face winding must be visible from asphalt under Godot cull-back convention; triangle={triangle:?} visible_direction={visible_direction:?}"
                );
                asphalt_facing_left_curb_triangles += 1;
            } else if centroid.x.abs() < 12.0 && (centroid.z - 5.0).abs() <= 0.2 {
                assert!(
                    visible_direction.z < 0.0,
                    "right curb face winding must be visible from asphalt under Godot cull-back convention; triangle={triangle:?} visible_direction={visible_direction:?}"
                );
                asphalt_facing_right_curb_triangles += 1;
            } else if (centroid.x + 20.0).abs() <= 0.2 && centroid.z.abs() <= 4.9 {
                assert!(
                    visible_direction.x > 0.0,
                    "start terminal curb cap must be visible from asphalt under Godot cull-back convention; triangle={triangle:?} visible_direction={visible_direction:?}"
                );
                asphalt_facing_start_cap_triangles += 1;
            } else if (centroid.x - 20.0).abs() <= 0.2 && centroid.z.abs() <= 4.9 {
                assert!(
                    visible_direction.x < 0.0,
                    "end terminal curb cap must be visible from asphalt under Godot cull-back convention; triangle={triangle:?} visible_direction={visible_direction:?}"
                );
                asphalt_facing_end_cap_triangles += 1;
            }
            vertical_face_triangles += 1;
        }

        let road_boundary_edges = top_surface_boundary_edges(&mesh_data.road_vertices);
        let curb_top_boundary_edges = top_surface_boundary_edges(&mesh_data.curb_vertices);
        let road_boundary_segments = top_surface_boundary_segments(&mesh_data.road_vertices);
        let curb_top_boundary_segments = top_surface_boundary_segments(&mesh_data.curb_vertices);
        let mut left_span_lower_edges = 0usize;
        let mut left_span_upper_edges = 0usize;
        let mut right_span_lower_edges = 0usize;
        let mut right_span_upper_edges = 0usize;
        let mut start_cap_lower_edges = 0usize;
        let mut start_cap_upper_edges = 0usize;
        let mut end_cap_lower_edges = 0usize;
        let mut end_cap_upper_edges = 0usize;

        for edge in vertical_curb_face_horizontal_edges(&mesh_data.curb_vertical_vertices) {
            let edge_key = RenderEdgeKey::new(edge[0], edge[1]);
            let matches_road = road_boundary_edges.contains(&edge_key);
            let matches_curb_top = curb_top_boundary_edges.contains(&edge_key);
            assert!(
                matches_road ^ matches_curb_top,
                "vertical curb face horizontal edge must exactly match one adjacent rendered top boundary edge; edge={edge:?} matches_road={matches_road} matches_curb_top={matches_curb_top} nearby_road=[{}] nearby_curb_top=[{}]",
                nearby_boundary_edges_debug(edge, &road_boundary_segments),
                nearby_boundary_edges_debug(edge, &curb_top_boundary_segments)
            );

            let Some(region) = curb_face_region(edge) else {
                continue;
            };
            match (region, matches_road) {
                (CurbFaceRegion::LeftSpan, true) => left_span_lower_edges += 1,
                (CurbFaceRegion::LeftSpan, false) => left_span_upper_edges += 1,
                (CurbFaceRegion::RightSpan, true) => right_span_lower_edges += 1,
                (CurbFaceRegion::RightSpan, false) => right_span_upper_edges += 1,
                (CurbFaceRegion::StartTerminalCap, true) => start_cap_lower_edges += 1,
                (CurbFaceRegion::StartTerminalCap, false) => start_cap_upper_edges += 1,
                (CurbFaceRegion::EndTerminalCap, true) => end_cap_lower_edges += 1,
                (CurbFaceRegion::EndTerminalCap, false) => end_cap_upper_edges += 1,
            }
        }

        assert!(
            flat_top_triangles > 0,
            "curb mesh should include flat curb top triangles"
        );
        assert!(
            vertical_face_triangles > 0,
            "curb mesh should include explicit vertical curb face triangles"
        );
        assert!(
            asphalt_facing_left_curb_triangles > 0 && asphalt_facing_right_curb_triangles > 0,
            "span curb faces should be one-sided and front-facing from the asphalt side; left={asphalt_facing_left_curb_triangles} right={asphalt_facing_right_curb_triangles} vertical={vertical_face_triangles}"
        );
        assert!(
            asphalt_facing_start_cap_triangles > 0 && asphalt_facing_end_cap_triangles > 0,
            "terminal curb caps should be one-sided and front-facing from the asphalt side; start={asphalt_facing_start_cap_triangles} end={asphalt_facing_end_cap_triangles} vertical={vertical_face_triangles}"
        );
        assert!(
            left_span_lower_edges > 0
                && left_span_upper_edges > 0
                && right_span_lower_edges > 0
                && right_span_upper_edges > 0,
            "span curb face edges must close exactly against rendered asphalt and curb top boundaries; left_lower={left_span_lower_edges} left_upper={left_span_upper_edges} right_lower={right_span_lower_edges} right_upper={right_span_upper_edges}"
        );
        assert!(
            start_cap_lower_edges > 0
                && start_cap_upper_edges > 0
                && end_cap_lower_edges > 0
                && end_cap_upper_edges > 0,
            "terminal curb cap face edges must close exactly against rendered asphalt and curb top boundaries; start_lower={start_cap_lower_edges} start_upper={start_cap_upper_edges} end_lower={end_cap_lower_edges} end_upper={end_cap_upper_edges}"
        );
    }

    #[test]
    fn test_two_carriageway_terminal_cap_upper_edge_stays_raised_and_closed() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(create_surface_edge(
            n0,
            n1,
            &[Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            10.0,
            TransitType::Road,
            EdgeClass::Standard,
            TransitFlags::CAR | TransitFlags::FOOT,
            1,
            1,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 60.0);

        let road_boundary_edges = top_surface_boundary_edges(&mesh_data.road_vertices);
        let curb_top_boundary_edges = top_surface_boundary_edges(&mesh_data.curb_vertices);
        let mut start_lower_length = 0.0f32;
        let mut start_upper_length = 0.0f32;
        let mut end_lower_length = 0.0f32;
        let mut end_upper_length = 0.0f32;

        for edge in vertical_curb_face_horizontal_edges(&mesh_data.curb_vertical_vertices) {
            let Some(region) = curb_face_region(edge) else {
                continue;
            };
            if !matches!(
                region,
                CurbFaceRegion::StartTerminalCap | CurbFaceRegion::EndTerminalCap
            ) {
                continue;
            }

            let edge_key = RenderEdgeKey::new(edge[0], edge[1]);
            let matches_road = road_boundary_edges.contains(&edge_key);
            let matches_curb_top = curb_top_boundary_edges.contains(&edge_key);
            assert!(
                matches_road ^ matches_curb_top,
                "terminal cap vertical edge must close against exactly one adjacent top boundary; edge={edge:?} matches_road={matches_road} matches_curb_top={matches_curb_top}"
            );

            let length = Vector2::new(edge[1].x - edge[0].x, edge[1].z - edge[0].z).length();
            match (region, matches_road) {
                (CurbFaceRegion::StartTerminalCap, true) => start_lower_length += length,
                (CurbFaceRegion::StartTerminalCap, false) => {
                    assert!(
                        edge[0].y >= 0.119 && edge[1].y >= 0.119,
                        "start terminal cap upper edge must stay at raised curb height; edge={edge:?}"
                    );
                    start_upper_length += length;
                }
                (CurbFaceRegion::EndTerminalCap, true) => end_lower_length += length,
                (CurbFaceRegion::EndTerminalCap, false) => {
                    assert!(
                        edge[0].y >= 0.119 && edge[1].y >= 0.119,
                        "end terminal cap upper edge must stay at raised curb height; edge={edge:?}"
                    );
                    end_upper_length += length;
                }
                _ => {}
            }
        }

        assert!(
            start_lower_length >= 9.9
                && start_upper_length >= 9.9
                && end_lower_length >= 9.9
                && end_upper_length >= 9.9,
            "terminal cap faces must cover the full two-carriageway mouth; start_lower={start_lower_length:.3} start_upper={start_upper_length:.3} end_lower={end_lower_length:.3} end_upper={end_upper_length:.3}"
        );
    }

    #[test]
    fn test_oblique_two_carriageway_terminal_vertical_faces_are_asphalt_facing() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(256, 256);
        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        let mut graph = RegionGraph::new();

        let start_point = Vector3::new(21.162, 0.0, -160.894);
        let end_point = Vector3::new(71.074, 0.0, -91.746);
        let n0 = graph.add_node(start_point, NodeType::Junction);
        let n1 = graph.add_node(end_point, NodeType::Junction);
        graph.add_edge(create_surface_edge(
            n0,
            n1,
            &[start_point, end_point],
            14.0,
            TransitType::Road,
            EdgeClass::Standard,
            TransitFlags::CAR | TransitFlags::FOOT,
            2,
            2,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 240.0);

        let direction_xz =
            Vector2::new(end_point.x - start_point.x, end_point.z - start_point.z).normalized();
        let left_xz = Vector2::new(-direction_xz.y, direction_xz.x);
        let length =
            Vector2::new(end_point.x - start_point.x, end_point.z - start_point.z).length();
        let road_half_width = 7.0;
        let mut start_cap_triangles = 0usize;
        let mut end_cap_triangles = 0usize;
        let mut left_side_triangles = 0usize;
        let mut right_side_triangles = 0usize;

        for triangle in triangles_from_vertices(&mesh_data.curb_vertical_vertices) {
            if triangle_projected_double_area(triangle).abs() >= 0.001
                || triangle_y_delta(triangle) < 0.05
            {
                continue;
            }
            let centroid = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
            let rel = Vector2::new(centroid.x - start_point.x, centroid.z - start_point.z);
            let along = rel.dot(direction_xz);
            let lateral = rel.dot(left_xz);
            let visible_direction = godot_cull_back_visible_direction(triangle);
            let visible_xz = Vector2::new(visible_direction.x, visible_direction.z);

            if along.abs() <= 0.25 && lateral.abs() <= road_half_width - 0.1 {
                assert!(
                    visible_xz.dot(direction_xz) > 0.0,
                    "oblique start terminal cap must be visible from asphalt under Godot cull-back convention; triangle={triangle:?} visible_direction={visible_direction:?}"
                );
                start_cap_triangles += 1;
            } else if (along - length).abs() <= 0.25 && lateral.abs() <= road_half_width - 0.1 {
                assert!(
                    visible_xz.dot(direction_xz) < 0.0,
                    "oblique end terminal cap must be visible from asphalt under Godot cull-back convention; triangle={triangle:?} visible_direction={visible_direction:?}"
                );
                end_cap_triangles += 1;
            } else if along > 2.0
                && along < length - 2.0
                && (lateral + road_half_width).abs() <= 0.25
            {
                assert!(
                    visible_xz.dot(left_xz) > 0.0,
                    "oblique negative-lateral curb face must be visible from asphalt under Godot cull-back convention; triangle={triangle:?} visible_direction={visible_direction:?}"
                );
                left_side_triangles += 1;
            } else if along > 2.0
                && along < length - 2.0
                && (lateral - road_half_width).abs() <= 0.25
            {
                assert!(
                    visible_xz.dot(left_xz) < 0.0,
                    "oblique positive-lateral curb face must be visible from asphalt under Godot cull-back convention; triangle={triangle:?} visible_direction={visible_direction:?}"
                );
                right_side_triangles += 1;
            }
        }

        assert!(
            start_cap_triangles > 0
                && end_cap_triangles > 0
                && left_side_triangles > 0
                && right_side_triangles > 0,
            "oblique two-carriageway terminal must emit asphalt-facing vertical faces on both caps and both sides; start_cap={start_cap_triangles} end_cap={end_cap_triangles} left_side={left_side_triangles} right_side={right_side_triangles}"
        );
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
            VisibleSurface::CurbOrSidewalk,
        );
        let right_shoulder = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(1.75, -4.75),
            Vector2::new(4.75, -3.5),
            0.25,
            VisibleSurface::CurbOrSidewalk,
        );
        let left_apron = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-3.0, -4.9),
            Vector2::new(-1.0, -3.6),
            0.2,
            VisibleSurface::CurbOrSidewalk,
        );
        let right_apron = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(1.0, -4.9),
            Vector2::new(3.0, -3.6),
            0.2,
            VisibleSurface::CurbOrSidewalk,
        );
        let opposite_shoulder = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-2.0, 3.6),
            Vector2::new(2.0, 4.9),
            0.2,
            VisibleSurface::CurbOrSidewalk,
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
    fn test_obtuse_bend_generates_legal_join_ownership() {
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
        assert!(throat_is_road >= 0.95, "throat_is_road={throat_is_road:.3}");
    }

    #[test]
    fn test_triangle_network_generates_valid_bend_corner_join_ownership_where_available() {
        let ab = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)];
        let bc = [Vector3::new(24.0, 0.0, 0.0), Vector3::new(12.0, 0.0, 20.0)];
        let ca = [Vector3::new(12.0, 0.0, 20.0), Vector3::new(0.0, 0.0, 0.0)];
        let (_graph, mesh_data, _terrain) =
            generate_editor_mesh(&[(&ab, 1, 1), (&bc, 1, 1), (&ca, 1, 1)]);
        validate_mesh(&mesh_data, 80.0);

        let corner_a_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(1.0, 1.0),
            Vector2::new(4.5, 4.5),
            0.25,
            VisibleSurface::Road,
        );
        let corner_b_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(19.5, 1.0),
            Vector2::new(23.0, 4.5),
            0.25,
            VisibleSurface::Road,
        );
        let corner_c_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(10.0, 14.5),
            Vector2::new(14.0, 18.5),
            0.25,
            VisibleSurface::Road,
        );

        assert!(corner_a_road >= 0.95, "corner_a_road={corner_a_road:.3}");
        assert!(corner_b_road >= 0.95, "corner_b_road={corner_b_road:.3}");
        assert!(corner_c_road >= 0.95, "corner_c_road={corner_c_road:.3}");
    }

    #[test]
    fn test_t_junction_compiles_with_legal_join_ownership() {
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

        assert!(
            center_road >= 0.90,
            "T junction should render legal join ownership at the center; center_road={center_road:.3}"
        );
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
    fn test_four_way_sidewalk_corners_stay_in_their_quadrants() {
        let north = [Vector3::new(0.0, 0.0, -20.0), Vector3::new(0.0, 0.0, 0.0)];
        let south = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 20.0)];
        let west = [Vector3::new(-20.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)];
        let east = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)];
        let (_graph, mesh_data, _terrain) =
            generate_editor_mesh(&[(&north, 1, 1), (&south, 1, 1), (&west, 1, 1), (&east, 1, 1)]);
        validate_mesh(&mesh_data, 60.0);

        for (label, min, max) in [
            (
                "north_west",
                Vector2::new(-6.0, -6.0),
                Vector2::new(-3.5, -3.5),
            ),
            (
                "north_east",
                Vector2::new(3.5, -6.0),
                Vector2::new(6.0, -3.5),
            ),
            ("south_east", Vector2::new(3.5, 3.5), Vector2::new(6.0, 6.0)),
            (
                "south_west",
                Vector2::new(-6.0, 3.5),
                Vector2::new(-3.5, 6.0),
            ),
        ] {
            let sidewalk =
                visible_coverage_ratio(&mesh_data, min, max, 0.25, VisibleSurface::Sidewalk);
            let road = visible_coverage_ratio(&mesh_data, min, max, 0.25, VisibleSurface::Road);
            assert!(
                sidewalk >= 0.35,
                "{label}_sidewalk={sidewalk:.3} {label}_road={road:.3}"
            );
            assert!(
                road <= 0.25,
                "{label}_road={road:.3} {label}_sidewalk={sidewalk:.3}"
            );
        }
    }

    #[test]
    fn test_four_way_inner_quadrants_stay_carriageway_owned() {
        let north = [Vector3::new(0.0, 0.0, -20.0), Vector3::new(0.0, 0.0, 0.0)];
        let south = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 20.0)];
        let west = [Vector3::new(-20.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)];
        let east = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)];
        let (_graph, mesh_data, _terrain) =
            generate_editor_mesh(&[(&north, 1, 1), (&south, 1, 1), (&west, 1, 1), (&east, 1, 1)]);
        validate_mesh(&mesh_data, 60.0);

        for (label, min, max) in [
            (
                "north_west",
                Vector2::new(-3.5, -3.5),
                Vector2::new(-1.0, -1.0),
            ),
            (
                "north_east",
                Vector2::new(1.0, -3.5),
                Vector2::new(3.5, -1.0),
            ),
            ("south_east", Vector2::new(1.0, 1.0), Vector2::new(3.5, 3.5)),
            (
                "south_west",
                Vector2::new(-3.5, 1.0),
                Vector2::new(-1.0, 3.5),
            ),
        ] {
            let road = visible_coverage_ratio(&mesh_data, min, max, 0.25, VisibleSurface::Road);
            assert!(road >= 0.55, "{label}_road={road:.3}");
        }
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
        let outer_band_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-2.0, -4.8),
            Vector2::new(2.0, -3.8),
            0.25,
            VisibleSurface::Road,
        );

        assert!(junction_core >= 0.8, "junction_core={junction_core:.3}");
        assert!(
            outer_band >= 0.25,
            "outer_sidewalk={outer_band:.3} outer_road={outer_band_road:.3}"
        );
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

        assert!(
            junction_core >= 0.90,
            "mixed-width T junction should render legal join ownership at the core; junction_core={junction_core:.3}"
        );
        assert!(bubble_zone <= 0.15);
    }

    #[test]
    fn test_two_way_diagonal_width_change_rejects_invalid_junction_surface() {
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
        assert!(branch_throat < 0.7);
    }

    #[test]
    fn test_bridge_deck_uses_compiled_surface_continuously() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        let mut graph = RegionGraph::new();

        let start = Vector3::new(-30.0, 6.0, 0.0);
        let mid = Vector3::new(0.0, 6.0, 0.0);
        let end = Vector3::new(30.0, 6.0, 0.0);
        let n0 = graph.add_node(start, NodeType::Junction);
        let n1 = graph.add_node(end, NodeType::Junction);
        graph.add_edge(create_surface_edge(
            n0,
            n1,
            &[start, mid, end],
            10.0,
            TransitType::Road,
            EdgeClass::Bridge,
            TransitFlags::CAR | TransitFlags::FOOT,
            1,
            1,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 90.0);

        let deck_road = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-8.0, -4.0),
            Vector2::new(8.0, 4.0),
            0.25,
            VisibleSurface::Road,
        );
        let deck_sidewalk = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-8.0, 4.4),
            Vector2::new(8.0, 6.0),
            0.25,
            VisibleSurface::CurbOrSidewalk,
        );

        assert!(deck_road >= 0.95);
        assert!(deck_sidewalk >= 0.5);
        assert!(
            !mesh_data.earthwork_vertices.is_empty(),
            "expected bridge abutments to keep explicit visible earthwork geometry"
        );
        assert!(
            !mesh_data.concrete_vertices.is_empty(),
            "expected bridge structural concrete to remain rendered"
        );
    }

    #[test]
    fn test_tunnel_surface_only_renders_portals() {
        let renderer = RoadRenderer;
        let terrain = TerrainSystem::new(128, 128);
        let lane_system = crate::simulation::network::lanes::LaneSystem::new();
        let mut graph = RegionGraph::new();

        let p0 = Vector3::new(-30.0, 0.0, 0.0);
        let p1 = Vector3::new(-10.0, -6.0, 0.0);
        let p2 = Vector3::new(10.0, -6.0, 0.0);
        let p3 = Vector3::new(30.0, 0.0, 0.0);
        let n0 = graph.add_node(p0, NodeType::Junction);
        let n1 = graph.add_node(p3, NodeType::Junction);
        graph.add_edge(create_surface_edge(
            n0,
            n1,
            &[p0, p1, p2, p3],
            10.0,
            TransitType::Road,
            EdgeClass::Tunnel,
            TransitFlags::CAR | TransitFlags::FOOT,
            1,
            1,
        ));

        graph.rebuild_adjacency_list();
        let mesh_data = renderer.generate_mesh_data(&graph, &lane_system, &terrain);
        validate_mesh(&mesh_data, 90.0);

        let left_portal = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-29.0, -4.0),
            Vector2::new(-18.0, 4.0),
            0.25,
            VisibleSurface::Road,
        );
        let right_portal = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(18.0, -4.0),
            Vector2::new(29.0, 4.0),
            0.25,
            VisibleSurface::Road,
        );
        let buried_center = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-6.0, -4.0),
            Vector2::new(6.0, 4.0),
            0.25,
            VisibleSurface::Road,
        );

        assert!(left_portal >= 0.2);
        assert!(right_portal >= 0.2);
        assert!(buried_center <= 0.05);
    }

    #[test]
    fn test_compiled_lane_markings_terminate_at_junction_throats() {
        let north = [Vector3::new(0.0, 0.0, -24.0), Vector3::new(0.0, 0.0, 0.0)];
        let east = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)];
        let west = [Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)];
        let (_graph, mesh_data, _terrain) =
            generate_editor_mesh(&[(&north, 1, 1), (&east, 1, 1), (&west, 1, 1)]);
        validate_mesh(&mesh_data, 70.0);

        let marking_triangles = triangles_from_vertices(&mesh_data.marking_vertices);
        let arm_markings = triangle_coverage_ratio(
            &marking_triangles,
            Vector2::new(7.0, -0.25),
            Vector2::new(15.0, 0.25),
            0.1,
        );
        let center_markings = triangle_coverage_ratio(
            &marking_triangles,
            Vector2::new(-1.25, -1.25),
            Vector2::new(1.25, 1.25),
            0.1,
        );

        assert!(arm_markings >= 0.2);
        assert!(center_markings <= 0.05);
    }

    #[test]
    fn test_walkway_join_attaches_to_one_side_without_mirrored_apron() {
        let main = [Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)];
        let walkway = [Vector3::new(-12.0, 0.0, -12.0), Vector3::ZERO];
        let (_graph, mesh_data, _terrain) =
            generate_editor_mesh(&[(&main, 1, 1), (&walkway, 0, 0)]);
        validate_mesh(&mesh_data, 80.0);

        let joined_side = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-9.5, -9.5),
            Vector2::new(-6.5, -6.5),
            0.25,
            VisibleSurface::Sidewalk,
        );
        let mirrored_side = visible_coverage_ratio(
            &mesh_data,
            Vector2::new(-9.5, 6.5),
            Vector2::new(-6.5, 9.5),
            0.25,
            VisibleSurface::Sidewalk,
        );

        assert!(
            joined_side >= 0.6,
            "expected joined quadrant to carry the footpath continuation, got joined_side={joined_side:.3} mirrored_side={mirrored_side:.3}"
        );
        assert!(
            mirrored_side <= 0.05,
            "expected only the joined quadrant to carry the footpath continuation, got joined_side={joined_side:.3} mirrored_side={mirrored_side:.3}"
        );
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
