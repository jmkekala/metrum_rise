// SPDX-License-Identifier: GPL-3.0-or-later

//! `PARK-25` long ways driven: the two lots of `drive_lines.rs`, every place given an
//! eight-point way with six bends (a jog out to x -8.5 on the gate line, a kink on the row's
//! line), published as the lot reconcile publishes a lot. A resident leaves one lot and parks in
//! the other along those ways, a save made on the way in continues tick for tick, a format 125
//! save loads, and a way changed under a pulling car ends its manoeuvre without stranding it.

use super::*;
use crate::simulation::parking::DockRoute;

// Building `index`'s lot-frame point in the world.
fn world(core: &SimCore, index: usize, p: Vector2) -> [f32; 2] {
    let b = &core.allocator.buildings[index];
    let (bx, bz) = building_local_xz_basis(b.facing_dir, [0.0, 0.0, 1.0]);
    let w = Vector2::new(b.center_x, b.center_y) + bx * p.x + bz * p.y;
    [w.x, w.y]
}

// The jog's far point, in the lot's frame. The four-point path keeps to x -7 and east of it, so a
// car west of x -7.5 is on the jog.
const JOG: Vector2 = Vector2::new(-8.5, 5.5);

// Place `anchor`'s long way in building `index`: its road connection, down the gate line with a
// jog out to x -8.5 and back, along its row's line with a 0.3 m kink half way, and in.
fn long_way(core: &SimCore, index: usize, anchor: u32) -> Vec<[f32; 2]> {
    let building = core.allocator.building_handles.handle(index).to_bits();
    let (_, [x, z], forward) = PLACES.into_iter().find(|p| p.0 == anchor).unwrap();
    let connection = core
        .parking
        .source_bays(PlaceSource::Building { building, anchor })[0]
        .1
        .path[0];
    let row = z - forward * 2.5;
    let inner = [
        Vector2::new(-7.0, 7.0),
        JOG,
        Vector2::new(-7.0, 4.0),
        Vector2::new(-7.0, row),
        Vector2::new((x - 7.0) / 2.0, row + 0.3),
        Vector2::new(x, row),
        Vector2::new(x, z + forward * 2.5),
    ];
    std::iter::once(connection)
        .chain(inner.map(|p| world(core, index, p)))
        .collect()
}

fn publish_long_ways(core: &mut SimCore, index: usize) {
    let ways: Vec<_> = PLACES
        .iter()
        .map(|&(anchor, ..)| {
            let way = DockRoute::new(&long_way(core, index, anchor)).unwrap();
            (anchor, way)
        })
        .collect();
    core.publish_long_car_ways(index, &ways);
}

// The drive-lines city, two lots 900 m apart on one road, every place on its long way, and a
// resident sent from one to the other: (city, agent, home, target).
fn city() -> (SimCore, usize, usize, usize) {
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
    for b in 0..2 {
        publish_long_ways(&mut c, b);
        let building = c.allocator.building_handles.handle(b).to_bits();
        for (anchor, ..) in PLACES {
            let place = c
                .parking
                .source_bays(PlaceSource::Building { building, anchor })[0]
                .0;
            let place = crate::simulation::core::handles::Handle::from_bits(place);
            assert_eq!(c.parking.long_way(place), Some(long_way(&c, b, anchor)));
        }
    }
    assert_eq!(c.parking.check_car_ways(), Ok(8 * 8));
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

// Inside a lot on every tick of both manoeuvres, the car keeps within 5 cm of its place's long
// way and out of every other place, and passes the jog's far point; the time doors sort a place
// by is its way's stored length at driveway speed. On the way in, past the jog, a save
// continues tick for tick, with the way loaded beside its row.
#[test]
fn a_car_parks_and_leaves_along_long_ways() {
    let (mut c, agent, home, target) = city();
    let (mut jogs, mut saved) = ([false; 2], false);
    let mut timed = false;
    for _ in 0..12_000 {
        tick(&mut c);
        let trip = c.agents.parking_trip[agent];
        let (place, building, slot) = match trip.phase {
            ParkingTripPhase::PullOut => (trip.source, home, 0),
            ParkingTripPhase::PullIn => (trip.destination, target, 1),
            _ => {
                if jogs[1] && c.agents.current_building[agent] == target {
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
        assert!(
            off < 0.05,
            "{:?} {off} m off its way at {car:?}",
            trip.phase
        );
        jogs[slot] |= car.x < -7.5;
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
        // Past the jog on the way in, a save continues tick for tick.
        if slot == 1 && jogs[1] && !saved && car.y < 4.0 {
            saved = true;
            let file = temp_save_path("long-ways-pull-in");
            let text = file.to_str().unwrap();
            c.save_game_settled_at(text, None, 0).unwrap();
            let live = trace(&mut c, agent);
            c.load_game_internal(text).unwrap();
            assert_eq!(
                c.parking.long_way(place),
                Some(way.iter().map(|p| [p.x, p.y]).collect())
            );
            assert_eq!(c.parking.check_car_ways(), Ok(8 * 8));
            assert_eq!(trace(&mut c, agent), live);
            std::fs::remove_file(file).unwrap();
        }
    }
    assert_eq!(jogs, [true, true], "it took the jog leaving and arriving");
    assert!(saved);
    assert_eq!(c.agents.current_building[agent], target, "it arrived");
}

// A save of format 125, which could hold no long way, loads its rows as saved.
#[test]
fn a_format_125_save_loads() {
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
    c.publish_pending_building_site_changes();
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

// The target lot republished without long ways while the car pulls in past the jog: the car
// is on no leg of the new path, so its manoeuvre ends at the place, and it walks on to the
// door. The freed ways leave the pool consistent.
#[test]
fn a_way_changed_under_a_pulling_car_ends_its_manoeuvre() {
    let (mut c, agent, _, target) = city();
    let mut changed = false;
    for _ in 0..12_000 {
        tick(&mut c);
        let trip = c.agents.parking_trip[agent];
        if !changed && trip.phase == ParkingTripPhase::PullIn {
            let at = Vector2::new(c.agents.pos_x[agent], c.agents.pos_y[agent]);
            if local(&c, target, at).x < -7.5 {
                c.publish_long_car_ways(target, &[]);
                assert!(c.parking.long_way(trip.destination).is_none());
                assert_eq!(c.parking.check_car_ways(), Ok(8 * 8));
                changed = true;
            }
        }
        if changed && c.agents.current_building[agent] == target {
            break;
        }
    }
    assert!(changed, "it reached the jog");
    assert_eq!(c.agents.current_building[agent], target, "it arrived");
}
