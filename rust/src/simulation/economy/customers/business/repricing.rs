// SPDX-License-Identifier: GPL-2.0-only

//! Reconciles an affected buyer's established promises with newly priced physical inputs.

use super::{BusinessBudgets, Receipt};

#[derive(Clone, Copy, Debug)]
struct Purchase {
    seller: usize,
    resource: u16,
    units: f64,
    unmatched: f64,
    quote: [f32; 2],
    input_origin: bool,
    order: usize,
}

/// Scratch belongs to one buyer reprice, never to accepted simulation state.
#[derive(Clone, Debug, Default)]
pub(super) struct Repricing {
    previous: Vec<Purchase>,
    current: Vec<Purchase>,
    accounts: Vec<(u16, usize)>,
}

impl BusinessBudgets {
    /// Number of this buyer's old cells available for reuse after its remembered withdrawal.
    pub(super) fn repricing_previous_count(&self) -> usize { self.repricing.previous.len() }

    /// Prepared receipt scratch can lag arena growth until the next whole-proof retry.
    pub(super) fn repricing_capacity(&self) -> usize {
        self.repricing.previous.capacity().min(self.repricing.current.capacity())
    }

    /// Prepares reconciliation scratch outside complete operation proofs. Each side holds
    /// at most the receipt arena's live entries, including new physical seed tranches.
    pub(super) fn prepare_repricing(&mut self) {
        debug_assert!(!self.revalidation.is_active());
        let capacity = self.committed.capacity();
        self.repricing.previous.clear();
        self.repricing.current.clear();
        self.repricing.previous.reserve(capacity);
        self.repricing.current.reserve(capacity);
        self.repricing.accounts.clear();
        self.repricing.accounts.reserve(self.accounts.capacity());
    }

    /// Remembers this buyer's purchases before reseeding, with allocation-free in-place
    /// ordering for source reconciliation. Exhaustion changes only temporary scratch.
    pub(super) fn remember_purchases(&mut self, buyer: usize) -> Result<(), usize> {
        debug_assert!(self.revalidation.is_active() && self.staged.is_empty());
        self.repricing.previous.clear();
        let mut slot = self.buyer_accounts[buyer];
        while slot != usize::MAX {
            let account = self.accounts[slot];
            let mut index = account.first_receipt;
            while index != usize::MAX {
                let receipt = self.committed[index];
                if self.repricing.previous.len() == self.repricing.previous.capacity() {
                    return Err(self.repricing.previous.len() + 1);
                }
                self.repricing.previous.push(Purchase { seller: receipt.seller,
                    resource: account.resource, units: receipt.units, unmatched: receipt.units, quote: receipt.quote,
                    input_origin: receipt.input_origin, order: self.repricing.previous.len() });
                index = receipt.next_account;
            }
            slot = account.next_buyer;
        }
        // Existing physical purchases consume their matching seed before future promises.
        // The explicit order tie-breaker makes unstable in-place sorting deterministic.
        self.repricing.previous.sort_unstable_by_key(|purchase|
            (purchase.resource, purchase.seller, !purchase.input_origin, purchase.order));
        Ok(())
    }

    /// Reconciles newly seeded deliveries against old purchases, then restores every still
    /// outstanding promise. No buyer funds become public until the outer proof validates
    /// the resulting finance. Returns false if the new recipe cannot preserve a commitment;
    /// the enclosing replacement must then roll back, never cancel an earlier seller.
    /// Returns an exact receipt-capacity request before restoring any outstanding promises.
    pub(super) fn restore_purchases(&mut self, buyer: usize) -> Result<bool, usize> {
        debug_assert!(self.revalidation.is_active() && self.staged.is_empty());
        self.repricing.current.clear();
        self.repricing.accounts.clear();
        let mut slot = self.buyer_accounts[buyer];
        while slot != usize::MAX {
            let account = self.accounts[slot];
            self.repricing.accounts.push((account.resource, slot));
            let mut index = account.first_receipt;
            while index != usize::MAX {
                let receipt = self.committed[index];
                debug_assert!(receipt.input_origin);
                self.repricing.current.push(Purchase { seller: receipt.seller,
                    resource: account.resource, units: receipt.units, unmatched: receipt.units, quote: receipt.quote,
                    input_origin: true, order: self.repricing.current.len() });
                index = receipt.next_account;
            }
            slot = account.next_buyer;
        }
        self.repricing.current.sort_unstable_by_key(|purchase|
            (purchase.resource, purchase.seller, purchase.order));
        self.repricing.accounts.sort_unstable();
        let mut current = 0;
        for old in &mut self.repricing.previous {
            let key = (old.resource, old.seller);
            while let Some(new) = self.repricing.current.get_mut(current) {
                let new_key = (new.resource, new.seller);
                if new_key > key { break; }
                if new_key < key || new.unmatched == 0.0 { current += 1; continue; }
                let used = old.unmatched.min(new.unmatched);
                old.unmatched -= used;
                new.unmatched -= used;
                if old.unmatched == 0.0 { break; }
            }
        }
        // Preflight with the same dependent account order as publication. Staging provides
        // already-prepared exact quantity/finance snapshots; reject restores them afterward.
        let (complete, added) = self.replay_purchases(buyer, false);
        self.reject();
        if !complete { return Ok(false); }
        let mut free = self.free_receipt;
        let mut reusable = 0;
        while free != usize::MAX && reusable < added {
            reusable += 1;
            free = self.committed[free].next;
        }
        let required = self.committed.len() + added - reusable;
        if required > self.committed.capacity().min(self.repricing_capacity()) { return Err(required); }
        let restored = self.replay_purchases(buyer, true);
        debug_assert_eq!(restored, (true, added));
        Ok(true)
    }

    fn replay_purchases(&mut self, buyer: usize, publish: bool) -> (bool, usize) {
        let mut cursor = 0;
        let mut added = 0;
        for index in 0..self.repricing.previous.len() {
            let purchase = self.repricing.previous[index];
            let mut remaining = purchase.unmatched;
            while remaining > 0.0 && cursor < self.repricing.accounts.len() {
                let (resource, slot) = self.repricing.accounts[cursor];
                let account = self.accounts[slot];
                if resource > purchase.resource { break; }
                if resource < purchase.resource || account.units == 0.0 { cursor += 1; continue; }
                {
                    let units = remaining.min(account.units);
                    if !publish {
                        self.stage(slot);
                        self.accounts[slot].units -= units;
                        remaining -= units;
                        added += 1;
                        continue;
                    }
                    // Quotes already contain the receiving multiplier. Reapplying or
                    // dividing it would change invoice bits. Never spend speculative savings.
                    let goods_delta = f64::from(purchase.quote[0]) - account.goods;
                    let freight_delta = f64::from(purchase.quote[1]) / f64::from(account.batch) - account.carrier;
                    let margin = units * (goods_delta + freight_delta).max(0.0);
                    let capital = units * (goods_delta * account.stock_exposure + freight_delta).max(0.0);
                    self.journal_account(slot);
                    self.journal_finance(buyer);
                    self.accounts[slot].units -= units;
                    self.finance[buyer][0] -= margin;
                    self.finance[buyer][1] -= capital;
                    self.insert_receipt(purchase.seller, usize::MAX, Receipt { account: slot,
                        units, margin, capital, goods: units * f64::from(purchase.quote[0]),
                        quote: purchase.quote, input_origin: false, next: usize::MAX,
                        seller: usize::MAX, previous: usize::MAX, next_account: usize::MAX,
                        previous_account: usize::MAX });
                    remaining -= units;
                    added += 1;
                }
            }
            if remaining > 0.0 { return (false, added); }
        }
        (true, added)
    }

    /// Reports only changed seller/resource totals, so an unchanged receipt reclassification
    /// cannot expand the affected-operation set through the entire connected market.
    pub(super) fn visit_repriced_sellers(&mut self, buyer: usize,
        mut changed: impl FnMut(usize, u16, [f64; 2], [f64; 2])) {
        self.repricing.current.clear();
        let mut slot = self.buyer_accounts[buyer];
        while slot != usize::MAX {
            let account = self.accounts[slot];
            let mut index = account.first_receipt;
            while index != usize::MAX {
                let receipt = self.committed[index];
                self.repricing.current.push(Purchase { seller: receipt.seller,
                    resource: account.resource, units: receipt.units, unmatched: 0.0,
                    quote: receipt.quote, input_origin: receipt.input_origin,
                    order: self.repricing.current.len() });
                index = receipt.next_account;
            }
            slot = account.next_buyer;
        }
        self.repricing.current.sort_unstable_by_key(|purchase|
            (purchase.resource, purchase.seller, !purchase.input_origin, purchase.order));
        let (mut old, mut new) = (0, 0);
        let previous = &self.repricing.previous;
        let current = &self.repricing.current;
        while old < previous.len() || new < current.len() {
            let key = |purchase: &Purchase| (purchase.resource, purchase.seller);
            let next = previous.get(old).map(key).into_iter().chain(current.get(new).map(key)).min().unwrap();
            let sum = |rows: &[Purchase], cursor: &mut usize| {
                let mut total = [0.0; 2];
                while let Some(purchase) = rows.get(*cursor) {
                    if key(purchase) != next { break; }
                    total[0] += purchase.units;
                    total[1] += purchase.units * f64::from(purchase.quote[0]);
                    *cursor += 1;
                }
                total
            };
            let before = sum(previous, &mut old);
            let after = sum(current, &mut new);
            if before != after { changed(next.1, next.0, before, after); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::operation::InputRequirement;

    #[test]
    fn buyer_repricing_deduplicates_delivered_promises_and_restores_exactly() {
        let input = InputRequirement { resource: 1, daily_units: 100.0, buffer_units: 400.0,
            replenishment_batch_units: Some(50.0) };
        let mut book = BusinessBudgets::default();
        book.reset(1, 3, 2, 2);
        book.set_finance(0, [2_000.0, 10_000.0]);
        let local = book.push_input(0, input, 11.0, 50.0, 0.0, 30.0, 1.1).unwrap();
        book.seed_for(1, local);
        let outside = book.push_input(0, input, 22.0, 100.0, 0.0, 70.0, 1.1).unwrap();
        assert_eq!(book.claim_opportunity(outside, 50.0, 25.0, 150.0), 50.0);
        book.try_extend_for(1).unwrap();
        book.reserve_accounts(8);
        book.reserve_receipts(32);
        let original = book.clone();
        let bits = |receipt: &Receipt| ([receipt.account, receipt.seller, receipt.next,
            receipt.previous, receipt.next_account, receipt.previous_account],
            [receipt.units, receipt.margin, receipt.capital, receipt.goods].map(f64::to_bits),
            receipt.quote.map(f32::to_bits), receipt.input_origin);
        let capacity = (book.committed.capacity(), book.repricing.previous.capacity(),
            book.repricing.current.capacity(), book.repricing.accounts.capacity());
        for round in 0..64 {
            book.begin_revalidation();
            book.remember_purchases(0).unwrap();
            book.withdraw_buyer(0, |_, _, _, _| {});
            let base = if round % 2 == 0 { [1_200.0, 8_000.0] } else { [100.0, 500.0] };
            book.set_finance(0, base);
            book.replace_inputs(0, 3, |book| {
                for (units, goods, freight) in [(30.0, 11.0, 50.0), (20.0, 27.5, 150.0)] {
                    let slot = book.push_input(0, input, goods, freight, 0.0, units, 1.1).unwrap();
                    book.seed_for(1, slot);
                }
                book.push_input(0, input, 22.0, 100.0, 0.0, 50.0, 1.1).unwrap();
            });
            assert_eq!(book.restore_purchases(0), Ok(true));
            assert_eq!(book.sales(1, 1, f64::INFINITY), original.sales(1, 1, f64::INFINITY));
            assert_eq!(book.finance[0], [base[0] - 195.0, base[1] - 690.0],
                "only the undelivered 30 units charge future price premiums");
            let mut changed = 0;
            book.visit_repriced_sellers(0, |_, _, _, _| changed += 1);
            assert_eq!(changed, 0, "unchanged purchases cannot propagate a new affected set");
            book.reject_revalidation();
            assert_eq!(book.finance, original.finance);
            assert_eq!(book.owners, original.owners);
            assert_eq!(book.buyer_accounts, original.buyer_accounts);
            assert_eq!(book.buyer_tails, original.buyer_tails);
            assert_eq!(book.free_receipt, original.free_receipt);
            assert!(book.committed.iter().map(bits).eq(original.committed.iter().map(bits)));
            assert_eq!((book.committed.capacity(), book.repricing.previous.capacity(),
                book.repricing.current.capacity(), book.repricing.accounts.capacity()), capacity);
        }
        book.begin_revalidation();
        book.remember_purchases(0).unwrap();
        book.withdraw_buyer(0, |_, _, _, _| {});
        book.set_finance(0, [10_000.0; 2]);
        book.replace_inputs(0, 1, |book| {
            book.push_input(0, InputRequirement { resource: 2, ..input }, 22.0, 100.0,
                0.0, 100.0, 1.1).unwrap();
        });
        assert_eq!(book.restore_purchases(0), Ok(false), "a new recipe cannot silently cancel earlier promises");
        book.reject_revalidation();
        assert_eq!(book.sales(1, 1, f64::INFINITY), original.sales(1, 1, f64::INFINITY));
    }

    #[test]
    fn buyer_repricing_requests_exact_fragment_capacity_before_publication() {
        let input = InputRequirement { resource: 1, daily_units: 100.0, buffer_units: 400.0,
            replenishment_batch_units: Some(50.0) };
        let mut book = BusinessBudgets::default();
        book.reset(1, 1, 8, 0);
        book.set_finance(0, [1_000.0; 2]);
        let slot = book.push_input(0, input, 10.0, 50.0, 0.0, 100.0, 1.0).unwrap();
        for seller in 0..8 {
            assert_eq!(book.claim_opportunity(slot, 12.5, 10.0, 50.0), 12.5);
            assert!(book.accept_for(seller));
        }
        book.reserve_accounts(9);
        let old_capacity = book.committed.capacity();
        assert!(old_capacity < 16);
        for attempt in 0..2 {
            book.begin_revalidation();
            book.remember_purchases(0).unwrap();
            book.withdraw_buyer(0, |_, _, _, _| {});
            book.set_finance(0, [1_000.0; 2]);
            book.replace_inputs(0, 9, |book| {
                for index in 0..9 {
                    book.push_input(0, input, 10.0, 50.0, 0.0,
                        if index == 8 { 12.0 } else { 11.0 }, 1.0).unwrap();
                }
            });
            let result = book.restore_purchases(0);
            if attempt == 0 {
                assert_eq!(result, Err(16));
                assert!(book.owners.iter().all(|&head| head == usize::MAX));
                assert_eq!(book.accounts.iter().map(|a| a.units).sum::<f64>(), 100.0);
                assert_eq!(book.committed.capacity(), old_capacity);
                book.reject_revalidation();
                book.reserve_receipts(16);
            } else {
                assert_eq!(result, Ok(true));
                for seller in 0..8 { assert_eq!(book.sales(seller, 1, 100.0), (12.5, 125.0)); }
                assert_eq!(book.finance[0], [1_000.0; 2]);
                book.accept_revalidation();
            }
        }
    }
}
