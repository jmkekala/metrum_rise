// SPDX-License-Identifier: GPL-2.0-only

//! `SIM-02-A` guards (`docs/simulation.md#behaviour-envelope-sim-02-a`): the behaviour
//! envelope and burst limits, economy invariants, and determinism checks.
//!
//! Every check builds a fresh populated city with the engine under test, because saves break
//! during `SIM-02`. Counters come from the test-only [`crate::debug::work`] and
//! [`crate::debug::money`] ledgers, so these tests run serially through `tools/sim02_gate.py`.

use super::super::frame::FRAME_DT_S;
use super::super::state::SimCore;
use super::PopulatedCity;
use crate::debug::money::{MONEY_FLOW_COUNT, MONEY_FLOW_NAMES, snapshot as money_snapshot};
use crate::debug::work::{Work, snapshot as work_snapshot};
use crate::simulation::economy::households::REPLENISHMENT_SHOPPING_TO_STORE;
use crate::simulation::economy::logistics::{ShipmentEndpoint, ShipmentStatus};
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

const MINUTES_PER_HOUR: u64 = 60;
const MINUTES_PER_DAY: u64 = 24 * 60;
const ENVELOPE_DAYS: u32 = 14;
// Hours with fewer departures of a kind are too sparse for a share to mean anything.
const BURST_MIN_DEPARTURES: u64 = 40;
const BURST_KINDS: [(Work, &str); 3] = [
    (Work::FreightDispatches, "freight"),
    (Work::ArrivalLaunches, "arrival"),
    (Work::ShopperDispatches, "shopping"),
];
const VIOLATION_EXAMPLES: usize = 8;

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .map(|value| {
            value
                .parse()
                .unwrap_or_else(|_| panic!("{name} must be an integer"))
        })
        .unwrap_or(default)
}

fn absolute_minute(core: &SimCore) -> u64 {
    u64::from(core.time.day_index.saturating_sub(1)) * MINUTES_PER_DAY
        + u64::from(core.time.minute_of_day)
}

// Runtime guard: the ignored checks must still compile in a debug test build.
#[allow(clippy::assertions_on_constants)]
fn release_only() {
    assert!(
        !cfg!(debug_assertions),
        "SIM-02 guards build populated cities; run them in a release test build"
    );
    // Full-book pass fingerprints are a separate correctness oracle with their own tests.
    crate::simulation::economy::demand::VERIFY_LIVE_PASSES
        .store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Raises commercial and industrial property tax to the fiscal-stress fixture's level.
// Every tenth household leaves at once: its home and jobs open, so demand admits newcomers
// over the next days and the arrival burst limit has hours to check (`SIM-02-E`).
fn apply_arrival_wave(city: &mut PopulatedCity) {
    let core = &mut city.core;
    let mut leaving: Vec<usize> = (0..core.households.households.len()).step_by(10).collect();
    core.households
        .remove_households(&mut leaving, &mut core.agents, &mut core.allocator);
}

pub(super) fn apply_fiscal_stress(city: &mut PopulatedCity) {
    for policy in [
        crate::simulation::economy::fiscal::POLICY_COMMERCIAL_PROPERTY_TAX,
        crate::simulation::economy::fiscal::POLICY_INDUSTRIAL_PROPERTY_TAX,
    ] {
        assert!(city.core.set_fiscal_policy_value(policy, 500.0));
    }
}

// ---------------------------------------------------------------------------------------------
// Fingerprints

/// Layout-independent state hash: plain values only, in dense order, so a change of entity
/// storage or references that leaves the simulation identical keeps the hash. Named parts let a
/// failed comparison say where the states first differ.
pub(super) fn semantic_fingerprint(city: &PopulatedCity) -> Vec<(&'static str, u64)> {
    let core = &city.core;
    let mut parts = Vec::with_capacity(8);
    let mut part = |name: &'static str, fill: &dyn Fn(&mut DefaultHasher)| {
        let mut hasher = DefaultHasher::new();
        fill(&mut hasher);
        parts.push((name, hasher.finish()));
    };
    part("clock", &|h| {
        (core.time.day_index, core.time.minute_of_day).hash(h);
        core.time.time_elapsed.to_bits().hash(h);
    });
    part("record", &|h| {
        let record = city.record();
        (
            record.households,
            record.residents,
            record.employed,
            record.jobs,
            record.buildings,
            record.vacant_homes,
            record.vacancy_reserve,
            record.unhoused_households,
            record.net_open_jobs,
            record.departures,
        )
            .hash(h);
        record.treasury.to_bits().hash(h);
    });
    part("agents", &|h| {
        let a = &core.agents.agents;
        a.len().hash(h);
        for i in 0..a.len() {
            (
                a.pos_x[i].to_bits(),
                a.pos_y[i].to_bits(),
                a.speed[i].to_bits(),
                a.lane_distance[i].to_bits(),
                a.activity[i],
                a.transit[i],
                a.transit_mode[i],
                a.age_group[i],
                a.pending_household_size[i],
                a.freight_shipment_id[i],
                a.happiness[i].to_bits(),
            )
                .hash(h);
        }
    });
    part("buildings", &|h| {
        core.allocator.buildings.len().hash(h);
        for b in &core.allocator.buildings {
            (
                b.parcel_id,
                b.build_generation,
                b.economy_profile_runtime_id,
                b.level,
                b.occupancy,
                b.worker_count,
                b.is_deserted,
                b.operating_budget.to_bits(),
                b.revenue.to_bits(),
            )
                .hash(h);
            for units in &b.resource_inventory {
                units.to_bits().hash(h);
            }
        }
    });
    part("households", &|h| {
        core.households.households.len().hash(h);
        for household in &core.households.households {
            (
                household.budget.to_bits(),
                household.stock.to_bits(),
                household.member_count,
                household.replenishment_state,
                household.reserved_amount.to_bits(),
                household.reserved_total_cost.to_bits(),
            )
                .hash(h);
        }
    });
    part("shipments", &|h| {
        for shipment in &core.logistics.shipments {
            (
                shipment.id,
                shipment.status.code(),
                shipment.resource_runtime_id,
                shipment.amount.to_bits(),
                shipment.total_cost.to_bits(),
                shipment.freight_cost.to_bits(),
            )
                .hash(h);
        }
    });
    part("treasury", &|h| {
        core.treasury.balance.to_bits().hash(h);
        core.pending_demand_spawns.len().hash(h);
    });
    parts
}

/// Order-insensitive hash of every saved table, so a reload or repeat run can be compared
/// against everything the save format persists. The save header is excluded: it holds the
/// wall-clock save time.
pub(super) fn save_fingerprint(core: &SimCore, path: &Path) -> BTreeMap<String, u64> {
    core.save_game_internal(path.to_str().expect("UTF-8 save path"), None)
        .unwrap_or_else(|error| panic!("could not save fingerprint city: {error}"));
    let connection = rusqlite::Connection::open(path).unwrap();
    let mut tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    tables.retain(|table| table != "save_meta");
    let mut fingerprints = BTreeMap::new();
    for table in tables {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM \"{table}\""))
            .unwrap();
        let columns = statement.column_count();
        let mut rows: Vec<u64> = statement
            .query_map([], |row| {
                let values: Vec<rusqlite::types::Value> = (0..columns)
                    .map(|column| row.get(column))
                    .collect::<Result<_, _>>()?;
                Ok(crate::debug::work::debug_hash(&values))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows.sort_unstable();
        fingerprints.insert(table, crate::debug::work::debug_hash(&rows));
    }
    drop(connection);
    std::fs::remove_file(path).unwrap();
    fingerprints
}

fn scratch_save_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "metrum_sim02_{label}_{}_{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

// ---------------------------------------------------------------------------------------------
// Invariants

/// Money held inside the city outside the treasury: business and household cash, shoppers'
/// reserved purchases, paid shipments still travelling to a building and the service-sales pool
/// between households and stores (`SIM-02-I`).
fn private_cash(core: &SimCore) -> f64 {
    let buildings: f64 = core
        .allocator
        .buildings
        .iter()
        .map(|b| f64::from(b.operating_budget))
        .sum();
    let households: f64 = core
        .households
        .households
        .iter()
        .map(|h| {
            f64::from(h.budget)
                + if h.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE {
                    f64::from(h.reserved_total_cost)
                } else {
                    0.0
                }
        })
        .sum();
    let escrow: f64 = core
        .logistics
        .shipments
        .iter()
        .filter(|s| {
            s.status == ShipmentStatus::InTransit
                && matches!(s.destination, ShipmentEndpoint::Building(_))
        })
        .map(|s| f64::from(s.total_cost))
        .sum();
    buildings + households + escrow + core.households.service_sales.money()
}

/// Running money-conservation check: the change of city money between two checks must equal
/// the recorded boundary flows. The treasury enters as an exact `f64` difference, so the large
/// fixture treasury does not hide small residuals.
struct MoneyCheck {
    treasury: f64,
    private_cash: f64,
    flows: [f64; MONEY_FLOW_COUNT],
}

impl MoneyCheck {
    fn new(core: &SimCore) -> Self {
        Self {
            treasury: core.treasury.balance,
            private_cash: private_cash(core),
            flows: money_snapshot(),
        }
    }

    /// Residual since the previous check, and the recorded flows by kind.
    fn advance(&mut self, core: &SimCore) -> (f64, [f64; MONEY_FLOW_COUNT]) {
        let next = Self::new(core);
        let flows: [f64; MONEY_FLOW_COUNT] = std::array::from_fn(|i| next.flows[i] - self.flows[i]);
        let change = (next.treasury - self.treasury) + (next.private_cash - self.private_cash);
        *self = next;
        (change - flows.iter().sum::<f64>(), flows)
    }
}

/// Reference, occupancy and job consistency. Returns violation counts by rule and a few
/// examples. Job counts are only rebuilt by the economy, so `jobs` asks for that rule too.
fn reference_violations(core: &SimCore, jobs: bool) -> (BTreeMap<&'static str, u64>, Vec<String>) {
    let mut counts = BTreeMap::new();
    let mut examples = Vec::new();
    let mut fail = |rule: &'static str, detail: String| {
        *counts.entry(rule).or_insert(0) += 1;
        if examples.len() < VIOLATION_EXAMPLES {
            examples.push(format!("{rule}: {detail}"));
        }
    };
    let buildings = core.allocator.buildings.len();
    let households = &core.households.households;
    let agents = &core.agents.agents;
    let shipments = &core.logistics.shipments;
    let building_ref = |index: usize| index == usize::MAX || index < buildings;
    let mut homed = vec![0_u32; buildings];
    let mut workers = vec![0_u32; buildings];
    for i in 0..agents.len() {
        for (rule, index) in [
            ("agent_home", agents.home_building[i]),
            ("agent_work", agents.work_building[i]),
            ("agent_current", agents.current_building[i]),
            ("agent_target", agents.target_building[i]),
        ] {
            if !building_ref(index) {
                fail(rule, format!("agent {i} -> building {index}"));
            }
        }
        if let Some(count) = workers.get_mut(agents.work_building[i]) {
            *count += 1;
        }
        let household = agents.household_id[i];
        if agents.pending_household_size[i] > 0 {
            if let Some(count) = homed.get_mut(agents.home_building[i]) {
                *count += 1;
            }
        } else if household != usize::MAX {
            match households.get(household) {
                None => fail(
                    "agent_household",
                    format!("agent {i} -> household {household}"),
                ),
                Some(record) if record.home_building_id != agents.home_building[i] => fail(
                    "agent_household_home",
                    format!(
                        "agent {i} home {} household {household} home {}",
                        agents.home_building[i], record.home_building_id
                    ),
                ),
                Some(_) => {}
            }
        }
        let shipment_id = agents.freight_shipment_id[i];
        if shipment_id != u64::MAX {
            match shipments.binary_search_by_key(&shipment_id, |s| s.id) {
                Ok(position)
                    if core.agents.resolve_agent(shipments[position].carrier) != Some(i) =>
                {
                    fail(
                        "carrier_shipment",
                        format!(
                            "agent {i} carries shipment {shipment_id} assigned to {:?}",
                            shipments[position].carrier
                        ),
                    )
                }
                Ok(_) => {}
                Err(_) => fail(
                    "carrier_shipment_missing",
                    format!("agent {i} carries missing shipment {shipment_id}"),
                ),
            }
        }
    }
    // An admitted household waiting for the `admissions` queue holds its home's place
    // (`SIM-02-J`).
    for admission in &core.households.admission_requests {
        if let Some(home) = core.allocator.building_handles.resolve(admission.home) {
            homed[home] += 1;
        }
    }
    for (id, household) in households.iter().enumerate() {
        if !building_ref(household.home_building_id) {
            fail(
                "household_home",
                format!("household {id} -> building {}", household.home_building_id),
            );
        }
        if let Some(count) = homed.get_mut(household.home_building_id) {
            *count += 1;
        }
        if household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE
            && household.reserved_store_building_id >= buildings
        {
            fail(
                "household_store",
                format!(
                    "household {id} reserved at building {}",
                    household.reserved_store_building_id
                ),
            );
        }
    }
    for shipment in shipments {
        for endpoint in [shipment.source, shipment.destination] {
            if let ShipmentEndpoint::Building(index) = endpoint
                && index >= buildings
            {
                fail(
                    "shipment_building",
                    format!("shipment {} -> building {index}", shipment.id),
                );
            }
        }
        if !shipment.carrier.is_none()
            && core
                .agents
                .resolve_agent(shipment.carrier)
                .is_none_or(|carrier| agents.freight_shipment_id[carrier] != shipment.id)
        {
            fail(
                "shipment_carrier",
                format!("shipment {} -> {:?}", shipment.id, shipment.carrier),
            );
        }
    }
    for (index, building) in core.allocator.buildings.iter().enumerate() {
        if building.occupancy != homed[index] {
            fail(
                "occupancy",
                format!(
                    "building {index} occupancy {} homes {}",
                    building.occupancy, homed[index]
                ),
            );
        }
        let capacity = core.allocator.household_capacity(index);
        if capacity > 0 && homed[index] > capacity {
            fail(
                "occupancy_capacity",
                format!("building {index} homes {} of {capacity}", homed[index]),
            );
        }
        if jobs && building.worker_count != workers[index] {
            fail(
                "worker_count",
                format!(
                    "building {index} worker_count {} workers {}",
                    building.worker_count, workers[index]
                ),
            );
        }
    }
    (counts, examples)
}

// ---------------------------------------------------------------------------------------------
// Envelope run

/// Per-minute departures of each burst kind within the current hour, and per-hour settlements
/// within the current day.
struct BurstTracker {
    work: [u64; crate::debug::work::WORK_COUNT],
    hour: u64,
    minutes: [[u64; MINUTES_PER_HOUR as usize]; BURST_KINDS.len()],
    settlement_hours: [u64; 24],
    // Day results: worst minute share and qualifying hours per kind; worst settlement hour share.
    worst_minute_share: [f64; BURST_KINDS.len()],
    qualifying_hours: [u32; BURST_KINDS.len()],
    departures: [u64; BURST_KINDS.len()],
}

impl BurstTracker {
    fn new(core: &SimCore) -> Self {
        let minute = absolute_minute(core);
        crate::debug::work::log_departures(true);
        Self {
            work: work_snapshot(),
            hour: minute / MINUTES_PER_HOUR,
            minutes: [[0; MINUTES_PER_HOUR as usize]; BURST_KINDS.len()],
            settlement_hours: [0; 24],
            worst_minute_share: [0.0; BURST_KINDS.len()],
            qualifying_hours: [0; BURST_KINDS.len()],
            departures: [0; BURST_KINDS.len()],
        }
    }

    fn close_hour(&mut self) {
        for kind in 0..BURST_KINDS.len() {
            let total: u64 = self.minutes[kind].iter().sum();
            self.departures[kind] += total;
            if total >= BURST_MIN_DEPARTURES {
                let busiest = *self.minutes[kind].iter().max().unwrap();
                self.qualifying_hours[kind] += 1;
                self.worst_minute_share[kind] =
                    self.worst_minute_share[kind].max(busiest as f64 / total as f64);
            }
            self.minutes[kind] = [0; MINUTES_PER_HOUR as usize];
        }
    }

    /// Counts each departure in the minute it left in, and settlements in the hour the frame
    /// ended in.
    fn after_frame(&mut self, core: &SimCore) {
        for (work, minute) in crate::debug::work::take_departures() {
            let Some(kind) = BURST_KINDS.iter().position(|(counter, _)| *counter == work) else {
                continue;
            };
            if minute / MINUTES_PER_HOUR > self.hour {
                self.close_hour();
                self.hour = minute / MINUTES_PER_HOUR;
            }
            self.minutes[kind][(minute % MINUTES_PER_HOUR) as usize] += 1;
        }
        let minute = absolute_minute(core);
        if minute / MINUTES_PER_HOUR != self.hour {
            self.close_hour();
            self.hour = minute / MINUTES_PER_HOUR;
        }
        let work = work_snapshot();
        let settled = |work: &[u64]| {
            work[Work::BuildingSettlements as usize] + work[Work::HouseholdSettlements as usize]
        };
        self.settlement_hours[((minute % MINUTES_PER_DAY) / MINUTES_PER_HOUR) as usize] +=
            settled(&work) - settled(&self.work);
        self.work = work;
    }

    /// Ends the day: burst shares by kind and the busiest settlement hour's share.
    fn close_day(&mut self) -> serde_json::Value {
        let settlements: u64 = self.settlement_hours.iter().sum();
        let busiest_settlement_hour = self.settlement_hours.iter().copied().max().unwrap_or(0);
        let mut kinds = serde_json::Map::new();
        for (kind, (_, name)) in BURST_KINDS.iter().enumerate() {
            kinds.insert(
                (*name).into(),
                serde_json::json!({
                    "departures":self.departures[kind],
                    "qualifying_hours":self.qualifying_hours[kind],
                    "worst_minute_share":self.worst_minute_share[kind],
                }),
            );
        }
        let value = serde_json::json!({
            "kinds":kinds,
            "settlements":settlements,
            "worst_settlement_hour_share":if settlements > 0 {
                busiest_settlement_hour as f64 / settlements as f64
            } else {
                0.0
            },
        });
        self.settlement_hours = [0; 24];
        self.worst_minute_share = [0.0; BURST_KINDS.len()];
        self.qualifying_hours = [0; BURST_KINDS.len()];
        self.departures = [0; BURST_KINDS.len()];
        value
    }
}

fn out_of_stock_households(core: &SimCore) -> u32 {
    core.households
        .households
        .iter()
        .filter(|h| h.member_count > 0 && h.stock <= 0.0)
        .count() as u32
}

#[test]
#[ignore = "SIM-02-A behaviour envelope: one fresh populated city over 14 game days"]
fn sim02_envelope() {
    release_only();
    let residents = env_u32("METRUM_SIM02_RESIDENTS", 10_000);
    let workload = std::env::var("METRUM_SIM02_WORKLOAD").unwrap_or_else(|_| "baseline".into());
    if !matches!(workload.as_str(), "baseline" | "fiscal_stress" | "arrivals") {
        panic!("unknown METRUM_SIM02_WORKLOAD {workload}");
    }
    let days = env_u32("METRUM_SIM02_DAYS", ENVELOPE_DAYS);
    let started = std::time::Instant::now();
    let mut city = PopulatedCity::build(residents);
    let build_s = started.elapsed().as_secs_f64();
    match workload.as_str() {
        "fiscal_stress" => apply_fiscal_stress(&mut city),
        "arrivals" => apply_arrival_wave(&mut city),
        _ => {}
    }
    let mut bursts = BurstTracker::new(&city.core);
    let mut money = MoneyCheck::new(&city.core);
    let mut day_work = work_snapshot();
    let mut day_treasury = city.core.treasury.balance;
    let mut day_flows = [0.0; MONEY_FLOW_COUNT];
    let mut day_money_residual = 0.0_f64;
    let mut max_hour_money_residual = 0.0_f64;
    let mut violations: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut examples = Vec::new();
    let mut hour = absolute_minute(&city.core) / MINUTES_PER_HOUR;
    let mut day = city.core.time.day_index;
    let mut sampled = 0;
    while sampled < days {
        city.core.step_frame(FRAME_DT_S);
        bursts.after_frame(&city.core);
        let now_hour = absolute_minute(&city.core) / MINUTES_PER_HOUR;
        if now_hour == hour {
            continue;
        }
        hour = now_hour;
        let (residual, flows) = money.advance(&city.core);
        day_money_residual += residual;
        max_hour_money_residual = max_hour_money_residual.max(residual.abs());
        for (total, flow) in day_flows.iter_mut().zip(flows) {
            *total += flow;
        }
        let (counts, found) = reference_violations(&city.core, true);
        for (rule, count) in counts {
            *violations.entry(rule).or_insert(0) += count;
        }
        if examples.len() < VIOLATION_EXAMPLES {
            examples.extend(found);
        }
        if city.core.time.day_index == day {
            continue;
        }
        day = city.core.time.day_index;
        if city.core.time.minute_of_day == 0 {
            city.departures += city.core.demand.housing_balance().2;
        }
        let work = work_snapshot();
        let delta = |counter: Work| work[counter as usize] - day_work[counter as usize];
        let record = city.record();
        let flows: serde_json::Map<_, _> = MONEY_FLOW_NAMES
            .iter()
            .zip(day_flows)
            .map(|(name, value)| (name.to_string(), value.into()))
            .collect();
        eprintln!(
            "SIM02_DAY {}",
            serde_json::json!({
                "residents":residents,
                "workload":workload,
                "sample":sampled, "day":record.day,
                "households":record.households, "residents_now":record.residents,
                "employed":record.employed, "jobs":record.jobs, "buildings":record.buildings,
                "vacant_homes":record.vacant_homes, "vacancy_reserve":record.vacancy_reserve,
                "unhoused_households":record.unhoused_households,
                "net_open_jobs":record.net_open_jobs, "departures":record.departures,
                "net_income":record.treasury - day_treasury,
                "bankruptcies":delta(Work::Desertions),
                "freight_deliveries":delta(Work::FreightDeliveries),
                "out_of_stock_households":out_of_stock_households(&city.core),
                "trip_failures":delta(Work::TripFailures),
                "buildings_placed":delta(Work::BuildingsPlaced),
                "buildings_removed":delta(Work::BuildingsRemoved),
                "money_residual":day_money_residual,
                "money_flows":flows,
                "bursts":bursts.close_day(),
                "fingerprint":crate::debug::work::debug_hash(&semantic_fingerprint(&city)),
            })
        );
        day_work = work;
        day_treasury = record.treasury;
        day_flows = [0.0; MONEY_FLOW_COUNT];
        day_money_residual = 0.0;
        sampled += 1;
    }
    eprintln!(
        "SIM02_ENVELOPE {}",
        serde_json::json!({
            "residents":residents,
            "workload":workload,
            "days":days, "build_s":build_s, "total_s":started.elapsed().as_secs_f64(),
            "max_hour_money_residual":max_hour_money_residual,
            "violations":violations, "violation_examples":examples,
            "record":format!("{:?}", city.record()),
        })
    );
}

// ---------------------------------------------------------------------------------------------
// Determinism

/// Hourly semantic fingerprints over `days` game days after the build, then the final save's.
type Run = (Vec<Vec<(&'static str, u64)>>, BTreeMap<String, u64>);

fn fingerprinted_run(residents: u32, days: u32, label: &str) -> Run {
    let mut city = PopulatedCity::build(residents);
    let mut hourly = Vec::new();
    let end_day = city.core.time.day_index + days;
    let mut hour = absolute_minute(&city.core) / MINUTES_PER_HOUR;
    while city.core.time.day_index < end_day {
        city.core.step_frame(FRAME_DT_S);
        let now = absolute_minute(&city.core) / MINUTES_PER_HOUR;
        if now != hour {
            hour = now;
            hourly.push(semantic_fingerprint(&city));
        }
    }
    let saved = save_fingerprint(&city.core, &scratch_save_path(label));
    (hourly, saved)
}

fn first_difference(reference: &Run, other: &Run) -> Option<String> {
    for (hour, (a, b)) in reference.0.iter().zip(&other.0).enumerate() {
        if let Some(((name, _), _)) = a.iter().zip(b).find(|(x, y)| x != y) {
            return Some(format!("hour {hour}: {name}"));
        }
    }
    if reference.0.len() != other.0.len() {
        return Some(format!(
            "hour count {} vs {}",
            reference.0.len(),
            other.0.len()
        ));
    }
    let tables: Vec<&String> = reference
        .1
        .iter()
        .filter(|(table, hash)| other.1.get(*table) != Some(hash))
        .map(|(table, _)| table)
        .collect();
    (!tables.is_empty() || reference.1.len() != other.1.len())
        .then(|| format!("saved tables {tables:?}"))
}

#[test]
#[ignore = "SIM-02-A determinism: repeat run, worker counts and save/reload continuation"]
fn sim02_determinism() {
    release_only();
    let residents = env_u32("METRUM_SIM02_RESIDENTS", 5_000);
    let days = env_u32("METRUM_SIM02_DAYS", 3);
    let reference = fingerprinted_run(residents, days, "reference");
    let mut results = serde_json::Map::new();
    let repeat = fingerprinted_run(residents, days, "repeat");
    results.insert(
        "repeat".into(),
        first_difference(&reference, &repeat).into(),
    );
    for threads in [1, 3] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let run = pool.install(|| fingerprinted_run(residents, days, "threads"));
        results.insert(
            format!("threads_{threads}"),
            first_difference(&reference, &run).into(),
        );
    }
    results.insert("reload".into(), reload_continuation(residents, days).into());
    eprintln!(
        "SIM02_DETERMINISM {}",
        serde_json::json!({"residents":residents,"days":days,"hours":reference.0.len(),
            "differences":results})
    );
    assert!(
        results.values().all(serde_json::Value::is_null),
        "determinism differences: {results:?}"
    );
}

/// Saves mid-hour, between frames, then runs the straight and reloaded cities side by side
/// across the next midnight. Returns the first difference.
fn reload_continuation(residents: u32, days: u32) -> Option<String> {
    let mut straight = PopulatedCity::build(residents);
    // 13:37 on the last day: mid-hour, with commutes, shopping and freight in flight.
    let target = (u64::from(straight.core.time.day_index) + u64::from(days) - 2) * MINUTES_PER_DAY
        + 13 * MINUTES_PER_HOUR
        + 37;
    while absolute_minute(&straight.core) < target {
        straight.core.step_frame(FRAME_DT_S);
    }
    let path = scratch_save_path("reload");
    straight
        .core
        .save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    let mut loaded = PopulatedCity {
        core: SimCore {
            benchmark_mode: true,
            ..SimCore::new(crate::simulation::core::config::WorldConfig::new(
                2048.0, 2048.0, 40.0, 10.0,
            ))
        },
        departures: straight.departures,
    };
    super::super::scenario::install_bootstrap_assets(&mut loaded.core);
    loaded
        .core
        .load_game_internal(path.to_str().unwrap())
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    loaded.core.time.speed_multiplier = straight.core.time.speed_multiplier;
    let end = target + 26 * MINUTES_PER_HOUR;
    let mut hour = target / MINUTES_PER_HOUR;
    while absolute_minute(&straight.core) < end {
        straight.core.step_frame(FRAME_DT_S);
        loaded.core.step_frame(FRAME_DT_S);
        let now = absolute_minute(&straight.core) / MINUTES_PER_HOUR;
        if now == hour {
            continue;
        }
        hour = now;
        let (a, b) = (
            semantic_fingerprint(&straight),
            semantic_fingerprint(&loaded),
        );
        if let Some(((name, _), _)) = a.iter().zip(&b).find(|(x, y)| x != y) {
            return Some(format!("hour {}: {name}", now - target / MINUTES_PER_HOUR));
        }
    }
    let a = save_fingerprint(&straight.core, &scratch_save_path("straight"));
    let b = save_fingerprint(&loaded.core, &scratch_save_path("loaded"));
    let tables: Vec<&String> = a
        .iter()
        .filter(|(table, hash)| b.get(*table) != Some(hash))
        .map(|(table, _)| table)
        .collect();
    (!tables.is_empty()).then(|| format!("saved tables {tables:?}"))
}

#[test]
#[ignore = "SIM-02-A invariants on a short populated run; release only"]
fn sim02_invariants_smoke() {
    release_only();
    let mut city = PopulatedCity::build(env_u32("METRUM_SIM02_RESIDENTS", 2_000));
    let mut money = MoneyCheck::new(&city.core);
    for _ in 0..30 {
        city.advance_hour();
        let (residual, flows) = money.advance(&city.core);
        let (counts, examples) = reference_violations(&city.core, true);
        eprintln!(
            "SIM02_INVARIANT {}",
            serde_json::json!({"day":city.core.time.day_index,"minute":city.core.time.minute_of_day,
                "residual":residual,"flows":flows.to_vec(),"violations":counts,"examples":examples})
        );
    }
}
