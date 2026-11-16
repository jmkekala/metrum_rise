// SPDX-License-Identifier: GPL-2.0-only

//! Site edits: bringing the published catchments up to date with the bays that changed.
//!
//! A door's list is a pure function of its own entrance, the usable bays in reach and the foot
//! lanes between them. A site edit leaves the lanes alone, so a door other than the changed
//! buildings' own can only gain or lose the changed bays: the rest of its list keeps its bits,
//! and those bays' times come from the door's own searches, cut short once they have settled.
//! Every road edit, and any edit that finds routing stale, takes the full rebuild instead.

use super::super::*;
use super::table::{EconomyRow, FootRow, finish_economy_ends, within_door_reach};
use super::walk::{ESTIMATE_MARGIN_S, FootEnds, FootGraph, WALK_LIMIT_S, WalkField};
use super::{DestinationCandidates, ParkingCandidate, ParkingEconomyEnds};
use crate::simulation::buildings::allocator::{
    BuildingAllocator, BuildingEntrance, BuildingEntranceKey,
};
use crate::simulation::economy::accessibility::ModeComponentIndex;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use rayon::prelude::*;

/// A bay whose place in the catchments may differ since the last publication: added, removed,
/// reactivated, or with a new path or access. Derived, never saved.
#[derive(Clone, Debug)]
pub(in crate::simulation::parking) struct PlaceChange {
    pub place: Handle<Place>,
    pub position: [f32; 2],
    // Its access when it last counted as supply; the default for a new bay.
    pub access: BuildingEntrance,
    // Whether it was usable supply at the last publication, so that a door's list may hold it.
    // A bay added or reactivated since is on no list: lists hold only usable bays.
    pub listed: bool,
}

// One changed bay as a publication reads it.
struct ChangedPlace {
    place: Handle<Place>,
    position: [f32; 2],
    listed: bool,
    // The ends a door loses with it, and the edge those ends name.
    lost: EconomyRow,
    lost_edge: usize,
    // Its rows now, when it is usable supply.
    row: Option<FootRow>,
    economy: EconomyRow,
}

// The usable changed bays' side of the doors' searches: the lanes round them, and one search from
// all of them at once, which tells the doors it cannot reach from those it must measure. The
// searches from their lanes' ends and to their lanes' starts bound each door's own two searches.
struct ChangedReach {
    foot: FootGraph,
    from_bays: WalkField,
    from_lane_ends: WalkField,
    to_lane_starts: WalkField,
    // One bit per slot index modulo 64 of each changed bay a list may hold: an entry whose bit
    // is clear is none of them, so a door's list is read without a search per entry.
    slots: u64,
}

#[inline]
fn slot_bit(place: Handle<Place>) -> u64 {
    1 << (place.index() % 64)
}

// One door's new list.
struct DoorEdit {
    handle: u64,
    door: [f32; 2],
    list: DestinationCandidates,
}

// The component labels and target pairs a door's list no longer gives.
type GoneEnds = (Vec<u32>, Vec<(u32, usize)>);

impl ParkingSystem {
    /// Journals one bay's change for the next publication; `listed` when it is usable supply
    /// that has been published. O(1), allocation-free while the journal's capacity lasts.
    pub(in crate::simulation::parking) fn journal_place(
        &mut self,
        place: Handle<Place>,
        listed: bool,
    ) {
        let Some(p) = self.place_handles.resolve(place) else {
            return;
        };
        let [x, _, z] = self.places.position[p];
        self.place_changes.push(PlaceChange {
            place,
            position: [x, z],
            access: self.places.access[p].clone(),
            listed,
        });
    }

    /// Publishes the bay changes of a site edit. `own` are the doors of the changed buildings
    /// that hold a list ([`holds_parking`]), rebuilt whole; `hosts` are those buildings, whose bays follow their doors; `near` are the
    /// doors any changed bay could reach. O(own full rebuilds + H bays of the hosts + D doors ×
    /// changed bays + R × (searches cut at the changed bays + list length)), R the doors a bay
    /// actually enters or leaves, parallel by door. The caller needs current routing: without it
    /// the economy ends wait for the adoption and the full rebuild owns that bookkeeping.
    /// Returns the doors whose list of places changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn publish_place_changes(
        &mut self,
        own: &[usize],
        hosts: &[u64],
        near: &[usize],
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
    ) -> Vec<[f32; 2]> {
        crate::thread_pools::install(|| {
            self.follow_host_doors(hosts, allocator, network, graph);
            let mut changed = self.changed_places(allocator, network, graph, components);
            let (chunks, bounds) = self.door_neighbourhood(own, allocator);
            // The lanes round the own doors and round the changed bays read neither the bays'
            // access nor the lists, so they are found while the access is refreshed.
            let ((), (foot, reach)) = rayon::join(
                || self.refresh_chunk_access(&chunks, bounds, allocator, network, graph),
                || {
                    rayon::join(
                        || super::door_foot_graph(own, allocator, network, graph),
                        || changed_reach(&mut changed, network, graph),
                    )
                },
            );
            // The changed buildings' own doors and every other door touch different lists, and
            // neither stage fills the pool alone: one door's table is a few chunks of work.
            let this = &*self;
            let (mut built, edits) = rayon::join(
                || {
                    let table = this.place_table(foot, &chunks, bounds, network, graph, None);
                    this.door_lists(own, &table, false, allocator, network, graph)
                },
                || {
                    near.par_iter()
                        .filter(|b| own.binary_search(b).is_err())
                        .filter_map(|&b| {
                            this.door_edit(
                                b, &changed, &reach, allocator, network, graph, components,
                            )
                        })
                        .collect::<Vec<_>>()
                },
            );
            let mut lists: Vec<_> = built
                .iter_mut()
                .filter_map(|(handle, _, list)| {
                    let b = allocator
                        .building_handles
                        .resolve(Handle::from_bits(*handle))?;
                    Some((b, list))
                })
                .collect();
            this.read_list_ends(&mut lists, allocator, network, graph, components);
            let mut doors = self.publish_door_lists(built, false);
            if !near.is_empty() {
                self.candidate_revision = self.candidate_revision.wrapping_add(1);
            }
            for edit in edits {
                let list = edit.list;
                let old = self
                    .candidates
                    .get(&edit.handle)
                    .map_or(&[][..], |c| &c.entries[..]);
                if !list
                    .entries
                    .iter()
                    .map(|e| e.place)
                    .eq(old.iter().map(|e| e.place))
                {
                    doors.push(edit.door);
                }
                if list.entries.is_empty() {
                    self.candidates.remove(&edit.handle);
                } else {
                    self.candidates.insert(edit.handle, list);
                }
            }
            doors
        })
    }

    // Journals the bays of `hosts` whose access moved with their door. O(their bays).
    fn follow_host_doors(
        &mut self,
        hosts: &[u64],
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        for host in hosts {
            let Some(sources) = self.building_sources.get(host) else {
                continue;
            };
            let bays: Vec<_> = sources
                .iter()
                .filter_map(|source| self.sources.get(source))
                .flatten()
                .copied()
                .collect();
            for place in bays {
                let Some(p) = self.place_handles.resolve(place) else {
                    continue;
                };
                let access = self
                    .derive_place_entrance(place, allocator, network, graph)
                    .unwrap_or_default();
                if self.places.active[p]
                    && BuildingEntranceKey::new(&access)
                        != BuildingEntranceKey::new(&self.places.access[p])
                {
                    self.journal_place(place, true);
                }
            }
        }
    }

    // Publishes the journaled bays' access and reads what the doors need of them, once. The
    // journal keeps its storage. O(changes log changes × trip ends).
    fn changed_places(
        &mut self,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
    ) -> Vec<ChangedPlace> {
        let mut journal = std::mem::take(&mut self.place_changes);
        // Stable: a bay changed twice keeps the access it last counted with.
        journal.sort_by_key(|change| change.place);
        journal.dedup_by_key(|change| change.place);
        let mut changed = Vec::with_capacity(journal.len());
        for change in journal.drain(..) {
            self.refresh_place_access(change.place, allocator, network, graph);
            let mut old = change.access;
            BuildingAllocator::refresh_entrance_lanes(&mut old, graph, &network.lane_system);
            let usable = self
                .place_handles
                .resolve(change.place)
                .filter(|&p| self.places.active[p]);
            let access = usable.and_then(|_| self.live_access(change.place, network, graph));
            changed.push(ChangedPlace {
                place: change.place,
                position: change.position,
                listed: change.listed,
                lost: EconomyRow::at(&old, components, network, graph),
                lost_edge: old.edge_idx,
                row: access
                    .as_ref()
                    .map(|access| self.foot_row(change.place, access, network, graph)),
                economy: access.as_ref().map_or_else(EconomyRow::default, |access| {
                    EconomyRow::at(access, components, network, graph)
                }),
            });
        }
        self.place_changes = journal;
        changed
    }

    // One door's list with the changed bays taken out and their new rows merged in, or `None`
    // when none of them concerns it.
    #[allow(clippy::too_many_arguments)]
    fn door_edit(
        &self,
        building: usize,
        changed: &[ChangedPlace],
        reach: &ChangedReach,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
    ) -> Option<DoorEdit> {
        let entrance = allocator.entrances.get(building)?;
        let handle = allocator.building_handles.handle(building);
        if handle.is_none() || !super::lists_door(allocator, building) {
            return None;
        }
        let handle = handle.to_bits();
        let door = [entrance.door_pos.x, entrance.door_pos.y];
        let in_reach = |at: [f32; 2]| within_door_reach(at[0] - door[0], at[1] - door[1]);
        if !changed.iter().any(|c| in_reach(c.position)) {
            return None;
        }
        let old = self.candidates.get(&handle);
        let entries = old.map_or(&[][..], |c| &c.entries[..]);
        let lost = |place: Handle<Place>| {
            (reach.slots & slot_bit(place) != 0)
                .then(|| changed.binary_search_by_key(&place, |c| c.place).ok())
                .flatten()
        };
        // A list holds none of the changed bays when none of them was published supply.
        let gone = if reach.slots == 0 {
            0
        } else {
            entries.iter().filter(|e| lost(e.place).is_some()).count()
        };
        let bays = changed.iter().filter_map(|c| {
            c.row
                .as_ref()
                .filter(|row| in_reach(row.position))
                .map(|row| (c.place, row))
        });
        // Only a door a usable bay may enter needs its own walking ends.
        let found = if bays.clone().next().is_some() {
            let ends = FootEnds::of(entrance, network, graph).on(&reach.foot);
            if reach.from_bays.time_at(&ends) <= WALK_LIMIT_S + ESTIMATE_MARGIN_S {
                let bounds = [&reach.from_lane_ends, &reach.to_lane_starts];
                super::table::door_rows(ends, bays, bounds, &reach.foot)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        if gone == 0 && found.is_empty() {
            return None;
        }
        let mut merged = Vec::with_capacity(entries.len() - gone + found.len());
        let mut found_at = found.iter().peekable();
        for &entry in entries.iter().filter(|e| lost(e.place).is_none()) {
            while let Some(&&next) = found_at.peek() {
                if !self.sorts_before(next, entry) {
                    break;
                }
                merged.push(next);
                found_at.next();
            }
            merged.push(entry);
        }
        merged.extend(found_at);
        let centre = &allocator.buildings[building];
        let centre = [centre.center_x, centre.center_y];
        let empty = ParkingEconomyEnds::default();
        let ends = old.map_or(&empty, |c| &c.economy);
        // The list's ends are the ends of its bays. Bays of one edit share an edge and its ends,
        // so each pair is proved once; an end of a lost bay that no bay left gives is dropped.
        let mut proved: Vec<(&EconomyRow, usize)> = Vec::new();
        let mut gone_ends = GoneEnds::default();
        // The list holds exactly `gone` of them: no scan once the last is found.
        for i in entries.iter().filter_map(|e| lost(e.place)).take(gone) {
            let bay = &changed[i];
            let pair = (&bay.lost, bay.lost_edge);
            if proved.contains(&pair) {
                continue;
            }
            proved.push(pair);
            if !self.still_ends(pair.0, pair.1, &merged, network, graph, components) {
                let out = &mut gone_ends;
                self.gone_ends(pair.0, pair.1, &merged, network, graph, components, out);
            }
        }
        let added = found.iter().map(|c| {
            let i = changed.binary_search_by_key(&c.place, |c| c.place);
            &changed[i.expect("found among the changed bays")].economy
        });
        let economy = if gone_ends.0.is_empty() && gone_ends.1.is_empty() {
            ends.with_rows(added, centre, graph)
        } else {
            ends.without(&gone_ends, centre, graph)
                .with_rows(added, centre, graph)
        };
        Some(DoorEdit {
            handle,
            door,
            list: DestinationCandidates {
                entries: merged,
                economy,
            },
        })
    }

    // The catchment order of two bays: walking time, the fixed driveway time, the place.
    fn sorts_before(&self, a: ParkingCandidate, b: ParkingCandidate) -> bool {
        a.walk_s
            .total_cmp(&b.walk_s)
            .then_with(|| {
                self.maneuver_seconds(a.place)
                    .total_cmp(&self.maneuver_seconds(b.place))
            })
            .then(a.place.cmp(&b.place))
            .is_lt()
    }

    // Whether the bays left on `edge` still give every end of `row`. A target names its frontage
    // edge, so no other bay can; a false answer only means the door must look for each end.
    fn still_ends(
        &self,
        row: &EconomyRow,
        edge: usize,
        entries: &[ParkingCandidate],
        network: &TransitNetwork,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
    ) -> bool {
        if row.is_empty() {
            return true;
        }
        entries.iter().any(|entry| {
            self.place_handles
                .resolve(entry.place)
                .filter(|&p| self.places.access[p].edge_idx == edge)
                .and_then(|_| self.live_access(entry.place, network, graph))
                .is_some_and(|access| {
                    EconomyRow::at(&access, components, network, graph).covers(row)
                })
        })
    }

    // Adds to `gone` the ends of `row`, a lost bay's on `edge`, that no bay of `entries` gives. A
    // target names its frontage edge, so only a bay on `edge` can give it; a component any bay.
    // O(entries × trip ends) at worst; the first bays usually give the components.
    #[allow(clippy::too_many_arguments)]
    fn gone_ends(
        &self,
        row: &EconomyRow,
        edge: usize,
        entries: &[ParkingCandidate],
        network: &TransitNetwork,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
        gone: &mut GoneEnds,
    ) {
        let mut labels = row.components().to_vec();
        let mut targets: Vec<_> = row.targets().collect();
        for entry in entries {
            if labels.is_empty() && targets.is_empty() {
                break;
            }
            let Some(p) = self.place_handles.resolve(entry.place) else {
                continue;
            };
            if labels.is_empty() && self.places.access[p].edge_idx != edge {
                continue;
            }
            let Some(access) = self.live_access(entry.place, network, graph) else {
                continue;
            };
            let given = EconomyRow::at(&access, components, network, graph);
            labels.retain(|label| !given.components().contains(label));
            targets.retain(|&target| given.targets().all(|own| own != target));
        }
        gone.0.extend(labels);
        gone.1.extend(targets);
    }

    // Reads each list's ends off its bays, each bay's ends once however many lists share it.
    // O(entries log entries + distinct bays × trip ends), parallel, nothing without a list.
    fn read_list_ends(
        &self,
        lists: &mut [(usize, &mut DestinationCandidates)],
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
    ) {
        let mut places: Vec<_> = lists
            .iter()
            .flat_map(|(_, list)| &list.entries)
            .map(|entry| entry.place)
            .collect();
        places.par_sort_unstable();
        places.dedup();
        let rows: Vec<_> = places
            .par_iter()
            .with_min_len(32)
            .map(|&place| {
                self.live_access(place, network, graph)
                    .map_or_else(EconomyRow::default, |access| {
                        EconomyRow::at(&access, components, network, graph)
                    })
            })
            .collect();
        lists.par_iter_mut().for_each(|(building, list)| {
            let mut labels = Vec::new();
            let mut targets = Vec::new();
            for entry in &list.entries {
                if let Ok(i) = places.binary_search(&entry.place) {
                    labels.extend_from_slice(rows[i].components());
                    targets.extend(rows[i].targets());
                }
            }
            let centre = &allocator.buildings[*building];
            list.economy =
                finish_economy_ends(labels, targets, [centre.center_x, centre.center_y], graph);
        });
    }
}

// Puts the usable changed bays on the lanes round them and searches from all of them. O(lanes in
// reach of them).
fn changed_reach(
    changed: &mut [ChangedPlace],
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> ChangedReach {
    let lanes = changed
        .iter()
        .flat_map(|c| c.row.iter())
        .flat_map(|row| row.foot.lanes())
        .collect();
    let foot = FootGraph::around(lanes, network, graph);
    let mut seeds = Vec::with_capacity(changed.len());
    for row in changed.iter_mut().filter_map(|c| c.row.as_mut()) {
        row.foot = row.foot.on(&foot);
        seeds.push(row.foot);
    }
    let lanes: Vec<_> = seeds.iter().map(|seed| seed.whole_lanes()).collect();
    let mut from_bays = WalkField::default();
    let mut from_lane_ends = WalkField::default();
    let mut to_lane_starts = WalkField::default();
    crate::thread_pools::install(|| {
        rayon::join(
            || from_bays.search(&seeds, false, &foot),
            || {
                rayon::join(
                    || from_lane_ends.search(&lanes, false, &foot),
                    || to_lane_starts.search(&lanes, true, &foot),
                )
            },
        )
    });
    let slots = changed
        .iter()
        .filter(|c| c.listed)
        .fold(0, |slots, c| slots | slot_bit(c.place));
    ChangedReach {
        foot,
        from_bays,
        from_lane_ends,
        to_lane_starts,
        slots,
    }
}

impl ParkingEconomyEnds {
    // These ends less `gone`, measured again from `centre`. O(ends × gone + ends log ends).
    fn without(&self, gone: &GoneEnds, centre: [f32; 2], graph: &RegionGraph) -> Self {
        let components = self
            .components
            .iter()
            .copied()
            .filter(|label| !gone.0.contains(label))
            .collect();
        let targets = self
            .targets
            .iter()
            .copied()
            .filter(|target| !gone.1.contains(target))
            .collect();
        finish_economy_ends(components, targets, centre, graph)
    }

    // These ends with `rows` added: the same ends, shared, when they already hold every value.
    fn with_rows<'a>(
        &self,
        rows: impl Iterator<Item = &'a EconomyRow> + Clone,
        centre: [f32; 2],
        graph: &RegionGraph,
    ) -> Self {
        let held = rows.clone().all(|row| {
            row.components()
                .iter()
                .all(|c| self.components.binary_search(c).is_ok())
                && row
                    .targets()
                    .all(|t| self.targets.binary_search(&t).is_ok())
        });
        if held {
            return Self {
                components: self.components.clone(),
                targets: self.targets.clone(),
                node_slack_m: self.node_slack_m,
            };
        }
        let mut components = self.components.to_vec();
        let mut targets = self.targets.to_vec();
        for row in rows {
            components.extend_from_slice(row.components());
            targets.extend(row.targets());
        }
        finish_economy_ends(components, targets, centre, graph)
    }
}
