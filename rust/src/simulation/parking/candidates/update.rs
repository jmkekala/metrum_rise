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
use super::walk::{FootEnds, FootGraph, WALK_LIMIT_S, WalkField};
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
}

// One changed bay as a publication reads it.
struct ChangedPlace {
    place: Handle<Place>,
    position: [f32; 2],
    // The ends a door loses with it, and the edge those ends name.
    lost: EconomyRow,
    lost_edge: usize,
    // Its rows now, when it is usable supply.
    row: Option<FootRow>,
    economy: EconomyRow,
}

// Far past the rounding between a walk summed from one end and the same walk from the other.
const ESTIMATE_MARGIN_S: f32 = 0.25;

// The usable changed bays' side of the doors' searches: the lanes round them, and one search from
// all of them at once, which tells the doors it cannot reach from those it must measure.
struct ChangedReach {
    foot: FootGraph,
    from_bays: WalkField,
}

// One door's new list; `whole_economy` when its ends must be read off every entry.
struct DoorEdit {
    building: usize,
    handle: u64,
    door: [f32; 2],
    list: DestinationCandidates,
    whole_economy: bool,
}

impl ParkingSystem {
    /// Journals one bay's change for the next publication. O(1), allocation-free while the
    /// journal's capacity lasts.
    pub(in crate::simulation::parking) fn journal_place(&mut self, place: Handle<Place>) {
        let Some(p) = self.place_handles.resolve(place) else {
            return;
        };
        let [x, _, z] = self.places.position[p];
        self.place_changes.push(PlaceChange {
            place,
            position: [x, z],
            access: self.places.access[p].clone(),
        });
    }

    /// Publishes the bay changes of a site edit. `own` are the doors of the changed buildings,
    /// rebuilt whole; `hosts` are those buildings, whose bays follow their doors; `near` are the
    /// doors any changed bay could reach. O(own full rebuilds + H bays of the hosts + D doors ×
    /// changed bays + R × (searches cut at the changed bays + list length)), R the doors a bay
    /// actually enters or leaves, parallel by door. The caller needs current routing: without it
    /// the economy ends wait for the adoption and the full rebuild owns that bookkeeping.
    /// Returns the doors whose list of places changed.
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
            let mut doors = self.rebuild_doors(own, false, allocator, network, graph, Some(components));
            let reach = changed_reach(&mut changed, network, graph);
            let this = &*self;
            let mut edits: Vec<_> = near
                .par_iter()
                .filter(|b| own.binary_search(b).is_err())
                .filter_map(|&b| {
                    this.door_edit(b, &changed, &reach, allocator, network, graph, components)
                })
                .collect();
            this.finish_whole_economy(&mut edits, allocator, network, graph, components);
            if !near.is_empty() {
                self.candidate_revision = self.candidate_revision.wrapping_add(1);
            }
            for edit in edits {
                let list = edit.list;
                let old = self
                    .candidates
                    .get(&edit.handle)
                    .map_or(&[][..], |c| &c.entries[..]);
                if !list.entries.iter().map(|e| e.place).eq(old.iter().map(|e| e.place)) {
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
                    self.journal_place(place);
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
        if handle.is_none() {
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
        let lost = |place| changed.binary_search_by_key(&place, |c: &ChangedPlace| c.place);
        let gone = entries.iter().filter(|e| lost(e.place).is_ok()).count();
        let bays = changed.iter().filter_map(|c| {
            c.row
                .as_ref()
                .filter(|row| in_reach(row.position))
                .map(|row| (c.place, row))
        });
        let ends = FootEnds::of(entrance, network, graph).on(&reach.foot);
        let found = if bays.clone().next().is_some()
            && reach.from_bays.time_at(&ends) <= WALK_LIMIT_S + ESTIMATE_MARGIN_S
        {
            super::table::door_rows(ends, bays, &reach.foot)
        } else {
            Vec::new()
        };
        if gone == 0 && found.is_empty() {
            return None;
        }
        let mut merged = Vec::with_capacity(entries.len() - gone + found.len());
        let mut found_at = found.iter().peekable();
        for &entry in entries.iter().filter(|e| lost(e.place).is_err()) {
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
        // Bays of one edit share an edge and its ends, so each pair is proved once.
        let mut proved: Vec<(&EconomyRow, usize)> = Vec::new();
        let whole_economy = entries.iter().filter_map(|e| lost(e.place).ok()).any(|i| {
            let bay = &changed[i];
            let pair = (&bay.lost, bay.lost_edge);
            if proved.contains(&pair) {
                return false;
            }
            proved.push(pair);
            !self.still_ends(pair.0, pair.1, &merged, network, graph, components)
        });
        let economy = if whole_economy {
            ParkingEconomyEnds::default()
        } else {
            let added = found.iter().map(|c| {
                let i = lost(c.place).expect("found among the changed bays");
                &changed[i].economy
            });
            ends.with_rows(added, centre, graph)
        };
        Some(DoorEdit {
            building,
            handle,
            door,
            list: DestinationCandidates {
                entries: merged,
                economy,
            },
            whole_economy,
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
    // edge, so no other bay can; a false answer only means the door must read its whole list.
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
                .is_some_and(|access| EconomyRow::at(&access, components, network, graph).covers(row))
        })
    }

    // Reads the economy ends of the doors that lost the last bay of an end, from one table of
    // their neighbourhood. O(bays around them + their entries), nothing without such a door.
    fn finish_whole_economy(
        &self,
        edits: &mut [DoorEdit],
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
        components: &ModeComponentIndex,
    ) {
        let doors: Vec<_> = edits
            .iter()
            .filter(|edit| edit.whole_economy)
            .map(|edit| edit.building)
            .collect();
        if doors.is_empty() {
            return;
        }
        let (chunks, bounds) = self.door_neighbourhood(&doors, allocator);
        let foot = FootGraph::default();
        let table = self.place_table(foot, &chunks, bounds, network, graph, Some(components));
        let index = table.place_index();
        edits.par_iter_mut().filter(|edit| edit.whole_economy).for_each(|edit| {
            let centre = &allocator.buildings[edit.building];
            edit.list.economy = table.economy_of(
                &edit.list.entries,
                &index,
                [centre.center_x, centre.center_y],
                graph,
            );
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
    let mut from_bays = WalkField::default();
    from_bays.search(&seeds, false, &foot);
    ChangedReach { foot, from_bays }
}

impl ParkingEconomyEnds {
    // These ends with `rows` added: the same ends, shared, when they already hold every value.
    fn with_rows<'a>(
        &self,
        rows: impl Iterator<Item = &'a EconomyRow> + Clone,
        centre: [f32; 2],
        graph: &RegionGraph,
    ) -> Self {
        let held = rows.clone().all(|row| {
            row.components().iter().all(|c| self.components.binary_search(c).is_ok())
                && row.targets().all(|t| self.targets.binary_search(&t).is_ok())
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
