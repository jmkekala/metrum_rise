// SPDX-License-Identifier: GPL-2.0-only

//! Live road topology and pathing rebuild regressions.

#[cfg(test)]
mod tests {
    use crate::assets::AssetManifest;
    use crate::assets::asset::{BuildingData, MeshPart, PlacementMode, ZoneClass};
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::Vector3;

    fn register_test_asset(
        allocator: &mut BuildingAllocator,
        pack_id: &str,
        asset_id: &str,
        zone: ZoneClass,
    ) -> String {
        let (household_capacity, worker_capacity) = match zone {
            ZoneClass::Residential => (Some(6), None),
            ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
            ZoneClass::Mixed => (Some(4), Some(2)),
        };
        allocator.registry.register(
            pack_id,
            AssetManifest {
                asset_id: asset_id.to_owned(),
                display_name: "Test".to_owned(),
                asset_set: None,
                tags: vec![],
                thumbnail: None,
                lods: vec![],
                mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
                anchors: vec![],
                site_surfaces: vec![],
                building: Some(BuildingData {
                    window_brightness: 3.0,
                    appearance: None,
                    flat_size_m2: None,
                    placement_mode: PlacementMode::ZonedPrivate,
                    zone_type: Some(zone),
                    density: Some("low".to_owned()),
                    lot_width_cells: 2,
                    lot_depth_cells: 2,
                    frontage_forward: None,
                    min_zone_width_cells: None,
                    min_zone_depth_cells: None,
                    level: 1,
                    household_capacity,
                    worker_capacity,
                    service_class: None,
                    economy_profile: None,
                    extractor: None,
                    field: None,
                    yard_hedge: None,
                    yard_planting: Vec::new(),
                }),
                prop: None,
                vehicle: None,
                character: None,
            },
            String::new(),
        );
        format!("{pack_id}:{asset_id}")
    }

    #[test]
    fn test_rebuild_pathing_skips_deleted_edges_without_compacting() {
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let end = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        for cost in [1.0, 7.0] {
            graph.add_edge(Edge {
                start_node: start,
                end_node: end,
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR,
                width: 8.0,
                fwd_lanes: 1,
                bkw_lanes: 1,
                speed_limit: 20.0,
                physical_length: 100.0,
                base_cost: cost,
                ..Edge::default()
            });
        }
        graph.edge_mut(0).deleted = true;
        graph.rebuild_adjacency_list();
        network.cch_dirty_chunks.insert((0, 0));
        network.rebuild_pathing(&mut graph);

        assert_eq!(
            graph.edge_count(),
            2,
            "pathing rebuild must retain deleted slots"
        );
        assert!(graph.edge(0).deleted);
        let (cost, _, nodes) = network
            .cch_graph
            .find_path(
                start,
                end,
                usize::MAX,
                usize::MAX,
                &graph,
                TransitFlags::CAR,
            )
            .expect("live edge must remain routable after rebuild");
        assert_eq!(cost, 7.0, "CCH must ignore the deleted lower-cost edge");
        assert_eq!(nodes, vec![start, end]);
    }

    #[test]
    fn test_topology_split_near_end() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        // long road with many segments (250m)
        let pts = vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(250.0, 0.0, 0.0)];
        let config = WorldConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        net.add_road(
            &mut graph,
            pts,
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // side road connecting near the end (segment 8 of 9)
        net.add_road(
            &mut graph,
            vec![Vector3::new(8.0, 0.0, 10.0), Vector3::new(8.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        assert!(!graph.edges.is_empty());
    }

    #[test]
    fn test_shallow_angle_intersection() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = WorldConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        // straight road
        net.add_road(
            &mut graph,
            vec![
                Vector3::new(-100.0, 0.0, 0.0),
                Vector3::new(100.0, 0.0, 0.0),
            ],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // super shallow angle road (approx 3 degrees)
        net.add_road(
            &mut graph,
            vec![Vector3::new(100.0, 0.0, 5.0), Vector3::new(0.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        assert!(!graph.edges.is_empty());
    }

    #[test]
    fn crossing_roads_from_debug_log_create_shared_junction() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = WorldConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        net.add_road(
            &mut graph,
            vec![
                Vector3::new(-71.690_02, 0.0, -74.325_25),
                Vector3::new(103.143_34, 0.0, 106.812_72),
            ],
            1,
            1,
            EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        net.add_road(
            &mut graph,
            vec![
                Vector3::new(105.070206, 0.0, -61.874428),
                Vector3::new(-46.610786, 0.0, 51.625_54),
            ],
            1,
            1,
            EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        let junction = graph
            .nodes
            .iter()
            .enumerate()
            .find(|(_, node)| {
                node.node_type == NodeType::Junction
                    && (node.pos.x - 9.413908).abs() < 0.01
                    && (node.pos.z - 9.703_34).abs() < 0.01
            })
            .map(|(node_id, _)| node_id as u32)
            .unwrap_or_else(|| {
                panic!(
                    "crossing roads should create a junction at the centerline intersection; nodes={:?} edges={:?}",
                    graph
                        .nodes
                        .iter()
                        .enumerate()
                        .map(|(node_id, node)| (node_id, node.pos.x, node.pos.z))
                        .collect::<Vec<_>>(),
                    graph
                        .edges
                        .iter()
                        .enumerate()
                        .map(|(edge_id, edge)| (
                            edge_id,
                            edge.start_node,
                            edge.end_node,
                            edge.deleted
                        ))
                        .collect::<Vec<_>>()
                )
            });

        let active_degree = graph
            .node_adjacency(junction)
            .iter()
            .filter(|&&edge_id| !graph.edge(edge_id).deleted)
            .count();

        assert_eq!(
            active_degree, 4,
            "the centerline crossing must split both roads into four incident spans"
        );
        assert_eq!(
            graph.edges.iter().filter(|edge| !edge.deleted).count(),
            4,
            "two crossing road spans should become four active graph edges"
        );
    }

    #[test]
    fn test_4_way_intersection() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let _center = graph.find_or_add_node(Vector3::new(0.0, 0.0, 0.0), 0.1, NodeType::Junction);

        // North, East, South, West roads connecting to center
        let dirs = [
            Vector3::new(0.0, 0.0, -100.0),
            Vector3::new(100.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 100.0),
            Vector3::new(-100.0, 0.0, 0.0),
        ];

        let config = WorldConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        for dir in dirs {
            net.add_road(
                &mut graph,
                vec![dir, Vector3::new(0.0, 0.0, 0.0)],
                1,
                1,
                crate::simulation::network::types::EdgeClass::Standard,
                &mut zoning,
                &mut allocator,
            );
        }

        assert!(graph.edges.len() >= 4);
    }

    #[test]
    fn test_extreme_angles() {
        use std::f32::consts::PI;

        let way_counts = [2, 3, 4, 8];

        for ways in way_counts {
            for deg in (10..=170).step_by(20) {
                // Test a subset to be faster
                let mut net = TransitNetwork::new();
                let mut graph = RegionGraph::new();
                let _center =
                    graph.find_or_add_node(Vector3::new(0.0, 0.0, 0.0), 0.1, NodeType::Junction);

                let config = WorldConfig::default();
                let mut zoning = ZoningSystem::new(&config);
                let mut allocator = BuildingAllocator::new();

                // Standard evenly spaced roads
                for w in 0..ways - 1 {
                    let standard_angle = (w as f32) * (PI * 2.0) / (ways as f32);
                    let dir = Vector3::new(
                        standard_angle.cos() * 100.0,
                        0.0,
                        standard_angle.sin() * 100.0,
                    );
                    net.add_road(
                        &mut graph,
                        vec![dir, Vector3::new(0.0, 0.0, 0.0)],
                        1,
                        1,
                        crate::simulation::network::types::EdgeClass::Standard,
                        &mut zoning,
                        &mut allocator,
                    );
                }

                // One extreme sweeper road relative to the 0-degree East road
                let rad = deg as f32 * PI / 180.0;
                let dir_extreme = Vector3::new(rad.cos() * 100.0, 0.0, rad.sin() * 100.0);
                net.add_road(
                    &mut graph,
                    vec![dir_extreme, Vector3::new(0.0, 0.0, 0.0)],
                    1,
                    1,
                    crate::simulation::network::types::EdgeClass::Standard,
                    &mut zoning,
                    &mut allocator,
                );

                // Verify angle-aware clips stay finite and fit inside the physical edge.
                for edge in graph.edges.iter() {
                    let length_m: f32 = edge
                        .geometry
                        .windows(2)
                        .map(|window| window[0].distance_to(window[1]))
                        .sum();
                    assert!(edge.start_clip.is_finite());
                    assert!(edge.end_clip.is_finite());
                    assert!(edge.start_clip >= 0.0);
                    assert!(edge.end_clip >= 0.0);
                    assert!(
                        edge.start_clip + edge.end_clip <= length_m.max(0.0),
                        "clips exceed edge length: start={} end={} length={}",
                        edge.start_clip,
                        edge.end_clip,
                        length_m
                    );
                }
            }
        }
    }

    #[test]
    fn test_transit_graph_add_road() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = WorldConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        // Add a 250m straight road
        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(250.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // 250m road: no intermediate subdivision nodes, single edge with two endpoint nodes.
        assert_eq!(
            graph.edges.len(),
            1,
            "Should have 1 edge for 250m road (no 100m subdivision)"
        );
        assert_eq!(graph.nodes.len(), 2, "Should have 2 nodes for 250m road");
    }

    #[test]
    fn test_transit_graph_split_edge() {
        use crate::simulation::network::topology::split_edge;
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = WorldConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        // 1. Add a 100m road.
        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        assert_eq!(graph.edges.len(), 1);
        let old_edge_id = 0;
        let _old_length = graph.edges[old_edge_id].physical_length;
        graph.edge_mut(old_edge_id).vehicle_frontage_access = VehicleFrontageAccess::SameSideOnly;
        let residential_asset = register_test_asset(
            &mut allocator,
            "test",
            "topology_split_residential",
            ZoneClass::Residential,
        );

        // 2. Add a building
        allocator
            .buildings
            .push(crate::simulation::buildings::allocator::Building {
                build_generation: 0,
                center_x: 75.0,
                center_y: 10.0,
                support_height_m: 0.0,
                width_cells: 2,
                depth_cells: 2,
                zone_profile_runtime_id: 0,
                parcel_id: 0,
                applicant_seed: 0,
                zone_type: crate::simulation::zoning::ZoneType::Residential,
                facing_dir: godot::prelude::Vector2::new(0.0, 1.0),
                frontage_t: 0.75,
                side_offset: 5.0,
                front_gap_m: [0.0; 3],
                budget_distress: false,
                is_deserted: false,
                edge_idx: old_edge_id,
                side: 1,
                cell_x: 7,
                cell_y: 0,
                occupancy: 0,
                worker_count: 0,
                service_funding_override: -1.0,
                asset_id: residential_asset,
                level: 1,
                construction_total_hours: 0,
                construction_complete_slot: 0,
                broken: false,
                economy_profile_runtime_id: 0,
                economy_broken: false,
                resource_inventory: Vec::new(),
                revenue: 0.0,
                operating_budget: 500.0,
                profit_tax_budget_baseline: 500.0,
                last_day_profit: 0.0,
                shipment_cooldown_until_slot: 0,
                daily_owa_input_value: 0.0,
                daily_local_input_value: 0.0,
                daily_city_funded_input_cost: 0.0,
                daily_household_sales_value: 0.0,
                daily_power_service_units: 0.0,
                daily_electricity: Default::default(),
                recent_electricity: Default::default(),
                hourly_economy_activity: 0.0,
                daily_power_served_units: 0.0,
                recent_power_service_units: 0.0,
                recent_power_served_units: 0.0,
                paid_workers_in_window: 0,
                funded_job_capacity: 0,
                unpaid_cash_wages_in_window: 0,
                recent_household_sales_value: 0.0,
                commercial_activity_floor_scale: 0.0,
                service_sales_capacity: 0.0,
                service_sales_profile_runtime_id: 0,
                funded_activity: 0.0,
                work_area_scale: 1.0,
                pending_redevelopment: false,
                rezone_grace_days_remaining: 0,
            });

        // 3. Create a split node at 40m
        let mid_pos = Vector3::new(40.0, 0.0, 0.0);
        let mid_id = graph.add_node(mid_pos, NodeType::Junction);

        // 4. Perform split
        split_edge(
            &mut net,
            &mut graph,
            old_edge_id,
            0,
            0.4,
            mid_id,
            &mut zoning,
            &mut allocator,
        );

        assert_eq!(graph.edges.iter().filter(|e| !e.deleted).count(), 2);
        for edge in graph.edges.iter().filter(|edge| !edge.deleted) {
            assert_eq!(
                edge.vehicle_frontage_access,
                VehicleFrontageAccess::SameSideOnly
            );
        }
    }

    // `ROAD-99`: grade separation by structure. A graph-only network and the roads added to it.
    struct Graded {
        net: TransitNetwork,
        graph: RegionGraph,
        zoning: ZoningSystem,
        allocator: BuildingAllocator,
    }

    impl Graded {
        fn new() -> Self {
            Self {
                net: TransitNetwork::new(),
                graph: RegionGraph::new(),
                zoning: ZoningSystem::new(&WorldConfig::default()),
                allocator: BuildingAllocator::new(),
            }
        }

        fn add(&mut self, points: [(f32, f32, f32); 2], class: EdgeClass) -> &mut Self {
            let grade = crate::simulation::network::topology::StrokeGrade::of_class(class);
            self.add_graded(points, class, grade)
        }

        fn add_graded(
            &mut self,
            points: [(f32, f32, f32); 2],
            class: EdgeClass,
            grade: crate::simulation::network::topology::StrokeGrade,
        ) -> &mut Self {
            self.net.add_road_graded(
                &mut self.graph,
                points.map(|(x, y, z)| Vector3::new(x, y, z)).to_vec(),
                1,
                1,
                None,
                class,
                grade,
                &mut self.zoning,
                &mut self.allocator,
            );
            self
        }

        fn live_edges(&self, class: EdgeClass) -> usize {
            (self.graph.edges.iter())
                .filter(|edge| !edge.deleted && edge.class == class)
                .count()
        }

        // The canonical nodes where three or more live edges meet.
        fn junctions(&self) -> Vec<u32> {
            (0..self.graph.node_count() as u32)
                .filter(|&node| self.graph.get_valid_node(node) == node)
                .filter(|&node| {
                    let arms = self.graph.node_adjacency(node);
                    arms.iter()
                        .filter(|&&edge| !self.graph.edge(edge).deleted)
                        .count()
                        >= 3
                })
                .collect()
        }
    }

    const ALONG_X: [(f32, f32, f32); 2] = [(-40.0, 0.0, 0.0), (40.0, 0.0, 0.0)];

    fn along_z(height: f32) -> [(f32, f32, f32); 2] {
        [(0.0, height, -40.0), (0.0, height, 40.0)]
    }

    // Two ground roads always meet where they cross, at any height between them (before
    // `ROAD-99`, 9 m apart they crossed without a node); the grade rules refuse what cannot meet.
    #[test]
    fn two_ground_roads_meet_at_any_height() {
        let mut graded = Graded::new();
        graded
            .add(ALONG_X, EdgeClass::Standard)
            .add(along_z(9.0), EdgeClass::Standard);
        assert_eq!(graded.junctions().len(), 1);
        assert_eq!(graded.live_edges(EdgeClass::Standard), 4);
    }

    // A ground road and a bridge, in either order and at any height between them, cross without
    // a node: neither is split. So do two bridges.
    #[test]
    fn a_structure_crosses_every_road_without_a_node() {
        for height in [1.0, 2.0, 6.0] {
            for (first, second) in [
                (EdgeClass::Standard, EdgeClass::Bridge),
                (EdgeClass::Bridge, EdgeClass::Standard),
                (EdgeClass::Bridge, EdgeClass::Bridge),
            ] {
                let case = format!("{first:?} then {second:?} {height} m above");
                let mut graded = Graded::new();
                graded.add(ALONG_X, first).add(along_z(height), second);
                assert!(graded.junctions().is_empty(), "{case}");
                assert_eq!(
                    graded.graph.edges.iter().filter(|e| !e.deleted).count(),
                    2,
                    "{case}"
                );
            }
        }
    }

    // A ground road passing through a bridge's bend node (only bridges meet it) passes under it;
    // through a bridge's abutment (a ground road meets it) it joins the node.
    #[test]
    fn a_ground_road_joins_a_bridge_at_its_abutment_only() {
        let mut bend = Graded::new();
        bend.add([(-40.0, 6.0, 0.0), (0.0, 6.0, 0.0)], EdgeClass::Bridge)
            .add([(0.0, 6.0, 0.0), (40.0, 6.0, 10.0)], EdgeClass::Bridge)
            .add(along_z(0.0), EdgeClass::Standard);
        assert!(bend.junctions().is_empty());
        assert_eq!(bend.live_edges(EdgeClass::Standard), 1);
        assert!(!bend.graph.grounded_node(bend.graph.edge(0).end_node));

        let mut abutment = Graded::new();
        abutment
            .add([(-40.0, 0.0, -20.0), (0.0, 0.0, 0.0)], EdgeClass::Standard)
            .add([(0.0, 0.0, 0.0), (40.0, 6.0, 0.0)], EdgeClass::Bridge)
            .add(along_z(0.0), EdgeClass::Standard);
        let junctions = abutment.junctions();
        assert_eq!(junctions.len(), 1);
        assert!(abutment.graph.grounded_node(junctions[0]));
        assert_eq!(abutment.graph.node(junctions[0]).pos, Vector3::ZERO);
        assert_eq!(abutment.live_edges(EdgeClass::Standard), 3);
        assert_eq!(abutment.live_edges(EdgeClass::Bridge), 1);
    }

    // A road's end 1.5 m from a span (within the join tolerance) joins it mid-span only when both
    // stand on the ground: a ground road's end joins a ground road, not a bridge; a bridge's end
    // at its support joins a ground road, its elevated end does not.
    #[test]
    fn an_end_joins_a_span_only_on_the_ground() {
        use crate::simulation::network::topology::StrokeGrade;
        let near_end = [(0.0, 0.0, -40.0), (0.0, 0.0, -1.5)];
        let mut ground = Graded::new();
        ground
            .add(ALONG_X, EdgeClass::Standard)
            .add(near_end, EdgeClass::Standard);
        assert_eq!(ground.junctions().len(), 1);

        let mut under = Graded::new();
        under
            .add([(-40.0, 6.0, 0.0), (40.0, 6.0, 0.0)], EdgeClass::Bridge)
            .add(near_end, EdgeClass::Standard);
        assert!(under.junctions().is_empty());

        for (ground_ends, joins) in [([false, true], true), ([true, false], false)] {
            let mut bridge = Graded::new();
            bridge.add(ALONG_X, EdgeClass::Standard).add_graded(
                [(0.0, 6.0, -40.0), (0.0, 0.0, -1.5)],
                EdgeClass::Bridge,
                StrokeGrade::bridge(ground_ends),
            );
            assert_eq!(
                bridge.junctions().len(),
                usize::from(joins),
                "{ground_ends:?}"
            );
        }
    }

    // A tunnel stroke's bore crosses a ground road without a node; its approach meets it.
    #[test]
    fn a_tunnel_strokes_approaches_meet_ground_roads_and_its_bore_does_not() {
        use crate::simulation::network::topology::StrokeGrade;
        // Knots every 10 m from x = -40 to 40; the bore from knot 2 (x = -20) to knot 6 (x = 20).
        let knots: Vec<Vector3> = (0..9)
            .map(|knot| Vector3::new(-40.0 + 10.0 * knot as f32, 0.0, 0.0))
            .collect();
        for (cross_x, meets) in [(0.0, false), (-30.0, true), (30.0, true)] {
            let mut graded = Graded::new();
            graded.add(
                [(cross_x, 0.0, -40.0), (cross_x, 0.0, 40.0)],
                EdgeClass::Standard,
            );
            graded.net.add_road_graded(
                &mut graded.graph,
                knots.clone(),
                1,
                1,
                None,
                EdgeClass::Tunnel,
                StrokeGrade::tunnel(knots.len(), [2, 6]),
                &mut graded.zoning,
                &mut graded.allocator,
            );
            assert_eq!(
                graded.junctions().len(),
                usize::from(meets),
                "crossing at x = {cross_x}"
            );
        }
    }
}
