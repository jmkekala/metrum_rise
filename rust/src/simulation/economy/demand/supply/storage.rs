// SPDX-License-Identifier: GPL-2.0-only

//! Capacity preflight and full-proposal replay shared by new and replacement operations.

use super::*;

/// Exact arena size requested before publishing an otherwise valid provisional operation.
pub(super) enum StorageRequirement {
    /// Buyer accounts and financial receipts, respectively; zero means no growth requested.
    Business([usize; 2]),
    /// Physical supplier-to-buyer receipts.
    Inputs(usize),
    /// Household customer receipts.
    Households(usize),
}

impl From<[usize; 2]> for StorageRequirement {
    fn from(required: [usize; 2]) -> Self {
        Self::Business(required)
    }
}

impl SiteSupply<'_> {
    /// Checks storage before any inner commit. Only the complete outer proof can publish it.
    pub(super) fn commit_project(&mut self) -> Result<bool, StorageRequirement> {
        let Some(owner) = self.reservations.staged_owner else {
            return Ok(false);
        };
        if self.replacement.is_none() && !self.reservations.business.can_accept() {
            return Ok(false);
        }
        self.book
            .check_accept_capacity(owner)
            .map_err(StorageRequirement::Inputs)?;
        self.reservations
            .customers
            .check_accept_capacity(owner)
            .map_err(StorageRequirement::Households)?;
        self.reservations
            .business
            .commit_joint_for(owner)
            .map_err(|size| StorageRequirement::Business([0, size]))?;
        self.book.accept_for(owner);
        self.reservations.customers.accept_for(owner);
        self.reservations.staged_committed = true;
        Ok(true)
    }

    /// Uses the same replay boundary for private sales and already funded public invoices.
    pub(super) fn fund_operation(
        &mut self,
        profile: &EconomyProfileRuntime,
        prices: OperationPrices,
        mut costs: OperatingCosts,
        capital: f64,
        public: bool,
    ) -> bool {
        loop {
            self.reservations.staged_operation = None;
            if !costs.inputs.is_finite() || !costs.freight.is_finite() {
                return false;
            }
            let headroom = if public {
                [f64::INFINITY; 2]
            } else {
                [
                    self.customer_quote.revenue - costs.total(),
                    capital - costs.working_capital,
                ]
            };
            if !(headroom[0] > 0.0 && headroom[1] >= 0.0) {
                return false;
            }
            let prices = OperationPrices {
                imports: self.tuning.owa_import_price_multiplier
                    * if self.goods_multiplier.is_finite() {
                        self.goods_multiplier
                    } else {
                        1.0
                    },
                ..prices
            };
            self.reservations.staged_operation = Some(FundedOperation {
                profile: profile.runtime_id,
                plan: self.plan,
                prices,
                headroom,
            });
            self.retain_site();
            let result = if self.replacement.is_some() {
                self.fund_replacement(profile)
            } else {
                self.publish_new_buyer(profile)
            };
            match result {
                Ok(funded) => return funded,
                Err(required) => {
                    costs = self.retry_project(profile, prices, public, required);
                }
            }
        }
    }

    fn retry_project(
        &mut self,
        profile: &EconomyProfileRuntime,
        prices: OperationPrices,
        public: bool,
        required: StorageRequirement,
    ) -> OperatingCosts {
        let owner = self.reservations.staged_owner.unwrap();
        let (entrance, buyer, buyer_site, source, multiplier, exports, plan, replacement) = (
            self.entrance.clone(),
            self.buyer,
            self.buyer_site,
            self.replacement_source,
            self.goods_multiplier,
            self.exports,
            self.plan,
            self.replacement,
        );
        self.reject();
        match required {
            StorageRequirement::Business([accounts, receipts]) => {
                self.reservations.business.reserve_receipts(receipts);
                self.reservations.business.reserve_accounts(accounts);
            }
            StorageRequirement::Inputs(receipts) => self.book.reserve_receipts(receipts),
            StorageRequirement::Households(receipts) => {
                self.reservations.customers.reserve_receipts(receipts)
            }
        }
        self.begin_entrance(entrance, buyer, multiplier);
        if replacement.is_some() {
            self.withdraw_replacement(owner);
            self.book.release_owner(owner, self.allocator, self.graph);
            self.reservations.customers.release_owner(owner);
            self.reservations.business.release_owner(owner);
        }
        self.reservations.staged_owner = Some(owner);
        self.replacement = replacement;
        self.buyer_site = buyer_site;
        self.replacement_source = source;
        if !public {
            self.customers(profile, plan.area, prices, exports);
        }
        self.set_supported_plan(plan);
        let mut costs = OperatingCosts::for_plan_with_input_quotes(
            profile,
            plan,
            prices,
            |_, _| 0.0,
            |input| self.quote(input, self.catalog, prices),
        );
        costs.freight += self.customer_quote.export_freight;
        costs.working_capital += self.customer_quote.export_freight;
        costs
    }
}
