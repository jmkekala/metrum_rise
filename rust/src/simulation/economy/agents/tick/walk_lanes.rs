// SPDX-License-Identifier: GPL-2.0-only

//! Walk lane witnesses (`ROAD-74`): the sidewalk a walker takes on each road of its node path.
//!
//! A walking route is a node path, and its legality is existential: some chain of foot lanes
//! and junction connectors walks it (`path_has_connectors_from`). With crossings off, a junction
//! joins only some sidewalks to some arms, so a walker that took any connector onto the next road
//! could stand on a sidewalk from which the rest of its route has no connector. The witness fixes
//! the chain once per route: the cheapest chain of foot lanes along the node path into the
//! planned exit lane, by connector length (both sidewalks of a road cost the same to walk), ties
//! within 1 mm broken by the agent's schedule seed. The choice at a node depends only on the lane
//! the walker arrives on and the rest of its route, so a witness built from any later lane of
//! the chain repeats it: a loaded save, which rebuilds it, continues the same walk.

use super::super::determinism::stable_index;
use super::lane_nav::lane_origin_node;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;
use std::cell::RefCell;

/// Costs within this many metres of the cheapest are a tie.
const TIE_M: f32 = 0.001;
/// A road the walker had already passed when its witness was built mid-route.
pub(super) const PASSED_HOP: i8 = i8::MIN;
/// Every road of a route that no chain walks from where the witness was built.
pub(super) const NO_WITNESS: i8 = i8::MIN + 1;

/// Where a witness starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WalkFrom {
    /// The foot lane the walker arrives on at the first node.
    Lane(usize),
    /// No lane yet: the walker enters the first road's sidewalk at the first node.
    Node,
}

#[derive(Default)]
struct Scratch {
    // Road `k`'s candidate lanes are `lanes[offsets[k]..offsets[k + 1]]`: (lane, cost to go).
    offsets: Vec<usize>,
    lanes: Vec<(usize, f32)>,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

/// Fills `out` with one `Lane::lane_idx` per road of `path` (`out[k]` for the road from
/// `path[k]` to `path[k + 1]`), the first `first_hop` entries [`PASSED_HOP`]. The chain starts
/// from `from` at `path[first_hop]` and ends in the exit lane `detach` (`usize::MAX` for none).
/// Returns `false`, with every entry [`NO_WITNESS`], when no chain walks the rest of the path.
///
/// O(roads × sidewalks per road × connectors per lane), with no allocation once the thread's
/// scratch and `out` have grown to the longest route.
#[allow(clippy::too_many_arguments)]
pub(super) fn fill_walk_lanes(
    path: &[u32],
    first_hop: usize,
    from: WalkFrom,
    detach: usize,
    seed: u32,
    network: &TransitNetwork,
    graph: &RegionGraph,
    out: &mut Vec<i8>,
) -> bool {
    out.clear();
    out.resize(first_hop, PASSED_HOP);
    let rest = path.get(first_hop..).unwrap_or_default();
    let found = rest.len() >= 2
        && SCRATCH.with_borrow_mut(|scratch| {
            scratch.collect(rest, network, graph)
                && scratch.choose(rest, from, detach, seed, network, out)
        });
    if !found {
        out.clear();
        out.resize(path.len().saturating_sub(1), NO_WITNESS);
    }
    found
}

impl Scratch {
    // Lists each road's foot lanes leaving its first node, in `lane_idx` order. False when a
    // node pair has no road or the road no sidewalk in that direction.
    fn collect(&mut self, path: &[u32], network: &TransitNetwork, graph: &RegionGraph) -> bool {
        let lanes = &network.lane_system.lanes;
        self.offsets.clear();
        self.lanes.clear();
        self.offsets.push(0);
        for pair in path.windows(2) {
            let Some(edge) = graph.get_edge_between_nodes(pair[0], pair[1]) else {
                return false;
            };
            let start = self.lanes.len();
            for &lane in network
                .lane_system
                .edge_lanes
                .get(&edge)
                .into_iter()
                .flatten()
            {
                if lanes[lane].lane_type == LaneType::Foot
                    && lane_origin_node(lane, network, graph) == Some(pair[0])
                {
                    self.lanes.push((lane, f32::INFINITY));
                }
            }
            if self.lanes.len() == start {
                return false;
            }
            self.lanes[start..].sort_unstable_by_key(|&(lane, _)| lanes[lane].lane_idx);
            self.offsets.push(self.lanes.len());
        }
        true
    }

    // Backward pass, then the forward choice. Each lane's cost to go is its cheapest connector
    // length to the exit lane along the rest of the path, infinite where no chain finishes it;
    // each step takes the cheapest connector plus cost to go from the lane it stands on.
    fn choose(
        &mut self,
        path: &[u32],
        from: WalkFrom,
        detach: usize,
        seed: u32,
        network: &TransitNetwork,
        out: &mut Vec<i8>,
    ) -> bool {
        let roads = self.offsets.len() - 1;
        for entry in &mut self.lanes[self.offsets[roads - 1]..] {
            entry.1 = if detach == usize::MAX || entry.0 == detach {
                0.0
            } else {
                connector_length(entry.0, detach, network).unwrap_or(f32::INFINITY)
            };
        }
        for k in (0..roads - 1).rev() {
            let (here, next) = self.lanes.split_at_mut(self.offsets[k + 1]);
            let next = &next[..self.offsets[k + 2] - self.offsets[k + 1]];
            for entry in &mut here[self.offsets[k]..] {
                let from = entry.0;
                entry.1 = cheapest(next, |to| connector_length(from, to, network))
                    .unwrap_or(f32::INFINITY);
            }
        }
        let lanes = &network.lane_system.lanes;
        let mut at = from;
        for k in 0..roads {
            let road = &self.lanes[self.offsets[k]..self.offsets[k + 1]];
            let step = |to: usize| match at {
                WalkFrom::Node => Some(0.0),
                WalkFrom::Lane(lane) => connector_length(lane, to, network),
            };
            let Some(best) = cheapest(road, &step) else {
                return false;
            };
            let ties = |&&(to, cost): &&(usize, f32)| {
                step(to).is_some_and(|len| len + cost <= best + TIE_M)
            };
            let count = road.iter().filter(ties).count();
            let pick = stable_index(tie_seed(seed, path[k], path[k + 1]), count);
            let &(lane, _) = road.iter().filter(ties).nth(pick).expect("pick < ties");
            out.push(lanes[lane].lane_idx);
            at = WalkFrom::Lane(lane);
        }
        true
    }
}

// The cheapest finite `step(lane) + cost to go` over `road`'s lanes.
fn cheapest(road: &[(usize, f32)], step: impl Fn(usize) -> Option<f32>) -> Option<f32> {
    road.iter()
        .filter_map(|&(lane, cost)| step(lane).map(|len| len + cost))
        .filter(|cost| cost.is_finite())
        .min_by(f32::total_cmp)
}

// The shortest foot connector from road lane `from` into road lane `to`. O(connectors of `from`).
fn connector_length(from: usize, to: usize, network: &TransitNetwork) -> Option<f32> {
    let lanes = &network.lane_system.lanes;
    lanes
        .get(from)?
        .next_lanes
        .iter()
        .filter_map(|&conn| {
            let conn = lanes.get(conn)?;
            (conn.edge_id == usize::MAX
                && conn.lane_type == LaneType::Foot
                && conn.next_lanes.first().copied() == Some(to))
            .then_some(conn.length)
        })
        .min_by(f32::total_cmp)
}

#[inline(always)]
fn tie_seed(seed: u32, node: u32, next: u32) -> u64 {
    u64::from(seed) ^ (u64::from(node) << 21) ^ (u64::from(next) << 42)
}
