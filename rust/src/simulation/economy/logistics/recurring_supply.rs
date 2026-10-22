// SPDX-License-Identifier: GPL-2.0-only

//! Finite production-rate quotes, separate from dispatch's physical-stock reservations.

use super::route_cache::FreightRouteCache;
use super::supplier_index::SupplierCandidateIndex;
use super::timing::freight_cost;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::accessibility::{
    BoundedScanEvent, ModeComponentIndex, ReachableBucketEntry, ReachableBucketIndex,
    chunk_for_point, lower_bound_travel_seconds, max_speed_for_modes,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, ResourceRuntimeId, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use crate::simulation::economy::households::{
    output_area_scale_for_building, physical_worker_capacity_for_profile,
};
use crate::simulation::economy::operation::{InputRequirement, InputSupplyCosts};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph, types::TransitFlags};
use rayon::prelude::*;

mod projected;
mod revalidation;
pub(crate) use projected::ProjectedSupplySource;

type LayoutRevision = (u64, u64, u32, u64, usize);
type GeometryRevision = (u64, u64, u32, usize);

// Route prices change without changing resource membership, doors or weak components.
fn geometry_revision(revision: Option<LayoutRevision>) -> Option<GeometryRevision> {
    revision
        .map(|(buildings, entrances, topology, _, count)| (buildings, entrances, topology, count))
}

#[derive(Clone, Copy, Debug)]
struct Claim {
    source: usize,
    resource: ResourceRuntimeId,
    slot: usize,
    previous: f64,
    units: f64,
    owner: usize,
    next_from_owner: usize,
    next_from_source: usize,
    previous_from_source: usize,
    unit_price: f32,
    carrier_quote: f32,
}

#[derive(Clone, Copy, Debug)]
struct Offer {
    source: usize,
    slot: usize,
    seconds: f32,
    next: usize,
}

#[derive(Clone, Copy, Debug)]
struct InputOffers {
    resource: ResourceRuntimeId,
    first: usize,
    last: usize,
    next: usize,
    exhausted: bool,
}

/// Reusable finite input market. Quotes stage claims; only accepted operations commit them.
/// Source quantities are net daily production of the funded recipe activity, capped by current staff.
#[derive(Clone, Debug, Default)]
pub(crate) struct RecurringSupply {
    offsets: Vec<usize>,
    active: Vec<bool>,
    revision: Option<LayoutRevision>,
    remaining: Vec<f64>,
    // Net production after household commitments, before business input reservations.
    production: Vec<f64>,
    projected: projected::ProjectedSources,
    staged: Vec<Claim>,
    // Live receipt links in both directions; retirement reuses slots rather than keeping
    // replacement history. Dependency visits therefore follow only current commitments.
    committed: Vec<Claim>,
    owner_receipts: Vec<usize>,
    owner_tails: Vec<usize>,
    source_receipts: Vec<usize>,
    free_receipt: usize,
    revalidation: revalidation::Revalidation,
    released: Vec<Claim>,
    releasing_owner: Option<usize>,
    receipt_capacity: usize,
    // Starts every census from an empty arena to exercise exact commit growth.
    #[cfg(test)]
    pub(crate) discard_receipt_estimate: bool,
    prices: Vec<(ResourceRuntimeId, f64)>,
    // A staffing search retries one operation against exactly restored quantities.
    // Replay its already-ranked offers instead of repeating spatial/route searches.
    offers: Vec<Offer>,
    input_offers: Vec<InputOffers>,
    nearest_incumbents: Vec<Option<usize>>,
    #[cfg(test)]
    search_counts: [u64; 6],
    // Nearest searches, candidates visited, routes priced and input quotes so far, which the
    // funding census counts as work (`SIM-02-K`); monotonic, never read by any result.
    work: [u64; 4],
    // Source handles below this are live buildings, the census's own and any appended since
    // (which hold no supply); prepared variants are numbered from here (`SIM-02-I`).
    source_base: usize,
    // Placed source doors, packed for the straight-line prune of every scanned candidate.
    doors: Vec<Option<godot::prelude::Vector2>>,
    index: SupplierCandidateIndex,
    // A reset's supplier index entries, sorted, and each resource's run of them, and the
    // layout the index is built for (`SIM-02-K`); empty runs when the reset keeps the index.
    index_entries: Vec<super::supplier_index::SupplierEntry>,
    index_runs: Vec<(ResourceRuntimeId, usize, usize)>,
    index_revision: LayoutRevision,
    components: Option<ModeComponentIndex>,
    max_speed: f32,
}

impl RecurringSupply {
    /// Diagnostic totals: nearest searches, candidate visits, route quotes, region/ring
    /// checks, retained-offer hits and staffing retries. Production carries no counters.
    #[cfg(test)]
    pub(crate) fn search_counts_for_test(&self) -> [u64; 6] {
        self.search_counts
    }

    /// Nearest searches, candidates visited, routes priced and input quotes made so far, for
    /// the funding census's work units (`SIM-02-K`). Monotonic counters; O(1).
    pub(crate) fn work_counts(&self) -> [u64; 4] {
        self.work
    }

    /// Prepares temporary project owners outside input quotation loops.
    pub(crate) fn prepare_owners(&mut self, owners: usize, input_ports: usize) {
        // Owner rows follow the city, with spare capacity (`SIM-02-K`).
        crate::utils::storage::grow_rows(&mut self.owner_receipts, owners, usize::MAX);
        crate::utils::storage::grow_rows(&mut self.owner_tails, owners, usize::MAX);
        let receipts = self.committed.len() + 2 * (self.remaining.len() + input_ports);
        // With spare capacity, which also sizes the receipt journals (`SIM-02-K`).
        self.reserve_receipts(crate::utils::storage::spare_capacity(
            receipts,
            self.committed.capacity(),
        ));
        self.revalidation.prepare(
            self.remaining.len(),
            self.owner_receipts.len(),
            self.committed.capacity(),
        );
    }

    /// Grows receipt and owner-withdrawal storage only between complete proposal attempts.
    pub(crate) fn reserve_receipts(&mut self, required: usize) {
        debug_assert!(!self.revalidation.is_active());
        let receipts = required.saturating_sub(self.committed.len());
        crate::utils::storage::reserve_cleared(&mut self.committed, receipts);
        let released = self
            .committed
            .capacity()
            .saturating_sub(self.released.len());
        crate::utils::storage::reserve_cleared(&mut self.released, released);
        self.revalidation.follow_receipts(self.committed.capacity());
    }

    /// Checks a pending full quote, counting the owner's retired cells before requesting growth.
    /// O(staged claims + reusable cells), without mutation or unrelated-owner scans.
    pub(crate) fn check_accept_capacity(&self, owner: usize) -> Result<(), usize> {
        self.check_receipt_capacity(self.staged.len(), Some(owner))
    }

    /// Checks an input-gap extension while preserving the owner's existing supplier receipts.
    pub(crate) fn check_extension_capacity(&self) -> Result<(), usize> {
        self.check_receipt_capacity(self.staged.len(), None)
    }

    fn check_receipt_capacity(&self, added: usize, retired: Option<usize>) -> Result<(), usize> {
        let mut missing = added;
        for first in [
            retired.map_or(usize::MAX, |owner| self.owner_receipts[owner]),
            self.free_receipt,
        ] {
            let mut index = first;
            while index != usize::MAX && missing > 0 {
                missing -= 1;
                index = self.committed[index].next_from_owner;
            }
        }
        let required = self.committed.len() + missing;
        if required > self.committed.capacity() {
            Err(required)
        } else {
            Ok(())
        }
    }

    /// Numbers prepared variants after `buildings` live buildings: a book lent to planning
    /// after buildings were appended to the census's copy covers them as live sources without
    /// supply, so a variant handle never names an appended building (`SIM-02-I`). Call before
    /// [`Self::prepare_projected_sources`]. O(1).
    pub(crate) fn cover_live_buildings(&mut self, buildings: usize) {
        debug_assert!(!self.projected_enabled());
        self.source_base = buildings.max(self.offsets.len().saturating_sub(1));
    }

    /// Invalidates the source view after job-state removal, remapping, or reload.
    pub(crate) fn invalidate(&mut self) {
        self.revision = None;
    }

    /// Whether supplier indices and geometry still refer to this allocator/network layout.
    /// Planning checks the census key instead (`SIM-02-I`).
    #[cfg(test)]
    pub(crate) fn matches_layout(
        &self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
    ) -> bool {
        self.revision
            == Some((
                allocator.building_ref_revision(),
                allocator.entrance_ref_revision(),
                network.cch_graph.build_generation,
                network.cch_graph.metric_revision,
                allocator.buildings.len(),
            ))
    }

    /// Forks accepted live reservations once for a construction pass. Unchanged geometry
    /// retains its index allocation; warm copying costs O(buildings + output ports + accepted
    /// input flows), never a city-sized copy per project.
    #[cfg(test)]
    pub(crate) fn copy_reservations_from(
        &mut self,
        source: &Self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        graph: &RegionGraph,
    ) {
        debug_assert!(source.staged.is_empty());
        debug_assert!(!self.revalidation.is_active() && !source.revalidation.is_active());
        debug_assert!(
            !source.projected_enabled(),
            "construction forks the live supply census"
        );
        if geometry_revision(self.revision) != geometry_revision(source.revision) {
            // Layout changes need fresh membership, not a second financial copy or loss
            // of the larger project arenas prepared by the preceding construction pass.
            self.offsets.clone_from(&source.offsets);
            self.active.clone_from(&source.active);
            self.doors.clone_from(&source.doors);
            self.index.clone_from(&source.index);
            self.components.clone_from(&source.components);
            self.nearest_incumbents
                .clone_from(&source.nearest_incumbents);
            self.receipt_capacity = self.receipt_capacity.max(source.receipt_capacity);
            self.staged.reserve(source.remaining.len());
            self.released.reserve(source.remaining.len());
            self.offers.reserve(source.remaining.len());
            self.prices.reserve(catalog.resource_count());
            self.input_offers.reserve(catalog.resource_count());
            #[cfg(test)]
            {
                self.discard_receipt_estimate = source.discard_receipt_estimate;
                self.search_counts = source.search_counts;
            }
        }
        self.revision = source.revision;
        self.source_base = source.source_base;
        self.max_speed = source.max_speed;
        self.staged.clear();
        self.released.clear();
        self.releasing_owner = None;
        self.committed.clone_from(&source.committed);
        self.committed
            .reserve(self.receipt_capacity.saturating_sub(self.committed.len()));
        self.owner_receipts.clone_from(&source.owner_receipts);
        self.owner_tails.clone_from(&source.owner_tails);
        self.source_receipts.clone_from(&source.source_receipts);
        self.free_receipt = source.free_receipt;
        self.prices.clear();
        self.offers.clear();
        self.input_offers.clear();
        self.remaining.clone_from(&source.remaining);
        self.production.clone_from(&source.production);
        self.clear_projected_sources();
        self.active.truncate(source.active.len());
        for index in 0..source.offsets.len().saturating_sub(1) {
            let Some(profile) = catalog
                .profile_by_runtime_id(allocator.buildings[index].economy_profile_runtime_id)
            else {
                continue;
            };
            for (port, output) in profile.outputs.iter().enumerate() {
                let slot = source.offsets[index] + port;
                if self.active[slot] != source.active[slot] {
                    self.set_available(
                        index,
                        output.resource_runtime_id,
                        source.active[slot],
                        allocator,
                        graph,
                    );
                    self.active[slot] = source.active[slot];
                }
            }
        }
        crate::debug::work::add(
            crate::debug::work::Work::ReservationRowsCopied,
            (self.committed.len()
                + 2 * self.owner_receipts.len()
                + self.source_receipts.len()
                + self.remaining.len()
                + self.production.len()
                + self.active.len()) as u64,
        );
    }

    /// Whether any funded staffed source has a positive remaining production rate.
    /// Test harness: runtime funding asks per recipe through `offers_inputs`.
    #[cfg(test)]
    pub(crate) fn has_offers(&self) -> bool {
        self.index.has_offers() || self.projected_has_offers()
    }

    /// Whether this recipe can claim any remaining local offer, in O(input ports * log resources).
    /// A false result permits reuse of the already computed import-only funding result.
    pub(crate) fn offers_inputs(&self, profile: &EconomyProfileRuntime) -> bool {
        profile.inputs.iter().any(|input| {
            let internal = profile
                .output_port(input.resource_runtime_id)
                .map_or(0.0, |output| output.units_per_day);
            input.units_per_day > internal && self.offers_resource(input.resource_runtime_id)
        })
    }

    fn offers_resource(&self, resource: ResourceRuntimeId) -> bool {
        [
            self.index.buckets_for_resource(resource),
            self.projected_buckets(resource),
        ]
        .into_iter()
        .flatten()
        .any(|buckets| buckets.has_available())
    }

    /// Visits existing source membership for a resource/component, including depleted or
    /// unfunded sources. Work follows that component's entries; no query storage is allocated.
    pub(crate) fn visit_possible_sources(
        &self,
        resource: ResourceRuntimeId,
        component: u32,
        visit: impl FnMut(usize),
    ) {
        if let Some(buckets) = self.index.buckets_for_resource(resource) {
            buckets.visit_component_members(component, visit);
        }
    }

    /// Prepares production rates and the existing resource/component/chunk index once per pass.
    /// Warm work is O(B + outputs + changed availability * log bucket size). Spatial
    /// rebuilding costs O(outputs log outputs + roads) only when building/access topology changes.
    /// Quantities use sparse output-port storage, not B*R.
    /// Physical shipment stock never creates a recurring offer.
    /// The census runs the same phases in slices (`SIM-02-K`); this runs them at once.
    #[cfg(test)]
    pub(crate) fn reset(
        &mut self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        network: &TransitNetwork,
        catalog: &RuntimeEconomyCatalog,
        funded_activity: &[f32],
    ) {
        if self.begin_reset(allocator, graph, network, catalog) {
            self.reset_layout(allocator, catalog, 0..allocator.buildings.len());
            self.reset_entries(allocator, graph, catalog);
        }
        self.build_reset_index(0..self.reset_index_resources());
        self.reset_rates(
            allocator,
            graph,
            catalog,
            funded_activity,
            0..allocator.buildings.len(),
        );
    }

    /// A reset's first phase (`SIM-02-K`): the receipt books' reset and whether the geometry
    /// changed. After a change, [`Self::reset_layout`] over every source range in order,
    /// [`Self::reset_entries`] and [`Self::build_reset_index`] over every range of
    /// [`Self::reset_index_resources`] in order; then [`Self::reset_rates`] over every source
    /// range in order. Any slicing gives one reset's result. Until the index is complete the
    /// book names no layout, so a reset begun again after an interruption rebuilds it. Returns
    /// whether the geometry changed. O(B / workers).
    pub(crate) fn begin_reset(
        &mut self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        network: &TransitNetwork,
        catalog: &RuntimeEconomyCatalog,
    ) -> bool {
        debug_assert!(!self.revalidation.is_active());
        self.clear_projected_sources();
        self.source_base = allocator.buildings.len();
        self.staged.clear();
        self.committed.clear();
        self.released.clear();
        self.releasing_owner = None;
        self.owner_receipts
            .resize(allocator.buildings.len(), usize::MAX);
        self.owner_receipts
            .par_iter_mut()
            .for_each(|head| *head = usize::MAX);
        self.owner_tails
            .resize(allocator.buildings.len(), usize::MAX);
        self.owner_tails
            .par_iter_mut()
            .for_each(|tail| *tail = usize::MAX);
        self.free_receipt = usize::MAX;
        self.prices.clear();
        self.offers.clear();
        self.input_offers.clear();
        self.prices.reserve(catalog.resource_count());
        self.input_offers.reserve(catalog.resource_count());
        let revision = (
            allocator.building_ref_revision(),
            allocator.entrance_ref_revision(),
            network.cch_graph.build_generation,
            network.cch_graph.metric_revision,
            allocator.buildings.len(),
        );
        self.index_runs.clear();
        self.index_revision = revision;
        let rebuilds = geometry_revision(self.revision) != geometry_revision(Some(revision));
        if rebuilds {
            self.nearest_incumbents
                .resize(catalog.resource_count() + 1, None);
            self.nearest_incumbents.fill(None);
            self.offsets.clear();
            self.offsets.reserve(allocator.buildings.len() + 1);
            self.offsets.push(0);
            self.receipt_capacity = 0;
            self.doors.clear();
            self.doors.reserve(allocator.buildings.len());
            // A torn layout or index is never reused: the layout is named once every run of
            // the index is built.
            self.revision = None;
        }
        if self.revision != Some(revision) {
            // A metric update can include changed speed limits, so refresh the exact
            // route-search bound even when resource/component geometry stays reusable.
            self.max_speed = max_speed_for_modes(graph, TransitFlags::CAR)
                .max(crate::config::AGENT_DRIVEWAY_SPEED_MS);
        }
        if !rebuilds {
            self.finish_reset_layout();
        }
        rebuilds
    }

    /// The output-port offsets, receipt estimate and doors of sources `range`, the next ones,
    /// in source order, after a geometry change (`SIM-02-K`). O(range), serial.
    pub(crate) fn reset_layout(
        &mut self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        range: std::ops::Range<usize>,
    ) {
        for (source, building) in allocator.buildings[range.clone()]
            .iter()
            .enumerate()
            .map(|(offset, building)| (range.start + offset, building))
        {
            let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
            self.receipt_capacity += profile.map_or(0, |profile| profile.inputs.len());
            let outputs = profile.map_or(0, |profile| profile.outputs.len());
            self.offsets
                .push(self.offsets.last().copied().unwrap() + outputs);
            self.doors.push(
                allocator
                    .entrances
                    .get(source)
                    .map(|entrance| entrance.door_pos),
            );
        }
    }

    /// After the layout, the quantity rows, the freight components and the supplier index's
    /// entries, sorted into one run per resource (`SIM-02-K`). O(B × outputs / workers + roads
    /// + E log E / workers) for E entries.
    pub(crate) fn reset_entries(
        &mut self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
    ) {
        self.remaining.resize(*self.offsets.last().unwrap(), 0.0);
        self.receipt_capacity += 2 * self.remaining.len();
        let components = ModeComponentIndex::build(graph, TransitFlags::CAR);
        // Inactive/unfinished sources remain geometrically indexed. Hourly quantities
        // enable them only when they have actual staffed, funded net production.
        SupplierCandidateIndex::collect_filtered(
            allocator,
            graph,
            catalog,
            &components,
            false,
            |_, _| true,
            &mut self.index_entries,
        );
        // Sorted entries split into one run per resource; each run builds the index that
        // resource's own sorted entries give (`SIM-02-K`).
        self.index_entries.par_sort_unstable();
        let mut start = 0;
        for run in self.index_entries.chunk_by(|left, right| left.0 == right.0) {
            self.index_runs.push((run[0].0, start, start + run.len()));
            start += run.len();
        }
        self.index.clear();
        self.active.resize(self.remaining.len(), true);
        self.active.fill(true);
        self.components = Some(components);
        self.finish_reset_layout();
    }

    // The reset's layout end: the layout's name without an index to build, and the quantity
    // rows' lengths. O(1) amortized.
    fn finish_reset_layout(&mut self) {
        if self.index_runs.is_empty() {
            self.revision = Some(self.index_revision);
        }
        self.remaining.resize(*self.offsets.last().unwrap(), 0.0);
        self.active.truncate(self.remaining.len());
        self.production.resize(self.remaining.len(), 0.0);
    }

    /// Resources whose supplier index the reset builds, zero without a geometry change.
    pub(crate) fn reset_index_resources(&self) -> usize {
        self.index_runs.len()
    }

    /// Builds the supplier index of the reset's resources `range`, in resource order, with
    /// reservations and door bounds; returns the entries built (`SIM-02-K`). The last range
    /// names the layout. O(entries log entries).
    pub(crate) fn build_reset_index(&mut self, range: std::ops::Range<usize>) -> usize {
        let last = range.end == self.index_runs.len();
        let mut built = 0;
        let doors = &self.doors;
        for &(resource, start, end) in &self.index_runs[range] {
            let mut buckets = ReachableBucketIndex::from_entries(
                self.index_entries[start..end]
                    .iter()
                    .map(|&(_, entry)| entry)
                    .collect(),
            );
            buckets.enable_reservations();
            buckets.enable_spatial_bounds(|source| {
                doors
                    .get(source)
                    .copied()
                    .flatten()
                    .map(|door| [door.x, door.y])
            });
            self.index.insert(resource, buckets);
            built += end - start;
        }
        if last {
            self.revision = Some(self.index_revision);
        }
        built
    }

    /// Production rates and supplier availability of sources `range`, in source order, after
    /// the index (`SIM-02-K`); the last range prepares the receipt storage. O(range + changed
    /// availability × log bucket size), the rates in parallel.
    pub(crate) fn reset_rates(
        &mut self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        funded_activity: &[f32],
        range: std::ops::Range<usize>,
    ) {
        let slots = self.offsets[range.start]..self.offsets[range.end];
        fill_production_rates(
            allocator,
            catalog,
            funded_activity,
            &self.offsets,
            range.start,
            range.end,
            &mut self.remaining[slots.clone()],
        );
        self.production[slots.clone()].copy_from_slice(&self.remaining[slots]);
        for source in range.clone() {
            let Some(profile) = catalog
                .profile_by_runtime_id(allocator.buildings[source].economy_profile_runtime_id)
            else {
                continue;
            };
            for (port, output) in profile.outputs.iter().enumerate() {
                let slot = self.offsets[source] + port;
                let available = self.remaining[slot] > 0.0;
                if available != self.active[slot] {
                    self.set_available(
                        source,
                        output.resource_runtime_id,
                        available,
                        allocator,
                        graph,
                    );
                    self.active[slot] = available;
                }
            }
        }
        if range.end < allocator.buildings.len() {
            return;
        }
        // At most one outstanding claim per eligible output port in an operation quote.
        self.staged.reserve(self.remaining.len());
        self.offers.reserve(self.remaining.len());
        // Warm-start estimate only; census commits preflight and grow exactly.
        let estimate = self.receipt_capacity;
        #[cfg(test)]
        let estimate = if self.discard_receipt_estimate {
            self.committed.shrink_to_fit();
            0
        } else {
            estimate
        };
        crate::utils::storage::reserve_cleared(&mut self.committed, estimate);
        crate::utils::storage::reserve_cleared(&mut self.released, self.remaining.len());
        self.source_receipts
            .resize(self.remaining.len(), usize::MAX);
        self.source_receipts.fill(usize::MAX);
    }

    /// Replaces a source's provisional production rate with its evaluated staffed activity.
    /// A decrease may expose overcommitted inputs; `has_overclaims` prevents publication until
    /// those buyers are requoted. O(output ports + changed eligibility * log bucket size).
    pub(crate) fn publish_funded_activity(
        &mut self,
        source: usize,
        before: f32,
        after: f32,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        graph: &RegionGraph,
    ) {
        debug_assert!(self.staged.is_empty() && self.released.is_empty());
        if after == before {
            return;
        }
        let building = &allocator.buildings[source];
        let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
        else {
            return;
        };
        let previous = production_activity(building, profile, before);
        let current = production_activity(building, profile, after);
        if previous == current {
            return;
        }
        for (port, output) in profile.outputs.iter().enumerate() {
            let rate = profile.net_output_units_per_day(output)
                * output_area_scale_for_building(building, profile);
            let added = f64::from(rate * current) - f64::from(rate * previous);
            let slot = self.offsets[source] + port;
            self.journal_source(source, output.resource_runtime_id, slot);
            self.remaining[slot] += added;
            self.production[slot] += added;
            let available = self.remaining[slot] > 0.0;
            if self.active[slot] != available {
                self.active[slot] = available;
                self.set_available(
                    source,
                    output.resource_runtime_id,
                    available,
                    allocator,
                    graph,
                );
            }
        }
    }

    /// Whether any provisional owner inputs exceed the finally financed staffed source rate.
    pub(crate) fn has_overclaims(&self) -> bool {
        self.remaining.par_iter().any(|remaining| *remaining < 0.0)
    }

    /// Withdraws only excess deliveries, newest first, retaining earlier buyers and prices.
    /// An outer journal restores exact quantities and both receipt chains on rejection.
    /// Caller prepares one affected-owner slot/flag per owner and clears touched flags after
    /// use. O(output slots + withdrawn flows + affected owners' flows), without allocation.
    pub(crate) fn withdraw_overclaims(
        &mut self,
        owners: &mut Vec<usize>,
        seen: &mut [bool],
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        debug_assert!(self.revalidation.is_active());
        debug_assert!(owners.is_empty() && self.staged.is_empty() && self.released.is_empty());
        for slot in 0..self.remaining.len() {
            let mut index = self.source_receipts[slot];
            while self.remaining[slot] < 0.0 && index != usize::MAX {
                let receipt = self.committed[index];
                self.journal_source(receipt.source, receipt.resource, slot);
                self.journal_receipt(index);
                let excess = -self.remaining[slot];
                if receipt.units > excess {
                    self.committed[index].units -= excess;
                    self.remaining[slot] = 0.0;
                } else {
                    // Keep owner links until the second pass, so each affected chain is
                    // traversed once even when several fragmented suppliers disappear.
                    self.committed[index].units = 0.0;
                    self.remaining[slot] += receipt.units;
                }
                if !seen[receipt.owner] {
                    seen[receipt.owner] = true;
                    assert!(
                        owners.len() < owners.capacity(),
                        "input repairs require owner preparation"
                    );
                    owners.push(receipt.owner);
                }
                index = receipt.next_from_source;
            }
        }
        for &owner in owners.iter() {
            let mut index = self.owner_receipts[owner];
            let mut previous = usize::MAX;
            while index != usize::MAX {
                let receipt = self.committed[index];
                if receipt.units == 0.0 {
                    self.journal_owner(owner);
                    if previous == usize::MAX {
                        self.owner_receipts[owner] = receipt.next_from_owner;
                    } else {
                        self.journal_receipt(previous);
                        self.committed[previous].next_from_owner = receipt.next_from_owner;
                    }
                    if self.owner_tails[owner] == index {
                        self.owner_tails[owner] = previous;
                    }
                    self.retire_receipt(index, receipt);
                    if self.source_receipts[receipt.slot] == usize::MAX {
                        self.remaining[receipt.slot] = self.production[receipt.slot];
                    }
                    let available = self.remaining[receipt.slot] > 0.0;
                    if self.active[receipt.slot] != available {
                        self.active[receipt.slot] = available;
                        self.set_available(
                            receipt.source,
                            receipt.resource,
                            available,
                            allocator,
                            graph,
                        );
                    }
                } else {
                    previous = index;
                }
                index = receipt.next_from_owner;
            }
        }
    }

    /// Replaces a household claim on staffed output in the shared supply ledger.
    /// Negative availability exposes affected business inputs for revalidation; changes cost
    /// O(source output ports + changed eligibility * log bucket size), with no allocation.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reserve_household_output(
        &mut self,
        source: usize,
        resource: ResourceRuntimeId,
        before: f64,
        after: f64,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        graph: &RegionGraph,
    ) {
        debug_assert!(self.staged.is_empty() && self.released.is_empty());
        if before == after {
            return;
        }
        let Some(slot) = self.slot(allocator, catalog, source, resource) else {
            return;
        };
        self.journal_source(source, resource, slot);
        self.remaining[slot] += before - after;
        self.production[slot] += before - after;
        let available = self.remaining[slot] > 0.0;
        if self.active[slot] != available {
            self.active[slot] = available;
            self.set_available(source, resource, available, allocator, graph);
        }
    }

    fn slot(
        &self,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        source: usize,
        resource: ResourceRuntimeId,
    ) -> Option<usize> {
        let profile = self.source_profile(source, allocator, catalog)?;
        let port = profile
            .outputs
            .iter()
            .position(|port| port.resource_runtime_id == resource)?;
        Some(self.source_range(source)?.start + port)
    }

    fn set_available(
        &mut self,
        source: usize,
        resource: ResourceRuntimeId,
        available: bool,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        if source >= self.source_base {
            self.set_projected_available(source, resource, available, graph);
            return;
        }
        let Some(components) = &self.components else {
            return;
        };
        let building = &allocator.buildings[source];
        let chunk = chunk_for_point(building.center_x, building.center_y);
        for &component in components
            .building_components(allocator, graph, source, TransitFlags::CAR)
            .as_slice()
        {
            self.index.set_available(
                resource,
                ReachableBucketEntry::new(component, chunk, source),
                available,
            );
        }
    }

    /// Rolls back a rejected operation in O(claims * log suppliers), retaining scratch capacity.
    pub(crate) fn reject(&mut self, allocator: &BuildingAllocator, graph: &RegionGraph) {
        self.retry(allocator, graph);
        while let Some(claim) = self.released.pop() {
            self.restore(claim, allocator, graph);
        }
        self.releasing_owner = None;
        self.offers.clear();
        self.input_offers.clear();
    }

    /// Restores the same operation's claims while retaining its exact supplier ordering.
    /// No other operation may mutate this book between retries. Each cached list advances
    /// in O(1) per offer; clearing a retry costs O(claims * log bucket size + input ports).
    pub(crate) fn retry(&mut self, allocator: &BuildingAllocator, graph: &RegionGraph) {
        #[cfg(test)]
        {
            self.search_counts[5] += 1;
        }
        while let Some(claim) = self.staged.pop() {
            self.restore(claim, allocator, graph);
        }
        self.prices.clear();
        for offers in &mut self.input_offers {
            offers.next = offers.first;
        }
    }

    /// Keeps accepted quantities reserved for all subsequent operations and projects.
    pub(crate) fn accept(&mut self) {
        if let Some(owner) = self.releasing_owner.take() {
            self.retire_owner(owner);
        }
        self.released.clear();
        self.staged.clear();
        self.prices.clear();
        self.offers.clear();
        self.input_offers.clear();
    }

    fn restore(&mut self, claim: Claim, allocator: &BuildingAllocator, graph: &RegionGraph) {
        self.journal_source(claim.source, claim.resource, claim.slot);
        self.remaining[claim.slot] = claim.previous;
        let available = claim.previous > 0.0;
        if self.active[claim.slot] != available {
            self.active[claim.slot] = available;
            self.set_available(claim.source, claim.resource, available, allocator, graph);
        }
    }

    /// Census commit for an owner quoted outside any revalidation transaction. Released and
    /// requoted owners can fragment claims beyond the reset estimate (up to twice the input
    /// ports plus output slots), so preflight reusable cells and grow the retained arena
    /// exactly before appending. O(staged claims + reusable cells); warm censuses keep the
    /// grown capacity and do not allocate.
    pub(crate) fn accept_growing_for(&mut self, owner: usize) {
        if let Err(required) = self.check_accept_capacity(owner) {
            self.reserve_receipts(required);
        }
        self.accept_for(owner);
    }

    /// Records a live owner's accepted input flows for a later replacement quote.
    /// O(claims), using the pass's preallocated sparse receipt arena.
    pub(crate) fn accept_for(&mut self, owner: usize) {
        debug_assert!(
            self.owner_receipts[owner] == usize::MAX || self.releasing_owner == Some(owner)
        );
        self.retire_owner(owner);
        self.releasing_owner = None;
        for index in 0..self.staged.len() {
            self.append_receipt(self.staged[index], owner);
        }
        self.accept();
    }

    /// Commits a repaired input gap without retiring the owner's unaffected receipts.
    /// The enclosing revalidation transaction owns rollback of both removal and extension.
    /// O(new claims), using the same prepared receipt arena as full owner quotations.
    pub(crate) fn extend_inputs_for(&mut self, owner: usize) {
        debug_assert!(self.revalidation.is_active());
        debug_assert!(self.releasing_owner.is_none() && self.released.is_empty());
        for index in 0..self.staged.len() {
            self.append_receipt(self.staged[index], owner);
        }
        self.accept();
    }

    fn append_receipt(&mut self, claim: Claim, owner: usize) {
        self.journal_source(claim.source, claim.resource, claim.slot);
        self.journal_owner(owner);
        let next_from_source = self.source_receipts[claim.slot];
        let receipt = Claim {
            owner,
            next_from_source,
            next_from_owner: usize::MAX,
            previous_from_source: usize::MAX,
            ..claim
        };
        let index = if self.free_receipt == usize::MAX {
            let index = self.committed.len();
            assert!(
                index < self.committed.capacity(),
                "input receipts require capacity preflight"
            );
            self.committed.push(receipt);
            index
        } else {
            let index = self.free_receipt;
            self.journal_receipt(index);
            self.free_receipt = self.committed[index].next_from_owner;
            self.committed[index] = receipt;
            index
        };
        if next_from_source != usize::MAX {
            self.journal_receipt(next_from_source);
            self.committed[next_from_source].previous_from_source = index;
        }
        self.source_receipts[claim.slot] = index;
        let last = self.owner_tails[owner];
        if last == usize::MAX {
            self.owner_receipts[owner] = index;
        } else {
            self.journal_receipt(last);
            self.committed[last].next_from_owner = index;
        }
        self.owner_tails[owner] = index;
    }

    /// Prepares at most one extra source receipt per matched customer before publication.
    pub(crate) fn prepare_customer_deliveries(&mut self, receipts: usize) {
        self.reserve_receipts(crate::utils::storage::spare_capacity(
            self.committed.len() + receipts,
            self.committed.capacity(),
        ));
    }

    /// Publishes the currently staffed part of a financed seller-to-buyer promise.
    /// Earlier input claims retain priority; the business book retains any promised future
    /// production. O(source ports + changed eligibility * log bucket size), no route query
    /// or per-delivery allocation after `prepare_customer_deliveries`. A storage shortage
    /// returns the exact required arena size before changing any source or receipt.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_customer_delivery(
        &mut self,
        source: usize,
        buyer: usize,
        resource: ResourceRuntimeId,
        requested: f64,
        quote: [f32; 2],
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        graph: &RegionGraph,
    ) -> Result<(), usize> {
        debug_assert!(self.staged.is_empty() && self.released.is_empty());
        let Some(slot) = self.slot(allocator, catalog, source, resource) else {
            return Ok(());
        };
        let previous = self.remaining[slot];
        let units = requested.min(previous);
        if units <= 0.0 {
            return Ok(());
        }
        self.check_receipt_capacity(1, None)?;
        self.journal_source(source, resource, slot);
        self.remaining[slot] -= units;
        if self.remaining[slot] == 0.0 {
            self.active[slot] = false;
            self.set_available(source, resource, false, allocator, graph);
        }
        self.append_receipt(
            Claim {
                source,
                resource,
                slot,
                previous,
                units,
                owner: buyer,
                next_from_owner: usize::MAX,
                next_from_source: usize::MAX,
                previous_from_source: usize::MAX,
                unit_price: quote[0],
                carrier_quote: quote[1],
            },
            buyer,
        );
        Ok(())
    }

    fn retire_owner(&mut self, owner: usize) {
        self.journal_owner(owner);
        let mut index = self.owner_receipts[owner];
        while index != usize::MAX {
            let receipt = self.committed[index];
            self.retire_receipt(index, receipt);
            index = receipt.next_from_owner;
        }
        self.owner_receipts[owner] = usize::MAX;
        self.owner_tails[owner] = usize::MAX;
    }

    fn retire_receipt(&mut self, index: usize, receipt: Claim) {
        self.journal_source(receipt.source, receipt.resource, receipt.slot);
        self.journal_receipt(index);
        if receipt.previous_from_source == usize::MAX {
            self.source_receipts[receipt.slot] = receipt.next_from_source;
        } else {
            self.journal_receipt(receipt.previous_from_source);
            self.committed[receipt.previous_from_source].next_from_source =
                receipt.next_from_source;
        }
        if receipt.next_from_source != usize::MAX {
            self.journal_receipt(receipt.next_from_source);
            self.committed[receipt.next_from_source].previous_from_source =
                receipt.previous_from_source;
        }
        self.committed[index].next_from_owner = self.free_receipt;
        self.free_receipt = index;
    }

    /// Receipt arena occupancy/capacity for allocation regressions; not simulation state.
    #[cfg(test)]
    pub(crate) fn receipt_storage(&self) -> (usize, usize) {
        (self.committed.len(), self.committed.capacity())
    }

    /// Removes spare capacity to exercise cold growth without corrupting any live receipt.
    #[cfg(test)]
    pub(crate) fn shrink_receipt_storage_for_test(&mut self) {
        debug_assert!(!self.revalidation.is_active());
        self.committed.shrink_to_fit();
        self.revalidation.prepare(
            self.remaining.len(),
            self.owner_receipts.len(),
            self.committed.capacity(),
        );
    }

    /// Visits only this source's committed buyers, including each resource's exact quantity.
    /// Cost is O(source ports + current input flows), without a citywide buyer scan,
    /// replacement-history traversal or per-visit allocation.
    pub(crate) fn visit_dependents(
        &self,
        source: usize,
        mut visit: impl FnMut(usize, ResourceRuntimeId, f64),
    ) {
        let Some(range) = self.source_range(source) else {
            return;
        };
        for slot in range {
            let mut index = self.source_receipts[slot];
            while index != usize::MAX {
                let receipt = self.committed[index];
                if Some(receipt.owner) != self.releasing_owner {
                    visit(receipt.owner, receipt.resource, receipt.units);
                }
                index = receipt.next_from_source;
            }
        }
    }

    /// Output already promised to current or prospective workers of accepted buyers.
    /// O(source output ports + that output's current receipts), without allocation.
    pub(crate) fn committed_output(
        &self,
        source: usize,
        resource: ResourceRuntimeId,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
    ) -> f64 {
        let Some(slot) = self.slot(allocator, catalog, source, resource) else {
            return 0.0;
        };
        let mut index = self.source_receipts[slot];
        let mut units = 0.0;
        while index != usize::MAX {
            let receipt = self.committed[index];
            if Some(receipt.owner) != self.releasing_owner {
                units += receipt.units;
            }
            index = receipt.next_from_source;
        }
        units
    }

    /// Temporarily releases only this incumbent's accepted inputs. Rejection restores exact
    /// prior quantities; acceptance retires the receipt so it cannot be credited twice.
    /// O(owner claims * log bucket size), without scanning other owners or suppliers.
    pub(crate) fn release_owner(
        &mut self,
        owner: usize,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        debug_assert!(self.staged.is_empty() && self.released.is_empty());
        let Some(&first) = self.owner_receipts.get(owner) else {
            return;
        };
        self.releasing_owner = Some(owner);
        let mut index = first;
        while index != usize::MAX {
            let receipt = self.committed[index];
            let previous = self.remaining[receipt.slot];
            self.journal_source(receipt.source, receipt.resource, receipt.slot);
            self.released.push(Claim {
                previous,
                ..receipt
            });
            self.remaining[receipt.slot] += receipt.units;
            if previous <= 0.0 && self.remaining[receipt.slot] > 0.0 {
                self.active[receipt.slot] = true;
                self.set_available(receipt.source, receipt.resource, true, allocator, graph);
            }
            index = receipt.next_from_owner;
        }
    }

    /// Removes only one source's deliveries to an affected buyer, retaining every other
    /// input receipt and price. O(owner flows + changed flows * log bucket size), without
    /// allocation or unrelated-owner scans. Requires outer revalidation for exact rollback.
    pub(crate) fn release_source_from_owner(
        &mut self,
        owner: usize,
        source: usize,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        debug_assert!(self.revalidation.is_active());
        debug_assert!(self.staged.is_empty() && self.released.is_empty());
        debug_assert!(self.releasing_owner.is_none());
        let mut index = self.owner_receipts[owner];
        let mut previous = usize::MAX;
        while index != usize::MAX {
            let receipt = self.committed[index];
            if receipt.source == source {
                self.journal_owner(owner);
                self.journal_source(source, receipt.resource, receipt.slot);
                self.remaining[receipt.slot] += receipt.units;
                if previous == usize::MAX {
                    self.owner_receipts[owner] = receipt.next_from_owner;
                } else {
                    self.journal_receipt(previous);
                    self.committed[previous].next_from_owner = receipt.next_from_owner;
                }
                if self.owner_tails[owner] == index {
                    self.owner_tails[owner] = previous;
                }
                self.retire_receipt(index, receipt);
                if self.source_receipts[receipt.slot] == usize::MAX {
                    // No claims remain. Restore the exact production quantity rather than
                    // retaining subtraction/addition roundoff from fractional old receipts.
                    self.remaining[receipt.slot] = self.production[receipt.slot];
                }
                let available = self.remaining[receipt.slot] > 0.0;
                if self.active[receipt.slot] != available {
                    self.active[receipt.slot] = available;
                    self.set_available(source, receipt.resource, available, allocator, graph);
                }
            } else {
                previous = index;
            }
            index = receipt.next_from_owner;
        }
    }

    /// Replacement price of an input in the staged operation, including finite source mixing.
    pub(crate) fn input_price(&self, resource: ResourceRuntimeId) -> f64 {
        self.prices
            .iter()
            .find(|&&(id, _)| id == resource)
            .map_or(0.0, |&(_, price)| price)
    }

    /// Visits the retained local quote tranches and real outside residual of an input.
    /// The callback receives source identity, daily units, goods price and carrier price.
    /// O(owner flows), in original quote order; returns false for unavailable residual supply.
    pub(crate) fn visit_input_sources(
        &self,
        owner: usize,
        input: InputRequirement,
        imported_price: f32,
        outside: Option<f32>,
        mut visit: impl FnMut(Option<usize>, f64, f32, f32),
    ) -> bool {
        let mut needed = f64::from(input.daily_units);
        let mut index = self
            .owner_receipts
            .get(owner)
            .copied()
            .unwrap_or(usize::MAX);
        while needed > 0.0 && index != usize::MAX {
            let receipt = self.committed[index];
            index = receipt.next_from_owner;
            if receipt.resource != input.resource {
                continue;
            }
            let units = needed.min(receipt.units);
            needed -= units;
            visit(
                Some(self.source_owner(receipt.source)),
                units,
                receipt.unit_price,
                receipt.carrier_quote,
            );
        }
        if needed > 0.0 {
            let Some(carrier) = outside else {
                return false;
            };
            visit(None, needed, imported_price, carrier);
        }
        true
    }

    /// Shares the prepared car components with business customer queries without rebuilding roads.
    pub(crate) fn freight_components(&self) -> Option<&ModeComponentIndex> {
        self.components.as_ref()
    }

    /// Sparse input-account preparation bound: local receipts plus one outside tranche per input.
    pub(crate) fn input_tranche_capacity(&self, input_ports: usize) -> usize {
        self.committed.len() + input_ports
    }

    /// Preparation for new project buyers: each quote either fills an input or exhausts a
    /// source, plus the retained input flows that replacements can release during this pass.
    /// Revalidated existing buyers require their own exact capacity preflight.
    pub(crate) fn project_input_tranche_capacity(&self, input_ports: usize) -> usize {
        self.committed.len() + self.remaining.len() + input_ports
    }

    /// Reprices a retained owner's input requirement from its accepted supplier receipts.
    /// Unclaimed units use the caller's real outside quote. O(owner input flows), without
    /// allocating, searching the city, changing reservations or valuing freight as stock.
    pub(crate) fn owner_input_costs(
        &self,
        owner: usize,
        input: InputRequirement,
        imported_price: f32,
        outside: Option<f32>,
    ) -> InputSupplyCosts {
        if input.daily_units <= 0.0 {
            return input.at_prices(imported_price, 0.0);
        }
        if input.replenishment_batch_units.is_none() {
            return InputSupplyCosts {
                goods: f64::INFINITY,
                ..Default::default()
            };
        }
        let (needed, mut costs) = self.retained_input_costs(owner, input);
        if needed > 0.0 {
            if let Some(carrier) = outside {
                costs += input.at_prices_for_units(needed, imported_price, carrier);
            } else {
                costs.goods = f64::INFINITY;
            }
        }
        costs
    }

    fn retained_input_costs(
        &self,
        owner: usize,
        input: InputRequirement,
    ) -> (f64, InputSupplyCosts) {
        let mut needed = f64::from(input.daily_units);
        let mut costs = InputSupplyCosts::default();
        let mut index = self
            .owner_receipts
            .get(owner)
            .copied()
            .unwrap_or(usize::MAX);
        while index != usize::MAX {
            let receipt = self.committed[index];
            index = receipt.next_from_owner;
            if receipt.resource != input.resource {
                continue;
            }
            let units = needed.min(receipt.units);
            needed -= units;
            costs += input.at_prices_for_units(units, receipt.unit_price, receipt.carrier_quote);
            if needed == 0.0 {
                break;
            }
        }
        (needed, costs)
    }

    /// Quotes and stages one input using exact directed routes and actual supplier prices.
    /// Repeated source searches skip depleted entries; an exhausted component costs O(log C).
    /// Imported residual supply requires a real border quote. Caller accepts or rejects the
    /// complete operation, so a later input/capital/income failure cannot consume local supply.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn quote(
        &mut self,
        input: InputRequirement,
        buyer: Option<usize>,
        retained_owner: Option<usize>,
        entrance: &BuildingEntrance,
        outside_quote: Option<f32>,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        network: &TransitNetwork,
        graph: &RegionGraph,
        routes: &mut FreightRouteCache,
        goods_multiplier: f32,
    ) -> InputSupplyCosts {
        self.work[3] += 1;
        if input.daily_units > 0.0 && !goods_multiplier.is_finite() {
            return InputSupplyCosts {
                goods: f64::INFINITY,
                buffer: f64::INFINITY,
                ..Default::default()
            };
        }
        let imported_price =
            super::imported_input_unit_price(catalog, tuning, input.resource, goods_multiplier);
        if input.daily_units <= 0.0 {
            // Internally replenished circulation stock needs valuation, not a recurring order.
            self.prices
                .push((input.resource, f64::from(imported_price)));
            return input.at_prices(imported_price, 0.0);
        }
        let Some(batch) = input.replenishment_batch_units else {
            return InputSupplyCosts {
                goods: f64::INFINITY,
                ..Default::default()
            };
        };
        // Keep f64 residuals: narrowing a fragmented remainder to f32 can invent or lose
        // supply. Buffer/freight exposure continues to use the complete input requirement.
        let (mut needed, mut result) = retained_owner.map_or_else(
            || (f64::from(input.daily_units), InputSupplyCosts::default()),
            |owner| self.retained_input_costs(owner, input),
        );
        while needed > 0.0 {
            let choice = self.next_offer(
                input.resource,
                buyer,
                self.releasing_owner.or(retained_owner),
                entrance,
                allocator,
                catalog,
                network,
                graph,
                routes,
            );
            let Some((source, slot, seconds)) = choice else {
                break;
            };
            let profile = self.source_profile(source, allocator, catalog).unwrap();
            let carrier = freight_cost(
                seconds,
                tuning.operational_clock.seconds_per_day,
                &tuning.logistics,
            );
            let local_price = profile.unit_price_currency * goods_multiplier;
            // A viable outside route remains available when the local offer costs more.
            if outside_quote.is_some_and(|outside| {
                imported_price + outside / batch < local_price + carrier / batch
            }) {
                break;
            }
            let previous = self.remaining[slot];
            let units = needed.min(previous);
            needed -= units;
            self.journal_source(source, input.resource, slot);
            self.remaining[slot] -= units;
            if self.remaining[slot] == 0.0 {
                self.active[slot] = false;
                self.set_available(source, input.resource, false, allocator, graph);
            }
            self.staged.push(Claim {
                source,
                resource: input.resource,
                slot,
                previous,
                units,
                owner: usize::MAX,
                next_from_owner: usize::MAX,
                next_from_source: usize::MAX,
                previous_from_source: usize::MAX,
                unit_price: local_price,
                carrier_quote: carrier,
            });
            result += input.at_prices_for_units(units, local_price, carrier);
        }
        if needed > 0.0 {
            if let Some(carrier) = outside_quote {
                result += input.at_prices_for_units(needed, imported_price, carrier);
            } else {
                result.goods = f64::INFINITY;
            }
        }
        self.prices.push((
            input.resource,
            if result.goods.is_finite() {
                result.goods / f64::from(input.daily_units)
            } else {
                f64::from(imported_price)
            },
        ));
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn next_offer(
        &mut self,
        resource: ResourceRuntimeId,
        buyer: Option<usize>,
        excluded: Option<usize>,
        entrance: &BuildingEntrance,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        network: &TransitNetwork,
        graph: &RegionGraph,
        routes: &mut FreightRouteCache,
    ) -> Option<(usize, usize, f32)> {
        let list = self
            .input_offers
            .iter()
            .position(|offers| offers.resource == resource)
            .unwrap_or_else(|| {
                self.input_offers.push(InputOffers {
                    resource,
                    first: usize::MAX,
                    last: usize::MAX,
                    next: usize::MAX,
                    exhausted: false,
                });
                self.input_offers.len() - 1
            });
        while self.input_offers[list].next != usize::MAX {
            let offer = self.offers[self.input_offers[list].next];
            self.input_offers[list].next = offer.next;
            if self.remaining[offer.slot] > 0.0 {
                #[cfg(test)]
                {
                    self.search_counts[4] += 1;
                }
                return Some((offer.source, offer.slot, offer.seconds));
            }
        }
        if self.input_offers[list].exhausted {
            return None;
        }
        // The existing resource indices can prove an empty market without deriving buyer
        // components or opening a spatial search. Normal quote fallback still prices imports.
        let choice = if self.offers_resource(resource) {
            self.nearest(
                resource, buyer, excluded, entrance, allocator, catalog, network, graph, routes,
            )
        } else {
            self.nearest_incumbents[resource as usize] = None;
            None
        };
        if let Some((source, slot, seconds)) = choice {
            let next = self.offers.len();
            let previous = self.input_offers[list].last;
            if previous != usize::MAX {
                self.offers[previous].next = next;
            } else {
                self.input_offers[list].first = next;
            }
            self.input_offers[list].last = next;
            self.offers.push(Offer {
                source,
                slot,
                seconds,
                next: usize::MAX,
            });
        } else {
            self.input_offers[list].exhausted = true;
        }
        choice
    }

    #[allow(clippy::too_many_arguments)]
    fn nearest(
        &mut self,
        resource: ResourceRuntimeId,
        buyer: Option<usize>,
        excluded: Option<usize>,
        entrance: &BuildingEntrance,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        network: &TransitNetwork,
        graph: &RegionGraph,
        routes: &mut FreightRouteCache,
    ) -> Option<(usize, usize, f32)> {
        let components =
            self.components
                .as_ref()?
                .entrance_components(entrance, graph, TransitFlags::CAR);
        let mut counts = [1, 0, 0, 0];
        // Reprice the preceding buyer's supplier as an initial upper bound. Every
        // candidate that can beat it is still checked, so this changes search cost,
        // never route eligibility or deterministic route/price/index ordering.
        let incumbent = self.nearest_incumbents[resource as usize];
        let mut best = incumbent.and_then(|source| {
            // Projected membership can shrink between forks. Validate the handle
            // before reading its owner; a stale bound has no current supply slot.
            let slot = self.slot(allocator, catalog, source, resource)?;
            if self.remaining[slot] <= 0.0
                || Some(self.source_owner(source)) == buyer
                || Some(self.source_owner(source)) == excluded
            {
                return None;
            }
            counts[2] += 1;
            let seconds =
                self.source_route(source, buyer, entrance, allocator, network, graph, routes)?;
            Some((source, slot, seconds))
        });
        for buckets in [
            self.index.buckets_for_resource(resource),
            self.projected_buckets(resource),
        ]
        .into_iter()
        .flatten()
        {
            buckets.scan_nearest_bounded(
                components,
                entrance.door_pos.x,
                entrance.door_pos.y,
                |event| match event {
                    BoundedScanEvent::Item { item_idx: source } => {
                        counts[1] += 1;
                        if Some(self.source_owner(source)) == buyer
                            || Some(self.source_owner(source)) == excluded
                            || best.is_some_and(|best| best.0 == source)
                        {
                            return true;
                        }
                        if let Some((_, _, seconds)) = best {
                            let door = match self.doors.get(source) {
                                Some(&door) => door,
                                None => self
                                    .source_entrance(source, allocator)
                                    .map(|origin| origin.door_pos),
                            };
                            let Some(door) = door else {
                                return true;
                            };
                            // The complete door route cannot beat straight-line travel at
                            // the fastest road/driveway speed. Avoid even a cached route
                            // lookup when this candidate cannot improve the incumbent.
                            // Every rejection here is pure, so pruning before the recipe
                            // slot lookup changes search cost only.
                            let distance_sq = (door - entrance.door_pos).length_squared();
                            if lower_bound_travel_seconds(distance_sq, self.max_speed) > seconds {
                                return true;
                            }
                        }
                        let Some(slot) = self.slot(allocator, catalog, source, resource) else {
                            return true;
                        };
                        if self.remaining[slot] <= 0.0 {
                            return true;
                        }
                        counts[2] += 1;
                        let seconds = self.source_route(
                            source, buyer, entrance, allocator, network, graph, routes,
                        );
                        if let Some(seconds) = seconds {
                            // Equal routes have the same carrier fee. Only read prices on
                            // a time tie; ordinary nearest-route searches need no extra lookup.
                            if best.is_none_or(|current| {
                                seconds
                                    .total_cmp(&current.2)
                                    .then_with(|| {
                                        let price = |index: usize| {
                                            self.source_profile(index, allocator, catalog)
                                                .unwrap()
                                                .unit_price_currency
                                        };
                                        price(source).total_cmp(&price(current.0))
                                    })
                                    .then_with(|| {
                                        self.source_owner(source).cmp(&self.source_owner(current.0))
                                    })
                                    .then_with(|| source.cmp(&current.0))
                                    .is_lt()
                            }) {
                                best = Some((source, slot, seconds));
                            }
                        }
                        true
                    }
                    // Every member of a rejected region fails the item prune above.
                    BoundedScanEvent::Region {
                        min_distance_sq: next_min_distance_sq,
                    }
                    | BoundedScanEvent::RingComplete {
                        next_min_distance_sq,
                    } => {
                        counts[3] += 1;
                        best.is_none_or(|(_, _, seconds)| {
                            lower_bound_travel_seconds(next_min_distance_sq, self.max_speed)
                                <= seconds
                        })
                    }
                },
            );
        }
        self.nearest_incumbents[resource as usize] = best.map(|(source, _, _)| source);
        for (total, count) in self.work.iter_mut().zip(&counts[..3]) {
            *total += count;
        }
        #[cfg(test)]
        {
            for (total, count) in self.search_counts.iter_mut().zip(counts) {
                *total += count;
            }
            use crate::debug::work::{Work, add};
            add(Work::SupplierSearches, counts[0]);
            add(Work::SupplierCandidates, counts[1]);
            add(Work::SupplierRoutes, counts[2]);
        }
        best
    }
}

// Prefix offsets split variable-length output rows into disjoint mutable ranges safely.
// Independent production arithmetic runs on Rayon; the dependent prefix itself stays serial.
fn fill_production_rates(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    funded: &[f32],
    offsets: &[usize],
    first: usize,
    end: usize,
    remaining: &mut [f64],
) {
    if end - first > 256 {
        let middle = first + (end - first) / 2;
        let (left, right) = remaining.split_at_mut(offsets[middle] - offsets[first]);
        rayon::join(
            || fill_production_rates(allocator, catalog, funded, offsets, first, middle, left),
            || fill_production_rates(allocator, catalog, funded, offsets, middle, end, right),
        );
        return;
    }
    for index in first..end {
        let building = &allocator.buildings[index];
        let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
        else {
            continue;
        };
        let activity =
            production_activity(building, profile, funded.get(index).copied().unwrap_or(0.0));
        let area = output_area_scale_for_building(building, profile);
        for (output, value) in profile.outputs.iter().zip(
            &mut remaining[offsets[index] - offsets[first]..offsets[index + 1] - offsets[first]],
        ) {
            *value = f64::from(profile.net_output_units_per_day(output) * area * activity);
        }
    }
}

fn production_activity(
    building: &crate::simulation::buildings::allocator::Building,
    profile: &EconomyProfileRuntime,
    funded: f32,
) -> f32 {
    if building.broken
        || building.economy_broken
        || building.is_deserted
        || building.is_under_construction()
        || building.edge_idx == usize::MAX
    {
        return 0.0;
    }
    let physical = physical_worker_capacity_for_profile(building, profile);
    (building.worker_count as f32 / physical.max(1) as f32)
        .min(funded)
        .clamp(0.0, 1.0)
}
