// SPDX-License-Identifier: GPL-3.0-or-later

//! `PARK-32` rendered stills fixture: a street with four lots where commuters cross the road to
//! enter and to leave a lot, and saves at the moments the stills show.

use super::*;
use crate::simulation::economy::agents::{
    ACTIVITY_WORK, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS, TRANSIT_NETWORK,
};

// Ticks the agents and the parking retries as `taken_trips` does.
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
}

// The street (x -300 to 300) with lots at (-150, 30) and (250, 30) north of it and (150, -30)
// and (-250, -30) south of it. Commuters from the north-west lot to the south-east one enter
// across the westbound lane; commuters from the north-east lot to the south-west one leave across
// the eastbound lane and drive west past the first group's crossing. The saves: the first car
// waiting in its lane to cross with a car queued behind it (`enter.sqlite`), and a car stepping
// at the north-east lot's mouth to cross, or crossing (`leave.sqlite`), each with the spot printed.
#[test]
#[ignore = "manual fixture for PARK-32 rendered stills; requires METRUM_CROSSING_STILL_OUT"]
fn write_the_crossing_still_saves() {
    let out = std::env::var("METRUM_CROSSING_STILL_OUT").expect("METRUM_CROSSING_STILL_OUT");
    let out = std::path::Path::new(&out);
    std::fs::create_dir_all(out).unwrap();
    let mut c = SimCore::new(
        WorldConfig::new(1024.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 10_000_000.0;
    commit(&mut c, (-300.0, 0.0), (300.0, 0.0));
    let id = register_lot(&mut c);
    let mut manifest = c.allocator.registry.get(&id).unwrap().manifest.clone();
    manifest.building.as_mut().unwrap().household_capacity = Some(64);
    c.allocator
        .registry
        .register("parking", manifest, String::new());
    let lots: Vec<usize> = [
        (-150.0, 30.0),
        (250.0, 30.0),
        (150.0, -30.0),
        (-250.0, -30.0),
    ]
    .iter()
    .map(|&(x, z)| c.place_service_building_internal(&id, x, z).unwrap())
    .collect();
    c.publish_pending_building_site_changes();
    let mut commuters = |c: &mut SimCore, home: usize, work: usize, count: usize| {
        (0..count)
            .filter_map(|_| {
                let agent = c.debug_add_parking_commuter(home, work)?;
                c.agents.assign_work_building(agent, work, 0);
                c.agents.release_departure(agent);
                c.agents.agents.planned_activity[agent] = ACTIVITY_WORK;
                Some(agent)
            })
            .collect::<Vec<_>>()
    };
    let entering = commuters(&mut c, lots[0], lots[2], 6);
    let leaving = commuters(&mut c, lots[1], lots[3], 10);
    assert!(!entering.is_empty() && !leaving.is_empty());
    let at = |c: &SimCore, i: usize| (c.agents.pos_x[i], c.agents.pos_y[i]);
    let (mut entered, mut left) = (false, false);
    for step in 0..36_000 {
        let a = &c.agents.agents;
        if !entered {
            // A crosser waiting in its lane with another car on that lane less than 15 m behind.
            let waiting = entering.iter().copied().find(|&i| {
                a.transit[i] == TRANSIT_NETWORK
                    && a.crossing_wait_s[i] > 1.0
                    && entering.iter().chain(&leaving).any(|&j| {
                        j != i
                            && a.transit[j] == TRANSIT_NETWORK
                            && a.current_lane_id[j] == a.current_lane_id[i]
                            && (0.0..15.0).contains(&(a.lane_distance[i] - a.lane_distance[j]))
                    })
            });
            if let Some(i) = waiting {
                let path = out.join("enter.sqlite");
                c.save_game_settled_at(path.to_str().unwrap(), None, 0)
                    .unwrap();
                eprintln!(
                    "CROSSING_STILL enter step {step} agent {i} at {:?}",
                    at(&c, i)
                );
                entered = true;
            }
        }
        let a = &c.agents.agents;
        if !left
            && let Some(&i) = leaving.iter().find(|&&i| {
                a.transit[i] == TRANSIT_ACCESS_EGRESS
                    && (a.crossing_wait_s[i] > 0.0 || a.crossing_lane[i] != u32::MAX)
            })
        {
            let path = out.join("leave.sqlite");
            c.save_game_settled_at(path.to_str().unwrap(), None, 0)
                .unwrap();
            eprintln!(
                "CROSSING_STILL leave step {step} agent {i} at {:?}",
                at(&c, i)
            );
            left = true;
        }
        if entered && left {
            break;
        }
        tick(&mut c);
    }
    let crossed = entering
        .iter()
        .filter(|&&i| c.agents.transit[i] == TRANSIT_ACCESS_INGRESS)
        .count();
    eprintln!("CROSSING_STILL entered {entered} left {left} entering-now {crossed}");
}
