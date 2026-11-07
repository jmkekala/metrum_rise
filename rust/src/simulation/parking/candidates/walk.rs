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
use std::cmp::Reverse;
use std::collections::BinaryHeap;

pub(super) const WALK_LIMIT_S: f32 = 400.0 / AGENT_WALK_SPEED_MS;

// A position on one pedestrian lane, with the two walking times along it. `lane` is `usize::MAX`
// when the entrance has no such lane.
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

impl FootEnds {
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
                return FootEnd {
                    lane: usize::MAX,
                    along_m: 0.0,
                    from_start_s: 0.0,
                    to_end_s: 0.0,
                };
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

/// One door's labels in one direction. Its storage is kept between doors, each search stamping
/// its own labels, so no door pays for the size of the city. `reverse` walks towards the door.
#[derive(Default)]
pub(super) struct WalkField {
    // Per lane: the stamp of the search that labelled it, and the label.
    labels: Vec<(u32, f32)>,
    stamp: u32,
    heap: BinaryHeap<Reverse<(u32, usize)>>,
    door: Option<FootEnds>,
    reverse: bool,
}

impl WalkField {
    fn begin(&mut self, door: FootEnds, reverse: bool, lanes: usize) {
        if self.labels.len() < lanes {
            self.labels.resize(lanes, (0, 0.0));
        }
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.labels.fill((0, 0.0));
            self.stamp = 1;
        }
        self.heap.clear();
        self.door = Some(door);
        self.reverse = reverse;
    }

    #[inline]
    fn label(&self, lane: usize) -> Option<f32> {
        self.labels
            .get(lane)
            .filter(|(stamp, _)| *stamp == self.stamp)
            .map(|&(_, cost)| cost)
    }

    fn relax(&mut self, lane: usize, cost: f32) {
        if cost.is_finite()
            && cost <= WALK_LIMIT_S
            && self.label(lane).is_none_or(|old| cost < old)
        {
            self.labels[lane] = (self.stamp, cost);
            self.heap.push(Reverse((cost.to_bits(), lane)));
        }
    }

    /// Labels every lane within the walking limit of `door`. O(L log L + adjacency), L lanes in
    /// reach; allocation-free once the field has seen as many lanes and as long a queue.
    pub(super) fn search(
        &mut self,
        door: FootEnds,
        reverse: bool,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.begin(door, reverse, network.lane_system.lanes.len());
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
            visit_adjacent(end.lane, reverse, network, graph, |next| {
                self.relax(next, cost)
            });
        }
        while let Some(Reverse((bits, id))) = self.heap.pop() {
            let cost = f32::from_bits(bits);
            if self.label(id) != Some(cost) {
                continue;
            }
            let next_cost = cost + network.lane_system.lanes[id].length / AGENT_WALK_SPEED_MS;
            if next_cost > WALK_LIMIT_S {
                continue;
            }
            visit_adjacent(id, reverse, network, graph, |next| {
                self.relax(next, next_cost)
            });
        }
    }

    /// Walking time between the door and `target`, infinite out of reach. O(1).
    #[inline]
    pub(super) fn time_at(&self, target: &FootEnds) -> f32 {
        let Some(door) = self.door else {
            return f32::INFINITY;
        };
        let mut best = f32::INFINITY;
        for end in target.ends {
            if end.lane == usize::MAX {
                continue;
            }
            if let Some(cost) = self.label(end.lane) {
                let along = if self.reverse {
                    end.to_end_s
                } else {
                    end.from_start_s
                };
                best = best.min(cost + along + target.local_s);
            }
            for seed in door.ends {
                let along = if self.reverse {
                    seed.along_m - end.along_m
                } else {
                    end.along_m - seed.along_m
                };
                if seed.lane == end.lane && along >= 0.0 {
                    best =
                        best.min(door.local_s + along / AGENT_WALK_SPEED_MS + target.local_s);
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
