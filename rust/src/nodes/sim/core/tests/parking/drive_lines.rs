// SPDX-License-Identifier: GPL-3.0-or-later

//! `PARK-24` drive-line routes driven: a lot whose places no straight path from its gate reaches
//! (an island stands across every diagonal), each place routed down the gate line, along its
//! row's line and in. A resident leaves one such lot and parks in another; on every tick inside
//! a lot the car keeps to its place's proven route and to no other place, it takes both bends, and
//! a save made while it pulls in continues tick for tick.

use super::*;
use crate::simulation::buildings::allocator::building_local_xz_basis;
use crate::simulation::parking::{ParkingTripPhase, PlaceSource};
use godot::prelude::Vector2;

// A 20 m lot paved whole, a 3 m car Driveway at its left, an island of planters 4 m by 5 m in the
// middle of its street half, and two rows of two places: one nose to the back (entries at z -2,
// its line at z 0.5) and one nose to the street (entries at z 4.95, its line at z 2.45).
const LOT: &str = r#"
asset_id = "building.parking.drive_lines"
display_name = "Two rows behind an island"

[building]
placement_mode = "explicit"
service_class = "parking"
lot_width_cells = 2
lot_depth_cells = 2
frontage_forward = [0, 0, 1]
worker_capacity = 0
household_capacity = 4
window_brightness = 0

[[mesh_parts]]
name = "island"
position = [-2, 0, 7.45]
scale = 1
[[mesh_parts.lods]]
file = "island.glb"
distance_min_m = 0

[[anchors]]
type = "entrance"
name = "main"
position = [0, 0, 10.0]
forward = [0, 0, 1]

[[anchors]]
type = "driveway"
position = [-7, 0, 10.0]
forward = [0, 0, -1]
width_m = 3
vehicle_class = "car"

[[anchors]]
type = "parking"
position = [2.5, 0, -2.0]
forward = [0, 0, -1]
width_m = 2.5
length_m = 5

[[anchors]]
type = "parking"
position = [5, 0, -2.0]
forward = [0, 0, -1]
width_m = 2.5
length_m = 5

[[anchors]]
type = "parking"
position = [2.5, 0, 4.95]
forward = [0, 0, 1]
width_m = 2.5
length_m = 5

[[anchors]]
type = "parking"
position = [5, 0, 4.95]
forward = [0, 0, 1]
width_m = 2.5
length_m = 5

[[site_surfaces]]
material = "asphalt"
y_m = 0.01
vertices = [[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]]
"#;

// The parking anchors' entry edges and forwards in the lot's frame, by anchor index.
const PLACES: [(u32, [f32; 2], f32); 4] = [
    (2, [2.5, -2.0], -1.0),
    (3, [5.0, -2.0], -1.0),
    (4, [2.5, 4.95], 1.0),
    (5, [5.0, 4.95], 1.0),
];

fn register(core: &mut SimCore) -> String {
    let mut manifest = AssetManifest::from_str(LOT).unwrap();
    manifest.mesh_parts[0].imported_bounds = Some([[-2.0, 0.0, -2.5], [2.0, 1.0, 2.5]]);
    let id = manifest.qualified_id("parking");
    core.allocator
        .registry
        .register("parking", manifest, String::new());
    id
}

// Building `index`'s world point in its lot's frame.
fn local(core: &SimCore, index: usize, p: Vector2) -> Vector2 {
    let b = &core.allocator.buildings[index];
    let (bx, bz) = building_local_xz_basis(b.facing_dir, [0.0, 0.0, 1.0]);
    let d = p - Vector2::new(b.center_x, b.center_y);
    Vector2::new(d.dot(bx), d.dot(bz))
}

fn segment_distance(p: Vector2, a: Vector2, b: Vector2) -> f32 {
    let along = b - a;
    let t = ((p - a).dot(along) / along.length_squared()).clamp(0.0, 1.0);
    p.distance_to(a + along * t)
}

fn tick(c: &mut SimCore) {
    crate::thread_pools::install(|| {
        c.agents.tick_with_parking(
            &c.allocator,
            &mut c.transit_network,
            &mut c.region_graph,
            0.1,
            &c.time,
            &mut c.parking,
        )
    });
    c.schedule_parking_retries();
    c.schedule_stored_parking_arrivals();
}

#[test]
fn a_car_parks_and_leaves_along_the_rows_of_a_lot_no_straight_path_reaches() {
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 1_000_000.0;
    commit(&mut c, (-600.0, 0.0), (600.0, 0.0));
    let id = register(&mut c);
    c.place_service_building_internal(&id, -450.0, -30.0)
        .unwrap();
    c.place_service_building_internal(&id, 450.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    // Every place is published, routed down the gate line (x -7) to its row's line and in.
    for b in 0..2 {
        let building = c.allocator.building_handles.handle(b).to_bits();
        assert!(
            c.parking_rejections(b).is_empty(),
            "{:?}",
            c.parking_rejections(b)
        );
        for (anchor, [x, z], forward) in PLACES {
            let bays = c
                .parking
                .source_bays(PlaceSource::Building { building, anchor });
            assert_eq!(bays.len(), 1, "anchor {anchor}");
            let path = bays[0]
                .1
                .path
                .map(|[x, z]| local(&c, b, Vector2::new(x, z)));
            let row = z - forward * 2.5;
            let expected = [
                Vector2::new(-7.0, row),
                Vector2::new(x, row),
                Vector2::new(x, z + forward * 2.5),
            ];
            for (p, e) in path[1..].iter().zip(expected) {
                assert!((*p - e).length() < 2e-3, "anchor {anchor}: {path:?}");
            }
        }
    }
    let eastbound = c.allocator.entrances[0].side == -1;
    let (home, target) = if eastbound { (0, 1) } else { (1, 0) };
    let agent = c.debug_add_parking_commuter(home, target).unwrap();
    c.agents.release_departure(agent);
    let (mut bends, mut pulled_in, mut saved) = ([false; 2], false, false);
    for _ in 0..12_000 {
        tick(&mut c);
        let trip = c.agents.parking_trip[agent];
        let (place, building, slot) = match trip.phase {
            ParkingTripPhase::PullOut => (trip.source, home, 0),
            ParkingTripPhase::PullIn => (trip.destination, target, 1),
            _ => {
                if pulled_in && c.agents.current_building[agent] == target {
                    break;
                }
                continue;
            }
        };
        pulled_in |= slot == 1;
        let at = Vector2::new(c.agents.pos_x[agent], c.agents.pos_y[agent]);
        let car = local(&c, building, at);
        // Off the road: inside the lot, short of its street edge.
        if car.y > 9.9 {
            continue;
        }
        let path = c
            .parking
            .dock_route(place)
            .unwrap()
            .map(|[x, z]| Vector2::new(x, z));
        let off = path
            .windows(2)
            .map(|leg| segment_distance(at, leg[0], leg[1]))
            .fold(f32::MAX, f32::min);
        assert!(
            off < 0.05,
            "{:?} {off} m off its route at {car:?}",
            trip.phase
        );
        // On the row's line between its two bends: it turned off the gate line.
        let between = at.distance_to(path[1]).min(at.distance_to(path[2])) > 0.1;
        if between && segment_distance(at, path[1], path[2]) < 0.05 {
            bends[slot] = true;
        }
        for (anchor, [x, z], forward) in PLACES {
            let centre = Vector2::new(x, z + forward * 2.5);
            let inside = (car.x - centre.x).abs() < 1.25 && (car.y - centre.y).abs() < 2.5;
            let own = c
                .parking
                .source_bays(PlaceSource::Building {
                    building: c.allocator.building_handles.handle(building).to_bits(),
                    anchor,
                })
                .first()
                .is_some_and(|&(h, _)| h == place.to_bits());
            assert!(
                !inside || own,
                "{:?}: the car crosses anchor {anchor} at {car:?}",
                trip.phase
            );
        }
        // Once on the row's line, a save continues tick for tick.
        if slot == 1 && bends[1] && !saved {
            saved = true;
            let file = temp_save_path("drive-lines-pull-in");
            let text = file.to_str().unwrap();
            c.save_game_settled_at(text, None, 0).unwrap();
            let trace = |c: &mut SimCore| {
                (0..30)
                    .map(|_| {
                        tick(c);
                        (
                            c.agents.pos_x[agent].to_bits(),
                            c.agents.pos_y[agent].to_bits(),
                            format!("{:?}", c.agents.parking_trip[agent]),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let live = trace(&mut c);
            c.load_game_internal(text).unwrap();
            assert_eq!(trace(&mut c), live);
            std::fs::remove_file(file).unwrap();
        }
    }
    assert_eq!(
        bends,
        [true, true],
        "it turned off the gate line leaving and arriving"
    );
    assert!(saved && pulled_in);
    assert_eq!(c.agents.current_building[agent], target, "it arrived");
}
