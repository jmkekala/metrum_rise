// SPDX-License-Identifier: GPL-2.0-only

//! Prepared recipe/door variants for supply changes inside construction transactions.

use super::*;
use crate::simulation::economy::accessibility::ReachableBucketIndex;
use std::ops::Range;
use crate::simulation::buildings::allocator::DemandSiteEntranceKey;

/// A proposed supplier retains its actual recipe and door independently of the live building.
/// Preparation owns this data; source handles never masquerade as allocator building indices.
#[derive(Clone, Debug)]
pub(crate) struct ProjectedSupplySource {
    owner: usize,
    asset: String,
    profile: u16,
    entrance: BuildingEntrance,
    entrance_key: Option<DemandSiteEntranceKey>,
}

impl ProjectedSupplySource {
    /// Records a specific replacement candidate during pass preparation, before quotations.
    pub(crate) fn new(owner: usize, asset: &str, profile: u16, entrance: BuildingEntrance) -> Self {
        Self { owner, asset: asset.to_owned(), profile, entrance, entrance_key: None }
    }

    /// Records exact geometry inputs for reuse after unrelated live-layout changes.
    pub(crate) fn with_entrance_key(mut self, key: DemandSiteEntranceKey) -> Self {
        self.entrance_key = Some(key);
        self
    }

    /// Searches the previous sorted variants without allocating or scanning other owners.
    pub(crate) fn retained_entrance<'a>(sources: &'a [Self], owner: usize, asset: &str)
        -> Option<(DemandSiteEntranceKey, &'a BuildingEntrance)> {
        let index = sources.binary_search_by(|source|
            source.owner.cmp(&owner).then(source.asset.as_str().cmp(asset))).ok()?;
        let source = &sources[index];
        Some((source.entrance_key?, &source.entrance))
    }
}

/// Geometry retained between construction passes; all quantities belong to the main book.
#[derive(Clone, Debug, Default)]
pub(super) struct ProjectedSources {
    revision: Option<(Option<GeometryRevision>, [u64; 3])>,
    sources: Vec<ProjectedSupplySource>,
    offsets: Vec<usize>,
    owners: Vec<Range<usize>>,
    index: SupplierCandidateIndex,
    enabled: bool,
}

impl RecurringSupply {
    /// Prepares only caller-selected recipe/door variants, reusing the existing supplier
    /// buckets. Geometry rebuild is O(projected ports log ports + owners); warm work is
    /// O(projected ports). No candidate allocates or scans unrelated owners when quoting.
    /// Caller revisions cover zoning/asset changes to proposed doors and recipes. Neither
    /// frontage prices nor financing rebuilds geometry. Call before prepare_owners.
    pub(crate) fn prepare_projected_sources(
        &mut self,
        catalog: &RuntimeEconomyCatalog,
        graph: &RegionGraph,
        caller_revision: [u64; 3],
        factory: impl FnOnce(&[ProjectedSupplySource]) -> Vec<ProjectedSupplySource>,
    ) {
        debug_assert!(!self.revalidation.is_active() && self.staged.is_empty());
        debug_assert!(!self.projected.enabled && self.released.is_empty(), "prepare once after a live fork");
        let Some(components) = &self.components else { return; };
        let base = self.offsets.len() - 1;
        let first_slot = self.offsets[base];
        let projected = &mut self.projected;
        let revision = Some((geometry_revision(self.revision), caller_revision));
        if projected.revision != revision || projected.offsets.first() != Some(&first_slot) {
            projected.sources = factory(&projected.sources);
            projected.sources.sort_unstable_by(|a, b| a.owner.cmp(&b.owner).then(a.asset.cmp(&b.asset)));
            projected.offsets.clear();
            projected.offsets.reserve(projected.sources.len() + 1);
            projected.offsets.push(first_slot);
            let owners = projected.sources.last().map_or(base, |source| base.max(source.owner + 1));
            projected.owners.resize(owners, 0..0);
            projected.owners.fill(0..0);
            // Prefix offsets and owner ranges depend on the previous source in stable order.
            for (index, source) in projected.sources.iter().enumerate() {
                let count = catalog.profile_by_runtime_id(source.profile).map_or(0, |p| p.outputs.len());
                projected.offsets.push(projected.offsets[index] + count);
                let range = &mut projected.owners[source.owner];
                if range.start == range.end { range.start = index; }
                range.end = index + 1;
            }
            let entries = projected.sources.par_iter().enumerate().flat_map_iter(|(index, source)| {
                let access = components.entrance_components(&source.entrance, graph, TransitFlags::CAR);
                let (values, count) = access.raw_parts();
                let chunk = chunk_for_point(source.entrance.door_pos.x, source.entrance.door_pos.y);
                let outputs = catalog.profile_by_runtime_id(source.profile).map_or(&[][..], |p| p.outputs.as_slice());
                outputs.iter().flat_map(move |output| {
                    (0..count).map(move |component| (output.resource_runtime_id,
                        ReachableBucketEntry::new(values[component], chunk, base + index)))
                })
            }).collect();
            projected.index = SupplierCandidateIndex::from_entries(entries);
            projected.index.enable_reservations();
            projected.revision = revision;
        }
        projected.index.disable_all();
        projected.enabled = true;
        let end = *projected.offsets.last().unwrap();
        self.remaining.resize(end, 0.0);
        self.remaining[first_slot..].fill(0.0);
        self.production.resize(end, 0.0);
        self.production[first_slot..].fill(0.0);
        self.active.resize(end, false);
        self.active[first_slot..].fill(false);
        self.source_receipts.resize(end, usize::MAX);
        self.source_receipts[first_slot..].fill(usize::MAX);
        self.staged.reserve(end);
        self.released.reserve(end);
        self.offers.reserve(end);
    }

    /// Finds the prepared variant in this owner's range, without a city or catalogue scan.
    pub(crate) fn projected_source(&self, owner: usize, asset: &str) -> Option<usize> {
        if !self.projected.enabled { return None; }
        self.projected.owners.get(owner)?.clone().find(|&index| self.projected.sources[index].asset == asset)
            .map(|index| self.offsets.len() - 1 + index)
    }

    /// Changes one source's funded net production while retaining outstanding buyer claims.
    /// Household quantities are already reserved by the caller. Zero activity withdraws old
    /// output; negative remaining quantities identify commitments that must be revalidated.
    /// O(source ports + changed bucket eligibility); outer rejection restores exact balances.
    pub(crate) fn replace_source_output(
        &mut self,
        source: usize,
        area: f32,
        activity: f32,
        household: impl Fn(u16) -> f64,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        graph: &RegionGraph,
    ) {
        debug_assert!(self.staged.is_empty() && self.released.is_empty());
        debug_assert!(self.revalidation.is_active(), "source replacement belongs to an enclosing proof");
        let Some(profile) = self.source_profile(source, allocator, catalog) else { return; };
        let Some(range) = self.source_range(source) else { return; };
        for (slot, output) in range.zip(&profile.outputs) {
            let produced = f64::from(profile.net_output_units_per_day(output) * area * activity);
            let available = (produced - household(output.resource_runtime_id)).max(0.0);
            self.journal_source(source, output.resource_runtime_id, slot);
            self.remaining[slot] += available - self.production[slot];
            self.production[slot] = available;
            let active = self.remaining[slot] > 0.0;
            if self.active[slot] != active {
                self.active[slot] = active;
                self.set_available(source, output.resource_runtime_id, active, allocator, graph);
            }
        }
    }

    /// Drops pass-specific eligibility, keeping variant geometry for the next warm fork.
    pub(super) fn clear_projected_sources(&mut self) { self.projected.enabled = false; }

    /// Whether the current book includes prepared variant quantities.
    pub(super) fn projected_enabled(&self) -> bool { self.projected.enabled }

    /// Whether any prepared variant currently has unreserved production.
    #[cfg(test)]
    pub(super) fn projected_has_offers(&self) -> bool {
        self.projected.enabled && self.projected.index.has_offers()
    }

    /// Resource-compatible proposed doors; disabled passes never expose stale eligibility.
    pub(super) fn projected_buckets(&self, resource: u16) -> Option<&ReachableBucketIndex> {
        self.projected.enabled.then(|| self.projected.index.buckets_for_resource(resource)).flatten()
    }

    /// Maps a supply handle to its real commitment owner, including unplaced recipe variants.
    pub(super) fn source_owner(&self, source: usize) -> usize {
        let base = self.offsets.len().saturating_sub(1);
        if source < base { source } else { self.projected.sources[source - base].owner }
    }

    /// Sparse quantity range for an original or prepared source handle.
    pub(super) fn source_range(&self, source: usize) -> Option<Range<usize>> {
        let base = self.offsets.len().saturating_sub(1);
        let (offsets, index) = if source < base { (&self.offsets, source) }
            else if self.projected.enabled { (&self.projected.offsets, source - base) }
            else { return None; };
        Some(*offsets.get(index)?..*offsets.get(index + 1)?)
    }

    /// Uses the candidate's actual recipe rather than the incumbent's output ports or price.
    pub(super) fn source_profile<'c>(&self, source: usize, allocator: &BuildingAllocator,
        catalog: &'c RuntimeEconomyCatalog) -> Option<&'c EconomyProfileRuntime> {
        let base = self.offsets.len().saturating_sub(1);
        let profile = if source < base { allocator.buildings.get(source)?.economy_profile_runtime_id }
            else if self.projected.enabled { self.projected.sources.get(source - base)?.profile }
            else { return None; };
        catalog.profile_by_runtime_id(profile)
    }

    /// Source geometry for exact direction, frontage delay and straight-line pruning.
    pub(super) fn source_entrance<'a>(&'a self, source: usize,
        allocator: &'a BuildingAllocator) -> Option<&'a BuildingEntrance> {
        let base = self.offsets.len().saturating_sub(1);
        if source < base { allocator.entrances.get(source) }
        else if self.projected.enabled { self.projected.sources.get(source - base).map(|s| &s.entrance) }
        else { None }
    }

    /// Live pairs keep their complete-route cache; proposed doors use prepared node-cost scratch.
    pub(super) fn source_route(&self, source: usize, buyer: Option<usize>, entrance: &BuildingEntrance,
        allocator: &BuildingAllocator, network: &TransitNetwork, graph: &RegionGraph,
        routes: &mut FreightRouteCache) -> Option<f32> {
        if source < self.offsets.len().saturating_sub(1) {
            if let Some(buyer) = buyer { routes.between_buildings(source, buyer, allocator, network, graph) }
            else { routes.to_site(source, entrance, allocator, network, graph) }
        } else {
            routes.between_entrances(self.source_entrance(source, allocator)?, entrance, network, graph)
        }
    }

    /// Updates only the fixed variant entry; rollback uses the same source handle and door.
    pub(super) fn set_projected_available(&mut self, source: usize, resource: u16,
        available: bool, graph: &RegionGraph) {
        let Some(components) = &self.components else { return; };
        let Some(variant) = self.projected.sources.get(source - (self.offsets.len() - 1)) else { return; };
        let chunk = chunk_for_point(variant.entrance.door_pos.x, variant.entrance.door_pos.y);
        for &component in components.entrance_components(&variant.entrance, graph, TransitFlags::CAR).as_slice() {
            self.projected.index.set_available(resource, ReachableBucketEntry::new(component, chunk, source), available);
        }
    }

    /// Retained quantity/query capacities for allocation regressions, never admission decisions.
    #[cfg(test)]
    pub(crate) fn projected_storage(&self) -> [usize; 9] {
        [self.remaining.capacity(), self.production.capacity(), self.active.capacity(),
            self.source_receipts.capacity(), self.staged.capacity(), self.offers.capacity(),
            self.projected.sources.capacity(), self.projected.offsets.capacity(), self.projected.owners.capacity()]
    }
}
