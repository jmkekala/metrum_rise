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
            vec![Vector3::new(8.0, 0.0, 10.0), Vector3::new(8.0, 0.0, 0.0)].into(),
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        assert!(graph.edges.len() > 0);
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

        assert!(graph.edges.len() > 0);
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
                Vector3::new(-71.690018, 0.0, -74.325249),
                Vector3::new(103.143341, 0.0, 106.812721),
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
                Vector3::new(-46.610786, 0.0, 51.625542),
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
                    && (node.pos.z - 9.703340).abs() < 0.01
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
                center_x: 75.0,
                center_y: 10.0,
                support_height_m: 0.0,
                width_cells: 2,
                depth_cells: 2,
                zone_profile_runtime_id: 0,
                parcel_id: 0,
                zone_type: crate::simulation::zoning::ZoneType::Residential,
                facing_dir: godot::prelude::Vector2::new(0.0, 1.0),
                frontage_t: 0.75,
                side_offset: 5.0,
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
                construction_remaining_hours: 0,
                broken: false,
                economy_profile_runtime_id: 0,
                economy_broken: false,
                resource_inventory: Vec::new(),
                revenue: 0.0,
                operating_budget: 500.0,
                profit_tax_budget_baseline: 500.0,
                last_day_profit: 0.0,
                shipment_cooldown_hours: 0,
                daily_owa_input_value: 0.0,
                daily_local_input_value: 0.0,
                daily_city_funded_input_cost: 0.0,
                daily_household_sales_value: 0.0,
                daily_power_service_units: 0.0,
                daily_power_served_units: 0.0,
                recent_power_service_units: 0.0,
                recent_power_served_units: 0.0,
                recent_household_sales_value: 0.0,
                commercial_activity_floor_scale: 0.0,
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

    #[test]
    fn test_remove_node_and_merge_edges_refuses_conflicting_vehicle_frontage_access() {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);

        let common = Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 10.0,
            physical_length: 10.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        };

        let e0 = graph.add_edge(common.clone());
        let e1 = graph.add_edge(Edge {
            start_node: n1,
            end_node: n2,
            geometry: vec![Vector3::new(10.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(10.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            vehicle_frontage_access: VehicleFrontageAccess::SameSideOnly,
            ..common
        });

        assert_eq!(graph.remove_node_and_merge_edges(n1), None);
        assert!(!graph.edge(e0).deleted);
        assert!(!graph.edge(e1).deleted);
    }
}
