// SPDX-License-Identifier: GPL-2.0-only

//! Resource-keyed supplier lookup over freight-reachable components.

use std::collections::BTreeMap;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::accessibility::{
    ModeComponentIndex, ReachableBucketEntry, ReachableBucketIndex, chunk_for_point,
};
use crate::simulation::economy::definitions::{ResourceRuntimeId, RuntimeEconomyCatalog};
use crate::simulation::economy::households::saleable_output_stock;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use rayon::prelude::*;

use super::quantization::fills_one_batch;
use super::reservations::ReservationViews;
use super::resource::building_outputs_can_supply_local_inputs;

/// Resource-compatible supplier candidates grouped by freight-reachable component.
#[derive(Clone, Debug, Default)]
pub(super) struct SupplierCandidateIndex {
    by_resource: BTreeMap<ResourceRuntimeId, ReachableBucketIndex>,
}

impl SupplierCandidateIndex {
    /// Builds a resource-keyed supplier index over freight-reachable component chunks.
    ///
    /// Lists a supplier under a resource only while its unreserved saleable stock fills one
    /// resource batch: a smaller stock quantizes to nothing for every request. Planning only adds
    /// reservations and never adds stock, so an index built at the start of the pass stays a
    /// superset of the shippable suppliers and the scan sees them in the same order. Without
    /// the filter a request no supplier can fill scans every supplier of the resource.
    pub(super) fn build(
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        freight_components: &ModeComponentIndex,
        reservations: &ReservationViews,
        truck_payload_kg: f32,
    ) -> Self {
        Self::build_filtered(
            allocator,
            graph,
            catalog,
            freight_components,
            true,
            |idx, resource| {
                let building = &allocator.buildings[idx];
                let profile = catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .unwrap();
                let available = (saleable_output_stock(catalog, building, profile, resource)
                    - reservations.reserved_outbound_amount(idx, resource))
                .max(0.0);
                let batch = catalog
                    .resource_measure(resource)
                    .map_or(0.0, |r| r.shipment_batch_kg);
                fills_one_batch(available, batch, truck_payload_kg)
            },
        )
    }

    /// Builds the same component/chunk index with caller-owned resource eligibility.
    /// Recurring forecasts must test production; dispatch continues to test physical stock.
    pub(super) fn build_filtered(
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        freight_components: &ModeComponentIndex,
        operational_only: bool,
        eligible: impl Fn(usize, ResourceRuntimeId) -> bool + Sync,
    ) -> Self {
        let entries: Vec<(ResourceRuntimeId, ReachableBucketEntry)> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter_map(|(idx, building)| {
                if (operational_only
                    && (building.broken
                        || building.economy_broken
                        || building.is_deserted
                        || building.is_under_construction()))
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
                    building,
                    profile,
                ))
            })
            .flat_map_iter(|(idx, chunk, components, _building, profile)| {
                let (component_values, component_count) = components.raw_parts();
                let eligible = &eligible;
                profile
                    .outputs
                    .iter()
                    .filter(move |output| eligible(idx, output.resource_runtime_id))
                    .flat_map(move |output| {
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

    /// Whether any resource still has an eligible supplier, in O(resource kinds).
    pub(super) fn has_offers(&self) -> bool {
        self.by_resource
            .values()
            .any(ReachableBucketIndex::has_available)
    }
    /// Enables finite-quantity updates while preserving the shared spatial grouping.
    pub(super) fn enable_reservations(&mut self) {
        for buckets in self.by_resource.values_mut() {
            buckets.enable_reservations();
        }
    }

    /// Toggles a resource supplier after a claim or rollback without rebuilding its index.
    pub(super) fn set_available(
        &mut self,
        resource: ResourceRuntimeId,
        entry: ReachableBucketEntry,
        available: bool,
    ) {
        if let Some(buckets) = self.by_resource.get_mut(&resource) {
            buckets.set_available(entry, available);
        }
    }
}
