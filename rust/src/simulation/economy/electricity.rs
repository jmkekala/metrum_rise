// SPDX-License-Identifier: GPL-2.0-only

//! Physical electricity quantities shared by hourly dispatch, billing, saves and inspectors.

use serde::{Deserialize, Serialize};

/// Delivered energy, including OWA fallback; local energy is a subset of consumption.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct ElectricityUsage {
    /// Electricity delivered during the accounting interval, in kWh.
    pub consumed_kwh: f32,
    /// Portion generated locally during the same hours, in kWh.
    pub local_kwh: f32,
}

impl ElectricityUsage {
    /// Accumulates one interval without deriving physical demand from payments.
    pub(crate) fn record(&mut self, kwh: f32, local_fraction: f32) {
        self.consumed_kwh += kwh;
        self.local_kwh += kwh * local_fraction;
    }

    /// Charges local and imported energy at their respective tariffs.
    pub(crate) fn bill(self, local_price: f32, import_multiplier: f32) -> ElectricityBill {
        ElectricityBill {
            local: self.local_kwh * local_price,
            imported: (self.consumed_kwh - self.local_kwh).max(0.0)
                * local_price
                * import_multiplier,
        }
    }
}

/// Separate payers for a mixed-use building, without charging its residential portion twice.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct BuildingElectricityUsage {
    /// Consumption charged to the business or city service.
    pub workplace: ElectricityUsage,
    /// Consumption apportioned to resident households, including the shared home base load.
    pub residential: ElectricityUsage,
}

/// Electricity charges and unpaid balances, retaining the local/OWA recipient split.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub(crate) struct ElectricityBill {
    /// Currency owed to the local utility operator.
    pub local: f32,
    /// Currency owed to OWA for delivered imports.
    pub imported: f32,
}

impl ElectricityBill {
    /// Total currency owed, independent of current cash.
    pub(crate) fn total(self) -> f32 {
        self.local + self.imported
    }

    /// Receives available cash proportionally; returns collected local/OWA payments.
    pub(crate) fn pay(&mut self, cash: &mut f32) -> Self {
        let total = self.total();
        let fraction = if total > 0.0 {
            (cash.max(0.0) / total).min(1.0)
        } else {
            0.0
        };
        let before = *cash;
        *cash -= total * fraction;
        // Cash accounts are f32. Credit recipients with the actual rounded debit,
        // so the payer and collected-revenue ledger describe the same transfer.
        let collected = before - *cash;
        let local = if total > 0.0 {
            collected * (self.local / total)
        } else {
            0.0
        };
        let paid = Self {
            local,
            imported: collected - local,
        };
        self.local = (self.local - paid.local).max(0.0);
        self.imported = (self.imported - paid.imported).max(0.0);
        paid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collected_power_payments_match_cash_debits_at_large_balances() {
        let mut cash = 1_000_000.0;
        let before = cash;
        let mut collected = 0.0;
        for _ in 0..365 {
            let mut bill = ElectricityBill {
                local: 0.2,
                imported: 0.35,
            };
            collected += bill.pay(&mut cash).total();
        }
        assert_eq!(before - cash, collected);
    }
}
