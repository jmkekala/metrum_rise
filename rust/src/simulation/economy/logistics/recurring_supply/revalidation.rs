// SPDX-License-Identifier: GPL-2.0-only

//! Atomic source/input revalidation across several locally committed owner quotes.

use super::{Claim, RecurringSupply};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;

#[derive(Clone, Copy, Debug)]
struct Source {
    slot: usize,
    source: usize,
    resource: u16,
    remaining: f64,
    production: f64,
    active: bool,
    first_receipt: usize,
}

/// First-touch undo storage for an outer operation; inner owner quotes keep their existing
/// retry journal. Capacities are prepared once per project pass, never per replacement.
#[derive(Clone, Debug, Default)]
pub(super) struct Revalidation {
    checkpoint: Option<(usize, usize)>,
    generation: u64,
    source_seen: Vec<u64>,
    owner_seen: Vec<u64>,
    receipt_seen: Vec<u64>,
    sources: Vec<Source>,
    owners: Vec<(usize, usize, usize)>,
    receipts: Vec<(usize, Claim)>,
    pass: Pass,
}

/// First-touch snapshots of a whole planning pass over the live book. Proposal proofs nest
/// inside it; projected source slots, temporary owners and appended receipt cells are
/// truncated instead of saved.
#[derive(Clone, Debug, Default)]
struct Pass {
    checkpoint: Option<PassCheckpoint>,
    generation: u64,
    source_seen: Vec<u64>,
    owner_seen: Vec<u64>,
    receipt_seen: Vec<u64>,
    sources: Vec<Source>,
    owners: Vec<(usize, usize, usize)>,
    receipts: Vec<(usize, Claim)>,
    // Search bounds are hints, but the live book returns them exactly as it lent them.
    nearest_incumbents: Vec<Option<usize>>,
}

#[derive(Clone, Copy, Debug)]
struct PassCheckpoint {
    receipts: usize,
    free_receipt: usize,
    slots: usize,
    owners: usize,
    // A lent book covers the buildings appended since its census for the pass only.
    source_base: usize,
}

impl Revalidation {
    /// Whether an outer operation still owns provisional inner commits.
    pub(super) fn is_active(&self) -> bool {
        self.checkpoint.is_some()
    }

    /// Prepares at most one undo entry per original source, owner and receipt arena slot.
    pub(super) fn prepare(&mut self, sources: usize, owners: usize, receipts: usize) {
        debug_assert!(self.checkpoint.is_none());
        // Rows that follow the book's row counts keep spare capacity (`SIM-02-K`).
        crate::utils::storage::resize_rows(&mut self.source_seen, sources, 0);
        crate::utils::storage::resize_rows(&mut self.owner_seen, owners, 0);
        self.receipt_seen.resize(receipts, 0);
        crate::utils::storage::reserve_rows(&mut self.sources, sources);
        crate::utils::storage::reserve_rows(&mut self.owners, owners);
        self.receipts
            .reserve(receipts.saturating_sub(self.receipts.len()));
    }

    /// Mirrors growth of the indexed receipt arena without discarding any active snapshots.
    pub(super) fn follow_receipts(&mut self, capacity: usize) {
        if !self.receipt_seen.is_empty() && capacity > self.receipt_seen.len() {
            self.receipt_seen.resize(capacity, 0);
            self.receipts
                .reserve(capacity.saturating_sub(self.receipts.len()));
        }
    }

    fn finish(&mut self) {
        self.checkpoint = None;
        self.sources.clear();
        self.owners.clear();
        self.receipts.clear();
    }
}

impl RecurringSupply {
    /// Starts one outer replacement/revalidation transaction in O(1). Accepted inner owner
    /// quotes remain provisional until `accept_revalidation`; one rejection restores them all.
    /// Call `prepare_owners` before the project pass to provide bounded journal storage.
    pub(crate) fn begin_revalidation(&mut self) {
        debug_assert!(self.staged.is_empty() && self.released.is_empty());
        let journal = &mut self.revalidation;
        assert!(
            journal.checkpoint.is_none(),
            "only owner quotes may nest inside revalidation"
        );
        debug_assert!(journal.source_seen.len() >= self.remaining.len());
        debug_assert!(journal.owner_seen.len() >= self.owner_receipts.len());
        debug_assert!(journal.receipt_seen.len() >= self.committed.len());
        journal.generation = journal.generation.wrapping_add(1);
        if journal.generation == 0 {
            journal.source_seen.fill(0);
            journal.owner_seen.fill(0);
            journal.receipt_seen.fill(0);
            journal.generation = 1;
        }
        journal.checkpoint = Some((self.committed.len(), self.free_receipt));
    }

    /// Publishes all accepted inner quotes together. This is also valid when an ordinary
    /// new project has no outer replacement transaction. No receipt history is retained.
    pub(crate) fn accept_revalidation(&mut self) {
        debug_assert!(self.staged.is_empty() && self.released.is_empty());
        self.revalidation.finish();
    }

    /// Rejects the active inner quote, then restores exact source balances and both directions
    /// of every touched receipt link. Work follows changed sources/owners/receipts, plus bucket
    /// updates; unrelated buildings are neither copied nor scanned. Arena growth is truncated.
    pub(crate) fn reject_revalidation(
        &mut self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) {
        self.reject(allocator, graph);
        let Some((len, free)) = self.revalidation.checkpoint.take() else {
            return;
        };
        while let Some(source) = self.revalidation.sources.pop() {
            self.remaining[source.slot] = source.remaining;
            self.production[source.slot] = source.production;
            self.source_receipts[source.slot] = source.first_receipt;
            if self.active[source.slot] != source.active {
                self.active[source.slot] = source.active;
                self.set_available(
                    source.source,
                    source.resource,
                    source.active,
                    allocator,
                    graph,
                );
            }
        }
        for (owner, first, last) in self.revalidation.owners.drain(..) {
            self.owner_receipts[owner] = first;
            self.owner_tails[owner] = last;
        }
        for (index, receipt) in self.revalidation.receipts.drain(..) {
            self.committed[index] = receipt;
        }
        self.committed.truncate(len);
        self.free_receipt = free;
    }

    /// Captures a source's balance and reverse receipt head before their first outer mutation.
    pub(super) fn journal_source(&mut self, source: usize, resource: u16, slot: usize) {
        let pass = &mut self.revalidation.pass;
        if let Some(checkpoint) = pass.checkpoint
            && slot < checkpoint.slots
            && pass.source_seen[slot] != pass.generation
        {
            pass.source_seen[slot] = pass.generation;
            pass.sources.push(Source {
                slot,
                source,
                resource,
                remaining: self.remaining[slot],
                production: self.production[slot],
                active: self.active[slot],
                first_receipt: self.source_receipts[slot],
            });
        }
        let journal = &mut self.revalidation;
        if journal.checkpoint.is_none() || journal.source_seen[slot] == journal.generation {
            return;
        }
        journal.source_seen[slot] = journal.generation;
        journal.sources.push(Source {
            slot,
            source,
            resource,
            remaining: self.remaining[slot],
            production: self.production[slot],
            active: self.active[slot],
            first_receipt: self.source_receipts[slot],
        });
    }

    /// Captures an owner's input-chain endpoints before retirement or extension.
    pub(super) fn journal_owner(&mut self, owner: usize) {
        let pass = &mut self.revalidation.pass;
        if let Some(checkpoint) = pass.checkpoint
            && owner < checkpoint.owners
            && pass.owner_seen[owner] != pass.generation
        {
            pass.owner_seen[owner] = pass.generation;
            pass.owners
                .push((owner, self.owner_receipts[owner], self.owner_tails[owner]));
        }
        let journal = &mut self.revalidation;
        if journal.checkpoint.is_none() || journal.owner_seen[owner] == journal.generation {
            return;
        }
        journal.owner_seen[owner] = journal.generation;
        journal
            .owners
            .push((owner, self.owner_receipts[owner], self.owner_tails[owner]));
    }

    /// Captures a pre-existing arena cell before recycling or editing either receipt link.
    /// Cells appended inside this transaction need no snapshot: rejection truncates them.
    pub(super) fn journal_receipt(&mut self, index: usize) {
        let pass = &mut self.revalidation.pass;
        if let Some(checkpoint) = pass.checkpoint
            && index < checkpoint.receipts
            && pass.receipt_seen[index] != pass.generation
        {
            pass.receipt_seen[index] = pass.generation;
            pass.receipts.push((index, self.committed[index]));
        }
        let journal = &mut self.revalidation;
        let Some((len, _)) = journal.checkpoint else {
            return;
        };
        if index >= len || journal.receipt_seen[index] == journal.generation {
            return;
        }
        journal.receipt_seen[index] = journal.generation;
        journal.receipts.push((index, self.committed[index]));
    }

    /// Live-book state a later census or pass reads, for exact pass-rollback checks; the
    /// planner's projected sources and quote scratch are excluded.
    #[cfg(test)]
    pub(crate) fn live_fingerprint_for_test(&self) -> Vec<(&'static str, u64)> {
        crate::debug_fingerprint!("offsets" => self.offsets, "active" => self.active, "revision" => self.revision,
            "remaining" => self.remaining, "production" => self.production, "committed" => self.committed,
            "owner_receipts" => self.owner_receipts, "owner_tails" => self.owner_tails,
            "source_receipts" => self.source_receipts, "free_receipt" => self.free_receipt,
            "nearest_incumbents" => self.nearest_incumbents, "doors" => self.doors, "index" => self.index,
            "max_speed" => self.max_speed.to_bits(), "source_base" => self.source_base)
    }

    /// Opens a planning pass over the live book in O(resources) amortized: snapshot storage
    /// follows the live row counts and is retained across passes.
    pub(crate) fn begin_pass(&mut self) {
        debug_assert!(
            self.staged.is_empty() && self.released.is_empty() && !self.revalidation.is_active()
        );
        debug_assert!(!self.projected_enabled());
        let pass = &mut self.revalidation.pass;
        assert!(pass.checkpoint.is_none(), "planning passes do not nest");
        pass.generation = pass.generation.wrapping_add(1);
        if pass.generation == 0 {
            pass.source_seen.fill(0);
            pass.owner_seen.fill(0);
            pass.receipt_seen.fill(0);
            pass.generation = 1;
        }
        // Journals follow the book's row counts, with spare capacity (`SIM-02-K`).
        crate::utils::storage::resize_rows(&mut pass.source_seen, self.remaining.len(), 0);
        crate::utils::storage::resize_rows(&mut pass.owner_seen, self.owner_receipts.len(), 0);
        crate::utils::storage::resize_rows(&mut pass.receipt_seen, self.committed.len(), 0);
        pass.nearest_incumbents.clone_from(&self.nearest_incumbents);
        pass.checkpoint = Some(PassCheckpoint {
            receipts: self.committed.len(),
            free_receipt: self.free_receipt,
            slots: self.remaining.len(),
            owners: self.owner_receipts.len(),
            source_base: self.source_base,
        });
    }

    /// Rejects any open proof, then restores every pre-pass source, owner chain and receipt
    /// cell the pass changed, with their bucket eligibility. Projected slots, temporary
    /// owners and appended cells are truncated. O(touched rows + resources).
    pub(crate) fn end_pass(&mut self, allocator: &BuildingAllocator, graph: &RegionGraph) {
        self.reject_revalidation(allocator, graph);
        self.reject(allocator, graph);
        let Some(checkpoint) = self.revalidation.pass.checkpoint.take() else {
            return;
        };
        while let Some(source) = self.revalidation.pass.sources.pop() {
            self.remaining[source.slot] = source.remaining;
            self.production[source.slot] = source.production;
            self.source_receipts[source.slot] = source.first_receipt;
            if self.active[source.slot] != source.active {
                self.active[source.slot] = source.active;
                self.set_available(
                    source.source,
                    source.resource,
                    source.active,
                    allocator,
                    graph,
                );
            }
        }
        let pass = &mut self.revalidation.pass;
        for (owner, first, last) in pass.owners.drain(..) {
            self.owner_receipts[owner] = first;
            self.owner_tails[owner] = last;
        }
        for (index, receipt) in pass.receipts.drain(..) {
            self.committed[index] = receipt;
        }
        self.nearest_incumbents.clone_from(&pass.nearest_incumbents);
        self.committed.truncate(checkpoint.receipts);
        self.free_receipt = checkpoint.free_receipt;
        self.source_receipts.truncate(checkpoint.slots);
        self.remaining.truncate(checkpoint.slots);
        self.production.truncate(checkpoint.slots);
        self.active.truncate(checkpoint.slots);
        self.owner_receipts.truncate(checkpoint.owners);
        self.owner_tails.truncate(checkpoint.owners);
        self.source_base = checkpoint.source_base;
        self.clear_projected_sources();
        self.prices.clear();
    }

    /// Retained journal capacities for regression checks; never used to decide feasibility.
    #[cfg(test)]
    pub(crate) fn revalidation_storage(&self) -> [usize; 3] {
        [
            self.revalidation.sources.capacity(),
            self.revalidation.owners.capacity(),
            self.revalidation.receipts.capacity(),
        ]
    }
}
