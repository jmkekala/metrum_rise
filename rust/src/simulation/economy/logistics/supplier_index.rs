// SPDX-License-Identifier: GPL-2.0-only

//! Resource-keyed supplier lookup over freight-reachable components.
//!
//! The dispatch index names suppliers by building handle (`SIM-02-I`): [`SupplierRoll`] builds
//! it over one hour of slots and the next hour reads it, while buildings leave and dense indices
//! move. Readers resolve each handle and skip a building that has left.

mod roll;

use std::collections::BTreeMap;
use std::ops::Range;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketEntry, ReachableBucketIndex,
    chunk_for_point,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, ResourceRuntimeId, RuntimeEconomyCatalog,
};
use crate::simulation::economy::households::saleable_output_stock;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use rayon::prelude::*;

use super::quantization::fills_one_batch;
use super::reservations::ReservationViews;
use super::resource::building_outputs_can_supply_local_inputs;

pub(crate) use self::roll::{SavedSupplierRoll, SavedSupplierSweep, SupplierRoll};

// Handle bits stand in for bucket item indices.
const _: () = assert!(usize::BITS >= u64::BITS);

// Building handle slots per parallel task of a whole fold.
const HANDLES_PER_TASK: usize = 1_024;

/// One dispatch supplier entry: the resource it can ship and its component/chunk bucket.
pub(super) type SupplierEntry = (ResourceRuntimeId, ReachableBucketEntry);

/// Resource-compatible supplier candidates grouped by freight-reachable component.
#[derive(Clone, Debug, Default)]
pub(super) struct SupplierCandidateIndex {
    by_resource: BTreeMap<ResourceRuntimeId, ReachableBucketIndex>,
}

/// Live state the dispatch eligibility reads.
pub(super) struct DispatchFold<'a> {
    pub(super) allocator: &'a BuildingAllocator,
    pub(super) graph: &'a RegionGraph,
    pub(super) catalog: &'a RuntimeEconomyCatalog,
    pub(super) components: &'a ModeComponentIndex,
    pub(super) reservations: &'a ReservationViews,
    pub(super) truck_payload_kg: f32,
}

impl DispatchFold<'_> {
    /// Passes the entries of the buildings in handle slots `slots` to `add`, in slot order; a
    /// free slot adds nothing. A supplier is listed under a resource while its unreserved
    /// saleable stock fills one resource batch: a smaller stock quantizes to nothing for every
    /// request. Item indices are building handle bits. O(slots × outputs), allocation-free.
    pub(super) fn fold(
        &self,
        slots: Range<usize>,
        mut add: impl FnMut(ResourceRuntimeId, ReachableBucketEntry),
    ) {
        let allocator = self.allocator;
        for slot in slots {
            let Some(idx) = allocator.building_handles.dense_at(slot) else {
                continue;
            };
            let Some((chunk, components, profile)) = supplier_site(
                allocator,
                self.graph,
                self.catalog,
                self.components,
                idx,
                true,
            ) else {
                continue;
            };
            let building = &allocator.buildings[idx];
            let item = allocator.building_handles.handle(idx).to_bits() as usize;
            for output in &profile.outputs {
                let resource = output.resource_runtime_id;
                let available = (saleable_output_stock(self.catalog, building, profile, resource)
                    - self.reservations.reserved_outbound_amount(idx, resource))
                .max(0.0);
                let batch = self
                    .catalog
                    .resource_measure(resource)
                    .map_or(0.0, |measure| measure.shipment_batch_kg);
                if !fills_one_batch(available, batch, self.truck_payload_kg) {
                    continue;
                }
                for &component in components.as_slice() {
                    add(resource, ReachableBucketEntry::new(component, chunk, item));
                }
            }
        }
    }

    /// The entries of every building handle slot, folded on Rayon in fixed parts and merged in
    /// slot order. O(handles × outputs).
    pub(super) fn fold_all(&self) -> Vec<SupplierEntry> {
        let handles = self.allocator.building_handles.capacity();
        (0..handles.div_ceil(HANDLES_PER_TASK))
            .into_par_iter()
            .flat_map_iter(|task| {
                let start = task * HANDLES_PER_TASK;
                let mut entries = Vec::new();
                self.fold(
                    start..(start + HANDLES_PER_TASK).min(handles),
                    |resource, entry| entries.push((resource, entry)),
                );
                entries
            })
            .collect()
    }
}

// Routing chunk, reachable freight components and profile of building `idx` when its outputs can
// supply local inputs, it is attached to a road and, if `operational_only`, it operates.
fn supplier_site<'a>(
    allocator: &BuildingAllocator,
    graph: &RegionGraph,
    catalog: &'a RuntimeEconomyCatalog,
    freight_components: &ModeComponentIndex,
    idx: usize,
    operational_only: bool,
) -> Option<(
    (i32, i32),
    BuildingModeComponents,
    &'a EconomyProfileRuntime,
)> {
    let building = &allocator.buildings[idx];
    if (operational_only
        && (building.broken
            || building.economy_broken
            || building.is_deserted
            || building.is_under_construction()))
        || building.edge_idx == usize::MAX
    {
        return None;
    }
    let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)?;
    if !building_outputs_can_supply_local_inputs(building, profile) {
        return None;
    }
    let components =
        freight_components.building_components(allocator, graph, idx, TransitFlags::CAR);
    if components.as_slice().is_empty() {
        return None;
    }
    Some((
        chunk_for_point(building.center_x, building.center_y),
        components,
        profile,
    ))
}

impl SupplierCandidateIndex {
    /// Builds the dispatch index of the current state at once, as one whole rolling sweep
    /// would ([`DispatchFold::fold`]); items are building handle bits, so every building must
    /// have a handle. O(handles × outputs) on Rayon plus O(E log E) for E entries.
    pub(super) fn build(
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        freight_components: &ModeComponentIndex,
        reservations: &ReservationViews,
        truck_payload_kg: f32,
    ) -> Self {
        let mut entries = DispatchFold {
            allocator,
            graph,
            catalog,
            components: freight_components,
            reservations,
            truck_payload_kg,
        }
        .fold_all();
        Self::from_entry_slice(&mut entries)
    }

    /// Builds the same component/chunk index with caller-owned resource eligibility and dense
    /// building indices as items. Recurring forecasts must test production; dispatch continues
    /// to test physical stock.
    #[cfg(test)]
    pub(super) fn build_filtered(
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        freight_components: &ModeComponentIndex,
        operational_only: bool,
        eligible: impl Fn(usize, ResourceRuntimeId) -> bool + Sync,
    ) -> Self {
        let mut entries = Vec::new();
        Self::collect_filtered(
            allocator,
            graph,
            catalog,
            freight_components,
            operational_only,
            eligible,
            &mut entries,
        );
        Self::from_entries(entries)
    }

    /// The entries [`Self::build_filtered`] indexes, in building order, into `entries`, which
    /// keeps its storage (`SIM-02-K`). O(B × outputs) on Rayon.
    pub(super) fn collect_filtered(
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        freight_components: &ModeComponentIndex,
        operational_only: bool,
        eligible: impl Fn(usize, ResourceRuntimeId) -> bool + Sync,
        entries: &mut Vec<SupplierEntry>,
    ) {
        entries.clear();
        entries.par_extend(
            (0..allocator.buildings.len())
                .into_par_iter()
                .filter_map(|idx| {
                    supplier_site(
                        allocator,
                        graph,
                        catalog,
                        freight_components,
                        idx,
                        operational_only,
                    )
                    .map(|(chunk, components, profile)| (idx, chunk, components, profile))
                })
                .flat_map_iter(|(idx, chunk, components, profile)| {
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
                }),
        );
    }

    /// Drops every resource's buckets, keeping the map for [`Self::insert`] (`SIM-02-K`).
    pub(super) fn clear(&mut self) {
        self.by_resource.clear();
    }

    /// Indexes `resource`'s suppliers as `buckets`, replacing any it had (`SIM-02-K`).
    pub(super) fn insert(&mut self, resource: ResourceRuntimeId, buckets: ReachableBucketIndex) {
        self.by_resource.insert(resource, buckets);
    }

    /// Groups prepared source identities using the same resource/component/chunk index.
    pub(super) fn from_entries(entries: Vec<(ResourceRuntimeId, ReachableBucketEntry)>) -> Self {
        let mut grouped = BTreeMap::new();
        for (resource_runtime_id, entry) in entries {
            grouped
                .entry(resource_runtime_id)
                .or_insert_with(Vec::new)
                .push(entry);
        }

        // Each resource's buckets follow from its own entries alone, so they build on the pool
        // and enter the map as the serial loop entered them (`SIM-02-K`).
        let built: Vec<(ResourceRuntimeId, ReachableBucketIndex)> = grouped
            .into_iter()
            .collect::<Vec<_>>()
            .into_par_iter()
            .map(|(resource_runtime_id, entries)| {
                (
                    resource_runtime_id,
                    ReachableBucketIndex::from_entries(entries),
                )
            })
            .collect();

        Self {
            by_resource: built.into_iter().collect(),
        }
    }

    /// Sorts `entries` in place and groups them as [`Self::from_entries`] does, keeping the
    /// caller's buffer. O(E log E), and O(E) for sorted entries.
    pub(super) fn from_entry_slice(entries: &mut [SupplierEntry]) -> Self {
        entries.sort_unstable();
        let by_resource = entries
            .chunk_by(|left, right| left.0 == right.0)
            .map(|run| {
                (
                    run[0].0,
                    ReachableBucketIndex::from_entries(
                        run.iter().map(|&(_, entry)| entry).collect(),
                    ),
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

    /// Visits every entry in resource, component, chunk and item order, in O(entries).
    pub(super) fn visit_entries(
        &self,
        mut visit: impl FnMut(ResourceRuntimeId, ReachableBucketEntry),
    ) {
        for (&resource, buckets) in &self.by_resource {
            buckets.visit_entries(|entry| visit(resource, entry));
        }
    }

    /// Every entry in resource, component, chunk and item order.
    #[cfg(test)]
    pub(super) fn entries(&self) -> Vec<SupplierEntry> {
        let mut entries = Vec::new();
        self.visit_entries(|resource, entry| entries.push((resource, entry)));
        entries
    }

    /// Whether any resource still has an eligible supplier, in O(resource kinds).
    #[cfg(test)]
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

    /// Clears only availability while retaining prepared geometry and storage.
    pub(super) fn disable_all(&mut self) {
        // Sparse catalogue keys are not a flat collection: Rayon would first collect this
        // map into a temporary vector. Keep the existing allocation-free map traversal.
        for buckets in self.by_resource.values_mut() {
            buckets.reset_available(|_| false);
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
