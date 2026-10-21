// SPDX-License-Identifier: GPL-2.0-only

//! Shared-finance rollback across several accepted supplier substitutions.

use super::{Account, BusinessBudgets, Receipt};

#[derive(Clone, Debug)]
struct Rows<T> {
    seen: Vec<u64>,
    saved: Vec<(usize, T)>,
}

impl<T> Default for Rows<T> {
    fn default() -> Self {
        Self {
            seen: Vec::new(),
            saved: Vec::new(),
        }
    }
}

impl<T: Copy> Rows<T> {
    // Rows follow the book's row counts, with spare capacity (`SIM-02-K`).
    fn prepare(&mut self, size: usize) {
        crate::utils::storage::resize_rows(&mut self.seen, size, 0);
        crate::utils::storage::reserve_rows(&mut self.saved, size);
    }

    fn record(&mut self, index: usize, value: T, generation: u64) {
        if self.seen[index] == generation {
            return;
        }
        self.seen[index] = generation;
        self.saved.push((index, value));
    }

    fn restore(&mut self, values: &mut [T]) {
        for (index, value) in self.saved.drain(..) {
            values[index] = value;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Checkpoint {
    accounts: usize,
    receipts: usize,
    free_receipt: usize,
}

#[derive(Clone, Copy, Debug)]
struct PassCheckpoint {
    accounts: usize,
    receipts: usize,
    free_receipt: usize,
    buyers: usize,
    owners: usize,
}

/// First-touch snapshots of a whole planning pass over a live book. Proposal proofs nest
/// inside it; rows appended by the pass, including temporary project buyers and owners,
/// are truncated rather than saved.
#[derive(Clone, Debug, Default)]
struct Pass {
    checkpoint: Option<PassCheckpoint>,
    generation: u64,
    accounts: Rows<Account>,
    buyers: Rows<[usize; 2]>,
    finance: Rows<[f64; 2]>,
    owners: Rows<usize>,
    receipts: Rows<Receipt>,
}

/// An outer financing proof may accept several inner seller quotes before it can publish
/// any of them. First-touch rows preserve the original numeric bits and receipt ownership.
#[derive(Clone, Debug, Default)]
pub(super) struct Revalidation {
    checkpoint: Option<Checkpoint>,
    generation: u64,
    accounts: Rows<Account>,
    buyers: Rows<[usize; 2]>,
    finance: Rows<[f64; 2]>,
    owners: Rows<usize>,
    receipts: Rows<Receipt>,
    pass: Pass,
}

impl Revalidation {
    /// Whether supplier commits still belong to an enclosing provisional operation.
    pub(super) fn is_active(&self) -> bool {
        self.checkpoint.is_some()
    }

    /// Keeps prepared receipt-indexed storage aligned if the backing arena grows. Such
    /// growth remains a separate allocation defect; it must not corrupt rollback indices.
    pub(super) fn follow_receipts(&mut self, capacity: usize) {
        if !self.receipts.seen.is_empty() && capacity > self.receipts.seen.len() {
            self.receipts.prepare(capacity);
        }
    }
}

impl BusinessBudgets {
    /// Reserves one snapshot per account, buyer, owner and receipt slot before project probes.
    pub(super) fn prepare_revalidation(&mut self) {
        debug_assert!(!self.revalidation.is_active());
        self.staged
            .reserve(self.accounts.capacity().saturating_sub(self.staged.len()));
        self.staged_slots.reserve(
            self.accounts
                .capacity()
                .saturating_sub(self.staged_slots.len()),
        );
        self.revalidation.accounts.prepare(self.accounts.capacity());
        self.revalidation.buyers.prepare(self.finance.len());
        self.revalidation.finance.prepare(self.finance.len());
        self.revalidation.owners.prepare(self.owners.len());
        self.revalidation
            .receipts
            .prepare(self.committed.capacity());
    }

    /// Begins an outer proof in O(1), retaining the existing inner seller-quote transactions.
    pub(super) fn begin_revalidation(&mut self) {
        debug_assert!(self.staged.is_empty());
        let journal = &mut self.revalidation;
        assert!(
            journal.checkpoint.is_none(),
            "only seller quotes may nest inside revalidation"
        );
        journal.generation = journal.generation.wrapping_add(1);
        if journal.generation == 0 {
            for seen in [
                &mut journal.accounts.seen,
                &mut journal.buyers.seen,
                &mut journal.finance.seen,
                &mut journal.owners.seen,
                &mut journal.receipts.seen,
            ] {
                seen.fill(0);
            }
            journal.generation = 1;
        }
        journal.checkpoint = Some(Checkpoint {
            accounts: self.accounts.len(),
            receipts: self.committed.len(),
            free_receipt: self.free_receipt,
        });
    }

    /// Publishes all inner seller commits together, dropping only touched-row snapshots.
    pub(super) fn accept_revalidation(&mut self) {
        debug_assert!(self.staged.is_empty());
        let journal = &mut self.revalidation;
        journal.checkpoint = None;
        journal.accounts.saved.clear();
        journal.buyers.saved.clear();
        journal.finance.saved.clear();
        journal.owners.saved.clear();
        journal.receipts.saved.clear();
    }

    /// Visits pre-proof account quantities so the router can restore only affected buckets.
    pub(super) fn visit_revalidation_accounts(&self, mut visit: impl FnMut(usize, Account)) {
        for &(slot, account) in &self.revalidation.accounts.saved {
            visit(slot, account);
        }
        if let Some(checkpoint) = self.revalidation.checkpoint {
            for slot in checkpoint.accounts..self.accounts.len() {
                visit(
                    slot,
                    Account {
                        units: 0.0,
                        ..self.accounts[slot]
                    },
                );
            }
        }
    }

    /// Visits buyers whose account chain or shared financing will be restored by rollback.
    pub(super) fn visit_revalidation_buyers(&self, mut visit: impl FnMut(usize)) {
        for &(buyer, _) in &self.revalidation.finance.saved {
            visit(buyer);
        }
        for &(buyer, _) in &self.revalidation.buyers.saved {
            visit(buyer);
        }
        self.visit_revalidation_accounts(|_, account| visit(account.buyer));
    }

    /// Restores accepted inner substitutions and the pending quote without arithmetic reversal.
    /// O(touched accounts + buyers + owners + receipts), with no city scan or new allocation.
    pub(super) fn reject_revalidation(&mut self) {
        self.reject();
        let journal = &mut self.revalidation;
        let Some(checkpoint) = journal.checkpoint.take() else {
            return;
        };
        journal.accounts.restore(&mut self.accounts);
        for (buyer, [head, tail]) in journal.buyers.saved.drain(..) {
            self.buyer_accounts[buyer] = head;
            self.buyer_tails[buyer] = tail;
        }
        journal.finance.restore(&mut self.finance);
        journal.owners.restore(&mut self.owners);
        journal.receipts.restore(&mut self.committed);
        self.accounts.truncate(checkpoint.accounts);
        self.staged_slots.truncate(checkpoint.accounts);
        self.committed.truncate(checkpoint.receipts);
        self.free_receipt = checkpoint.free_receipt;
    }

    /// Saves an account once before any quantity or quote-basis mutation in the outer proof.
    pub(super) fn journal_account(&mut self, slot: usize) {
        let journal = &mut self.revalidation;
        if let Some(checkpoint) = journal.checkpoint
            && slot < checkpoint.accounts
        {
            journal
                .accounts
                .record(slot, self.accounts[slot], journal.generation);
        }
        let pass = &mut journal.pass;
        if let Some(checkpoint) = pass.checkpoint
            && slot < checkpoint.accounts
        {
            pass.accounts
                .record(slot, self.accounts[slot], pass.generation);
        }
    }

    /// Saves the original input-chain endpoints before appending a replacement's tranches.
    pub(super) fn journal_buyer(&mut self, buyer: usize) {
        let journal = &mut self.revalidation;
        let ends = [self.buyer_accounts[buyer], self.buyer_tails[buyer]];
        if journal.is_active() {
            journal.buyers.record(buyer, ends, journal.generation);
        }
        let pass = &mut journal.pass;
        if let Some(checkpoint) = pass.checkpoint
            && buyer < checkpoint.buyers
        {
            pass.buyers.record(buyer, ends, pass.generation);
        }
    }

    /// Saves shared buyer headroom before the first financial change from any supplier.
    pub(super) fn journal_finance(&mut self, buyer: usize) {
        let journal = &mut self.revalidation;
        if journal.is_active() {
            journal
                .finance
                .record(buyer, self.finance[buyer], journal.generation);
        }
        let pass = &mut journal.pass;
        if let Some(checkpoint) = pass.checkpoint
            && buyer < checkpoint.buyers
        {
            pass.finance
                .record(buyer, self.finance[buyer], pass.generation);
        }
    }

    /// Saves the seller's receipt head before inner retirement, seeding or extension.
    pub(super) fn journal_owner(&mut self, owner: usize) {
        let journal = &mut self.revalidation;
        if journal.is_active() {
            journal
                .owners
                .record(owner, self.owners[owner], journal.generation);
        }
        let pass = &mut journal.pass;
        if let Some(checkpoint) = pass.checkpoint
            && owner < checkpoint.owners
        {
            pass.owners
                .record(owner, self.owners[owner], pass.generation);
        }
    }

    /// Saves old arena cells before recycling/link edits; new cells are discarded by truncation.
    pub(super) fn journal_receipt(&mut self, index: usize) {
        let journal = &mut self.revalidation;
        if let Some(checkpoint) = journal.checkpoint
            && index < checkpoint.receipts
        {
            journal
                .receipts
                .record(index, self.committed[index], journal.generation);
        }
        let pass = &mut journal.pass;
        if let Some(checkpoint) = pass.checkpoint
            && index < checkpoint.receipts
        {
            pass.receipts
                .record(index, self.committed[index], pass.generation);
        }
    }

    /// Opens a planning pass over this live book in O(1) amortized: snapshot storage
    /// follows the live row counts and is retained across passes.
    pub(super) fn begin_pass(&mut self) {
        debug_assert!(self.staged.is_empty() && !self.revalidation.is_active());
        let pass = &mut self.revalidation.pass;
        assert!(pass.checkpoint.is_none(), "planning passes do not nest");
        pass.generation = pass.generation.wrapping_add(1);
        if pass.generation == 0 {
            for seen in [
                &mut pass.accounts.seen,
                &mut pass.buyers.seen,
                &mut pass.finance.seen,
                &mut pass.owners.seen,
                &mut pass.receipts.seen,
            ] {
                seen.fill(0);
            }
            pass.generation = 1;
        }
        let checkpoint = PassCheckpoint {
            accounts: self.accounts.len(),
            receipts: self.committed.len(),
            free_receipt: self.free_receipt,
            buyers: self.finance.len(),
            owners: self.owners.len(),
        };
        pass.accounts.prepare(checkpoint.accounts);
        pass.buyers.prepare(checkpoint.buyers);
        pass.finance.prepare(checkpoint.buyers);
        pass.owners.prepare(checkpoint.owners);
        pass.receipts.prepare(checkpoint.receipts);
        pass.checkpoint = Some(checkpoint);
    }

    /// Visits each pass-touched account with its pre-pass value; appended accounts read as
    /// empty, so the router can restore exactly the bucket entries the pass changed.
    pub(super) fn visit_pass_accounts(&self, mut visit: impl FnMut(usize, Account)) {
        let pass = &self.revalidation.pass;
        let Some(checkpoint) = pass.checkpoint else {
            return;
        };
        for &(slot, account) in &pass.accounts.saved {
            visit(slot, account);
        }
        for slot in checkpoint.accounts..self.accounts.len() {
            visit(
                slot,
                Account {
                    units: 0.0,
                    ..self.accounts[slot]
                },
            );
        }
    }

    /// Buyers whose financing, account chain or tranches the pass changed.
    pub(super) fn visit_pass_buyers(&self, mut visit: impl FnMut(usize)) {
        let pass = &self.revalidation.pass;
        for &(buyer, _) in &pass.finance.saved {
            visit(buyer);
        }
        for &(buyer, _) in &pass.buyers.saved {
            visit(buyer);
        }
        for &(_, account) in &pass.accounts.saved {
            visit(account.buyer);
        }
    }

    /// Whether the pass saved this pre-pass account; unsaved pre-pass rows are unchanged.
    pub(super) fn pass_saved_account(&self, slot: usize) -> bool {
        let pass = &self.revalidation.pass;
        pass.checkpoint
            .is_some_and(|checkpoint| slot < checkpoint.accounts)
            && pass.accounts.seen[slot] == pass.generation
    }

    /// Pre-pass row counts of accounts and buyers, if a pass is open.
    pub(super) fn pass_extent(&self) -> Option<(usize, usize)> {
        self.revalidation
            .pass
            .checkpoint
            .map(|checkpoint| (checkpoint.accounts, checkpoint.buyers))
    }

    /// Restores every pre-pass row the pass changed and truncates rows it appended.
    /// O(touched rows); proposal proofs must already be closed.
    pub(super) fn end_pass(&mut self) {
        debug_assert!(self.staged.is_empty() && !self.revalidation.is_active());
        let pass = &mut self.revalidation.pass;
        let Some(checkpoint) = pass.checkpoint.take() else {
            return;
        };
        pass.accounts.restore(&mut self.accounts);
        for (buyer, [head, tail]) in pass.buyers.saved.drain(..) {
            self.buyer_accounts[buyer] = head;
            self.buyer_tails[buyer] = tail;
        }
        pass.finance.restore(&mut self.finance);
        pass.owners.restore(&mut self.owners);
        pass.receipts.restore(&mut self.committed);
        self.accounts.truncate(checkpoint.accounts);
        self.staged_slots.truncate(checkpoint.accounts);
        self.committed.truncate(checkpoint.receipts);
        self.free_receipt = checkpoint.free_receipt;
        for rows in [&mut self.buyer_accounts, &mut self.buyer_tails] {
            rows.truncate(checkpoint.buyers);
        }
        self.finance.truncate(checkpoint.buyers);
        self.finance_staged.truncate(checkpoint.buyers);
        self.owners.truncate(checkpoint.owners);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::operation::InputRequirement;

    #[test]
    fn replacement_input_tranches_restore_accounts_finance_and_both_receipt_chains() {
        let mut book = BusinessBudgets::default();
        book.reset(3, 2, 3, 4);
        let input = InputRequirement {
            resource: 1,
            daily_units: 100.0,
            buffer_units: 400.0,
            replenishment_batch_units: Some(50.0),
        };
        for buyer in 0..2 {
            book.set_finance(buyer, [100.0, 10_000.0]);
            let slot = book
                .push_input(buyer, input, 10.0, 50.0, 0.0, 100.0, 1.0)
                .unwrap();
            book.seed_for(2, slot);
        }
        book.reserve_accounts(8);
        let original = book.clone();
        let account_bits = |a: &Account| {
            (
                [a.buyer, a.resource as usize, a.next_buyer, a.first_receipt],
                [a.units, a.goods, a.carrier, a.stock_exposure].map(f64::to_bits),
                [a.batch, a.goods_multiplier].map(f32::to_bits),
            )
        };
        let receipt_bits = |r: &Receipt| {
            (
                [
                    r.account,
                    r.seller,
                    r.next,
                    r.previous,
                    r.next_account,
                    r.previous_account,
                ],
                [r.units, r.margin, r.capital, r.goods].map(f64::to_bits),
                r.quote.map(f32::to_bits),
                r.input_origin,
            )
        };
        let capacities = |b: &BusinessBudgets| {
            [
                b.accounts.capacity(),
                b.staged_slots.capacity(),
                b.staged.capacity(),
                b.committed.capacity(),
                b.revalidation.accounts.saved.capacity(),
                b.revalidation.buyers.saved.capacity(),
                b.revalidation.receipts.saved.capacity(),
            ]
        };
        let prepared = capacities(&book);
        for round in 0..64 {
            book.begin_revalidation();
            book.withdraw_buyer(0, |seller, resource, units, revenue| {
                assert_eq!((seller, resource, units, revenue), (2, 1, 100.0, 1_000.0));
            });
            book.set_finance(0, [250.0, 2_000.0]);
            book.set_finance(2, [300.0, 3_000.0]);
            // Both a populated chain and a previously empty buyer get new recipe tranches.
            for buyer in [0, 2, 0] {
                let slot = book
                    .push_input(
                        buyer,
                        InputRequirement {
                            resource: 2,
                            ..input
                        },
                        12.0,
                        75.0,
                        20.0,
                        30.0,
                        1.25,
                    )
                    .unwrap();
                assert_eq!(book.claim_opportunity(slot, 30.0, 8.0, 50.0), 30.0);
            }
            assert!(book.accept_for(1));
            assert_eq!(book.sales(1, 2, 100.0), (90.0, 900.0));
            // Seeded physical purchases also recycle old receipts and append account links.
            let slot = book
                .push_input(0, input, 9.0, 20.0, 0.0, 15.0, 1.0)
                .unwrap();
            book.seed_for(2, slot);
            if round % 2 == 0 {
                book.release_owner(1);
                book.claim_opportunity(slot - 1, 10.0, 10.0, 50.0);
            }
            book.reject_revalidation();
            assert_eq!(capacities(&book), prepared);
            assert_eq!(book.accounts.len(), original.accounts.len());
            assert!(
                book.accounts
                    .iter()
                    .map(account_bits)
                    .eq(original.accounts.iter().map(account_bits))
            );
            assert!(
                book.committed
                    .iter()
                    .map(receipt_bits)
                    .eq(original.committed.iter().map(receipt_bits))
            );
            assert!(
                book.finance
                    .iter()
                    .map(|f| f.map(f64::to_bits))
                    .eq(original.finance.iter().map(|f| f.map(f64::to_bits)))
            );
            assert_eq!(book.buyer_accounts, original.buyer_accounts);
            assert_eq!(book.buyer_tails, original.buyer_tails);
            assert_eq!(book.owners, original.owners);
            assert_eq!(book.free_receipt, original.free_receipt);
            assert_eq!(book.staged_slots, original.staged_slots);
        }
        book.begin_revalidation();
        book.withdraw_buyer(0, |_, _, _, _| {});
        book.set_finance(0, [150.0, 2_000.0]);
        let slot = book
            .push_input(0, input, 9.0, 20.0, 0.0, 15.0, 1.0)
            .unwrap();
        book.seed_for(1, slot);
        book.accept_revalidation();
        // A later rejection restores the accepted recipe, not the pass's initial account set.
        book.begin_revalidation();
        book.withdraw_buyer(0, |_, _, _, _| {});
        book.push_input(0, input, 14.0, 25.0, 0.0, 30.0, 1.0)
            .unwrap();
        book.reject_revalidation();
        assert_eq!(book.buyer_tails[0], slot);
        assert_eq!(book.accounts.len(), slot + 1);
        assert_eq!(book.sales(1, 1, 100.0), (15.0, 135.0));
        assert_eq!(book.finance[0], [150.0, 2_000.0]);
    }

    #[test]
    fn finite_finance_replay_preserves_every_receipt() {
        let mut book = BusinessBudgets::default();
        let mut passes = [0; 2];
        let mut results = Vec::new();
        for pass_count in &mut passes {
            loop {
                *pass_count += 1;
                // The reset's receipt estimate, with its spare rows, stays short of the 112
                // receipts, so the first pass replays.
                book.reset(16, 16, 32, 24);
                for buyer in 0..16 {
                    book.set_finance(buyer, [100.0, 1_000_000.0]);
                    book.push_input(
                        buyer,
                        InputRequirement {
                            resource: 1,
                            daily_units: 100.0,
                            buffer_units: 400.0,
                            replenishment_batch_units: Some(50.0),
                        },
                        10.0,
                        50.0,
                        0.0,
                        100.0,
                        1.0,
                    )
                    .unwrap();
                }
                let capacity = book.committed.capacity();
                let mut required = None;
                for owner in 0..8 {
                    for slot in 0..16 {
                        book.claim_opportunity(slot, 100.0, 11.0, 50.0);
                    }
                    let previous_len = book.committed.len();
                    if let Err(size) = book.try_extend_for(owner) {
                        assert_eq!(book.committed.len(), previous_len);
                        assert_eq!(book.owners[owner], usize::MAX);
                        required = Some(size);
                        break;
                    }
                    assert_eq!(book.committed.capacity(), capacity);
                }
                assert_eq!(book.committed.capacity(), capacity);
                if let Some(size) = required {
                    book.reserve_receipts(size);
                    continue;
                }
                break;
            }
            assert_eq!(book.committed.len(), 112);
            results.push((
                book.finance
                    .iter()
                    .map(|f| f.map(f64::to_bits))
                    .collect::<Vec<_>>(),
                (0..8)
                    .map(|owner| {
                        let (u, r) = book.sales(owner, 1, 100.0);
                        [u.to_bits(), r.to_bits()]
                    })
                    .collect::<Vec<_>>(),
            ));
        }
        assert_eq!(passes, [2, 1]);
        assert_eq!(results[0], results[1]);
    }

    #[test]
    fn financial_fragmentation_preserves_rollback_after_receipt_arena_growth() {
        let mut book = BusinessBudgets::default();
        book.reset(16, 16, 32, 32);
        for buyer in 0..16 {
            book.set_finance(buyer, [100.0, 1_000_000.0]);
            book.push_input(
                buyer,
                InputRequirement {
                    resource: 1,
                    daily_units: 100.0,
                    buffer_units: 400.0,
                    replenishment_batch_units: Some(50.0),
                },
                10.0,
                50.0,
                0.0,
                100.0,
                1.0,
            )
            .unwrap();
        }
        book.prepare_revalidation();
        for owner in 0..8 {
            for slot in 0..16 {
                book.claim_opportunity(slot, 100.0, 11.0, 50.0);
            }
            assert!(book.accept_for(owner));
        }
        assert_eq!(
            book.committed.len(),
            112,
            "representable financing-limited remnants cannot be rounded away"
        );
        let balances: Vec<_> = book
            .finance
            .iter()
            .map(|balance| balance.map(f64::to_bits))
            .collect();
        let sales = book.sales(6, 1, 100.0);
        assert!(
            sales.0 > 0.0 && sales.1 > 0.0,
            "a large requested limit cannot erase a small but funded sale"
        );
        for _ in 0..8 {
            book.begin_revalidation();
            book.release_owner(6);
            assert!(book.accept_for(6));
            book.reject_revalidation();
            assert_eq!(book.sales(6, 1, 100.0), sales);
            assert!(
                book.finance
                    .iter()
                    .map(|balance| balance.map(f64::to_bits))
                    .eq(balances.iter().copied())
            );
        }
    }

    #[test]
    fn enclosing_rejection_restores_committed_financing_and_recycled_receipts() {
        let mut book = BusinessBudgets::default();
        book.reset(1, 2, 3, 3);
        book.set_finance(0, [300.0, 2_000.0]);
        let mut slots = Vec::new();
        for resource in [1, 2] {
            slots.push(
                book.push_input(
                    0,
                    InputRequirement {
                        resource,
                        daily_units: 100.0,
                        buffer_units: 400.0,
                        replenishment_batch_units: Some(50.0),
                    },
                    10.0,
                    50.0,
                    0.0,
                    100.0,
                    1.0,
                )
                .unwrap(),
            );
        }
        assert_eq!(book.claim_opportunity(slots[0], 100.0, 8.0, 50.0), 100.0);
        assert!(book.accept_for(0));
        assert_eq!(book.claim_opportunity(slots[1], 50.0, 12.0, 50.0), 50.0);
        assert!(book.accept_for(1));
        book.prepare_revalidation();
        let original = book.clone();
        let receipt_bits = |receipt: &Receipt| {
            (
                receipt.account,
                receipt.units.to_bits(),
                receipt.margin.to_bits(),
                receipt.capital.to_bits(),
                receipt.goods.to_bits(),
                receipt.quote.map(f32::to_bits),
                receipt.input_origin,
                [
                    receipt.next,
                    receipt.seller,
                    receipt.previous,
                    receipt.next_account,
                    receipt.previous_account,
                ],
            )
        };
        let capacities = (
            book.committed.capacity(),
            book.revalidation.accounts.saved.capacity(),
            book.revalidation.finance.saved.capacity(),
            book.revalidation.owners.saved.capacity(),
            book.revalidation.receipts.saved.capacity(),
        );
        for _ in 0..128 {
            book.begin_revalidation();
            // Append a new owner's receipt, then recycle both existing owners' cells.
            assert_eq!(book.claim_opportunity(slots[1], 10.0, 10.0, 50.0), 10.0);
            assert!(book.accept_for(2));
            book.release_owner(0);
            assert_eq!(book.claim_opportunity(slots[0], 100.0, 9.0, 50.0), 100.0);
            assert!(book.accept_for(0));
            book.release_owner(1);
            assert_eq!(book.claim_opportunity(slots[1], 50.0, 13.0, 50.0), 50.0);
            assert!(book.accept_for(1));
            book.release_owner(0);
            assert!(book.claim_opportunity(slots[0], 100.0, 100.0, 50.0) < 100.0);
            book.reject_revalidation();
            assert_eq!(
                book.finance[0].map(f64::to_bits),
                original.finance[0].map(f64::to_bits)
            );
            for (current, previous) in book.accounts.iter().zip(&original.accounts) {
                assert_eq!(current.units.to_bits(), previous.units.to_bits());
                assert_eq!(current.first_receipt, previous.first_receipt);
                assert_eq!(current.next_buyer, previous.next_buyer);
            }
            assert_eq!(book.owners, original.owners);
            assert_eq!(book.free_receipt, original.free_receipt);
            assert!(
                book.committed
                    .iter()
                    .map(receipt_bits)
                    .eq(original.committed.iter().map(receipt_bits))
            );
            assert_eq!(
                (
                    book.committed.capacity(),
                    book.revalidation.accounts.saved.capacity(),
                    book.revalidation.finance.saved.capacity(),
                    book.revalidation.owners.saved.capacity(),
                    book.revalidation.receipts.saved.capacity()
                ),
                capacities
            );
        }
        // A dearer accepted replacement debits margin and capital; later rejection keeps it.
        assert_eq!(book.finance[0], [200.0, 1_600.0]);
        book.begin_revalidation();
        book.release_owner(0);
        assert_eq!(book.claim_opportunity(slots[0], 100.0, 11.0, 50.0), 100.0);
        assert!(book.accept_for(0));
        book.accept_revalidation();
        assert_eq!(book.finance[0], [100.0, 1_200.0]);
        book.reject_revalidation();
        assert_eq!(book.finance[0], [100.0, 1_200.0]);
    }
}
