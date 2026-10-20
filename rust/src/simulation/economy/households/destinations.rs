// SPDX-License-Identifier: GPL-2.0-only

//! Where households shop and visit (`SIM-02-J`): the stores selling household supplies and the
//! service stores hosting visible visits, as building handles in buckets by foot and car
//! component and 512 m chunk.
//!
//! The index is a rolling snapshot (`docs/simulation.md#snapshots`). The hour's first slot
//! freezes the building handle capacity `H`; chunk `k` folds the buildings in handle slots
//! `[k·H/S, (k+1)·H/S)`, with `S` the hour's slots, against the live state of its slot, and the
//! hour's last slot sorts the folded entries into the front that the next hour's restock and
//! visit requests read. Readers resolve each handle and check the destination again, so only
//! eligibility lags: a store that sold out since its chunk is skipped, and one restocked since
//! waits for the next sweep. A sweep that misses a slot (the clock set directly) or whose
//! components change is abandoned; a reader that finds no front for its hour and components
//! builds one at once.

use std::ops::Range;

use rayon::prelude::*;

use super::metrics::{building_operation_factors, economy_profile_for_building};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::handles::Handle;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketIndex, chunk_for_point,
    max_speed_for_modes,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;

// Destination kinds: the index of each in the per-kind arrays.
const STORES: usize = 0;
const SERVICES: usize = 1;

// Building handle slots per parallel task of a whole fold.
const HANDLES_PER_TASK: usize = 1_024;

/// Foot and car components of the road graph, rebuilt only when the CCH is: every topology edit
/// rebuilds it, so its build generation covers edges and their modes. O(1) when unchanged.
#[derive(Clone, Debug, Default)]
pub(crate) struct HouseholdTopology {
    generation: Option<u32>,
    foot: ModeComponentIndex,
    car: ModeComponentIndex,
    // Top walking and car commute speeds, and the CCH build and metric revision they read.
    speeds: Option<((u32, u64), [f32; 2])>,
}

impl HouseholdTopology {
    /// Brings the components up to date with `network`: O(nodes + edges) after a rebuild.
    pub(crate) fn refresh(&mut self, network: &TransitNetwork, graph: &RegionGraph) {
        let generation = Some(network.cch_graph.build_generation);
        if self.generation != generation {
            self.foot = ModeComponentIndex::build(graph, TransitFlags::FOOT);
            self.car = ModeComponentIndex::build(graph, TransitFlags::CAR);
            self.generation = generation;
        }
    }

    /// CCH build generation of the components; `None` before the first refresh.
    pub(crate) fn generation(&self) -> Option<u32> {
        self.generation
    }

    /// Foot component labels of the last refresh.
    pub(crate) fn foot(&self) -> &ModeComponentIndex {
        &self.foot
    }

    /// Car component labels of the last refresh.
    pub(crate) fn car(&self) -> &ModeComponentIndex {
        &self.car
    }

    /// The fastest walking and car commute speeds, for job search's lower bounds: O(edges)
    /// after a CCH rebuild or metric change, O(1) otherwise.
    pub(crate) fn max_commute_speeds(
        &mut self,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> [f32; 2] {
        let key = (
            network.cch_graph.build_generation,
            network.cch_graph.metric_revision,
        );
        if let Some((cached, speeds)) = self.speeds
            && cached == key
        {
            return speeds;
        }
        let speeds = [
            max_speed_for_modes(graph, TransitFlags::FOOT).max(1.0),
            max_speed_for_modes(graph, TransitFlags::FOOT | TransitFlags::CAR).max(1.0),
        ];
        self.speeds = Some((key, speeds));
        speeds
    }
}

/// One destination: its building, 512 m chunk and the components its door reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DestinationEntry {
    pub(super) building: Handle<Building>,
    pub(super) chunk: (i32, i32),
    pub(super) foot: BuildingModeComponents,
    pub(super) car: BuildingModeComponents,
}

/// One saved destination: building handle bits, chunk x and y, then the foot and car
/// components as `(labels, count)`.
pub(crate) type SavedDestination = (u64, i32, i32, ([u32; 2], usize), ([u32; 2], usize));

impl DestinationEntry {
    fn saved(self) -> SavedDestination {
        (
            self.building.to_bits(),
            self.chunk.0,
            self.chunk.1,
            self.foot.raw_parts(),
            self.car.raw_parts(),
        )
    }

    fn loaded((building, x, y, foot, car): SavedDestination) -> Result<Self, String> {
        Ok(Self {
            building: Handle::from_bits(building),
            chunk: (x, y),
            foot: BuildingModeComponents::from_raw_parts(foot.0, foot.1)
                .ok_or("destination with too many foot components")?,
            car: BuildingModeComponents::from_raw_parts(car.0, car.1)
                .ok_or("destination with too many car components")?,
        })
    }
}

/// Destinations of one kind in `(chunk, handle)` order, with their foot and car buckets, whose
/// items are positions in that order.
#[derive(Clone, Debug, Default)]
pub(super) struct DestinationIndex {
    pub(super) entries: Vec<DestinationEntry>,
    pub(super) foot_buckets: ReachableBucketIndex,
    pub(super) car_buckets: ReachableBucketIndex,
}

impl DestinationIndex {
    // O(E log E) for E entries.
    fn from_entries(mut entries: Vec<DestinationEntry>) -> Self {
        entries.sort_unstable_by_key(|entry| (entry.chunk, entry.building));
        let mut foot_bucket_entries = Vec::with_capacity(entries.len());
        let mut car_bucket_entries = Vec::with_capacity(entries.len());
        for (position, entry) in entries.iter().enumerate() {
            entry
                .foot
                .append_bucket_entries(&mut foot_bucket_entries, entry.chunk, position);
            entry
                .car
                .append_bucket_entries(&mut car_bucket_entries, entry.chunk, position);
        }
        Self {
            entries,
            foot_buckets: ReachableBucketIndex::from_entries(foot_bucket_entries),
            car_buckets: ReachableBucketIndex::from_entries(car_bucket_entries),
        }
    }

    /// Whether the index lists any destination.
    pub(super) fn has_any(&self) -> bool {
        !self.entries.is_empty()
    }
}

/// The destinations one hour's requests read.
#[derive(Clone, Debug)]
pub(super) struct DestinationFront {
    // Absolute hour served, and the topology generation of its components.
    hour: u64,
    components: Option<u32>,
    pub(super) stores: DestinationIndex,
    pub(super) services: DestinationIndex,
}

impl DestinationFront {
    fn new(
        hour: u64,
        components: Option<u32>,
        [stores, services]: [Vec<DestinationEntry>; 2],
    ) -> Self {
        Self {
            hour,
            components,
            stores: DestinationIndex::from_entries(stores),
            services: DestinationIndex::from_entries(services),
        }
    }
}

/// Whether `building` sells household supplies (`supplies`) to shoppers now. O(outputs).
pub(super) fn store_sells_supplies(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    supplies: u16,
) -> bool {
    !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
        && building.edge_idx != usize::MAX
        && matches!(building.zone_type, ZoneType::Commercial)
        && building.inventory_units(supplies) > 0.0
        && economy_profile_for_building(catalog, building)
            .is_some_and(|profile| profile.output_port(supplies).is_some())
}

/// Whether `building` is a staffed service store selling `resource` that can host a visible
/// visit now. O(outputs).
pub(super) fn service_hosts_visits(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    resource: ResourceRuntimeId,
) -> bool {
    if building.broken
        || building.economy_broken
        || building.is_deserted
        || building.is_under_construction()
        || building.edge_idx == usize::MAX
        || !matches!(building.zone_type, ZoneType::Commercial)
    {
        return false;
    }
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return false;
    };
    profile.kind == EconomyProfileRuntimeKind::ServiceStore
        && profile.output_port(resource).is_some()
        && building_operation_factors(catalog, building, profile).throughput_factor > 0.0
}

/// Live state a fold reads.
pub(super) struct DestinationFold<'a> {
    pub(super) allocator: &'a BuildingAllocator,
    pub(super) graph: &'a RegionGraph,
    pub(super) catalog: &'a RuntimeEconomyCatalog,
    pub(super) topology: &'a HouseholdTopology,
    pub(super) supplies: u16,
    pub(super) services: Option<ResourceRuntimeId>,
}

impl DestinationFold<'_> {
    // The kinds building `idx` serves now, and its entry; `None` when it serves neither or its
    // door reaches no component. O(outputs).
    fn entry(&self, idx: usize) -> Option<([bool; 2], DestinationEntry)> {
        let building = &self.allocator.buildings[idx];
        let kinds = [
            store_sells_supplies(self.catalog, building, self.supplies),
            self.services
                .is_some_and(|resource| service_hosts_visits(self.catalog, building, resource)),
        ];
        if kinds == [false; 2] {
            return None;
        }
        let allocator = self.allocator;
        let foot =
            self.topology
                .foot
                .building_components(allocator, self.graph, idx, TransitFlags::FOOT);
        let car =
            self.topology
                .car
                .building_components(allocator, self.graph, idx, TransitFlags::CAR);
        if foot.as_slice().is_empty() && car.as_slice().is_empty() {
            return None;
        }
        Some((
            kinds,
            DestinationEntry {
                building: allocator.building_handles.handle(idx),
                chunk: chunk_for_point(building.center_x, building.center_y),
                foot,
                car,
            },
        ))
    }

    // Appends the destinations in handle slots `slots` to `back`, in slot order; a free slot
    // adds nothing. O(slots × outputs), allocation-free once `back` has room.
    fn fold(&self, slots: Range<usize>, back: &mut [Vec<DestinationEntry>; 2]) {
        for slot in slots {
            let Some(idx) = self.allocator.building_handles.dense_at(slot) else {
                continue;
            };
            if let Some((kinds, entry)) = self.entry(idx) {
                for kind in [STORES, SERVICES] {
                    if kinds[kind] {
                        back[kind].push(entry);
                    }
                }
            }
        }
    }

    // Every handle slot's destinations, folded on Rayon in fixed parts and merged in slot
    // order. O(handles × outputs).
    fn fold_all(&self) -> [Vec<DestinationEntry>; 2] {
        let handles = self.allocator.building_handles.capacity();
        let parts: Vec<[Vec<DestinationEntry>; 2]> = (0..handles.div_ceil(HANDLES_PER_TASK))
            .into_par_iter()
            .map(|task| {
                let start = task * HANDLES_PER_TASK;
                let mut back = [Vec::new(), Vec::new()];
                self.fold(start..(start + HANDLES_PER_TASK).min(handles), &mut back);
                back
            })
            .collect();
        let mut all = [Vec::new(), Vec::new()];
        for part in parts {
            for (kind, entries) in part.into_iter().enumerate() {
                all[kind].extend(entries);
            }
        }
        all
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sweep {
    // Building handle slots frozen at the hour's first slot.
    building_handles: u64,
    // Absolute hour whose requests read the finished front.
    hour: u64,
    // Topology generation the entries were folded with.
    components: Option<u32>,
}

/// Saved sweep in progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SavedDestinationSweep {
    /// Building handle slots frozen at the hour's first slot.
    pub(crate) building_handles: u64,
    /// Absolute hour whose requests read the finished front.
    pub(crate) hour: u64,
    /// Chunk the sweep expects next.
    pub(crate) next_chunk: u64,
    /// Stores and service stores folded so far, each in handle order.
    pub(crate) entries: [Vec<SavedDestination>; 2],
}

/// Saved state of a [`DestinationRoll`], so a loaded game continues it identically.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SavedDestinationRoll {
    /// The absolute hour the front serves, and its stores and service stores in index order.
    pub(crate) front: Option<(u64, [Vec<SavedDestination>; 2])>,
    /// The sweep in progress.
    pub(crate) sweep: Option<SavedDestinationSweep>,
    /// Whether the front and the sweep were folded with the saved game's road topology. A load
    /// labels only those with its own; the others are rebuilt or abandoned at their next use,
    /// as the saved game would have.
    pub(crate) current: [bool; 2],
}

/// Front and back of the household destination index.
#[derive(Clone, Debug, Default)]
pub(crate) struct DestinationRoll {
    front: Option<DestinationFront>,
    sweep: Option<Sweep>,
    next_chunk: u64,
    // Destinations folded so far by kind, each in handle order.
    back: [Vec<DestinationEntry>; 2],
    // Whether a restored front and sweep are current, until a load labels them.
    restored_current: [bool; 2],
}

impl DestinationRoll {
    /// Slot `slot`'s chunk of the hour's sweep. O(H/S × outputs) and allocation-free once the
    /// back has room; the hour's last slot also builds the front, O(E log E) for E entries.
    pub(super) fn advance(&mut self, slot: u64, fold: &DestinationFold) {
        let chunk = slot % SLOTS_PER_HOUR;
        let components = fold.topology.generation();
        if chunk == 0 {
            self.sweep = Some(Sweep {
                building_handles: fold.allocator.building_handles.capacity() as u64,
                hour: slot / SLOTS_PER_HOUR + 1,
                components,
            });
            self.back.iter_mut().for_each(Vec::clear);
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
        fold.fold(part(chunk, sweep.building_handles), &mut self.back);
        if chunk == SLOTS_PER_HOUR - 1 {
            let back = self.back.clone();
            self.front = Some(DestinationFront::new(sweep.hour, sweep.components, back));
            self.sweep = None;
        }
    }

    /// Lends the front for the requests of absolute hour `hour`, built at once when none was
    /// finished for that hour and the fold's components; [`Self::restore_front`] takes it back.
    /// A sweep in progress keeps going and replaces it at the hour's end. O(1) with a front.
    pub(super) fn take_front(&mut self, hour: u64, fold: &DestinationFold) -> DestinationFront {
        let components = fold.topology.generation();
        match self.front.take() {
            Some(front) if front.hour == hour && front.components == components => front,
            _ => DestinationFront::new(hour, components, fold.fold_all()),
        }
    }

    /// Whether the front for absolute hour `hour` lists stores and service stores; `None`
    /// before the hour has one. O(1).
    pub(super) fn listed(&self, hour: u64) -> Option<[bool; 2]> {
        self.front
            .as_ref()
            .filter(|front| front.hour == hour)
            .map(|front| [front.stores.has_any(), front.services.has_any()])
    }

    /// Takes back a front lent by [`Self::take_front`].
    pub(super) fn restore_front(&mut self, front: DestinationFront) {
        self.front = Some(front);
    }

    /// Stamps a loaded roll with the loaded topology's generation. Components label as the
    /// saved graph did: labels follow node order, and saving keeps the order of the nodes it
    /// keeps.
    pub(crate) fn adopt_loaded_topology(&mut self, topology: &HouseholdTopology) {
        let [front_current, sweep_current] = std::mem::take(&mut self.restored_current);
        if let Some(front) = &mut self.front
            && front_current
        {
            front.components = topology.generation();
        }
        if let Some(sweep) = &mut self.sweep
            && sweep_current
        {
            sweep.components = topology.generation();
        }
    }

    /// Saved form, for comparisons; see [`Self::saved_for`].
    #[cfg(test)]
    pub(crate) fn saved(&self) -> SavedDestinationRoll {
        self.saved_for(None)
    }

    /// Saved form for a game whose road components are of CCH build `generation`, `None` while
    /// a road change waits for the next rebuild, marking which buffers were folded with them;
    /// [`Self::restore`] rebuilds a roll that continues identically.
    pub(crate) fn saved_for(&self, generation: Option<u32>) -> SavedDestinationRoll {
        let entries = |entries: &[DestinationEntry]| -> Vec<SavedDestination> {
            entries.iter().map(|entry| entry.saved()).collect()
        };
        let current = |components: Option<Option<u32>>, restored: bool| match components {
            Some(Some(components)) => generation == Some(components),
            _ => restored,
        };
        SavedDestinationRoll {
            current: [
                current(
                    self.front.as_ref().map(|front| front.components),
                    self.restored_current[0],
                ),
                current(
                    self.sweep.map(|sweep| sweep.components),
                    self.restored_current[1],
                ),
            ],
            front: self.front.as_ref().map(|front| {
                (
                    front.hour,
                    [
                        entries(&front.stores.entries),
                        entries(&front.services.entries),
                    ],
                )
            }),
            sweep: self.sweep.map(|sweep| SavedDestinationSweep {
                building_handles: sweep.building_handles,
                hour: sweep.hour,
                next_chunk: self.next_chunk,
                entries: [entries(&self.back[STORES]), entries(&self.back[SERVICES])],
            }),
        }
    }

    /// Rebuilds a saved roll; its components' generation is unknown until
    /// [`Self::adopt_loaded_topology`] labels the buffers saved as current.
    pub(crate) fn restore(saved: SavedDestinationRoll) -> Result<Self, String> {
        let loaded =
            |entries: [Vec<SavedDestination>; 2]| -> Result<[Vec<DestinationEntry>; 2], String> {
                let [stores, services] = entries;
                let load = |entries: Vec<SavedDestination>| {
                    entries
                        .into_iter()
                        .map(DestinationEntry::loaded)
                        .collect::<Result<Vec<_>, _>>()
                };
                Ok([load(stores)?, load(services)?])
            };
        let mut roll = Self {
            restored_current: saved.current,
            ..Self::default()
        };
        if let Some((hour, entries)) = saved.front {
            roll.front = Some(DestinationFront::new(hour, None, loaded(entries)?));
        }
        if let Some(sweep) = saved.sweep {
            if sweep.next_chunk == 0 || sweep.next_chunk >= SLOTS_PER_HOUR {
                return Err("destination sweep chunk outside the hour".to_string());
            }
            roll.sweep = Some(Sweep {
                building_handles: sweep.building_handles,
                hour: sweep.hour,
                components: None,
            });
            roll.next_chunk = sweep.next_chunk;
            roll.back = loaded(sweep.entries)?;
        }
        Ok(roll)
    }
}

// Handle slots folded by chunk `chunk` of a sweep over `handles` slots.
fn part(chunk: u64, handles: u64) -> Range<usize> {
    (chunk * handles / SLOTS_PER_HOUR) as usize..((chunk + 1) * handles / SLOTS_PER_HOUR) as usize
}
