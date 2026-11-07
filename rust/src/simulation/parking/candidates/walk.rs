// SPDX-License-Identifier: GPL-2.0-only

//! Bounded walking searches from one door over the directed pedestrian lanes.
//!
//! A lane's label is the least left-to-right float sum over every path to it: float addition is
//! monotone, so that value depends neither on the order lanes settle in nor on anything beyond
//! the lanes a path uses. Labels of a settled lane are final, which is what lets a search stop
//! as soon as the lanes it was asked about have settled.

use crate::config::AGENT_WALK_SPEED_MS;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph, lanes::LaneType};
use rayon::prelude::*;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

pub(super) const WALK_LIMIT_S: f32 = 400.0 / AGENT_WALK_SPEED_MS;

// A position on one pedestrian lane, with the two walking times along it. `lane` is `usize::MAX`
// when the entrance has no such lane in reach.
#[derive(Clone, Copy, Debug)]
struct FootEnd {
    lane: usize,
    along_m: f32,
    // Walking from the lane's start to here, and from here to its end.
    from_start_s: f32,
    to_end_s: f32,
}

/// Where an entrance meets the pedestrian lanes, and the walk between its door and the kerb.
/// Depends on the entrance and the lanes alone, so a rebuild derives it once, not once a door.
#[derive(Clone, Copy, Debug)]
pub(super) struct FootEnds {
    ends: [FootEnd; 2],
    local_s: f32,
}

impl Default for FootEnd {
    fn default() -> Self {
        Self {
            lane: usize::MAX,
            along_m: 0.0,
            from_start_s: 0.0,
            to_end_s: 0.0,
        }
    }
}

impl Default for FootEnds {
    fn default() -> Self {
        Self {
            ends: [FootEnd::default(); 2],
            local_s: 0.0,
        }
    }
}

impl FootEnds {
    /// The pedestrian lanes this entrance meets. O(1).
    pub(super) fn lanes(&self) -> impl Iterator<Item = usize> + '_ {
        self.ends.iter().map(|end| end.lane).filter(|&lane| lane != usize::MAX)
    }

    /// These ends on `foot`'s lane numbers. A lane outside it is out of every search's reach, so
    /// the end drops out as if the lane were missing. O(1) expected.
    pub(super) fn on(mut self, foot: &FootGraph) -> Self {
        for end in &mut self.ends {
            end.lane = foot.ids.get(&end.lane).map_or(usize::MAX, |&id| id as usize);
        }
        self
    }

    /// O(log edge samples + lane geometry), allocation-free.
    pub(super) fn of(
        entrance: &BuildingEntrance,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Self {
        let point = graph.get_edge(entrance.edge_idx).map(|e| {
            BuildingAllocator::sample_pos_on_edge(
                graph,
                entrance.edge_idx,
                entrance.entrance_s_m / e.physical_length,
            )
        });
        let ends = [entrance.foot_lane_fwd, entrance.foot_lane_bkw].map(|id| {
            let Some(lane) = network.lane_system.lanes.get(id) else {
                return FootEnd::default();
            };
            let along_m = point.map_or(0.0, |p| {
                BuildingAllocator::project_point_to_polyline_s(&lane.geometry, p)
            });
            FootEnd {
                lane: id,
                along_m,
                from_start_s: along_m / AGENT_WALK_SPEED_MS,
                to_end_s: (lane.length - along_m) / AGENT_WALK_SPEED_MS,
            }
        });
        Self {
            ends,
            local_s: entrance.door_pos.distance_to(entrance.curb_pos) / AGENT_WALK_SPEED_MS,
        }
    }
}

// A search's labels, in storage kept between searches: each stamps its own, so no search pays
// for the size of the city.
#[derive(Default)]
struct Labels {
    // Per lane: the stamp of the search that labelled it, and the label.
    labels: Vec<(u32, f32)>,
    stamp: u32,
    heap: BinaryHeap<Reverse<(u32, u32)>>,
}

impl Labels {
    fn begin(&mut self, lanes: usize) {
        if self.labels.len() < lanes {
            self.labels.resize(lanes, (0, 0.0));
        }
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.labels.fill((0, 0.0));
            self.stamp = 1;
        }
        self.heap.clear();
    }

    #[inline]
    fn get(&self, lane: usize) -> Option<f32> {
        let (stamp, cost) = self.labels[lane];
        (stamp == self.stamp).then_some(cost)
    }

    #[inline]
    fn relax(&mut self, lane: usize, cost: f32) {
        if cost.is_finite() && cost <= WALK_LIMIT_S && self.get(lane).is_none_or(|old| cost < old) {
            self.labels[lane] = (self.stamp, cost);
            self.heap.push(Reverse((cost.to_bits(), lane as u32)));
        }
    }

    // The next lane to settle, skipping entries a better label replaced.
    #[inline]
    fn settle(&mut self) -> Option<(usize, f32)> {
        while let Some(Reverse((bits, lane))) = self.heap.pop() {
            let cost = f32::from_bits(bits);
            if self.get(lane as usize) == Some(cost) {
                return Some((lane as usize, cost));
            }
        }
        None
    }
}

thread_local! {
    static FLOOD: RefCell<Labels> = RefCell::default();
}

// Every lane within the walking limit of `seeds`, whose whole length counts as free, the seeds
// among them. A walker on a seed starts somewhere along it, so this holds every lane a search from
// an entrance on a seed can label. O(L log L + adjacency), L those lanes.
fn flood(
    seeds: &[usize],
    reverse: bool,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> Vec<usize> {
    FLOOD.with_borrow_mut(|labels| {
        labels.begin(network.lane_system.lanes.len());
        for &seed in seeds {
            labels.labels[seed] = (labels.stamp, 0.0);
        }
        for &seed in seeds {
            visit_adjacent(seed, reverse, network, graph, |next| labels.relax(next, 0.0));
        }
        let mut reached = seeds.to_vec();
        while let Some((lane, cost)) = labels.settle() {
            reached.push(lane);
            let next_cost = cost + network.lane_system.lanes[lane].length / AGENT_WALK_SPEED_MS;
            if next_cost <= WALK_LIMIT_S {
                visit_adjacent(lane, reverse, network, graph, |next| {
                    labels.relax(next, next_cost)
                });
            }
        }
        reached
    })
}

/// The foot lanes within the walking limit of a set of entrances, numbered compactly with their
/// successors and predecessors read once. Every search of one rebuild crosses the same lanes;
/// the per-junction predecessor scan and the wide lane rows stay out of those searches.
#[derive(Default)]
pub(super) struct FootGraph {
    ids: HashMap<usize, u32>,
    // Walking time of each lane, and each lane's neighbours by direction.
    walk_s: Vec<f32>,
    adjacent: [Adjacency; 2],
}

#[derive(Default)]
struct Adjacency {
    starts: Vec<u32>,
    lanes: Vec<u32>,
}

impl Adjacency {
    #[inline]
    fn of(&self, lane: usize) -> &[u32] {
        &self.lanes[self.starts[lane] as usize..self.starts[lane + 1] as usize]
    }
}

impl FootGraph {
    /// The lanes round `seeds`, the lanes of the entrances the searches will start from or end
    /// at. O(L log L + adjacency), L lanes in reach of them; the floods run side by side and the
    /// adjacency in parallel, independent of the rest of the city.
    pub(super) fn around(
        mut seeds: Vec<usize>,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Self {
        seeds.sort_unstable();
        seeds.dedup();
        let (mut lanes, backward) = rayon::join(
            || flood(&seeds, false, network, graph),
            || flood(&seeds, true, network, graph),
        );
        lanes.extend(backward);
        lanes.sort_unstable();
        lanes.dedup();
        let ids: HashMap<usize, u32> = lanes
            .iter()
            .enumerate()
            .map(|(i, &lane)| (lane, i as u32))
            .collect();
        let read = |reverse| {
            let parts: Vec<_> = lanes
                .par_chunks(256)
                .map(|chunk| {
                    let mut counts = Vec::with_capacity(chunk.len());
                    let mut adjacent: Vec<u32> = Vec::with_capacity(chunk.len() * 2);
                    for &lane in chunk {
                        let start = adjacent.len();
                        visit_adjacent(lane, reverse, network, graph, |next| {
                            adjacent.extend(ids.get(&next));
                        });
                        counts.push((adjacent.len() - start) as u32);
                    }
                    (counts, adjacent)
                })
                .collect();
            let mut out = Adjacency {
                starts: Vec::with_capacity(lanes.len() + 1),
                lanes: Vec::new(),
            };
            out.starts.push(0);
            for (counts, adjacent) in parts {
                let mut at = out.lanes.len() as u32;
                for count in counts {
                    at += count;
                    out.starts.push(at);
                }
                out.lanes.extend(adjacent);
            }
            out
        };
        let adjacent = [read(false), read(true)];
        let walk_s = lanes
            .iter()
            .map(|&lane| network.lane_system.lanes[lane].length / AGENT_WALK_SPEED_MS)
            .collect();
        Self {
            ids,
            walk_s,
            adjacent,
        }
    }

    /// Whether no entrance gave it a lane: rows read off it need no walking ends. O(1).
    pub(super) fn is_empty(&self) -> bool {
        self.walk_s.is_empty()
    }

    #[inline]
    fn neighbours(&self, lane: usize, reverse: bool) -> &[u32] {
        self.adjacent[usize::from(reverse)].of(lane)
    }
}

/// One entrance's labels in one direction, or several entrances' at once. `reverse` walks towards
/// the entrance. Searches run on [`FootGraph`] numbers: see [`FootEnds::on`].
#[derive(Default)]
pub(super) struct WalkField {
    labels: Labels,
    seeds: Vec<FootEnds>,
    reverse: bool,
}

impl WalkField {
    /// Labels every lane within the walking limit of `seeds`. O(L log L + adjacency), L lanes in
    /// reach; allocation-free once the field has seen as many lanes and as long a queue.
    pub(super) fn search(&mut self, seeds: &[FootEnds], reverse: bool, foot: &FootGraph) {
        self.search_to(seeds, reverse, &[], foot);
    }

    /// [`Self::search`], stopping once every lane of `wanted` (sorted) has settled. Only those
    /// lanes' labels may be read after that.
    pub(super) fn search_to(
        &mut self,
        seeds: &[FootEnds],
        reverse: bool,
        wanted: &[usize],
        foot: &FootGraph,
    ) {
        let mut unsettled = wanted.len();
        self.labels.begin(foot.walk_s.len());
        self.seeds.clear();
        self.seeds.extend_from_slice(seeds);
        self.reverse = reverse;
        for door in seeds {
            for end in door.ends {
                if end.lane == usize::MAX {
                    continue;
                }
                let along = if reverse {
                    end.from_start_s
                } else {
                    end.to_end_s
                };
                let cost = door.local_s + along;
                for &next in foot.neighbours(end.lane, reverse) {
                    self.labels.relax(next as usize, cost);
                }
            }
        }
        while let Some((id, cost)) = self.labels.settle() {
            // A lane settles once: later entries of it no longer match its label.
            if wanted.binary_search(&id).is_ok() {
                unsettled -= 1;
                if unsettled == 0 {
                    return;
                }
            }
            let next_cost = cost + foot.walk_s[id];
            if next_cost > WALK_LIMIT_S {
                continue;
            }
            for &next in foot.neighbours(id, reverse) {
                self.labels.relax(next as usize, next_cost);
            }
        }
    }

    /// Walking time between the seeds and `target`, infinite out of reach. O(seeds).
    #[inline]
    pub(super) fn time_at(&self, target: &FootEnds) -> f32 {
        let mut best = f32::INFINITY;
        for end in target.ends {
            if end.lane == usize::MAX {
                continue;
            }
            if let Some(cost) = self.labels.get(end.lane) {
                let along = if self.reverse {
                    end.to_end_s
                } else {
                    end.from_start_s
                };
                best = best.min(cost + along + target.local_s);
            }
            for door in &self.seeds {
                for seed in door.ends {
                    let along = if self.reverse {
                        seed.along_m - end.along_m
                    } else {
                        end.along_m - seed.along_m
                    };
                    if seed.lane == end.lane && along >= 0.0 {
                        best = best
                            .min(door.local_s + along / AGENT_WALK_SPEED_MS + target.local_s);
                    }
                }
            }
        }
        best
    }
}

fn visit_adjacent(
    id: usize,
    reverse: bool,
    network: &TransitNetwork,
    graph: &RegionGraph,
    mut visit: impl FnMut(usize),
) {
    let lanes = &network.lane_system.lanes;
    let valid = |i: usize| {
        lanes.get(i).is_some_and(|l| {
            l.lane_type == LaneType::Foot
                && (l.edge_id == usize::MAX
                    || graph.get_edge(l.edge_id).is_some_and(|e| !e.deleted))
        })
    };
    if !valid(id) {
        return;
    }
    if !reverse {
        for &next in &lanes[id].next_lanes {
            if valid(next) {
                visit(next);
            }
        }
        return;
    }
    let lane = &lanes[id];
    let node = if lane.edge_id == usize::MAX {
        lane.node_id as u32
    } else {
        let Some(edge) = graph.get_edge(lane.edge_id) else {
            return;
        };
        if lane.is_fwd {
            edge.start_node
        } else {
            edge.end_node
        }
    };
    // Existing per-node and per-edge lane indices enumerate only this junction's predecessors.
    if let Some(connections) = network.lane_system.node_lanes.get(&(node as usize)) {
        for &candidate in connections {
            if valid(candidate) && lanes[candidate].next_lanes.contains(&id) {
                visit(candidate);
            }
        }
    }
    if (node as usize) < graph.node_adjacency_count() {
        for &edge in graph.node_adjacency(node) {
            if let Some(roads) = network.lane_system.edge_lanes.get(&edge) {
                for &candidate in roads {
                    if valid(candidate) && lanes[candidate].next_lanes.contains(&id) {
                        visit(candidate);
                    }
                }
            }
        }
    }
}
