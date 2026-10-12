// SPDX-License-Identifier: GPL-2.0-only

//! Test-only work counters for `ECON-13` scaling attribution.
//!
//! Counters separate algorithmic growth from cache effects: a tier whose counts grow 10× but
//! whose time grows faster is memory-bound. Gameplay and benchmark builds compile every
//! [`add`] call to nothing, so instrumented call sites cost no time outside tests.

/// One counted unit of simulation work. Rows are summed over every call of their pass.
// Some counters are recorded only by test-only search traces.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Work {
    /// Hourly demand planning passes.
    PlanningPasses,
    /// Live hourly funding censuses run by employment.
    LiveCensuses,
    /// Funding censuses rebuilt by a planning pass that could not reuse the live one.
    PlannerCensuses,
    /// Funding censuses run by queued-construction execution.
    ExecutionCensuses,
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
}

/// Number of [`Work`] counters.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const WORK_COUNT: usize = Work::StaffingTrials as usize + 1;

#[cfg(test)]
static COUNTS: [std::sync::atomic::AtomicU64; WORK_COUNT] =
    [const { std::sync::atomic::AtomicU64::new(0) }; WORK_COUNT];

/// Adds `amount` to one counter. Test builds only; otherwise compiled out.
#[cfg(test)]
pub(crate) fn add(work: Work, amount: u64) {
    COUNTS[work as usize].fetch_add(amount, std::sync::atomic::Ordering::Relaxed);
}

/// Adds `amount` to one counter. Test builds only; otherwise compiled out.
#[cfg(not(test))]
#[inline(always)]
pub(crate) fn add(_work: Work, _amount: u64) {}

/// Cumulative counter values, for subtraction across a measured interval.
#[cfg(test)]
pub(crate) fn snapshot() -> [u64; WORK_COUNT] {
    std::array::from_fn(|index| COUNTS[index].load(std::sync::atomic::Ordering::Relaxed))
}

/// Stable report names, in counter order.
#[cfg(test)]
pub(crate) const WORK_NAMES: [&str; WORK_COUNT] = [
    "planning_passes", "live_censuses", "planner_censuses", "execution_censuses",
    "census_buildings", "reservation_rows_copied", "owner_slots_prepared",
    "supplier_source_rebuilds", "supplier_sources_built", "buyer_site_rebuilds",
    "buyer_sites_built", "spawn_parcels_scanned", "supplier_searches", "supplier_candidates",
    "supplier_routes", "job_search_keys", "cch_point_searches", "cch_bucket_sources",
    "staffing_operations", "staffing_trials",
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
