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
