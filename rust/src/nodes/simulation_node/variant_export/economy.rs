// SPDX-License-Identifier: GPL-2.0-only

//! Economy variant export helpers.

use super::super::*;

pub(in crate::nodes::simulation_node) fn budget_ledger_entry_dict(
    entry: &DailyBudgetLedgerEntry,
) -> VarDictionary {
    let mut dict = VarDictionary::new();
    dict.set("day_index", entry.day_index as i32);
    dict.set("income", entry.income);
    dict.set("expenses", entry.expenses);
    dict.set("net", entry.net);
    dict.set("treasury", entry.treasury);
    dict.set("tax_income", entry.tax_income);
    dict.set("income_tax", entry.income_tax);
    dict.set("household_vat", entry.household_vat);
    dict.set("business_profit_tax", entry.business_profit_tax);
    dict.set("property_tax", entry.property_tax);
    dict.set("residential_property_tax", entry.residential_property_tax);
    dict.set("commercial_property_tax", entry.commercial_property_tax);
    dict.set("industrial_property_tax", entry.industrial_property_tax);
    dict.set("utility_service_revenue", entry.utility_service_revenue);
    dict.set("benefits", entry.benefits);
    dict.set("unemployment_benefits", entry.unemployment_benefits);
    dict.set("pensions", entry.pensions);
    dict.set("child_support", entry.child_support);
    dict.set("city_wages", entry.city_wages);
    dict.set("fuel_input_purchases", entry.fuel_input_purchases);
    dict.set("imports_owa", entry.imports_owa);
    dict.set(
        "construction_service_costs",
        entry.construction_service_costs,
    );
    dict.set("power_produced", entry.power_produced);
    dict.set("power_consumed", entry.power_consumed);
    dict.set("power_unmet", entry.power_unmet);
    dict.set("power_coverage", entry.power_coverage);
    dict.set("coal_inventory", entry.coal_inventory);
    dict.set("coal_bought", entry.coal_bought);
    dict.set("coal_consumed", entry.coal_consumed);
    dict.set("electricity_fuel_cost", entry.electricity_fuel_cost);
    dict.set("electricity_wage_cost", entry.electricity_wage_cost);
    dict.set("electricity_revenue", entry.electricity_revenue);
    dict.set("electricity_net", entry.electricity_net);
    dict
}
