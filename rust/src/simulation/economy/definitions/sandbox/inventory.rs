//! Inventory mutation and throughput helpers for sandbox playback.

use crate::simulation::economy::definitions::schema::{EconomyProfile, ResourcePort, ScenarioEdge};
use std::collections::BTreeMap;

pub(super) type Inventories = BTreeMap<String, BTreeMap<String, f32>>;

pub(super) fn compute_throughput(
    profile: &EconomyProfile,
    inventories: &Inventories,
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

pub(super) fn add_outputs_to_inventory(
    inventories: &mut Inventories,
    node_id: &str,
    outputs: &[ResourcePort],
    scale: f32,
) {
    let stock = inventories.entry(node_id.to_owned()).or_default();
    for output in outputs {
        *stock.entry(output.resource.clone()).or_default() += output.units_per_day * scale;
    }
}

pub(super) fn consume_inputs_from_inventory(
    inventories: &mut Inventories,
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

pub(super) fn take_all_incoming_stock(
    inventories: &mut Inventories,
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

pub(super) fn transfer_outgoing_stock(
    inventories: &mut Inventories,
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
