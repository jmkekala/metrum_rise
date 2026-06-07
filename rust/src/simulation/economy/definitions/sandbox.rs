//! Authored economy sandbox playback for editor-facing scenario diagnostics.

use super::schema::{
    EconomyController, EconomyProfile, EconomyProject, EconomyScenario, ResourcePort, ScenarioEdge,
    ScenarioNode,
};
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};

#[derive(Serialize)]
pub(super) struct SandboxResult {
    pub(super) scenario_id: String,
    pub(super) display_name: String,
    pub(super) duration_days: u32,
    pub(super) daily_household_demand_units: f32,
    pub(super) final_household_stock_days: f32,
    pub(super) lowest_household_stock_days: f32,
    pub(super) total_delivered_units: f32,
    pub(super) total_unmet_units: f32,
    pub(super) average_household_cost_per_day: f32,
    pub(super) bottlenecks: Vec<String>,
    pub(super) daily: Vec<DailySandboxMetric>,
}

#[derive(Serialize)]
pub(super) struct DailySandboxMetric {
    pub(super) day: u32,
    pub(super) household_stock_days: f32,
    pub(super) delivered_units: f32,
    pub(super) unmet_units: f32,
    pub(super) average_household_cost: f32,
}
pub(super) fn run_sandbox(
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

    let topo_order = topological_profile_node_order(scenario)?;
    let demand_sink_node = scenario
        .nodes
        .iter()
        .find(|node| {
            node.ref_kind == "profile"
                && profile_map
                    .get(node.ref_id.as_str())
                    .is_some_and(|profile| profile.kind == "demand_sink")
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
    let mut inventories: BTreeMap<String, BTreeMap<String, f32>> = BTreeMap::new();
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

            if profile.kind == "demand_sink" {
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

    let mut bottlenecks = Vec::new();

    // Average daily supply actually delivered over the run.
    let avg_daily_supply = if supply_day_count > 0 {
        total_daily_supply / supply_day_count as f32
    } else {
        0.0
    };
    let daily_deficit = household_demand_per_day - avg_daily_supply;

    if lowest_stock_days < 1.0 {
        if let Some(zero_day) = day_stock_zeroed {
            bottlenecks.push(format!(
                "Households ran out of supplies on day {zero_day} (of {}) and never recovered. \
                 Supply ({:.1}/day) covers only {:.0}% of the {:.1}/day demand. \
                 To keep up, increase the final node's output by at least {:.1} units/day.",
                scenario.duration_days,
                avg_daily_supply,
                if household_demand_per_day > 0.0 {
                    avg_daily_supply / household_demand_per_day * 100.0
                } else {
                    0.0
                },
                household_demand_per_day,
                daily_deficit.max(0.0),
            ));
        } else {
            bottlenecks.push(format!(
                "Household stock fell below 1 day's worth of supplies (lowest: {:.2} days). \
                 The chain is under-supplied by {:.1} units/day on average.",
                lowest_stock_days,
                daily_deficit.max(0.0)
            ));
        }
    }
    if total_unmet_units > 0.0 {
        let days_covered_by_buffer = if daily_deficit > 0.0 {
            let starting_buffer =
                household_demand_per_day * scenario.starting_household_stock_days as f32;
            starting_buffer / daily_deficit
        } else {
            scenario.duration_days as f32
        };
        bottlenecks.push(format!(
            "{:.0} units went undelivered over {} days (avg {:.1} unmet/day after buffer ran out ~day {:.0}). \
             Households consumed their starting stock in the first {:.1} days before supply fell short.",
            total_unmet_units,
            scenario.duration_days,
            if scenario.duration_days as f32 - days_covered_by_buffer > 0.0 {
                total_unmet_units / (scenario.duration_days as f32 - days_covered_by_buffer).max(1.0)
            } else { 0.0 },
            days_covered_by_buffer.min(scenario.duration_days as f32),
            days_covered_by_buffer.min(scenario.duration_days as f32),
        ));
    }

    for (node_id, cumulative_profit) in &node_cumulative_profits {
        if *cumulative_profit < 0.0 {
            bottlenecks.push(format!(
                "Node '{}' is insolvent: it lost {:.1} currency over {} days. \
                 Wages and input costs exceed revenue — raise the unit price or reduce worker count.",
                node_id, cumulative_profit.abs(), scenario.duration_days
            ));
        }
    }

    if bottlenecks.is_empty() {
        bottlenecks.push("Starter chain remains stocked for the whole sandbox run.".to_owned());
    }

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
fn build_outgoing_edges<'a>(
    scenario: &'a EconomyScenario,
) -> BTreeMap<&'a str, Vec<&'a ScenarioEdge>> {
    let mut outgoing: BTreeMap<&str, Vec<&ScenarioEdge>> = BTreeMap::new();
    for edge in &scenario.edges {
        outgoing.entry(edge.from.as_str()).or_default().push(edge);
    }
    outgoing
}

fn household_cost_multiplier(
    scenario: &EconomyScenario,
    demand_sink_node_id: &str,
    node_map: &BTreeMap<&str, &ScenarioNode>,
    controller_map: &BTreeMap<&str, &EconomyController>,
) -> f32 {
    for link in &scenario.controller_links {
        if link.target_node_id != demand_sink_node_id {
            continue;
        }
        let Some(controller_node) = node_map.get(link.controller_node_id.as_str()) else {
            continue;
        };
        if controller_node.ref_kind != "controller" {
            continue;
        }
        let Some(controller) = controller_map.get(controller_node.ref_id.as_str()) else {
            continue;
        };
        if controller.kind != "household_restock_cost" {
            continue;
        }
        let t = controller.default_weight.clamp(0.0, 1.0);
        return controller.min_multiplier
            + (controller.max_multiplier - controller.min_multiplier) * t;
    }
    1.0
}

fn inferred_unit_price(
    scenario: &EconomyScenario,
    demand_sink_node_id: &str,
    outgoing_edges: &BTreeMap<&str, Vec<&ScenarioEdge>>,
    node_map: &BTreeMap<&str, &ScenarioNode>,
    profile_map: &BTreeMap<&str, &EconomyProfile>,
) -> f32 {
    for edge in &scenario.edges {
        if edge.to != demand_sink_node_id {
            continue;
        }
        let Some(source_node) = node_map.get(edge.from.as_str()) else {
            continue;
        };
        let Some(source_profile) = profile_map.get(source_node.ref_id.as_str()) else {
            continue;
        };
        if source_profile.unit_price_currency > 0.0 {
            return source_profile.unit_price_currency;
        }
    }
    for (node_id, edges) in outgoing_edges {
        if edges.iter().any(|edge| edge.to == demand_sink_node_id) {
            let Some(source_node) = node_map.get(node_id) else {
                continue;
            };
            let Some(source_profile) = profile_map.get(source_node.ref_id.as_str()) else {
                continue;
            };
            if source_profile.unit_price_currency > 0.0 {
                return source_profile.unit_price_currency;
            }
        }
    }
    0.0
}

fn input_unit_price(
    node_id: &str,
    resource: &str,
    scenario: &EconomyScenario,
    node_map: &BTreeMap<&str, &ScenarioNode>,
    profile_map: &BTreeMap<&str, &EconomyProfile>,
) -> f32 {
    for edge in &scenario.edges {
        if edge.to == node_id && edge.resource == resource {
            if let Some(source_node) = node_map.get(edge.from.as_str()) {
                if let Some(source_profile) = profile_map.get(source_node.ref_id.as_str()) {
                    if source_profile.unit_price_currency > 0.0 {
                        return source_profile.unit_price_currency;
                    }
                }
            }
        }
    }
    0.0
}

fn compute_throughput(
    profile: &EconomyProfile,
    inventories: &BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
) -> f32 {
    if profile.inputs.is_empty() {
        return profile.base_rate_units_per_day.max(0.0);
    }
    if profile.base_rate_units_per_day <= 0.0 {
        return 0.0;
    }

    let mut throughput = profile.base_rate_units_per_day;
    for input in &profile.inputs {
        let available = inventories
            .get(node_id)
            .and_then(|stock| stock.get(input.resource.as_str()))
            .copied()
            .unwrap_or(0.0);
        if input.units_per_day <= 0.0 {
            continue;
        }
        let allowed = available / input.units_per_day * profile.base_rate_units_per_day;
        throughput = throughput.min(allowed);
    }
    throughput.max(0.0)
}

fn add_outputs_to_inventory(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
    outputs: &[ResourcePort],
    scale: f32,
) {
    let stock = inventories.entry(node_id.to_owned()).or_default();
    for output in outputs {
        *stock.entry(output.resource.clone()).or_default() += output.units_per_day * scale;
    }
}

fn consume_inputs_from_inventory(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
    inputs: &[ResourcePort],
    scale: f32,
) {
    let stock = inventories.entry(node_id.to_owned()).or_default();
    for input in inputs {
        let entry = stock.entry(input.resource.clone()).or_default();
        *entry = (*entry - input.units_per_day * scale).max(0.0);
    }
}

fn take_all_incoming_stock(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
    inputs: &[ResourcePort],
) -> f32 {
    let stock = inventories.entry(node_id.to_owned()).or_default();
    let mut total = 0.0;
    for input in inputs {
        if let Some(entry) = stock.get_mut(input.resource.as_str()) {
            total += *entry;
            *entry = 0.0;
        }
    }
    total
}

fn transfer_outgoing_stock(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    from_node_id: &str,
    outgoing_edges: Option<&Vec<&ScenarioEdge>>,
) {
    let Some(outgoing_edges) = outgoing_edges else {
        return;
    };

    let mut transfers: Vec<(String, String, f32)> = Vec::new();
    {
        let Some(stock) = inventories.get_mut(from_node_id) else {
            return;
        };
        for edge in outgoing_edges {
            let Some(amount) = stock.get_mut(edge.resource.as_str()) else {
                continue;
            };
            if *amount <= 0.0 {
                continue;
            }
            let moved = *amount;
            *amount = 0.0;
            transfers.push((edge.to.clone(), edge.resource.clone(), moved));
        }
    }

    for (target_node, resource, amount) in transfers {
        *inventories
            .entry(target_node)
            .or_default()
            .entry(resource)
            .or_default() += amount;
    }
}

fn topological_profile_node_order(scenario: &EconomyScenario) -> Result<Vec<String>, String> {
    let mut outgoing: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, u32> = scenario
        .nodes
        .iter()
        .filter(|node| node.ref_kind == "profile")
        .map(|node| (node.id.as_str(), 0))
        .collect();

    for edge in &scenario.edges {
        outgoing
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
        *indegree.entry(edge.to.as_str()).or_insert(0) += 1;
    }

    let mut queue = VecDeque::new();
    for (&node_id, &degree) in &indegree {
        if degree == 0 {
            queue.push_back(node_id);
        }
    }

    let mut ordered = Vec::with_capacity(indegree.len());
    while let Some(node_id) = queue.pop_front() {
        ordered.push(node_id.to_owned());
        if let Some(next) = outgoing.get(node_id) {
            for &target in next {
                if let Some(degree) = indegree.get_mut(target) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(target);
                    }
                }
            }
        }
    }

    if ordered.len() != indegree.len() {
        return Err(format!(
            "scenario '{}' contains a profile-cycle",
            scenario.id
        ));
    }

    Ok(ordered)
}
