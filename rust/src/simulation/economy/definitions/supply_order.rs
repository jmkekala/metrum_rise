// SPDX-License-Identifier: GPL-2.0-only

//! Supplier-before-buyer phases compiled from authored net material dependencies.

use super::runtime::{EconomyProfileRuntimeKind, RuntimeEconomyCatalog};

/// Compiles phases in O(recipe ports² + authored supplier/buyer edges + profiles + resources).
/// No city data is involved. Compact resource ids join recipes, and explicit traversal
/// stacks keep long authored chains off the call stack.
pub(super) fn compile_phases(catalog: &RuntimeEconomyCatalog) -> (Vec<usize>, usize) {
    let profiles = catalog.all_profiles();
    let mut producers = vec![Vec::new(); catalog.resource_count() + 1];
    for (index, profile) in profiles.iter().enumerate() {
        if matches!(
            profile.kind,
            EconomyProfileRuntimeKind::Producer
                | EconomyProfileRuntimeKind::Processor
                | EconomyProfileRuntimeKind::FieldProducer
                | EconomyProfileRuntimeKind::Extractor
                | EconomyProfileRuntimeKind::Store
        ) {
            for output in &profile.outputs {
                if profile.net_output_units_per_day(output) > 0.0 {
                    producers[output.resource_runtime_id as usize].push(index);
                }
            }
        }
    }
    let mut outgoing = vec![Vec::new(); profiles.len()];
    let mut incoming = vec![Vec::new(); profiles.len()];
    for (buyer, profile) in profiles.iter().enumerate() {
        for input in &profile.inputs {
            let internal = profile
                .output_port(input.resource_runtime_id)
                .map_or(0.0, |output| output.units_per_day);
            if input.units_per_day <= internal {
                continue;
            }
            for &source in &producers[input.resource_runtime_id as usize] {
                outgoing[source].push(buyer);
                incoming[buyer].push(source);
            }
        }
    }
    phases(&outgoing, &incoming)
}

fn phases(outgoing: &[Vec<usize>], incoming: &[Vec<usize>]) -> (Vec<usize>, usize) {
    let (groups, count) =
        crate::simulation::pathing::components::strong_components(outgoing, incoming);
    // Visit each group once, retaining every member's outgoing edges. Using depth
    // rather than arbitrary group ids preserves building priority across sibling recipes.
    let mut members = vec![Vec::new(); count];
    for (node, &group) in groups.iter().enumerate() {
        members[group].push(node);
    }
    let mut depth = vec![0; count];
    for group in 0..count {
        for &node in &members[group] {
            for &target in &outgoing[node] {
                let target_group = groups[target];
                if target_group != group {
                    debug_assert!(target_group > group);
                    depth[target_group] = depth[target_group].max(depth[group] + 1);
                }
            }
        }
    }
    let count = depth.iter().max().copied().unwrap_or(0) + 1;
    (
        groups.into_iter().map(|group| depth[group]).collect(),
        count,
    )
}

#[cfg(test)]
mod tests {
    use super::phases;

    #[test]
    fn cyclic_suppliers_precede_downstream_buyers_without_reordering_siblings() {
        // 0 -> 1 -> 2 -> 1 is a seeded material cycle; 2 -> 3 and 1 -> 4
        // are its terminal buyers. Node 5 is an independent recipe.
        let outgoing = vec![vec![1], vec![2, 4], vec![1, 3], vec![], vec![], vec![]];
        let incoming = vec![vec![], vec![0, 2], vec![1], vec![2], vec![1], vec![]];
        assert_eq!(phases(&outgoing, &incoming), (vec![0, 1, 1, 2, 2, 0], 3));
    }
}
