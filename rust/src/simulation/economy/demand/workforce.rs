// SPDX-License-Identifier: GPL-2.0-only

//! Completion-time labour reservations shared by all private projects in one planning pass.

use std::sync::atomic::{AtomicU32, Ordering};

use rayon::prelude::*;

use super::actions::DemandSpawnAction;
use super::queued::QueuedConstruction;
use super::snapshot::{DailyDemandSnapshot, UnhousedWorkforce};
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, construction_duration_hours,
};
use crate::simulation::economy::agents::MAX_ADULTS_PER_HOUSEHOLD;
use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
#[cfg(test)]
use crate::simulation::economy::households::applicant_workers;
use crate::simulation::economy::households::{applicant_places_and_members, level_tuning_value};
use crate::simulation::zoning::ZoneType;

// A sum tree over operational hours, not buildings. A claim consumes the latest eligible
// supply first, leaving early homes available for projects with earlier completion dates.
#[derive(Default)]
struct ReadySupply {
    sums: Vec<u64>,
    leaves: usize,
}

impl ReadySupply {
    fn reset(&mut self, hours: usize) {
        self.leaves = hours.next_power_of_two();
        self.sums.resize(self.leaves * 2, 0);
        self.sums.fill(0);
    }

    fn add(&mut self, hour: usize, workers: u32) {
        let mut node = self.leaves + hour;
        while node > 0 {
            self.sums[node] += u64::from(workers);
            node /= 2;
        }
    }

    fn available(&self, hour: usize) -> u64 {
        let mut left = self.leaves;
        let mut right = self.leaves + hour + 1;
        let mut total = 0;
        while left < right {
            if left % 2 == 1 {
                total += self.sums[left];
                left += 1;
            }
            if right % 2 == 1 {
                right -= 1;
                total += self.sums[right];
            }
            left /= 2;
            right /= 2;
        }
        total
    }

    fn take(&mut self, hour: usize, workers: u32) -> u32 {
        let take = self.available(hour).min(u64::from(workers));
        let mut remaining = take;
        while remaining > 0 {
            // Find the final positive leaf in the eligible prefix by its cumulative rank.
            let mut rank = self.available(hour);
            let mut node = 1;
            while node < self.leaves {
                let left_sum = self.sums[node * 2];
                node *= 2;
                if rank > left_sum {
                    rank -= left_sum;
                    node += 1;
                }
            }
            let consumed = self.sums[node].min(remaining);
            remaining -= consumed;
            while node > 0 {
                self.sums[node] -= consumed;
                node /= 2;
            }
        }
        take as u32
    }
}

// `order` is the building index, or the parcel id before placement. `seed` seeds the applicant
// offered each place: the building's applicant seed, a queued home's spawn id, or a proposed
// home's parcel id. Places `first_slot..first_slot + slots` are free.
struct FutureHome {
    hour: usize,
    level: u8,
    order: u64,
    seed: usize,
    slots: u32,
    first_slot: u32,
    flat_size_m2: f32,
    threshold: f32,
    optional: Option<usize>,
    // Adults supplied by arrivals after displaced residents claim their places.
    arrival_workers: u32,
}

// Leaves retain household order; subtree maxima find the first affordable claimant
// in O(log U), even when authored thresholds are not monotone by housing level.
#[derive(Default)]
struct DisplacedHouseholds {
    households: Vec<UnhousedWorkforce>,
    maxima: Vec<f32>,
    leaves: usize,
}

impl DisplacedHouseholds {
    fn reset(&mut self, households: &[UnhousedWorkforce]) {
        self.households.clear();
        self.households.extend_from_slice(households);
        self.leaves = households.len().max(1).next_power_of_two();
        self.maxima.resize(self.leaves * 2, f32::NEG_INFINITY);
        self.maxima.fill(f32::NEG_INFINITY);
        for (index, household) in households.iter().enumerate() {
            self.maxima[self.leaves + index] = household.reserve_days + f32::EPSILON;
        }
        for node in (1..self.leaves).rev() {
            self.maxima[node] = self.maxima[node * 2].max(self.maxima[node * 2 + 1]);
        }
    }

    fn claim(&mut self, threshold: f32) -> Option<u32> {
        if self.maxima[1] < threshold {
            return None;
        }
        let mut node = 1;
        while node < self.leaves {
            node *= 2;
            if self.maxima[node] < threshold {
                node += 1;
            }
        }
        let workers = self.households[node - self.leaves].workers;
        self.maxima[node] = f32::NEG_INFINITY;
        node /= 2;
        while node > 0 {
            self.maxima[node] = self.maxima[node * 2].max(self.maxima[node * 2 + 1]);
            node /= 2;
        }
        Some(workers)
    }
}

/// Reused hourly scratch; no allocations per building or per project reservation.
#[derive(Default)]
pub(super) struct WorkforceReservations {
    homes: Vec<FutureHome>,
    homes_by_hour: Vec<AtomicU32>,
    // Ready arrival places by their applicants' adults (`DEM-02-C`): the hour-zero worker
    // supply in places, so claims in workers can still leave a count of free homes.
    ready_places: [AtomicU32; MAX_ADULTS_PER_HOUSEHOLD as usize + 1],
    // Arrival members per hour, whose own shopping needs store workers (`DEM-02-C`).
    members_by_hour: Vec<AtomicU32>,
    store_labor_per_resident: f64,
    // Running store labor of individual future homes, in their matching order.
    home_store_labor: f64,
    displaced: DisplacedHouseholds,
    optional_selected: Vec<bool>,
    arrivals_possible: bool,
    jobs_by_hour: Vec<AtomicU32>,
    committed: ReadySupply,
    optional: ReadySupply,
    optional_initial: Vec<u32>,
    unemployed: u32,
    unfilled_existing: u32,
    horizon: usize,
    hours: Vec<u64>,
    new_start_hour: u64,
}

impl WorkforceReservations {
    /// Collects already committed housing and workplaces once, in one parallel pass over the
    /// buildings (`SIM-02-K`), O(B / p + Q + hours). A workplace the census did not see
    /// complete (`census_operating`) claims its constructing workers, as one still under
    /// construction does (`SIM-02-I`). Planning passes scan the free places beside their site
    /// preparation instead ([`Self::scan_free_places`], [`Self::reset_scanned`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reset(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        snapshot: &DailyDemandSnapshot,
        queued: &[QueuedConstruction],
        new_start_hour: u64,
        reservations: Option<&super::supply::ProjectReservations>,
        census_operating: impl Fn(usize) -> bool + Sync,
    ) {
        let free_places = Self::scan_free_places(allocator, tuning, snapshot);
        self.reset_scanned(
            free_places,
            allocator,
            catalog,
            tuning,
            snapshot,
            queued,
            new_start_hour,
            reservations,
            census_operating,
        );
    }

    /// The part of a reset's pass over the buildings that reads neither the project books nor
    /// the queue (`SIM-02-K`): every building's construction hours left and its free places,
    /// for `snapshot`. A planning pass runs it beside its site preparation and project market,
    /// and hands it to [`Self::reset_scanned`]. O(B / p).
    pub(super) fn scan_free_places(
        allocator: &BuildingAllocator,
        tuning: &RuntimeEconomyTuning,
        snapshot: &DailyDemandSnapshot,
    ) -> FreePlaceScan {
        let places = FreePlaces::for_snapshot(snapshot, tuning);
        FreePlaceScan(
            allocator
                .buildings
                .par_iter()
                .enumerate()
                .fold(BuildingHours::default, |mut scan, (idx, building)| {
                    scan.add_free_places(allocator, places, idx, building);
                    scan
                })
                .reduce(BuildingHours::default, BuildingHours::merge),
        )
    }

    /// [`Self::reset`] on a scan [`Self::scan_free_places`] took of the same buildings and
    /// snapshot. The constructing or not yet operating workplaces' jobs are added from the
    /// project books' constructing rows, which hold every building that predicate selects
    /// (`supply::constructing`), or from every building without books. O(constructing + Q +
    /// hours) with books.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reset_scanned(
        &mut self,
        free_places: FreePlaceScan,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        snapshot: &DailyDemandSnapshot,
        queued: &[QueuedConstruction],
        new_start_hour: u64,
        reservations: Option<&super::supply::ProjectReservations>,
        census_operating: impl Fn(usize) -> bool + Sync,
    ) {
        let arrivals_possible = snapshot.connected_border_count > 0;
        let places = FreePlaces::for_snapshot(snapshot, tuning);
        let FreePlaceScan(mut scan) = free_places;
        scan.add_constructing_jobs(
            allocator,
            catalog,
            snapshot,
            reservations,
            &census_operating,
        );
        self.horizon = scan.horizon.max(
            tuning
                .construction
                .residential_hours_by_level
                .iter()
                .chain(&tuning.construction.commercial_hours_by_level)
                .chain(&tuning.construction.industrial_hours_by_level)
                .copied()
                .max()
                .unwrap_or(0) as usize,
        );
        // Keep the bounded authored/remaining hours dense for O(1) building aggregation.
        // Queue delays add coordinates, not empty hours: a distant saved due date cannot
        // cause an allocation proportional to elapsed time.
        self.hours.clear();
        self.hours.extend(0..=self.horizon as u64);
        self.hours
            .extend(queued.iter().map(|project| project.completion_hour));
        self.hours.extend(
            tuning
                .construction
                .residential_hours_by_level
                .iter()
                .chain(&tuning.construction.commercial_hours_by_level)
                .chain(&tuning.construction.industrial_hours_by_level)
                .map(|&duration| new_start_hour + u64::from(duration)),
        );
        self.hours.sort_unstable();
        self.hours.dedup();
        self.horizon = self.hours.len() - 1;
        self.new_start_hour = new_start_hour;
        let hours = self.hours.len();
        for bins in [
            &mut self.homes_by_hour,
            &mut self.jobs_by_hour,
            &mut self.members_by_hour,
        ] {
            bins.resize_with(hours, || AtomicU32::new(0));
            for bin in bins {
                bin.store(0, Ordering::Relaxed);
            }
        }
        for (bins, sums) in [
            (&self.homes_by_hour, &scan.homes),
            (&self.jobs_by_hour, &scan.jobs),
            (&self.members_by_hour, &scan.members),
        ] {
            // Every building's hour is at most the horizon, below which hours are dense.
            for (bin, &sum) in bins.iter().zip(sums) {
                bin.store(sum, Ordering::Relaxed);
            }
        }
        for (places, &sum) in self.ready_places.iter().zip(&scan.ready) {
            places.store(sum, Ordering::Relaxed);
        }
        self.store_labor_per_resident = resident_store_labor(catalog);
        self.home_store_labor = 0.0;
        self.committed.reset(hours);
        self.optional.reset(hours);
        self.optional_initial.resize(hours, 0);
        self.optional_initial.fill(0);
        self.unemployed = snapshot.existing_unemployed_member_count;
        self.unfilled_existing = 0;
        self.arrivals_possible = arrivals_possible;
        self.displaced.reset(&snapshot.unhoused_workforce);
        self.optional_selected.clear();
        let thresholds = &tuning
            .households
            .residential_move_in_min_reserve_days_by_level;
        self.homes.clear();
        self.homes.append(&mut scan.future);
        let match_displaced = matches!(places, FreePlaces::Homes(_));
        for project in queued {
            if !project.active {
                continue;
            }
            let hour = self.hours.binary_search(&project.completion_hour).unwrap();
            if project.household_slots > 0 && !match_displaced {
                if arrivals_possible {
                    self.homes_by_hour[hour]
                        .fetch_add(project.household_workers, Ordering::Relaxed);
                    self.members_by_hour[hour]
                        .fetch_add(project.household_members, Ordering::Relaxed);
                }
            } else if project.household_slots > 0 {
                self.homes.push(FutureHome {
                    hour,
                    level: project.level,
                    order: project.parcel_id,
                    seed: project.home_seed,
                    slots: project.household_slots,
                    first_slot: 0,
                    flat_size_m2: project.flat_size_m2,
                    threshold: level_tuning_value(thresholds, project.level),
                    optional: None,
                    arrival_workers: 0,
                });
            }
            if let Some(profile) = catalog.profile_by_runtime_id(project.profile_runtime_id) {
                self.jobs_by_hour[hour].fetch_add(
                    reservations.map_or_else(
                        || snapshot.output_absorption.committed_workers(profile),
                        |claims| {
                            claims
                                .queued_workers
                                .binary_search_by_key(&project.parcel_id, |&(parcel, _)| parcel)
                                .ok()
                                .map_or(0, |index| claims.queued_workers[index].1)
                        },
                    ),
                    Ordering::Relaxed,
                );
            }
        }
        self.jobs_by_hour[0].fetch_add(snapshot.open_job_slots, Ordering::Relaxed);
        for (hour, homes) in self.homes_by_hour.iter().enumerate() {
            let members = self.members_by_hour[hour].load(Ordering::Relaxed);
            let store_workers = (f64::from(members) * self.store_labor_per_resident).ceil() as u32;
            self.committed.add(
                hour,
                homes.load(Ordering::Relaxed).saturating_sub(store_workers),
            );
        }
    }

    /// Adds a legal home, distinguishing selected construction from conditional pairing.
    pub(super) fn add_home(
        &mut self,
        allocator: &BuildingAllocator,
        tuning: &RuntimeEconomyTuning,
        action: &DemandSpawnAction,
        optional: bool,
    ) {
        let Some(authored) = allocator
            .registry
            .get(&action.asset_id)
            .and_then(|entry| entry.manifest.building.as_ref())
        else {
            return;
        };
        let hour = self.project_hour(construction_duration_hours(
            ZoneType::Residential,
            authored.level,
            tuning,
        ));
        let optional = optional.then(|| {
            let index = self.optional_selected.len();
            self.optional_selected.push(false);
            index
        });
        self.homes.push(FutureHome {
            hour,
            level: authored.level,
            order: action.parcel_id,
            seed: action.parcel_id as usize,
            slots: allocator.registry.household_capacity(&action.asset_id),
            first_slot: 0,
            flat_size_m2: allocator.registry.flat_size_m2(&action.asset_id),
            threshold: level_tuning_value(
                &tuning
                    .households
                    .residential_move_in_min_reserve_days_by_level,
                authored.level,
            ),
            optional,
            arrival_workers: 0,
        });
    }

    /// Reserves incumbents first, including their unmet hiring needs after completion.
    pub(super) fn reserve_existing(&mut self) {
        self.match_homes();
        let mut deficit = 0_u32;
        for hour in 0..=self.horizon {
            let workers = self.jobs_by_hour[hour].load(Ordering::Relaxed);
            deficit = deficit.saturating_add(workers - self.take(hour, workers));
        }
        // Incumbent vacancies also claim later homes; a new employer cannot count those
        // workers simply because an existing project's original deadline has passed.
        self.unfilled_existing = deficit - self.take(self.horizon, deficit);
    }

    /// Checks a proposed workforce without consuming it before placement can succeed.
    pub(super) fn can_claim(&self, completion_hour: u16, workers: u32) -> bool {
        let hour = self.project_hour(completion_hour);
        u64::from(self.unemployed) + self.committed.available(hour) + self.optional.available(hour)
            >= u64::from(workers)
    }

    /// Workers a project completing after `completion_hour` could still claim. O(log H).
    pub(super) fn available(&self, completion_hour: u16) -> u32 {
        self.available_at(self.project_hour(completion_hour))
    }

    /// [`Self::available`] for a queued deadline, as [`Self::claim_by_hour`] reads it.
    pub(super) fn available_by_hour(&self, completion_hour: u64) -> u32 {
        self.available_at(self.hours.partition_point(|&hour| hour <= completion_hour) - 1)
    }

    fn available_at(&self, hour: usize) -> u32 {
        (u64::from(self.unemployed)
            + self.committed.available(hour)
            + self.optional.available(hour))
        .min(u64::from(u32::MAX)) as u32
    }

    /// Atomically reserves a project's workforce; rejection leaves all supply untouched.
    pub(super) fn claim(&mut self, completion_hour: u16, workers: u32) -> bool {
        if !self.can_claim(completion_hour, workers) {
            return false;
        }
        let hour = self.project_hour(completion_hour);
        self.take(hour, workers);
        true
    }

    /// Reserves a queued deadline against placed housing, including distant saved due dates.
    /// The final prepared source bin covers later deadlines without allocating empty hours.
    pub(super) fn claim_by_hour(&mut self, completion_hour: u64, workers: u32) -> bool {
        let hour = self.hours.partition_point(|&hour| hour <= completion_hour) - 1;
        if u64::from(self.unemployed)
            + self.committed.available(hour)
            + self.optional.available(hour)
            < u64::from(workers)
        {
            return false;
        }
        self.take(hour, workers);
        true
    }

    /// Immediate upgrades may claim only unemployed adults and homes that are already ready.
    /// Unlike queued construction, this does not inherit the next project's start-hour delay.
    pub(super) fn claim_ready(&mut self, workers: u32) -> bool {
        if !self.can_claim_ready(workers) {
            return false;
        }
        self.take(0, workers);
        true
    }

    /// Checks immediate staffing without spending workers before physical placement succeeds.
    pub(super) fn can_claim_ready(&self, workers: u32) -> bool {
        u64::from(self.unemployed) + self.committed.available(0) >= u64::from(workers)
    }
    /// Ready vacancies left after all workplace claims; nonworker arrivals may use these.
    /// Claimed workers occupy the fewest-adult worker places first, so as many places stay
    /// reserved as the claims could need; places offered to non-workers are never needed. O(1).
    pub(super) fn unreserved_ready_homes(&self) -> u32 {
        let [idle, singles, couples] = self
            .ready_places
            .each_ref()
            .map(|places| places.load(Ordering::Relaxed));
        let supplied = singles + 2 * couples;
        let claimed = supplied.saturating_sub(self.committed.sums[self.committed.leaves] as u32);
        let needed = if claimed <= singles {
            claimed
        } else {
            singles + (claimed - singles).div_ceil(2)
        };
        (idle + singles + couples).saturating_sub(needed)
    }

    /// Removes ready places, counted by their applicants' adults, left after workplace claims;
    /// never local workers or future homes. Rejected downsizing leaves the calendar unchanged;
    /// a successful claim costs O(log H).
    pub(super) fn retire_ready_homes(
        &mut self,
        places: [u32; MAX_ADULTS_PER_HOUSEHOLD as usize + 1],
    ) -> bool {
        let workers = place_workers(&places);
        if self.unreserved_ready_homes() < places.iter().sum::<u32>()
            || self.committed.available(0) < u64::from(workers)
        {
            return false;
        }
        self.committed.take(0, workers);
        for (ready, retired) in self.ready_places.iter_mut().zip(places) {
            *ready.get_mut() -= retired;
        }
        true
    }

    fn project_hour(&self, duration: u16) -> usize {
        self.hours
            .partition_point(|&hour| hour <= self.new_start_hour + u64::from(duration))
            - 1
    }

    fn take(&mut self, hour: usize, workers: u32) -> u32 {
        let locals = workers.min(self.unemployed);
        self.unemployed -= locals;
        let committed = self.committed.take(hour, workers - locals);
        let optional = self.optional.take(hour, workers - locals - committed);
        locals + committed + optional
    }

    fn match_homes(&mut self) {
        if !self.displaced.households.is_empty() {
            self.homes.sort_unstable_by_key(|home| {
                (
                    home.hour,
                    std::cmp::Reverse(home.level),
                    home.optional.is_some(),
                    home.order,
                )
            });
        }
        // Finite household/home claims are dependent and therefore serial. Each displaced
        // family is removed once: O(F log F + U log U + F log T), reusable scratch.
        for home in &mut self.homes {
            let mut remaining = home.slots;
            let mut workers = 0;
            while remaining > 0 && !self.displaced.households.is_empty() {
                let Some(available) = self.displaced.claim(home.threshold) else {
                    break;
                };
                workers += available;
                remaining -= 1;
            }
            if home.hour == 0 {
                self.unemployed += workers;
                workers = 0;
            }
            if self.arrivals_possible {
                // Arrivals bring their applicants' adults, not one worker per place.
                let end = home.first_slot + home.slots;
                let (places, members) = applicant_places_and_members(
                    home.flat_size_m2,
                    home.seed,
                    end - remaining..end,
                );
                if home.hour == 0 {
                    add_places(&self.ready_places, &places);
                }
                // Whole store workers for the running total of arrivals' shopping.
                let before = self.home_store_labor.ceil();
                self.home_store_labor += f64::from(members) * self.store_labor_per_resident;
                let store_workers = (self.home_store_labor.ceil() - before) as u32;
                home.arrival_workers = place_workers(&places).saturating_sub(store_workers);
                workers += home.arrival_workers;
            }
            if let Some(index) = home.optional {
                // A home claimed by an existing resident must be built even if that
                // resident supplies no workers. Its physical places are not worker credits.
                if remaining < home.slots {
                    self.optional_selected[index] = true;
                    self.committed.add(home.hour, workers);
                } else {
                    self.optional.add(home.hour, workers);
                    self.optional_initial[home.hour] += workers;
                }
            } else {
                self.committed.add(home.hour, workers);
            }
        }
    }

    /// Materializes displaced residents' homes and whole optional homes claimed by employers.
    pub(super) fn select_optional_homes(&mut self) {
        for home in &self.homes {
            let Some(index) = home.optional else {
                continue;
            };
            if self.optional_selected[index] {
                continue;
            }
            let remaining = self.optional.sums[self.optional.leaves + home.hour];
            // A home whose applicants bring no adults cannot supply claimed workers.
            if home.arrival_workers > 0 && u64::from(self.optional_initial[home.hour]) > remaining {
                self.optional_selected[index] = true;
                self.optional_initial[home.hour] =
                    self.optional_initial[home.hour].saturating_sub(home.arrival_workers);
            }
        }
    }

    /// Reads the decision in the original optional-candidate order.
    pub(super) fn needs_home(&self, index: usize) -> bool {
        self.optional_selected[index]
    }
}

/// Store workers one resident's authored consumption needs (`DEM-02-C`): for each resource
/// households buy from stores, the consumption rate times the fewest workers per daily unit
/// among the stores selling it. Arrivals offer their adults net of this labor, so stores
/// their shopping needs keep workers. O(profiles x ports), once per calendar reset.
pub(super) fn resident_store_labor(catalog: &RuntimeEconomyCatalog) -> f64 {
    let mut labor = 0.0;
    for sink in catalog.all_profiles() {
        if sink.kind != EconomyProfileRuntimeKind::DemandSink {
            continue;
        }
        for input in &sink.inputs {
            let per_unit = catalog
                .all_profiles()
                .iter()
                .filter(|store| {
                    matches!(
                        store.kind,
                        EconomyProfileRuntimeKind::Store | EconomyProfileRuntimeKind::ServiceStore
                    ) && store.worker_capacity > 0
                })
                .filter_map(|store| {
                    let port = store.output_port(input.resource_runtime_id)?;
                    let output = store.net_output_units_per_day(port);
                    (output > 0.0).then(|| f64::from(store.worker_capacity) / f64::from(output))
                })
                .min_by(f64::total_cmp);
            if let Some(per_unit) = per_unit {
                labor += f64::from(sink.consumption_rate_per_resident) * per_unit;
            }
        }
    }
    labor
}

fn place_workers(places: &[u32; MAX_ADULTS_PER_HOUSEHOLD as usize + 1]) -> u32 {
    places
        .iter()
        .enumerate()
        .map(|(adults, &count)| adults as u32 * count)
        .sum()
}

fn add_places(
    ready: &[AtomicU32; MAX_ADULTS_PER_HOUSEHOLD as usize + 1],
    places: &[u32; MAX_ADULTS_PER_HOUSEHOLD as usize + 1],
) {
    for (ready, &count) in ready.iter().zip(places) {
        ready.fetch_add(count, Ordering::Relaxed);
    }
}

// A building's first free place and capacity: a constructing building's places are all its
// asset's, an operating one's those above its occupancy.
fn free_places(allocator: &BuildingAllocator, idx: usize, building: &Building) -> (u32, u32) {
    if building.is_under_construction() {
        (0, allocator.registry.household_capacity(&building.asset_id))
    } else {
        (building.occupancy, allocator.household_capacity(idx))
    }
}

// A reset's one pass over the buildings (`SIM-02-K`): the latest construction hour of every
// building and, by construction hours left, the workers and members of arrival places and the
// jobs of constructing or not yet operating workplaces, summed per thread and merged. The sums
// wrap, as the shared atomic adds they replace did, so they are exact in any order.
#[derive(Default)]
struct BuildingHours {
    horizon: usize,
    homes: Vec<u32>,
    members: Vec<u32>,
    jobs: Vec<u32>,
    ready: [u32; MAX_ADULTS_PER_HOUSEHOLD as usize + 1],
    // The future homes displaced households may claim, in building order.
    future: Vec<FutureHome>,
}

/// The free places of every building, taken for one workforce reset
/// ([`WorkforceReservations::scan_free_places`]).
pub(super) struct FreePlaceScan(BuildingHours);

// What a reset's free places supply: future homes with move-in thresholds by level, for the
// displaced households to claim first; arrival workers by hour; or nothing (no arrivals).
#[derive(Clone, Copy)]
enum FreePlaces<'a> {
    Homes(&'a [f32]),
    Arrivals,
    Ignored,
}

impl<'a> FreePlaces<'a> {
    // Displaced households claim free places first; without them free places only add to
    // the hour bins.
    fn for_snapshot(snapshot: &DailyDemandSnapshot, tuning: &'a RuntimeEconomyTuning) -> Self {
        if !snapshot.unhoused_workforce.is_empty() {
            Self::Homes(
                &tuning
                    .households
                    .residential_move_in_min_reserve_days_by_level,
            )
        } else if snapshot.connected_border_count > 0 {
            Self::Arrivals
        } else {
            Self::Ignored
        }
    }
}

impl BuildingHours {
    fn add(bins: &mut Vec<u32>, hour: usize, value: u32) {
        if bins.len() <= hour {
            bins.resize(hour + 1, 0);
        }
        bins[hour] = bins[hour].wrapping_add(value);
    }

    // Whether a building neither supplies places nor claims jobs.
    fn skipped(building: &Building) -> bool {
        building.broken
            || building.economy_broken
            || building.is_deserted
            || building.pending_redevelopment
    }

    // The jobs of the constructing or not yet operating workplaces, as the reset's pass over
    // the buildings added them: over the books' constructing rows, which hold every building
    // the predicate selects, or over every building without books. The sums wrap, so the
    // order is free.
    fn add_constructing_jobs(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        snapshot: &DailyDemandSnapshot,
        reservations: Option<&super::supply::ProjectReservations>,
        census_operating: &(impl Fn(usize) -> bool + Sync),
    ) {
        let mut add = |idx: usize| {
            let building = &allocator.buildings[idx];
            if !Self::skipped(building)
                && (building.is_under_construction() || !census_operating(idx))
                && matches!(
                    building.zone_type,
                    ZoneType::Commercial | ZoneType::Industrial
                )
                && let Some(profile) =
                    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            {
                Self::add(
                    &mut self.jobs,
                    usize::from(building.construction_hours_left(allocator.now_slot)),
                    reservations.map_or_else(
                        || snapshot.output_absorption.committed_workers(profile),
                        |claims| claims.building_workers.get(idx).copied().unwrap_or(0),
                    ),
                );
            }
        };
        match reservations {
            Some(claims) => {
                // Test builds: the rows hold every building the predicate selects.
                #[cfg(test)]
                if crate::debug::work::city_checks() {
                    let rows = claims.constructing_rows();
                    for (idx, building) in allocator.buildings.iter().enumerate() {
                        if super::supply::constructing(building, census_operating(idx)) {
                            assert!(
                                rows.binary_search(&idx).is_ok(),
                                "constructing building {idx} missing from the books' rows"
                            );
                        }
                    }
                }
                claims.constructing_rows().iter().for_each(|&idx| add(idx));
            }
            None => (0..allocator.buildings.len()).for_each(add),
        }
    }

    // Building `idx`'s construction hours left and free places, as the reset's pass over the
    // buildings added them.
    fn add_free_places(
        &mut self,
        allocator: &BuildingAllocator,
        places: FreePlaces<'_>,
        idx: usize,
        building: &Building,
    ) {
        let hour = usize::from(building.construction_hours_left(allocator.now_slot));
        self.horizon = self.horizon.max(hour);
        if Self::skipped(building) {
            return;
        }
        if matches!(places, FreePlaces::Ignored) {
            return;
        }
        let (first_slot, capacity) = free_places(allocator, idx, building);
        if capacity <= first_slot {
            return;
        }
        let flat_size_m2 = allocator.registry.flat_size_m2(&building.asset_id);
        if let FreePlaces::Homes(thresholds) = places {
            self.future.push(FutureHome {
                hour,
                level: building.level,
                order: idx as u64,
                seed: allocator.applicant_seed(idx),
                slots: capacity - first_slot,
                first_slot,
                flat_size_m2,
                threshold: level_tuning_value(thresholds, building.level),
                optional: None,
                arrival_workers: 0,
            });
            return;
        }
        // Each free place supplies the adults of the applicant offered it.
        let (places, members) = applicant_places_and_members(
            flat_size_m2,
            allocator.applicant_seed(idx),
            first_slot..capacity,
        );
        Self::add(&mut self.homes, hour, place_workers(&places));
        Self::add(&mut self.members, hour, members);
        if hour == 0 {
            for (ready, &count) in self.ready.iter_mut().zip(&places) {
                *ready = ready.wrapping_add(count);
            }
        }
    }

    fn merge(mut self, mut other: Self) -> Self {
        self.horizon = self.horizon.max(other.horizon);
        for (bins, others) in [
            (&mut self.homes, other.homes),
            (&mut self.members, other.members),
            (&mut self.jobs, other.jobs),
        ] {
            if bins.len() < others.len() {
                bins.resize(others.len(), 0);
            }
            for (bin, value) in bins.iter_mut().zip(others) {
                *bin = bin.wrapping_add(value);
            }
        }
        for (ready, count) in self.ready.iter_mut().zip(other.ready) {
            *ready = ready.wrapping_add(count);
        }
        // `reduce` merges adjacent building ranges in order, so homes stay in building order.
        self.future.append(&mut other.future);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fits only singles; a single is a working adult four times in five.
    const SINGLE_FLAT_M2: f32 = 50.0;

    // A future home whose applicants are all working singles, so each place supplies one
    // worker. The applicant seed is searched from `order * 64`, preserving the given order.
    fn home(
        hour: usize,
        level: u8,
        order: u64,
        slots: u32,
        threshold: f32,
        optional: Option<usize>,
    ) -> FutureHome {
        let order = (order * 64..)
            .find(|&seed| applicant_workers(SINGLE_FLAT_M2, seed as usize, 0..slots) == slots)
            .unwrap();
        FutureHome {
            hour,
            level,
            order,
            seed: order as usize,
            slots,
            first_slot: 0,
            flat_size_m2: SINGLE_FLAT_M2,
            threshold,
            optional,
            arrival_workers: 0,
        }
    }

    fn calendar(unemployed: u32) -> WorkforceReservations {
        let mut result = WorkforceReservations {
            unemployed,
            horizon: 36,
            hours: (0..37).collect(),
            ..Default::default()
        };
        result.committed.reset(37);
        result.optional.reset(37);
        result.optional_initial.resize(37, 0);
        result.jobs_by_hour.resize_with(37, || AtomicU32::new(0));
        result
    }

    #[test]
    fn downsizing_cannot_take_reserved_homes_workers_or_future_supply() {
        let mut supply = calendar(2);
        supply.committed.add(0, 5);
        *supply.ready_places[1].get_mut() = 5; // Five ready places offered to working singles.
        supply.committed.add(12, 8);
        assert!(supply.claim_ready(5)); // Two residents and three ready homes.
        assert!(!supply.retire_ready_homes([0, 3, 0]));
        assert_eq!(supply.unreserved_ready_homes(), 2);
        assert!(supply.retire_ready_homes([0, 2, 0]));
        assert!(!supply.claim_ready(1));
        assert!(supply.claim(12, 8));
    }

    #[test]
    fn projects_cannot_share_workers_or_count_late_homes() {
        let mut supply = calendar(2);
        supply.committed.add(6, 4);
        supply.committed.add(18, 10);
        assert!(!supply.claim(12, 7));
        assert_eq!(supply.unemployed, 2);
        assert!(supply.claim(12, 6));
        assert!(!supply.claim(12, 1));
        assert!(supply.claim(18, 10));
        assert!(!supply.claim(36, 1));
    }

    #[test]
    fn later_project_preserves_early_homes() {
        let mut supply = calendar(0);
        supply.committed.add(6, 3);
        supply.committed.add(18, 5);
        assert!(supply.claim(24, 5));
        assert!(supply.claim(8, 3));
    }

    #[test]
    fn future_homes_match_affordability_deadlines_and_actual_workers() {
        let mut supply = calendar(0);
        supply.arrivals_possible = true;
        supply.displaced.reset(&[
            UnhousedWorkforce {
                reserve_days: 5.0,
                workers: 0,
            },
            UnhousedWorkforce {
                reserve_days: 40.0,
                workers: 7,
            },
        ]);
        // Level one is more expensive here: eligibility must not assume monotone levels.
        supply.homes.push(home(18, 2, 1, 2, 2.0, None));
        supply.homes.push(home(6, 1, 2, 1, 30.0, None));
        supply.reserve_existing();
        assert!(!supply.claim(5, 1));
        assert!(
            supply.claim(6, 7),
            "existing households are not capped at starter size"
        );
        assert!(!supply.claim(6, 1));
        assert!(
            supply.claim(18, 1),
            "only the unclaimed place supports an arrival"
        );
        assert!(!supply.claim(18, 1));
    }

    #[test]
    fn optional_nonworker_home_cannot_be_skipped_to_manufacture_arrival_workers() {
        let mut supply = calendar(0);
        supply.arrivals_possible = true;
        supply.displaced.reset(&[UnhousedWorkforce {
            reserve_days: 5.0,
            workers: 0,
        }]);
        supply.optional_selected.resize(3, false);
        for index in 0..3 {
            supply
                .homes
                .push(home(12, 1, index as u64, 1, 2.0, Some(index)));
        }
        supply.reserve_existing();
        assert!(!supply.claim(12, 3));
        assert!(supply.claim(12, 1));
        supply.select_optional_homes();
        assert!(
            supply.needs_home(0),
            "displaced resident's physical home is required"
        );
        assert!(supply.needs_home(1));
        assert!(!supply.needs_home(2));
    }

    #[test]
    fn arrival_places_supply_their_applicants_adults() {
        // DEM-02-C: a place offered to a couple supplies two workers, one offered to an
        // elder none, so employers materialize only the home that staffs them.
        const FAMILY_FLAT_M2: f32 = 200.0;
        let couple = (0..)
            .find(|&seed| applicant_workers(FAMILY_FLAT_M2, seed, 0..1) == 2)
            .unwrap();
        let elder = (0..)
            .find(|&seed| applicant_workers(SINGLE_FLAT_M2, seed, 0..1) == 0)
            .unwrap();
        let mut supply = calendar(0);
        supply.arrivals_possible = true;
        supply.optional_selected.resize(2, false);
        for (index, (seed, flat_size_m2)) in [(elder, SINGLE_FLAT_M2), (couple, FAMILY_FLAT_M2)]
            .into_iter()
            .enumerate()
        {
            supply.homes.push(FutureHome {
                flat_size_m2,
                optional: Some(index),
                ..home(12, 1, 0, 1, 2.0, None)
            });
            supply.homes[index].order = seed as u64;
            supply.homes[index].seed = seed;
        }
        supply.reserve_existing();
        assert!(!supply.claim(12, 3));
        assert!(supply.claim(12, 2));
        supply.select_optional_homes();
        assert!(!supply.needs_home(0), "an elder's home supplies no workers");
        assert!(supply.needs_home(1));
    }

    #[test]
    fn disconnected_town_can_rehouse_existing_workers_without_arrival_credits() {
        let mut supply = calendar(0);
        supply.displaced.reset(&[UnhousedWorkforce {
            reserve_days: 5.0,
            workers: 4,
        }]);
        supply.homes.push(home(12, 1, 0, 3, 2.0, None));
        supply.reserve_existing();
        assert!(supply.claim(12, 4));
        assert!(!supply.claim(12, 1));
    }

    #[test]
    #[ignore = "manual release completion-time household matching and prepared-claim scaling"]
    fn benchmark_future_home_matching() {
        use std::{hint::black_box, time::Instant};
        for homes in [1_024, 8_192, 65_536] {
            let households = vec![
                UnhousedWorkforce {
                    reserve_days: 5.0,
                    workers: 0
                };
                128
            ];
            let mut supply = calendar(0);
            supply.arrivals_possible = true;
            supply.homes.reserve(homes);
            let mut samples = [[0.0; 2]; 11];
            for round in 0..14 {
                supply.displaced.reset(&households);
                supply.committed.reset(37);
                supply.homes.clear();
                for index in 0..homes {
                    supply
                        .homes
                        .push(home(6 + index % 24, 1, index as u64, 1, 2.0, None));
                }
                let start = Instant::now();
                supply.reserve_existing();
                let matching_ms = start.elapsed().as_secs_f64() * 1_000.0;
                assert_eq!(supply.committed.available(36), homes as u64 - 128);
                let start = Instant::now();
                for _ in 0..512 {
                    assert!(black_box(supply.claim(36, 1)));
                }
                let claims_ms = start.elapsed().as_secs_f64() * 1_000.0;
                if round >= 3 {
                    samples[round - 3] = [matching_ms, claims_ms];
                }
            }
            let median = |column| {
                let mut values = samples.map(|sample| sample[column]);
                values.sort_by(f64::total_cmp);
                values[5]
            };
            eprintln!(
                "future_home_matching homes={homes} displaced=128 claims=512 matching_ms={:.4} prepared_claims_ms={:.4}",
                median(0),
                median(1)
            );
        }
    }

    #[test]
    fn incumbents_reserve_future_homes_before_new_projects() {
        let mut supply = calendar(2);
        supply.jobs_by_hour[0].store(4, Ordering::Relaxed);
        supply.jobs_by_hour[12].store(3, Ordering::Relaxed);
        supply.committed.add(6, 2);
        supply.optional.add(18, 5);
        supply.optional_initial[18] = 5;
        supply.reserve_existing();
        assert!(!supply.claim(12, 1));
        assert!(supply.claim(24, 2));
        assert!(!supply.claim(24, 1));
        assert_eq!(supply.optional.available(36), 0);
    }
}
