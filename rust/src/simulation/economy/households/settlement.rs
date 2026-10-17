// SPDX-License-Identifier: GPL-2.0-only

//! Per-entity daily settlement (`SIM-02-G`, `docs/simulation.md#phases`).
//!
//! Each building, household and worker settles its own 24-hour window at its daily phase
//! instead of all at midnight: about N / 23,040 of each per slot. Building accounts are in
//! `building_economy.rs` and wages in `employment.rs`; this module settles a household's window
//! and holds the requests a phase leaves for the next hour's pass. City-wide totals post into
//! [`super::data::CityDayTotals`] as each entity settles, so the calendar close is O(1).

use super::HouseholdSystem;
use super::building_economy::DaySettlement;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::fiscal::{CityFiscalPolicy, FiscalRevenue};

impl HouseholdSystem {
    /// Settles household `household_id`'s 24-hour window at its daily phase, in the order of the
    /// former midnight settlement: transfers, residential property tax, water and sewage, then
    /// the electricity bill. The window then closes; the caller reports it and opens the next.
    /// O(1).
    pub(crate) fn settle_household_day(
        &mut self,
        household_id: usize,
        allocator: &BuildingAllocator,
        day: &DaySettlement,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: &mut f64,
    ) -> FiscalRevenue {
        self.ensure_daily_ledger_len();
        self.pay_household_transfer(household_id, allocator, treasury_balance, fiscal_policy);
        let residential_property_tax =
            self.charge_household_property_tax(household_id, allocator, fiscal_policy);
        self.settle_household_utilities(household_id, day, treasury_balance);
        self.settle_household_electricity(
            household_id,
            day.prices[0],
            day.tuning.owa_import_price_multiplier,
            treasury_balance,
        );
        self.finish_household_window(household_id);
        FiscalRevenue {
            residential_property_tax,
            ..FiscalRevenue::default()
        }
    }

    /// Queues household `household_id` for the housing review the next hour serves: housing
    /// affordability, relocation and eviction (`SIM-02-G`; a per-slot queue in `SIM-02-J`).
    /// O(1).
    pub(crate) fn request_housing_review(&mut self, household_id: usize) {
        if self.households[household_id].member_count > 0 {
            self.housing_reviews.push(self.handles.handle(household_id));
        }
    }

    /// Queues household `household_id` to leave the city in the next hour's pass. O(1).
    pub(crate) fn queue_exit(&mut self, household_id: usize) {
        self.exits.push(self.handles.handle(household_id));
    }
}
