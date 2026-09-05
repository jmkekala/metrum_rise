// SPDX-License-Identifier: GPL-2.0-only

//! Resource-keyed supplier lookup over freight-reachable components.

use std::collections::BTreeMap;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::accessibility::{
    ModeComponentIndex, ReachableBucketEntry, ReachableBucketIndex, chunk_for_point,
};
use crate::simulation::economy::definitions::{ResourceRuntimeId, RuntimeEconomyCatalog};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use rayon::prelude::*;

use super::resource::building_outputs_can_supply_local_inputs;

/// Resource-compatible supplier candidates grouped by freight-reachable component.
pub(super) struct SupplierCandidateIndex {
    by_resource: BTreeMap<ResourceRuntimeId, ReachableBucketIndex>,
}

impl SupplierCandidateIndex {
    /// Builds a resource-keyed supplier index over freight-reachable component chunks.
    pub(super) fn build(
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        freight_components: &ModeComponentIndex,
    ) -> Self {
        let mut entries: Vec<(ResourceRuntimeId, ReachableBucketEntry)> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter_map(|(idx, building)| {
                if building.broken
                    || building.economy_broken
                    || building.is_deserted
                    || building.is_under_construction()
                    || building.edge_idx == usize::MAX
                {
                    return None;
                }
                let Some(profile) =
                    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                else {
                    return None;
                };
                if !building_outputs_can_supply_local_inputs(building, profile) {
                    return None;
                }
                let components = freight_components.building_components(
                    allocator,
                    graph,
                    idx,
                    TransitFlags::CAR,
                );
                if components.as_slice().is_empty() {
                    return None;
                }
                Some((
                    idx,
                    chunk_for_point(building.center_x, building.center_y),
                    components,
                    profile.outputs.as_slice(),
                ))
            })
            .flat_map_iter(|(idx, chunk, components, outputs)| {
                let (component_values, component_count) = components.raw_parts();
                outputs.iter().flat_map(move |output| {
                    (0..component_count).map(move |component_idx| {
                        let component = component_values[component_idx];
                        (
                            output.resource_runtime_id,
                            ReachableBucketEntry::new(component, chunk, idx),
                        )
                    })
                })
            })
            .collect();
        entries.sort_unstable_by_key(|(resource_runtime_id, entry)| (*resource_runtime_id, *entry));

        let mut grouped = BTreeMap::new();
        for (resource_runtime_id, entry) in entries {
            grouped
                .entry(resource_runtime_id)
                .or_insert_with(Vec::new)
                .push(entry);
        }

        let by_resource = grouped
            .into_iter()
            .map(|(resource_runtime_id, entries)| {
                (
                    resource_runtime_id,
                    ReachableBucketIndex::from_entries(entries),
                )
            })
            .collect();

        Self { by_resource }
    }

    /// Returns the bucket index for suppliers that can output the requested resource.
    pub(super) fn buckets_for_resource(
        &self,
        resource_runtime_id: ResourceRuntimeId,
    ) -> Option<&ReachableBucketIndex> {
        self.by_resource.get(&resource_runtime_id)
    }
}
