// SPDX-License-Identifier: GPL-3.0-or-later

//! The family arrival (`PARK-35`): a moving-in household steps out of its parked car at the
//! car's doors and walks to the home's door, the household forming once as the car parks.

use super::*;
use crate::simulation::economy::agents::{
    AgentHandle, HouseholdAgeComposition, MODE_WALK, TRANSIT_ACCESS_EGRESS, TRANSIT_IN_BUILDING,
};
use crate::simulation::parking::ParkingTripPhase;

// The largest household admission draws (`MAX_STARTER_IMMIGRANT_HOUSEHOLD_SIZE`): two adults,
// one elder, three children; one more than the car's five seats.
pub(super) const SIZE: u16 = 6;
pub(super) const COMPOSITION: HouseholdAgeComposition = HouseholdAgeComposition {
    adult_count: 2,
    elder_count: 1,
    child_count: 3,
};
pub(super) const FRAME_S: f64 = 1.0 / 60.0;
pub(super) const SPEED: f32 = 8.0;

// A 1.2 km road with the bundled small lot as the home at its middle, and a household of six
// leaving the border on the lot's side by car, as `parking_move_in_save_round_trip_mid_arrival`
// sets one of one out (the same carrier identity, so the same car decision).
fn arriving() -> (SimCore, AgentHandle) {
    let (mut c, border) = road_and_home();
    let pos = c.region_graph.node(border).pos;
    let i = c
        .agents
        .spawn_household_arrival_carrier(0, SIZE, COMPOSITION, border, pos.x, pos.z);
    assert!(c.agents.start_parking_move_in(
        i,
        &mut c.parking,
        &c.allocator,
        &c.transit_network,
        &c.region_graph
    ));
    assert_eq!(
        c.agents.parking_trip[i].phase,
        ParkingTripPhase::Drive,
        "the carrier drives"
    );
    c.agents.release_departure(i);
    c.households.recount_pending_arrivals(&mut c.agents);
    c.time.speed_multiplier = SPEED;
    let carrier = c.agents.agent_handle(i);
    (c, carrier)
}

// The road, the home and the border on the home's side.
pub(super) fn road_and_home() -> (SimCore, u32) {
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 1_000_000.0;
    commit(&mut c, (-600.0, 0.0), (600.0, 0.0));
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    c.publish_pending_building_site_changes();
    let border = if c.allocator.entrances[0].side == -1 {
        c.region_graph.edge(0).start_node
    } else {
        c.region_graph.edge(0).end_node
    };
    (c, border)
}

// Frames until `done`, at most `limit`; the frames taken.
pub(super) fn step_until(c: &mut SimCore, limit: usize, done: impl Fn(&SimCore) -> bool) -> usize {
    for frame in 0..limit {
        if done(c) {
            return frame;
        }
        c.step_frame(FRAME_S);
    }
    assert!(done(c), "not done after {limit} frames");
    limit
}

fn household_of(c: &SimCore, carrier: AgentHandle) -> Option<usize> {
    let i = c.agents.resolve_agent(carrier)?;
    let h = c.agents.household_id[i];
    (h != usize::MAX).then_some(h)
}

// The household's agents in member order: the carrier, then the members it brought in spawn
// (index) order.
fn members(c: &SimCore, carrier: AgentHandle) -> Vec<usize> {
    let carrier = c.agents.resolve_agent(carrier).unwrap();
    let mut others: Vec<_> = c
        .agents
        .household_members(c.agents.household_id[carrier])
        .filter(|&i| i != carrier)
        .collect();
    others.sort_unstable();
    std::iter::once(carrier).chain(others).collect()
}

#[test]
fn the_family_steps_out_at_the_car_doors_and_walks_in_once() {
    let (mut c, carrier) = arriving();
    let households = c.households.households.len();
    step_until(&mut c, 4000, |c| household_of(c, carrier).is_some());
    let h = household_of(&c, carrier).unwrap();
    assert_eq!(
        c.households.households.len(),
        households + 1,
        "one household formed"
    );
    assert_eq!(c.households.households[h].member_count, SIZE);
    assert_eq!(
        c.households.pending_arrivals.households, 0,
        "no longer pending"
    );
    let place = c.agents.parking_trip[c.agents.resolve_agent(carrier).unwrap()].destination;
    let family = members(&c, carrier);
    assert_eq!(
        family.len(),
        usize::from(SIZE),
        "every member steps out of the one car"
    );
    for (m, &i) in family.iter().enumerate() {
        let [x, z] = c
            .parking
            .car_exit_point(place, m, crate::config::LEFT_HAND_DRIVE, false)
            .unwrap();
        assert_eq!(
            (c.agents.pos_x[i], c.agents.pos_y[i]),
            (x, z),
            "member {m} at its door"
        );
        assert_eq!(
            c.agents.age_group[i],
            COMPOSITION.member_age_group(m as u16),
            "member {m}"
        );
        assert_eq!(
            c.agents.transit[i], TRANSIT_ACCESS_EGRESS,
            "member {m} walking"
        );
        assert_eq!(c.agents.transit_mode[i], MODE_WALK);
        assert_eq!(c.agents.parking_trip[i].phase, ParkingTripPhase::WalkToDoor);
        assert_eq!(c.agents.home_building[i], 0);
    }
    // They walk the whole way at walking pace, each at its own (`TRAFFIC-40`), none jumping
    // ahead, and all get in.
    let top_pace = 1.0 + crate::simulation::economy::agents::walker_gait::PACE_SPREAD;
    let step_m = crate::config::AGENT_WALK_SPEED_MS * top_pace * (FRAME_S as f32 * SPEED) + 1e-3;
    for _ in 0..4000 {
        if family
            .iter()
            .all(|&i| c.agents.transit[i] == TRANSIT_IN_BUILDING)
        {
            break;
        }
        let before: Vec<_> = family
            .iter()
            .map(|&i| (c.agents.pos_x[i], c.agents.pos_y[i], c.agents.transit[i]))
            .collect();
        c.step_frame(FRAME_S);
        for (&i, &(x, z, transit)) in family.iter().zip(&before) {
            if transit == TRANSIT_ACCESS_EGRESS && c.agents.transit[i] == TRANSIT_ACCESS_EGRESS {
                let moved = (c.agents.pos_x[i] - x).hypot(c.agents.pos_y[i] - z);
                assert!(moved <= step_m, "agent {i} moved {moved} m in one frame");
            }
        }
    }
    for &i in &family {
        assert_eq!(c.agents.transit[i], TRANSIT_IN_BUILDING, "agent {i} got in");
        assert_eq!(c.agents.current_building[i], 0, "agent {i} is home");
    }
    // Nobody counted twice or lost: one household of six at the home, six residents.
    assert_eq!(c.households.households.len(), households + 1);
    assert_eq!(
        c.households
            .households
            .iter()
            .filter(|hh| hh.home_building_id == 0)
            .count(),
        1
    );
    assert_eq!(
        (0..c.agents.len())
            .filter(|&i| c.agents.home_building[i] == 0)
            .count(),
        usize::from(SIZE)
    );
    assert_eq!(c.agents.household_members(h).count(), usize::from(SIZE));
    assert_eq!(c.households.pending_arrivals.households, 0);
}

#[test]
fn a_home_removed_during_the_walk_leaves_the_family_unhoused_where_it_stands() {
    let (mut c, carrier) = arriving();
    step_until(&mut c, 4000, |c| household_of(c, carrier).is_some());
    for _ in 0..3 {
        c.step_frame(FRAME_S);
    }
    let household = c
        .households
        .handles
        .handle(household_of(&c, carrier).unwrap());
    let family = members(&c, carrier);
    assert!(
        family
            .iter()
            .any(|&i| c.agents.transit[i] == TRANSIT_ACCESS_EGRESS),
        "still walking"
    );
    assert!(c.bulldoze_building(0));
    for _ in 0..200 {
        c.step_frame(FRAME_S);
    }
    let h = c.households.handles.resolve(household).unwrap();
    assert_eq!(
        c.households.households[h].home_building_id,
        usize::MAX,
        "the household is unhoused"
    );
    let family: Vec<_> = c.agents.household_members(h).collect();
    assert_eq!(family.len(), usize::from(SIZE), "nobody lost");
    // Each waits where its walk stopped, as any evicted resident out of its home does: in no
    // building, or a walker at its lane's end (`TRAFFIC-24`).
    let at = |c: &SimCore| {
        family
            .iter()
            .map(|&i| (c.agents.pos_x[i], c.agents.pos_y[i]))
            .collect::<Vec<_>>()
    };
    let waiting = at(&c);
    for _ in 0..60 {
        c.step_frame(FRAME_S);
    }
    assert_eq!(at(&c), waiting, "the family stands still");
    for &i in &family {
        assert_eq!(
            c.agents.home_building[i],
            usize::MAX,
            "agent {i} has no home"
        );
        assert_eq!(
            c.agents.target_building[i],
            usize::MAX,
            "agent {i} goes nowhere"
        );
        assert!(
            c.agents.transit[i] != TRANSIT_IN_BUILDING
                || c.agents.current_building[i] == usize::MAX,
            "agent {i} is in no building"
        );
    }
}

#[test]
fn a_road_edit_mid_walk_splits_the_frontage_and_every_member_still_gets_home() {
    let (mut c, carrier) = arriving();
    step_until(&mut c, 4000, |c| household_of(c, carrier).is_some());
    for _ in 0..3 {
        c.step_frame(FRAME_S);
    }
    let family = members(&c, carrier);
    assert!(
        family
            .iter()
            .any(|&i| c.agents.transit[i] != TRANSIT_IN_BUILDING),
        "still walking"
    );
    // A side road joins the frontage edge beside the lot: the edge splits and its lanes are
    // renumbered under the walkers.
    commit(&mut c, (30.0, 0.0), (30.0, 120.0));
    step_until(&mut c, 4000, |c| {
        family.iter().all(|&i| {
            c.agents.transit[i] == TRANSIT_IN_BUILDING && c.agents.current_building[i] == 0
        })
    });
}

// Every agent's position, state and household, bit for bit, and the household count.
pub(super) fn digest(c: &SimCore) -> Vec<u64> {
    let mut out = vec![c.households.households.len() as u64, c.agents.len() as u64];
    for i in 0..c.agents.len() {
        out.extend([
            u64::from(c.agents.pos_x[i].to_bits()) << 32 | u64::from(c.agents.pos_y[i].to_bits()),
            u64::from(c.agents.transit[i]) << 8 | u64::from(c.agents.age_group[i]),
            c.agents.household_id[i] as u64,
            c.agents.current_building[i] as u64,
            c.agents.parking_trip[i].phase as u64,
        ]);
    }
    out
}

// Saves now, runs `frames` on, loads the save and runs the same frames again: both runs must
// end in the same state.
pub(super) fn continues_after_load(c: &mut SimCore, name: &str, frames: usize) {
    let path = temp_save_path(name);
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    for _ in 0..frames {
        c.step_frame(FRAME_S);
    }
    let unsaved = digest(c);
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    c.time.speed_multiplier = SPEED;
    for _ in 0..frames {
        c.step_frame(FRAME_S);
    }
    assert_eq!(digest(c), unsaved, "{name}: the loaded run differs");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_save_before_the_family_steps_out_and_one_mid_walk_continue_as_the_unsaved_run() {
    // The frame the family steps out in, and how many more until its last member is inside.
    let (frames, walk) = {
        let (mut c, carrier) = arriving();
        let frames = step_until(&mut c, 4000, |c| household_of(c, carrier).is_some());
        let family = members(&c, carrier);
        let walk = step_until(&mut c, 4000, |c| {
            family
                .iter()
                .all(|&i| c.agents.transit[i] == TRANSIT_IN_BUILDING)
        });
        (frames, walk)
    };
    assert!(walk >= 4, "the walk in is long enough to save in: {walk} frames");
    // Saved while the car pulls in, before anyone has stepped out.
    let (mut c, carrier) = arriving();
    for _ in 0..frames - 20 {
        c.step_frame(FRAME_S);
    }
    assert!(
        household_of(&c, carrier).is_none(),
        "nobody has stepped out yet"
    );
    // On to the middle of the family's walk in, whatever the walking speed.
    continues_after_load(&mut c, "family-arrival-before-step-out", 20 + walk / 2);
    assert!(
        household_of(&c, carrier).is_some(),
        "the family stepped out after the load"
    );
    // Mid-walk: some members still on their way.
    let family = members(&c, carrier);
    assert!(
        family
            .iter()
            .any(|&i| c.agents.transit[i] != TRANSIT_IN_BUILDING),
        "still walking"
    );
    continues_after_load(&mut c, "family-arrival-mid-walk", 120);
}

#[test]
fn a_carrier_without_a_car_is_driven_in_not_walking() {
    let (mut c, border) = road_and_home();
    let pos = c.region_graph.node(border).pos;
    let i = c
        .agents
        .spawn_household_arrival_carrier(0, 3, COMPOSITION, border, pos.x, pos.z);
    // No car (`arrival_car` decided none): a drop-off (`PARK-34`), never the walk from the
    // border.
    c.agents.parking_trip[i].arrival_ownership = true;
    assert!(c.agents.parking_trip[i].vehicle.is_none());
    assert!(c.agents.start_parking_move_in(
        i,
        &mut c.parking,
        &c.allocator,
        &c.transit_network,
        &c.region_graph
    ));
    let trip = c.agents.parking_trip[i];
    assert!(trip.drop_off);
    assert_eq!(trip.phase, ParkingTripPhase::Drive);
    assert_eq!(
        c.agents.transit_mode[i],
        crate::simulation::economy::agents::MODE_CAR,
        "it rides a car"
    );
}
