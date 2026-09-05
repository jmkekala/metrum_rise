// SPDX-License-Identifier: GPL-2.0-only

//! Authored scenario graph validation.

use super::common::duplicate_ids;
use super::messages::{ValidationMessage, error, warning};
use crate::simulation::economy::definitions::scenario_graph::{
    build_profile_scenario_graph, port_exists,
};
use crate::simulation::economy::definitions::schema::{
    AuthoredProfileKind, EconomyController, EconomyProfile, EconomyScenario,
    NODE_REF_KIND_CONTROLLER, NODE_REF_KIND_PROFILE, ScenarioNode,
};
use std::collections::BTreeMap;

pub(super) fn validate_scenario(
    scenario: &EconomyScenario,
    profile_map: &BTreeMap<&str, &EconomyProfile>,
    controller_map: &BTreeMap<&str, &EconomyController>,
    messages: &mut Vec<ValidationMessage>,
) {
    if scenario.nodes.is_empty() {
        messages.push(error(
            "empty_scenario",
            format!("scenario.{}", scenario.id),
            "scenario has no graph nodes".to_owned(),
        ));
        return;
    }

    let duplicate_node_ids = duplicate_ids(scenario.nodes.iter().map(|node| node.id.as_str()));
    for duplicate in duplicate_node_ids {
        messages.push(error(
            "duplicate_scenario_node_id",
            format!("scenario.{}", scenario.id),
            format!("scenario node id '{duplicate}' is defined more than once"),
        ));
    }

    let node_map: BTreeMap<&str, &ScenarioNode> = scenario
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let mut demand_sink_count = 0u32;

    for node in &scenario.nodes {
        match node.ref_kind.as_str() {
            NODE_REF_KIND_PROFILE => {
                let Some(profile) = profile_map.get(node.ref_id.as_str()) else {
                    messages.push(error(
                        "missing_profile_ref",
                        format!("scenario.{}.node.{}", scenario.id, node.id),
                        format!("scenario node references missing profile '{}'", node.ref_id),
                    ));
                    continue;
                };
                if profile.authored_kind() == AuthoredProfileKind::DemandSink {
                    demand_sink_count += 1;
                }
            }
            NODE_REF_KIND_CONTROLLER => {
                if !controller_map.contains_key(node.ref_id.as_str()) {
                    messages.push(error(
                        "missing_controller_ref",
                        format!("scenario.{}.node.{}", scenario.id, node.id),
                        format!(
                            "scenario node references missing controller '{}'",
                            node.ref_id
                        ),
                    ));
                }
            }
            other => messages.push(error(
                "invalid_node_kind",
                format!("scenario.{}.node.{}", scenario.id, node.id),
                format!(
                    "scenario node kind '{other}' is invalid; expected 'profile' or 'controller'"
                ),
            )),
        }
    }

    if demand_sink_count == 0 {
        messages.push(error(
            "missing_demand_sink",
            format!("scenario.{}", scenario.id),
            "scenario must include one household demand sink node".to_owned(),
        ));
    } else if demand_sink_count > 1 {
        messages.push(warning(
            "multiple_demand_sinks",
            format!("scenario.{}", scenario.id),
            "scenario includes multiple demand sinks; sandbox playback uses the first one"
                .to_owned(),
        ));
    }

    for edge in &scenario.edges {
        let Some(from_node) = node_map.get(edge.from.as_str()) else {
            messages.push(error(
                "missing_edge_source_node",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!("edge source node '{}' does not exist", edge.from),
            ));
            continue;
        };
        let Some(to_node) = node_map.get(edge.to.as_str()) else {
            messages.push(error(
                "missing_edge_target_node",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!("edge target node '{}' does not exist", edge.to),
            ));
            continue;
        };
        if from_node.ref_kind != NODE_REF_KIND_PROFILE || to_node.ref_kind != NODE_REF_KIND_PROFILE
        {
            messages.push(error(
                "invalid_edge_endpoint_kind",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                "scenario edges may connect profile nodes only".to_owned(),
            ));
            continue;
        }

        let Some(from_profile) = profile_map.get(from_node.ref_id.as_str()) else {
            continue;
        };
        let Some(to_profile) = profile_map.get(to_node.ref_id.as_str()) else {
            continue;
        };

        if !port_exists(&from_profile.outputs, edge.resource.as_str()) {
            messages.push(error(
                "edge_resource_not_produced",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!(
                    "source profile '{}' does not output resource '{}'",
                    from_profile.id, edge.resource
                ),
            ));
        }
        if !port_exists(&to_profile.inputs, edge.resource.as_str()) {
            messages.push(error(
                "edge_resource_not_consumed",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!(
                    "target profile '{}' does not consume resource '{}'",
                    to_profile.id, edge.resource
                ),
            ));
        }
    }

    let profile_graph = build_profile_scenario_graph(scenario, &node_map, profile_map);

    for node in &scenario.nodes {
        if node.ref_kind != NODE_REF_KIND_PROFILE {
            continue;
        }
        let Some(profile) = profile_map.get(node.ref_id.as_str()) else {
            continue;
        };
        let received = profile_graph.incoming_resources_for(node.id.as_str());
        for input in &profile.inputs {
            if received.is_none_or(|resources| !resources.contains(input.resource.as_str())) {
                messages.push(error(
                    "disconnected_required_input",
                    format!("scenario.{}.node.{}", scenario.id, node.id),
                    format!(
                        "profile '{}' requires input '{}' but no edge supplies it",
                        profile.id, input.resource
                    ),
                ));
            }
        }
    }

    for link in &scenario.controller_links {
        let Some(controller_node) = node_map.get(link.controller_node_id.as_str()) else {
            messages.push(error(
                "missing_controller_link_source",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                format!(
                    "controller node '{}' does not exist",
                    link.controller_node_id
                ),
            ));
            continue;
        };
        let Some(target_node) = node_map.get(link.target_node_id.as_str()) else {
            messages.push(error(
                "missing_controller_link_target",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                format!("target node '{}' does not exist", link.target_node_id),
            ));
            continue;
        };
        if controller_node.ref_kind != NODE_REF_KIND_CONTROLLER {
            messages.push(error(
                "invalid_controller_link_source_kind",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                "controller link source must be a controller node".to_owned(),
            ));
        }
        if target_node.ref_kind != NODE_REF_KIND_PROFILE {
            messages.push(error(
                "invalid_controller_link_target_kind",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                "controller links must target profile nodes".to_owned(),
            ));
        }
    }

    if profile_graph.has_cycle() {
        messages.push(error(
            "cyclic_scenario_graph",
            format!("scenario.{}", scenario.id),
            "scenario graph contains a cycle; bootstrap playback requires an acyclic profile chain"
                .to_owned(),
        ));
    }
}
