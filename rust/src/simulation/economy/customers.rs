// SPDX-License-Identifier: GPL-2.0-only

//! Finite household customer quantities and recurring spending reservations.

use super::definitions::{EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning};
use super::fiscal::CityFiscalPolicy;
use super::households::{Household, consumer_basket_cost, visit_consumer_budgets};
use crate::simulation::buildings::allocator::BuildingAllocator;
use rayon::prelude::*;

mod routing;
pub(crate) use routing::ConsumerMarket;

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

#[derive(Clone, Copy, Debug)]
struct Receipt {
    slot: usize,
    acquired: Funds,
}

/// Reusable per-family consumer accounts; neither savings nor another family's income
/// can fund a claim. Resource categories retain their authored basket spending shares.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConsumerBudgets {
    categories: Vec<Category>,
    category_by_resource: Vec<usize>,
    households: Vec<(u16, f64)>,
    funds: Vec<Funds>,
    staged: Vec<Claim>,
    staged_slots: Vec<usize>,
    committed: Vec<Receipt>,
    owners: Vec<(usize, usize)>,
    releasing_owner: Option<usize>,
    affordable_residents: f64,
}

impl ConsumerBudgets {
    /// Withdraws the census while retaining reusable allocations for the next refresh.
    pub(crate) fn clear(&mut self) {
        self.households.clear();
        self.funds.clear();
        self.staged.clear();
        self.staged_slots.clear();
        self.committed.clear();
        self.owners.clear();
        self.releasing_owner = None;
        self.affordable_residents = 0.0;
    }

    /// Prepares actual recurring family budgets in O(households * consumer categories +
    /// authored profile ports). Benefit claims are ordered; independent category accounts
    /// are filled with Rayon. All quote/journal storage is reserved before any claims.
    pub(crate) fn reset(
        &mut self,
        households: &[Household],
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        policy: &CityFiscalPolicy,
        treasury: f64,
    ) {
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
        self.households.resize(households.len(), (0, 0.0));
        self.households
            .par_iter_mut()
            .for_each(|value| *value = (0, 0.0));
        self.affordable_residents = 0.0;
        let gross_basket = consumer_basket_cost(catalog, policy.household_vat_rate);
        visit_consumer_budgets(
            households,
            allocator,
            catalog,
            tuning,
            policy,
            treasury,
            |index, members, spending| {
                self.households[index] = (members, spending);
                self.affordable_residents += if gross_basket > 0.0 {
                    (spending / gross_basket).min(f64::from(members))
                } else {
                    f64::from(members)
                };
            },
        );
        let count = self.categories.len();
        self.funds
            .resize(households.len() * count, Funds::default());
        if count > 0 {
            self.funds
                .par_chunks_mut(count)
                .zip(&self.households)
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
                            units: f64::from(members) * category.units_per_resident,
                            gross_spending: budget,
                        };
                    }
                });
        }
        self.staged.clear();
        self.staged.reserve(self.funds.len());
        self.staged_slots.resize(self.funds.len(), usize::MAX);
        self.staged_slots.fill(usize::MAX);
        self.committed.clear();
        self.owners.fill((0, 0));
        self.releasing_owner = None;
    }

    fn prepare_owners(&mut self, owners: usize, output_ports: usize) {
        self.owners.resize(owners, (0, 0));
        // Each accepted quote exhausts an account or finishes one output port. A live
        // owner can be replaced once per planning pass; anonymous new sites need no receipt.
        self.committed
            .reserve(2 * (self.funds.len() + output_ports));
    }

    fn copy_reservations_from(&mut self, source: &Self) {
        debug_assert!(source.staged.is_empty());
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
        self.releasing_owner = None;
        self.affordable_residents = source.affordable_residents;
    }

    fn stage(&mut self, slot: usize) -> usize {
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
        let (first, end) = self.owners.get(owner).copied().unwrap_or((0, 0));
        for index in first..end {
            let receipt = self.committed[index];
            self.stage(receipt.slot);
            self.funds[receipt.slot].units += receipt.acquired.units;
            self.funds[receipt.slot].gross_spending += receipt.acquired.gross_spending;
        }
    }

    fn accept_for(&mut self, owner: usize) {
        debug_assert!(
            self.releasing_owner
                .is_none_or(|released| released == owner)
        );
        let first = self.committed.len();
        for claim in &self.staged {
            if claim.acquired.units > 0.0 {
                self.committed.push(Receipt {
                    slot: claim.slot,
                    acquired: claim.acquired,
                });
            }
        }
        self.owners[owner] = (first, self.committed.len());
        self.accept();
    }

    /// Catalog-priced diagnostic total, calculated from the same per-family budget census.
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
