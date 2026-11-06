// SPDX-License-Identifier: GPL-2.0-only

//! Pre-populated city for timing the economy ticks (`ECON-13`).
//!
//! Residential streets run every 50 m with back-to-back lots between them, crossed by a street
//! every 400 m, and are zoned in repeating row districts with the player's brush. Every lot is
//! filled through the demand spawn path with the repository's bootstrap pack, and households are
//! admitted through the demand admission path; their carriers are placed at home instead of
//! driving in, which is the only step that bypasses the game. The city then runs frame by frame
//! at maximum speed, so agents move and the trips the economy starts finish as in the game. See
//! `docs/economy.md#econ-13-economy-tick-baseline`.

#[cfg(test)]
pub(super) mod census_churn;
#[cfg(test)]
mod commit_locality;
#[cfg(test)]
mod contract;
#[cfg(test)]
mod envelope;
#[cfg(test)]
mod frame_budget;
#[cfg(test)]
mod load_check;
#[cfg(test)]
mod routing_benchmark;
#[cfg(test)]
mod snapshot;
#[cfg(test)]
mod walking_proof;
#[cfg(test)]
mod watchdog_probe;

use std::time::{Duration, Instant};

use glam::DVec2;
use godot::prelude::Vector3;

use super::budget::CityTreasury;
use super::frame::FRAME_DT_S;
use super::scenario::{GrowthDayRecord, install_bootstrap_assets, paint};
use super::state::SimCore;
use crate::simulation::buildings::allocator::BuildingSiteEnvironment;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::SIMULATION_SPEED_STEPS;
use crate::simulation::economy::demand::DemandBuildingActionPlan;

use crate::simulation::zoning::ZoneType;
use crate::simulation::zoning::cells::CellSelectionShape;

// Two bootstrap lots fit back to back between neighbouring rows.
const ROW_SPACING_M: f32 = 50.0;
const CROSS_SPACING_M: f32 = 400.0;
const WORLD_MARGIN_M: f32 = 512.0;
// Measured fill of this layout with the bootstrap pack: about 2,800 residents per km².
const RESIDENTS_PER_M2: f32 = 2.8e-3;
// Admission stops at the requested residents, so the grid keeps some homes empty.
const SIDE_HEADROOM: f32 = 1.08;
// Bootstrap homes hold one household of about two people.
const RESIDENTS_PER_HOUSEHOLD: u32 = 2;
// Rows repeat this district pattern from south to north.
const DISTRICTS: [ZoneType; 8] = [
    ZoneType::Residential,
    ZoneType::Residential,
    ZoneType::Commercial,
    ZoneType::Residential,
    ZoneType::Residential,
    ZoneType::Industrial,
    ZoneType::Residential,
    ZoneType::Residential,
];
// Construction is free and the treasury never runs out, so city services stay funded in every
// tier; solvency is the growth scenario's measure, not this one. A 100k city spends about 1e6 a
// day, and at 1e10 an f64 still resolves 2e-6: at 1e15 every posting rounded to 0.125, which
// broke money conservation once settlement posted per entity (`SIM-02-G`).
const FUNDED_TREASURY: f64 = 1.0e10;
const WARM_UP_DAYS: u32 = 2;
// One border trunk per this many residents. Imports cross the border on the roads alone, so a
// single trunk could not supply a 100k city's industry; 10k keeps its one trunk.
const RESIDENTS_PER_BORDER: f32 = 10_000.0;

/// Wall-clock cost of the fixed-slot systems at the start of one hour, as
/// [`PopulatedCity::advance_hour`] and [`PopulatedCity::advance_hour_through_actions`] step it.
/// Entity phases, queues and rolling chunks run in every slot and are not included.
#[derive(Clone, Copy, Debug)]
pub struct EconomyHourTimes {
    /// The census publication, what is left of the operational hour and the planning slots that
    /// ran in the stepped frames (`SIM-02-K`).
    pub hourly: Duration,
    /// The calendar close; present only when the hour started a day. Daily settlement runs at
    /// each entity's own daily phase in every slot (`SIM-02-G`).
    pub daily: Option<Duration>,
    /// Whether the hour does not start a day. Every hour runs its four planning slots; the
    /// economy benchmark and `PERF-01` time the 23 hours without the calendar close, as they
    /// did before `SIM-02-K`, so their means stay comparable.
    pub demand_pass: bool,
}

/// Wall-clock cost of each [`PopulatedCity::build`] phase, for projecting larger tiers.
#[derive(Clone, Copy, Debug, Default)]
pub struct PopulatedCityBuildTimes {
    /// Street grid, border connection and the bulk road finalization.
    pub street_grid: Duration,
    /// Painting the row districts with the player's brush.
    pub zoning: Duration,
    /// Spawning and completing a building on every legal lot.
    pub lots: Duration,
    /// Household admission until the requested residents live in the city.
    pub admission: Duration,
    /// The warm-up days' operational hours and daily settlements.
    pub warm_up: Duration,
}

/// A fully built, fully housed city whose economy ticks can be timed hour by hour.
pub struct PopulatedCity {
    core: SimCore,
    departures: u32,
}

impl PopulatedCity {
    /// Builds a street grid large enough for `residents`, fills every zoned lot, admits
    /// households until at least `residents` people live in the city, then runs two simulated
    /// days so employment and inventories settle. The clock starts at day 1, 07:30, at maximum
    /// speed.
    ///
    /// The grid floors its width to whole cross-street blocks. Where that loses more area than
    /// the side's headroom keeps, so the floored grid cannot house `residents` (1,300 and 13,000
    /// residents, `ROAD-68-K24`), the city is built once more one block wider, covering its side;
    /// a tier the floored grid houses is built exactly as before. Panics if the wider grid holds
    /// too few homes as well, which means the sizing constants no longer match the bootstrap pack.
    pub fn build(residents: u32) -> Self {
        Self::build_timed(residents).0
    }

    /// [`Self::build`], also returning the wall-clock cost of each phase, summed over both
    /// grids when the floored one could not house `residents`.
    pub fn build_timed(residents: u32) -> (Self, PopulatedCityBuildTimes) {
        let side_m = side_m_for(residents);
        let floored = crosses_for(side_m);
        let mut times = PopulatedCityBuildTimes::default();
        let mut housed = 0;
        for crosses in [floored, floored + 1] {
            let started = Instant::now();
            let (mut core, row_z, half_x) = street_grid_with_crosses(side_m, crosses);
            times.street_grid += started.elapsed();
            let started = Instant::now();
            paint_districts(&mut core, &row_z, half_x);
            times.zoning += started.elapsed();
            let started = Instant::now();
            fill_lots(&mut core);
            times.lots += started.elapsed();
            core.time.speed_multiplier = SIMULATION_SPEED_STEPS[SIMULATION_SPEED_STEPS.len() - 1];
            let mut city = Self {
                core,
                departures: 0,
            };
            let started = Instant::now();
            housed = city.admit_households(residents);
            times.admission += started.elapsed();
            if housed < residents {
                continue;
            }
            let started = Instant::now();
            for _ in 0..WARM_UP_DAYS * 24 {
                city.advance_hour();
            }
            times.warm_up = started.elapsed();
            return (city, times);
        }
        panic!("{housed} residents housed, {residents} requested");
    }

    /// City totals at the current clock, in the growth scenario's record form.
    pub fn record(&self) -> GrowthDayRecord {
        let city = self.core.daily_city_flow_diagnostics();
        let (net_open_jobs, vacancy_reserve, _) = self.core.demand.housing_balance();
        GrowthDayRecord {
            day: self.core.time.day_index,
            households: city.active_households,
            residents: city.resident_agents,
            employed: city.employed_agents,
            jobs: city
                .commercial_job_capacity
                .saturating_add(city.industrial_job_capacity),
            buildings: self.core.allocator.buildings.len() as u32,
            treasury: self.core.treasury.balance,
            vacant_homes: city.vacant_household_slots,
            vacancy_reserve,
            unhoused_households: city.unhoused_households,
            net_open_jobs,
            departures: self.departures,
        }
    }

    /// Steps frames until the clock crosses the next whole hour and returns the cost of the
    /// fixed-slot systems that frame ran: the census publication, what is left of the
    /// operational hour, the planning slots it reached and, when a day starts, the calendar
    /// close. Agent movement and the other frame phases run but are not timed.
    pub fn advance_hour(&mut self) -> EconomyHourTimes {
        loop {
            // One maximum-speed frame spans about half an authored minute, so it never crosses
            // two hours.
            let frame = self.core.step_frame(FRAME_DT_S);
            if frame.hourly_ticks > 0 {
                let midnight = frame.daily_ticks > 0;
                if midnight {
                    self.departures += self.core.demand.housing_balance().2;
                }
                return EconomyHourTimes {
                    hourly: Duration::from_secs_f64(frame.hourly_ms / 1000.0),
                    daily: midnight.then(|| Duration::from_secs_f64(frame.daily_ms / 1000.0)),
                    demand_pass: !midnight,
                };
            }
        }
    }

    /// [`Self::advance_hour`], then the frames up to and including the hour's actions slot,
    /// following the hour's slot layout. The hour's demand planning runs at the first slots of
    /// its minutes 0–3 and its building actions (permits) at minute 3's (`SIM-02-I`,
    /// `PlanningSlot`), each in a frame of its own at maximum speed (about 8.5 slots a frame).
    /// A window ending here holds the whole hour's planning and actions; the planning slots of
    /// the frames after the first are added to `hourly`.
    pub fn advance_hour_through_actions(&mut self) -> EconomyHourTimes {
        use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
        use crate::simulation::economy::demand::PlanningSlot;
        let mut times = self.advance_hour();
        // The clock's slot in progress has run (`TimeSystem::absolute_slot`).
        while self.core.time.absolute_slot() % SLOTS_PER_HOUR < PlanningSlot::Actions as u64 {
            let frame = self.core.step_frame(FRAME_DT_S);
            times.hourly += Duration::from_secs_f64(frame.hourly_ms / 1000.0);
        }
        times
    }

    // Admits households until `residents` live in the city or no home is left; returns the
    // residents housed.
    fn admit_households(&mut self, residents: u32) -> u32 {
        let core = &mut self.core;
        core.transit_network
            .rebuild_pathing_if_dirty(&mut core.region_graph);
        loop {
            let core = &mut self.core;
            let housed = core.daily_city_flow_diagnostics().resident_agents;
            let wanted = residents
                .saturating_sub(housed)
                .div_ceil(RESIDENTS_PER_HOUSEHOLD);
            let launched = core
                .allocator
                .execute_demand_household_admission_with_income(
                    wanted,
                    core.households.households.len(),
                    &mut core.agents,
                    &core.transit_network,
                    &core.region_graph,
                    // This fixture explicitly prepopulates a city outside demand admission.
                    |_, _, _| true,
                );
            // Carriers arrive at once instead of driving in from the border, and move in. One
            // that a swap-remove moved into the lane snapshot leaves it first (`ROAD-68-K24`).
            core.agents.held_arrivals.clear();
            for i in 0..core.agents.len() {
                if core.agents.pending_household_size[i] > 0 {
                    core.agents.place_arrival_carrier_at_home(i);
                    core.households.materialize_arrival_carrier(
                        i,
                        &mut core.agents,
                        &core.allocator,
                    );
                }
            }
            self.core.publish_parking_households();
            self.advance_hour();
            let housed = self.core.daily_city_flow_diagnostics().resident_agents;
            if housed >= residents || launched == 0 {
                return housed;
            }
        }
    }
}

fn side_m_for(residents: u32) -> f32 {
    (residents as f32 / RESIDENTS_PER_M2).sqrt() * SIDE_HEADROOM
}

// The cross streets of a `side_m` grid: whole blocks, floored.
fn crosses_for(side_m: f32) -> usize {
    (side_m / CROSS_SPACING_M) as usize + 1
}

/// Builds the street grid of a `side_m` square city with every zoned lot built and complete.
#[cfg(test)]
pub(super) fn zoned_city(side_m: f32) -> SimCore {
    let (mut core, row_z, half_x) = street_grid(side_m);
    paint_districts(&mut core, &row_z, half_x);
    fill_lots(&mut core);
    core
}

// [`zoned_city`] in a square world `world_m` wide instead of the grid's own margin, its trunk
// continuing to that world's west edge (`ROAD-68-G` part 8, the commit locality fixture).
#[cfg(test)]
fn zoned_city_in_world(side_m: f32, world_m: f32) -> SimCore {
    let (mut core, row_z, half_x) =
        street_grid_in_world(side_m, crosses_for(side_m), Some(world_m));
    paint_districts(&mut core, &row_z, half_x);
    fill_lots(&mut core);
    core
}

// Zones every row street in the repeating district pattern with the player's brush.
fn paint_districts(core: &mut SimCore, row_z: &[f32], half_x: f32) {
    let brush = CellSelectionShape::Brush {
        radius_m: f64::from(ROW_SPACING_M) * 0.5 - 1.0,
    };
    for (row, &z) in row_z.iter().enumerate() {
        let z = f64::from(z);
        paint(
            core,
            DISTRICTS[row % DISTRICTS.len()],
            brush,
            &[
                DVec2::new(f64::from(-half_x), z),
                DVec2::new(f64::from(half_x), z),
            ],
        );
    }
}

/// Builds the street grid of a `side_m` square city and its border connections, unzoned: one row
/// street per [`RESIDENTS_PER_BORDER`] of the city's fill continues to the world edge, evenly
/// spaced and alternating west and east. Returns the city, the z of every row street and the
/// half width of the grid along x.
#[cfg(test)]
pub(super) fn street_grid(side_m: f32) -> (SimCore, Vec<f32>, f32) {
    street_grid_with_crosses(side_m, crosses_for(side_m))
}

// `street_grid` with `crosses` cross streets instead of the side's floored count.
fn street_grid_with_crosses(side_m: f32, crosses: usize) -> (SimCore, Vec<f32>, f32) {
    street_grid_in_world(side_m, crosses, None)
}

// `street_grid_with_crosses` in a world `world_m` wide, or the grid's own margin around it.
fn street_grid_in_world(
    side_m: f32,
    crosses: usize,
    world_m: Option<f32>,
) -> (SimCore, Vec<f32>, f32) {
    let rows = (side_m / ROW_SPACING_M) as usize + 1;
    let half_x = (crosses - 1) as f32 * CROSS_SPACING_M * 0.5;
    let half_z = (rows - 1) as f32 * ROW_SPACING_M * 0.5;
    let world = world_m.unwrap_or(2.0 * half_x.max(half_z) + 2.0 * WORLD_MARGIN_M);
    let mut core = SimCore {
        treasury: CityTreasury::new(FUNDED_TREASURY),
        benchmark_mode: true,
        ..SimCore::new(WorldConfig::new(world, world, 40.0, 10.0))
    };
    core.precompute_road_mesh_data();
    install_bootstrap_assets(&mut core);

    let row_z: Vec<f32> = (0..rows)
        .map(|i| i as f32 * ROW_SPACING_M - half_z)
        .collect();
    let residents = (side_m / SIDE_HEADROOM).powi(2) * RESIDENTS_PER_M2;
    let trunks = ((residents / RESIDENTS_PER_BORDER).round() as usize).clamp(1, rows);
    // Trunk k runs from the west edge when k is even and to the east edge when it is odd; a
    // single trunk is the middle row from the west.
    let trunk_rows: Vec<usize> = (0..trunks)
        .map(|k| (2 * k + 1) * rows / (2 * trunks))
        .collect();
    let edge_x = core.heightmap.half_world_extents().0 - 1.0;
    // The bulk road path of the in-game benchmark city: lanes are rebuilt once at the end.
    core.transit_network.bulk_load = true;
    // Cross streets go in one block at a time, so a row meets existing junctions instead of
    // splitting a street that runs the whole city: every split recompiles the longer remainder's
    // span, which made the grid grow with rows × crosses × city side (`ROAD-52`).
    let arterial = arterial_lanes();
    for i in 0..crosses {
        let x = i as f32 * CROSS_SPACING_M - half_x;
        for block in row_z.windows(2) {
            add_road(&mut core, (x, block[0]), (x, block[1]), arterial);
        }
    }
    for (row, &z) in row_z.iter().enumerate() {
        let (start, end, lanes) = match trunk_rows.iter().position(|&trunk| trunk == row) {
            Some(k) if k % 2 == 0 => (-edge_x, half_x, arterial),
            Some(_) => (-half_x, edge_x, arterial),
            None => (-half_x, half_x, 1),
        };
        add_road(&mut core, (start, z), (end, z), lanes);
    }
    {
        let core = &mut core;
        core.transit_network
            .finalize_bulk_load(&mut core.region_graph, &mut core.allocator);
    }
    for (k, &row) in trunk_rows.iter().enumerate() {
        let (x, z) = (if k % 2 == 0 { -edge_x } else { edge_x }, row_z[row]);
        let y = core.heightmap.sample_height_world(x, z);
        let border = core.check_border_candidate_internal(Vector3::new(x, y, z));
        assert!(border >= 0, "trunk {k} must end at the world edge");
        core.set_border_connection_internal(border as i32);
    }
    // The grid's lanes, routing and roadbed settle below, with the pockets' pieces.
    #[cfg(test)]
    add_perf_pockets(&mut core);
    #[cfg(test)]
    add_perf_roundabouts(&mut core);
    // The fixture admits households outside frames, so it installs the border connections'
    // routing rebuild (D7) now instead of holding its first frames.
    core.transit_network.settle_cch();
    core.fit_lanes_to_road_surface();
    // A bulk road build is a world-scale change: the graded terrain resets once here, as a load
    // does, not left to the player's first commit (`ROAD-68-D` part 6).
    core.subsume_world_scale_road_changes();
    core.bump_global_terrain_payload_generation();
    (core, row_z, half_x)
}

// Lanes per direction of the grid's arterials: every cross street and the border trunks. One
// (the 2-Lane Road) unless a test sets `METRUM_PERF_ARTERIALS=4lane`, which gives them the
// 4-Lane Road (2 + 2): the `ROAD-94` diagnosis variant, the one populated city whose approaches
// have lanes to choose. Not a contract tier.
fn arterial_lanes() -> i32 {
    #[cfg(test)]
    if let Ok(value) = std::env::var("METRUM_PERF_ARTERIALS") {
        assert_eq!(value, "4lane", "METRUM_PERF_ARTERIALS is 4lane");
        return 2;
    }
    1
}

// `METRUM_PERF_POCKETS=right`, `left` or `both` (test builds, `ROAD-95` rounds R5a3 and R5c1): once
// the street grid is built and before it is zoned, as a player would add them before zoning, a
// 40 m turn pocket of those roles on every arterial road's end at a junction (a left pocket on
// these undivided roads moves the oncoming side out by a lane): the border trunks' approaches,
// and the cross streets' 50 m blocks, which are too short for one (`pocket_road_too_short`).
// Zoned parcels refuse the widening (`parcel_overlap`), so a built city takes none; and the 1+1
// row streets get none: their pockets would cost the back-to-back lots behind them (with them the
// wider grid housed 9,302 residents for 10,000). The requests are listed before the first edit, by edge id and then its
// end node first, its start second: a pocket at the end keeps the road's id on the part that
// still reaches its start junction. Prints the placed pockets and the refusals by reason on a
// `PERF_POCKETS` line. Not a contract tier.
#[cfg(test)]
fn add_perf_pockets(core: &mut SimCore) {
    use crate::simulation::network::pockets::PocketRequest;
    use crate::simulation::network::roadbed::EdgeEnd;
    use std::collections::BTreeMap;

    let Ok(value) = std::env::var("METRUM_PERF_POCKETS") else {
        return;
    };
    let (left, right) = match value.as_str() {
        "right" => (false, true),
        "left" => (true, false),
        "both" => (true, true),
        _ => panic!("METRUM_PERF_POCKETS is right, left or both"),
    };
    let graph = &core.region_graph;
    let junction = |node: u32| graph.node_adjacency(graph.get_valid_node(node)).len() >= 3;
    let requests: Vec<PocketRequest> = (0..graph.edge_count())
        .filter(|&edge| {
            let road = graph.edge(edge);
            !road.deleted && road.fwd_lanes >= 2 && road.bkw_lanes >= 2
        })
        .flat_map(|edge| {
            let road = graph.edge(edge);
            [
                (EdgeEnd::End, road.end_node),
                (EdgeEnd::Start, road.start_node),
            ]
            .into_iter()
            .filter(|&(_, node)| junction(node))
            .map(move |(end, _)| PocketRequest {
                edge,
                end,
                left,
                right,
                body_m: 40.0,
            })
        })
        .collect();
    let mut placed = 0;
    let mut refused: BTreeMap<String, u32> = BTreeMap::new();
    for request in requests {
        if core.set_road_pocket_internal(request) {
            placed += 1;
        } else {
            let reason = core
                .last_road_rejection
                .take()
                .map_or_else(|| "unknown".to_owned(), |rejection| rejection.reason);
            *refused.entry(reason).or_default() += 1;
        }
    }
    // Each edit rebuilt its lanes locally, appending lane ids; the grid's lanes are rebuilt whole,
    // numbered as the bulk build and a load number them, so the prepared snapshot's continuation
    // check compares like with like (it compares raw lane ids).
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    eprintln!(
        "PERF_POCKETS {}",
        serde_json::json!({"placed": placed, "refused": refused})
    );
}

// `METRUM_PERF_ROUNDABOUTS=n` (test builds, `ROAD-62` round R7c, D17): once the street grid is
// built and before it is zoned, as `METRUM_PERF_POCKETS` does, up to n junctions become rings at
// the default diameter through the tool's own transaction, the highest degree first and in id
// order. Every junction of the 1+1 city is 1+1; on `METRUM_PERF_ARTERIALS=4lane` each has a 2+2
// arm, so its rings have two lanes. A junction the transaction refuses is passed over, a block's
// neighbour of a ring most often. Prints the rings placed, the junctions they replace and the
// refusals by reason on a `PERF_ROUNDABOUTS` line. Not a contract tier.
#[cfg(test)]
fn add_perf_roundabouts(core: &mut SimCore) {
    use crate::simulation::network::roundabouts::{
        RoundaboutRequest, RoundaboutSite, default_diameter_m,
    };
    use std::collections::BTreeMap;

    let Ok(value) = std::env::var("METRUM_PERF_ROUNDABOUTS") else {
        return;
    };
    let wanted: usize = value
        .parse()
        .expect("METRUM_PERF_ROUNDABOUTS must be an integer");
    let graph = &core.region_graph;
    let mut junctions: Vec<(usize, u32)> = (0..graph.node_count() as u32)
        .map(|node| (graph.node_adjacency(node).len(), node))
        .filter(|&(degree, _)| degree >= 3)
        .collect();
    junctions.sort_unstable_by_key(|&(degree, node)| (std::cmp::Reverse(degree), node));
    // `METRUM_PERF_RINGS_SKIP_TRUNK` (round R7c2, a measurement): the junctions on a border trunk
    // stay plain, so the rings stand where only the grid's own traffic crosses them. Off by
    // default: D17's selection is unchanged.
    let trunk_junctions: Vec<u32> = if std::env::var_os("METRUM_PERF_RINGS_SKIP_TRUNK").is_some() {
        let mut borders = Vec::new();
        crate::simulation::economy::logistics::collect_connected_border_nodes(
            &core.region_graph,
            &mut borders,
        );
        let neighbours = |&node: &u32| -> Vec<u32> {
            let adjacent = core.region_graph.node_adjacency(node).iter();
            adjacent
                .map(|&edge| core.region_graph.edge(edge))
                .map(|edge| {
                    if edge.start_node == node {
                        edge.end_node
                    } else {
                        edge.start_node
                    }
                })
                .collect()
        };
        borders.iter().flat_map(neighbours).collect()
    } else {
        Vec::new()
    };
    let mut placed = Vec::new();
    let mut refused: BTreeMap<String, u32> = BTreeMap::new();
    for (_, node) in junctions {
        if placed.len() == wanted {
            break;
        }
        if trunk_junctions.contains(&node) {
            continue;
        }
        let site = RoundaboutSite::Junction(node);
        let diameter_m = default_diameter_m(site.lanes(&core.region_graph));
        if core.place_roundabout_internal(RoundaboutRequest { site, diameter_m }) {
            placed.push(node);
        } else {
            let reason = core
                .last_road_rejection
                .take()
                .map_or_else(|| "unknown".to_owned(), |rejection| rejection.reason);
            *refused.entry(reason).or_default() += 1;
        }
    }
    // A load drops the junctions the rings replaced and renumbers the nodes, and the prepared
    // snapshot's continuation check compares raw ids: the grid goes through a save here, so the
    // city is built on the ids a load gives it.
    let path = std::env::temp_dir().join(format!("metrum_rings_{}.sqlite", std::process::id()));
    let path_text = path.to_str().expect("a temporary path");
    core.save_game_settled_at(path_text, None, 0)
        .expect("the street grid saves");
    core.load_game_internal(path_text)
        .expect("the street grid loads");
    let _ = std::fs::remove_file(&path);
    eprintln!(
        "PERF_ROUNDABOUTS {}",
        serde_json::json!({"placed": placed.len(), "junctions": placed, "refused": refused})
    );
}

fn add_road(core: &mut SimCore, (x0, z0): (f32, f32), (x1, z1): (f32, f32), lanes: i32) {
    let outcome = core.add_road_internal(
        vec![Vector3::new(x0, 0.0, z0), Vector3::new(x1, 0.0, z1)],
        lanes,
        lanes,
    );
    assert!(
        outcome.committed,
        "road ({x0}, {z0}) -> ({x1}, {z1}) rejected: {}",
        core.last_road_timing
    );
}

// Spawns a building on every legal lot through the demand executor, then completes construction.
fn fill_lots(core: &mut SimCore) {
    loop {
        core.prepare_cell_lots_internal();
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        core.transit_network
            .roadbed
            .compile_dirty(&core.region_graph);
        let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
            &core.zoning,
            &core.region_graph,
            core.demand.runtime_catalog(),
            &[],
            BuildingSiteEnvironment {
                road_surface: &core.transit_network.roadbed,
                terrain: &core.heightmap,
            },
        );
        let mut plan = DemandBuildingActionPlan::default();
        for (spawns, candidates) in [
            (&mut plan.residential.spawns, candidates.residential),
            (&mut plan.commercial.spawns, candidates.commercial),
            (&mut plan.industrial.spawns, candidates.industrial),
        ] {
            spawns.extend(candidates.into_iter().map(|candidate| candidate.action));
        }
        let execution = core.allocator.execute_demand_building_actions(
            &plan,
            &mut core.zoning,
            &mut core.agents,
            &mut core.households,
            &mut core.logistics,
            &mut core.treasury.balance,
            &core.region_graph,
            &core.transit_network,
            &core.heightmap,
            core.demand.runtime_catalog(),
            core.demand.runtime_tuning(),
            &mut (),
        );
        core.publish_pending_building_site_changes();
        let placed = execution.residential.spawn_executed
            + execution.commercial.spawn_executed
            + execution.industrial.spawn_executed;
        if placed == 0 {
            break;
        }
    }
    core.allocator.complete_all_construction();
}
