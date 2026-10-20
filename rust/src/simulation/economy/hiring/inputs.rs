// SPDX-License-Identifier: GPL-2.0-only

//! The funding census's rolling input copy (`SIM-02-I`, `docs/simulation.md#pure-jobs`).
//!
//! The hourly census reads a copy of what it reads besides the network instead of the live
//! stores: the buildings, the households, each building's paid freight and the treasury, fiscal
//! policy, city electricity funding and clock. A back copy is filled over the hour's first
//! [`FILL_SLOTS`] slots in dense order and becomes the front at the last of them; the census
//! then reads the front over the hour's second half and publishes at the next hour's first slot
//! (`hiring/rolling.rs`). A row is the entity as it stood at its copy slot. Entrances and the
//! asset registry are derived from the network and the asset packs, so the front takes them
//! from the live allocator when the census starts.
//!
//! **While a census reads the front** (from its start until it is published) the front keeps
//! following removals, which restart the census, but not in-place changes or appends: building
//! rows changed in place are kept in a deferred list, and a census restart copies them and the
//! rows created since before it starts again.
//!
//! **Alignment.** A copy holds a prefix of a live dense store: row `i` is the entity at live
//! index `i`. A swap-remove is mirrored in O(1): when the moved last entity has been copied, its
//! row moves; otherwise the live entity is copied into the removed row now. A building removal
//! also retargets the copied households' home and store references, an O(households) parallel
//! pass like the live one. The allocator names the buildings changed in place
//! (`RowChanges`); each slot and the census copy those rows again, so the front the census reads
//! has the live structure and claims the live building and entrance revisions: caches keyed on
//! them stay valid for the live allocator. A remap the copy cannot follow (undo, chained
//! mappings) or a cleared store drops both copies, and the census then copies the city at once.
//!
//! **Costs.** Fill slot `k` copies the rows up to `(k+1)/C` of the store lengths at the hour's
//! first slot, `C` = [`FILL_SLOTS`], O((B + H)/C) row copies; the last fill slot copies the
//! rows created since and sums the freight working capital, O(B + S) as the census did. The census copies the entrances,
//! O(B), only after their revision changed. Rows reuse their storage, so steady state allocates
//! nothing; growth and removal allocate or free the rows they add or drop.

use std::collections::HashMap;

use rayon::prelude::*;

use super::JobFunding;
use super::steps::{CensusContext, CensusProgress};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator, RowChanges};
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::households::{
    Household, service_funded_worker_capacity, write_electricity_funding_by_building,
};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};

mod encoding;
#[cfg(test)]
mod tests;

use encoding::{Reader, Writer};

// The copy's production revisions count from here, so they never equal a live allocator's: a
// consumer that last read one store rebuilds on the other.
const COPY_PRODUCTION_REVISIONS: u64 = 1 << 63;

/// Slots of the hour's first half, over which the copy is filled.
pub(crate) const FILL_SLOTS: u64 = SLOTS_PER_HOUR / 2;

/// The census parameters a finished copy freezes at its last slot.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrozenCensusParameters {
    /// City treasury balance.
    pub(crate) treasury: f64,
    /// Fiscal policy.
    pub(crate) fiscal: CityFiscalPolicy,
    /// City electricity service funding; each power producer's override applies on top of it.
    pub(crate) electricity_funding: f32,
    /// Minute of day at which the census the copy feeds opens: the next hour's.
    pub(crate) minute_of_day: u16,
}

/// The census input copy as saved; [`CensusInputs`] owns the encoding.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SavedCensusInputs {
    /// The finished copy no census has read yet; `None` otherwise.
    pub(crate) front: Option<Vec<u8>>,
    /// The fill in progress; `None` between fills.
    pub(crate) back: Option<Vec<u8>>,
    /// The building rows changed in place since the copy last read them.
    pub(crate) changes: Vec<u8>,
    /// Whether a census reads the front, and the front rows it defers.
    pub(crate) deferred: Vec<u8>,
    /// The census published at the hour's first slot, saved before the hour's actions slot,
    /// whose planning reads it ([`CensusInputs::save_published`]); `None` otherwise.
    pub(crate) published: Option<Vec<u8>>,
}

// A copied prefix `items[..len]` of a live store. Items past `len` are storage kept so that
// copying into them reuses their allocations.
struct Rows<T> {
    items: Vec<T>,
    len: usize,
}

impl<T> Default for Rows<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            len: 0,
        }
    }
}

impl<T: Clone + Send + Sync> Rows<T> {
    fn rows(&self) -> &[T] {
        &self.items[..self.len]
    }

    fn rows_mut(&mut self) -> &mut [T] {
        &mut self.items[..self.len]
    }

    // Copies `row` into row `index`, at most one past the prefix; extends the prefix to it.
    fn set(&mut self, index: usize, row: &T) {
        match self.items.get_mut(index) {
            Some(item) => item.clone_from(row),
            None => self.items.push(row.clone()),
        }
        self.len = self.len.max(index + 1);
    }

    // Copies live rows `len..end`. O(end - len).
    fn copy_through(&mut self, live: &[T], end: usize) {
        for (index, row) in live.iter().enumerate().take(end).skip(self.len) {
            self.set(index, row);
        }
    }

    // Mirrors the live swap-remove of `removed`, whose last row is `last`, before it happens.
    fn mirror_swap_remove(&mut self, removed: usize, last: usize, live: &[T]) {
        if removed >= self.len {
            return;
        }
        if last < self.len {
            self.items.swap(removed, self.len - 1);
            self.len -= 1;
        } else {
            self.items[removed].clone_from(&live[last]);
        }
    }

    // Copies every row of the prefix again. O(len), parallel.
    fn copy_all_again(&mut self, live: &[T]) {
        let len = self.len;
        self.items[..len]
            .par_iter_mut()
            .zip(&live[..len])
            .for_each(|(row, live)| row.clone_from(live));
    }

    // Hands the prefix over as `target`, exactly sized, and keeps `target`'s old rows as storage.
    fn publish_into(&mut self, target: &mut Vec<T>) {
        self.items.truncate(self.len);
        std::mem::swap(&mut self.items, target);
        self.len = 0;
    }
}

// Adds `changes` to the rows deferred for a front a census reads; more rows than the store holds
// collapse into `all`, as in the allocator's list. O(changed rows), allocation-free once grown.
fn defer_changes(deferred: &mut RowChanges, changes: &RowChanges, buildings: usize) {
    if deferred.all {
        return;
    }
    if changes.all || deferred.rows.len() + changes.rows.len() > buildings.saturating_add(2) {
        deferred.all = true;
        deferred.rows.clear();
    } else {
        deferred.rows.extend_from_slice(&changes.rows);
    }
}

// The fill in progress.
#[derive(Default)]
struct Back {
    buildings: Rows<Building>,
    households: Rows<Household>,
    // Live store lengths at the hour's first slot; slot `k` copies up to `(k+1)/C` of them.
    totals: [usize; 2],
    // Live production revision at the hour's first slot: every row is a later copy. `None`
    // after a load, whose live revisions count afresh.
    production_floor: Option<u64>,
    // Chunk the fill expects next; `None` between fills. A missed slot abandons the fill.
    next_chunk: Option<u64>,
}

// The copy a census reads.
#[derive(Default)]
struct Front {
    // `buildings` is the copy; `entrances` and `registry` are taken from the live allocator when
    // the census starts, and the building and entrance revisions are the live ones then.
    allocator: BuildingAllocator,
    households: Vec<Household>,
    // Paid goods and unsettled sales of each building when the copy finished.
    freight_capital: Vec<f64>,
    // Electricity service funding by building, derived from the copy when the census starts.
    service_funding: Vec<f32>,
    parameters: FrozenCensusParameters,
    production_floor: Option<u64>,
}

/// Front and back copies of the census inputs, the census's own freight route cache and the
/// copy's revisions (`SIM-02-I`); see the module documentation. Front, back and the pending row
/// changes are saved, so a loaded game continues bit-identically.
pub(crate) struct CensusInputs {
    front: Front,
    // Whether the front holds a copy no census has published; only then is it kept aligned.
    pending: bool,
    // Whether a census reads the front: it then keeps its rows and defers in-place changes.
    reading: bool,
    // Front rows changed in place while a census reads it, copied when the census restarts.
    deferred: RowChanges,
    back: Back,
    // An empty shipment store used only for its freight route cache, keyed on the front's
    // revisions, so the census and live logistics never invalidate each other's routes.
    routes: ShipmentSystem,
    // Live entrance and registry revisions the front's entrances and registry were taken at.
    entrances_taken: Option<u64>,
    registry_taken: Option<u64>,
    // Production floor of the front the last census read, and the copy's production revision.
    census_floor: Option<u64>,
    production_revision: u64,
    // The building swap-remove `invalidate_building` announced: `(last, removed)`.
    expected_remap: Option<(usize, usize)>,
    // Changes restored by a load, adopted by the loaded allocator once loading is done.
    loaded_changes: Option<RowChanges>,
    changed_owners: Vec<usize>,
    // Whether the front is the copy the published census read, unchanged since: from the
    // publication until the fill's last slot replaces it or a census prepares it again. While
    // it is not pending, removals, in-place changes and policy changes leave it alone.
    published_front: bool,
    // The published census a load restored: `Some(None)` for books that were not valid at the
    // save, `Some(Some(adopted))` for a census to recompute from the restored front.
    loaded_published: Option<Option<Vec<u32>>>,
}

impl Default for CensusInputs {
    fn default() -> Self {
        Self {
            front: Front::default(),
            pending: false,
            reading: false,
            deferred: RowChanges::default(),
            back: Back::default(),
            routes: ShipmentSystem::new(),
            entrances_taken: None,
            registry_taken: None,
            census_floor: None,
            production_revision: COPY_PRODUCTION_REVISIONS,
            expected_remap: None,
            loaded_changes: None,
            changed_owners: Vec::new(),
            published_front: false,
            loaded_published: None,
        }
    }
}

// End of part `chunk` of the fill's equal parts of `0..total`.
fn part_end(chunk: u64, total: usize) -> usize {
    ((chunk + 1) * total as u64 / FILL_SLOTS) as usize
}

impl CensusInputs {
    /// Runs chunk `slot mod 960` of the hour's fill while it is below [`FILL_SLOTS`]: the first
    /// chunk starts it, the last one finishes it with `parameters` and swaps it in as the
    /// front. Building rows changed in place since the last slot are copied again first, or
    /// deferred while a census reads the front. O((B + H)/C) row copies plus the changed rows;
    /// the last chunk adds the rows created during the fill and O(B + S) for the freight working
    /// capital. Later slots only take the changed rows.
    pub(crate) fn advance(
        &mut self,
        slot: u64,
        live: &mut BuildingAllocator,
        households: &[Household],
        logistics: &ShipmentSystem,
        parameters: FrozenCensusParameters,
    ) {
        self.copy_changed_rows(live);
        let chunk = slot % SLOTS_PER_HOUR;
        if chunk >= FILL_SLOTS {
            return;
        }
        let back = &mut self.back;
        if chunk == 0 {
            back.buildings.len = 0;
            back.households.len = 0;
            back.totals = [live.buildings.len(), households.len()];
            back.production_floor = Some(live.production_revision());
        } else if back.next_chunk != Some(chunk) {
            back.next_chunk = None;
            return;
        }
        let last = chunk == FILL_SLOTS - 1;
        let [buildings, household_total] = if last {
            [live.buildings.len(), households.len()]
        } else {
            [
                part_end(chunk, back.totals[0]).min(live.buildings.len()),
                part_end(chunk, back.totals[1]).min(households.len()),
            ]
        };
        back.buildings.copy_through(&live.buildings, buildings);
        back.households.copy_through(households, household_total);
        if !last {
            back.next_chunk = Some(chunk + 1);
            return;
        }
        back.next_chunk = None;
        let front = &mut self.front;
        back.buildings.publish_into(&mut front.allocator.buildings);
        back.households.publish_into(&mut front.households);
        front.freight_capital.clear();
        front
            .freight_capital
            .resize(front.allocator.buildings.len(), 0.0);
        logistics.write_freight_working_capital(&mut front.freight_capital);
        front.parameters = parameters;
        front.production_floor = back.production_floor;
        self.pending = true;
        self.published_front = false;
        // A census of an older front that was never published is abandoned with it.
        self.reading = false;
        self.deferred = RowChanges::default();
    }

    /// Drops the finished copy, so the next census copies the city at once: for test drivers
    /// that publish a census of a city they built by hand. A fill in progress continues.
    #[cfg(test)]
    pub(crate) fn discard_front(&mut self) {
        self.pending = false;
        self.reading = false;
        self.deferred.rows.clear();
        self.deferred.all = false;
    }

    /// Drops both copies after a change they cannot follow; the next census copies the city
    /// at once and the next hour starts a new fill.
    pub(crate) fn invalidate(&mut self) {
        self.pending = false;
        self.reading = false;
        self.deferred = RowChanges::default();
        self.back.next_chunk = None;
        self.expected_remap = None;
    }

    // Whether a fill or a finished copy must follow the live stores.
    fn active(&self) -> bool {
        self.pending || self.back.next_chunk.is_some()
    }

    /// Mirrors the swap-remove of building `removed` that follows, before it happens: `live`
    /// still holds it. O(1), plus an O(households) retargeting pass when it is the last
    /// building; otherwise that pass runs at the remap the removal announces.
    pub(crate) fn remove_building(&mut self, removed: usize, live: &BuildingAllocator) {
        if !self.active() || removed >= live.buildings.len() {
            return;
        }
        let last = live.buildings.len() - 1;
        if self.back.next_chunk.is_some() {
            self.back
                .buildings
                .mirror_swap_remove(removed, last, &live.buildings);
        }
        if self.pending {
            let front = &mut self.front;
            let buildings = &mut front.allocator.buildings;
            if removed < buildings.len() {
                if last < buildings.len() {
                    buildings.swap_remove(removed);
                    front.freight_capital.swap_remove(removed);
                } else {
                    // A building placed after the copy finished has no freight yet.
                    buildings[removed].clone_from(&live.buildings[last]);
                    front.freight_capital[removed] = 0.0;
                }
            }
        }
        if removed == last {
            self.retarget_buildings(removed, None);
        } else {
            self.expected_remap = Some((last, removed));
        }
    }

    /// Follows a building remap: the swap-remove [`Self::remove_building`] announced retargets
    /// the copied households' references in O(households), in parallel; any other mapping drops
    /// both copies.
    pub(crate) fn remap_buildings(&mut self, mapping: &HashMap<usize, usize>) {
        let expected = self.expected_remap.take();
        if mapping.is_empty() || !self.active() {
            return;
        }
        match expected {
            Some((from, to)) if mapping.len() == 1 && mapping.get(&from) == Some(&to) => {
                self.retarget_buildings(to, Some(from));
            }
            _ => self.invalidate(),
        }
    }

    // References to `removed` become none and references to `moved` become `removed`, as the
    // live remap does. O(copied households), parallel.
    fn retarget_buildings(&mut self, removed: usize, moved: Option<usize>) {
        let retarget = |id: &mut usize| {
            if *id == removed {
                *id = usize::MAX;
            } else if Some(*id) == moved {
                *id = removed;
            }
        };
        let back: &mut [Household] = if self.back.next_chunk.is_some() {
            self.back.households.rows_mut()
        } else {
            &mut []
        };
        let front: &mut [Household] = if self.pending {
            &mut self.front.households
        } else {
            &mut []
        };
        // Main-thread edits that remove buildings reach here outside any pool (CODE-17).
        crate::thread_pools::install(|| {
            back.par_iter_mut()
                .chain(front.par_iter_mut())
                .for_each(|household| {
                    retarget(&mut household.home_building_id);
                    retarget(&mut household.reserved_store_building_id);
                });
        });
    }

    /// Mirrors the swap-remove of household `removed` that follows, before it happens: `live`
    /// still holds it. O(1).
    pub(crate) fn remove_household(&mut self, removed: usize, live: &[Household]) {
        if removed >= live.len() {
            return;
        }
        let last = live.len() - 1;
        if self.back.next_chunk.is_some() {
            self.back.households.mirror_swap_remove(removed, last, live);
        }
        if self.pending && removed < self.front.households.len() {
            let households = &mut self.front.households;
            if last < households.len() {
                households.swap_remove(removed);
            } else {
                households[removed] = live[last].clone();
            }
        }
    }

    // Copies the building rows the allocator names as changed in place again, into the fill and
    // the finished copy, and empties the allocator's list. O(changed rows), or O(copied rows)
    // after a change that named none; a store shorter than a copy drops the copies.
    fn copy_changed_rows(&mut self, live: &mut BuildingAllocator) {
        let mut changes = std::mem::take(&mut live.census_copy_changes);
        let filling = self.back.next_chunk.is_some();
        let live_len = live.buildings.len();
        if (filling && self.back.buildings.len > live_len)
            || (self.pending && self.front.allocator.buildings.len() > live_len)
        {
            self.invalidate();
        }
        // A census reading the front keeps its rows; a restart copies them.
        let front = self.pending && !self.reading;
        if self.pending && self.reading {
            defer_changes(&mut self.deferred, &changes, live_len);
        }
        if changes.all {
            if self.back.next_chunk.is_some() {
                self.back.buildings.copy_all_again(&live.buildings);
            }
            if front {
                self.front.allocator.buildings[..]
                    .par_iter_mut()
                    .zip(&live.buildings[..])
                    .for_each(|(row, live)| row.clone_from(live));
            }
        } else if self.active() {
            for &row in &changes.rows {
                if row >= live_len {
                    continue;
                }
                if self.back.next_chunk.is_some() && row < self.back.buildings.len {
                    self.back.buildings.items[row].clone_from(&live.buildings[row]);
                }
                if front && row < self.front.allocator.buildings.len() {
                    self.front.allocator.buildings[row].clone_from(&live.buildings[row]);
                }
            }
        }
        changes.rows.clear();
        changes.all = false;
        live.census_copy_changes = changes;
    }

    // Copies the front rows deferred while a census read it, then any rows changed since.
    // O(deferred rows), or O(front rows) after a change that named none.
    fn copy_deferred_rows(&mut self, live: &mut BuildingAllocator) {
        self.copy_changed_rows(live);
        let deferred = std::mem::take(&mut self.deferred);
        let buildings = &mut self.front.allocator.buildings;
        if deferred.all {
            buildings
                .par_iter_mut()
                .zip(&live.buildings[..])
                .for_each(|(row, live)| row.clone_from(live));
        } else {
            for &row in &deferred.rows {
                if let (Some(copy), Some(live)) = (buildings.get_mut(row), live.buildings.get(row))
                {
                    copy.clone_from(live);
                }
            }
        }
        // Keep the list's storage.
        self.deferred = RowChanges {
            rows: deferred.rows,
            all: false,
        };
        self.deferred.rows.clear();
    }

    /// Readies the front for a census that starts now and reads it until [`Self::consume`]:
    /// the finished copy with the rows changed or created since (and, when a census restarts,
    /// the rows deferred while it read the front), or, without one, a copy of the city taken
    /// now with `fallback`'s parameters. Then binds it ([`Self::bind`]). O(changed, deferred
    /// and created rows) plus the binding; a copy taken now adds O(B + H + S).
    pub(crate) fn prepare(
        &mut self,
        live: &mut BuildingAllocator,
        households: &[Household],
        logistics: &ShipmentSystem,
        catalog: &RuntimeEconomyCatalog,
        fallback: FrozenCensusParameters,
    ) {
        self.published_front = false;
        if self.reading {
            self.copy_deferred_rows(live);
            self.reading = false;
        }
        self.copy_changed_rows(live);
        if self.pending {
            self.complete_front(live, households);
        }
        if !self.pending {
            self.copy_now(live, households, logistics, fallback);
        }
        self.bind(live, catalog, true);
        self.reading = true;
    }

    /// Binds the loaded front a census was reading when the game was saved, as that census
    /// started on it: no row changed or created since joins it, and its entrances are derived
    /// from the copied rows, as the census took them from the live rows then. O(B) entrance
    /// derivations plus the binding.
    pub(crate) fn bind_loaded(
        &mut self,
        live: &BuildingAllocator,
        graph: &RegionGraph,
        lanes: &LaneSystem,
        catalog: &RuntimeEconomyCatalog,
    ) {
        let copy = &mut self.front.allocator;
        BuildingAllocator::derive_entrances_of(
            &copy.buildings,
            graph,
            lanes,
            &live.registry,
            &mut copy.entrances,
        );
        self.entrances_taken = None;
        self.bind(live, catalog, false);
        self.reading = true;
    }

    // Takes the entrances (when `take_entrances` and their revision changed) and the registry
    // from the live allocator with its building and entrance revisions, steps the copy's
    // production revision and derives a finished copy's service funding. O(B) for the funding
    // and copied entrances.
    fn bind(
        &mut self,
        live: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        take_entrances: bool,
    ) {
        let copy = &mut self.front.allocator;
        if take_entrances
            && (self.entrances_taken != Some(live.entrance_ref_revision)
                || copy.entrances.len() != live.entrances.len())
        {
            copy.entrances.clone_from(&live.entrances);
            self.entrances_taken = Some(live.entrance_ref_revision);
        }
        if self.registry_taken != Some(live.registry.revision()) {
            copy.registry.clone_from(&live.registry);
            self.registry_taken = Some(live.registry.revision());
        }
        copy.building_ref_revision = live.building_ref_revision;
        copy.entrance_ref_revision = live.entrance_ref_revision;
        // Owners whose production inputs changed between the last census's copy and this one
        // changed live since that copy's floor; unknown history makes the market rebuild.
        let since = self.production_revision;
        let owners = self
            .census_floor
            .filter(|&floor| live.production_changes_since(floor, &mut self.changed_owners))
            .map(|_| &self.changed_owners[..]);
        self.production_revision += 1;
        copy.set_copied_production_step(since, self.production_revision, owners);
        let front = &mut self.front;
        write_electricity_funding_by_building(
            &front.allocator.buildings,
            catalog,
            front.parameters.electricity_funding,
            &mut front.service_funding,
        );
    }

    /// Building and household rows of the front a census reads.
    pub(crate) fn front_len(&self) -> (usize, usize) {
        (
            self.front.allocator.buildings.len(),
            self.front.households.len(),
        )
    }

    // Copies the entities created since the copy finished, or copies the city now when the
    // copy no longer fits the live stores.
    fn complete_front(&mut self, live: &BuildingAllocator, households: &[Household]) {
        let front = &mut self.front;
        if front.allocator.buildings.len() > live.buildings.len()
            || front.households.len() > households.len()
        {
            self.pending = false;
            return;
        }
        let copied = front.allocator.buildings.len();
        front
            .allocator
            .buildings
            .extend_from_slice(&live.buildings[copied..]);
        // Freight is planned at the census's hour; a building placed since has none.
        front.freight_capital.resize(live.buildings.len(), 0.0);
        let copied = front.households.len();
        front.households.extend_from_slice(&households[copied..]);
    }

    // The whole city now, with `parameters`. O(B + H + S), parallel over buildings.
    fn copy_now(
        &mut self,
        live: &BuildingAllocator,
        households: &[Household],
        logistics: &ShipmentSystem,
        parameters: FrozenCensusParameters,
    ) {
        let front = &mut self.front;
        let buildings = &mut front.allocator.buildings;
        buildings.truncate(live.buildings.len());
        let kept = buildings.len();
        buildings
            .par_iter_mut()
            .zip(&live.buildings[..kept])
            .for_each(|(row, live)| row.clone_from(live));
        buildings.extend_from_slice(&live.buildings[kept..]);
        front.households.clear();
        front.households.extend_from_slice(households);
        front.freight_capital.clear();
        front.freight_capital.resize(live.buildings.len(), 0.0);
        logistics.write_freight_working_capital(&mut front.freight_capital);
        front.parameters = parameters;
        front.production_floor = Some(live.production_revision());
        self.pending = true;
    }

    /// Writes a changed fiscal policy and city electricity funding into the finished copy's
    /// frozen parameters (`SIM-02-I`): a census restarting on it derives its service funding
    /// from them and the copied rows' overrides. A fill in progress freezes the live values at
    /// its last slot. O(1).
    pub(crate) fn follow_policy(&mut self, fiscal: CityFiscalPolicy, electricity_funding: f32) {
        if !self.pending {
            return;
        }
        let front = &mut self.front;
        front.parameters.fiscal = fiscal;
        front.parameters.electricity_funding = electricity_funding;
    }

    /// The census that read the front was published (or ran at once): the front is done.
    pub(crate) fn consume(&mut self) {
        self.pending = false;
        self.reading = false;
        self.deferred.rows.clear();
        self.deferred.all = false;
        self.census_floor = self.front.production_floor;
    }

    /// Keeps the front the census just published read (`SIM-02-I`): the hour's planning slots
    /// read that census, and a save before the hour's actions slot stores the copy so that a
    /// load recomputes the census ([`Self::save_published`]). O(1).
    pub(crate) fn keep_published_front(&mut self) {
        self.published_front = true;
    }

    /// Whether a census reads the front.
    pub(crate) fn is_read(&self) -> bool {
        self.reading
    }

    /// The published census's saved form (`SIM-02-I`): whether its books are `valid` for the
    /// live city and, for valid books, the copy the census read with `adopted`, the service
    /// ceilings it adopted for buildings appended after the copy. `None` for valid books
    /// without a kept copy, which no game path leaves since `SIM-02-K` part 1: the load then
    /// counts the loaded city. O(copied rows).
    pub(crate) fn save_published(
        &self,
        valid: bool,
        adopted: &[u32],
        edge: impl Fn(usize) -> u64,
    ) -> Option<Vec<u8>> {
        let mut out = Writer(Vec::new());
        out.bool(valid);
        if valid {
            // The slot clock keeps the front from the publication at the hour's first slot
            // until the fill's last slot, and every change to it or to the books' validity in
            // between clears the books or keeps the front. A load there restores the kept front
            // or books that are not valid, and a world's first slot never runs, so a new world
            // has no valid books before its first publication.
            debug_assert!(
                self.published_front && !self.pending,
                "valid books in the hour's planning slots must keep the copy their census read"
            );
            if !self.published_front || self.pending {
                return None;
            }
            self.put_front(&mut out, &edge);
            out.len(adopted.len());
            for &ceiling in adopted {
                out.u32(ceiling);
            }
        }
        Some(out.0)
    }

    /// Takes the published census a load restored ([`Self::save_published`]): `None` without
    /// one, `Some(None)` for books that were not valid at the save, `Some(Some(adopted))` for a
    /// census to recompute from the restored front, with the service ceilings it adopted.
    pub(crate) fn take_loaded_published(&mut self) -> Option<Option<Vec<u32>>> {
        self.loaded_published.take()
    }

    // The front's rows, freight capital and frozen parameters, as saved.
    fn put_front(&self, out: &mut Writer, edge: &impl Fn(usize) -> u64) {
        let front = &self.front;
        put_buildings(out, &front.allocator.buildings, edge);
        put_households(out, &front.households);
        for &capital in &front.freight_capital {
            out.f64(capital);
        }
        let parameters = &front.parameters;
        out.f64(parameters.treasury);
        encoding::put_fiscal(out, &parameters.fiscal);
        out.f32(parameters.electricity_funding);
        out.u16(parameters.minute_of_day);
    }

    // Reads a front [`Self::put_front`] wrote.
    fn read_front(&mut self, reader: &mut Reader<'_>) -> Result<(), String> {
        let front = &mut self.front;
        front.allocator.buildings = read_buildings(reader)?;
        front.households = read_households(reader)?;
        front.freight_capital = (0..front.allocator.buildings.len())
            .map(|_| reader.f64())
            .collect::<Result<_, _>>()?;
        front.parameters = FrozenCensusParameters {
            treasury: reader.f64()?,
            fiscal: encoding::read_fiscal(reader)?,
            electricity_funding: reader.f32()?,
            minute_of_day: reader.u16()?,
        };
        Ok(())
    }

    /// The saved form; `changes` is the allocator's pending row list and `edge` maps a live
    /// road edge to its saved id ([`u64::MAX`] when the save drops it). O(copied rows).
    pub(crate) fn save(
        &self,
        changes: &RowChanges,
        edge: impl Fn(usize) -> u64,
    ) -> SavedCensusInputs {
        let front = self.pending.then(|| {
            let mut out = Writer(Vec::new());
            self.put_front(&mut out, &edge);
            out.0
        });
        let back = self.back.next_chunk.map(|next_chunk| {
            let back = &self.back;
            let mut out = Writer(Vec::new());
            out.u64(next_chunk);
            for total in back.totals {
                out.u64(total as u64);
            }
            put_buildings(&mut out, back.buildings.rows(), &edge);
            put_households(&mut out, back.households.rows());
            out.0
        });
        SavedCensusInputs {
            front,
            back,
            changes: put_row_changes(Writer(Vec::new()), changes),
            deferred: {
                let mut out = Writer(Vec::new());
                out.bool(self.reading);
                put_row_changes(out, &self.deferred)
            },
            published: None,
        }
    }

    /// Rebuilds a saved copy, rejecting malformed buffers. The production floors are not
    /// saved: the first census after a load rebuilds its production market.
    pub(crate) fn restore(saved: &SavedCensusInputs) -> Result<Self, String> {
        let mut inputs = Self::default();
        if let Some(bytes) = &saved.front {
            let mut reader = Reader(bytes);
            inputs.read_front(&mut reader)?;
            reader.finish()?;
            inputs.pending = true;
        }
        if let Some(bytes) = &saved.published {
            let mut reader = Reader(bytes);
            inputs.loaded_published = Some(if reader.bool()? {
                if inputs.pending {
                    return Err("census input copy saved both pending and published".into());
                }
                inputs.read_front(&mut reader)?;
                inputs.published_front = true;
                let adopted = (0..reader.len(4)?)
                    .map(|_| reader.u32())
                    .collect::<Result<_, _>>()?;
                Some(adopted)
            } else {
                None
            });
            reader.finish()?;
        }
        if let Some(bytes) = &saved.back {
            let mut reader = Reader(bytes);
            let next_chunk = reader.u64()?;
            if !(1..SLOTS_PER_HOUR).contains(&next_chunk) {
                return Err("census input fill chunk outside the hour".into());
            }
            let back = &mut inputs.back;
            back.next_chunk = Some(next_chunk);
            for total in &mut back.totals {
                *total = usize::try_from(reader.u64()?)
                    .map_err(|_| "census input fill total overflow".to_string())?;
            }
            back.buildings.items = read_buildings(&mut reader)?;
            back.buildings.len = back.buildings.items.len();
            back.households.items = read_households(&mut reader)?;
            back.households.len = back.households.items.len();
            reader.finish()?;
        }
        let mut reader = Reader(&saved.changes);
        inputs.loaded_changes = Some(read_row_changes(&mut reader)?);
        reader.finish()?;
        let mut reader = Reader(&saved.deferred);
        inputs.reading = reader.bool()?;
        inputs.deferred = read_row_changes(&mut reader)?;
        reader.finish()?;
        if inputs.reading && !inputs.pending {
            return Err("census input copy is read without a front".into());
        }
        Ok(inputs)
    }

    /// Gives the loaded allocator the row changes saved with the copy, replacing those its own
    /// reconstruction recorded; call once loading has finished.
    pub(crate) fn finish_load(&mut self, allocator: &mut BuildingAllocator) {
        if let Some(changes) = self.loaded_changes.take() {
            allocator.census_copy_changes = changes;
        }
    }
}

#[cfg(test)]
impl CensusInputs {
    /// Whether a finished copy waits for the census.
    pub(crate) fn has_pending_front(&self) -> bool {
        self.pending
    }

    /// Whether a fill is in progress.
    pub(crate) fn is_filling(&self) -> bool {
        self.back.next_chunk.is_some()
    }

    /// Checks that every copied row is the live entity at its index: the same building, the
    /// same household (told apart by `replenishment_search_cursor`) with the live references.
    pub(crate) fn assert_aligned(&self, live: &BuildingAllocator, households: &[Household]) {
        let mut copies = Vec::new();
        if self.is_filling() {
            copies.push((self.back.buildings.rows(), self.back.households.rows()));
        }
        if self.pending {
            copies.push((
                &self.front.allocator.buildings[..],
                &self.front.households[..],
            ));
        }
        for (buildings, copied_households) in copies {
            assert!(buildings.len() <= live.buildings.len());
            assert!(copied_households.len() <= households.len());
            for (index, (row, building)) in buildings.iter().zip(&live.buildings).enumerate() {
                assert_eq!(
                    (row.parcel_id, row.build_generation, &row.asset_id),
                    (
                        building.parcel_id,
                        building.build_generation,
                        &building.asset_id
                    ),
                    "building row {index}"
                );
                assert_eq!(row.center_x.to_bits(), building.center_x.to_bits());
            }
            for (index, (row, household)) in copied_households.iter().zip(households).enumerate() {
                assert_eq!(
                    (
                        row.replenishment_search_cursor,
                        row.home_building_id,
                        row.reserved_store_building_id
                    ),
                    (
                        household.replenishment_search_cursor,
                        household.home_building_id,
                        household.reserved_store_building_id
                    ),
                    "household row {index}"
                );
            }
        }
    }

    /// The finished copy's frozen parameters.
    pub(crate) fn front_parameters_for_test(&self) -> FrozenCensusParameters {
        self.front.parameters
    }

    /// Whether the finished copy is the live stores bit for bit.
    pub(crate) fn front_equals(&self, live: &BuildingAllocator, households: &[Household]) -> bool {
        let rows = |buildings: &[Building], households: &[Household]| {
            let mut out = Writer(Vec::new());
            put_buildings(&mut out, buildings, &|edge| edge as u64);
            put_households(&mut out, households);
            out.0
        };
        self.pending
            && rows(&self.front.allocator.buildings, &self.front.households)
                == rows(&live.buildings, households)
    }
}

fn put_row_changes(mut out: Writer, changes: &RowChanges) -> Vec<u8> {
    out.bool(changes.all);
    out.len(changes.rows.len());
    for &row in &changes.rows {
        out.u64(row as u64);
    }
    out.0
}

fn read_row_changes(reader: &mut Reader<'_>) -> Result<RowChanges, String> {
    let all = reader.bool()?;
    let rows = (0..reader.len(8)?)
        .map(|_| reader.index())
        .collect::<Result<_, _>>()?;
    Ok(RowChanges { rows, all })
}

fn put_buildings(out: &mut Writer, buildings: &[Building], edge: &impl Fn(usize) -> u64) {
    out.len(buildings.len());
    for building in buildings {
        encoding::put_building(out, building, edge);
    }
}

fn put_households(out: &mut Writer, households: &[Household]) {
    out.len(households.len());
    for household in households {
        encoding::put_household(out, household);
    }
}

fn read_buildings(reader: &mut Reader<'_>) -> Result<Vec<Building>, String> {
    (0..reader.len(encoding::MIN_BUILDING_BYTES)?)
        .map(|_| encoding::read_building(reader))
        .collect()
}

fn read_households(reader: &mut Reader<'_>) -> Result<Vec<Household>, String> {
    (0..reader.len(encoding::MIN_HOUSEHOLD_BYTES)?)
        .map(|_| encoding::read_household(reader))
        .collect()
}

impl JobFunding {
    /// Runs the census on the front [`CensusInputs::prepare`] readied, with its parameters and
    /// its own freight route cache, then marks the front done, in one call (tests). Its outputs
    /// are indexed by the front's rows, which are the live dense order.
    #[cfg(test)]
    pub(crate) fn refresh_from_inputs(
        &mut self,
        inputs: &mut CensusInputs,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) {
        self.step_from_inputs(
            inputs,
            &mut CensusProgress::default(),
            network,
            graph,
            catalog,
            tuning,
            u64::MAX,
        );
        inputs.consume();
    }

    /// Runs census steps on the front [`CensusInputs::prepare`] readied until `budget` work
    /// units are used (`docs/simulation.md#pure-jobs`, `hiring/steps.rs`); returns whether the
    /// census is done.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn step_from_inputs(
        &mut self,
        inputs: &mut CensusInputs,
        progress: &mut CensusProgress,
        network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        budget: u64,
    ) -> bool {
        let front = &inputs.front;
        let context = CensusContext {
            allocator: &front.allocator,
            households: &front.households,
            network,
            graph,
            catalog,
            tuning,
            fiscal: &front.parameters.fiscal,
            treasury_balance: front.parameters.treasury,
            minute_of_day: front.parameters.minute_of_day,
            service_capacity: |index, physical, profile: &EconomyProfileRuntime| {
                service_funded_worker_capacity(physical, profile, index, &front.service_funding)
            },
            previous: None,
            freight_capital: Some(&front.freight_capital),
        };
        self.run_census_steps(&context, &mut inputs.routes, progress, budget)
    }
}
