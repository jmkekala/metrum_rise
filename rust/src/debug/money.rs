// SPDX-License-Identifier: GPL-2.0-only

//! Test-only ledger of money entering or leaving the city, for the `SIM-02-A` money
//! conservation invariant (`docs/simulation.md#behaviour-envelope-sim-02-a`).
//!
//! Inside the city money only moves between the treasury, building and household budgets and
//! three escrows (a shopper's reserved purchase, a paid shipment in transit, the service-sales
//! pool between households and stores). Every site where money is created or destroyed records
//! the signed amount here: positive enters the city. Gameplay and benchmark builds compile
//! every [`record`] call to nothing.

/// One kind of money crossing the city boundary.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MoneyFlow {
    /// Owner capital of a newly placed or topped-up business.
    OwnerCapital,
    /// Cash an immigrant household brings.
    ImmigrantCash,
    /// Outside-world payments for exports and distress liquidation, net of carrier fees.
    OutsideSales,
    /// Imported inputs and carrier fees, paid when the load is delivered.
    FreightPurchases,
    /// The outside share of utility bills, and local fees with no city provider.
    OutsideUtilities,
    /// Construction cost and refunds paid by the treasury.
    Construction,
    /// Daily road upkeep paid by the treasury.
    RoadUpkeep,
    /// Cash removed with a demolished building; a negative budget enters the city.
    RemovedBuildingCash,
    /// Cash and escrow carried away by departing households.
    DepartedHouseholdCash,
}

/// Number of [`MoneyFlow`] kinds.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const MONEY_FLOW_COUNT: usize = MoneyFlow::DepartedHouseholdCash as usize + 1;

/// Stable report names, in flow order.
#[cfg(test)]
pub(crate) const MONEY_FLOW_NAMES: [&str; MONEY_FLOW_COUNT] = [
    "owner_capital",
    "immigrant_cash",
    "outside_sales",
    "freight_purchases",
    "outside_utilities",
    "construction",
    "road_upkeep",
    "removed_building_cash",
    "departed_household_cash",
];

#[cfg(test)]
static LEDGER: std::sync::Mutex<[f64; MONEY_FLOW_COUNT]> =
    std::sync::Mutex::new([0.0; MONEY_FLOW_COUNT]);

/// Records `amount` crossing the city boundary; positive enters the city. Test builds only.
#[cfg(test)]
pub(crate) fn record(flow: MoneyFlow, amount: f64) {
    LEDGER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())[flow as usize] += amount;
}

/// Records `amount` crossing the city boundary; positive enters the city. Compiled out.
#[cfg(not(test))]
#[inline(always)]
pub(crate) fn record(_flow: MoneyFlow, _amount: f64) {}

/// Records the exact change of one `f32` stock from `before` to `after`, so rounding in the
/// stock's own arithmetic is not mistaken for a leak.
#[inline(always)]
pub(crate) fn record_change(flow: MoneyFlow, before: f32, after: f32) {
    record(flow, f64::from(after) - f64::from(before));
}

/// Cumulative recorded flows, for subtraction across a checked interval.
#[cfg(test)]
pub(crate) fn snapshot() -> [f64; MONEY_FLOW_COUNT] {
    *LEDGER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
