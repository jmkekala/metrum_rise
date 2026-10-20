// SPDX-License-Identifier: GPL-2.0-only

//! The demand snapshot as a rolling snapshot over the hour (`SIM-02-I`,
//! `docs/simulation.md#snapshots`).
//!
//! The hourly demand pass composes its snapshot from a front buffer while a back buffer is
//! folded over the hour's `C` slots. The hour's first slot freezes the household and building
//! handle capacities `Hh` and `Hb` and the treasury; chunk `k` folds household handle slots
//! `[k·Hh/C, (k+1)·Hh/C)` and building handle slots `[k·Hb/C, (k+1)·Hb/C)`. A freed slot is
//! skipped and a slot reused before its chunk folds its new entity. The hour's last slot swaps
//! the back in.
//!
//! A buffer covers the handle slots below its end. The entities created in new slots since the
//! freeze are folded into the front at the next hour's operational hour, before its job search,
//! so a building placed or a household arrived after the freeze counts at the next pass as it
//! was before that slot's hires, like every entity the sweep folded: a hire folded after it
//! would leave its job open in the swept building and its adult employed in the new household,
//! counting one open job twice. The pass folds any created since: O(entities created since the
//! hour's first slot).
//!
//! Households fold in handle order: the affordable-consumer walk claims each household's
//! transfers from a running copy of the frozen treasury in that order, and displaced households
//! keep that order as their priority. Funded capacities and input scales come from the copies
//! the published census left on each building as it is folded, which saves keep: a census
//! rebuilt by a load or a policy change mid-hour never changes the sweep. A sweep that misses a
//! slot is abandoned and the pass keeps the last front; a world without one builds it from the
//! whole city at once, in the same order.

use std::ops::Range;

use super::{
    BuildingSnapshotAccumulator, FundedJobs, HouseholdSnapshotAccumulator, JobTotals,
    OccupantEntry, ResidentialOccupantSnapshot, SnapshotTotals, UnhousedWorkforce,
    demand_sink_rates_into,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::handles::HandleTable;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::definitions::{
    ResourceRuntimeId, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use crate::simulation::economy::demand::config::DemandConfig;
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::{
    CommercialActivitySums, CommercialActivityTerms, ConsumerBudgetTerms, HouseholdSystem,
};

/// What a chunk of the sweep reads.
#[derive(Clone, Copy)]
pub(in crate::simulation::economy::demand) struct SweepInputs<'a> {
    pub(in crate::simulation::economy::demand) allocator: &'a BuildingAllocator,
    pub(in crate::simulation::economy::demand) households: &'a HouseholdSystem,
    pub(in crate::simulation::economy::demand) config: &'a DemandConfig,
    pub(in crate::simulation::economy::demand) catalog: &'a RuntimeEconomyCatalog,
    pub(in crate::simulation::economy::demand) tuning: &'a RuntimeEconomyTuning,
    pub(in crate::simulation::economy::demand) policy: &'a CityFiscalPolicy,
    /// The city treasury; the first chunk freezes it for the transfer claims.
    pub(in crate::simulation::economy::demand) treasury: f64,
}

/// One buffer of the sweep: the totals a snapshot is composed from and the residential
/// occupants the planner reads, over the household and building handle slots below its ends.
#[derive(Clone, Debug, Default)]
pub(in crate::simulation::economy::demand) struct SnapshotBuffer {
    pub(in crate::simulation::economy::demand) totals: SnapshotTotals,
    pub(in crate::simulation::economy::demand) occupants: ResidentialOccupantSnapshot,
    // Fold slots the buffer covers: frozen at a sweep's first chunk, raised by `top_up`.
    households_end: u64,
    buildings_end: u64,
    // What remains of the frozen treasury after the folded households' transfer claims.
    treasury: f64,
}

/// The hour-long sweep: the front the pass reads, the back being folded and its cursor. Front,
/// back and cursor are saved, so a loaded game continues bit-identically.
#[derive(Default)]
pub(in crate::simulation::economy::demand) struct DemandSnapshotSweep {
    front: Option<SnapshotBuffer>,
    back: SnapshotBuffer,
    // Chunk the sweep in progress expects next; `None` between sweeps. A slot that skips a
    // chunk abandons the sweep.
    next_chunk: Option<u64>,
    // Catalog terms rebuilt in each chunk without allocating; not saved.
    sink_rates: Vec<(ResourceRuntimeId, f32)>,
    store_outputs: Vec<(ResourceRuntimeId, f32)>,
}

/// The demand snapshot sweep as saved (`SIM-02-I`); the demand system owns the buffers'
/// encoding.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SavedDemandSnapshot {
    /// Encoded front; `None` before a world's first finished sweep or synchronous build.
    pub(crate) front: Option<Vec<u8>>,
    /// The sweep in progress: its next chunk and encoded back; `None` between sweeps.
    pub(crate) sweep: Option<(u64, Vec<u8>)>,
}

impl DemandSnapshotSweep {
    /// Runs chunk `slot mod SLOTS_PER_HOUR` of the hour's sweep; the caller registers appended
    /// entities' handles first. O(Hh/C + Hb/C) and allocation-free once the buffers have
    /// grown; the first chunk empties the back in O(1).
    pub(in crate::simulation::economy::demand) fn advance(
        &mut self,
        slot: u64,
        inputs: &SweepInputs<'_>,
    ) {
        let chunk = slot % SLOTS_PER_HOUR;
        if chunk == 0 {
            self.back.start(inputs);
        } else if self.next_chunk != Some(chunk) {
            self.next_chunk = None;
            return;
        }
        demand_sink_rates_into(
            inputs.catalog,
            &mut self.store_outputs,
            &mut self.sink_rates,
        );
        let households = part(chunk, self.back.households_end);
        let buildings = part(chunk, self.back.buildings_end);
        self.back.fold_households(households, inputs);
        self.back
            .fold_buildings(buildings, inputs, &self.sink_rates);
        if chunk < SLOTS_PER_HOUR - 1 {
            self.next_chunk = Some(chunk + 1);
            return;
        }
        match &mut self.front {
            Some(front) => std::mem::swap(front, &mut self.back),
            None => self.front = Some(std::mem::take(&mut self.back)),
        }
        self.next_chunk = None;
    }

    /// Moves the front out for the pass, which also mutates the demand system;
    /// [`Self::restore_front`] puts it back. The entities created in new handle slots since
    /// the front's sweep began are folded first, O(those entities). A world without a front
    /// builds one from the whole city at once, in the sweep's order: O(households + buildings),
    /// serial.
    pub(in crate::simulation::economy::demand) fn take_front(
        &mut self,
        inputs: &SweepInputs<'_>,
    ) -> SnapshotBuffer {
        match self.front.take() {
            Some(mut front) => {
                front.top_up(inputs);
                front
            }
            None => build(inputs),
        }
    }

    /// Folds the entities created in new handle slots since the front's sweep began, before
    /// the hour's job search (see the module docs); a world without a front builds one at the
    /// pass. O(those entities).
    pub(in crate::simulation::economy::demand) fn top_up_front(
        &mut self,
        inputs: &SweepInputs<'_>,
    ) {
        if let Some(front) = &mut self.front {
            front.top_up(inputs);
        }
    }

    /// The saved form of the front [`Self::top_up_front`] would leave now, without changing it.
    #[cfg(test)]
    pub(in crate::simulation::economy::demand) fn topped_up_front_bits(
        &self,
        inputs: &SweepInputs<'_>,
    ) -> Option<Vec<u8>> {
        self.front.as_ref().map(|front| {
            let mut front = front.clone();
            front.top_up(inputs);
            encode(&front)
        })
    }

    /// Returns a front taken with [`Self::take_front`].
    pub(in crate::simulation::economy::demand) fn restore_front(&mut self, front: SnapshotBuffer) {
        self.front = Some(front);
    }

    /// Drops the front so that the next pass builds a current one, for subsystem tests that
    /// plan on hand-built state without the slot clock. A sweep in progress continues.
    #[cfg(test)]
    pub(in crate::simulation::economy::demand) fn discard_front(&mut self) {
        self.front = None;
    }

    /// Saved form; [`Self::restore`] rebuilds an identical sweep.
    pub(in crate::simulation::economy::demand) fn save(&self) -> SavedDemandSnapshot {
        SavedDemandSnapshot {
            front: self.front.as_ref().map(encode),
            sweep: self
                .next_chunk
                .map(|next_chunk| (next_chunk, encode(&self.back))),
        }
    }

    /// Rebuilds a saved sweep, rejecting malformed buffers.
    pub(in crate::simulation::economy::demand) fn restore(
        saved: &SavedDemandSnapshot,
    ) -> Result<Self, String> {
        let mut sweep = Self {
            front: saved.front.as_deref().map(decode).transpose()?,
            ..Self::default()
        };
        if let Some((next_chunk, back)) = &saved.sweep {
            if !(1..SLOTS_PER_HOUR).contains(next_chunk) {
                return Err("demand snapshot chunk outside the hour".into());
            }
            sweep.next_chunk = Some(*next_chunk);
            sweep.back = decode(back)?;
        }
        Ok(sweep)
    }
}

impl SnapshotBuffer {
    // Empties the buffer in O(1), keeping its storage, and freezes its ends and the treasury.
    fn start(&mut self, inputs: &SweepInputs<'_>) {
        self.totals.clear();
        self.occupants.clear();
        self.households_end = household_slots(inputs);
        self.buildings_end = building_slots(inputs);
        self.treasury = inputs.treasury.max(0.0);
    }

    // Folds the entities created in new handle slots since the buffer's ends were set.
    fn top_up(&mut self, inputs: &SweepInputs<'_>) {
        let mut sink_rates = Vec::new();
        demand_sink_rates_into(inputs.catalog, &mut Vec::new(), &mut sink_rates);
        let households = household_slots(inputs).max(self.households_end);
        self.fold_households(self.households_end as usize..households as usize, inputs);
        self.households_end = households;
        let buildings = building_slots(inputs).max(self.buildings_end);
        self.fold_buildings(
            self.buildings_end as usize..buildings as usize,
            inputs,
            &sink_rates,
        );
        self.buildings_end = buildings;
    }

    // Folds household fold slots `slots` in order; their transfer claims draw on the buffer's
    // treasury.
    fn fold_households(&mut self, slots: Range<usize>, inputs: &SweepInputs<'_>) {
        let SweepInputs {
            allocator,
            households,
            config,
            catalog,
            tuning,
            policy,
            ..
        } = *inputs;
        let activity = CommercialActivityTerms::new(catalog);
        let budgets = ConsumerBudgetTerms::new(catalog, tuning, policy);
        let totals = &mut self.totals;
        for slot in slots {
            let Some(index) = fold_dense(&households.handles, households.households.len(), slot)
            else {
                continue;
            };
            let household = &households.households[index];
            totals
                .households
                .absorb_household(allocator, catalog, tuning, config, household);
            totals
                .activity
                .add_household(activity, household, allocator);
            if let Some(residents) =
                budgets.affordable_residents(household, allocator, policy, &mut self.treasury)
            {
                totals.affordable_residents += residents;
            }
            if let Some(workforce) = UnhousedWorkforce::of(household, allocator, catalog, tuning) {
                totals.unhoused_workforce.push(workforce);
            }
            self.occupants
                .add_household(allocator, catalog, tuning, household);
        }
    }

    // Folds building fold slots `slots` in order.
    fn fold_buildings(
        &mut self,
        slots: Range<usize>,
        inputs: &SweepInputs<'_>,
        sink_rates: &[(ResourceRuntimeId, f32)],
    ) {
        let SweepInputs {
            allocator,
            catalog,
            policy,
            ..
        } = *inputs;
        let activity = CommercialActivityTerms::new(catalog);
        let totals = &mut self.totals;
        for slot in slots {
            let Some(index) =
                fold_dense(&allocator.building_handles, allocator.buildings.len(), slot)
            else {
                continue;
            };
            let building = &allocator.buildings[index];
            totals.buildings.absorb_building(
                allocator,
                catalog,
                policy.income_tax_rate,
                sink_rates,
                FundedJobs::Published,
                index,
                building,
            );
            totals.activity.add_building(activity, catalog, building);
        }
    }
}

// The whole city folded at once, in the sweep's order and with its treasury rule: the result of
// an uninterrupted sweep over unchanged state.
fn build(inputs: &SweepInputs<'_>) -> SnapshotBuffer {
    let mut buffer = SnapshotBuffer::default();
    buffer.start(inputs);
    let mut sink_rates = Vec::new();
    demand_sink_rates_into(inputs.catalog, &mut Vec::new(), &mut sink_rates);
    buffer.fold_households(0..buffer.households_end as usize, inputs);
    buffer.fold_buildings(0..buffer.buildings_end as usize, inputs, &sink_rates);
    buffer
}

fn household_slots(inputs: &SweepInputs<'_>) -> u64 {
    fold_capacity(
        &inputs.households.handles,
        inputs.households.households.len(),
    ) as u64
}

fn building_slots(inputs: &SweepInputs<'_>) -> u64 {
    fold_capacity(
        &inputs.allocator.building_handles,
        inputs.allocator.buildings.len(),
    ) as u64
}

// Part `chunk` of the hour's equal parts of `0..total`.
fn part(chunk: u64, total: u64) -> Range<usize> {
    (chunk * total / SLOTS_PER_HOUR) as usize..((chunk + 1) * total / SLOTS_PER_HOUR) as usize
}

// Fold slots of a store of `len` entities: its handle slots, then the entities appended to it
// directly and not yet registered (tests and tools fill stores directly), in dense order.
fn fold_capacity<K>(handles: &HandleTable<K>, len: usize) -> usize {
    handles.capacity() + len.saturating_sub(handles.registered())
}

// Dense index of the entity in fold slot `slot`; `None` for a free handle slot.
fn fold_dense<K>(handles: &HandleTable<K>, len: usize, slot: usize) -> Option<usize> {
    match slot.checked_sub(handles.capacity()) {
        None => handles.dense_at(slot),
        Some(offset) => Some(handles.registered() + offset).filter(|&dense| dense < len),
    }
}

// Encoded buffer: little-endian scalars in field order, then each list as a `u32` length and
// its items.
fn encode(buffer: &SnapshotBuffer) -> Vec<u8> {
    let mut out = Vec::new();
    let put_u32 = |out: &mut Vec<u8>, value: u32| out.extend_from_slice(&value.to_le_bytes());
    let put_f32 = |out: &mut Vec<u8>, value: f32| out.extend_from_slice(&value.to_le_bytes());
    let totals = &buffer.totals;
    let households = &totals.households;
    for value in [
        households.housed_resident_count,
        households.housed_unemployed_count,
        households.live_child_count,
        households.live_elder_count,
        households.housed_household_count,
        households.unhoused_household_count,
        households.zero_budget_household_count,
        households.persistent_exit_eligible_household_count,
    ] {
        put_u32(&mut out, value);
    }
    let buildings = &totals.buildings;
    for value in [
        buildings.total_household_slots,
        buildings.occupied_household_slots,
        buildings.under_construction_household_slots,
        buildings.constructing_by_use[0],
        buildings.constructing_by_use[1],
        buildings.constructing_by_use[2],
        buildings.jobs.open_job_slots,
        buildings.jobs.physical_worker_capacity,
        buildings.jobs.funded_worker_capacity,
        buildings.jobs.open_jobs_unfunded,
    ] {
        put_u32(&mut out, value);
    }
    put_f32(&mut out, buildings.jobs.open_job_wage_sum);
    for list in [
        &buildings.committed_output_capacity_by_resource,
        &buildings.committed_commercial_output_capacity_by_resource,
        &buildings.business_input_need_by_resource,
    ] {
        put_u32(&mut out, list.len() as u32);
        for &(resource, amount) in list {
            out.extend_from_slice(&resource.to_le_bytes());
            put_f32(&mut out, amount);
        }
    }
    for value in [
        totals.activity.daily_consumption,
        totals.activity.stock_gap,
        totals.activity.retail_stock,
    ] {
        put_f32(&mut out, value);
    }
    out.extend_from_slice(&totals.affordable_residents.to_le_bytes());
    put_u32(&mut out, totals.unhoused_workforce.len() as u32);
    for workforce in &totals.unhoused_workforce {
        put_f32(&mut out, workforce.reserve_days);
        put_u32(&mut out, workforce.workers);
    }
    put_u32(&mut out, buffer.occupants.entries.len() as u32);
    for entry in &buffer.occupants.entries {
        put_u32(&mut out, entry.generation);
        put_u32(&mut out, entry.households);
        put_f32(&mut out, entry.min_reserve_days);
    }
    out.extend_from_slice(&buffer.households_end.to_le_bytes());
    out.extend_from_slice(&buffer.buildings_end.to_le_bytes());
    out.extend_from_slice(&buffer.treasury.to_le_bytes());
    out
}

fn decode(bytes: &[u8]) -> Result<SnapshotBuffer, String> {
    let mut reader = Reader(bytes);
    let [
        housed_resident_count,
        housed_unemployed_count,
        live_child_count,
        live_elder_count,
        housed_household_count,
        unhoused_household_count,
        zero_budget_household_count,
        persistent_exit_eligible_household_count,
    ] = reader.u32s()?;
    let households = HouseholdSnapshotAccumulator {
        housed_resident_count,
        housed_unemployed_count,
        live_child_count,
        live_elder_count,
        housed_household_count,
        unhoused_household_count,
        zero_budget_household_count,
        persistent_exit_eligible_household_count,
    };
    let [
        total_household_slots,
        occupied_household_slots,
        under_construction_household_slots,
        constructing_residential,
        constructing_commercial,
        constructing_industrial,
        open_job_slots,
        physical_worker_capacity,
        funded_worker_capacity,
        open_jobs_unfunded,
    ] = reader.u32s()?;
    let open_job_wage_sum = reader.f32()?;
    let committed_output_capacity_by_resource = reader.resource_amounts()?;
    let committed_commercial_output_capacity_by_resource = reader.resource_amounts()?;
    let business_input_need_by_resource = reader.resource_amounts()?;
    let buildings = BuildingSnapshotAccumulator {
        total_household_slots,
        occupied_household_slots,
        under_construction_household_slots,
        constructing_by_use: [
            constructing_residential,
            constructing_commercial,
            constructing_industrial,
        ],
        jobs: JobTotals {
            open_job_slots,
            open_job_wage_sum,
            physical_worker_capacity,
            funded_worker_capacity,
            open_jobs_unfunded,
        },
        committed_output_capacity_by_resource,
        committed_commercial_output_capacity_by_resource,
        business_input_need_by_resource,
    };
    let activity = CommercialActivitySums {
        daily_consumption: reader.f32()?,
        stock_gap: reader.f32()?,
        retail_stock: reader.f32()?,
    };
    let affordable_residents = f64::from_le_bytes(reader.take()?);
    let unhoused_workforce = (0..reader.len(8)?)
        .map(|_| {
            Ok(UnhousedWorkforce {
                reserve_days: reader.f32()?,
                workers: reader.u32()?,
            })
        })
        .collect::<Result<_, String>>()?;
    let entries = (0..reader.len(12)?)
        .map(|_| {
            Ok(OccupantEntry {
                generation: reader.u32()?,
                households: reader.u32()?,
                min_reserve_days: reader.f32()?,
            })
        })
        .collect::<Result<_, String>>()?;
    let households_end = u64::from_le_bytes(reader.take()?);
    let buildings_end = u64::from_le_bytes(reader.take()?);
    let treasury = f64::from_le_bytes(reader.take()?);
    if !reader.0.is_empty() {
        return Err("demand snapshot buffer has trailing bytes".into());
    }
    Ok(SnapshotBuffer {
        totals: SnapshotTotals {
            households,
            buildings,
            activity,
            affordable_residents,
            unhoused_workforce,
        },
        occupants: ResidentialOccupantSnapshot { entries },
        households_end,
        buildings_end,
        treasury,
    })
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or("demand snapshot buffer is truncated")?;
        self.0 = rest;
        Ok(*head)
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.take().map(u32::from_le_bytes)
    }

    fn f32(&mut self) -> Result<f32, String> {
        self.take().map(f32::from_le_bytes)
    }

    fn u32s<const N: usize>(&mut self) -> Result<[u32; N], String> {
        let mut values = [0; N];
        for value in &mut values {
            *value = self.u32()?;
        }
        Ok(values)
    }

    // A list length whose items of `item_bytes` each fit in the rest of the buffer.
    fn len(&mut self, item_bytes: usize) -> Result<usize, String> {
        let len = self.u32()? as usize;
        if len.saturating_mul(item_bytes) > self.0.len() {
            return Err("demand snapshot list overruns its buffer".into());
        }
        Ok(len)
    }

    fn resource_amounts(&mut self) -> Result<Vec<(ResourceRuntimeId, f32)>, String> {
        (0..self.len(6)?)
            .map(|_| Ok((u16::from_le_bytes(self.take()?), self.f32()?)))
            .collect()
    }
}
