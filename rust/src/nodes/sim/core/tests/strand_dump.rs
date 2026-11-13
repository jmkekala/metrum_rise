// SPDX-License-Identifier: GPL-2.0-only

//! `ECON-29` / `TRAFFIC-37` end-to-end (round FSTRAND, from round STRAND's diagnosis): loads the
//! owner's dump, saved before the fix with five households moving in whose new homes a walkway
//! took (carriers with a pending household and no home), and follows them and the walkers queued
//! behind them. The load makes each an unhoused household (`unhouse_homeless_arrival_carriers`);
//! the housing queue rehouses them by its own rule, and the walkers behind them pass.
//!
//! `METRUM_STRAND_SAVE` (a copy of the dump), `METRUM_STRAND_MODS` (a copy of the packs) and
//! `METRUM_STRAND_FRAMES` (default 120,000 at the saved 4x: about two game hours).

use super::super::frame::FRAME_DT_S;
use super::super::state::SimCore;
use crate::assets::scan_pack_dir;
use crate::simulation::buildings::allocator::baseline_private_zone_slot;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::economy::agents::{ACCESS_WALK_STRANDED, AgentHandle, TRANSIT_IN_BUILDING};
use std::path::Path;

fn load(source: &str, mods: &str) -> SimCore {
    let mut core = SimCore::new(WorldConfig::new(2048.0, 2048.0, 40.0, 10.0));
    let scan = scan_pack_dir(Path::new(mods));
    assert!(!scan.packs.is_empty(), "{:?}", scan.warnings);
    core.replace_asset_registry(scan.packs.into_iter().flat_map(|pack| {
        let pack_id = pack.pack.pack_id;
        pack.assets
            .into_iter()
            .map(move |(manifest, dir)| (pack_id.clone(), manifest, dir))
    }));
    core.load_game_internal(source).unwrap();
    core
}

fn describe(core: &SimCore, i: usize) -> String {
    let a = &core.agents.agents;
    format!(
        "row {i} pending {} household {} home {} target {} transit {} mode {} node {} edge {} lane {} d {:.1} flags {:#x} fails {} next {:.1} pos ({:.1},{:.1})",
        a.pending_household_size[i],
        a.household_id[i] as i64,
        a.home_building[i] as i64,
        a.target_building[i] as i64,
        a.transit[i],
        a.transit_mode[i],
        a.current_node[i] as i64,
        a.current_edge[i] as i64,
        a.current_lane_id[i] as i64,
        a.lane_distance[i],
        a.access_flags[i],
        a.network_replan_failures[i],
        a.next_replan_time[i],
        a.pos_x[i],
        a.pos_y[i],
    )
}

// The households moving in whose homes the walkway took: saved with `pending_household_size`
// and no home, the save's `agent_id`s, checked by position. The load made each the first member
// of an unhoused household.
fn homeless_carriers(core: &SimCore, source: &str) -> Vec<usize> {
    let conn = rusqlite::Connection::open(source).unwrap();
    let mut stmt = conn
        .prepare(
            "select agent_id, pending_household_size, pos_x from agents \
             where pending_household_size > 0 and home_building = -1 order by agent_id",
        )
        .unwrap();
    let rows: Vec<(usize, u16, f64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let a = &core.agents.agents;
    for &(i, pending, x) in &rows {
        assert_eq!(a.pos_x[i], x as f32, "row {i} is the saved agent");
        let household = a.household_id[i];
        assert!(
            household < core.households.households.len(),
            "row {i} has its household"
        );
        let record = &core.households.households[household];
        assert_eq!(record.home_building_id, usize::MAX, "unhoused");
        assert_eq!(record.member_count, pending, "row {i}: every member");
        assert_eq!(a.pending_household_size[i], 0);
        eprintln!("LOADED FAMILY row {i}: household {household} of {pending}, unhoused");
    }
    rows.into_iter().map(|(i, _, _)| i).collect()
}

fn vacancies(core: &SimCore) -> Vec<(usize, u32, u32, String)> {
    let slot =
        baseline_private_zone_slot(crate::simulation::zoning::ZoneType::Residential).unwrap();
    core.allocator.vacancy_index[slot]
        .iter()
        .map(|&b| {
            (
                b,
                core.allocator.buildings[b].occupancy,
                core.allocator.household_capacity(b),
                core.allocator.buildings[b].asset_id.clone(),
            )
        })
        .collect()
}

#[test]
#[ignore = "STRAND dump replay; METRUM_STRAND_SAVE, METRUM_STRAND_MODS"]
fn follow_the_households_whose_homes_the_walkway_took() {
    let source = std::env::var("METRUM_STRAND_SAVE").expect("save");
    let mods = std::env::var("METRUM_STRAND_MODS").expect("mods");
    let frames: u32 = std::env::var("METRUM_STRAND_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120_000);
    let mut core = load(&source, &mods);
    // The save holds the session's 4x; a loaded game starts paused.
    core.time.speed_multiplier = 4.0;
    eprintln!(
        "LOADED t {:.1} day {} minute {} speed {} agents {} buildings {} households {}",
        core.agents.sim_time,
        core.time.day_index,
        core.time.minute_of_day,
        core.time.speed_multiplier,
        core.agents.len(),
        core.allocator.buildings.len(),
        core.households.households.len()
    );
    eprintln!(
        "VACANCIES (building, occupancy, capacity, asset) {:?}",
        vacancies(&core)
    );
    core.agents
        .refresh_stranded_report(&core.transit_network, &core.region_graph);
    eprintln!("STRANDED REPORT {:?}", core.agents.stranded_report());
    let homeless = homeless_carriers(&core, &source);
    for &i in &homeless {
        eprintln!("HOMELESS FAMILY {}", describe(&core, i));
    }
    // Every agent on a lane a homeless carrier stands on, behind it.
    let a = &core.agents.agents;
    let mut queued = Vec::new();
    for &h in &homeless {
        let lane = a.current_lane_id[h];
        let behind: Vec<usize> = (0..a.len())
            .filter(|&i| {
                i != h
                    && lane != usize::MAX
                    && a.current_lane_id[i] == lane
                    && a.lane_distance[i] < a.lane_distance[h]
            })
            .collect();
        eprintln!(
            "LANE {lane} (edge {}, carrier {h} at {:.1} m, stranded {}): {} behind it, of them {} carriers with a home",
            a.current_edge[h] as i64,
            a.lane_distance[h],
            a.access_flags[h] & ACCESS_WALK_STRANDED != 0,
            behind.len(),
            behind
                .iter()
                .filter(|&&i| a.pending_household_size[i] > 0 && a.home_building[i] != usize::MAX)
                .count()
        );
        queued.extend(behind);
    }
    queued.sort_unstable();
    queued.dedup();
    let queued_carrier_rows: Vec<usize> = queued
        .iter()
        .copied()
        .filter(|&i| core.agents.pending_household_size[i] > 0)
        .collect();
    let queued_carriers: Vec<_> = queued_carrier_rows
        .iter()
        .map(|&i| core.agents.agent_handle(i))
        .collect();
    let queued: Vec<_> = queued
        .iter()
        .map(|&i| core.agents.agent_handle(i))
        .collect();
    let homeless_handles: Vec<_> = homeless
        .iter()
        .map(|&i| core.agents.agent_handle(i))
        .collect();
    let families: Vec<_> = homeless
        .iter()
        .map(|&i| core.households.handles.handle(core.agents.household_id[i]))
        .collect();
    let households_before = core.households.households.len();

    let resolve = |core: &SimCore, handle: AgentHandle| core.agents.resolve_agent(handle);
    let starts: Vec<_> = homeless_handles
        .iter()
        .filter_map(|&h| resolve(&core, h))
        .map(|i| (core.agents.pos_x[i], core.agents.pos_y[i]))
        .collect();
    for frame in 0..frames {
        core.step_frame(FRAME_DT_S);
        if frame % 10_000 == 9_999 {
            let arrived = queued
                .iter()
                .filter(|&&h| {
                    resolve(&core, h).is_none_or(|i| core.agents.transit[i] == TRANSIT_IN_BUILDING)
                })
                .count();
            let housed = families
                .iter()
                .filter_map(|&f| core.households.handles.resolve(f))
                .filter(|&h| core.households.households[h].home_building_id != usize::MAX)
                .count();
            eprintln!(
                "frame {} t {:.1} day {} minute {}: queued walkers indoors {arrived} of {}, families housed {housed} of {}, households {}",
                frame + 1,
                core.agents.sim_time,
                core.time.day_index,
                core.time.minute_of_day,
                queued.len(),
                families.len(),
                core.households.households.len()
            );
        }
    }
    for (n, &h) in homeless_handles.iter().enumerate() {
        match resolve(&core, h) {
            Some(i) => {
                let moved = starts
                    .get(n)
                    .is_some_and(|&(x, y)| core.agents.pos_x[i] != x || core.agents.pos_y[i] != y);
                eprintln!("AFTER CARRIER moved {moved}: {}", describe(&core, i));
            }
            None => eprintln!("AFTER CARRIER gone"),
        }
    }
    for &family in &families {
        match core.households.handles.resolve(family) {
            Some(h) => {
                let record = &core.households.households[h];
                let members: Vec<_> = core
                    .agents
                    .household_members(h)
                    .map(|m| {
                        (
                            core.agents.transit[m],
                            core.agents.current_building[m] as i64,
                        )
                    })
                    .collect();
                eprintln!(
                    "AFTER FAMILY household {h}: home {} unhoused days {} members (transit, building) {members:?}",
                    record.home_building_id as i64, record.unhoused_days_elapsed
                );
            }
            None => eprintln!("AFTER FAMILY gone (left by the exit rules)"),
        }
    }
    // Queued households moving in that materialized (their carrier joined its household).
    let arrived = queued
        .iter()
        .filter(|&&h| queued_carriers.contains(&h))
        .filter(|&&h| resolve(&core, h).is_none_or(|i| core.agents.pending_household_size[i] == 0))
        .count();
    let still_on_road = queued
        .iter()
        .filter_map(|&h| resolve(&core, h))
        .filter(|&i| core.agents.transit[i] != TRANSIT_IN_BUILDING)
        .count();
    core.agents
        .refresh_stranded_report(&core.transit_network, &core.region_graph);
    core.households.recount_pending_arrivals(&mut core.agents);
    eprintln!(
        "RESULT frames {frames} t {:.1}: queued {} (carriers {}) carriers materialized {arrived} still on the road {still_on_road}; households {} -> {}; pending arrivals {:?}; stranded {:?}",
        core.agents.sim_time,
        queued.len(),
        queued_carriers.len(),
        households_before,
        core.households.households.len(),
        core.households.pending_arrivals,
        core.agents.stranded_report()
    );
    for line in &core.agents.trip_failures.lines {
        eprintln!("FAILURE {line}");
    }
    eprintln!("VACANCIES AFTER {:?}", vacancies(&core));
}
