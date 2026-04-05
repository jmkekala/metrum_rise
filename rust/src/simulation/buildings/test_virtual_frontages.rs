#[cfg(test)]
mod tests {
    use crate::assets::asset::{BuildingData, LodEntry, ZoneClass};
    use crate::assets::AssetManifest;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::economy::agents::{
        MODE_CAR, TRANSIT_ARRIVING, TRANSIT_ON_ROAD,
    };
    use crate::simulation::economy::demand::DemandSystem;
    use crate::simulation::grid::desirability::DesirabilitySystem;
    use crate::simulation::grid::noise::NoiseSystem;
    use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::EdgeClass;
    use godot::prelude::Vector3;

    fn register_test_residential(allocator: &mut BuildingAllocator) {
        allocator.registry.register("base", AssetManifest {
            asset_id: "b.res.house".to_owned(),
            display_name: "Test House".to_owned(),
            asset_set: None, tags: vec![], thumbnail: None,
            lods: vec![LodEntry { file: "lod0.glb".to_owned(), distance_min_m: 0.0, distance_max_m: None }],
            anchors: vec![],
            building: Some(BuildingData {
                zone_type: ZoneClass::Residential,
                lot_width_cells: 1, lot_depth_cells: 1, level: 1,
                residents_capacity: Some(6), worker_capacity: None,
                service_class: None, preview_scale: None,
            }),
            prop: None, vehicle: None, character: None,
        });
    }

    #[test]
    fn test_virtual_frontage_placement() {
        // Option C: each building placement calls split_for_frontage, inserting a real graph
        // node at the exact frontage position and splitting the edge into two half-edges.
        let config = MapConfig::default();
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        register_test_residential(&mut allocator);
        let mut agents = AgentSystem::new();
        let mut demand = DemandSystem::new();

        let mut desirability = DesirabilitySystem::new(&config);
        let (env_w, env_h) = config.get_env_grid_size();
        for x in 0..env_w {
            for y in 0..env_h {
                desirability.grid.set(x, y, 100.0);
            }
        }
        let noise = NoiseSystem::new(&config);

        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // Zone cells 5-7 (frontage_t ≈ 0.65 on the original 100m edge).
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, 5 + dx, dy, ZoneType::Residential, &graph);
            }
        }
        demand.residential = 500.0;

        let initial_nodes = graph.node_count();
        let initial_edges = graph.edge_count();

        allocator.tick(
            &mut demand, &mut zoning, &desirability, &noise,
            &mut agents, &mut net, &mut graph, &config,
        );

        assert_eq!(allocator.buildings.len(), 1, "Building should have spawned");
        let b = &allocator.buildings[0];

        // Option C: a real node was inserted at the frontage position.
        assert_eq!(
            graph.node_count(), initial_nodes + 1,
            "Exactly one frontage node should have been inserted"
        );
        assert!(
            graph.edge_count() > initial_edges,
            "Edge should have been split into two half-edges"
        );

        // The building references the new node directly.
        let expected_frontage_node = initial_nodes as u32;
        assert_eq!(
            b.frontage_node, expected_frontage_node,
            "Building should reference the new frontage node"
        );

        // After the split the building migrates to the start of the second half-edge
        // (cell_x=0 on the new edge). frontage_t is small — the building center is one
        // half-cell (5 m) from the frontage node that begins the second half.
        assert!(
            b.frontage_t < 0.3,
            "frontage_t near start of second half-edge should be small, got {}",
            b.frontage_t
        );
    }

    #[test]
    fn test_virtual_frontage_routing_targets() {
        // Option C: two sequential ticks each insert a frontage node. The second building lands
        // on the second half-edge produced by the first split. Both buildings get distinct nodes.
        let config = MapConfig::default();
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        register_test_residential(&mut allocator);
        let mut agents = AgentSystem::new();
        let mut demand = DemandSystem::new();

        let mut desirability = DesirabilitySystem::new(&config);
        let (env_w, env_h) = config.get_env_grid_size();
        for x in 0..env_w {
            for y in 0..env_h {
                desirability.grid.set(x, y, 100.0);
            }
        }
        let noise = NoiseSystem::new(&config);

        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        let orig_start = graph.edge(0).start_node;
        let orig_end = graph.edge(0).end_node;

        // Zone columns 1–8 only (skip 0 and 9). The allocator uses a zigzag scan that visits
        // column 0 and column 9 first. Both are 5 m from their respective endpoints, which is
        // below MIN_FRONTAGE_DISTANCE (8 m), causing the frontage to snap to the original node
        // rather than inserting a new one. Columns 1–8 are each ≥ 15 m from the nearest
        // endpoint, safely beyond the snap threshold.
        for dx in 1..9 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, dx, dy, ZoneType::Residential, &graph);
            }
        }
        demand.residential = 500.0;

        // First tick: spawns the building closest to the start.
        allocator.tick(
            &mut demand, &mut zoning, &desirability, &noise,
            &mut agents, &mut net, &mut graph, &config,
        );
        assert_eq!(allocator.buildings.len(), 1, "First building should have spawned");
        let b1_fn = allocator.buildings[0].frontage_node;

        // Second tick: spawns on the second half-edge created by the first split.
        allocator.tick(
            &mut demand, &mut zoning, &desirability, &noise,
            &mut agents, &mut net, &mut graph, &config,
        );
        assert_eq!(allocator.buildings.len(), 2, "Second building should have spawned");
        let b2_fn = allocator.buildings[1].frontage_node;

        // Each building must have its own distinct frontage node.
        assert_ne!(b1_fn, b2_fn, "Buildings should have different frontage nodes");

        // Neither frontage node is the original road endpoint — they are real mid-edge nodes.
        assert_ne!(b1_fn, orig_start, "b1 frontage should not be orig start_node");
        assert_ne!(b1_fn, orig_end,   "b1 frontage should not be orig end_node");
        assert_ne!(b2_fn, orig_start, "b2 frontage should not be orig start_node");
        assert_ne!(b2_fn, orig_end,   "b2 frontage should not be orig end_node");

        // Verify the nodes are reachable from each other via CCH.
        let cch = &net.cch_graph;
        assert!(
            cch.find_path(b1_fn, b2_fn, usize::MAX, &graph,
                crate::simulation::network::types::TransitFlags::FOOT).is_some(),
            "frontage nodes should be connected via road network"
        );
    }

    #[test]
    fn test_wide_road_arrival() {
        let config = MapConfig::default();
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        register_test_residential(&mut allocator);
        let mut agents = AgentSystem::new();
        let mut demand = DemandSystem::new();

        let mut desirability = DesirabilitySystem::new(&config);
        let (env_w, env_h) = config.get_env_grid_size();
        for x in 0..env_w {
            for y in 0..env_h {
                desirability.grid.set(x, y, 100.0);
            }
        }
        let noise = NoiseSystem::new(&config);

        // 4-lane road (14m wide)
        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            2,
            2,
            EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // Spawn building at t=0.5
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, 3 + dx, dy, ZoneType::Residential, &graph);
            }
        }
        demand.residential = 500.0;
        allocator.tick(
            &mut demand,
            &mut zoning,
            &desirability,
            &noise,
            &mut agents,
            &mut net,
            &mut graph,
            &config,
        );

        // Rebuild lane system and CCH after the allocator's frontage split.
        net.lane_system.rebuild(&mut graph);
        net.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&graph);

        let b_idx = 0;
        let b_edge_idx = allocator.buildings[b_idx].edge_idx;
        let b_frontage_t = allocator.buildings[b_idx].frontage_t;

        // Find a forward vehicle lane on the building's edge (any lane offset works — the
        // midway arrival check uses progress_ratio * physical_len, so offset doesn't affect it).
        let fwd_veh_lane = *net.lane_system.edge_lanes[&b_edge_idx]
            .iter()
            .find(|&&lid| {
                let l = &net.lane_system.lanes[lid];
                l.is_fwd && l.lane_type == crate::simulation::network::lanes::LaneType::Vehicle
            })
            .expect("forward vehicle lane on building edge");

        let b_edge = graph.edge(b_edge_idx);
        let physical_len = b_edge.physical_length;
        let frontage_dist = b_frontage_t * physical_len;

        // Setup agent on the road part of the building's edge.
        let agent_idx = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
        agents.target_building[agent_idx] = b_idx;
        agents.current_edge[agent_idx] = b_edge_idx;
        agents.current_node[agent_idx] = b_edge.start_node;
        agents.transit[agent_idx] = TRANSIT_ON_ROAD;
        agents.transit_mode[agent_idx] = MODE_CAR;
        agents.speed[agent_idx] = 20.0; // 20 m/s so a single tick moves 0.32 m
        agents.current_lane_id[agent_idx] = fwd_veh_lane;
        agents.current_path[agent_idx] = vec![b_edge.start_node, b_edge.end_node];
        agents.current_path_index[agent_idx] = 1;
        agents.target_node[agent_idx] = b_edge.end_node;

        // Place agent just behind the frontage so one tick (0.32 m) crosses it.
        agents.lane_distance[agent_idx] = frontage_dist - 0.2;

        agents.tick(&mut allocator, &net, &mut graph, 0.016);

        assert_eq!(
            agents.transit[agent_idx], TRANSIT_ARRIVING,
            "Agent should have arrived via projected distance check"
        );
    }
}
