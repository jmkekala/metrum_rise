//! Sandbox graph, controller, and price lookup helpers.

use crate::simulation::economy::definitions::schema::{
    CONTROLLER_KIND_HOUSEHOLD_RESTOCK_COST, EconomyController, EconomyProfile, EconomyScenario,
    NODE_REF_KIND_CONTROLLER, ScenarioEdge, ScenarioNode,
};
use std::collections::BTreeMap;

pub(super) fn build_outgoing_edges<'a>(
    scenario: &'a EconomyScenario,
) -> BTreeMap<&'a str, Vec<&'a ScenarioEdge>> {
    let mut outgoing: BTreeMap<&str, Vec<&ScenarioEdge>> = BTreeMap::new();
    for edge in &scenario.edges {
        outgoing.entry(edge.from.as_str()).or_default().push(edge);
    }
    outgoing
}

pub(super) fn household_cost_multiplier(
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
        if controller_node.ref_kind != NODE_REF_KIND_CONTROLLER {
            continue;
        }
        let Some(controller) = controller_map.get(controller_node.ref_id.as_str()) else {
            continue;
        };
        if controller.kind != CONTROLLER_KIND_HOUSEHOLD_RESTOCK_COST {
            continue;
        }
        let t = controller.default_weight.clamp(0.0, 1.0);
        return controller.min_multiplier
            + (controller.max_multiplier - controller.min_multiplier) * t;
    }
    1.0
}

pub(super) fn inferred_unit_price(
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

pub(super) fn input_unit_price(
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
