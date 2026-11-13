// SPDX-License-Identifier: GPL-2.0-only

//! STRAND diagnosis (temporary, 2026-11-13): loads the owner's dump and follows the households
//! moving in whose new homes a walkway took, and the walkers queued behind them.
//!
//! `METRUM_STRAND_SAVE` (a copy of the dump), `METRUM_STRAND_MODS` (a copy of the packs),
//! `METRUM_STRAND_FRAMES` (default 20,000 at the saved 4x: 22 game minutes) and
//! `METRUM_STRAND_MODE`: `asis` (default), `retarget` (each homeless carrier claims the next
//! vacancy and is sent there, the proposed fix's first half) or `drop` (each is removed, the
//! proposed fallback's effect on the sidewalk).

use super::*;
use crate::simulation::buildings::allocator::baseline_private_zone_slot;
use crate::simulation::economy::agents::{ACCESS_WALK_STRANDED, AgentHandle, TRANSIT_IN_BUILDING};

fn load(source: &str, mods: &str) -> SimCore {
    let mut core = SimCore::new(WorldConfig::new(WORLD_SIZE_M, WORLD_SIZE_M, 40.0, 10.0));
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
// and no home. The loader drops that size for a carrier with no home (`save/agents.rs`), so the
// loaded rows are restored from the save to continue the running game's state; the rows are the
// save's `agent_id`s, checked by position.
fn homeless_carriers(core: &mut SimCore, source: &str) -> Vec<usize> {
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
    let a = &mut core.agents.agents;
    for &(i, pending, x) in &rows {
        assert_eq!(a.pos_x[i], x as f32, "row {i} is the saved agent");
        eprintln!(
            "LOADED CARRIER row {i}: saved pending {pending}, loaded {}",
            a.pending_household_size[i]
        );
        a.pending_household_size[i] = pending;
    }
    core.agents.rebuild_arrival_carriers();
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
#[ignore = "STRAND dump diagnosis; METRUM_STRAND_SAVE, METRUM_STRAND_MODS"]
fn follow_the_households_whose_homes_the_walkway_took() {
    let source = std::env::var("METRUM_STRAND_SAVE").expect("save");
    let mods = std::env::var("METRUM_STRAND_MODS").expect("mods");
    let mode = std::env::var("METRUM_STRAND_MODE").unwrap_or_else(|_| "asis".into());
    let frames: u32 = std::env::var("METRUM_STRAND_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
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
    let homeless = homeless_carriers(&mut core, &source);
    for &i in &homeless {
        eprintln!("HOMELESS CARRIER {}", describe(&core, i));
    }
    // Every agent on a lane a homeless carrier stands on, behind it.
    let a = &core.agents.agents;
    let mut queued = Vec::new();
    for &h in &homeless {
        let lane = a.current_lane_id[h];
        let behind: Vec<usize> = (0..a.len())
            .filter(|&i| {
                i != h && a.current_lane_id[i] == lane && a.lane_distance[i] < a.lane_distance[h]
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
        if let Some(&next) = behind
            .iter()
            .max_by(|&&x, &&y| a.lane_distance[x].total_cmp(&a.lane_distance[y]))
        {
            eprintln!("  NEXT BEHIND {}", describe(&core, next));
        }
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
    let households_before = core.households.households.len();

    match mode.as_str() {
        "retarget" => {
            for &i in &homeless {
                let slot =
                    baseline_private_zone_slot(crate::simulation::zoning::ZoneType::Residential)
                        .unwrap();
                let Some(&home) = core.allocator.vacancy_index[slot].first() else {
                    eprintln!("RETARGET row {i}: no vacancy");
                    continue;
                };
                core.allocator.claim_vacancy(home);
                let a = &mut core.agents.agents;
                a.home_building[i] = home;
                a.target_building[i] = home;
                a.access_flags[i] = 0;
                a.network_replan_failures[i] = 0;
                a.next_replan_time[i] = 0.0;
                a.current_path[i].clear();
                eprintln!("RETARGET row {i} -> building {home}");
            }
        }
        "drop" => {
            let mut rows = homeless.clone();
            rows.sort_unstable_by(|x, y| y.cmp(x));
            for i in rows {
                core.agents.kill_agent(i, &mut core.allocator);
            }
            eprintln!("DROPPED {} carriers", homeless.len());
        }
        _ => {}
    }

    let resolve = |core: &SimCore, handle: AgentHandle| core.agents.resolve_agent(handle);
    let starts: Vec<_> = homeless_handles
        .iter()
        .filter_map(|&h| resolve(&core, h))
        .map(|i| (core.agents.pos_x[i], core.agents.pos_y[i]))
        .collect();
    for frame in 0..frames {
        core.step_frame(FRAME_DT_S);
        if frame % 5_000 == 4_999 {
            let arrived = queued
                .iter()
                .filter(|&&h| {
                    resolve(&core, h).is_none_or(|i| core.agents.transit[i] == TRANSIT_IN_BUILDING)
                })
                .count();
            eprintln!(
                "frame {} t {:.1}: queued walkers indoors {arrived} of {}, households {}",
                frame + 1,
                core.agents.sim_time,
                queued.len(),
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
        "RESULT mode {mode} frames {frames} t {:.1}: queued {} (carriers {}) carriers materialized {arrived} still on the road {still_on_road}; households {} -> {}; pending arrivals {:?}; stranded {:?}",
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
