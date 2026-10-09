// SPDX-License-Identifier: GPL-2.0-only

//! Completion-time labour reservations shared by all private projects in one planning pass.

use std::sync::atomic::{AtomicU32, Ordering};

use rayon::prelude::*;

use super::actions::DemandSpawnAction;
use super::queued::QueuedConstruction;
use super::snapshot::{DailyDemandSnapshot, UnhousedWorkforce};
use crate::simulation::buildings::allocator::{BuildingAllocator, construction_duration_hours};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::households::level_tuning_value;
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

struct FutureHome {
    hour: usize,
    level: u8,
    order: u64,
    slots: u32,
    threshold: f32,
    optional: Option<usize>,
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
    /// Collects already committed housing and workplaces once, in parallel.
    pub(super) fn reset(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        snapshot: &DailyDemandSnapshot,
        queued: &[QueuedConstruction],
        new_start_hour: u64,
        reservations: Option<&super::supply::ProjectReservations>,
    ) {
        self.horizon = allocator
            .buildings
            .par_iter()
            .map(|building| usize::from(building.construction_remaining_hours))
            .max()
            .unwrap_or(0)
            .max(
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
        for bins in [&mut self.homes_by_hour, &mut self.jobs_by_hour] {
            bins.resize_with(hours, || AtomicU32::new(0));
            for bin in bins {
                bin.store(0, Ordering::Relaxed);
            }
        }
        self.committed.reset(hours);
        self.optional.reset(hours);
        self.optional_initial.resize(hours, 0);
        self.optional_initial.fill(0);
        self.unemployed = snapshot.existing_unemployed_member_count;
        self.unfilled_existing = 0;
        let arrivals_possible = snapshot.connected_border_count > 0;
        self.arrivals_possible = arrivals_possible;
        self.displaced.reset(&snapshot.unhoused_workforce);
        self.optional_selected.clear();
        self.homes.clear();
        let thresholds = &tuning
            .households
            .residential_move_in_min_reserve_days_by_level;
        let match_displaced = !snapshot.unhoused_workforce.is_empty();
        let homes_by_hour = &self.homes_by_hour;
        let jobs_by_hour = &self.jobs_by_hour;
        self.homes
            .par_extend(allocator.buildings.par_iter().enumerate().filter_map(
                |(idx, building)| {
                    if building.broken
                        || building.economy_broken
                        || building.is_deserted
                        || building.pending_redevelopment
                    {
                        return None;
                    }
                    let hour = usize::from(building.construction_remaining_hours);
                    if building.is_under_construction()
                        && matches!(
                            building.zone_type,
                            ZoneType::Commercial | ZoneType::Industrial
                        )
                        && let Some(profile) =
                            catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                    {
                        jobs_by_hour[hour].fetch_add(
                            reservations.map_or_else(
                                || snapshot.output_absorption.committed_workers(profile),
                                |claims| claims.building_workers.get(idx).copied().unwrap_or(0)),
                            Ordering::Relaxed,
                        );
                    }
                    let slots = if building.is_under_construction() {
                        allocator.registry.household_capacity(&building.asset_id)
                    } else {
                        allocator
                            .household_capacity(idx)
                            .saturating_sub(building.occupancy)
                    };
                    if !match_displaced {
                        if arrivals_possible && slots > 0 {
                            homes_by_hour[hour].fetch_add(slots, Ordering::Relaxed);
                        }
                        return None;
                    }
                    (slots > 0).then(|| FutureHome {
                        hour,
                        level: building.level,
                        order: idx as u64,
                        slots,
                        threshold: level_tuning_value(thresholds, building.level),
                        optional: None,
                    })
                },
            ));
        for project in queued {
            if !project.active { continue; }
            let hour = self.hours.binary_search(&project.completion_hour).unwrap();
            if project.household_slots > 0 && !match_displaced {
                if arrivals_possible {
                    self.homes_by_hour[hour].fetch_add(project.household_slots, Ordering::Relaxed);
                }
            } else if project.household_slots > 0 {
                self.homes.push(FutureHome {
                    hour,
                    level: project.level,
                    order: project.parcel_id,
                    slots: project.household_slots,
                    threshold: level_tuning_value(thresholds, project.level),
                    optional: None,
                });
            }
            if let Some(profile) = catalog.profile_by_runtime_id(project.profile_runtime_id) {
                self.jobs_by_hour[hour].fetch_add(
                    reservations.map_or_else(
                        || snapshot.output_absorption.committed_workers(profile),
                        |claims| claims.queued_workers.binary_search_by_key(&project.parcel_id, |&(parcel, _)| parcel)
                            .ok().map_or(0, |index| claims.queued_workers[index].1)),
                    Ordering::Relaxed,
                );
            }
        }
        self.jobs_by_hour[0].fetch_add(snapshot.open_job_slots, Ordering::Relaxed);
        for (hour, homes) in self.homes_by_hour.iter().enumerate() {
            self.committed.add(hour, homes.load(Ordering::Relaxed));
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
            slots: allocator.registry.household_capacity(&action.asset_id),
            threshold: level_tuning_value(
                &tuning
                    .households
                    .residential_move_in_min_reserve_days_by_level,
                authored.level,
            ),
            optional,
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
        if u64::from(self.unemployed) + self.committed.available(hour)
            + self.optional.available(hour) < u64::from(workers) {
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
    pub(super) fn unreserved_ready_homes(&self) -> u32 {
        self.committed.sums[self.committed.leaves] as u32
    }

    /// Removes only ready housing left after workplace claims, never local workers or future homes.
    /// Rejected downsizing leaves the calendar unchanged; a successful claim costs O(log H).
    pub(super) fn retire_ready_homes(&mut self, slots: u32) -> bool {
        if self.unreserved_ready_homes() < slots {
            return false;
        }
        self.committed.take(0, slots);
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
        for home in &self.homes {
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
                workers += remaining;
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
            if u64::from(self.optional_initial[home.hour]) > remaining {
                self.optional_selected[index] = true;
                self.optional_initial[home.hour] =
                    self.optional_initial[home.hour].saturating_sub(home.slots);
            }
        }
    }

    /// Reads the decision in the original optional-candidate order.
    pub(super) fn needs_home(&self, index: usize) -> bool {
        self.optional_selected[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        supply.committed.add(12, 8);
        assert!(supply.claim_ready(5)); // Two residents and three ready homes.
        assert!(!supply.retire_ready_homes(3));
        assert_eq!(supply.unreserved_ready_homes(), 2);
        assert!(supply.retire_ready_homes(2));
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
        supply.homes.push(FutureHome {
            hour: 18,
            level: 2,
            order: 1,
            slots: 2,
            threshold: 2.0,
            optional: None,
        });
        supply.homes.push(FutureHome {
            hour: 6,
            level: 1,
            order: 2,
            slots: 1,
            threshold: 30.0,
            optional: None,
        });
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
            supply.homes.push(FutureHome {
                hour: 12,
                level: 1,
                order: index as u64,
                slots: 1,
                threshold: 2.0,
                optional: Some(index),
            });
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
    fn disconnected_town_can_rehouse_existing_workers_without_arrival_credits() {
        let mut supply = calendar(0);
        supply.displaced.reset(&[UnhousedWorkforce {
            reserve_days: 5.0,
            workers: 4,
        }]);
        supply.homes.push(FutureHome {
            hour: 12,
            level: 1,
            order: 0,
            slots: 3,
            threshold: 2.0,
            optional: None,
        });
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
                    supply.homes.push(FutureHome {
                        hour: 6 + index % 24,
                        level: 1,
                        order: index as u64,
                        slots: 1,
                        threshold: 2.0,
                        optional: None,
                    });
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
