// SPDX-License-Identifier: GPL-2.0-only

//! Test-only work counters for the short performance contracts and scaling attribution.
//!
//! Counters separate algorithmic growth from cache effects: a tier whose counts grow 10× but
//! whose time grows faster is memory-bound. Gameplay and benchmark builds compile every
//! [`add`] call to nothing, so instrumented call sites cost no time outside tests.

/// One counted unit of simulation work. Rows are summed over every call of their pass.
// Some counters are recorded only by test-only search traces.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Work {
    /// Successful building insertions, for hourly fixture churn coverage.
    BuildingsPlaced,
    /// Successful building removals, including deserted-building cleanup.
    BuildingsRemoved,
    /// Hourly demand planning passes.
    PlanningPasses,
    /// Live hourly funding censuses run by employment.
    LiveCensuses,
    /// Residential planning slots skipped because the published census's books were not valid
    /// for the live city (`SIM-02-I`); the zone waits for the next hour.
    ResidentialPlanningSkips,
    /// Commercial planning slots skipped on books not valid for the live city (`SIM-02-I`).
    CommercialPlanningSkips,
    /// Industrial planning slots skipped on books not valid for the live city (`SIM-02-I`).
    IndustrialPlanningSkips,
    /// Action slots whose downgrade and admission planning was skipped on books not valid for
    /// the live city (`SIM-02-I`).
    ActionsPlanningSkips,
    /// Construction epochs opened at planning slots (`SIM-02-I`).
    ConstructionEpochs,
    /// Construction epochs skipped because the published census's books were not valid for
    /// the live city (`SIM-02-I`); the queue's business permits wait for the next slot.
    ConstructionEpochSkips,
    /// Building rows visited across all censuses.
    CensusBuildings,
    /// Live business/customer reservation rows copied into a planning fork.
    ReservationRowsCopied,
    /// Project owner rows prepared for planning (live buildings plus parcel slots).
    OwnerSlotsPrepared,
    /// Rebuilds of the projected supplier-source table.
    SupplierSourceRebuilds,
    /// Projected supplier sources produced by those rebuilds.
    SupplierSourcesBuilt,
    /// Rebuilds of the proposed business buyer-site table.
    BuyerSiteRebuilds,
    /// Proposed buyer sites produced by those rebuilds.
    BuyerSitesBuilt,
    /// Parcels scanned for fresh spawn sites by planning passes.
    SpawnParcelsScanned,
    /// Nearest-supplier searches.
    SupplierSearches,
    /// Supplier candidates scanned by those searches.
    SupplierCandidates,
    /// Route quotes issued by those searches.
    SupplierRoutes,
    /// Distinct household job-option keys searched.
    JobSearchKeys,
    /// CCH point searches (cost and path).
    CchPointSearches,
    /// CCH bucket many-to-many source searches.
    CchBucketSources,
    /// Operations sized by the funding census's staffing search.
    StaffingOperations,
    /// Worker counts tried by those searches.
    StaffingTrials,
    /// Freight carriers dispatched onto the road (`SIM-02` burst envelope).
    FreightDispatches,
    /// Household arrival carriers launched from the border.
    ArrivalLaunches,
    /// Household shoppers sent to a store with a reserved purchase.
    ShopperDispatches,
    /// Buildings passed through daily settlement.
    BuildingSettlements,
    /// Households passed through daily settlement.
    HouseholdSettlements,
    /// Buildings newly marked deserted.
    Desertions,
    /// Freight loads delivered to a building or the border.
    FreightDeliveries,
    /// Trip failures recorded by the movement pass.
    TripFailures,
    /// Households removed from the dense store.
    HouseholdsRemoved,
    /// Building or household index shifts and reuses: swap-removes, remaps, clears and undo
    /// restores (`SIM-02-I`).
    StructuralChanges,
    /// Funding censuses restarted by a network or structural change during their window.
    CensusRestarts,
    /// Zones whose flow fields a frame's pathing rebuild found marked (`SIM-02-I`).
    FlowFieldZonesDrained,
    /// Flow fields built by a frame's pathing rebuild, car and foot counted apart.
    FlowFieldsBuilt,
    /// Flow fields of a marked zone left as they were: the same sources on the same graph.
    FlowFieldsSkipped,
    /// Business spawns and upgrades an earlier planning slot of the hour planned that a later
    /// slot's proof rejected, dropped from the hour's plan (`SIM-02-I`).
    ReproofDrops,
    /// Building phases whose freight check a freight cooldown turned away (`SIM-02-J`).
    FreightCooldownRejections,
    /// Queued level changes rejected because their building no longer matched their permit
    /// (`SIM-02-J`).
    LevelChangePermitRejections,
    /// Queued admissions dropped because their home left or their border took no arrivals
    /// (`SIM-02-J`).
    AdmissionsDropped,
}

/// Number of [`Work`] counters.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const WORK_COUNT: usize = Work::AdmissionsDropped as usize + 1;

#[cfg(test)]
static COUNTS: [std::sync::atomic::AtomicU64; WORK_COUNT] =
    [const { std::sync::atomic::AtomicU64::new(0) }; WORK_COUNT];
// The same counters, added only while the clock processes an hour's second half: the funding
// census's window (`SIM-02-I`).
#[cfg(test)]
static WINDOW_COUNTS: [std::sync::atomic::AtomicU64; WORK_COUNT] =
    [const { std::sync::atomic::AtomicU64::new(0) }; WORK_COUNT];

// Departures of the burst kinds by the absolute minute the clock was processing, so the burst
// envelope counts each in its own minute rather than the frame that ran it (`SIM-02-E`).
#[cfg(test)]
static CLOCK_MINUTE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(test)]
static LOG_DEPARTURES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
static DEPARTURES: std::sync::Mutex<Vec<(Work, u64)>> = std::sync::Mutex::new(Vec::new());

/// Adds `amount` to one counter. Test builds only; otherwise compiled out.
#[cfg(test)]
pub(crate) fn add(work: Work, amount: u64) {
    COUNTS[work as usize].fetch_add(amount, std::sync::atomic::Ordering::Relaxed);
    if CLOCK_MINUTE.load(std::sync::atomic::Ordering::Relaxed) % 60 >= 30 {
        WINDOW_COUNTS[work as usize].fetch_add(amount, std::sync::atomic::Ordering::Relaxed);
    }
    if matches!(
        work,
        Work::FreightDispatches | Work::ArrivalLaunches | Work::ShopperDispatches
    ) && LOG_DEPARTURES.load(std::sync::atomic::Ordering::Relaxed)
    {
        let minute = CLOCK_MINUTE.load(std::sync::atomic::Ordering::Relaxed);
        DEPARTURES
            .lock()
            .unwrap()
            .extend(std::iter::repeat_n((work, minute), amount as usize));
    }
}

/// Adds `amount` to one counter. Test builds only; otherwise compiled out.
#[cfg(not(test))]
#[inline(always)]
pub(crate) fn add(_work: Work, _amount: u64) {}

/// Records the absolute minute the simulation clock is processing. Test builds only.
#[cfg(test)]
pub(crate) fn set_clock_minute(minute: u64) {
    CLOCK_MINUTE.store(minute, std::sync::atomic::Ordering::Relaxed);
}

/// Records the absolute minute the simulation clock is processing. Test builds only.
#[cfg(not(test))]
#[inline(always)]
pub(crate) fn set_clock_minute(_minute: u64) {}

// Whether test builds run the operational hour's whole-city consistency checks (`SIM-02-K`).
#[cfg(test)]
static CITY_CHECKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Switches the operational hour's test-build consistency checks: the freight reservations
/// against a fresh build, the household member lists against a rebuild and the published
/// census's input supply. Ordinary tests keep them; the serialized timing runners
/// (`PERF-01`, `PERF-07`) switch them off, so test-only work stays out of the timed frames.
#[cfg(test)]
pub(crate) fn set_city_checks(enabled: bool) {
    CITY_CHECKS.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the operational hour's test-build consistency checks run ([`set_city_checks`]).
#[cfg(test)]
pub(crate) fn city_checks() -> bool {
    CITY_CHECKS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Starts or stops logging departures with their minute, for one burst-envelope run.
#[cfg(test)]
pub(crate) fn log_departures(enabled: bool) {
    LOG_DEPARTURES.store(enabled, std::sync::atomic::Ordering::Relaxed);
    DEPARTURES.lock().unwrap().clear();
}

/// Departures logged since the last call, each with the minute it left in.
#[cfg(test)]
pub(crate) fn take_departures() -> Vec<(Work, u64)> {
    std::mem::take(&mut DEPARTURES.lock().unwrap())
}

/// Cumulative counter values, for subtraction across a measured interval.
#[cfg(test)]
pub(crate) fn snapshot() -> [u64; WORK_COUNT] {
    std::array::from_fn(|index| COUNTS[index].load(std::sync::atomic::Ordering::Relaxed))
}

/// Cumulative counter values added during hours' second halves.
#[cfg(test)]
pub(crate) fn window_snapshot() -> [u64; WORK_COUNT] {
    std::array::from_fn(|index| WINDOW_COUNTS[index].load(std::sync::atomic::Ordering::Relaxed))
}

/// Stable report names, in counter order.
#[cfg(test)]
pub(crate) const WORK_NAMES: [&str; WORK_COUNT] = [
    "buildings_placed",
    "buildings_removed",
    "planning_passes",
    "live_censuses",
    "residential_planning_skips",
    "commercial_planning_skips",
    "industrial_planning_skips",
    "actions_planning_skips",
    "construction_epochs",
    "construction_epoch_skips",
    "census_buildings",
    "reservation_rows_copied",
    "owner_slots_prepared",
    "supplier_source_rebuilds",
    "supplier_sources_built",
    "buyer_site_rebuilds",
    "buyer_sites_built",
    "spawn_parcels_scanned",
    "supplier_searches",
    "supplier_candidates",
    "supplier_routes",
    "job_search_keys",
    "cch_point_searches",
    "cch_bucket_sources",
    "staffing_operations",
    "staffing_trials",
    "freight_dispatches",
    "arrival_launches",
    "shopper_dispatches",
    "building_settlements",
    "household_settlements",
    "desertions",
    "freight_deliveries",
    "trip_failures",
    "households_removed",
    "structural_changes",
    "census_restarts",
    "flow_field_zones_drained",
    "flow_fields_built",
    "flow_fields_skipped",
    "reproof_drops",
    "freight_cooldown_rejections",
    "level_change_permit_rejections",
    "admissions_dropped",
];

/// Hashes a value's `Debug` rendering without building the string. Test-only exact state
/// comparison: float rendering round-trips, so equal hashes follow equal bits.
#[cfg(test)]
pub(crate) fn debug_hash(value: &(impl std::fmt::Debug + ?Sized)) -> u64 {
    use std::hash::Hasher;
    struct Sink(std::collections::hash_map::DefaultHasher);
    impl std::fmt::Write for Sink {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            self.0.write(text.as_bytes());
            Ok(())
        }
    }
    let mut sink = Sink(Default::default());
    std::fmt::write(&mut sink, format_args!("{value:?}")).unwrap();
    sink.0.finish()
}

/// Named per-field [`debug_hash`]es, so a failed comparison names the differing field.
#[cfg(test)]
#[macro_export]
macro_rules! debug_fingerprint {
    ($($name:literal => $value:expr),* $(,)?) => {
        vec![$(($name, $crate::debug::work::debug_hash(&$value))),*]
    };
}

/// Inclusive phase attribution. Nested phases deliberately overlap; do not sum their totals.
#[cfg(test)]
pub(crate) mod phases {
    use super::{WORK_COUNT, WORK_NAMES, snapshot};
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Instant;

    static ENABLED: AtomicBool = AtomicBool::new(false);
    type PhaseTotal = (&'static str, u64, f64, [u64; WORK_COUNT]);
    static TOTALS: Mutex<Vec<PhaseTotal>> = Mutex::new(Vec::new());

    /// Adds this invocation's inclusive time and work when the scope ends.
    pub(crate) struct Guard {
        name: &'static str,
        start: Instant,
        counts: [u64; WORK_COUNT],
    }

    /// Opens a measured scope only while the serialized contract runner is active.
    pub(crate) fn enter(name: &'static str) -> Option<Guard> {
        ENABLED.load(Ordering::Relaxed).then(|| Guard {
            name,
            start: Instant::now(),
            counts: snapshot(),
        })
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            let ms = self.start.elapsed().as_secs_f64() * 1000.0;
            let after = snapshot();
            let mut totals = TOTALS.lock().unwrap();
            let index = totals
                .iter()
                .position(|row| row.0 == self.name)
                .unwrap_or_else(|| {
                    totals.push((self.name, 0, 0.0, [0; WORK_COUNT]));
                    totals.len() - 1
                });
            let row = &mut totals[index];
            row.1 += 1;
            row.2 += ms;
            for (i, value) in after.into_iter().enumerate() {
                row.3[i] += value - self.counts[i];
            }
        }
    }

    /// Starts an empty phase report. Only serial ignored contract tests call this.
    pub(crate) fn begin() {
        TOTALS.lock().unwrap().clear();
        ENABLED.store(true, Ordering::Relaxed);
    }

    /// Stops attribution and returns named inclusive times and counter deltas.
    pub(crate) fn finish() -> serde_json::Value {
        ENABLED.store(false, Ordering::Relaxed);
        let totals = TOTALS.lock().unwrap();
        serde_json::Value::Object(
            totals
                .iter()
                .map(|(name, calls, ms, counts)| {
                    let counts: serde_json::Map<_, _> = WORK_NAMES
                        .iter()
                        .zip(counts)
                        .map(|(name, count)| (name.to_string(), (*count).into()))
                        .collect();
                    (
                        name.to_string(),
                        serde_json::json!({"calls":calls,"inclusive_ms":ms,"counts":counts}),
                    )
                })
                .collect(),
        )
    }
}
