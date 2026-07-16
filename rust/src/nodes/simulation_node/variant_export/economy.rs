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
    dict.set("utility_service_revenue", entry.utility_service_revenue);
    dict.set("benefits", entry.benefits);
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
