// SPDX-License-Identifier: GPL-2.0-only

//! Buyer-local receipt withdrawal using the same accounts, seller chains and undo journal.

use super::{BusinessBudgets, Receipt};

impl BusinessBudgets {
    /// Inserts after `previous`, or at the seller head, preserving existing seller order.
    /// Account links independently support buyer withdrawal without searching other sellers.
    pub(super) fn insert_receipt(
        &mut self,
        seller: usize,
        previous: usize,
        receipt: Receipt,
    ) -> usize {
        self.journal_owner(seller);
        self.journal_account(receipt.account);
        let next = if previous == usize::MAX {
            self.owners[seller]
        } else {
            self.committed[previous].next
        };
        let next_account = self.accounts[receipt.account].first_receipt;
        let receipt = Receipt {
            seller,
            previous,
            next,
            next_account,
            previous_account: usize::MAX,
            ..receipt
        };
        let index = if self.free_receipt == usize::MAX {
            self.append_receipt(receipt)
        } else {
            let index = self.free_receipt;
            self.journal_receipt(index);
            self.free_receipt = self.committed[index].next;
            self.committed[index] = receipt;
            index
        };
        if previous == usize::MAX {
            self.owners[seller] = index;
        } else {
            self.journal_receipt(previous);
            self.committed[previous].next = index;
        }
        if next != usize::MAX {
            self.journal_receipt(next);
            self.committed[next].previous = index;
        }
        if next_account != usize::MAX {
            self.journal_receipt(next_account);
            self.committed[next_account].previous_account = index;
        }
        self.accounts[receipt.account].first_receipt = index;
        index
    }

    /// O(1) unlink from both directions; snapshot every neighbor before recycling this cell.
    pub(super) fn remove_receipt(&mut self, index: usize) {
        let receipt = self.committed[index];
        self.journal_receipt(index);
        self.journal_owner(receipt.seller);
        self.journal_account(receipt.account);
        if receipt.previous == usize::MAX {
            self.owners[receipt.seller] = receipt.next;
        } else {
            self.journal_receipt(receipt.previous);
            self.committed[receipt.previous].next = receipt.next;
        }
        if receipt.next != usize::MAX {
            self.journal_receipt(receipt.next);
            self.committed[receipt.next].previous = receipt.previous;
        }
        if receipt.previous_account == usize::MAX {
            self.accounts[receipt.account].first_receipt = receipt.next_account;
        } else {
            self.journal_receipt(receipt.previous_account);
            self.committed[receipt.previous_account].next_account = receipt.next_account;
        }
        if receipt.next_account != usize::MAX {
            self.journal_receipt(receipt.next_account);
            self.committed[receipt.next_account].previous_account = receipt.previous_account;
        }
        self.committed[index].next = self.free_receipt;
        self.free_receipt = index;
    }

    /// Withdraws one buyer's demand and reports exactly the sellers losing its purchases.
    /// This is part of an enclosing operation proof: affected sellers still need revenue and
    /// capital revalidation. O(buyer input tranches + their receipts), no allocation or city scan.
    pub(super) fn withdraw_buyer(
        &mut self,
        buyer: usize,
        mut affected: impl FnMut(usize, u16, f64, f64),
    ) {
        debug_assert!(self.revalidation.is_active() && self.staged.is_empty());
        let mut slot = self.buyer_accounts[buyer];
        while slot != usize::MAX {
            let account = self.accounts[slot];
            let mut index = account.first_receipt;
            while index != usize::MAX {
                let receipt = self.committed[index];
                affected(
                    receipt.seller,
                    account.resource,
                    receipt.units,
                    receipt.goods,
                );
                self.remove_receipt(index);
                index = receipt.next_account;
            }
            self.journal_account(slot);
            self.accounts[slot].units = 0.0;
            slot = account.next_buyer;
        }
        self.set_finance(buyer, [0.0; 2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::operation::InputRequirement;

    fn assert_links(book: &BusinessBudgets) {
        let mut sellers = vec![false; book.committed.len()];
        let mut accounts = sellers.clone();
        for (seller, &head) in book.owners.iter().enumerate() {
            let (mut index, mut previous) = (head, usize::MAX);
            while index != usize::MAX {
                assert!(
                    !sellers[index],
                    "receipt must occur exactly once in seller chains"
                );
                sellers[index] = true;
                let receipt = book.committed[index];
                assert_eq!((receipt.seller, receipt.previous), (seller, previous));
                previous = index;
                index = receipt.next;
            }
        }
        for (slot, account) in book.accounts.iter().enumerate() {
            let (mut index, mut previous) = (account.first_receipt, usize::MAX);
            while index != usize::MAX {
                assert!(
                    !accounts[index],
                    "receipt must occur exactly once in account chains"
                );
                accounts[index] = true;
                let receipt = book.committed[index];
                assert_eq!(
                    (receipt.account, receipt.previous_account),
                    (slot, previous)
                );
                previous = index;
                index = receipt.next_account;
            }
        }
        assert_eq!(sellers, accounts);
        let mut free = book.free_receipt;
        while free != usize::MAX {
            assert!(
                !sellers[free],
                "a recycled cell cannot remain linked to a commitment"
            );
            sellers[free] = true;
            free = book.committed[free].next;
        }
        assert!(sellers.into_iter().all(|seen| seen));
    }

    #[test]
    fn buyer_withdrawal_visits_only_its_sales_and_restores_recycled_links_exactly() {
        let mut book = BusinessBudgets::default();
        book.reset(3, 5, 3, 3);
        for buyer in 0..3 {
            book.set_finance(buyer, [1_000.0, 10_000.0]);
        }
        // Non-contiguous accounts ensure the reverse lookup follows identity, not seed order.
        for buyer in [0, 1, 0, 2, 1] {
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
        for (seller, claims) in [
            (0, &[(0, 40.0), (1, 20.0)][..]),
            (1, &[(0, 60.0), (1, 80.0), (2, 100.0), (3, 100.0)][..]),
            (2, &[(4, 100.0)][..]),
        ] {
            for &(slot, units) in claims {
                assert_eq!(book.claim_opportunity(slot, units, 10.0, 50.0), units);
            }
            assert!(book.accept_for(seller));
        }
        assert_links(&book);
        book.prepare_revalidation();
        let original = book.clone();
        let capacity = book.committed.capacity();
        let bits = |r: &Receipt| {
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
        for _ in 0..64 {
            book.begin_revalidation();
            let mut lost = [0.0; 3];
            book.withdraw_buyer(0, |seller, resource, units, goods| {
                assert_eq!(resource, 1);
                assert_eq!(goods, units * 10.0);
                lost[seller] += units;
            });
            assert_eq!(lost, [40.0, 160.0, 0.0]);
            assert_eq!(book.sales(0, 1, f64::INFINITY), (20.0, 200.0));
            assert_eq!(book.sales(1, 1, f64::INFINITY), (180.0, 1_800.0));
            assert_eq!(book.sales(2, 1, f64::INFINITY), (100.0, 1_000.0));
            assert_eq!(book.finance[0], [0.0; 2]);
            assert_eq!(book.finance[1..], original.finance[1..]);
            assert_links(&book);
            // Recycle withdrawn cells in an inner accepted seller substitution.
            book.release_owner(1);
            assert_eq!(book.claim_opportunity(1, 80.0, 9.0, 50.0), 80.0);
            assert_eq!(book.claim_opportunity(3, 100.0, 9.0, 50.0), 100.0);
            assert!(book.accept_for(1));
            assert_links(&book);
            book.reject_revalidation();
            assert_links(&book);
            assert_eq!(book.committed.capacity(), capacity);
            assert!(
                book.committed
                    .iter()
                    .map(bits)
                    .eq(original.committed.iter().map(bits))
            );
            assert_eq!(book.owners, original.owners);
            assert_eq!(book.free_receipt, original.free_receipt);
            assert_eq!(
                book.finance
                    .iter()
                    .map(|v| v.map(f64::to_bits))
                    .collect::<Vec<_>>(),
                original
                    .finance
                    .iter()
                    .map(|v| v.map(f64::to_bits))
                    .collect::<Vec<_>>()
            );
            for (account, before) in book.accounts.iter().zip(&original.accounts) {
                assert_eq!(
                    (
                        account.units.to_bits(),
                        account.first_receipt,
                        account.next_buyer
                    ),
                    (
                        before.units.to_bits(),
                        before.first_receipt,
                        before.next_buyer
                    )
                );
            }
        }
        book.begin_revalidation();
        book.withdraw_buyer(0, |_, _, _, _| {});
        book.accept_revalidation();
        book.reject_revalidation();
        assert_links(&book);
        assert_eq!(book.sales(1, 1, f64::INFINITY), (180.0, 1_800.0));
    }
}
