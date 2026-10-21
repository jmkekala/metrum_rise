// SPDX-License-Identifier: GPL-2.0-only

//! Finite household customer quantities and recurring spending reservations.

use super::definitions::{EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning};
use super::fiscal::CityFiscalPolicy;
use super::households::{Household, consumer_basket_cost, visit_consumer_budgets_in};
use crate::simulation::buildings::allocator::BuildingAllocator;
use rayon::prelude::*;

pub(crate) mod business;
mod revalidation;
mod routing;
pub(crate) use routing::{ConsumerMarket, DirectConsumerQuoteKey, RefreshInputs, RefreshState};

#[derive(Clone, Copy, Debug)]
struct Category {
    resource: u16,
    units_per_resident: f64,
    budget_share: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Funds {
    units: f64,
    gross_spending: f64,
}

#[derive(Clone, Copy, Debug)]
struct Claim {
    slot: usize,
    previous: Funds,
    acquired: Funds,
}

/// A consumer budget walk between census slices (`SIM-02-K`): whether every row so far matched
/// the previous book, the next household not yet written and the walk's running treasury.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BudgetWalk {
    unchanged: bool,
    next_household: usize,
    treasury: f64,
}

#[derive(Clone, Copy, Debug)]
struct Receipt {
    slot: usize,
    acquired: Funds,
    next: usize,
}

/// Reusable per-family consumer accounts; neither savings nor another family's income
/// can fund a claim. Resource categories retain their authored basket spending shares.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConsumerBudgets {
    categories: Vec<Category>,
    previous_categories: Vec<Category>,
    category_by_resource: Vec<usize>,
    households: Vec<(u16, f64)>,
    funds: Vec<Funds>,
    staged: Vec<Claim>,
    staged_slots: Vec<usize>,
    committed: Vec<Receipt>,
    owners: Vec<usize>,
    free_receipt: usize,
    releasing_owner: Option<usize>,
    affordable_residents: f64,
    revalidation: revalidation::Revalidation,
    #[cfg(test)]
    discard_receipt_estimate: bool,
}

impl ConsumerBudgets {
    /// Withdraws the census while retaining reusable allocations for the next refresh.
    pub(crate) fn clear(&mut self) {
        debug_assert!(!self.revalidation.is_active());
        self.households.clear();
        self.funds.clear();
        self.staged.clear();
        self.staged_slots.clear();
        self.committed.clear();
        self.free_receipt = usize::MAX;
        self.owners.clear();
        self.releasing_owner = None;
        self.affordable_residents = 0.0;
    }

    /// Prepares actual recurring family budgets in O(households * consumer categories +
    /// authored profile ports). Benefit claims are ordered; independent category accounts
    /// are filled with Rayon. All quote/journal storage is reserved before any claims.
    #[cfg(test)]
    pub(crate) fn reset(
        &mut self,
        households: &[Household],
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        policy: &CityFiscalPolicy,
        treasury: f64,
    ) {
        self.prepare_budgets(households, allocator, catalog, tuning, policy, treasury);
        self.reset_claims();
    }

    // Refresh immutable account inputs before deciding whether an accepted book can survive.
    // The existing ordered benefit traversal compares each row, including skipped families.
    #[cfg(test)]
    fn prepare_budgets(
        &mut self,
        households: &[Household],
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        policy: &CityFiscalPolicy,
        treasury: f64,
    ) -> bool {
        let mut walk = self.begin_budgets(households.len(), catalog, treasury);
        self.walk_budgets(
            &mut walk,
            households,
            0..households.len(),
            allocator,
            catalog,
            tuning,
            policy,
        );
        self.finish_budgets(&walk)
    }

    // The walk's start: the basket's categories and their comparison with the previous book.
    // O(profile ports). Walking every household range in order, then finishing, prepares the
    // budgets as one walk does (`SIM-02-K`).
    fn begin_budgets(
        &mut self,
        households: usize,
        catalog: &RuntimeEconomyCatalog,
        treasury: f64,
    ) -> BudgetWalk {
        debug_assert!(!self.revalidation.is_active());
        self.previous_categories.clone_from(&self.categories);
        self.categories.clear();
        self.categories.reserve(catalog.resource_count());
        self.category_by_resource
            .resize(catalog.resource_count() + 1, usize::MAX);
        self.category_by_resource.fill(usize::MAX);
        for profile in catalog.all_profiles() {
            if profile.kind != EconomyProfileRuntimeKind::DemandSink {
                continue;
            }
            for input in &profile.inputs {
                let resource = input.resource_runtime_id;
                let slot = &mut self.category_by_resource[resource as usize];
                if *slot == usize::MAX {
                    *slot = self.categories.len();
                    self.categories.push(Category {
                        resource,
                        units_per_resident: 0.0,
                        budget_share: 0.0,
                    });
                }
                self.categories[*slot].units_per_resident +=
                    f64::from(profile.consumption_rate_per_resident);
            }
        }
        let net_basket = consumer_basket_cost(catalog, 0.0);
        for category in &mut self.categories {
            category.budget_share = if net_basket > 0.0 {
                category.units_per_resident
                    * f64::from(
                        catalog
                            .unit_price_for_resource(category.resource)
                            .unwrap_or(0.0),
                    )
                    / net_basket
            } else {
                0.0
            };
        }
        let unchanged = self.households.len() == households
            && self.categories.len() == self.previous_categories.len()
            && self
                .categories
                .iter()
                .zip(&self.previous_categories)
                .all(|(a, b)| {
                    a.resource == b.resource
                        && a.units_per_resident.to_bits() == b.units_per_resident.to_bits()
                        && a.budget_share.to_bits() == b.budget_share.to_bits()
                });
        self.households.resize(households, (0, 0.0));
        self.affordable_residents = 0.0;
        BudgetWalk {
            unchanged,
            next_household: 0,
            treasury: treasury.max(0.0),
        }
    }

    // Writes the budgets of `range`, the walk's next households, comparing each row with the
    // previous book. O(range + profile ports), allocation-free.
    #[allow(clippy::too_many_arguments)]
    fn walk_budgets(
        &mut self,
        walk: &mut BudgetWalk,
        households: &[Household],
        range: std::ops::Range<usize>,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        policy: &CityFiscalPolicy,
    ) {
        let gross_basket = consumer_basket_cost(catalog, policy.household_vat_rate);
        let BudgetWalk {
            unchanged,
            next_household,
            treasury,
        } = walk;
        visit_consumer_budgets_in(
            households,
            range,
            allocator,
            catalog,
            tuning,
            policy,
            treasury,
            |index, members, spending| {
                for previous in &mut self.households[*next_household..index] {
                    *unchanged &= previous.0 == 0 && previous.1.to_bits() == 0.0_f64.to_bits();
                    *previous = (0, 0.0);
                }
                let previous = self.households[index];
                *unchanged &= previous.0 == members && previous.1.to_bits() == spending.to_bits();
                self.households[index] = (members, spending);
                *next_household = index + 1;
                self.affordable_residents += if gross_basket > 0.0 {
                    (spending / gross_basket).min(f64::from(members))
                } else {
                    f64::from(members)
                };
            },
        );
    }

    // Clears the rows after the last housed family and returns whether every row matched.
    fn finish_budgets(&mut self, walk: &BudgetWalk) -> bool {
        let mut unchanged = walk.unchanged;
        for previous in &mut self.households[walk.next_household..] {
            unchanged &= previous.0 == 0 && previous.1.to_bits() == 0.0_f64.to_bits();
            *previous = (0, 0.0);
        }
        unchanged
    }

    #[cfg(test)]
    fn reset_claims(&mut self) {
        self.begin_reset_claims();
        self.reset_claim_funds(0..self.households.len());
    }

    /// Gives this copy of `live` the capacity of the claim staging a census refills
    /// (`SIM-02-K`, [`crate::utils::storage::reserve_capacity`]). O(1) without growth.
    pub(crate) fn reserve_like(&mut self, live: &Self) {
        crate::utils::storage::reserve_capacity(&mut self.staged, live.staged.capacity());
    }

    // The claims' reset without the accounts (`SIM-02-K`): the account rows' length and the
    // receipt books. Then `reset_claim_funds` over every household range. O(owners).
    fn begin_reset_claims(&mut self) {
        debug_assert!(!self.revalidation.is_active());
        let count = self.categories.len();
        self.funds
            .resize(self.households.len() * count, Funds::default());
        self.staged.clear();
        self.staged.reserve(self.funds.len());
        self.staged_slots.resize(self.funds.len(), usize::MAX);
        self.staged_slots.fill(usize::MAX);
        self.committed.clear();
        self.owners.fill(usize::MAX);
        self.free_receipt = usize::MAX;
        self.releasing_owner = None;
    }

    // Returns the accounts of households `range` to their prepared budgets. O(range × K) in
    // parallel.
    fn reset_claim_funds(&mut self, range: std::ops::Range<usize>) {
        let count = self.categories.len();
        if count > 0 {
            self.funds[range.start * count..range.end * count]
                .par_chunks_mut(count)
                .zip(&self.households[range])
                .for_each(|(accounts, &(members, spending))| {
                    let mut remainder = spending;
                    for (index, (account, category)) in
                        accounts.iter_mut().zip(&self.categories).enumerate()
                    {
                        let budget = if index + 1 == count {
                            remainder
                        } else {
                            (spending * category.budget_share).min(remainder)
                        };
                        remainder -= budget;
                        *account = Funds {
                            // Match the physical f32 consumption product before keeping
                            // an f64 reservation ledger. Promoting the authored rate first
                            // can turn 1000 * 0.1 into 100.00000149 and an extra employee.
                            units: f64::from(
                                f32::from(members) * category.units_per_resident as f32,
                            ),
                            gross_spending: budget,
                        };
                    }
                });
        }
    }

    fn prepare_owners(&mut self, owners: usize, output_ports: usize) {
        // Owner rows follow the city, with spare capacity (`SIM-02-K`).
        crate::utils::storage::resize_rows(&mut self.owners, owners, usize::MAX);
        // Warm-start estimate only. Every commit preflights reusable cells: census commits
        // grow exactly, and enclosing proofs grow between attempts and replay.
        #[cfg(test)]
        if self.discard_receipt_estimate {
            // Retained census claims stay intact; only spare cells are dropped.
            self.committed.shrink_to_fit();
            return;
        }
        // With spare capacity, which also sizes the receipt journals (`SIM-02-K`).
        let additional = crate::utils::storage::spare_capacity(
            self.committed.len() + 2 * (self.funds.len() + output_ports),
            self.committed.capacity(),
        ) - self.committed.len();
        crate::utils::storage::reserve_cleared(&mut self.committed, additional);
    }

    // Test forks of the live book; planning lends it instead.
    #[cfg(test)]
    fn copy_reservations_from(&mut self, source: &Self) {
        debug_assert!(source.staged.is_empty());
        debug_assert!(!self.revalidation.is_active() && !source.revalidation.is_active());
        self.categories.clone_from(&source.categories);
        self.category_by_resource
            .clone_from(&source.category_by_resource);
        self.households.clone_from(&source.households);
        self.funds.clone_from(&source.funds);
        self.staged.clear();
        self.staged.reserve(self.funds.len());
        self.staged_slots.clone_from(&source.staged_slots);
        self.committed.clone_from(&source.committed);
        self.committed.reserve(
            source
                .committed
                .capacity()
                .saturating_sub(self.committed.len()),
        );
        self.owners.clone_from(&source.owners);
        self.free_receipt = source.free_receipt;
        self.releasing_owner = None;
        self.affordable_residents = source.affordable_residents;
        crate::debug::work::add(
            crate::debug::work::Work::ReservationRowsCopied,
            (self.households.len()
                + self.funds.len()
                + self.staged_slots.len()
                + self.committed.len()
                + self.owners.len()) as u64,
        );
    }

    fn stage(&mut self, slot: usize) -> usize {
        self.journal_funds(slot);
        if self.staged_slots[slot] == usize::MAX {
            self.staged_slots[slot] = self.staged.len();
            self.staged.push(Claim {
                slot,
                previous: self.funds[slot],
                acquired: Funds::default(),
            });
        }
        self.staged_slots[slot]
    }

    fn release_owner(&mut self, owner: usize) {
        debug_assert!(self.staged.is_empty());
        self.releasing_owner = Some(owner);
        let mut index = self.owners.get(owner).copied().unwrap_or(usize::MAX);
        while index != usize::MAX {
            let receipt = self.committed[index];
            self.stage(receipt.slot);
            self.funds[receipt.slot].units += receipt.acquired.units;
            self.funds[receipt.slot].gross_spending += receipt.acquired.gross_spending;
            index = receipt.next;
        }
    }

    fn accept_for(&mut self, owner: usize) {
        debug_assert!(
            self.releasing_owner
                .is_none_or(|released| released == owner)
        );
        debug_assert!(self.owners[owner] == usize::MAX || self.releasing_owner == Some(owner));
        self.retire(owner);
        // Prepending in reverse preserves the original household/category quote order.
        for staged in (0..self.staged.len()).rev() {
            let claim = self.staged[staged];
            if claim.acquired.units > 0.0 {
                let receipt = Receipt {
                    slot: claim.slot,
                    acquired: claim.acquired,
                    next: self.owners[owner],
                };
                let index = if self.free_receipt == usize::MAX {
                    let index = self.committed.len();
                    assert!(
                        index < self.committed.capacity(),
                        "household receipts require capacity preflight"
                    );
                    self.committed.push(receipt);
                    index
                } else {
                    let index = self.free_receipt;
                    self.journal_receipt(index);
                    self.free_receipt = self.committed[index].next;
                    self.committed[index] = receipt;
                    index
                };
                self.owners[owner] = index;
            }
        }
        self.releasing_owner = None;
        self.accept();
    }

    fn check_accept_capacity(&self, owner: usize) -> Result<(), usize> {
        let mut missing = self
            .staged
            .iter()
            .filter(|claim| claim.acquired.units > 0.0)
            .count();
        for first in [self.owners[owner], self.free_receipt] {
            let mut index = first;
            while index != usize::MAX && missing > 0 {
                missing -= 1;
                index = self.committed[index].next;
            }
        }
        let required = self.committed.len() + missing;
        if required > self.committed.capacity() {
            Err(required)
        } else {
            Ok(())
        }
    }

    fn reserve_receipts(&mut self, required: usize) {
        debug_assert!(!self.revalidation.is_active());
        self.committed
            .reserve(required.saturating_sub(self.committed.len()));
        self.revalidation.follow_receipts(self.committed.capacity());
    }

    fn retire(&mut self, owner: usize) {
        self.journal_owner(owner);
        let mut index = self.owners[owner];
        while index != usize::MAX {
            let next = self.committed[index].next;
            self.journal_receipt(index);
            self.committed[index].next = self.free_receipt;
            self.free_receipt = index;
            index = next;
        }
        self.owners[owner] = usize::MAX;
    }

    fn reserved_units(&self, owner: usize, resource: u16) -> f64 {
        let Some(&category) = self.category_by_resource.get(resource as usize) else {
            return 0.0;
        };
        if category == usize::MAX {
            return 0.0;
        }
        let mut units = 0.0;
        let mut index = self.owners.get(owner).copied().unwrap_or(usize::MAX);
        while index != usize::MAX {
            let receipt = self.committed[index];
            if receipt.slot % self.categories.len() == category {
                units += receipt.acquired.units;
            }
            index = receipt.next;
        }
        units
    }

    /// Catalog-priced diagnostic total, calculated from the same per-family budget census.
    #[cfg(test)]
    pub(crate) fn affordable_residents(&self) -> f64 {
        self.affordable_residents
    }

    fn slot(&self, household: usize, resource: u16) -> Option<usize> {
        let &category = self.category_by_resource.get(resource as usize)?;
        (household < self.households.len() && category != usize::MAX)
            .then(|| household * self.categories.len() + category)
    }

    /// Units this particular family can still buy at the actual tax-inclusive unit price.
    /// This is financial eligibility only; the caller must also prove both directed trips.
    /// O(1), with no scan or allocation.
    pub(crate) fn available(&self, household: usize, resource: u16, gross_unit_price: f32) -> f64 {
        if !gross_unit_price.is_finite() || gross_unit_price <= 0.0 {
            return 0.0;
        }
        self.slot(household, resource).map_or(0.0, |slot| {
            let account = self.funds[slot];
            account
                .units
                .min(account.gross_spending / f64::from(gross_unit_price))
        })
    }

    /// Stages a reached family's finite quantity and spending claim. Repeated visits to
    /// the same account retain one original receipt, bounding journal storage by accounts.
    /// O(1); accepted net seller revenue is units times the seller's price excluding VAT.
    pub(crate) fn claim(
        &mut self,
        household: usize,
        resource: u16,
        requested: f64,
        gross_unit_price: f32,
    ) -> f64 {
        let units = requested
            .max(0.0)
            .min(self.available(household, resource, gross_unit_price));
        if units <= 0.0 {
            return 0.0;
        }
        let slot = self.slot(household, resource).unwrap();
        let claim = self.stage(slot);
        let account = &mut self.funds[slot];
        let previous_spending = account.gross_spending;
        let spending_limit = account.gross_spending / f64::from(gross_unit_price);
        account.units -= units;
        account.gross_spending = if units >= spending_limit {
            // The quote exhausted this exact budget bound. Do not leave multiplication
            // roundoff indexed as another buyer with a fractional-cent residual budget.
            0.0
        } else {
            (account.gross_spending - units * f64::from(gross_unit_price)).max(0.0)
        };
        self.staged[claim].acquired.units += units;
        self.staged[claim].acquired.gross_spending += previous_spending - account.gross_spending;
        units
    }

    /// Commits a completely funded operation's customers in O(touched accounts).
    pub(crate) fn accept(&mut self) {
        if let Some(owner) = self.releasing_owner.take() {
            self.retire(owner);
        }
        for claim in &self.staged {
            self.staged_slots[claim.slot] = usize::MAX;
        }
        self.staged.clear();
        self.releasing_owner = None;
    }

    /// Restores exact pre-quote quantities and spending in O(touched accounts), including
    /// failed staffing probes and repeated claims against one family/category account.
    pub(crate) fn reject(&mut self) {
        while let Some(claim) = self.staged.pop() {
            self.funds[claim.slot] = claim.previous;
            self.staged_slots[claim.slot] = usize::MAX;
        }
        self.releasing_owner = None;
    }
}
