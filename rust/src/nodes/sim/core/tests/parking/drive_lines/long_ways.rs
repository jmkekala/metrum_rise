// SPDX-License-Identifier: GPL-3.0-or-later

//! `PARK-25` long ways from `PARK-26` own routes, driven: the lot of `drive_lines.rs` with every
//! place given its own route in the asset (`route_in`: down the gate line, along its row's line
//! with a 0.3 m kink half way, and in), proven and published by the lot reconcile as six-point
//! long ways. A resident leaves one lot and parks in the other along those ways, a save made on
//! the way in continues tick for tick, a format 125 save loads, and a way changed under a pulling
//! car ends its manoeuvre without stranding it. A two-waypoint own route is a plain path, and a
//! failing one rejects its place with the hint and no fallback.

use super::*;
use crate::assets::AssetManifest;

// The lot with `routes` (anchor, lot-frame waypoints) as its places' own routes, registered
// over the drive-lines asset.
fn register_routes(core: &mut SimCore, routes: &[(u32, Vec<[f32; 2]>)]) {
    let mut manifest = AssetManifest::from_str(LOT).unwrap();
    manifest.mesh_parts[0].imported_bounds = Some([[-2.0, 0.0, -2.5], [2.0, 1.0, 2.5]]);
    for (anchor, route) in routes {
        manifest.anchors[*anchor as usize].route_in = route.clone();
    }
    core.allocator
        .registry
        .register("parking", manifest, String::new());
}

// Place `anchor`'s own route: down the gate line (x -7), along its row's line with a 0.3 m kink
// half way to the place, and to the point in front of it.
fn own_route(anchor: u32) -> Vec<[f32; 2]> {
    let (_, [x, z], forward) = PLACES.into_iter().find(|p| p.0 == anchor).unwrap();
    let row = z - forward * 2.5;
    vec![[-7.0, 7.0], [-7.0, row], [(x - 7.0) / 2.0, row + 0.3], [x, row]]
}

// Building `index`'s lot-frame point in the world.
fn world(core: &SimCore, index: usize, [x, z]: [f32; 2]) -> [f32; 2] {
    let b = &core.allocator.buildings[index];
    let (bx, bz) = building_local_xz_basis(b.facing_dir, [0.0, 0.0, 1.0]);
    let w = Vector2::new(b.center_x, b.center_y) + bx * x + bz * z;
    [w.x, w.y]
}

// Republishes building `index`'s lot as a site edit does.
fn republish(core: &mut SimCore, index: usize) {
    let handle = core.allocator.building_handles.handle(index);
    core.allocator.parking_site_changes.push(handle);
    core.publish_parking_sites();
}

fn place(core: &SimCore, index: usize, anchor: u32) -> crate::simulation::core::handles::Handle<crate::simulation::parking::Place> {
    let building = core.allocator.building_handles.handle(index).to_bits();
    let place = core
        .parking
        .source_bays(PlaceSource::Building { building, anchor })[0]
        .0;
    crate::simulation::core::handles::Handle::from_bits(place)
}

// Two lots 900 m apart on one road with `routes` registered.
fn lots(routes: &[(u32, Vec<[f32; 2]>)]) -> SimCore {
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 1_000_000.0;
    commit(&mut c, (-600.0, 0.0), (600.0, 0.0));
    let id = register(&mut c);
    register_routes(&mut c, routes);
    c.place_service_building_internal(&id, -450.0, -30.0)
        .unwrap();
    c.place_service_building_internal(&id, 450.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    c
}

// The drive-lines city, every place on its own long way, and a resident sent from one lot to the
// other: (city, agent, home, target).
fn city() -> (SimCore, usize, usize, usize) {
    let routes: Vec<_> = PLACES.iter().map(|&(a, ..)| (a, own_route(a))).collect();
    let mut c = lots(&routes);
    for b in 0..2 {
        assert!(c.parking_rejections(b).is_empty(), "{:?}", c.parking_rejections(b));
        for (anchor, ..) in PLACES {
            let way = c.parking.long_way(place(&c, b, anchor)).expect("a long way");
            let connection = way[0];
            let expected: Vec<_> = std::iter::once(connection)
                .chain(own_route(anchor).into_iter().map(|p| world(&c, b, p)))
                .collect();
            assert_eq!(way.len(), 6, "anchor {anchor}");
            for (p, e) in way[..5].iter().zip(&expected) {
                assert!((p[0] - e[0]).hypot(p[1] - e[1]) < 1e-3, "anchor {anchor}: {way:?}");
            }
        }
    }
    assert_eq!(c.parking.check_car_ways(), Ok(8 * 6));
    let eastbound = c.allocator.entrances[0].side == -1;
    let (home, target) = if eastbound { (0, 1) } else { (1, 0) };
    let agent = c.debug_add_parking_commuter(home, target).unwrap();
    c.agents.release_departure(agent);
    (c, agent, home, target)
}

// Ticks the car's 30 next positions and trip rows.
fn trace(c: &mut SimCore, agent: usize) -> Vec<(u32, u32, String)> {
    (0..30)
        .map(|_| {
            tick(c);
            (
                c.agents.pos_x[agent].to_bits(),
                c.agents.pos_y[agent].to_bits(),
                format!("{:?}", c.agents.parking_trip[agent]),
            )
        })
        .collect()
}

// Whether the car at lot-frame `car` is on the kink of its way `way` (lot frame): more than
// 0.2 m off the row's line, east of the corner onto it.
fn on_kink(way: &[Vector2], car: Vector2) -> bool {
    (car.y - way[2].y).abs() > 0.2 && car.x > way[2].x + 1.0 && car.x < way[4].x
}

// Inside a lot on every tick of both manoeuvres, the car keeps within 5 cm of its place's long
// way and out of every other place, and takes the kink; the time doors sort a place by is its
// way's stored length at driveway speed. On the way in, past the kink, a save continues tick for
// tick, with the way loaded beside its row.
#[test]
fn a_car_parks_and_leaves_along_its_places_own_routes() {
    let (mut c, agent, home, target) = city();
    let (mut kinks, mut saved) = ([false; 2], false);
    let mut timed = false;
    for _ in 0..12_000 {
        tick(&mut c);
        let trip = c.agents.parking_trip[agent];
        let (place, building, slot) = match trip.phase {
            ParkingTripPhase::PullOut => (trip.source, home, 0),
            ParkingTripPhase::PullIn => (trip.destination, target, 1),
            _ => {
                if kinks[1] && c.agents.current_building[agent] == target {
                    break;
                }
                continue;
            }
        };
        let way: Vec<_> = c
            .parking
            .long_way(place)
            .expect("a long way")
            .into_iter()
            .map(|[x, z]| Vector2::new(x, z))
            .collect();
        if !timed {
            let metres: f32 = way.windows(2).map(|w| w[0].distance_to(w[1])).sum();
            let seconds = c.parking.maneuver_seconds(place);
            assert!((seconds * crate::config::AGENT_DRIVEWAY_SPEED_MS - metres).abs() < 1e-3);
            timed = true;
        }
        let at = Vector2::new(c.agents.pos_x[agent], c.agents.pos_y[agent]);
        let car = local(&c, building, at);
        if car.y > 9.9 {
            continue;
        }
        let off = way
            .windows(2)
            .map(|leg| segment_distance(at, leg[0], leg[1]))
            .fold(f32::MAX, f32::min);
        assert!(off < 0.05, "{:?} {off} m off its way at {car:?}", trip.phase);
        let local_way: Vec<_> = way.iter().map(|&p| local(&c, building, p)).collect();
        kinks[slot] |= on_kink(&local_way, car);
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
            assert!(!inside || own, "{:?}: the car crosses anchor {anchor} at {car:?}", trip.phase);
        }
        // Past the kink on the way in, a save continues tick for tick.
        if slot == 1 && kinks[1] && !saved {
            saved = true;
            let file = temp_save_path("own-routes-pull-in");
            let text = file.to_str().unwrap();
            c.save_game_settled_at(text, None, 0).unwrap();
            let live = trace(&mut c, agent);
            c.load_game_internal(text).unwrap();
            assert_eq!(
                c.parking.long_way(place),
                Some(way.iter().map(|p| [p.x, p.y]).collect())
            );
            assert_eq!(c.parking.check_car_ways(), Ok(8 * 6));
            assert_eq!(trace(&mut c, agent), live);
            std::fs::remove_file(file).unwrap();
        }
    }
    assert_eq!(kinks, [true, true], "it took the kink leaving and arriving");
    assert!(saved);
    assert_eq!(c.agents.current_building[agent], target, "it arrived");
}

// A save of format 125, which could hold no long way, loads its rows as saved.
#[test]
fn a_format_125_save_loads() {
    let mut c = lots(&[]);
    let file = temp_save_path("long-ways-format-125");
    let text = file.to_str().unwrap();
    c.save_game_settled_at(text, None, 0).unwrap();
    rusqlite::Connection::open(&file)
        .unwrap()
        .execute_batch(&format!(
            "UPDATE save_meta SET version = {};",
            crate::simulation::save::schema::PRE_CAR_WAYS_SAVE_VERSION
        ))
        .unwrap();
    let rows = c.parking.place_product_hash();
    c.load_game_internal(text).expect("format 125 loads");
    std::fs::remove_file(file).unwrap();
    assert_eq!(c.parking.place_product_hash(), rows);
    assert_eq!(c.parking.check_car_ways(), Ok(0));
}

// Back to automatic while the car pulls in past the kink: the target lot, republished without
// its own routes, gives the car no leg of the new path, so its manoeuvre ends at the place and it
// walks on to the door. The freed ways leave the pool consistent.
#[test]
fn a_way_changed_under_a_pulling_car_ends_its_manoeuvre() {
    let (mut c, agent, _, target) = city();
    let mut changed = false;
    for _ in 0..12_000 {
        tick(&mut c);
        let trip = c.agents.parking_trip[agent];
        if !changed && trip.phase == ParkingTripPhase::PullIn {
            let at = Vector2::new(c.agents.pos_x[agent], c.agents.pos_y[agent]);
            let way: Vec<_> = c.parking.long_way(trip.destination).unwrap().into_iter()
                .map(|[x, z]| local(&c, target, Vector2::new(x, z)))
                .collect();
            if on_kink(&way, local(&c, target, at)) {
                register_routes(&mut c, &[]);
                republish(&mut c, target);
                assert!(c.parking.long_way(trip.destination).is_none());
                assert_eq!(c.parking.check_car_ways(), Ok(8 * 6));
                changed = true;
            }
        }
        if changed && c.agents.current_building[agent] == target {
            break;
        }
    }
    assert!(changed, "it reached the kink");
    assert_eq!(c.agents.current_building[agent], target, "it arrived");
}

// Own routes of two waypoints and of one: the first is the place's plain four-point path (no
// long way), along the lines the automatic route takes; the second, its last leg askew, rejects
// its place with the reason and the hint, and no other route is tried.
#[test]
fn a_short_own_route_is_a_plain_path_and_a_failing_one_rejects() {
    let auto = lots(&[]);
    let mut c = lots(&[(2, vec![[-7.0, 0.5], [2.5, 0.5]]), (3, vec![[-7.0, 0.5]])]);
    let path = c.parking.source_bays(PlaceSource::Building {
        building: c.allocator.building_handles.handle(0).to_bits(),
        anchor: 2,
    })[0]
        .1
        .path;
    let automatic = auto.parking.source_bays(PlaceSource::Building {
        building: auto.allocator.building_handles.handle(0).to_bits(),
        anchor: 2,
    })[0]
        .1
        .path;
    for (p, a) in path.iter().zip(automatic) {
        assert!((p[0] - a[0]).hypot(p[1] - a[1]) < 1e-4, "{path:?} {automatic:?}");
    }
    assert!(c.parking.long_way(place(&c, 0, 2)).is_none());
    assert_eq!(c.parking.check_car_ways(), Ok(0));
    let building = c.allocator.building_handles.handle(0).to_bits();
    assert!(c.parking.source_bays(PlaceSource::Building { building, anchor: 3 }).is_empty());
    let reasons = c.parking_rejections(0).into_owned();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(
        reasons[0].contains("parking anchor 3: the way into the spot from waypoint 1 runs ")
            && reasons[0].ends_with(
                "° off the spot's direction: a car drives into its spot along it, so put the last waypoint in line behind the spot (within 30°). This spot follows its own route: move its waypoints, or choose Back to automatic"
            ),
        "{reasons:?}"
    );
    republish(&mut c, 0);
    assert_eq!(c.parking_rejections(0).into_owned(), reasons);
}

// The polyline car `agent` walks in its current manoeuvre, lane point included.
fn maneuver(c: &SimCore, agent: usize) -> Vec<Vector2> {
    let trip = c.agents.parking_trip[agent];
    let egress = trip.phase == ParkingTripPhase::PullOut;
    let (place, lane, distance) = if egress {
        (trip.source, c.agents.planned_attach_lane_id[agent], c.agents.planned_attach_lane_d[agent])
    } else {
        (trip.destination, c.agents.planned_detach_lane_id[agent], c.agents.planned_detach_lane_d[agent])
    };
    c.parking
        .maneuver_path(place, lane as usize, distance, &c.transit_network, !egress, false)
        .unwrap()
        .as_slice()
        .to_vec()
}

// `PARK-29`: on every tick of both manoeuvres the snapshot draws the car on its way's line rounded
// at each bend with the car's radius, facing along it, within the drawn offset bound of where it
// is, and where it is on the straight parts. Each manoeuvre takes arcs, the snapshot's car
// instance stands at the drawn point, and the car's positions and arrival are the same tick for
// tick as in a city that builds no snapshot.
#[test]
fn a_car_is_drawn_round_its_ways_bends_and_moves_as_before() {
    use crate::nodes::sim::core::snapshot::off_lane_car_pose;
    use crate::simulation::parking::turns::{CAR_TURN_RADIUS_M, MAX_DRAWN_OFFSET_M, rounded_line};
    let (mut c, agent, _, target) = city();
    let (mut plain, twin, ..) = city();
    assert_eq!(agent, twin);
    let mut arcs = [0; 2];
    let mut checked_instance = false;
    for tick_index in 0..12_000 {
        tick(&mut c);
        tick(&mut plain);
        assert_eq!(
            (c.agents.pos_x[agent].to_bits(), c.agents.pos_y[agent].to_bits(), c.agents.transit[agent]),
            (plain.agents.pos_x[agent].to_bits(), plain.agents.pos_y[agent].to_bits(), plain.agents.transit[agent]),
            "tick {tick_index}"
        );
        assert_eq!(
            format!("{:?}", c.agents.parking_trip[agent]),
            format!("{:?}", plain.agents.parking_trip[agent])
        );
        let snapshot = c.build_snapshot();
        let phase = c.agents.parking_trip[agent].phase;
        let slot = match phase {
            ParkingTripPhase::PullOut => 0,
            ParkingTripPhase::PullIn => 1,
            _ => {
                if c.agents.current_building[agent] == target {
                    break;
                }
                continue;
            }
        };
        if c.agents.current_lane_id[agent] != usize::MAX {
            continue;
        }
        let stored = Vector2::new(c.agents.pos_x[agent], c.agents.pos_y[agent]);
        let (x, z, facing) = off_lane_car_pose(&c, agent, stored.x, stored.y);
        let drawn = Vector2::new(x, z);
        let facing = facing.map(|f| Vector2::new(f.x, f.z));
        let path = maneuver(&c, agent);
        let line = rounded_line(&path, CAR_TURN_RADIUS_M);
        let off_line = line
            .windows(2)
            .map(|leg| segment_distance(drawn, leg[0], leg[1]))
            .fold(f32::MAX, f32::min);
        // The line samples each arc every 10°: 1 cm from the circle at a 2.5 m radius.
        assert!(off_line < 0.012, "{phase:?}: drawn {drawn:?} {off_line} m off its line");
        assert!(drawn.distance_to(stored) <= MAX_DRAWN_OFFSET_M);
        if drawn.distance_to(stored) > 1e-3 {
            arcs[slot] += 1;
            // Along the line: the drawn point a little further on lies ahead of the facing.
            let facing = facing.expect("a facing");
            assert!((facing.length() - 1.0).abs() < 1e-4);
            let nearest = line
                .windows(2)
                .min_by(|a, b| {
                    segment_distance(drawn, a[0], a[1]).total_cmp(&segment_distance(drawn, b[0], b[1]))
                })
                .unwrap();
            let chord = (nearest[1] - nearest[0]).normalized();
            assert!(facing.dot(chord) > (6.0f32).to_radians().cos(), "{phase:?}: {facing:?} {chord:?}");
            if !checked_instance {
                let id = c.agents.render_id[agent] as i64;
                let (key, index) = snapshot
                    .car_render_ids
                    .iter()
                    .find_map(|(&key, ids)| ids.iter().position(|&i| i == id).map(|i| (key, i)))
                    .expect("the car's instance");
                let pose = &snapshot.car_transforms[&key][index * 12..index * 12 + 12];
                assert!(
                    Vector2::new(pose[3], pose[11]).distance_to(drawn) < 1e-3,
                    "the instance stands at the drawn point: {pose:?} {drawn:?}"
                );
                checked_instance = true;
            }
        }
        let target = crate::simulation::economy::agents::tick::parking_path_target_segment(stored, &path)
            .expect("a leg");
        if crate::simulation::parking::turns::drawn_turn(&path, target, stored, CAR_TURN_RADIUS_M).is_none() {
            // On a straight part: drawn where it is.
            assert_eq!((x.to_bits(), z.to_bits()), (stored.x.to_bits(), stored.y.to_bits()));
        }
    }
    assert!(arcs[0] > 3 && arcs[1] > 3, "both manoeuvres took arcs: {arcs:?}");
    assert!(checked_instance);
    assert_eq!(c.agents.current_building[agent], target, "it arrived");
    assert_eq!(plain.agents.current_building[agent], target);
}
