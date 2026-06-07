//! Bottleneck summary construction for sandbox playback results.

use crate::simulation::economy::definitions::schema::EconomyScenario;
use std::collections::BTreeMap;

pub(super) struct BottleneckInputs<'a> {
    pub(super) scenario: &'a EconomyScenario,
    pub(super) household_demand_per_day: f32,
    pub(super) lowest_stock_days: f32,
    pub(super) day_stock_zeroed: Option<u32>,
    pub(super) total_daily_supply: f32,
    pub(super) supply_day_count: u32,
    pub(super) total_unmet_units: f32,
    pub(super) node_cumulative_profits: &'a BTreeMap<String, f32>,
}

pub(super) fn build_bottlenecks(inputs: BottleneckInputs<'_>) -> Vec<String> {
    let scenario = inputs.scenario;
    let mut bottlenecks = Vec::new();

    let avg_daily_supply = if inputs.supply_day_count > 0 {
        inputs.total_daily_supply / inputs.supply_day_count as f32
    } else {
        0.0
    };
    let daily_deficit = inputs.household_demand_per_day - avg_daily_supply;

    if inputs.lowest_stock_days < 1.0 {
        if let Some(zero_day) = inputs.day_stock_zeroed {
            bottlenecks.push(format!(
                "Households ran out of supplies on day {zero_day} (of {}) and never recovered. \
                 Supply ({:.1}/day) covers only {:.0}% of the {:.1}/day demand. \
                 To keep up, increase the final node's output by at least {:.1} units/day.",
                scenario.duration_days,
                avg_daily_supply,
                if inputs.household_demand_per_day > 0.0 {
                    avg_daily_supply / inputs.household_demand_per_day * 100.0
                } else {
                    0.0
                },
                inputs.household_demand_per_day,
                daily_deficit.max(0.0),
            ));
        } else {
            bottlenecks.push(format!(
                "Household stock fell below 1 day's worth of supplies (lowest: {:.2} days). \
                 The chain is under-supplied by {:.1} units/day on average.",
                inputs.lowest_stock_days,
                daily_deficit.max(0.0)
            ));
        }
    }
    if inputs.total_unmet_units > 0.0 {
        let days_covered_by_buffer = if daily_deficit > 0.0 {
            let starting_buffer =
                inputs.household_demand_per_day * scenario.starting_household_stock_days as f32;
            starting_buffer / daily_deficit
        } else {
            scenario.duration_days as f32
        };
        bottlenecks.push(format!(
            "{:.0} units went undelivered over {} days (avg {:.1} unmet/day after buffer ran out ~day {:.0}). \
             Households consumed their starting stock in the first {:.1} days before supply fell short.",
            inputs.total_unmet_units,
            scenario.duration_days,
            if scenario.duration_days as f32 - days_covered_by_buffer > 0.0 {
                inputs.total_unmet_units
                    / (scenario.duration_days as f32 - days_covered_by_buffer).max(1.0)
            } else {
                0.0
            },
            days_covered_by_buffer.min(scenario.duration_days as f32),
            days_covered_by_buffer.min(scenario.duration_days as f32),
        ));
    }

    for (node_id, cumulative_profit) in inputs.node_cumulative_profits {
        if *cumulative_profit < 0.0 {
            bottlenecks.push(format!(
                "Node '{}' is insolvent: it lost {:.1} currency over {} days. \
                 Wages and input costs exceed revenue — raise the unit price or reduce worker count.",
                node_id,
                cumulative_profit.abs(),
                scenario.duration_days
            ));
        }
    }

    if bottlenecks.is_empty() {
        bottlenecks.push("Starter chain remains stocked for the whole sandbox run.".to_owned());
    }

    bottlenecks
}
