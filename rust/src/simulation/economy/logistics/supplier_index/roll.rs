// SPDX-License-Identifier: GPL-2.0-only

//! The hour-long rolling build of the dispatch supplier index (`SIM-02-I`,
//! `docs/simulation.md#snapshots`).
//!
//! Readers use the front, which the previous hour's sweep finished. The hour's first slot
//! freezes the building handle capacity `H`; chunk `k` folds the buildings in handle slots
//! `[k·H/S, (k+1)·H/S)`, with `S` the hour's slots, into the back with the dispatch eligibility
//! against the live state of its slot. A slot freed since the start is skipped, a reused one
//! folds its current building, and buildings in later slots count next hour. The back keeps
//! each component/chunk bucket's items in handle order, already sorted, so the hour's last slot
//! groups it into the front in linear time; the next hour's pass reads that front. A sweep that
//! misses a slot (the clock set directly) or whose freight components change is abandoned; the
//! next pass then builds its own index at once.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::accessibility::ReachableBucketEntry;
use crate::simulation::economy::definitions::{
    ResourceRuntimeId, load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

use super::super::data::ShipmentSystem;
use super::super::reservations::current_views;
use super::{DispatchFold, SupplierCandidateIndex, SupplierEntry};

/// One saved entry: resource, component, routing chunk x and y, building handle bits.
pub(crate) type SavedSupplierEntry = (u16, u32, i32, i32, u64);

/// Saved sweep in progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SavedSupplierSweep {
    /// Building handle slots frozen at the hour's first slot.
    pub(crate) building_handles: u64,
    /// Logistics hour whose pass reads the finished index.
    pub(crate) hour: u32,
    /// Chunk the sweep expects next.
    pub(crate) next_chunk: u64,
    /// Entries folded so far, sorted.
    pub(crate) entries: Vec<SavedSupplierEntry>,
}

/// Saved state of a [`SupplierRoll`], so a loaded game continues it bit-identically.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SavedSupplierRoll {
    /// The logistics hour the front serves and its entries, sorted.
    pub(crate) front: Option<(u32, Vec<SavedSupplierEntry>)>,
    /// The sweep in progress.
    pub(crate) sweep: Option<SavedSupplierSweep>,
    /// Whether the front and the sweep were folded with the saved road graph's freight
    /// components. A load labels only those with its own; the others stay unusable, as the
    /// saved game would have found them at their next use.
    pub(crate) current: [bool; 2],
}

// Resource, component and routing chunk of one bucket of the back.
type BucketKey = (ResourceRuntimeId, u32, (i32, i32));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sweep {
    // Building handle slots frozen at the hour's first slot.
    building_handles: u64,
    // Logistics hour whose pass reads the finished index.
    hour: u32,
    // Freight component generation the entries were folded with.
    components: Option<u32>,
}

/// Front and back of the dispatch supplier index.
#[derive(Clone, Debug, Default)]
pub(crate) struct SupplierRoll {
    // The front, lent to the planning pass of logistics hour `front_hour`, and the component
    // generation its labels belong to.
    front: Option<SupplierCandidateIndex>,
    front_hour: Option<u32>,
    front_components: Option<u32>,
    // The sweep in progress and the chunk it expects next.
    sweep: Option<Sweep>,
    next_chunk: u64,
    // Items folded so far per bucket, each list in ascending handle order. An emptied list
    // keeps its key and room, so sweeps over the same buckets allocate nothing.
    back: BTreeMap<BucketKey, Vec<usize>>,
    // Component generation of the back's keys.
    back_components: Option<u32>,
    // Reused entry list of the publish.
    publish_entries: Vec<SupplierEntry>,
    // Whether a restored front and sweep are current, until a load labels them.
    restored_current: [bool; 2],
}

impl SupplierRoll {
    /// The front for the pass of logistics hour `hour` over freight components of generation
    /// `components`; `None` when no sweep finished one for them. O(1).
    pub(in crate::simulation::economy::logistics) fn take_front(
        &mut self,
        hour: u32,
        components: Option<u32>,
    ) -> Option<SupplierCandidateIndex> {
        if self.front_hour != Some(hour) || self.front_components != components {
            return None;
        }
        self.front.take()
    }

    /// Takes back the front lent by [`Self::take_front`] for hour `hour`.
    pub(in crate::simulation::economy::logistics) fn restore_front(
        &mut self,
        hour: u32,
        index: SupplierCandidateIndex,
    ) {
        if self.front_hour == Some(hour) {
            self.front = Some(index);
        }
    }

    /// Makes `index`, built at once for logistics hour `hour` over freight components of
    /// generation `components`, the front that hour's later passes read. A sweep in progress
    /// keeps going and replaces it at the hour's end. O(1).
    pub(in crate::simulation::economy::logistics) fn install_front(
        &mut self,
        hour: u32,
        components: Option<u32>,
        index: SupplierCandidateIndex,
    ) {
        self.front = Some(index);
        self.front_hour = Some(hour);
        self.front_components = components;
    }

    // Chunk `slot mod S` of the hour's sweep for the pass after logistics hour `hour_ticks`.
    fn advance(
        &mut self,
        slot: u64,
        hour_ticks: u32,
        components: Option<u32>,
        fold: &DispatchFold,
    ) {
        let chunk = slot % SLOTS_PER_HOUR;
        if chunk == 0 {
            self.start(hour_ticks, components, fold);
        } else if chunk != self.next_chunk
            || self
                .sweep
                .is_some_and(|sweep| sweep.components != components)
        {
            self.sweep = None;
        }
        let Some(sweep) = self.sweep else {
            return;
        };
        self.next_chunk = chunk + 1;
        let back = &mut self.back;
        fold.fold(part(chunk, sweep.building_handles), |resource, entry| {
            add_to_back(back, resource, entry)
        });
        if chunk == SLOTS_PER_HOUR - 1 {
            self.publish(sweep);
        }
    }

    // Freezes the handle capacity and empties the back: O(back buckets), and a clear of the
    // buckets themselves after the components changed.
    fn start(&mut self, hour_ticks: u32, components: Option<u32>, fold: &DispatchFold) {
        self.sweep = Some(Sweep {
            building_handles: fold.allocator.building_handles.capacity() as u64,
            hour: hour_ticks.wrapping_add(1),
            components,
        });
        if self.back_components == components {
            self.back.values_mut().for_each(Vec::clear);
        } else {
            self.back.clear();
            self.back_components = components;
        }
    }

    // The back's buckets, in key order and each sorted, become the front. O(E) for E entries.
    fn publish(&mut self, sweep: Sweep) {
        self.publish_entries.clear();
        for (&(resource, component, chunk), items) in &self.back {
            self.publish_entries.extend(
                items
                    .iter()
                    .map(|&item| (resource, ReachableBucketEntry::new(component, chunk, item))),
            );
        }
        self.front = Some(SupplierCandidateIndex::from_entry_slice(
            &mut self.publish_entries,
        ));
        self.front_hour = Some(sweep.hour);
        self.front_components = sweep.components;
        self.sweep = None;
    }

    /// Saved form, for comparisons; see [`Self::saved_for`].
    #[cfg(test)]
    pub(crate) fn saved(&self) -> SavedSupplierRoll {
        self.saved_for(None)
    }

    /// Saved form for a game whose freight components are of generation `layout`, `None` when
    /// a road change has not reached them yet; [`Self::restore`] rebuilds a roll that continues
    /// identically.
    pub(crate) fn saved_for(&self, layout: Option<u32>) -> SavedSupplierRoll {
        let current = |components: Option<u32>, restored: bool| match components {
            Some(components) => layout == Some(components),
            None => restored,
        };
        let current = [
            self.front_hour.is_some() && current(self.front_components, self.restored_current[0]),
            self.sweep
                .is_some_and(|sweep| current(sweep.components, self.restored_current[1])),
        ];
        let front = self
            .front_hour
            .zip(self.front.as_ref())
            .map(|(hour, front)| {
                let mut entries = Vec::new();
                front.visit_entries(|resource, entry| entries.push(saved_entry(resource, entry)));
                (hour, entries)
            });
        let sweep = self.sweep.map(|sweep| {
            let mut entries = Vec::new();
            for (&(resource, component, chunk), items) in &self.back {
                entries.extend(items.iter().map(|&item| {
                    saved_entry(resource, ReachableBucketEntry::new(component, chunk, item))
                }));
            }
            SavedSupplierSweep {
                building_handles: sweep.building_handles,
                hour: sweep.hour,
                next_chunk: self.next_chunk,
                entries,
            }
        });
        SavedSupplierRoll {
            front,
            sweep,
            current,
        }
    }

    /// Rebuilds a saved roll; its component generation is unknown until
    /// [`ShipmentSystem::adopt_loaded_topology`] labels the buffers saved as current.
    pub(crate) fn restore(saved: SavedSupplierRoll) -> Result<Self, String> {
        let mut roll = Self {
            restored_current: saved.current,
            ..Self::default()
        };
        if let Some((hour, entries)) = saved.front {
            let mut entries: Vec<SupplierEntry> = entries.into_iter().map(loaded_entry).collect();
            roll.front = Some(SupplierCandidateIndex::from_entry_slice(&mut entries));
            roll.front_hour = Some(hour);
        }
        if let Some(sweep) = saved.sweep {
            if sweep.next_chunk == 0 || sweep.next_chunk >= SLOTS_PER_HOUR {
                return Err("supplier sweep chunk outside the hour".to_string());
            }
            roll.sweep = Some(Sweep {
                building_handles: sweep.building_handles,
                hour: sweep.hour,
                components: None,
            });
            roll.next_chunk = sweep.next_chunk;
            let mut entries: Vec<SupplierEntry> =
                sweep.entries.into_iter().map(loaded_entry).collect();
            entries.sort_unstable();
            for (resource, entry) in entries {
                add_to_back(&mut roll.back, resource, entry);
            }
        }
        Ok(roll)
    }
}

impl ShipmentSystem {
    /// Slot `slot`'s chunk of the hour's supplier sweep (`SIM-02-I`); the caller registers
    /// appended buildings' handles first. O(H/S × outputs × log buckets) per slot and
    /// allocation-free once every bucket has room; the hour's first slot also empties the
    /// back's buckets and its last builds the front, O(E) for E entries. A road-graph change
    /// costs O(nodes + edges) once.
    pub(crate) fn advance_supplier_sweep(
        &mut self,
        slot: u64,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.with_dispatch_fold(
            allocator,
            network,
            graph,
            |roll, hour_ticks, components, fold| roll.advance(slot, hour_ticks, components, fold),
        );
    }

    /// Stamps a loaded supplier roll with the loaded road graph's freight components, where the
    /// saved game's buffers were folded with its own. They label every component as the saved
    /// graph did: labels follow node order, and saving keeps the order of the nodes it keeps. A
    /// buffer folded before a road change stays unlabelled, so it is rebuilt or abandoned at its
    /// next use as in the saved game. O(nodes + edges).
    pub(crate) fn adopt_loaded_topology(&mut self, network: &TransitNetwork, graph: &RegionGraph) {
        self.freight_topology.refresh(network, graph);
        let components = self.freight_topology.components_generation();
        let roll = &mut self.supplier_roll;
        let [front_current, sweep_current] = std::mem::take(&mut roll.restored_current);
        if roll.front_hour.is_some() && front_current {
            roll.front_components = components;
        }
        if let Some(sweep) = &mut roll.sweep
            && sweep_current
        {
            sweep.components = components;
            roll.back_components = components;
        }
    }

    /// The supplier roll as saved: its buffers count as current while they were folded with the
    /// network's CCH build and no road change waits for the next rebuild.
    pub(crate) fn saved_supplier_roll(&self, network: &TransitNetwork) -> SavedSupplierRoll {
        let layout = network
            .cch_dirty_chunks
            .is_empty()
            .then_some(network.cch_graph.build_generation);
        self.supplier_roll.saved_for(layout)
    }

    // Runs `step` on the roll with the live dispatch eligibility, after refreshing the freight
    // topology and the reservation views it reads.
    fn with_dispatch_fold(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        step: impl FnOnce(&mut SupplierRoll, u32, Option<u32>, &DispatchFold),
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let truck_payload_kg = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"))
            .logistics
            .truck_payload_kg;
        self.freight_topology.refresh(network, graph);
        let reservations = current_views(
            &mut self.reservations,
            &self.shipments,
            catalog.resource_count(),
        );
        let fold = DispatchFold {
            allocator,
            graph,
            catalog: &catalog,
            components: self.freight_topology.components(),
            reservations,
            truck_payload_kg,
        };
        step(
            &mut self.supplier_roll,
            self.hour_ticks,
            self.freight_topology.components_generation(),
            &fold,
        );
    }
}

// Appends `entry` to its bucket; handle slots fold in ascending order, so a repeat (one profile
// listing a resource twice) can only be the bucket's last item. O(log buckets).
fn add_to_back(
    back: &mut BTreeMap<BucketKey, Vec<usize>>,
    resource: ResourceRuntimeId,
    entry: ReachableBucketEntry,
) {
    let (component, chunk, item) = entry.parts();
    let items = back.entry((resource, component, chunk)).or_default();
    if items.last() != Some(&item) {
        items.push(item);
    }
}

// Handle slots folded by chunk `chunk` of a sweep over `handles` slots.
fn part(chunk: u64, handles: u64) -> Range<usize> {
    (chunk * handles / SLOTS_PER_HOUR) as usize..((chunk + 1) * handles / SLOTS_PER_HOUR) as usize
}

fn saved_entry(resource: ResourceRuntimeId, entry: ReachableBucketEntry) -> SavedSupplierEntry {
    let (component, (x, y), item) = entry.parts();
    (resource, component, x, y, item as u64)
}

fn loaded_entry((resource, component, x, y, item): SavedSupplierEntry) -> SupplierEntry {
    (
        resource,
        ReachableBucketEntry::new(component, (x, y), item as usize),
    )
}
