// SPDX-License-Identifier: GPL-2.0-only

//! `TRAFFIC-14` ring diagnostics, test code only: when wait-for cycles of full lanes form while a
//! tier city is built and warmed to its snapshot day, which cars stand in them and how those
//! cars entered their lanes; the cycles of a saved city; and what re-customizing the routing
//! metric costs on a prepared snapshot (option (g) of the round R14A brief).

use super::super::{FRAME_DT_S, absolute_minute};
use super::super::{cache_dir, residents, snapshot, timing_mode};
use super::{detach_plans, in_play};
use super::waits::wait_cycles;
use crate::nodes::sim::core::populated_city::{
    PopulatedCity, crosses_for, fill_lots, paint_districts, side_m_for, street_grid_with_crosses,
};
use crate::nodes::sim::core::state::SimCore;
use crate::simulation::core::time::SIMULATION_SPEED_STEPS;
use crate::simulation::economy::agents::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_LANE_ATTACH_PENDING, TRANSIT_ACCESS_EGRESS,
    TRANSIT_NETWORK,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

// How a car came onto a road lane: from a connector, a direct pass-through, a lane change, a
// node start (no lane before, in network transit), building egress, or anything else.
const SOURCES: [&str; 6] = [
    "connector",
    "direct",
    "lane_change",
    "node_start",
    "egress",
    "other",
];

// Per-lane entry counts by source, over the current check window and the whole run.
#[derive(Default)]
struct Entries {
    previous_lane: Vec<usize>,
    previous_transit: Vec<u8>,
    window: Vec<[u32; 6]>,
    total: Vec<[u32; 6]>,
}

impl Entries {
    // O(agents) per frame. Agent indices move on swap-removes, so a removal frame can misfile a
    // few entries; a diagnostic tolerance.
    fn note(&mut self, core: &SimCore) {
        let agents = &core.agents.agents;
        let lanes = &core.transit_network.lane_system.lanes;
        self.previous_lane.resize(agents.len(), usize::MAX);
        self.previous_transit.resize(agents.len(), u8::MAX);
        self.window.resize(lanes.len(), [0; 6]);
        self.total.resize(lanes.len(), [0; 6]);
        for i in 0..agents.len() {
            let lane = if in_play(agents, i) {
                agents.current_lane_id[i]
            } else {
                usize::MAX
            };
            let previous = self.previous_lane[i];
            if lane < lanes.len() && lane != previous && lanes[lane].edge_id != usize::MAX {
                let source = match lanes.get(previous) {
                    Some(p) if p.edge_id == usize::MAX => 0,
                    Some(p) if p.edge_id == lanes[lane].edge_id => 2,
                    Some(_) => 1,
                    None if self.previous_transit[i] == TRANSIT_NETWORK => 3,
                    None if self.previous_transit[i] == TRANSIT_ACCESS_EGRESS => 4,
                    None => 5,
                };
                self.window[lane][source] += 1;
                self.total[lane][source] += 1;
            }
            self.previous_lane[i] = lane;
            self.previous_transit[i] = agents.transit[i];
        }
    }

    fn json(counts: &[u32; 6]) -> serde_json::Value {
        serde_json::Value::Object(
            SOURCES
                .iter()
                .zip(counts)
                .filter(|(_, n)| **n > 0)
                .map(|(name, n)| ((*name).to_string(), (*n).into()))
                .collect(),
        )
    }
}

// The lanes of a cycle, sorted, as its identity: the same ring with other cars is the same ring.
fn ring_key(core: &SimCore, cycle: &[usize]) -> BTreeSet<usize> {
    cycle
        .iter()
        .map(|&car| core.agents.agents.current_lane_id[car])
        .collect()
}

// Who stands on a ring's lanes and how they came there.
fn ring_detail(core: &SimCore, key: &BTreeSet<usize>, entries: &Entries) -> serde_json::Value {
    let agents = &core.agents.agents;
    let lanes = &core.transit_network.lane_system.lanes;
    let mut kinds = BTreeMap::<&str, u32>::new();
    let mut per_lane: BTreeMap<usize, (u32, u32)> = key.iter().map(|&l| (l, (0, 0))).collect();
    let mut path_left = Vec::new();
    for i in 0..agents.len() {
        let lane = agents.current_lane_id[i];
        if !in_play(agents, i) || !key.contains(&lane) {
            continue;
        }
        let kind = match (
            agents.freight_shipment_id[i] != u64::MAX,
            (agents.access_flags[i] & ACCESS_FREIGHT_BORDER_DESTINATION) != 0,
        ) {
            (true, true) => "truck_to_border",
            (true, false) => "truck_to_building",
            (false, true) => "car_to_border",
            (false, false) => "car",
        };
        *kinds.entry(kind).or_default() += 1;
        let entry = per_lane.get_mut(&lane).expect("lane is in the key");
        entry.0 += 1;
        entry.1 += u32::from(agents.freight_shipment_id[i] != u64::MAX);
        path_left.push(
            agents.current_path[i]
                .len()
                .saturating_sub(agents.current_path_index[i]),
        );
    }
    path_left.sort_unstable();
    let lane_rows: Vec<serde_json::Value> = per_lane
        .iter()
        .map(|(&lane, &(cars, trucks))| {
            let l = &lanes[lane];
            serde_json::json!({
                "lane": lane, "edge": l.edge_id as i64,
                "node": if l.edge_id == usize::MAX { l.node_id as i64 } else { -1 },
                "length": l.length, "cars": cars, "trucks": trucks,
                "entries_window": Entries::json(&entries.window.get(lane).copied().unwrap_or_default()),
                "entries_total": Entries::json(&entries.total.get(lane).copied().unwrap_or_default()),
            })
        })
        .collect();
    serde_json::json!({
        "kinds": kinds, "lanes": lane_rows,
        "path_left_median": path_left.get(path_left.len() / 2),
        "path_left_max": path_left.last(),
    })
}

// Checks the city for wait-for cycles every `every` game minutes while `frames` steps it, and
// prints every ring the first time it is seen and a count line whenever the counts change.
struct RingWatch {
    every: u64,
    next: u64,
    // Gridlock check timings (`TRAFFIC-14`) since the last check line: checks, sum, max (ms).
    gridlock: (u64, f64, f64),
    seen: BTreeMap<BTreeSet<usize>, u64>,
    entries: Entries,
    last_counts: (usize, usize),
}

impl RingWatch {
    fn new(core: &SimCore) -> Self {
        let every = std::env::var("METRUM_RING_EVERY_MINUTES")
            .map(|value| value.parse().expect("METRUM_RING_EVERY_MINUTES must be an integer"))
            .unwrap_or(30);
        Self {
            every,
            next: absolute_minute(core),
            gridlock: (0, 0.0, 0.0),
            seen: BTreeMap::new(),
            entries: Entries::default(),
            last_counts: (usize::MAX, 0),
        }
    }

    fn frame(&mut self, core: &SimCore, label: &str) {
        self.check(core, label, false);
    }

    // Adds a frame's gridlock check time; a frame runs at most a few checks, so the per-check
    // time is the frame's share.
    fn gridlock_frame(&mut self, checks: u64, ms: f64) {
        if checks > 0 {
            let per_check = ms / checks as f64;
            self.gridlock.0 += checks;
            self.gridlock.1 += ms;
            self.gridlock.2 = self.gridlock.2.max(per_check);
        }
    }

    // The last check of a run always prints its count line.
    fn check(&mut self, core: &SimCore, label: &str, last: bool) {
        self.entries.note(core);
        let now = absolute_minute(core);
        if now < self.next {
            return;
        }
        self.next = now + self.every;
        let (checks, sum_ms, max_ms) = std::mem::replace(&mut self.gridlock, (0, 0.0, 0.0));
        eprintln!(
            "TRAFFIC14_GRIDLOCK {}",
            serde_json::json!({
                "label": label, "minute": now, "day": core.time.day_index,
                "minute_of_day": core.time.minute_of_day,
                "report": core.agents.gridlock.report(),
                "window_checks": checks,
                "check_ms_mean": if checks > 0 { sum_ms / checks as f64 } else { 0.0 },
                "check_ms_max": max_ms,
            })
        );
        let started = Instant::now();
        let cycles = wait_cycles(core);
        let cars: usize = cycles.iter().map(|cycle| cycle.cars.len()).sum();
        let with_way_out = cycles.iter().filter(|c| c.heads_with_room > 0).count();
        let mut new = 0;
        for info in &cycles {
            let cycle = &info.cars;
            let key = ring_key(core, cycle);
            if self.seen.contains_key(&key) {
                continue;
            }
            new += 1;
            eprintln!(
                "TRAFFIC14_RING_NEW {}",
                serde_json::json!({
                    "label": label, "minute": now, "day": core.time.day_index,
                    "minute_of_day": core.time.minute_of_day, "cycle_cars": cycle.len(),
                    "road_heads": info.road_heads, "heads_with_way_out": info.heads_with_room,
                    "ways_out": info.ways_out,
                    "detail": ring_detail(core, &key, &self.entries),
                })
            );
            self.seen.insert(key, now);
        }
        if (cycles.len(), cars) != self.last_counts || new > 0 || last {
            let held_at_node = (0..core.agents.agents.len())
                .filter(|&i| {
                    let a = &core.agents.agents;
                    a.transit[i] == TRANSIT_NETWORK
                        && a.current_lane_id[i] == usize::MAX
                        && !a.current_path[i].is_empty()
                })
                .count();
            eprintln!(
                "TRAFFIC14_RING_COUNT {}",
                serde_json::json!({
                    "label": label, "minute": now, "day": core.time.day_index,
                    "minute_of_day": core.time.minute_of_day, "cycles": cycles.len(),
                    "cars_in_cycles": cars, "rings_seen": self.seen.len(), "new": new,
                    "cycles_with_way_out": with_way_out,
                    "vehicles_on_road": (0..core.agents.agents.len())
                        .filter(|&i| in_play(&core.agents.agents, i))
                        .count(),
                    "held_at_node": held_at_node,
                    "held_at_attach": (0..core.agents.agents.len())
                        .filter(|&i| {
                            let a = &core.agents.agents;
                            a.transit[i] == TRANSIT_NETWORK
                                && a.current_lane_id[i] == usize::MAX
                                && (a.access_flags[i] & ACCESS_LANE_ATTACH_PENDING) != 0
                        })
                        .count(),
                    "detach_plans": detach_plans(core),
                    "check_ms": started.elapsed().as_secs_f64() * 1e3,
                })
            );
            self.last_counts = (cycles.len(), cars);
        }
        self.entries.window.fill([0; 6]);
    }
}

#[test]
#[ignore = "TRAFFIC-14 ring formation over a tier city's build and warm-up; release, heavy"]
fn traffic14_ring_formation() {
    timing_mode();
    let residents = residents();
    // METRUM_RING_DAY: the day to stop at (default 12, the snapshot boundary).
    let stop_day: u32 = std::env::var("METRUM_RING_DAY")
        .map(|value| value.parse().expect("METRUM_RING_DAY must be an integer"))
        .unwrap_or(12);
    // As `PopulatedCity::build_timed`, but stepping the warm-up frames here so the watch sees
    // every one; then on to the snapshot boundary as `warm_to_snapshot_boundary` does.
    let side_m = side_m_for(residents);
    let floored = crosses_for(side_m);
    for crosses in [floored, floored + 1] {
        let (mut core, row_z, half_x) = street_grid_with_crosses(side_m, crosses);
        paint_districts(&mut core, &row_z, half_x);
        fill_lots(&mut core);
        core.time.speed_multiplier = SIMULATION_SPEED_STEPS[SIMULATION_SPEED_STEPS.len() - 1];
        let mut city = PopulatedCity {
            core,
            departures: 0,
        };
        if city.admit_households(residents) < residents {
            continue;
        }
        let core = &mut city.core;
        let mut watch = RingWatch::new(core);
        eprintln!(
            "TRAFFIC14_RING_START {}",
            serde_json::json!({"residents": residents, "crosses": crosses,
                "day": core.time.day_index, "minute_of_day": core.time.minute_of_day,
                "lanes": core.transit_network.lane_system.lanes.len()})
        );
        while core.time.day_index < stop_day {
            watch.frame(core, "build");
            let checks = core.agents.gridlock.report().checks;
            let frame = core.step_frame(FRAME_DT_S);
            watch.gridlock_frame(
                core.agents.gridlock.report().checks - checks,
                frame.gridlock_ms,
            );
        }
        watch.next = 0;
        watch.check(core, "build", true);
        return;
    }
    panic!("the grid housed too few residents");
}

#[test]
#[ignore = "TRAFFIC-14 rings of a saved city over METRUM_RING_HOURS; needs METRUM_RING_SAVE"]
fn traffic14_rings_in_save() {
    timing_mode();
    let path = std::env::var("METRUM_RING_SAVE").expect("METRUM_RING_SAVE: path of a save copy");
    let hours: u64 = std::env::var("METRUM_RING_HOURS")
        .map(|value| value.parse().expect("METRUM_RING_HOURS must be an integer"))
        .unwrap_or(24);
    let speed: f32 = std::env::var("METRUM_PERF07_SPEED")
        .map(|value| value.parse().expect("METRUM_PERF07_SPEED must be a number"))
        .unwrap_or(32.0);
    let mut core = SimCore {
        benchmark_mode: true,
        ..SimCore::new(crate::simulation::core::config::WorldConfig::new(
            2048.0, 2048.0, 40.0, 10.0,
        ))
    };
    crate::nodes::sim::core::populated_city::install_bootstrap_assets(&mut core);
    core.load_game_internal(&path).unwrap();
    core.time.speed_multiplier = speed;
    let start = absolute_minute(&core);
    let mut watch = RingWatch::new(&core);
    eprintln!(
        "TRAFFIC14_RING_START {}",
        serde_json::json!({"save": path, "agents": core.agents.agents.len(),
            "day": core.time.day_index, "minute_of_day": core.time.minute_of_day,
            "lanes": core.transit_network.lane_system.lanes.len()})
    );
    while absolute_minute(&core) < start + hours * 60 {
        watch.frame(&core, "save");
        let checks = core.agents.gridlock.report().checks;
        let frame = core.step_frame(FRAME_DT_S);
        watch.gridlock_frame(
            core.agents.gridlock.report().checks - checks,
            frame.gridlock_ms,
        );
    }
    watch.next = 0;
    watch.check(&core, "save", true);
}

#[test]
#[ignore = "TRAFFIC-14 option (g): cost of one routing-metric customization and of what follows; release"]
fn traffic14_customize_cost() {
    timing_mode();
    let mut durations = Vec::new();
    // Two loads of the same snapshot: a control hour, then an hour that starts with one metric
    // customization (equal weights, a new revision, so every revision-keyed cache reprices).
    let minutes: u64 = std::env::var("METRUM_RING_HOURS")
        .map(|value| value.parse::<u64>().expect("METRUM_RING_HOURS must be an integer") * 60)
        .unwrap_or(60);
    for customize in [false, true, false, true] {
        let mut city = snapshot::load_snapshot(residents(), &cache_dir());
        let core = &mut city.core;
        let mut customize_ms = Vec::new();
        let mut flow_ms = None;
        if customize {
            for _ in 0..5 {
                let started = Instant::now();
                core.transit_network
                    .cch_graph
                    .customize(&core.region_graph);
                customize_ms.push(started.elapsed().as_secs_f64() * 1e3);
            }
            // A full flow-field rebuild on the same metric, as a road edit's
            // `mark_all_dirty` asks for.
            let started = Instant::now();
            core.transit_network.flow_fields.mark_all_dirty();
            let revision = core.transit_network.flow_graph_revision();
            let alloc = &core.allocator;
            let graph = &core.region_graph;
            core.transit_network
                .flow_fields
                .rebuild_dirty(graph, revision, |zone, mode_flags, sources| {
                    alloc.collect_sources_for_zone(zone, graph, mode_flags, sources)
                });
            flow_ms = Some(started.elapsed().as_secs_f64() * 1e3);
        }
        let work_before = crate::debug::work::snapshot();
        let start = absolute_minute(core);
        let started = Instant::now();
        let mut frames = 0_u32;
        let (mut hourly_ms, mut worst_ms) = (0.0_f64, 0.0_f64);
        let mut first_frames_ms = Vec::new();
        while absolute_minute(core) < start + minutes {
            let frame_started = Instant::now();
            let frame = core.step_frame(FRAME_DT_S);
            let ms = frame_started.elapsed().as_secs_f64() * 1e3;
            hourly_ms += frame.hourly_ms;
            worst_ms = worst_ms.max(ms);
            if first_frames_ms.len() < 8 {
                first_frames_ms.push(ms);
            }
            frames += 1;
        }
        let work_after = crate::debug::work::snapshot();
        let work: BTreeMap<usize, u64> = work_after
            .iter()
            .zip(work_before.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(k, (a, b))| (k, a - b))
            .collect();
        customize_ms.sort_by(f64::total_cmp);
        let wall_s = started.elapsed().as_secs_f64();
        durations.push(wall_s);
        eprintln!(
            "TRAFFIC14_CUSTOMIZE {}",
            serde_json::json!({
                "customize_first": customize, "customize_ms": customize_ms,
                "flow_fields_rebuild_ms": flow_ms,
                "minutes": minutes, "frames": frames, "wall_s": wall_s,
                "hourly_ms": hourly_ms, "worst_frame_ms": worst_ms,
                "first_frames_ms": first_frames_ms,
                "metric_revision": core.transit_network.cch_graph.metric_revision,
                "work_delta_by_index": work,
            })
        );
    }
}

#[test]
#[ignore = "TRAFFIC-14 gridlock check cost on a prepared snapshot; release, P-cores"]
fn traffic14_gridlock_check_cost() {
    // The snapshot city stepped METRUM_RING_MINUTES game minutes (default 30, so the checker has
    // run), then METRUM_GRIDLOCK_REPEATS (default 64) extra checks on the standing state, each
    // timed alone. The extra checks see the same lanes, so after the first they confirm what the
    // previous one saw: the cost of a check, not of a detour.
    timing_mode();
    let minutes: u64 = std::env::var("METRUM_RING_MINUTES")
        .map(|value| value.parse().expect("METRUM_RING_MINUTES must be an integer"))
        .unwrap_or(30);
    let repeats: usize = std::env::var("METRUM_GRIDLOCK_REPEATS")
        .map(|value| value.parse().expect("METRUM_GRIDLOCK_REPEATS must be an integer"))
        .unwrap_or(64);
    let mut city = snapshot::load_snapshot(residents(), &cache_dir());
    let core = &mut city.core;
    let start = absolute_minute(core);
    while absolute_minute(core) < start + minutes {
        core.step_frame(FRAME_DT_S);
    }
    let slot = core.time.absolute_slot();
    let mut samples = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let started = Instant::now();
        crate::thread_pools::install(|| {
            core.agents.check_gridlock(
                &core.allocator,
                &core.transit_network,
                &core.region_graph,
                slot,
                Some(&core.parking),
            )
        });
        samples.push(started.elapsed().as_secs_f64() * 1e3);
    }
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "TRAFFIC14_GRIDLOCK_COST {}",
        serde_json::json!({
            "residents": residents(), "day": core.time.day_index,
            "minute_of_day": core.time.minute_of_day,
            "lanes": core.transit_network.lane_system.lanes.len(),
            "report": core.agents.gridlock.report(),
            "ms_min": samples[0], "ms_median": samples[samples.len() / 2],
            "ms_max": samples[samples.len() - 1],
        })
    );
}
