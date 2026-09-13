// SPDX-License-Identifier: GPL-2.0-only

//! Shared operations on compact resource totals used by demand and household settlement.
//! Lookup is linear in the represented catalog resources, independent of city population.

use super::definitions::ResourceRuntimeId;

/// Merges resource totals in their existing accumulation order.
pub(super) fn merge_resource_amounts(
    target: &mut Vec<(ResourceRuntimeId, f32)>,
    source: Vec<(ResourceRuntimeId, f32)>,
) {
    for (resource_runtime_id, amount) in source {
        add_resource_amount(target, resource_runtime_id, amount);
    }
}

/// Adds a positive contribution, preserving first-seen resource and accumulation order.
pub(super) fn add_resource_amount(
    amounts: &mut Vec<(ResourceRuntimeId, f32)>,
    resource_runtime_id: ResourceRuntimeId,
    amount: f32,
) {
    if amount <= 0.0 {
        return;
    }
    if let Some((_, existing)) = amounts
        .iter_mut()
        .find(|(resource, _)| *resource == resource_runtime_id)
    {
        *existing += amount;
    } else {
        amounts.push((resource_runtime_id, amount));
    }
}

/// Reads a resource's accumulated amount, or zero when it has no contributions.
pub(super) fn resource_amount(
    amounts: &[(ResourceRuntimeId, f32)],
    resource_runtime_id: ResourceRuntimeId,
) -> f32 {
    amounts
        .iter()
        .find_map(|(resource, amount)| (*resource == resource_runtime_id).then_some(*amount))
        .unwrap_or(0.0)
}
