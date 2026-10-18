// SPDX-License-Identifier: GPL-2.0-only

//! Capacity and prepared-door reservations for construction awaiting minute-queue placement.

use super::actions::DemandSpawnAction;
use super::snapshot::DailyDemandSnapshot;
use super::spawn_need::candidate_economy_profile;
use super::system::DemandSystem;
use crate::simulation::buildings::allocator::{
    BuildingAllocator, BuildingEntrance, construction_duration_hours,
};
use crate::simulation::economy::definitions::RuntimeEconomyCatalog;
use crate::simulation::economy::households::applicant_places_and_members;
use crate::simulation::zoning::{ParcelId, ZoneType, ZoningSystem};

/// A queued parcel's promised capacity and actual operational completion tick.
#[derive(Clone)]
pub(super) struct QueuedConstruction {
    /// False after minute execution consumes the queue entry, even if placement failed.
    pub(super) active: bool,
    /// Id of the queue entry this project is: the earliest for its parcel.
    pub(super) spawn_id: u64,
    pub(super) parcel_id: u64,
    pub(super) completion_hour: u64,
    pub(super) household_slots: u32,
    /// Seed of a home's applicant stream: its spawn id, the applicant seed it is placed with
    /// (`SIM-02-I`), so planning and permits count the applicants it draws once placed.
    pub(super) home_seed: usize,
    /// Adults the applicant stream offers these places (`DEM-02-C`), seeded by `home_seed`.
    pub(super) household_workers: u32,
    /// Members of those applicants, whose shopping needs store workers.
    pub(super) household_members: u32,
    /// Authored flat size, for applicants offered places left after displaced residents.
    pub(super) flat_size_m2: f32,
    /// Housing tier used for the same relocation threshold as completed homes.
    pub(super) level: u8,
    pub(super) profile_runtime_id: u16,
    pub(super) commercial: bool,
    // Fixed-size proposed freight geometry; no asset strings are copied into the census.
    pub(super) entrance: Option<BuildingEntrance>,
    sequence: usize,
}

/// Hour boundaries from the current hour to the first one at or after the start minute, which
/// is at least the next minute. A project of `D` hours started then is ready `D` boundaries
/// later, as `Building::construction_hours_left` counts placed sites (`SIM-02-D`).
fn start_hour_delay(now: u64, due_minute: u64) -> u64 {
    due_minute.max(now + 1).div_ceil(60) - now / 60
}

impl DemandSystem {
    /// Refreshes derived queue capacities and doors without cloning names or changing saved state.
    /// Retains parcel lookup order and a reusable execution-order permutation for finite inputs.
    /// `queued` yields each entry's spawn id, due minute, zone and action, in queue order.
    pub(crate) fn set_queued_construction_with_entrances<'a>(
        &mut self,
        allocator: &BuildingAllocator,
        zoning: &ZoningSystem,
        now: u64,
        queued: impl IntoIterator<Item = (u64, u64, ZoneType, &'a DemandSpawnAction)>,
        site_entrance: impl Fn(&DemandSpawnAction) -> Option<BuildingEntrance>,
    ) {
        self.queued_reference_hour = now / 60;
        self.queued_construction.clear();
        let mut next_due = now.saturating_add(1);
        for (sequence, (spawn_id, due, zone, action)) in queued.into_iter().enumerate() {
            // Even a stale action consumes an execution slot before placement rejects it.
            let due = due.max(next_due);
            next_due = due.saturating_add(1);
            if !zoning
                .parcels
                .get(ParcelId::from_raw(action.parcel_id))
                .is_some_and(|parcel| parcel.is_available())
            {
                continue;
            }
            let Some(authored) = allocator
                .registry
                .get(&action.asset_id)
                .and_then(|entry| entry.manifest.building.as_ref())
            else {
                continue;
            };
            let household_slots = if zone == ZoneType::Residential {
                allocator.registry.household_capacity(&action.asset_id)
            } else {
                0
            };
            let flat_size_m2 = allocator.registry.flat_size_m2(&action.asset_id);
            let home_seed = spawn_id as usize;
            let (places, household_members) =
                applicant_places_and_members(flat_size_m2, home_seed, 0..household_slots);
            self.queued_construction.push(QueuedConstruction {
                active: true,
                spawn_id,
                level: authored.level,
                parcel_id: action.parcel_id,
                completion_hour: start_hour_delay(now, due)
                    + u64::from(construction_duration_hours(
                        zone,
                        authored.level,
                        &self.runtime_tuning,
                    )),
                household_slots,
                home_seed,
                household_workers: places[1] + 2 * places[2],
                household_members,
                flat_size_m2,
                profile_runtime_id: candidate_economy_profile(
                    allocator,
                    &self.runtime_catalog,
                    &action.asset_id,
                )
                .map_or(0, |profile| profile.runtime_id),
                commercial: zone == ZoneType::Commercial,
                entrance: if zone == ZoneType::Residential {
                    None
                } else {
                    site_entrance(action)
                },
                sequence,
            });
        }
        self.next_construction_start_hour = start_hour_delay(now, next_due);
        // Historical queues may contain repeated parcels. Only the earliest start can place.
        self.queued_construction
            .sort_unstable_by_key(|project| (project.parcel_id, project.sequence));
        self.queued_construction
            .dedup_by_key(|project| project.parcel_id);
        self.queued_supply_order.clear();
        self.queued_supply_order
            .extend(0..self.queued_construction.len());
        self.queued_supply_order
            .sort_unstable_by_key(|&index| self.queued_construction[index].sequence);
    }

    /// The active project queue entry `spawn_id` is for `parcel`: none for a later duplicate
    /// of a queued parcel or an entry already consumed. O(log Q).
    pub(super) fn queued_project(&self, spawn_id: u64, parcel: u64) -> Option<&QueuedConstruction> {
        self.queued_construction
            .binary_search_by_key(&parcel, |project| project.parcel_id)
            .ok()
            .map(|index| &self.queued_construction[index])
            .filter(|project| project.active && project.spawn_id == spawn_id)
    }

    /// Retires queue entry `spawn_id` for `parcel` and returns its recipe without moving owner
    /// identities: `None` when it was no active project (a later duplicate, or consumed).
    pub(super) fn retire_queued_construction(&mut self, spawn_id: u64, parcel: u64) -> Option<u16> {
        let index = self
            .queued_construction
            .binary_search_by_key(&parcel, |project| project.parcel_id)
            .ok()?;
        let project = &mut self.queued_construction[index];
        (project.spawn_id == spawn_id && std::mem::replace(&mut project.active, false))
            .then_some(project.profile_runtime_id)
    }

    /// A queued home's seed, applicant adults and members, as planning and permits read them.
    #[cfg(test)]
    pub(crate) fn queued_home_estimate_for_test(
        &self,
        spawn_id: u64,
        parcel: u64,
    ) -> Option<(usize, u32, u32)> {
        self.queued_project(spawn_id, parcel)
            .filter(|project| project.household_slots > 0)
            .map(|project| {
                (
                    project.home_seed,
                    project.household_workers,
                    project.household_members,
                )
            })
    }

    /// Numeric reservation fixtures do not need road geometry; runtime queues always supply doors.
    /// Entries take their queue positions as spawn ids.
    #[cfg(test)]
    pub(crate) fn set_queued_construction<'a>(
        &mut self,
        allocator: &BuildingAllocator,
        zoning: &ZoningSystem,
        now: u64,
        queued: impl IntoIterator<Item = (u64, ZoneType, &'a DemandSpawnAction)>,
    ) {
        self.set_queued_construction_with_entrances(
            allocator,
            zoning,
            now,
            queued
                .into_iter()
                .enumerate()
                .map(|(id, (due, zone, action))| (id as u64, due, zone, action)),
            |_| None,
        );
    }
}

impl DailyDemandSnapshot {
    pub(super) fn reserve_queued_construction(
        &mut self,
        catalog: &RuntimeEconomyCatalog,
        queued: &[QueuedConstruction],
    ) {
        for project in queued {
            if !project.active {
                continue;
            }
            let index = if project.household_slots > 0 {
                0
            } else if project.commercial {
                1
            } else {
                2
            };
            self.queued_by_use[index] += 1;
            self.under_construction_household_slots = self
                .under_construction_household_slots
                .saturating_add(project.household_slots);
            if let Some(profile) = catalog.profile_by_runtime_id(project.profile_runtime_id) {
                self.output_absorption.reserve_candidate(profile);
                if project.commercial {
                    for (resource, unmet) in
                        &mut self.committed_unmet_commercial_consumer_demand_by_resource
                    {
                        for output in &profile.outputs {
                            if output.resource_runtime_id == *resource {
                                *unmet =
                                    (*unmet - profile.net_output_units_per_day(output)).max(0.0);
                            }
                        }
                    }
                }
            }
        }
        // Future employers never contribute to move_in_job_slots or admission income.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_deadlines_count_boundaries_to_the_start() {
        assert_eq!(start_hour_delay(0, 1), 1);
        assert_eq!(start_hour_delay(0, 60), 1);
        assert_eq!(start_hour_delay(0, 61), 2);
        assert_eq!(start_hour_delay(60, 120), 1);
        assert_eq!(start_hour_delay(60, 121), 2);
        assert_eq!(start_hour_delay(1440, 1500), 1);
        // An overdue start runs at the next minute.
        assert_eq!(start_hour_delay(90, 30), 1);
    }
}
