// SPDX-License-Identifier: GPL-2.0-only

//! Exact seller-price eligibility over the existing business customer buckets.

use super::*;

/// Records each changed buyer at most once in storage reserved before candidate queries.
pub(super) fn mark_dirty(dirty: &mut Vec<usize>, flags: &mut [bool], buyer: usize) {
    if !flags[buyer] {
        flags[buyer] = true;
        dirty.push(buyer);
    }
}

fn account_mask(budgets: &BusinessBudgets, classes: &[Vec<u32>], slot: usize) -> u64 {
    let account = budgets.accounts[slot];
    if account.units <= 0.0 {
        return 0;
    }
    classes[account.resource as usize]
        .iter()
        .enumerate()
        .fold(0, |mask, (bit, &price)| {
            mask | (u64::from(budgets.potentially_claimable(slot, f32::from_bits(price))) << bit)
        })
}

impl BusinessMarket {
    // Prices are exact authored f32 values, not intervals: negative stock exposure means
    // goods-price monotonicity is not a valid pruning proof. Overflow/unknown classes use
    // the ordinary scan. This bounds per-account preparation by 64 pure affordability tests.
    /// Collects up to 64 exact authored prices per resource outside candidate queries.
    pub(super) fn prepare_price_classes(&mut self, catalog: &RuntimeEconomyCatalog) {
        self.price_classes
            .resize_with(catalog.resource_count() + 1, Vec::new);
        self.price_classes.iter_mut().for_each(Vec::clear);
        for profile in catalog.all_profiles() {
            for output in &profile.outputs {
                let classes = &mut self.price_classes[output.resource_runtime_id as usize];
                let price = profile.unit_price_currency.to_bits();
                if classes.len() < 64 && !classes.contains(&price) {
                    classes.push(price);
                }
            }
        }
        self.price_classes
            .iter_mut()
            .for_each(|classes| classes.sort_unstable());
    }

    /// Reserves one dirty-list entry per buyer before any transactional mutation.
    pub(super) fn prepare_finance_mask_storage(&mut self) {
        crate::utils::storage::resize_rows(
            &mut self.buyer_dirty,
            self.budgets.finance.len(),
            false,
        );
        crate::utils::storage::reserve_rows(&mut self.dirty_buyers, self.buyer_dirty.len());
        self.account_masks.reserve(
            self.budgets
                .accounts
                .capacity()
                .saturating_sub(self.account_masks.len()),
        );
    }

    /// Defers one buyer's eligibility refresh until restored or staged budgets are readable.
    pub(super) fn mark_finance_dirty(&mut self, buyer: usize) {
        mark_dirty(&mut self.dirty_buyers, &mut self.buyer_dirty, buyer);
    }

    /// Rebuilds masks alongside the existing O(accounts + ports) financial census.
    pub(super) fn reset_finance_masks(&mut self) {
        self.prepare_finance_mask_storage();
        self.account_masks.resize(self.budgets.accounts.len(), 0);
        self.account_masks
            .par_iter_mut()
            .enumerate()
            .for_each(|(slot, mask)| {
                *mask = account_mask(&self.budgets, &self.price_classes, slot);
            });
        self.port_masks.resize(self.identities.len(), 0);
        self.port_masks.fill(0);
        // Account tranches share recipe ports; their OR reduction retains deterministic order.
        for (slot, &mask) in self.account_masks.iter().enumerate() {
            self.port_masks[self.account_ports[slot]] |= mask;
        }
        self.indices.all_mut().for_each(|index| {
            index.reset_query_masks(|port| self.port_masks[port]);
        });
        self.dirty_buyers.clear();
        self.buyer_dirty.fill(false);
    }

    /// Returns the exact price class, falling back to ordinary scanning for unknown prices.
    pub(super) fn finance_query_mask(&self, resource: u16, price: f32) -> Option<u64> {
        #[cfg(test)]
        if self.disable_finance_masks {
            return None;
        }
        self.price_classes
            .get(resource as usize)?
            .binary_search(&price.to_bits())
            .ok()
            .map(|bit| 1u64 << bit)
    }

    // Refresh after rollback has restored the budgets, never from provisional snapshots.
    // Within one ordinary query claims only decrease finance and units. An excluded account
    // cannot change its own units, so a false mask stays false until the query ends.
    // Cost follows changed buyers' tranches and their existing bucket-tree paths.
    /// Refreshes only changed buyers; no allocation or unrelated-account traversal. A
    /// census's financing publication changes every buyer, so the minute-59 preparation
    /// refreshes its book before the hour's first masked claim would (`SIM-02-K`).
    pub(crate) fn refresh_finance_masks(&mut self) {
        while let Some(buyer) = self.dirty_buyers.pop() {
            self.buyer_dirty[buyer] = false;
            let first = self.budgets.buyer_accounts[buyer];
            let mut slot = first;
            while slot != usize::MAX {
                self.port_masks[self.account_ports[slot]] = 0;
                slot = self.budgets.accounts[slot].next_buyer;
            }
            slot = first;
            while slot != usize::MAX {
                self.port_masks[self.account_ports[slot]] |=
                    account_mask(&self.budgets, &self.price_classes, slot);
                slot = self.budgets.accounts[slot].next_buyer;
            }
            slot = first;
            while slot != usize::MAX {
                let port = self.account_ports[slot];
                let access = self.port_access[port];
                let resource = self.identities[port].1;
                let index = self.indices.port_mut(resource, port);
                for &component in access.components.as_slice() {
                    index.set_query_mask(
                        ReachableBucketEntry::new(component, access.chunk, port),
                        self.port_masks[port],
                    );
                }
                slot = self.budgets.accounts[slot].next_buyer;
            }
        }
    }
}
