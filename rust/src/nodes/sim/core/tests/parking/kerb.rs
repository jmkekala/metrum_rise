// SPDX-License-Identifier: GPL-2.0-only

//! PARK-05 kerbside bays: layout along the parking lane, driveway gaps, edits and saves.

use super::*;
use crate::simulation::network::graph::RoadParking;
use crate::simulation::parking::kerb::KERB_BAY_LENGTH_M;
use crate::simulation::parking::{Bay, PlaceSource};

fn kerb_core(parking: RoadParking) -> (SimCore, usize) {
    let mut core = SimCore::new(
        WorldConfig::new(512.0, 512.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    core.treasury.balance = 1_000_000.0;
    commit_with(&mut core, (-120.0, 0.0), (120.0, 0.0), parking);
    let edge = (0..core.region_graph.edge_count())
        .find(|&e| core.region_graph.get_edge(e).is_some_and(|e| !e.deleted))
        .unwrap();
    (core, edge)
}

fn side(c: &SimCore, edge: usize, right: bool) -> Vec<Bay> {
    c.parking
        .source_bays(PlaceSource::Kerb {
            edge: edge as u32,
            right,
        })
        .into_iter()
        .map(|(_, bay)| bay)
        .collect()
}

#[test]
fn kerbside_bays_line_the_parking_lane_end_to_end_clear_of_the_road_ends() {
    let (c, edge) = kerb_core(RoadParking {
        left: false,
        right: true,
    });
    assert!(side(&c, edge, false).is_empty());
    let bays = side(&c, edge, true);
    assert!(bays.len() >= 30, "{} bays", bays.len());
    assert_eq!(c.parking.counts().0, bays.len());
    for pair in bays.windows(2) {
        let gap = (pair[1].position[0] - pair[0].position[0])
            .hypot(pair[1].position[2] - pair[0].position[2]);
        assert!((gap - KERB_BAY_LENGTH_M).abs() < 1e-3, "{gap}");
        assert_eq!(pair[0].heading, pair[1].heading);
        assert_eq!(pair[0].position[2], pair[1].position[2]);
    }
    for bay in &bays {
        assert!(bay.position[0].abs() + 0.5 * KERB_BAY_LENGTH_M <= 115.0 + 1e-3);
        assert_eq!(bay.path[3], [bay.position[0], bay.position[2]]);
    }
    // Half a parking lane in from the kerb of a 2 × 3.5 m road with one 2.5 m parking lane:
    // 9.5 m wide, so 4.75 - 1.25 m off its centreline.
    let edge_z = c.region_graph.get_edge(edge).unwrap().physical_geometry[0].z;
    let offset = (bays[0].position[2] - edge_z).abs();
    assert!((offset - 3.5).abs() < 0.05, "{offset}");
}

#[test]
fn kerbside_bays_leave_a_lot_driveway_clear_and_close_the_gap_when_it_goes() {
    let (mut c, edge) = kerb_core(RoadParking {
        left: true,
        right: true,
    });
    let full = [side(&c, edge, false), side(&c, edge, true)];
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    let gapped = [side(&c, edge, false), side(&c, edge, true)];
    let lot_side = usize::from(gapped[1].len() < full[1].len());
    assert!(gapped[lot_side].len() < full[lot_side].len());
    assert_eq!(gapped[1 - lot_side], full[1 - lot_side]);
    // The 6 m driveway at x = 0, a metre either side, then half a bay to the nearest centre.
    let nearest = gapped[lot_side]
        .iter()
        .map(|bay| bay.position[0].abs())
        .fold(f32::INFINITY, f32::min);
    assert!(
        nearest >= 3.0 + 1.0 + 0.5 * KERB_BAY_LENGTH_M - 1e-3,
        "{nearest}"
    );
    let kerb = gapped[0].len() + gapped[1].len();
    assert_eq!(c.parking.counts().0, LOT_BAYS + kerb);
    assert!(c.bulldoze_building(0));
    c.publish_pending_building_site_changes();
    assert_eq!([side(&c, edge, false), side(&c, edge, true)], full);
}

#[test]
fn kerbside_bays_leave_a_drive_clear_opposite_a_building_without_driveway_anchors() {
    let (mut c, edge) = kerb_core(RoadParking {
        left: true,
        right: true,
    });
    let full = [side(&c, edge, false), side(&c, edge, true)];
    // The small lot's frame with only its main entrance (x = 0 at the frontage) left of its
    // anchors: no driveway, no bays.
    let toml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../godot/bootstrap/mods/parking/assets/building.parking.small_lot/asset.toml"
    ))
    .replace(
        "building.parking.small_lot",
        "building.parking.kerb_test_shop",
    );
    let toml = toml
        .split("\n[[")
        .filter(|block| !block.starts_with("anchors]]") || block.contains("type = \"entrance\""))
        .collect::<Vec<_>>()
        .join("\n[[");
    let id = register_lot_toml(&mut c, &toml);
    c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    let gapped = [side(&c, edge, false), side(&c, edge, true)];
    let near = usize::from(gapped[1].len() < full[1].len());
    assert!(gapped[near].len() < full[near].len());
    assert_eq!(gapped[1 - near], full[1 - near]);
    // A 3 m drive at x = 0 and a metre either side: the nearest bay centre is at least half a
    // bay past it and less than a whole bay further.
    let nearest = gapped[near]
        .iter()
        .map(|bay| bay.position[0].abs())
        .fold(f32::INFINITY, f32::min);
    let clear = 1.5 + 1.0 + 0.5 * KERB_BAY_LENGTH_M;
    assert!(
        (clear - 1e-3..clear + KERB_BAY_LENGTH_M).contains(&nearest),
        "{nearest}"
    );
}

#[test]
fn kerbside_bays_keep_their_cars_through_a_save_and_go_with_their_road() {
    let (mut c, edge) = kerb_core(RoadParking {
        left: true,
        right: false,
    });
    let bays = side(&c, edge, false);
    assert!(!bays.is_empty());
    c.parking.fill_test_fleet();
    let counts = c.parking.counts();
    let before = c.parking.save_json(|s| s).unwrap();
    let path = temp_save_path("parking-kerb");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    assert_eq!(c.parking.counts(), counts);
    assert_eq!(c.parking.save_json(|s| s).unwrap(), before);
    assert!(c.bulldoze_road_edge_for_test(edge));
    assert_eq!(c.parking.counts().0, 0);
    assert_eq!(c.parking.counts().2, counts.1);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_parking_lane_turned_on_in_place_publishes_its_bays_and_undo_takes_them_away() {
    let (mut c, edge) = kerb_core(RoadParking::NONE);
    assert_eq!(c.parking.counts().0, 0);
    let right = RoadParking {
        left: false,
        right: true,
    };
    assert!(c.set_edge_cross_section_internal(
        edge as i32,
        1,
        1,
        None,
        right,
        crate::simulation::network::road_types::RoadAttributes::DEFAULT,
    ));
    c.finalize_network_render_products();
    assert_eq!(c.region_graph.get_edge(edge).unwrap().parking, right);
    let bays = side(&c, edge, true);
    assert!(bays.len() >= 30, "{} bays", bays.len());
    assert_eq!(c.parking.counts().0, bays.len());
    assert!(c.undo_action_internal());
    c.finalize_network_render_products();
    assert_eq!(
        c.region_graph.get_edge(edge).unwrap().parking,
        RoadParking::NONE
    );
    assert_eq!(c.parking.counts().0, 0);
}

// `PARK-10`: a stroke splits the road beside a home with no lot of its own and the undo takes the
// split back, through the undo command's two calls. The buildings return to the restored road
// before their parking is read, so a lot in reach keeps its bays and takes the car at once; with
// none the car waits off the map and, with no border to drive in from, is towed to a free bay.
#[test]
fn an_undone_road_commit_keeps_its_bays_and_its_kerb_cars_stand_or_come_back() {
    for with_lot in [true, false] {
        let mut c = SimCore::new(
            WorldConfig::new(1024.0, 1024.0, 16.0, 10.0)
                .with_chunking(128.0, 0.0)
                .with_terrain_resolution(2.0),
        );
        c.treasury.balance = 10_000_000.0;
        let both = RoadParking {
            left: true,
            right: true,
        };
        commit_with(&mut c, (-300.0, 0.0), (300.0, 0.0), both);
        let toml = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../godot/bootstrap/mods/parking/assets/building.parking.small_lot/asset.toml"
        ))
        .replace("building.parking.small_lot", "building.parking.kerb_test_home")
        .split("\n[[")
        .filter(|block| !block.starts_with("anchors]]") || block.contains("type = \"entrance\""))
        .collect::<Vec<_>>()
        .join("\n[[");
        let home_id = register_lot_toml(&mut c, &toml);
        let mut manifest = c.allocator.registry.get(&home_id).unwrap().manifest.clone();
        manifest.building.as_mut().unwrap().household_capacity = Some(2);
        c.allocator.registry.register("parking", manifest, String::new());
        let home = c.place_service_building_internal(&home_id, 150.0, -30.0).unwrap();
        c.publish_pending_building_site_changes();
        let agent = c.debug_add_parking_commuter(home, home).unwrap();
        let car = c.agents.parking_trip[agent].vehicle;
        let kerb = c.parking.vehicle_place(car).unwrap();
        assert!(
            c.parking.source_bays(PlaceSource::Kerb { edge: 0, right: true }).iter()
                .chain(&c.parking.source_bays(PlaceSource::Kerb { edge: 0, right: false }))
                .any(|&(bay, _)| bay == kerb.to_bits()),
            "the fixture's car stands at the kerb"
        );
        if with_lot {
            let lot = register_lot(&mut c);
            c.place_service_building_internal(&lot, 210.0, -30.0).unwrap();
            c.publish_pending_building_site_changes();
        }
        let bays = c.parking.counts().0;
        c.time.speed_multiplier = 8.0;
        let run = |c: &mut SimCore| {
            for _ in 0..240 {
                c.step_frame(1.0 / 60.0);
            }
        };
        commit(&mut c, (40.0, 0.0), (40.0, 200.0));
        run(&mut c);
        assert!(c.parking.vehicle_place(car).is_some(), "with_lot={with_lot}: after the commit");

        assert!(c.undo_action_internal());
        if with_lot {
            assert_eq!(c.parking.counts().0, bays, "the undo keeps the lot");
            assert_eq!(c.parking.counts().2, 0, "the car moved straight to a bay in reach");
        }
        c.finalize_network_render_products();
        assert_eq!(c.allocator.buildings[home].edge_idx, 0, "with_lot={with_lot}");
        assert_eq!(c.parking.counts().0, bays, "with_lot={with_lot}");
        assert!(c.parking.first_free_candidate(
            c.allocator.building_handles.handle(home).to_bits()
        ).is_some());
        run(&mut c);
        assert!(c.parking.vehicle_place(car).is_some(), "with_lot={with_lot}: after the undo");
        assert_eq!(c.parking.counts().2, 0, "with_lot={with_lot}");

        let before = c.parking.save_json(|s| s).unwrap();
        let path = temp_save_path("parking-undone-commit");
        c.save_game_settled_at(path.to_str().unwrap(), None, 0).unwrap();
        c.load_game_internal(path.to_str().unwrap()).unwrap();
        assert_eq!(c.parking.save_json(|s| s).unwrap(), before);
        std::fs::remove_file(path).unwrap();
    }
}

// D4 per side, through the two edit paths: a stroke and an in-place change, the side with the
// sidewalk accepted and the shoulder side refused, and a sidewalk cannot go from under its lane.
#[test]
fn parking_needs_a_sidewalk_on_its_own_side_on_a_stroke_and_in_place() {
    use crate::simulation::network::graph::SidewalkSides;
    use crate::simulation::network::road_types::RoadAttributes;
    let left = RoadParking {
        left: true,
        right: false,
    };
    let right = RoadParking {
        left: false,
        right: true,
    };
    let attributes = |sidewalks| RoadAttributes {
        sidewalks,
        ..RoadAttributes::DEFAULT
    };
    let stroke = |core: &mut SimCore, parking, sidewalks, z: f32| {
        core.precompute_road_mesh_data();
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        core.commit_road(
            RoadCommitRequest {
                points: vec![Vector3::new(-120.0, 0.0, z), Vector3::new(120.0, 0.0, z)],
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
                preview: None,
                tunnel: false,
                median: None,
                parking,
                attributes: attributes(sidewalks),
            },
            Instant::now(),
            RoadEditMetrics::default(),
        )
    };
    let (mut c, edge) = kerb_core(RoadParking::NONE);
    let refused = stroke(&mut c, right, SidewalkSides::LEFT, 80.0);
    assert!(!refused.committed);
    assert_eq!(refused.rejection_reason, "layout_parking_needs_sidewalk");
    assert!(stroke(&mut c, left, SidewalkSides::LEFT, 80.0).committed);
    let refusal = |c: &mut SimCore, parking, sides| {
        c.preview_edge_cross_section_internal(edge as i32, 1, 1, None, parking, attributes(sides))
            .err()
            .map(|refusal| refusal.reason)
    };
    assert_eq!(
        refusal(&mut c, right, SidewalkSides::LEFT).as_deref(),
        Some("layout_parking_needs_sidewalk")
    );
    assert_eq!(refusal(&mut c, right, SidewalkSides::RIGHT), None);
    assert!(c.set_edge_cross_section_internal(
        edge as i32,
        1,
        1,
        None,
        right,
        RoadAttributes::DEFAULT
    ));
    assert_eq!(
        refusal(&mut c, right, SidewalkSides::NONE).as_deref(),
        Some("layout_parking_needs_sidewalk"),
        "the sidewalk stays while its parking lane does"
    );
}

// D3 in the in-place path: the Apply charge of a parking lane is its 0.2 weight over the road.
#[test]
fn an_in_place_parking_lane_charges_its_weight() {
    let (mut c, edge) = kerb_core(RoadParking::NONE);
    let right = RoadParking {
        left: false,
        right: true,
    };
    let verdict = c
        .preview_edge_cross_section_internal(
            edge as i32,
            1,
            1,
            None,
            right,
            crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        )
        .unwrap();
    let length = f64::from(c.region_graph.get_edge(edge).unwrap().physical_length);
    let expected = 0.2 * crate::simulation::network::road_types::cost::BUILD_COST_PER_M * length;
    assert!(
        (verdict.cost_delta - expected).abs() < 1e-6,
        "{}",
        verdict.cost_delta
    );
}

// D5 through the normal edit path: a pocket added at the end of a street with kerbside bays takes
// the bays on its side off the pocket edge, tows the car parked there and keeps the far side's.
#[test]
fn a_pocket_added_later_takes_the_bays_along_it_and_undo_returns_them() {
    use crate::simulation::network::pockets::PocketRequest;
    use crate::simulation::network::roadbed::EdgeEnd;
    let both = RoadParking {
        left: true,
        right: true,
    };
    let (mut c, _) = kerb_core(both);
    commit_with(&mut c, (120.0, -120.0), (120.0, 120.0), RoadParking::NONE);
    c.finalize_network_render_products();
    let live = |c: &SimCore| -> Vec<usize> {
        (0..c.region_graph.edge_count())
            .filter(|&e| c.region_graph.get_edge(e).is_some_and(|e| !e.deleted))
            .collect()
    };
    let street = live(&c)
        .into_iter()
        .find(|&e| c.region_graph.edge(e).parking == both)
        .expect("the street");
    let end = c
        .region_graph
        .edge(street)
        .physical_geometry
        .last()
        .unwrap()
        .x
        > 0.0;
    let bays_before = c.parking.counts().0;
    c.parking.fill_test_fleet();
    let cars = c.parking.counts().1;
    let end = if end { EdgeEnd::End } else { EdgeEnd::Start };
    let request = PocketRequest {
        edge: street,
        end,
        left: false,
        right: true,
        body_m: 40.0,
    };
    if let Err(refusal) = c.preview_road_pocket_internal(request) {
        panic!("{}: {}", refusal.reason, refusal.detail);
    }
    assert!(c.set_road_pocket_internal(request));
    c.finalize_network_render_products();
    let pocket = live(&c)
        .into_iter()
        .find(|&e| c.region_graph.edge(e).pocket.is_some())
        .expect("a pocket edge");
    let edge = c.region_graph.edge(pocket);
    assert_eq!(
        edge.parking, both,
        "the pocket edge keeps the road's section"
    );
    let right = edge.pocket_moves_kerb(true);
    assert_ne!(right, edge.pocket_moves_kerb(false), "one kerb moves");
    assert!(
        side(&c, pocket, right).is_empty(),
        "no bay along the pocket"
    );
    assert!(
        !side(&c, pocket, !right).is_empty(),
        "the far kerb keeps its own"
    );
    let counts = c.parking.counts();
    assert!(counts.0 < bays_before);
    assert_eq!(counts.1, cars, "a towed car is kept");
    assert!(
        counts.2 > 0,
        "with every bay taken, the bays' cars wait off the map"
    );
    assert!(c.undo_action_internal());
    c.finalize_network_render_products();
    assert_eq!(c.parking.counts().0, bays_before);
}
