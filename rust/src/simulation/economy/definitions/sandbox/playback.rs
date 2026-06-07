//! Sandbox playback orchestrator.

use super::bottlenecks::{BottleneckInputs, build_bottlenecks};
use super::inventory::{
    Inventories, add_outputs_to_inventory, compute_throughput, consume_inputs_from_inventory,
    take_all_incoming_stock, transfer_outgoing_stock,
};
use super::pricing::{
    build_outgoing_edges, household_cost_multiplier, inferred_unit_price, input_unit_price,
};
use super::types::{DailySandboxMetric, SandboxResult};
use crate::simulation::economy::definitions::scenario_graph::build_profile_scenario_graph;
use crate::simulation::economy::definitions::schema::{
    AuthoredProfileKind, EconomyController, EconomyProfile, EconomyProject, NODE_REF_KIND_PROFILE,
    ScenarioNode,
};
use std::collections::BTreeMap;

pub(in crate::simulation::economy::definitions) fn run_sandbox(
    project: &EconomyProject,
    scenario_id: &str,
) -> Result<SandboxResult, String> {
    let scenario = project
        .scenarios
        .iter()
        .find(|scenario| scenario.id == scenario_id)
        .ok_or_else(|| format!("scenario '{scenario_id}' not found"))?;

    let profile_map: BTreeMap<&str, &EconomyProfile> = project
        .profiles
        .iter()
        .map(|profile| (profile.id.as_str(), profile))
        .collect();
    let controller_map: BTreeMap<&str, &EconomyController> = project
        .controllers
        .iter()
        .map(|controller| (controller.id.as_str(), controller))
        .collect();
    let node_map: BTreeMap<&str, &ScenarioNode> = scenario
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();

    let topo_order = build_profile_scenario_graph(scenario, &node_map, &profile_map)
        .topological_order()
        .map_err(|()| format!("scenario '{}' contains a profile-cycle", scenario.id))?;
    let demand_sink_node = scenario
        .nodes
        .iter()
        .find(|node| {
            node.ref_kind == NODE_REF_KIND_PROFILE
                && profile_map
                    .get(node.ref_id.as_str())
                    .is_some_and(|profile| {
                        profile.authored_kind() == AuthoredProfileKind::DemandSink
                    })
        })
        .ok_or_else(|| format!("scenario '{}' has no demand sink", scenario.id))?;
    let demand_sink_profile = profile_map
        .get(demand_sink_node.ref_id.as_str())
        .copied()
        .ok_or_else(|| {
            format!(
                "demand sink profile '{}' is missing",
                demand_sink_node.ref_id
            )
        })?;

    let household_demand_per_day = scenario.household_count as f32
        * scenario.average_household_size
        * demand_sink_profile.consumption_rate_per_resident.max(0.0);

    let mut household_stock_units =
        household_demand_per_day * scenario.starting_household_stock_days.max(0.0);
    let mut lowest_stock_days = if household_demand_per_day > 0.0 {
        household_stock_units / household_demand_per_day
    } else {
        0.0
    };
    let mut total_delivered_units = 0.0;
    let mut total_unmet_units = 0.0;
    let mut total_household_cost = 0.0;
    let mut total_daily_supply = 0.0f32;
    let mut supply_day_count = 0u32;
    let mut day_stock_zeroed: Option<u32> = None;
    let mut daily = Vec::with_capacity(scenario.duration_days as usize);
    let mut inventories = Inventories::new();
    let mut node_cumulative_profits: BTreeMap<String, f32> = BTreeMap::new();

    let outgoing_edges = build_outgoing_edges(scenario);
    let household_price_multiplier = household_cost_multiplier(
        scenario,
        demand_sink_node.id.as_str(),
        &node_map,
        &controller_map,
    );

    for day in 1..=scenario.duration_days {
        let mut delivered_today = 0.0;
        let mut unmet_today = 0.0;
        let mut household_cost_today = 0.0;

        for node_id in &topo_order {
            let node = node_map.get(node_id.as_str()).copied().ok_or_else(|| {
                format!(
                    "scenario node '{}' missing during sandbox playback",
                    node_id
                )
            })?;
            let profile = profile_map
                .get(node.ref_id.as_str())
                .copied()
                .ok_or_else(|| {
                    format!("profile '{}' missing during sandbox playback", node.ref_id)
                })?;

            if profile.authored_kind() == AuthoredProfileKind::DemandSink {
                let delivered_to_sink =
                    take_all_incoming_stock(&mut inventories, node.id.as_str(), &profile.inputs);
                delivered_today += delivered_to_sink;
                household_stock_units += delivered_to_sink;
                let consumed = household_stock_units.min(household_demand_per_day);
                unmet_today = household_demand_per_day - consumed;
                household_stock_units -= consumed;
                household_cost_today += delivered_to_sink
                    * inferred_unit_price(
                        scenario,
                        node.id.as_str(),
                        &outgoing_edges,
                        &node_map,
                        &profile_map,
                    )
                    * household_price_multiplier;
                continue;
            }

            let throughput = compute_throughput(profile, &inventories, node.id.as_str());
            let mut scale = 0.0;
            if profile.inputs.is_empty() {
                add_outputs_to_inventory(&mut inventories, node.id.as_str(), &profile.outputs, 1.0);
                scale = 1.0;
            } else if throughput > 0.0 && profile.base_rate_units_per_day > 0.0 {
                scale = throughput / profile.base_rate_units_per_day;
                consume_inputs_from_inventory(
                    &mut inventories,
                    node.id.as_str(),
                    &profile.inputs,
                    scale,
                );
                add_outputs_to_inventory(
                    &mut inventories,
                    node.id.as_str(),
                    &profile.outputs,
                    scale,
                );
            }

            let daily_labor_cost =
                profile.worker_capacity as f32 * profile.wage_max_currency_per_day;

            let mut daily_input_cost = 0.0;
            for input in &profile.inputs {
                let unit_price = input_unit_price(
                    node.id.as_str(),
                    input.resource.as_str(),
                    scenario,
                    &node_map,
                    &profile_map,
                );
                daily_input_cost += input.units_per_day * scale * unit_price;
            }

            let mut daily_revenue = 0.0;
            for output in &profile.outputs {
                daily_revenue += output.units_per_day * scale * profile.unit_price_currency;
            }

            let daily_profit = daily_revenue - (daily_labor_cost + daily_input_cost);
            *node_cumulative_profits
                .entry(node.id.clone())
                .or_insert(0.0) += daily_profit;

            transfer_outgoing_stock(
                &mut inventories,
                node.id.as_str(),
                outgoing_edges.get(node.id.as_str()),
            );
        }

        let stock_days = if household_demand_per_day > 0.0 {
            household_stock_units / household_demand_per_day
        } else {
            0.0
        };
        lowest_stock_days = lowest_stock_days.min(stock_days);
        if stock_days == 0.0 && day_stock_zeroed.is_none() {
            day_stock_zeroed = Some(day);
        }
        total_daily_supply += delivered_today;
        supply_day_count += 1;
        total_delivered_units += delivered_today;
        total_unmet_units += unmet_today;
        total_household_cost += household_cost_today;
        daily.push(DailySandboxMetric {
            day,
            household_stock_days: stock_days,
            delivered_units: delivered_today,
            unmet_units: unmet_today,
            average_household_cost: if scenario.household_count > 0 {
                household_cost_today / scenario.household_count as f32
            } else {
                0.0
            },
        });
    }

    let final_stock_days = if household_demand_per_day > 0.0 {
        household_stock_units / household_demand_per_day
    } else {
        0.0
    };

    let bottlenecks = build_bottlenecks(BottleneckInputs {
        scenario,
        household_demand_per_day,
        lowest_stock_days,
        day_stock_zeroed,
        total_daily_supply,
        supply_day_count,
        total_unmet_units,
        node_cumulative_profits: &node_cumulative_profits,
    });

    Ok(SandboxResult {
        scenario_id: scenario.id.clone(),
        display_name: scenario.display_name.clone(),
        duration_days: scenario.duration_days,
        daily_household_demand_units: household_demand_per_day,
        final_household_stock_days: final_stock_days,
        lowest_household_stock_days: lowest_stock_days,
        total_delivered_units,
        total_unmet_units,
        average_household_cost_per_day: if scenario.duration_days > 0
            && scenario.household_count > 0
        {
            total_household_cost / (scenario.duration_days as f32 * scenario.household_count as f32)
        } else {
            0.0
        },
        bottlenecks,
        daily,
    })
}
