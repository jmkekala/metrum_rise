// SPDX-License-Identifier: GPL-2.0-only

//! Shared deterministic helpers for authored economy scenario graphs.

use super::schema::{
    EconomyProfile, EconomyScenario, NODE_REF_KIND_PROFILE, ResourcePort, ScenarioNode,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) struct ProfileScenarioGraph<'a> {
    incoming_resources: BTreeMap<&'a str, BTreeSet<&'a str>>,
    outgoing: BTreeMap<&'a str, BTreeSet<&'a str>>,
    indegree: BTreeMap<&'a str, u32>,
}

impl<'a> ProfileScenarioGraph<'a> {
    pub(super) fn incoming_resources_for(&self, node_id: &str) -> Option<&BTreeSet<&'a str>> {
        self.incoming_resources.get(node_id)
    }

    pub(super) fn has_cycle(&self) -> bool {
        self.topological_order().is_err()
    }

    pub(super) fn topological_order(&self) -> Result<Vec<String>, ()> {
        let mut indegree = self.indegree.clone();
        let mut queue = VecDeque::new();
        for (&node_id, &degree) in &indegree {
            if degree == 0 {
                queue.push_back(node_id);
            }
        }

        let mut ordered = Vec::with_capacity(indegree.len());
        while let Some(node_id) = queue.pop_front() {
            ordered.push(node_id.to_owned());
            if let Some(targets) = self.outgoing.get(node_id) {
                for &target in targets {
                    if let Some(degree) = indegree.get_mut(target) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(target);
                        }
                    }
                }
            }
        }

        if ordered.len() == self.indegree.len() {
            Ok(ordered)
        } else {
            Err(())
        }
    }
}

pub(super) fn build_profile_scenario_graph<'a>(
    scenario: &'a EconomyScenario,
    node_map: &BTreeMap<&'a str, &'a ScenarioNode>,
    profile_map: &BTreeMap<&'a str, &'a EconomyProfile>,
) -> ProfileScenarioGraph<'a> {
    let profile_node_ids: BTreeSet<&str> = node_map
        .iter()
        .filter_map(|(&node_id, node)| {
            if node.ref_kind == NODE_REF_KIND_PROFILE
                && profile_map.contains_key(node.ref_id.as_str())
            {
                Some(node_id)
            } else {
                None
            }
        })
        .collect();

    let mut incoming_resources = BTreeMap::new();
    let mut outgoing: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, u32> = profile_node_ids
        .iter()
        .map(|&node_id| (node_id, 0))
        .collect();

    for edge in &scenario.edges {
        let from = edge.from.as_str();
        let to = edge.to.as_str();
        if !profile_node_ids.contains(from) || !profile_node_ids.contains(to) {
            continue;
        }

        incoming_resources
            .entry(to)
            .or_insert_with(BTreeSet::new)
            .insert(edge.resource.as_str());

        let inserted = outgoing.entry(from).or_default().insert(to);
        if inserted {
            *indegree.entry(to).or_insert(0) += 1;
        }
    }

    ProfileScenarioGraph {
        incoming_resources,
        outgoing,
        indegree,
    }
}

pub(super) fn port_exists(ports: &[ResourcePort], resource: &str) -> bool {
    ports.iter().any(|port| port.resource == resource)
}
