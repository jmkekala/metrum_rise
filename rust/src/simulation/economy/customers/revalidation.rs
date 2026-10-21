// SPDX-License-Identifier: GPL-2.0-only

//! Household reservation rollback across several accepted owner changes.

use super::{ConsumerBudgets, Funds, Receipt};

/// First-touch snapshots keep an enclosing operation separate from inner customer quotes.
#[derive(Clone, Debug, Default)]
pub(super) struct Revalidation {
    checkpoint: Option<(usize, usize)>,
    generation: u64,
    funds_seen: Vec<u64>,
    owner_seen: Vec<u64>,
    receipt_seen: Vec<u64>,
    funds: Vec<(usize, Funds)>,
    owners: Vec<(usize, usize)>,
    receipts: Vec<(usize, Receipt)>,
    pass: Pass,
}

/// First-touch snapshots of a whole planning pass over the live book. Proposal proofs nest
/// inside it; temporary project owners and appended receipt cells are truncated instead.
#[derive(Clone, Debug, Default)]
struct Pass {
    // Receipt count, free head and owner count before the pass.
    checkpoint: Option<(usize, usize, usize)>,
    generation: u64,
    funds_seen: Vec<u64>,
    owner_seen: Vec<u64>,
    receipt_seen: Vec<u64>,
    funds: Vec<(usize, Funds)>,
    owners: Vec<(usize, usize)>,
    receipts: Vec<(usize, Receipt)>,
}

impl Revalidation {
    /// Whether accepted inner quotes still belong to an unpublished operation proof.
    pub(super) fn is_active(&self) -> bool {
        self.checkpoint.is_some()
    }

    /// Follows receipt-arena growth without losing original cells. Preparing all receipt
    /// capacity between proofs remains required; growth here must not corrupt rollback.
    pub(super) fn follow_receipts(&mut self, capacity: usize) {
        if !self.receipt_seen.is_empty() && capacity > self.receipt_seen.len() {
            self.receipt_seen.resize(capacity, 0);
            self.receipts
                .reserve(capacity.saturating_sub(self.receipts.len()));
        }
    }
}

impl ConsumerBudgets {
    /// Prepares journals once before project quotation, retaining warm storage.
    pub(super) fn prepare_revalidation(&mut self) {
        debug_assert!(!self.revalidation.is_active());
        let journal = &mut self.revalidation;
        // Rows that follow the book's row counts keep spare capacity (`SIM-02-K`).
        crate::utils::storage::resize_rows(&mut journal.funds_seen, self.funds.len(), 0);
        crate::utils::storage::resize_rows(&mut journal.owner_seen, self.owners.len(), 0);
        journal.receipt_seen.resize(self.committed.capacity(), 0);
        crate::utils::storage::reserve_rows(&mut journal.funds, self.funds.len());
        crate::utils::storage::reserve_rows(&mut journal.owners, self.owners.len());
        journal.receipts.reserve(
            self.committed
                .capacity()
                .saturating_sub(journal.receipts.len()),
        );
    }

    /// Starts an enclosing operation in O(1), without copying household or owner rows.
    pub(super) fn begin_revalidation(&mut self) {
        debug_assert!(self.staged.is_empty());
        let journal = &mut self.revalidation;
        assert!(
            journal.checkpoint.is_none(),
            "only customer quotes may nest inside revalidation"
        );
        journal.generation = journal.generation.wrapping_add(1);
        if journal.generation == 0 {
            journal.funds_seen.fill(0);
            journal.owner_seen.fill(0);
            journal.receipt_seen.fill(0);
            journal.generation = 1;
        }
        journal.checkpoint = Some((self.committed.len(), self.free_receipt));
    }

    /// Publishes every accepted inner owner change together, discarding touched-row snapshots.
    pub(super) fn accept_revalidation(&mut self) {
        debug_assert!(self.staged.is_empty());
        let journal = &mut self.revalidation;
        journal.checkpoint = None;
        journal.funds.clear();
        journal.owners.clear();
        journal.receipts.clear();
    }

    /// Visits original account values so routing restores only changed bucket entries.
    pub(super) fn visit_revalidation_funds(&self, mut visit: impl FnMut(usize, Funds)) {
        for &(slot, funds) in &self.revalidation.funds {
            visit(slot, funds);
        }
    }

    /// Restores original numeric bits and recycled receipt links in O(touched rows).
    pub(super) fn reject_revalidation(&mut self) {
        self.reject();
        let journal = &mut self.revalidation;
        let Some((len, free)) = journal.checkpoint.take() else {
            return;
        };
        for (slot, funds) in journal.funds.drain(..) {
            self.funds[slot] = funds;
        }
        for (owner, first) in journal.owners.drain(..) {
            self.owners[owner] = first;
        }
        for (index, receipt) in journal.receipts.drain(..) {
            self.committed[index] = receipt;
        }
        self.committed.truncate(len);
        self.free_receipt = free;
    }

    /// Retains the account's original quantity and spending before its first outer change.
    pub(super) fn journal_funds(&mut self, slot: usize) {
        let journal = &mut self.revalidation;
        let pass = &mut journal.pass;
        if pass.checkpoint.is_some() && pass.funds_seen[slot] != pass.generation {
            pass.funds_seen[slot] = pass.generation;
            pass.funds.push((slot, self.funds[slot]));
        }
        if !journal.is_active() || journal.funds_seen[slot] == journal.generation {
            return;
        }
        journal.funds_seen[slot] = journal.generation;
        journal.funds.push((slot, self.funds[slot]));
    }

    /// Retains the original owner-chain head before committing any replacement receipts.
    pub(super) fn journal_owner(&mut self, owner: usize) {
        let journal = &mut self.revalidation;
        let pass = &mut journal.pass;
        if let Some((_, _, owners)) = pass.checkpoint
            && owner < owners
            && pass.owner_seen[owner] != pass.generation
        {
            pass.owner_seen[owner] = pass.generation;
            pass.owners.push((owner, self.owners[owner]));
        }
        if !journal.is_active() || journal.owner_seen[owner] == journal.generation {
            return;
        }
        journal.owner_seen[owner] = journal.generation;
        journal.owners.push((owner, self.owners[owner]));
    }

    /// Retains an original arena cell before recycling or changing its link.
    pub(super) fn journal_receipt(&mut self, index: usize) {
        let journal = &mut self.revalidation;
        let pass = &mut journal.pass;
        if let Some((len, _, _)) = pass.checkpoint
            && index < len
            && pass.receipt_seen[index] != pass.generation
        {
            pass.receipt_seen[index] = pass.generation;
            pass.receipts.push((index, self.committed[index]));
        }
        let Some((len, _)) = journal.checkpoint else {
            return;
        };
        if index >= len || journal.receipt_seen[index] == journal.generation {
            return;
        }
        journal.receipt_seen[index] = journal.generation;
        journal.receipts.push((index, self.committed[index]));
    }

    /// Opens a planning pass over the live book in O(1) amortized; snapshot storage follows
    /// the live row counts and is retained across passes.
    pub(super) fn begin_pass(&mut self) {
        debug_assert!(self.staged.is_empty() && !self.revalidation.is_active());
        let pass = &mut self.revalidation.pass;
        assert!(pass.checkpoint.is_none(), "planning passes do not nest");
        pass.generation = pass.generation.wrapping_add(1);
        if pass.generation == 0 {
            pass.funds_seen.fill(0);
            pass.owner_seen.fill(0);
            pass.receipt_seen.fill(0);
            pass.generation = 1;
        }
        // Journals follow the book's row counts, with spare capacity (`SIM-02-K`).
        crate::utils::storage::resize_rows(&mut pass.funds_seen, self.funds.len(), 0);
        crate::utils::storage::resize_rows(&mut pass.owner_seen, self.owners.len(), 0);
        crate::utils::storage::resize_rows(&mut pass.receipt_seen, self.committed.len(), 0);
        pass.checkpoint = Some((self.committed.len(), self.free_receipt, self.owners.len()));
    }

    /// Visits each pass-touched account with its pre-pass quantity and spending.
    pub(super) fn visit_pass_funds(&self, mut visit: impl FnMut(usize, Funds)) {
        for &(slot, funds) in &self.revalidation.pass.funds {
            visit(slot, funds);
        }
    }

    /// Restores every pre-pass account, owner head and receipt cell the pass changed and
    /// truncates temporary owners and appended cells. O(touched rows).
    pub(super) fn end_pass(&mut self) {
        debug_assert!(self.staged.is_empty() && !self.revalidation.is_active());
        let pass = &mut self.revalidation.pass;
        let Some((len, free, owners)) = pass.checkpoint.take() else {
            return;
        };
        for (slot, funds) in pass.funds.drain(..) {
            self.funds[slot] = funds;
        }
        for (owner, first) in pass.owners.drain(..) {
            self.owners[owner] = first;
        }
        for (index, receipt) in pass.receipts.drain(..) {
            self.committed[index] = receipt;
        }
        self.committed.truncate(len);
        self.free_receipt = free;
        self.owners.truncate(owners);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_chains(book: &ConsumerBudgets) {
        let mut seen = vec![false; book.committed.len()];
        for first in book.owners.iter().copied().chain([book.free_receipt]) {
            let mut index = first;
            while index != usize::MAX {
                assert!(
                    !seen[index],
                    "each cell belongs to exactly one owner or the free list"
                );
                seen[index] = true;
                index = book.committed[index].next;
            }
        }
        assert!(seen.into_iter().all(|cell| cell));
    }

    #[test]
    fn customer_fragment_capacity_is_requested_before_inner_commit_and_replayed_exactly() {
        let mut book = ConsumerBudgets {
            categories: vec![super::super::Category {
                resource: 1,
                units_per_resident: 1.0,
                budget_share: 1.0,
            }],
            category_by_resource: vec![usize::MAX, 0],
            households: vec![(100, 1_000.0); 3],
            funds: vec![
                Funds {
                    units: 100.0,
                    gross_spending: 1_000.0
                };
                3
            ],
            staged_slots: vec![usize::MAX; 3],
            free_receipt: usize::MAX,
            ..Default::default()
        };
        book.staged.reserve(3);
        book.prepare_owners(3, 3);
        for owner in 0..3 {
            assert_eq!(book.claim(owner, 1, 10.0, 10.0), 10.0);
            book.accept_for(owner);
        }
        book.committed.shrink_to_fit();
        assert_eq!(book.committed.capacity(), 3);
        book.prepare_revalidation();
        let state = |book: &ConsumerBudgets| {
            (
                book.funds
                    .iter()
                    .map(|f| [f.units, f.gross_spending].map(f64::to_bits))
                    .collect::<Vec<_>>(),
                book.owners.clone(),
                book.free_receipt,
                book.committed
                    .iter()
                    .map(|r| {
                        (
                            r.slot,
                            r.next,
                            [r.acquired.units, r.acquired.gross_spending].map(f64::to_bits),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let original = state(&book);
        let quote = |book: &mut ConsumerBudgets| {
            book.begin_revalidation();
            book.release_owner(2);
            assert_eq!(book.claim(2, 1, 20.0, 11.0), 20.0);
            assert_eq!(book.check_accept_capacity(2), Ok(()));
            book.accept_for(2);
            book.release_owner(0);
            for family in 0..3 {
                assert_eq!(book.claim(family, 1, 5.0, 12.0), 5.0);
            }
        };
        quote(&mut book);
        let staged = state(&book);
        assert_eq!(book.check_accept_capacity(0), Err(5));
        assert_eq!(
            state(&book),
            staged,
            "preflight does not publish a partial inner quote"
        );
        book.reject_revalidation();
        assert_eq!(
            state(&book),
            original,
            "all earlier inner changes roll back too"
        );
        book.reserve_receipts(5);
        let capacity = (
            book.committed.capacity(),
            book.revalidation.receipts.capacity(),
        );
        for _ in 0..64 {
            quote(&mut book);
            assert_eq!(book.check_accept_capacity(0), Ok(()));
            book.accept_for(0);
            book.reject_revalidation();
            assert_eq!(state(&book), original);
            assert_eq!(
                (
                    book.committed.capacity(),
                    book.revalidation.receipts.capacity()
                ),
                capacity
            );
        }
        quote(&mut book);
        book.accept_for(0);
        book.accept_revalidation();
        assert_eq!(book.reserved_units(0, 1), 15.0);
        assert_eq!(book.reserved_units(2, 1), 20.0);
    }

    #[test]
    fn several_customer_replacements_restore_exact_funds_owners_and_recycled_receipts() {
        let mut book = ConsumerBudgets {
            categories: vec![super::super::Category {
                resource: 1,
                units_per_resident: 1.0,
                budget_share: 1.0,
            }],
            category_by_resource: vec![usize::MAX, 0],
            households: vec![(100, 1_000.0); 3],
            funds: vec![
                Funds {
                    units: 100.0,
                    gross_spending: 1_000.0
                };
                3
            ],
            staged_slots: vec![usize::MAX; 3],
            free_receipt: usize::MAX,
            ..Default::default()
        };
        book.staged.reserve(3);
        book.prepare_owners(4, 4);
        for (owner, claims) in [
            (0, &[(0, 60.0, 10.0)][..]),
            (1, &[(0, 40.0, 10.0), (1, 30.0, 8.0)][..]),
            (2, &[(2, 50.0, 9.0)][..]),
        ] {
            for &(family, units, price) in claims {
                assert_eq!(book.claim(family, 1, units, price), units);
            }
            book.accept_for(owner);
        }
        book.prepare_revalidation();
        let original = book.clone();
        let capacities = |b: &ConsumerBudgets| {
            [
                b.committed.capacity(),
                b.staged.capacity(),
                b.revalidation.funds.capacity(),
                b.revalidation.owners.capacity(),
                b.revalidation.receipts.capacity(),
            ]
        };
        let capacity = capacities(&book);
        let funds_bits = |f: &Funds| [f.units, f.gross_spending].map(f64::to_bits);
        let receipt_bits = |r: &Receipt| (r.slot, r.next, funds_bits(&r.acquired));
        for round in 0..128 {
            if round == 64 {
                book.revalidation.generation = u64::MAX;
            }
            book.begin_revalidation();
            book.release_owner(0);
            assert_eq!(book.claim(0, 1, 30.0, 12.0), 30.0);
            assert_eq!(book.claim(1, 1, 20.0, 8.0), 20.0);
            book.accept_for(0);
            book.release_owner(1);
            assert_eq!(book.claim(0, 1, 50.0, 11.0), 50.0);
            assert_eq!(book.claim(2, 1, 10.0, 7.0), 10.0);
            book.accept_for(1);
            assert_eq!(book.claim(1, 1, 5.0, 3.0), 5.0);
            book.accept_for(3);
            book.release_owner(2);
            assert_eq!(book.claim(2, 1, 70.0, 10.0), 70.0);
            if round % 2 == 0 {
                book.accept_for(2);
            }
            assert_chains(&book);
            book.reject_revalidation();
            assert_chains(&book);
            assert_eq!(capacities(&book), capacity);
            assert!(
                book.funds
                    .iter()
                    .map(funds_bits)
                    .eq(original.funds.iter().map(funds_bits))
            );
            assert!(
                book.committed
                    .iter()
                    .map(receipt_bits)
                    .eq(original.committed.iter().map(receipt_bits))
            );
            assert_eq!(book.owners, original.owners);
            assert_eq!(book.free_receipt, original.free_receipt);
            assert_eq!(book.staged_slots, original.staged_slots);
        }
        // Accepted repeated replacements retain current receipts instead of growing history.
        let mut peak = book.committed.len();
        for round in 0..128 {
            book.begin_revalidation();
            book.release_owner(0);
            let units = if round % 2 == 0 { 20.0 } else { 30.0 };
            assert_eq!(book.claim(0, 1, units, 11.0), units);
            book.accept_for(0);
            book.accept_revalidation();
            book.reject_revalidation();
            assert_eq!(book.reserved_units(0, 1), units);
            assert_chains(&book);
            peak = peak.max(book.committed.len());
            assert_eq!(peak, original.committed.len());
            assert_eq!(capacities(&book), capacity);
        }
    }
}
