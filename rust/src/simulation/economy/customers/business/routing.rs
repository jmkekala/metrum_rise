// SPDX-License-Identifier: GPL-2.0-only

//! Directed business customer offers through the existing resource/component/chunk buckets.

use super::BusinessBudgets;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketEntry, ReachableBucketIndex,
    ReachableBucketScanEvent, chunk_for_point,
};
use crate::simulation::economy::definitions::{RuntimeEconomyCatalog, RuntimeEconomyTuning};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph, types::TransitFlags};
use rayon::prelude::*;

mod finance_masks;
mod sites;
pub(crate) use sites::PreparedBusinessSite;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Access {
    chunk: (i32, i32),
    components: BuildingModeComponents,
}

/// Net seller receipts from financed business customers, excluding carrier payments.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct BusinessSales {
    /// Quantity whose customer accounts and directed deliveries were reserved.
    pub(crate) units: f64,
    /// Actual goods receipts, including each buyer's receiving-price multiplier.
    pub(crate) revenue: f64,
}

/// Sparse financed buyer accounts with stable directed customer selection.
/// Road components are borrowed from the caller's existing access census; this book owns
/// only account buckets, not another road graph or spatial partition.
#[derive(Clone, Debug, Default)]
pub(crate) struct BusinessMarket {
    budgets: BusinessBudgets,
    access: Vec<Access>,
    profiles: Vec<u16>,
    identities: Vec<(usize, u16)>,
    port_access: Vec<Access>,
    port_sites: Vec<Option<usize>>,
    sites: sites::PreparedSites,
    buyer_ports: Vec<std::ops::Range<usize>>,
    account_ports: Vec<usize>,
    account_available: Vec<bool>,
    previous_ports: Vec<usize>,
    port_changed: Vec<u64>,
    port_generation: u64,
    port_checkpoint: Option<usize>,
    pass_ports: PassPorts,
    port_available: Vec<usize>,
    indices: PortIndices,
    // Each resource's bucket entries between an index rebuild's collection and its build.
    index_entries: Vec<Vec<ReachableBucketEntry>>,
    revision: Option<(u64, u64, u32)>,
    visited: Vec<u64>,
    visit: u64,
    // Scan events the claims visited and carrier quotes they priced so far, which the funding
    // census counts as work (`SIM-02-K`); monotonic, never read by any result.
    claim_work: [u64; 2],
    #[cfg(test)]
    index_builds: u64,
    #[cfg(test)]
    quote_counts: [u64; 3],
    #[cfg(test)]
    rejection_causes: [u64; 5],
    price_classes: Vec<Vec<u32>>,
    account_masks: Vec<u64>,
    port_masks: Vec<u64>,
    dirty_buyers: Vec<usize>,
    buyer_dirty: Vec<bool>,
    // The census's own layout while a planning pass covers buildings appended since.
    census_buyers: Option<CensusBuyers>,
    #[cfg(test)]
    disable_finance_masks: bool,
}

/// A business layout after seeding between census slices (`SIM-02-K`): the revision it lays
/// out for, whether the buyers' access changed, whether the ports and indices are rebuilt and
/// the resource count. [`BusinessMarket::begin_layout`] starts one.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SeedingLayout {
    revision: (u64, u64, u32),
    changed: bool,
    rebuilds: bool,
    resources: usize,
}

impl SeedingLayout {
    /// Whether the buyers' door chunks and components are recomputed.
    pub(crate) fn changes_access(&self) -> bool {
        self.changed
    }

    /// Whether the recipe ports and live indices are rebuilt.
    pub(crate) fn rebuilds(&self) -> bool {
        self.rebuilds
    }

    /// Resources whose live index a rebuild builds.
    pub(crate) fn resources(&self) -> usize {
        self.resources
    }
}

/// The census's buyer count, live port count and live index, kept aside while a lent book
/// lays out the buildings appended since its census.
#[derive(Clone, Debug, Default)]
struct CensusBuyers {
    buyers: usize,
    live_ports: usize,
    live: Vec<ReachableBucketIndex>,
}

impl BusinessMarket {
    /// Scan events visited and carrier quotes priced by claims so far, for the funding census's
    /// work units (`SIM-02-K`). Monotonic counters; O(1).
    pub(crate) fn claim_work(&self) -> [u64; 2] {
        self.claim_work
    }

    /// Lifetime affordability prechecks, rejected routes, and rejects that free freight
    /// would not prove. Diagnostics only; production does not repeat the free-price check.
    #[cfg(test)]
    pub(crate) fn quote_counts_for_test(&self) -> [u64; 3] {
        self.quote_counts
    }

    /// Examined rejected account rows, quantity-rounding loss, invalid financing, remaining
    /// free-freight failures, and carrier-only failures. Rows can exceed rejected ports.
    #[cfg(test)]
    pub(crate) fn rejection_causes_for_test(&self) -> [u64; 5] {
        self.rejection_causes
    }

    /// Buyers whose finance masks wait for the next refresh, for tests.
    #[cfg(test)]
    pub(crate) fn changed_finance_buyers_for_test(&self) -> usize {
        self.dirty_buyers.len()
    }

    /// Uses the original complete candidate scan as an independent transactional oracle.
    #[cfg(test)]
    pub(crate) fn disable_finance_masks_for_test(&mut self) {
        self.disable_finance_masks = true;
    }

    /// Forks the jointly financed live book, for tests: planning lends it instead. Warm cost
    /// follows account tranches and receipts; the resource buckets retain their storage.
    #[cfg(test)]
    pub(crate) fn copy_reservations_from(&mut self, source: &Self) {
        if self.revision != source.revision
            || self.access != source.access
            || self.identities.get(..source.identities.len()) != Some(source.identities.as_slice())
            || self.indices.live.len() != source.indices.live.len()
        {
            // Refresh layout once, retaining account/journal capacity for the common
            // financial copy below. Prepared variants keep geometry, never old eligibility.
            self.access.clone_from(&source.access);
            self.identities.clone_from(&source.identities);
            self.port_access.clone_from(&source.port_access);
            self.port_sites.clone_from(&source.port_sites);
            self.buyer_ports.clone_from(&source.buyer_ports);
            self.indices.live.clone_from(&source.indices.live);
            self.indices.live_ports = source.indices.live_ports;
            // Proposed ports follow the live ports; prepare_sites renumbers and re-indexes them.
            self.indices.sites.clear();
            self.revision = source.revision;
            self.sites.layout = None;
            #[cfg(test)]
            {
                self.index_builds = source.index_builds;
                self.quote_counts = source.quote_counts;
                self.rejection_causes = source.rejection_causes;
                self.disable_finance_masks = source.disable_finance_masks;
            }
        }
        self.budgets.copy_reservations_from(&source.budgets);
        self.profiles.clone_from(&source.profiles);
        self.price_classes.clone_from(&source.price_classes);
        self.reset_accounts();
    }

    /// Gives this copy of `live` the capacity of the arenas a census refills (`SIM-02-K`).
    pub(crate) fn reserve_like(&mut self, live: &Self) {
        self.budgets.reserve_like(&live.budgets);
    }

    /// Prepares live and temporary project owner slots outside the proposal loop.
    pub(crate) fn prepare_owners(&mut self, owners: usize, output_ports: usize) {
        // Owner rows follow the city; spare rows keep an appended building from reallocating
        // them all in one planning slot (`SIM-02-K`).
        use crate::utils::storage::{grow_rows, reserve_rows};
        let budgets = &mut self.budgets;
        grow_rows(&mut budgets.owners, owners, usize::MAX);
        grow_rows(&mut budgets.finance, owners, [0.0; 2]);
        grow_rows(&mut budgets.buyer_accounts, owners, usize::MAX);
        grow_rows(&mut budgets.buyer_tails, owners, usize::MAX);
        grow_rows(&mut budgets.finance_staged, owners, false);
        reserve_rows(&mut budgets.staged_finance, owners);
        let receipts = crate::utils::storage::spare_capacity(
            2 * (self.budgets.accounts.len() + output_ports),
            self.budgets.committed.capacity(),
        );
        self.budgets
            .committed
            .reserve(receipts.saturating_sub(self.budgets.committed.len()));
        self.reserve_accounts(self.budgets.accounts.capacity());
        self.prepare_finance_mask_storage();
    }

    /// Prepares finance-account mappings between complete proofs, retaining recipe geometry.
    pub(crate) fn reserve_accounts(&mut self, required: usize) {
        self.budgets.reserve_accounts(required);
        let capacity = self.budgets.accounts.capacity();
        self.account_ports
            .reserve(capacity.saturating_sub(self.account_ports.len()));
        self.account_available
            .reserve(capacity.saturating_sub(self.account_available.len()));
        self.account_masks
            .reserve(capacity.saturating_sub(self.account_masks.len()));
        self.previous_ports.resize(capacity, usize::MAX);
        self.port_changed.resize(capacity, 0);
    }

    /// Prepares the pass's bounded new-buyer tranches and their source-owned receipts.
    /// Existing replacement buyers still use the exact reseeding preflight.
    /// Both arenas keep spare capacity past the request (`SIM-02-K`).
    pub(crate) fn prepare_project_inputs(&mut self, additional_tranches: usize) {
        use crate::utils::storage::spare_capacity;
        let budgets = &self.budgets;
        let receipts = spare_capacity(
            budgets.committed.len() + additional_tranches,
            budgets.committed.capacity(),
        );
        let accounts = spare_capacity(
            budgets.accounts.len() + additional_tranches,
            budgets.accounts.capacity(),
        );
        self.reserve_receipts(receipts);
        self.reserve_accounts(accounts);
    }

    /// Starts an enclosing replacement proof over preallocated account/receipt snapshots.
    pub(crate) fn begin_revalidation(&mut self) {
        self.budgets.begin_revalidation();
        self.port_generation = self.port_generation.wrapping_add(1);
        if self.port_generation == 0 {
            self.port_changed.fill(0);
            self.port_generation = 1;
        }
        self.port_checkpoint = Some(self.account_ports.len());
    }

    /// Publishes the accepted inner supplier substitutions as one completed proof.
    pub(crate) fn accept_revalidation(&mut self) {
        self.budgets.accept_revalidation();
        self.port_checkpoint = None;
    }

    /// Restores accepted inner quotes and only the affected customer bucket entries.
    pub(crate) fn reject_revalidation(&mut self) {
        self.reject();
        let dirty = &mut self.dirty_buyers;
        let flags = &mut self.buyer_dirty;
        self.budgets
            .visit_revalidation_buyers(|buyer| finance_masks::mark_dirty(dirty, flags, buyer));
        let access = &self.port_access;
        let indices = &mut self.indices;
        let ports = &mut self.account_ports;
        let accounts = &mut self.account_available;
        let available = &mut self.port_available;
        self.budgets.visit_revalidation_accounts(|slot, account| {
            if self.port_checkpoint.is_some_and(|len| slot < len)
                && self.port_changed[slot] == self.port_generation
            {
                let current = self.budgets.accounts[slot];
                sync_account(
                    indices,
                    access[ports[slot]],
                    ports,
                    accounts,
                    available,
                    slot,
                    current.resource,
                    false,
                );
                ports[slot] = self.previous_ports[slot];
            }
            sync_account(
                indices,
                access[ports[slot]],
                ports,
                accounts,
                available,
                slot,
                account.resource,
                account.units > 0.0,
            );
        });
        self.budgets.reject_revalidation();
        self.account_ports.truncate(self.budgets.accounts.len());
        self.account_available.truncate(self.budgets.accounts.len());
        self.port_checkpoint = None;
    }

    /// Whether a staged replacement leaves every affected buyer financed.
    pub(crate) fn can_accept(&self) -> bool {
        self.budgets.can_accept()
    }

    /// Preparation bound for publishing additional deliveries; includes retained input receipts.
    pub(crate) fn receipt_count(&self) -> usize {
        self.budgets.committed.len()
    }

    /// Visits customer matches not already represented by the input ledger, in seller order.
    /// O(sellers + current receipts), without allocation or retired-history traversal.
    pub(crate) fn visit_additional_sales(
        &self,
        mut visit: impl FnMut(usize, usize, u16, f64, [f32; 2]),
    ) {
        for (seller, &first) in self.budgets.owners.iter().enumerate() {
            let mut index = first;
            while index != usize::MAX {
                let receipt = self.budgets.committed[index];
                index = receipt.next;
                if receipt.input_origin {
                    continue;
                }
                let account = self.budgets.accounts[receipt.account];
                visit(
                    seller,
                    account.buyer,
                    account.resource,
                    receipt.units,
                    receipt.quote,
                );
            }
        }
    }

    /// Net financed business sales reserved for one output; O(owner customer receipts).
    pub(crate) fn sales(&self, seller: usize, resource: u16, limit: f64) -> BusinessSales {
        let (units, revenue) = self.budgets.sales(seller, resource, limit);
        BusinessSales { units, revenue }
    }

    /// Financing after all staged supplier substitutions, before correcting provisional sales.
    pub(crate) fn financing(&self, buyer: usize) -> [f64; 2] {
        self.budgets.finance[buyer]
    }

    /// Updates one revalidated operation inside the same enclosing transaction.
    pub(crate) fn set_financing(&mut self, buyer: usize, headroom: [f64; 2]) {
        self.budgets.set_finance(buyer, headroom);
        self.mark_finance_dirty(buyer);
    }

    /// Visits a seller's current customers, following only its live receipt chain.
    pub(crate) fn visit_customers(
        &self,
        seller: usize,
        mut visit: impl FnMut(usize, u16, f64, [f32; 2]),
    ) {
        let mut index = self.budgets.owners[seller];
        while index != usize::MAX {
            let receipt = self.budgets.committed[index];
            let account = self.budgets.accounts[receipt.account];
            visit(
                account.buyer,
                account.resource,
                receipt.units,
                receipt.quote,
            );
            index = receipt.next;
        }
    }

    /// Visits only this buyer's current suppliers, including future commitments.
    pub(crate) fn visit_suppliers(&self, buyer: usize, mut visit: impl FnMut(usize)) {
        let mut slot = self.budgets.buyer_accounts[buyer];
        while slot != usize::MAX {
            let mut index = self.budgets.accounts[slot].first_receipt;
            while index != usize::MAX {
                let receipt = self.budgets.committed[index];
                visit(receipt.seller);
                index = receipt.next_account;
            }
            slot = self.budgets.accounts[slot].next_buyer;
        }
    }

    /// Publishes the completed joint sales proof as each buyer's financing limit.
    pub(crate) fn publish_financing(&mut self, headroom: &[[f64; 2]]) {
        debug_assert!(!self.budgets.revalidation.is_active());
        self.budgets
            .finance
            .par_iter_mut()
            .zip(headroom)
            .for_each(|(finance, &actual)| {
                *finance = actual;
            });
        for buyer in 0..self.budgets.finance.len() {
            self.mark_finance_dirty(buyer);
        }
    }

    /// Prepares an exact storage request between complete matching attempts, never inside
    /// the seller loop. The next reset discards the unpublished prefix and replays all claims.
    pub(crate) fn reserve_receipts(&mut self, required: usize) {
        self.budgets.reserve_receipts(required);
    }

    /// Retains matched customer claims without allocating. Exhaustion returns the required
    /// arena size; the caller must grow between attempts and replay before publishing finance.
    pub(crate) fn try_extend_for(&mut self, seller: usize) -> Result<(), usize> {
        self.budgets.try_extend_for(seller)
    }

    /// Seeds a complete evaluated financing set before exposing any of its purchases.
    /// Warm preparation follows account tranches and authored input ports. Spatial membership
    /// belongs to buyer/recipe ports, so financial fragmentation and metric changes cannot
    /// rebuild it. Geometry changes rebuild existing buckets, parallel across resources.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reset(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
        catalog: &RuntimeEconomyCatalog,
        input_tranches: usize,
        output_ports: usize,
        seed: impl FnOnce(&mut BusinessBudgets),
    ) {
        self.begin_seeding(allocator.buildings.len(), input_tranches, output_ports);
        seed(&mut self.budgets);
        self.finish_seeding(allocator, network, graph, components, catalog);
    }

    /// Seeding's first phase (`SIM-02-K`): the sparse accounts' reset for `buildings` buyers
    /// and sellers. Then the caller seeds buyers through [`Self::seeding_book`] in buyer
    /// order, in any number of calls, and [`Self::finish_seeding`] prepares the layout and the
    /// availability; [`Self::reset`] runs the three at once. O(B) fills.
    pub(crate) fn begin_seeding(
        &mut self,
        buildings: usize,
        input_tranches: usize,
        output_ports: usize,
    ) {
        self.budgets
            .reset(buildings, input_tranches, buildings, output_ports);
    }

    /// The accounts being seeded between [`Self::begin_seeding`] and [`Self::finish_seeding`].
    pub(crate) fn seeding_book(&mut self) -> &mut BusinessBudgets {
        &mut self.budgets
    }

    /// Seeding's last phases at once (`SIM-02-K`): price classes, the buyer and recipe-port
    /// layout and spatial buckets after a geometry or profile change, and every account's
    /// availability; see [`Self::begin_layout`]. O(B + accounts), plus O(ports log ports)
    /// after a layout change.
    pub(crate) fn finish_seeding(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
        catalog: &RuntimeEconomyCatalog,
    ) {
        let buyers = allocator.buildings.len();
        let layout = self.begin_layout(allocator, network, catalog);
        self.layout_access(&layout, allocator, graph, components, 0..buyers);
        self.layout_ports(&layout, allocator, catalog, 0..buyers);
        self.collect_layout_entries(&layout);
        self.build_layout_indices(&layout, 0..layout.resources);
        self.finish_layout(&layout, allocator);
    }

    /// The layout's first phase after seeding (`SIM-02-K`): price classes and whether the
    /// buyer and recipe-port layout changed. Then [`Self::layout_access`] and
    /// [`Self::layout_ports`] over every buyer range in order, [`Self::collect_layout_entries`],
    /// [`Self::build_layout_indices`] over every resource range and [`Self::finish_layout`]; any
    /// slicing gives one call's layout. A rebuild names no layout until it finishes, so one
    /// left part way is rebuilt by the next seeding. O(B).
    pub(crate) fn begin_layout(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        catalog: &RuntimeEconomyCatalog,
    ) -> SeedingLayout {
        self.prepare_price_classes(catalog);
        debug_assert!(self.budgets.staged.is_empty());
        let revision = (
            allocator.building_ref_revision(),
            allocator.entrance_ref_revision(),
            network.cch_graph.build_generation,
        );
        let changed = self.revision != Some(revision)
            || !self.profiles.iter().copied().eq(allocator
                .buildings
                .iter()
                .map(|building| building.economy_profile_runtime_id));
        let resources = catalog.resource_count() + 1;
        self.access
            .resize(allocator.buildings.len(), Access::default());
        let rebuilds = changed || self.indices.live.len() != resources;
        if rebuilds {
            self.revision = None;
            self.profiles.clear();
            self.identities.clear();
            self.port_access.clear();
            self.port_sites.clear();
            self.sites.invalidate_layout();
            self.buyer_ports.resize(allocator.buildings.len(), 0..0);
        }
        SeedingLayout {
            revision,
            changed,
            rebuilds,
            resources,
        }
    }

    /// Each buyer's door chunk and freight components for buyers `range` after a layout
    /// change (`SIM-02-K`). O(range) in parallel.
    pub(crate) fn layout_access(
        &mut self,
        layout: &SeedingLayout,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
        range: std::ops::Range<usize>,
    ) {
        if !layout.changed {
            return;
        }
        let first = range.start;
        self.access[range]
            .par_iter_mut()
            .enumerate()
            .for_each(|(offset, access)| {
                let buyer = first + offset;
                *access = Access {
                    chunk: allocator.entrances.get(buyer).map_or((0, 0), |entrance| {
                        chunk_for_point(entrance.door_pos.x, entrance.door_pos.y)
                    }),
                    components: components.building_components(
                        allocator,
                        graph,
                        buyer,
                        TransitFlags::CAR,
                    ),
                };
            });
    }

    /// The recipe ports of buyers `range`, the next ones, in buyer order, after a layout
    /// change (`SIM-02-K`); the last range finishes the port tables. O(range + their ports),
    /// serial.
    pub(crate) fn layout_ports(
        &mut self,
        layout: &SeedingLayout,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        range: std::ops::Range<usize>,
    ) {
        if !layout.rebuilds {
            return;
        }
        let last = range.end == allocator.buildings.len();
        let ports = self.identities.len();
        // Prefix ranges depend on the previous buyer; catalogue input order is stable.
        for (buyer, building) in allocator.buildings[range.clone()]
            .iter()
            .enumerate()
            .map(|(offset, building)| (range.start + offset, building))
        {
            let first = self.identities.len();
            if let Some(profile) =
                catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            {
                self.identities.extend(
                    profile
                        .inputs
                        .iter()
                        .map(|input| (buyer, input.resource_runtime_id)),
                );
            }
            self.buyer_ports[buyer] = first..self.identities.len();
        }
        self.port_access.extend(
            self.identities[ports..]
                .iter()
                .map(|&(buyer, _)| self.access[buyer]),
        );
        if last {
            self.port_sites.resize(self.identities.len(), None);
            self.indices.live_ports = self.identities.len();
            self.indices.sites.clear();
        }
    }

    /// Every resource's bucket entries of the live ports after a layout change (`SIM-02-K`).
    /// O(ports), serial.
    pub(crate) fn collect_layout_entries(&mut self, layout: &SeedingLayout) {
        if layout.rebuilds {
            self.collect_index_entries(layout.resources, false);
        }
    }

    /// Builds the live indices of resources `range` after a layout change (`SIM-02-K`).
    /// O(entries log entries) for those resources, in parallel.
    pub(crate) fn build_layout_indices(
        &mut self,
        layout: &SeedingLayout,
        range: std::ops::Range<usize>,
    ) {
        if layout.rebuilds {
            self.build_index_resources(range, false);
        }
    }

    /// The layout's end (`SIM-02-K`): the buyers' profiles after a rebuild, every account's
    /// availability and finance masks, and the layout's revision. O(B + accounts + ports).
    pub(crate) fn finish_layout(&mut self, layout: &SeedingLayout, allocator: &BuildingAllocator) {
        if layout.rebuilds {
            // `begin_layout` cleared them; written last, they name the finished layout.
            self.profiles.extend(
                allocator
                    .buildings
                    .iter()
                    .map(|building| building.economy_profile_runtime_id),
            );
        }
        self.reset_accounts();
        self.revision = Some(layout.revision);
    }

    /// Fixed recipe-port count and spatial rebuild count for allocation/invalidation checks.
    #[cfg(test)]
    pub(crate) fn prepared_layout(&self) -> (usize, u64) {
        (self.identities.len(), self.index_builds)
    }

    /// Whether the buyer/recipe-port layout was prepared for these buildings, entrances and
    /// roads. A census pass can end before reseeding; after a building removal the previous
    /// layout then names moved or missing owners. O(buildings).
    pub(crate) fn layout_matches(
        &self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
    ) -> bool {
        self.revision
            == Some((
                allocator.building_ref_revision(),
                allocator.entrance_ref_revision(),
                network.cch_graph.build_generation,
            ))
            && self.profiles.iter().copied().eq(allocator
                .buildings
                .iter()
                .map(|building| building.economy_profile_runtime_id))
    }

    /// Counts potential input buyers and their maximum purchased units within existing freight
    /// components. This independent O(indexed input membership) census survives ordinary
    /// finance-account resets.
    pub(crate) fn prepare_possible_buyers(
        &mut self,
        possible: impl Fn(usize, u16) -> Option<f64> + Sync,
    ) {
        let identities = &self.identities;
        self.indices.live.par_iter_mut().for_each(|index| {
            index.reset_member_counts(|port| {
                let (owner, resource) = identities[port];
                possible(owner, resource)
            })
        });
    }

    /// Withdraws one counted input and visits freight components whose last buyer vanished.
    /// Cost is O(authored input ports + log components), with no route query or allocation.
    pub(crate) fn withdraw_possible_input(
        &mut self,
        buyer: usize,
        resource: u16,
        mut empty: impl FnMut(u32),
    ) {
        let Some(port) = self.buyer_ports[buyer]
            .clone()
            .find(|&port| self.identities[port].1 == resource)
        else {
            return;
        };
        for &component in self.port_access[port].components.as_slice() {
            if self.indices.live[resource as usize].remove_counted_member(component) {
                empty(component);
            }
        }
    }

    /// Conservative customer presence in either seller entrance component, in O(log components).
    /// Positive counts cannot establish directed reachability, purchases or financing.
    pub(crate) fn has_possible_buyers(&self, seller: usize, resource: u16) -> bool {
        self.indices.live[resource as usize].has_counted_members(self.access[seller].components)
    }

    /// Marks a counted input's unit totals stale, visiting each newly marked component.
    /// O(authored input ports + log components), with no route query or allocation.
    pub(crate) fn mark_possible_input(
        &mut self,
        buyer: usize,
        resource: u16,
        mut marked: impl FnMut(u32),
    ) {
        let Some(port) = self.buyer_ports[buyer]
            .clone()
            .find(|&port| self.identities[port].1 == resource)
        else {
            return;
        };
        for &component in self.port_access[port].components.as_slice() {
            if self.indices.live[resource as usize].mark_member_units(component) {
                marked(component);
            }
        }
    }

    /// Freshly sums counted buyers' maximum units in one resource/component, in
    /// O(log components + component membership).
    pub(crate) fn recount_possible_input(
        &mut self,
        resource: u16,
        component: u32,
        units: impl Fn(usize) -> Option<f64>,
    ) {
        let identities = &self.identities;
        self.indices.live[resource as usize]
            .recount_member_units(component, |port| units(identities[port].0));
    }

    /// Counted buyers and an upper bound on their purchases in either seller entrance
    /// component, in O(log components). A buyer reachable through both may count twice.
    pub(crate) fn possible_demand(&self, seller: usize, resource: u16) -> (usize, f64) {
        self.indices.live[resource as usize].counted_units(self.access[seller].components)
    }

    /// Account count and retained arena/mapping capacities for replacement scaling checks.
    #[cfg(test)]
    pub(crate) fn account_storage(&self) -> (usize, [usize; 3]) {
        (
            self.budgets.accounts.len(),
            [
                self.budgets.accounts.capacity(),
                self.account_ports.capacity(),
                self.account_available.capacity(),
            ],
        )
    }

    /// Forces the whole-proposal capacity retry in runtime transaction regressions.
    #[cfg(test)]
    pub(crate) fn discard_repricing_scratch_for_test(&mut self) {
        self.budgets.repricing = Default::default();
    }

    // Financial tranches retain their original order independently of fixed recipe-port IDs.
    // Mapping costs O(accounts * authored input ports per buyer), without spatial rebuilding.
    fn reset_accounts(&mut self) {
        self.account_ports
            .resize(self.budgets.accounts.len(), usize::MAX);
        self.account_available
            .resize(self.budgets.accounts.len(), false);
        self.port_available.resize(self.identities.len(), 0);
        self.port_available.fill(0);
        for (slot, account) in self.budgets.accounts.iter().enumerate() {
            let port = self.buyer_ports[account.buyer]
                .clone()
                .find(|&port| self.identities[port].1 == account.resource)
                .expect("financed input must belong to its buyer's prepared recipe");
            self.account_ports[slot] = port;
            let available = account.units > 0.0;
            self.account_available[slot] = available;
            self.port_available[port] += usize::from(available);
        }
        self.indices
            .all_mut()
            .for_each(|index| index.reset_available(|port| self.port_available[port] > 0));
        self.visited.resize(self.identities.len(), 0);
        self.reset_finance_masks();
    }

    /// Reserves actual source-to-buyer deliveries in stable chunk/account order.
    /// Weak road components prune candidates; every accepted offer has a directed freight
    /// quote and fits its buyer's shared profit/capital limits at the actual seller price.
    /// Cost follows visited bucket candidates and their route queries, not all businesses.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn claim_reachable(
        &mut self,
        entrance: &BuildingEntrance,
        seller: Option<usize>,
        resource: u16,
        requested: f64,
        price: f32,
        components: &ModeComponentIndex,
        allocator: &BuildingAllocator,
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        tuning: &RuntimeEconomyTuning,
    ) -> BusinessSales {
        self.claim_reachable_for(
            entrance, seller, resource, requested, price, components, allocator, logistics,
            network, graph, tuning, false,
        )
    }

    /// Stages a coupled replacement's customer quantities for later joint financing proof.
    /// Uses the same directed routes and prices, while allowing provisional buyer deficits.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn claim_reachable_joint(
        &mut self,
        entrance: &BuildingEntrance,
        seller: usize,
        resource: u16,
        requested: f64,
        price: f32,
        components: &ModeComponentIndex,
        allocator: &BuildingAllocator,
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        tuning: &RuntimeEconomyTuning,
    ) -> BusinessSales {
        self.claim_reachable_for(
            entrance,
            Some(seller),
            resource,
            requested,
            price,
            components,
            allocator,
            logistics,
            network,
            graph,
            tuning,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn claim_reachable_for(
        &mut self,
        entrance: &BuildingEntrance,
        seller: Option<usize>,
        resource: u16,
        requested: f64,
        price: f32,
        components: &ModeComponentIndex,
        allocator: &BuildingAllocator,
        logistics: &mut ShipmentSystem,
        network: &TransitNetwork,
        graph: &RegionGraph,
        tuning: &RuntimeEconomyTuning,
        joint: bool,
    ) -> BusinessSales {
        let mut sales = BusinessSales::default();
        let mask = if joint {
            None
        } else {
            self.finance_query_mask(resource, price)
        };
        if mask.is_some() {
            self.refresh_finance_masks();
        }
        let Some(live) = self.indices.live.get(resource as usize) else {
            return sales;
        };
        let site = self.indices.sites.get(resource as usize);
        if requested <= 0.0
            || !(live.has_available() || site.is_some_and(ReachableBucketIndex::has_available))
        {
            return sales;
        }
        let both = [live, site.unwrap_or(live)];
        let indices = &both[..1 + usize::from(site.is_some())];
        self.visit = self.visit.wrapping_add(1);
        if self.visit == 0 {
            self.visited.par_iter_mut().for_each(|visit| *visit = 0);
            self.visit = 1;
        }
        let reached = components.entrance_components(entrance, graph, TransitFlags::CAR);
        ReachableBucketIndex::scan_nearest_masked_over(
            indices,
            reached,
            entrance.door_pos.x,
            entrance.door_pos.y,
            mask,
            |event| {
                self.claim_work[0] += 1;
                let ReachableBucketScanEvent::Item { item_idx: port } = event else {
                    return true;
                };
                if self.visited[port] == self.visit {
                    return true;
                }
                self.visited[port] = self.visit;
                let buyer = self.identities[port].0;
                if Some(buyer) == seller || self.port_available[port] == 0 {
                    return true;
                }
                let destination = match self.port_sites[port] {
                    Some(site) => Some(&self.sites.entries[site].entrance),
                    None => allocator.entrances.get(buyer),
                };
                let Some(destination) = destination else {
                    return true;
                };
                // A buyer unable to take any units at a proven lower carrier price needs no
                // route quote: every actual route costs at least as much. O(buyer accounts).
                let minimum_carrier = logistics.customer_carrier_lower_bound(
                    entrance,
                    destination,
                    network,
                    graph,
                    tuning,
                );
                #[cfg(test)]
                {
                    self.quote_counts[0] += 1;
                }
                let mut slot = self.budgets.buyer_accounts[buyer];
                let mut claimable = false;
                while slot != usize::MAX && !claimable {
                    let account = self.budgets.accounts[slot];
                    claimable = self.account_ports[slot] == port
                        && account.units > 0.0
                        && self.budgets.claimable_at_carrier_lower_bound(
                            slot,
                            requested - sales.units,
                            price,
                            minimum_carrier,
                            !joint,
                        );
                    slot = account.next_buyer;
                }
                if !claimable {
                    #[cfg(test)]
                    {
                        self.quote_counts[1] += 1;
                        let mut slot = self.budgets.buyer_accounts[buyer];
                        while slot != usize::MAX {
                            let account = self.budgets.accounts[slot];
                            if self.account_ports[slot] == port && account.units > 0.0 {
                                let remaining = requested - sales.units;
                                let free = self.budgets.claimable_at_carrier_lower_bound(
                                    slot, remaining, price, 0.0, !joint,
                                );
                                let finance = self.budgets.finance[buyer];
                                let cause = if account.units - remaining == account.units {
                                    1
                                } else if !joint && !(finance[0] > 0.0 && finance[1] >= 0.0) {
                                    2
                                } else if !free {
                                    3
                                } else {
                                    4
                                };
                                self.rejection_causes[0] += 1;
                                self.rejection_causes[cause] += 1;
                                if free {
                                    self.quote_counts[2] += 1;
                                    break;
                                }
                            }
                            slot = account.next_buyer;
                        }
                    }
                    return true;
                }
                self.claim_work[1] += 1;
                if let Some(carrier) = logistics.customer_carrier_quote(
                    entrance,
                    destination,
                    seller,
                    self.port_sites[port].is_none().then_some(buyer),
                    allocator,
                    network,
                    graph,
                    tuning,
                ) {
                    let mut slot = self.budgets.buyer_accounts[buyer];
                    while slot != usize::MAX && sales.units < requested {
                        let account = self.budgets.accounts[slot];
                        if self.account_ports[slot] == port && account.units > 0.0 {
                            let units = if joint {
                                self.budgets.claim_joint_opportunity(
                                    slot,
                                    requested - sales.units,
                                    price,
                                    carrier,
                                )
                            } else {
                                self.budgets.claim_opportunity(
                                    slot,
                                    requested - sales.units,
                                    price,
                                    carrier,
                                )
                            };
                            sales.units += units;
                            sales.revenue += units * f64::from(price * account.goods_multiplier);
                        }
                        slot = account.next_buyer;
                    }
                }
                sales.units < requested
            },
        );
        self.sync_staged(false);
        sales
    }

    fn sync_staged(&mut self, previous: bool) {
        for &(buyer, _) in &self.budgets.staged_finance {
            finance_masks::mark_dirty(&mut self.dirty_buyers, &mut self.buyer_dirty, buyer);
        }
        for change in &self.budgets.staged {
            let account = self.budgets.accounts[change.account];
            let access = self.port_access[self.account_ports[change.account]];
            let available = if previous {
                change.previous_units
            } else {
                account.units
            } > 0.0;
            sync_account(
                &mut self.indices,
                access,
                &self.account_ports,
                &mut self.account_available,
                &mut self.port_available,
                change.account,
                account.resource,
                available,
            );
        }
    }

    /// Releases only this seller's receipts while retaining exact pre-probe snapshots.
    pub(crate) fn release_owner(&mut self, seller: usize) {
        self.reject();
        self.budgets.release_owner(seller);
        self.sync_staged(false);
    }

    /// Withdraws one buyer inside an outer proof and reports suppliers whose sales change.
    /// Only this buyer's account buckets and current receipts are visited; its old operation
    /// cannot remain advertised as demand while a replacement recipe is being evaluated.
    pub(crate) fn withdraw_buyer(
        &mut self,
        buyer: usize,
        affected: impl FnMut(usize, u16, f64, f64),
    ) {
        self.mark_finance_dirty(buyer);
        let mut slot = self.budgets.buyer_accounts[buyer];
        while slot != usize::MAX {
            let account = self.budgets.accounts[slot];
            sync_account(
                &mut self.indices,
                self.port_access[self.account_ports[slot]],
                &self.account_ports,
                &mut self.account_available,
                &mut self.port_available,
                slot,
                account.resource,
                false,
            );
            slot = account.next_buyer;
        }
        self.budgets.withdraw_buyer(buyer, affected);
    }

    /// Replaces a buyer's financed input tranches inside an enclosing operation proof.
    /// The recipe/door must already be prepared. `seed` adds at most `input_tranches` rows
    /// for this buyer, including source-owned receipts from its evaluated input quote.
    /// Exhaustion leaves the book untouched; grow and replay the complete proof before retry.
    /// Cost follows this buyer's old/new tranches and receipts, with no spatial rebuild.
    /// Test harness for the shared reseed step: runtime replacements must preserve earlier
    /// seller promises and therefore use `reprice_buyer_inputs`/`seed_new_buyer_inputs`.
    #[cfg(test)]
    pub(crate) fn replace_buyer_inputs(
        &mut self,
        buyer: usize,
        headroom: [f64; 2],
        input_tranches: usize,
        seed: impl FnOnce(&mut BusinessBudgets),
        affected: impl FnMut(usize, u16, f64, f64),
    ) -> Result<(), usize> {
        self.replace_inputs_at(buyer, None, headroom, input_tranches, seed, affected)
    }

    /// Seeds a newly financed buyer inside the enclosing operation transaction. An empty
    /// account chain proves there are no earlier purchases to reconcile. Preflight bounds
    /// every account/receipt write; cost is O(input tranches + their prepared port lookups).
    pub(crate) fn seed_new_buyer_inputs(
        &mut self,
        buyer: usize,
        site: Option<usize>,
        headroom: [f64; 2],
        input_tranches: usize,
        seed: impl FnOnce(&mut BusinessBudgets),
    ) -> Result<(), [usize; 2]> {
        assert_eq!(
            self.budgets.buyer_accounts[buyer],
            usize::MAX,
            "new buyer publication cannot discard established input accounts"
        );
        self.budgets
            .check_replacement_capacity(buyer, input_tranches)
            .map_err(|size| [size, 0])?;
        self.check_input_seed_receipts(input_tranches, 0)
            .map_err(|size| [0, size])?;
        self.replace_inputs_at(buyer, site, headroom, input_tranches, seed, |_, _, _, _| {})
            .map_err(|size| [size, 0])
    }

    // Each input tranche can seed at most one receipt. Withdrawn buyer cells and existing
    // free cells are reusable; preflight never changes financial state or grows the arena.
    fn check_input_seed_receipts(
        &self,
        input_tranches: usize,
        mut reusable: usize,
    ) -> Result<(), usize> {
        let mut free = self.budgets.free_receipt;
        while free != usize::MAX && reusable < input_tranches {
            reusable += 1;
            free = self.budgets.committed[free].next;
        }
        let required = self.budgets.committed.len() + input_tranches.saturating_sub(reusable);
        if required > self.budgets.committed.capacity() {
            Err(required)
        } else {
            Ok(())
        }
    }

    /// Reprices an affected buyer while preserving every established supplier promise.
    /// `headroom` is the new physical-input proof before outstanding future-price premiums.
    /// The outer proof must validate the final finances together with changed seller revenues.
    /// Returns false for an incompatible prior commitment, or required [accounts, receipts]
    /// storage for rollback/growth/replay. Neither failure permits publishing the partial seed.
    /// Work is O((buyer receipts + tranches) log (buyer receipts + tranches) + new fragments),
    /// with prepared scratch and no city scan or spatial-index rebuild.
    pub(crate) fn reprice_buyer_inputs(
        &mut self,
        buyer: usize,
        site: Option<usize>,
        headroom: [f64; 2],
        input_tranches: usize,
        seed: impl FnOnce(&mut BusinessBudgets),
        affected: impl FnMut(usize, u16, [f64; 2], [f64; 2]),
    ) -> Result<bool, [usize; 2]> {
        self.budgets
            .check_replacement_capacity(buyer, input_tranches)
            .map_err(|size| [size, 0])?;
        if self.budgets.repricing_capacity() < self.budgets.committed.capacity() {
            return Err([0, self.budgets.committed.capacity()]);
        }
        self.budgets
            .remember_purchases(buyer)
            .map_err(|size| [0, size])?;
        self.check_input_seed_receipts(input_tranches, self.budgets.repricing_previous_count())
            .map_err(|size| [0, size])?;
        self.replace_inputs_at(buyer, site, headroom, input_tranches, seed, |_, _, _, _| {})
            .map_err(|size| [size, 0])?;
        let complete = self
            .budgets
            .restore_purchases(buyer)
            .map_err(|size| [0, size])?;
        let mut slot = self.budgets.buyer_accounts[buyer];
        while slot != usize::MAX {
            let account = self.budgets.accounts[slot];
            sync_account(
                &mut self.indices,
                self.port_access[self.account_ports[slot]],
                &self.account_ports,
                &mut self.account_available,
                &mut self.port_available,
                slot,
                account.resource,
                account.units > 0.0,
            );
            slot = account.next_buyer;
        }
        if complete {
            self.budgets.visit_repriced_sellers(buyer, affected);
        }
        Ok(complete)
    }

    fn replace_inputs_at(
        &mut self,
        buyer: usize,
        site: Option<usize>,
        headroom: [f64; 2],
        input_tranches: usize,
        seed: impl FnOnce(&mut BusinessBudgets),
        affected: impl FnMut(usize, u16, f64, f64),
    ) -> Result<(), usize> {
        debug_assert!(self.budgets.revalidation.is_active() && self.budgets.staged.is_empty());
        self.budgets
            .check_replacement_capacity(buyer, input_tranches)?;
        self.withdraw_buyer(buyer, affected);
        self.budgets.set_finance(buyer, headroom);
        self.budgets.replace_inputs(buyer, input_tranches, seed);
        self.account_ports
            .resize(self.budgets.accounts.len(), usize::MAX);
        self.account_available
            .resize(self.budgets.accounts.len(), false);
        let mut slot = self.budgets.buyer_accounts[buyer];
        while slot != usize::MAX {
            let account = self.budgets.accounts[slot];
            assert_eq!(account.buyer, buyer);
            let mut range = match site {
                Some(site) => self.sites.ports[site].clone(),
                None => self.buyer_ports[buyer].clone(),
            };
            let port = range.find(|&port| self.identities[port].1 == account.resource);
            // Unused peak-capacity rows may retain a resource absent from the new recipe.
            if let Some(port) = port {
                self.set_account_port(slot, port);
            } else {
                assert!(account.units == 0.0 && account.first_receipt == usize::MAX);
            }
            sync_account(
                &mut self.indices,
                self.port_access[self.account_ports[slot]],
                &self.account_ports,
                &mut self.account_available,
                &mut self.port_available,
                slot,
                account.resource,
                account.units > 0.0,
            );
            slot = account.next_buyer;
        }
        Ok(())
    }

    /// Commits one seller's proven operation and keeps its customer receipts for replacement.
    /// Test harness: runtime proofs commit through `try_extend_for`/`commit_joint_for`.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn accept_for(&mut self, seller: usize) -> bool {
        self.budgets.accept_for(seller)
    }

    /// Keeps a replacement quote provisional until all affected finances pass together.
    pub(crate) fn commit_joint_for(&mut self, seller: usize) -> Result<(), usize> {
        self.budgets.commit_joint_for(seller)
    }

    /// Restores both financial accounts and bucket eligibility after a rejected operation.
    pub(crate) fn reject(&mut self) {
        self.sync_staged(true);
        self.budgets.reject();
    }

    /// Live-book state a later census or pass reads, for exact pass-rollback checks. Finance
    /// masks are a conservative pruning cache: claims also test availability and exact
    /// affordability, so a wider mask only adds rejected visits. The check requires the
    /// refreshed live masks to contain every exact bit, then fingerprints the exact masks.
    /// Proposed-site rows and scan scratch are planner-owned and excluded.
    #[cfg(test)]
    pub(crate) fn live_fingerprint_for_test(&self) -> Vec<(&'static str, u64)> {
        let mut market = self.clone();
        market.refresh_finance_masks();
        let cached = market.port_masks.clone();
        market.reset_finance_masks();
        assert!(
            cached
                .iter()
                .zip(&market.port_masks)
                .all(|(cached, exact)| cached & exact == *exact),
            "finance masks may be wider than exact, never narrower"
        );
        let live = market.indices.live_ports;
        let b = &market.budgets;
        crate::debug_fingerprint!("accounts" => b.accounts, "buyer_accounts" => b.buyer_accounts,
            "buyer_tails" => b.buyer_tails, "finance" => b.finance, "committed" => b.committed,
            "owners" => b.owners, "free_receipt" => b.free_receipt, "staged_slots" => b.staged_slots,
            "finance_staged" => b.finance_staged, "access" => market.access, "profiles" => market.profiles,
            "identities" => market.identities[..live], "port_access" => market.port_access[..live],
            "buyer_ports" => market.buyer_ports, "account_ports" => market.account_ports,
            "account_available" => market.account_available, "port_available" => market.port_available[..live],
            "port_masks" => market.port_masks[..live], "indices" => market.indices.live,
            "revision" => market.revision)
    }

    /// Lays out buyer ports for the buildings appended since this book's census, for a
    /// planning pass on the lent book (`SIM-02-I`). Their ports follow the census's live
    /// ports, as a census of the whole city would number them, and hold no accounts; the live
    /// index is rebuilt with them, keeping the census's availability and finance classes, and
    /// the proposed sites are laid out again after them. The census's index waits aside until
    /// [`Self::restore_census_buyers`]. O(live ports log live ports + appended buyers) when
    /// buildings were appended, O(1) otherwise.
    pub(crate) fn append_live_buyers(
        &mut self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
        catalog: &RuntimeEconomyCatalog,
    ) {
        let buyers = self.buyer_ports.len();
        if allocator.buildings.len() <= buyers || self.revision.is_none() {
            return;
        }
        debug_assert!(self.census_buyers.is_none());
        let live_ports = self.indices.live_ports;
        // Proposed sites are numbered after the live ports.
        self.identities.truncate(live_ports);
        self.port_access.truncate(live_ports);
        self.port_sites.truncate(live_ports);
        self.port_available.truncate(live_ports);
        self.port_masks.truncate(live_ports);
        self.sites.invalidate_layout();
        self.indices.sites.clear();
        for (buyer, building) in allocator.buildings.iter().enumerate().skip(buyers) {
            let access = Access {
                chunk: allocator.entrances.get(buyer).map_or((0, 0), |entrance| {
                    chunk_for_point(entrance.door_pos.x, entrance.door_pos.y)
                }),
                components: components.building_components(
                    allocator,
                    graph,
                    buyer,
                    TransitFlags::CAR,
                ),
            };
            self.access.push(access);
            self.profiles.push(building.economy_profile_runtime_id);
            let first = self.identities.len();
            if let Some(profile) =
                catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            {
                for input in &profile.inputs {
                    self.identities.push((buyer, input.resource_runtime_id));
                    self.port_access.push(access);
                }
            }
            self.buyer_ports.push(first..self.identities.len());
        }
        let ports = self.identities.len();
        self.port_sites.resize(ports, None);
        self.port_available.resize(ports, 0);
        self.port_masks.resize(ports, 0);
        self.visited.resize(ports, 0);
        self.indices.live_ports = ports;
        let census = std::mem::take(&mut self.indices.live);
        self.rebuild_indices(catalog.resource_count() + 1, false);
        let (available, masks) = (&self.port_available, &self.port_masks);
        self.indices.live.par_iter_mut().for_each(|index| {
            index.reset_available(|port| available[port] > 0);
            index.reset_query_masks(|port| masks[port]);
        });
        self.census_buyers = Some(CensusBuyers {
            buyers,
            live_ports,
            live: census,
        });
    }

    /// Returns the census's layout after a pass that covered appended buildings: the
    /// appended buyers and their ports leave, the census's live index returns, and the
    /// proposed sites are laid out again at the next pass. Call after [`Self::end_pass`].
    /// O(appended buyers + site ports).
    pub(crate) fn restore_census_buyers(&mut self) {
        let Some(census) = self.census_buyers.take() else {
            return;
        };
        self.identities.truncate(census.live_ports);
        self.port_access.truncate(census.live_ports);
        self.port_sites.truncate(census.live_ports);
        self.port_available.truncate(census.live_ports);
        self.port_masks.truncate(census.live_ports);
        self.visited.truncate(census.live_ports);
        self.access.truncate(census.buyers);
        self.profiles.truncate(census.buyers);
        self.buyer_ports.truncate(census.buyers);
        self.sites.invalidate_layout();
        self.indices.sites.clear();
        self.indices.live = census.live;
        self.indices.live_ports = census.live_ports;
    }

    /// Opens a planning pass directly on this live market. Proposals nest ordinary proofs
    /// inside it; [`Self::end_pass`] restores the live book bit for bit. O(1) amortized.
    pub(crate) fn begin_pass(&mut self) {
        self.budgets.begin_pass();
        let pass = &mut self.pass_ports;
        pass.generation = pass.generation.wrapping_add(1);
        if pass.generation == 0 {
            pass.seen.fill(0);
            pass.generation = 1;
        }
        pass.seen.resize(self.account_ports.len(), 0);
        pass.checkpoint = Some(self.account_ports.len());
    }

    /// Rejects any open quote or proof, then restores every account, buyer, owner, receipt
    /// and port mapping the pass changed, with exactly their bucket eligibility. Appended
    /// project rows are truncated; proposed-site geometry stays prepared. O(touched rows).
    pub(crate) fn end_pass(&mut self) {
        self.reject_revalidation();
        self.reject();
        let Some((accounts, buyers)) = self.budgets.pass_extent() else {
            return;
        };
        let dirty = &mut self.dirty_buyers;
        let flags = &mut self.buyer_dirty;
        self.budgets.visit_pass_buyers(|buyer| {
            if buyer < buyers {
                finance_masks::mark_dirty(dirty, flags, buyer)
            }
        });
        let access = &self.port_access;
        let indices = &mut self.indices;
        let ports = &mut self.account_ports;
        let available = &mut self.account_available;
        let counts = &mut self.port_available;
        // Re-pointed tranches leave their pass port before returning to the original one.
        for &(slot, port) in &self.pass_ports.saved {
            let resource = self.budgets.accounts[slot].resource;
            sync_account(
                indices,
                access[ports[slot]],
                ports,
                available,
                counts,
                slot,
                resource,
                false,
            );
            ports[slot] = port;
            if !self.budgets.pass_saved_account(slot) {
                let account = self.budgets.accounts[slot];
                sync_account(
                    indices,
                    access[port],
                    ports,
                    available,
                    counts,
                    slot,
                    account.resource,
                    account.units > 0.0,
                );
            }
        }
        self.budgets.visit_pass_accounts(|slot, account| {
            sync_account(
                indices,
                access[ports[slot]],
                ports,
                available,
                counts,
                slot,
                account.resource,
                account.units > 0.0,
            );
        });
        self.budgets.end_pass();
        self.account_ports.truncate(accounts);
        self.account_available.truncate(accounts);
        self.account_masks.truncate(accounts);
        self.pass_ports.saved.clear();
        self.pass_ports.checkpoint = None;
        self.dirty_buyers.retain(|&buyer| buyer < buyers);
        self.buyer_dirty.truncate(buyers);
    }
}

/// First-touch account-to-port mappings changed by a planning pass.
#[derive(Clone, Debug, Default)]
struct PassPorts {
    checkpoint: Option<usize>,
    generation: u64,
    seen: Vec<u64>,
    saved: Vec<(usize, usize)>,
}

/// Live buyer ports and proposed-site ports keep separate bucket indices. Site ports are
/// numbered after every live port, so scanning live then site buckets reproduces the order
/// of one merged index; site preparation never rebuilds the live index.
#[derive(Clone, Debug, Default)]
struct PortIndices {
    live: Vec<ReachableBucketIndex>,
    sites: Vec<ReachableBucketIndex>,
    // First proposed-site port.
    live_ports: usize,
}

impl PortIndices {
    fn port_mut(&mut self, resource: u16, port: usize) -> &mut ReachableBucketIndex {
        if port < self.live_ports {
            &mut self.live[resource as usize]
        } else {
            &mut self.sites[resource as usize]
        }
    }

    fn all_mut(&mut self) -> impl ParallelIterator<Item = &mut ReachableBucketIndex> {
        self.live.par_iter_mut().chain(self.sites.par_iter_mut())
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_account(
    indices: &mut PortIndices,
    access: Access,
    ports: &[usize],
    accounts: &mut [bool],
    counts: &mut [usize],
    slot: usize,
    resource: u16,
    available: bool,
) {
    if accounts[slot] == available {
        return;
    }
    accounts[slot] = available;
    let port = ports[slot];
    let previous = counts[port] > 0;
    if available {
        counts[port] += 1;
    } else {
        counts[port] -= 1;
    }
    let available = counts[port] > 0;
    if previous == available {
        return;
    }
    let index = indices.port_mut(resource, port);
    for &component in access.components.as_slice() {
        index.set_available(
            ReachableBucketEntry::new(component, access.chunk, port),
            available,
        );
    }
}
