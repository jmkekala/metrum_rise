// SPDX-License-Identifier: GPL-2.0-only

//! Pre-populated city for timing the economy ticks (`ECON-13`).
//!
//! Residential streets run every 50 m with back-to-back lots between them, crossed by a street
//! every 400 m, and are zoned in repeating row districts with the player's brush. Every lot is
//! filled through the demand spawn path with the repository's bootstrap pack, and households are
//! admitted through the demand admission path; their carriers are placed at home instead of
//! driving in, which is the only step that bypasses the game. The city then runs the hourly and
//! daily ticks on their real cadence without agent movement, so trips the economy starts never
//! finish. See `docs/economy.md#econ-13-economy-tick-baseline`.

use std::time::{Duration, Instant};

use glam::DVec2;
use godot::prelude::Vector3;

use super::budget::CityTreasury;
use super::scenario::{GrowthDayRecord, install_bootstrap_assets, paint};
use super::state::SimCore;
use crate::simulation::buildings::allocator::BuildingSiteEnvironment;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::economy::agents::TRANSIT_IN_BUILDING;
use crate::simulation::economy::demand::DemandBuildingActionPlan;
use crate::simulation::network::surface::RoadSurfaceCompileReason;
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
// tier; solvency is the growth scenario's measure, not this one.
const FUNDED_TREASURY: f64 = 1.0e15;
const WARM_UP_DAYS: u32 = 2;
const MINUTES_PER_HOUR: u16 = 60;
const MINUTES_PER_DAY: u16 = 24 * MINUTES_PER_HOUR;

/// Wall-clock cost of the economy ticks run by one [`PopulatedCity::advance_hour`] call.
#[derive(Clone, Copy, Debug)]
pub struct EconomyHourTimes {
    /// Operational-hour tick; includes the hourly demand pass when `demand_pass` is set.
    pub hourly: Duration,
    /// Daily settlement tick; present only when the hour was midnight.
    pub daily: Option<Duration>,
    /// Whether the hour ran the hourly demand pass, as every hour but midnight does.
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
    day: u32,
    minute_of_day: u16,
}

impl PopulatedCity {
    /// Builds a street grid large enough for `residents`, fills every zoned lot, admits
    /// households until at least `residents` people live in the city, then runs two simulated
    /// days so employment and inventories settle.
    ///
    /// Panics if the grid holds too few homes for `residents`, which means the sizing constants
    /// no longer match the bootstrap pack.
    pub fn build(residents: u32) -> Self {
        Self::build_timed(residents).0
    }

    /// [`Self::build`], also returning the wall-clock cost of each phase.
    pub fn build_timed(residents: u32) -> (Self, PopulatedCityBuildTimes) {
        let mut times = PopulatedCityBuildTimes::default();
        let started = Instant::now();
        let (mut core, row_z, half_x) = street_grid(side_m_for(residents));
        times.street_grid = started.elapsed();
        let started = Instant::now();
        paint_districts(&mut core, &row_z, half_x);
        times.zoning = started.elapsed();
        let started = Instant::now();
        fill_lots(&mut core);
        times.lots = started.elapsed();
        let mut city = Self {
            core,
            day: 1,
            minute_of_day: 0,
        };
        let started = Instant::now();
        city.admit_households(residents);
        times.admission = started.elapsed();
        let started = Instant::now();
        for _ in 0..WARM_UP_DAYS * 24 {
            city.advance_hour();
        }
        times.warm_up = started.elapsed();
        (city, times)
    }

    /// City totals at the current clock, in the growth scenario's record form.
    pub fn record(&self) -> GrowthDayRecord {
        let city = self.core.daily_city_flow_diagnostics();
        GrowthDayRecord {
            day: self.day,
            households: city.active_households,
            residents: city.resident_agents,
            employed: city.employed_agents,
            jobs: city
                .commercial_job_capacity
                .saturating_add(city.industrial_job_capacity),
            buildings: self.core.allocator.buildings.len() as u32,
            treasury: self.core.treasury.balance,
        }
    }

    /// Advances the clock one hour and runs the economy ticks due then, as the sim thread does
    /// at that hour boundary: the operational hour, then daily settlement at midnight.
    pub fn advance_hour(&mut self) -> EconomyHourTimes {
        self.minute_of_day += MINUTES_PER_HOUR;
        if self.minute_of_day == MINUTES_PER_DAY {
            self.minute_of_day = 0;
            self.day += 1;
        }
        let started = Instant::now();
        self.core
            .simulate_operational_hour_internal(self.day, self.minute_of_day);
        let hourly = started.elapsed();
        let daily = (self.minute_of_day == 0).then(|| {
            let started = Instant::now();
            self.core.simulate_tick_internal(self.day);
            started.elapsed()
        });
        EconomyHourTimes {
            hourly,
            daily,
            demand_pass: self.minute_of_day != 0,
        }
    }

    fn admit_households(&mut self, residents: u32) {
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
                .execute_demand_household_admission_with_preference(
                    wanted,
                    core.households.households.len(),
                    false,
                    &mut core.agents,
                    &core.transit_network,
                    &core.region_graph,
                );
            // Carriers arrive at once instead of driving in from the border; the next hour
            // materializes them as households.
            let agents = &mut core.agents.agents;
            for i in 0..agents.len() {
                if agents.pending_household_size[i] > 0 {
                    agents.transit[i] = TRANSIT_IN_BUILDING;
                    agents.current_building[i] = agents.home_building[i];
                }
            }
            self.advance_hour();
            let housed = self.core.daily_city_flow_diagnostics().resident_agents;
            if housed >= residents {
                return;
            }
            assert!(
                launched > 0,
                "{housed} residents housed, {residents} requested"
            );
        }
    }
}

fn side_m_for(residents: u32) -> f32 {
    (residents as f32 / RESIDENTS_PER_M2).sqrt() * SIDE_HEADROOM
}

/// Builds the street grid of a `side_m` square city with every zoned lot built and complete.
#[cfg(test)]
pub(super) fn zoned_city(side_m: f32) -> SimCore {
    let (mut core, row_z, half_x) = street_grid(side_m);
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

/// Builds the street grid of a `side_m` square city and its border connection, unzoned. Returns
/// the city, the z of every row street and the half width of the grid along x.
pub(super) fn street_grid(side_m: f32) -> (SimCore, Vec<f32>, f32) {
    let rows = (side_m / ROW_SPACING_M) as usize + 1;
    let crosses = (side_m / CROSS_SPACING_M) as usize + 1;
    let half_x = (crosses - 1) as f32 * CROSS_SPACING_M * 0.5;
    let half_z = (rows - 1) as f32 * ROW_SPACING_M * 0.5;
    let world = 2.0 * half_x.max(half_z) + 2.0 * WORLD_MARGIN_M;
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
    let trunk_z = row_z[rows / 2];
    let border_x = 1.0 - core.heightmap.half_world_extents().0;
    // The bulk road path of the in-game benchmark city: lanes are rebuilt once at the end.
    core.transit_network.bulk_load = true;
    for i in 0..crosses {
        let x = i as f32 * CROSS_SPACING_M - half_x;
        add_road(&mut core, (x, -half_z), (x, half_z));
    }
    for &z in &row_z {
        let start = if z == trunk_z { border_x } else { -half_x };
        add_road(&mut core, (start, z), (half_x, z));
    }
    {
        let core = &mut core;
        core.transit_network
            .finalize_bulk_load(&mut core.region_graph, &mut core.allocator);
    }
    let border_y = core.heightmap.sample_height_world(border_x, trunk_z);
    let border = core.check_border_candidate_internal(Vector3::new(border_x, border_y, trunk_z));
    assert!(border >= 0, "the trunk must end at the world edge");
    core.set_border_connection_internal(border as i32);
    (core, row_z, half_x)
}

fn add_road(core: &mut SimCore, (x0, z0): (f32, f32), (x1, z1): (f32, f32)) {
    let outcome = core.add_road_internal(
        vec![Vector3::new(x0, 0.0, z0), Vector3::new(x1, 0.0, z1)],
        1,
        1,
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
        core.transit_network.road_surface.compile_dirty_with_reason(
            &core.region_graph,
            &core.heightmap,
            RoadSurfaceCompileReason::SimCommit,
        );
        let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
            &core.zoning,
            &core.region_graph,
            core.demand.runtime_catalog(),
            &[],
            BuildingSiteEnvironment {
                road_surface: &core.transit_network.road_surface,
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
            &core.transit_network.lane_system,
            &core.transit_network.road_surface,
            &core.heightmap,
            core.demand.runtime_catalog(),
            core.demand.runtime_tuning(),
        );
        core.publish_pending_building_site_changes();
        let placed = execution.residential.spawn_executed
            + execution.commercial.spawn_executed
            + execution.industrial.spawn_executed;
        if placed == 0 {
            break;
        }
    }
    while core
        .allocator
        .buildings
        .iter()
        .any(|building| building.construction_remaining_hours > 0)
    {
        core.allocator.advance_construction_hour();
    }
}
