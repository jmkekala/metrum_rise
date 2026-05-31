#[cfg(test)]
mod tests {
    use crate::assets::AssetManifest;
    use crate::assets::asset::{
        Anchor, AnchorType, BuildingData, LodEntry, PlacementMode, ZoneClass,
    };
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
    use crate::simulation::economy::agents::{
        ACCESS_PLAN_VALID, AgentSystem, MODE_WALK, TRANSIT_ACCESS_EGRESS, TRANSIT_NETWORK,
    };
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::lanes::LaneType;
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use crate::simulation::pathing::cch::CchGraph;
    use crate::simulation::zoning::ZoneType;
    use godot::prelude::{Vector2, Vector3};

    use crate::simulation::LANE_CONFIGS;

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
        let manifest = AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![LodEntry {
                file: "lod0.glb".to_owned(),
                distance_min_m: 0.0,
                distance_max_m: None,
            }],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
            }],
            building: Some(BuildingData {
                flat_size_m2: None,
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(zone),
                density: Some("low".to_owned()),
                lot_width_cells: 1,
                lot_depth_cells: 1,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity,
                worker_capacity,
                service_class: None,
                economy_profile: match zone {
                    ZoneClass::Commercial => Some("grocery_basic".to_owned()),
                    ZoneClass::Industrial => Some("food_processor_basic".to_owned()),
                    _ => None,
                },
                preview_scale: Some(1.0),
            }),
            prop: None,
            vehicle: None,
            character: None,
            pivot_offset: None,
        };
        allocator
            .registry
            .register(pack_id, manifest, String::new());
        format!("{pack_id}:{asset_id}")
    }

    fn create_test_edge(n0: u32, n1: u32, fwd: u8, bkw: u8) -> Edge {
        Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: (fwd as f32 + bkw as f32) * 3.5,
            fwd_lanes: fwd,
            bkw_lanes: bkw,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        }
    }

    fn create_test_building(edge_idx: usize, side: i8, asset_id: &str) -> Building {
        Building {
            center_x: 0.0,
            center_y: 0.0,
            width_cells: 1,
            depth_cells: 1,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(1.0, 0.0),
            frontage_t: 0.5, // t=0.5 → depart node = end_node of the edge
            side_offset: 5.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx,
            side,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id: asset_id.to_owned(),
            level: 1,
            broken: false,
            economy_profile_runtime_id: 0,
            economy_broken: false,
            resource_inventory: Vec::new(),
            revenue: 0.0,
            operating_budget: 500.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        }
    }

    // Shared departure-side check. Spawns an agent departing from a building on the given
    // `side` (1 = LEFT, -1 = RIGHT) and asserts it lands on the matching sidewalk lane.
    //
    // `lane_idx` for sidewalks is a fixed constant regardless of vehicle lane count:
    //   LEFT sidewalk  → lane_idx =  100
    //   RIGHT sidewalk → lane_idx = -100
    // (See lanes.rs `build_lane` calls at the sidewalk section.)
    fn check_departure_side(
        fwd: u8,
        bkw: u8,
        building_side: i8,
        expected_lane_idx: i8,
        label: &str,
    ) {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(create_test_edge(n0, n1, fwd, bkw));
        graph.rebuild_adjacency_list();

        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = CchGraph::build(&graph);

        let mut allocator = BuildingAllocator::new();
        let asset_id = register_test_asset(
            &mut allocator,
            "test",
            &format!("departure_side_{label}_{building_side}"),
            ZoneClass::Residential,
        );
        let building = create_test_building(edge_idx, building_side, &asset_id);
        allocator.buildings.push(building);
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let entrance = allocator.entrances[0].clone();
        let lane_id = [entrance.foot_lane_fwd, entrance.foot_lane_bkw]
            .into_iter()
            .find(|&lane_id| {
                lane_id != usize::MAX
                    && network.lane_system.lanes[lane_id].lane_idx == expected_lane_idx
            })
            .expect("expected exact sidewalk lane");
        let planned_attach_node = if network.lane_system.lanes[lane_id].is_fwd {
            n1
        } else {
            n0
        };
        let projected_s =
            crate::simulation::buildings::allocator::BuildingAllocator::project_point_to_polyline_s(
                &network.lane_system.lanes[lane_id].geometry,
                crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_edge(
                    &graph,
                    edge_idx,
                    entrance.entrance_s_m / graph.edge(edge_idx).physical_length,
                ),
            );

        let mut agents = AgentSystem::new();
        let a_id = agents.spawn_border_arrival_agent(0, n0, 100.0, 0.0, n0, 100.0, 0.0);
        agents.transit[a_id] = TRANSIT_ACCESS_EGRESS;
        agents.transit_mode[a_id] = MODE_WALK;
        agents.current_building[a_id] = 0;
        agents.pos_x[a_id] = entrance.door_pos.x;
        agents.pos_y[a_id] = entrance.door_pos.y;
        agents.planned_attach_node[a_id] = planned_attach_node;
        agents.planned_attach_lane_id[a_id] = lane_id as u32;
        agents.planned_attach_lane_d[a_id] = projected_s;
        agents.access_flags[a_id] = ACCESS_PLAN_VALID;

        for _ in 0..500 {
            agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
            if agents.transit[a_id] == TRANSIT_NETWORK {
                break;
            }
        }

        assert_eq!(
            agents.transit[a_id], TRANSIT_NETWORK,
            "[{label}] agent never reached ON_ROAD"
        );

        let lane_id = agents.current_lane_id[a_id];
        assert_ne!(
            lane_id,
            usize::MAX,
            "[{label}] lane attachment stayed invalid"
        );
        let lane = &network.lane_system.lanes[lane_id];

        assert_eq!(
            lane.lane_type,
            LaneType::Foot,
            "[{label}] side={building_side}: agent should be on a Foot lane, found {:?}",
            lane.lane_type
        );
        assert_eq!(
            lane.lane_idx, expected_lane_idx,
            "[{label}] side={building_side}: expected lane_idx={expected_lane_idx} but found {}",
            lane.lane_idx
        );
    }

    #[test]
    fn test_agent_departure_uses_correct_side_sidewalk() {
        for &(fwd, bkw, label) in LANE_CONFIGS {
            // LEFT building (side=1) → LEFT sidewalk (lane_idx=100)
            check_departure_side(fwd, bkw, 1, 100, label);
            // RIGHT building (side=-1) → RIGHT sidewalk (lane_idx=-100)
            check_departure_side(fwd, bkw, -1, -100, label);
        }
    }
}
