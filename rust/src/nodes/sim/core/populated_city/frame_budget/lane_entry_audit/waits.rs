// SPDX-License-Identifier: GPL-2.0-only

//! Wait diagnosis of the lane-entry audit: how long each held car has waited, what stands in its
//! way, and which lanes keep a stopped head car from one checkpoint to the next. Read-only
//! diagnostics over the agent columns; nothing here touches the simulation.
//!
//! A car is held when it has no lane and waits at a node for its first lane (`TRAFFIC-07`) or at
//! an attach point for a gap (`TRAFFIC-11`). Its blocker is the car whose slot conflicts with the
//! entry (the same static separation `lane_attach_slot_clear` tests). A blocked car on a lane has
//! the car ahead of it, or, at the lane end, the car on the entry of its next lane.

use super::{STACK_RADIUS_M, in_play};
use crate::config::{CAR_LENGTH, IDM_S_MIN};
use crate::nodes::sim::core::state::SimCore;
use crate::simulation::economy::agents::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_LANE_ATTACH_PENDING, ACCESS_PLAN_VALID, AgentVec,
    MODE_WALK, TRANSIT_NETWORK,
};
use crate::simulation::network::lanes::LaneType;
use std::collections::{BTreeMap, HashMap};

// Static separation of two cars, as `lane_attach_slot_clear` uses.
const SPACING: f32 = CAR_LENGTH + IDM_S_MIN;
// A car faster than this is moving; its queue will clear.
const MOVING_MS: f32 = 0.5;
// A stopped car this close behind another counts as queued behind it.
const QUEUE_REACH_M: f32 = 3.0 * SPACING;
// A car this close to its lane's end waits to leave it.
const AT_END_M: f32 = 1.0;
// Waits over this many game minutes get a blocker chain.
pub(super) const LONG_WAIT_MIN: u64 = 60;
// Longest chain followed for one car.
const MAX_CHAIN: usize = 20_000;

/// What a held car waits for. The key tells a new wait from the same one: the node, or the
/// attach lane.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HoldKind {
    Node,
    Attach,
}

struct Wait {
    kind: HoldKind,
    key: usize,
    since: u64,
    seen: u64,
}

/// Start minute of every held car's current wait, by runtime-stable car id.
#[derive(Default)]
pub(super) struct WaitBook {
    waits: HashMap<u64, Wait>,
    // Lane -> (head car id, distance, minute first seen stopped) at the previous checkpoint.
    heads: HashMap<usize, (u64, f32, u64)>,
    // Car id -> (lane, distance) at the previous checkpoint.
    positions: HashMap<u64, (usize, f32)>,
    // `TRAFFIC-14`: car ids followed frame by frame (METRUM_AUDIT_FOLLOW) and frames left.
    follow: Vec<u64>,
    follow_left: u32,
    follow_frames_printed: u32,
    // First car id of each followed car's next entry lanes in the previous traced frame.
    follow_first: HashMap<usize, u64>,
    // Per followed car: its lane and, per next entry, (slot clear, cars, first car id) in the
    // previous traced frame; and its contests (entries it saw clear and someone else took, with
    // that car's agent index against its own, and entries it won).
    follow_prev: HashMap<u64, (usize, Vec<EntryState>)>,
    follow_contests: BTreeMap<u64, Contests>,
}

// A next entry lane as a followed car saw it: lane, slot clear, cars, first car id.
type EntryState = (usize, bool, usize, u64);

#[derive(Default, serde::Serialize)]
struct Contests {
    agent: usize,
    lost: u32,
    lost_to_lower_index: u32,
    won: u32,
    clear_frames: u32,
}

fn hold(agents: &AgentVec, i: usize) -> Option<(HoldKind, usize)> {
    if agents.transit[i] != TRANSIT_NETWORK
        || agents.transit_mode[i] == MODE_WALK
        || agents.current_lane_id[i] != usize::MAX
    {
        return None;
    }
    if (agents.access_flags[i] & ACCESS_LANE_ATTACH_PENDING) != 0 {
        return Some((HoldKind::Attach, agents.planned_attach_lane_id[i] as usize));
    }
    (!agents.current_path[i].is_empty())
        .then_some((HoldKind::Node, agents.current_node[i] as usize))
}

impl WaitBook {
    /// Records the wait start of every car newly held, forgets cars that left their hold. O(agents).
    pub(super) fn update(&mut self, core: &SimCore, now: u64) {
        let agents = &core.agents.agents;
        for i in 0..agents.len() {
            let Some((kind, key)) = hold(agents, i) else {
                continue;
            };
            let wait = self.waits.entry(agents.render_id[i]).or_insert(Wait {
                kind,
                key,
                since: now,
                seen: now,
            });
            if wait.kind != kind || wait.key != key {
                *wait = Wait {
                    kind,
                    key,
                    since: now,
                    seen: now,
                };
            }
            wait.seen = now;
        }
        self.waits.retain(|_, wait| wait.seen == now);
    }
}

/// The lane occupancy the diagnosis reads, built from the agent columns: per lane the cars
/// sorted by distance, plus the agent index of each car id.
struct Occupancy<'a> {
    core: &'a SimCore,
    buckets: Vec<Vec<(f32, usize)>>,
    index_of: HashMap<u64, usize>,
}

impl<'a> Occupancy<'a> {
    fn new(core: &'a SimCore) -> Self {
        let agents = &core.agents.agents;
        let lane_count = core.transit_network.lane_system.lanes.len();
        let mut buckets = vec![Vec::new(); lane_count];
        let mut index_of = HashMap::new();
        for i in 0..agents.len() {
            index_of.insert(agents.render_id[i], i);
            if in_play(agents, i) && agents.current_lane_id[i] < lane_count {
                buckets[agents.current_lane_id[i]].push((agents.lane_distance[i], i));
            }
        }
        for bucket in &mut buckets {
            bucket.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        }
        Self {
            core,
            buckets,
            index_of,
        }
    }

    fn agents(&self) -> &AgentVec {
        &self.core.agents.agents
    }

    // The car whose slot conflicts with a car placed at `d` of `lane` (nearer one first).
    fn in_the_way(&self, lane: usize, d: f32) -> Option<usize> {
        let bucket = self.buckets.get(lane)?;
        let insert = bucket.partition_point(|entry| entry.0 < d);
        let behind = (insert > 0 && d - bucket[insert - 1].0 < SPACING).then(|| bucket[insert - 1]);
        let ahead =
            (insert < bucket.len() && bucket[insert].0 - d < SPACING).then(|| bucket[insert]);
        match (behind, ahead) {
            (Some(b), Some(a)) => Some(if d - b.0 <= a.0 - d { b.1 } else { a.1 }),
            (Some(b), None) => Some(b.1),
            (None, Some(a)) => Some(a.1),
            (None, None) => None,
        }
    }

    // First-lane candidates of a car at `node` bound for `next`: the vehicle lanes leaving the node
    // on the edge between them.
    fn first_lanes(&self, node: u32, next: u32) -> Vec<usize> {
        let core = self.core;
        let Some(edge) = core.region_graph.get_edge_between_nodes(node, next) else {
            return Vec::new();
        };
        let forward = core.region_graph.edge(edge).start_node == node;
        let lanes = &core.transit_network.lane_system;
        lanes.edge_lanes.get(&edge).map_or_else(Vec::new, |ids| {
            ids.iter()
                .copied()
                .filter(|&id| {
                    lanes.lanes[id].is_fwd == forward
                        && lanes.lanes[id].lane_type == LaneType::Vehicle
                })
                .collect()
        })
    }

    // The lanes whose entry a car at the end of `lane` needs: the direct vehicle lane or the
    // connectors that lead to the edge of its next path node.
    fn next_entries(&self, i: usize, lane: usize) -> Vec<usize> {
        let core = self.core;
        let agents = self.agents();
        let lanes = &core.transit_network.lane_system.lanes;
        let l = &lanes[lane];
        if l.edge_id == usize::MAX {
            return l.next_lanes.first().copied().into_iter().collect();
        }
        let edge = core.region_graph.edge(l.edge_id);
        let end = if l.is_fwd {
            edge.end_node
        } else {
            edge.start_node
        };
        let path = &agents.current_path[i];
        let idx = agents.current_path_index[i];
        // The path index sits on the lane's end node while the car is on its lane.
        let next = match path.get(idx) {
            Some(&node) if node == end => path.get(idx + 1).copied(),
            other => other.copied(),
        };
        let Some(next) = next else {
            // The route ends at this node: the car turns onto its planned detach lane.
            let detach = agents.planned_detach_lane_id[i] as usize;
            if detach >= lanes.len() || agents.planned_detach_node[i] != end {
                return Vec::new();
            }
            return l
                .next_lanes
                .iter()
                .copied()
                .filter(|&succ| {
                    succ == detach
                        || (lanes[succ].edge_id == usize::MAX
                            && lanes[succ].next_lanes.first() == Some(&detach))
                })
                .collect();
        };
        let Some(to_edge) = core.region_graph.get_edge_between_nodes(end, next) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for &succ in &l.next_lanes {
            let s = &lanes[succ];
            let direct = s.edge_id == to_edge && s.lane_type == LaneType::Vehicle;
            let connector = s.edge_id == usize::MAX
                && s.next_lanes
                    .first()
                    .is_some_and(|&target| lanes[target].edge_id == to_edge);
            if direct || connector {
                entries.push(succ);
            }
        }
        entries
    }

    // Why car `i` is not moving: the car in its way, or the end of the chain.
    fn step(&self, i: usize) -> Step {
        let core = self.core;
        let agents = self.agents();
        let lanes = &core.transit_network.lane_system.lanes;
        let lane = agents.current_lane_id[i];
        if lane == usize::MAX {
            return self.held_step(i);
        }
        if lane >= lanes.len() {
            return Step::Unexplained("lane id out of range".into());
        }
        let d = agents.lane_distance[i];
        // A car placed at its lane's end keeps its last speed while it waits to leave the lane.
        let at_end = d + AT_END_M >= lanes[lane].length;
        if agents.speed[i] > MOVING_MS && !at_end {
            return Step::Moving;
        }
        let bucket = &self.buckets[lane];
        let slot = bucket.partition_point(|entry| (entry.0, entry.1) < (d, i));
        if let Some(&(leader_d, leader)) = bucket.get(slot + 1)
            && leader_d - d < QUEUE_REACH_M
        {
            return Step::Blocked(
                leader,
                format!("queued behind on lane {lane} ({:.1} m ahead)", leader_d - d),
            );
        }
        if at_end {
            let candidates: Vec<usize> = self.next_entries(i, lane);
            if candidates.is_empty() {
                return Step::Unexplained(format!("at the end of lane {lane} with no next entry"));
            }
            return self.pick_blocker(&candidates, 0.0, "entry of lane");
        }
        Step::Unexplained("stopped mid-lane with no car ahead".into())
    }

    // The blocker of a held car: the car in the way of its attach slot or of every first lane.
    fn held_step(&self, i: usize) -> Step {
        let core = self.core;
        let agents = self.agents();
        let lanes = &core.transit_network.lane_system.lanes;
        if let Some((lane, d)) = agents.pending_lane_attach(i) {
            return match self.in_the_way(lane as usize, d) {
                Some(car) => Step::Blocked(car, format!("attach slot of lane {lane} at {d:.1} m")),
                None => {
                    Step::Unexplained(format!("attach slot of lane {lane} at {d:.1} m is clear"))
                }
            };
        }
        let node = agents.current_node[i];
        let candidates = match agents.current_path[i].get(agents.current_path_index[i]) {
            Some(&next) => self.first_lanes(node, next),
            None => {
                let lane = agents.planned_detach_lane_id[i] as usize;
                if lane < lanes.len() {
                    vec![lane]
                } else {
                    Vec::new()
                }
            }
        };
        if candidates.is_empty() {
            return Step::Unexplained(format!("no first lane at node {node}"));
        }
        self.pick_blocker(&candidates, 0.0, "first-lane entry of lane")
    }

    // A car waits when every candidate entry has a car in the way; follow the slowest one.
    fn pick_blocker(&self, candidates: &[usize], d: f32, what: &str) -> Step {
        let agents = self.agents();
        let mut best: Option<(usize, usize)> = None;
        for &lane in candidates {
            let Some(car) = self.in_the_way(lane, d) else {
                return Step::Unexplained(format!(
                    "{what} {lane} is clear now (claim lost or not yet processed)"
                ));
            };
            if best.is_none_or(|(_, b)| agents.speed[car] < agents.speed[b]) {
                best = Some((lane, car));
            }
        }
        let (lane, car) = best.expect("candidates is not empty");
        Step::Blocked(car, format!("{what} {lane}"))
    }

    // Where car `i` is, without its own state: the lane, or the node it is held at.
    fn place(&self, i: usize) -> String {
        let agents = self.agents();
        let lanes = &self.core.transit_network.lane_system.lanes;
        let lane = agents.current_lane_id[i];
        match lanes.get(lane) {
            Some(l) if l.edge_id == usize::MAX => {
                format!("lane {lane} (connector of node {})", l.node_id)
            }
            Some(l) => {
                let edge = self.core.region_graph.edge(l.edge_id);
                let end = if l.is_fwd {
                    edge.end_node
                } else {
                    edge.start_node
                };
                format!("lane {lane} (edge {} toward node {end})", l.edge_id)
            }
            None => match agents.pending_lane_attach(i) {
                Some((lane, d)) => format!("held at attach lane {lane} d {d:.1}"),
                None => format!("held at node {}", agents.current_node[i]),
            },
        }
    }

    // Everything that decides what a root car does next, for the report.
    fn root_state(&self, i: usize) -> serde_json::Value {
        let core = self.core;
        let agents = self.agents();
        let lanes = &core.transit_network.lane_system.lanes;
        let lane = agents.current_lane_id[i];
        let entries: Vec<serde_json::Value> = if lane < lanes.len() {
            self.next_entries(i, lane)
                .iter()
                .map(|&entry| {
                    let first = self.buckets[entry].first().copied();
                    serde_json::json!({
                        "lane": entry,
                        "first_car_d": first.map(|f| f.0),
                        "first_car_speed": first.map(|f| agents.speed[f.1]),
                        "cars": self.buckets[entry].len(),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let path = &agents.current_path[i];
        let idx = agents.current_path_index[i];
        serde_json::json!({
            "car": agents.render_id[i], "agent": i, "place": self.place(i),
            "d": agents.lane_distance[i], "speed": agents.speed[i],
            "length": lanes.get(lane).map(|l| l.length),
            "next_lanes": lanes.get(lane).map(|l| l.next_lanes.iter().take(8).copied().collect::<Vec<_>>()),
            "transit": agents.transit[i], "mode": agents.transit_mode[i],
            "access_flags": agents.access_flags[i],
            "border_destination": (agents.access_flags[i] & ACCESS_FREIGHT_BORDER_DESTINATION) != 0,
            "freight": agents.freight_shipment_id[i], "border_node": agents.freight_target_border_node[i],
            "path_len": path.len(), "path_index": idx,
            "path_window": path.iter().skip(idx.saturating_sub(1)).take(5).copied().collect::<Vec<_>>(),
            "current_node": agents.current_node[i], "current_edge": agents.current_edge[i],
            "target_building": agents.target_building[i], "home": agents.home_building[i],
            "detach_lane": agents.planned_detach_lane_id[i], "detach_d": agents.planned_detach_lane_d[i],
            "detach_node": agents.planned_detach_node[i],
            "next_replan_time": agents.next_replan_time[i], "sim_time": core.agents.sim_time,
            "entries": entries,
        })
    }

    // Where car `i` stands on its lane (`TRAFFIC-14` question 1): connector or road lane, at the
    // lane end, queued right behind a leader, or neither, with an active lane change noted.
    fn still_class(&self, i: usize) -> &'static str {
        let agents = self.agents();
        let lanes = &self.core.transit_network.lane_system.lanes;
        let lane = agents.current_lane_id[i];
        let Some(l) = lanes.get(lane) else {
            return "off_lane";
        };
        let d = agents.lane_distance[i];
        let bucket = &self.buckets[lane];
        let slot = bucket.partition_point(|entry| (entry.0, entry.1) < (d, i));
        let queued = bucket
            .get(slot + 1)
            .is_some_and(|&(leader_d, _)| leader_d - d < SPACING + 0.3);
        let changing = agents.lane_change_from_lane_id[i] != u32::MAX;
        match (l.edge_id == usize::MAX, d + AT_END_M >= l.length, queued) {
            (true, true, _) => "connector_end",
            (true, false, true) => "connector_queued",
            (true, false, false) => "connector_mid_free",
            (false, true, _) => "road_end",
            (false, false, true) if changing => "road_queued_lane_change",
            (false, false, true) => "road_queued",
            (false, false, false) if changing => "road_mid_lane_change",
            (false, false, false) => "road_mid_free",
        }
    }

    fn describe(&self, i: usize) -> String {
        let agents = self.agents();
        let truck = if agents.freight_shipment_id[i] != u64::MAX {
            " truck"
        } else {
            ""
        };
        let on_lane = self
            .core
            .transit_network
            .lane_system
            .lanes
            .get(agents.current_lane_id[i]);
        let at = on_lane.map_or(String::new(), |l| {
            format!(" d {:.1}/{:.1}", agents.lane_distance[i], l.length)
        });
        format!(
            "car {} {}{at} v {:.2}{truck}",
            agents.render_id[i],
            self.place(i),
            agents.speed[i]
        )
    }
}

enum Step {
    Moving,
    Blocked(usize, String),
    Unexplained(String),
}

/// Where a blocker chain ends.
#[derive(Clone)]
enum Outcome {
    /// Reaches a moving car (agent index).
    Moving(usize),
    /// Reaches a stopped car with nothing in its way (agent index, why).
    Unexplained(usize, String),
    /// Closes a wait-for cycle (agent indices in order).
    Cycle(Vec<usize>),
}

impl Outcome {
    fn class(&self) -> &'static str {
        match self {
            Self::Moving(_) => "moving",
            Self::Unexplained(..) => "unexplained",
            Self::Cycle(_) => "cycle",
        }
    }
}

// Follows the chain from `start` to its end, memoised in `memo` for every car on it.
fn resolve(
    occ: &Occupancy<'_>,
    steps: &mut HashMap<usize, Step>,
    memo: &mut HashMap<usize, Outcome>,
    start: usize,
) -> Outcome {
    let mut path: Vec<usize> = Vec::new();
    let mut position: HashMap<usize, usize> = HashMap::new();
    let mut cur = start;
    let outcome = loop {
        if let Some(done) = memo.get(&cur) {
            break done.clone();
        }
        if let Some(&pos) = position.get(&cur) {
            break Outcome::Cycle(path[pos..].to_vec());
        }
        if path.len() >= MAX_CHAIN {
            break Outcome::Unexplained(cur, "chain longer than the follow limit".into());
        }
        position.insert(cur, path.len());
        path.push(cur);
        let step = steps.entry(cur).or_insert_with(|| occ.step(cur));
        match step {
            Step::Moving => break Outcome::Moving(cur),
            Step::Unexplained(why) => break Outcome::Unexplained(cur, why.clone()),
            Step::Blocked(by, _) => cur = *by,
        }
    };
    for car in path {
        memo.entry(car).or_insert_with(|| outcome.clone());
    }
    outcome
}

fn cycle_summary(occ: &Occupancy<'_>, cycle: &[usize]) -> Vec<String> {
    cycle.iter().map(|&car| occ.describe(car)).collect()
}

impl WaitBook {
    /// Counts of waits over one game hour and the longest wait, for every checkpoint.
    pub(super) fn summary(&self, now: u64) -> serde_json::Value {
        let long = self
            .waits
            .values()
            .filter(|wait| now - wait.since > LONG_WAIT_MIN)
            .count();
        let long_4h = self
            .waits
            .values()
            .filter(|wait| now - wait.since > 240)
            .count();
        let longest = self
            .waits
            .values()
            .map(|wait| now - wait.since)
            .max()
            .unwrap_or(0);
        let attach = self
            .waits
            .values()
            .filter(|wait| wait.kind == HoldKind::Attach)
            .count();
        serde_json::json!({
            "held": self.waits.len(), "held_attach": attach,
            "waits_over_1h": long, "waits_over_4h": long_4h, "longest_wait_min": longest,
        })
    }

    /// Prints the 20 longest waits with their blocker chains and, for every wait over one game
    /// hour, the class of the chain's end (moving car, unexplained stop, cycle) with the most
    /// common roots. O(held cars + chain lengths), memoised.
    pub(super) fn report(&self, core: &SimCore, now: u64, minute: u64) {
        let occ = Occupancy::new(core);
        let mut steps = HashMap::new();
        let mut memo = HashMap::new();
        let mut waits: Vec<(u64, u64, &Wait)> = self
            .waits
            .iter()
            .map(|(&id, wait)| (now - wait.since, id, wait))
            .collect();
        waits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut root_cars = HashMap::<usize, u32>::new();
        let mut node_waits = HashMap::<usize, Vec<(u64, usize)>>::new();
        let mut classes = HashMap::<&str, u32>::new();
        let mut roots = HashMap::<String, u32>::new();
        let mut cycles = Vec::new();
        for (rank, &(waited, id, wait)) in waits.iter().enumerate() {
            let Some(&agent) = occ.index_of.get(&id) else {
                continue;
            };
            let long = waited > LONG_WAIT_MIN;
            if wait.kind == HoldKind::Node {
                node_waits
                    .entry(wait.key)
                    .or_default()
                    .push((waited, agent));
            }
            if !long && rank >= 20 {
                continue;
            }
            let outcome = resolve(&occ, &mut steps, &mut memo, agent);
            if long {
                *classes.entry(outcome.class()).or_default() += 1;
                let root = match &outcome {
                    Outcome::Moving(car) => format!("moving: {}", occ.place(*car)),
                    Outcome::Unexplained(car, why) => {
                        format!("unexplained: {} ({why})", occ.place(*car))
                    }
                    Outcome::Cycle(cycle) => {
                        let mut lanes: Vec<usize> = cycle
                            .iter()
                            .map(|&car| occ.agents().current_lane_id[car])
                            .collect();
                        lanes.sort_unstable();
                        format!("cycle through lanes {lanes:?}")
                    }
                };
                if let Outcome::Unexplained(car, _) = &outcome {
                    *root_cars.entry(*car).or_default() += 1;
                }
                *roots.entry(root).or_default() += 1;
                if let Outcome::Cycle(cycle) = &outcome {
                    cycles.push(cycle.clone());
                }
            }
            if rank < 20 {
                // Re-walk the chain for the printout; a run of cars queued on one lane prints as its
                // first and last car.
                let mut chain = vec![occ.describe(agent)];
                let mut cur = agent;
                let mut run = 0;
                let mut seen = std::collections::HashSet::new();
                for _ in 0..MAX_CHAIN {
                    let Some(Step::Blocked(by, via)) = steps.get(&cur) else {
                        break;
                    };
                    if !seen.insert(cur) {
                        chain.push("(joins a cycle)".into());
                        break;
                    }
                    let next_queued = matches!(steps.get(by), Some(Step::Blocked(_, v)) if v.starts_with("queued"));
                    if via.starts_with("queued") && next_queued {
                        run += 1;
                    } else {
                        if run > 0 {
                            chain.push(format!("... {run} more queued cars"));
                            run = 0;
                        }
                        chain.push(format!("blocked by [{via}] {}", occ.describe(*by)));
                    }
                    cur = *by;
                    if cur == agent {
                        chain.push("(back at the first car)".into());
                        break;
                    }
                }
                if let Some(end) = steps.get(&cur) {
                    match end {
                        Step::Moving => chain.push("ends: moving".into()),
                        Step::Unexplained(why) => chain.push(format!("ends: unexplained, {why}")),
                        Step::Blocked(..) => {}
                    }
                }
                if chain.len() > 30 {
                    let skipped = chain.len() - 28;
                    let tail = chain.split_off(chain.len() - 14);
                    chain.truncate(14);
                    chain.push(format!("... {skipped} more steps"));
                    chain.extend(tail);
                }
                eprintln!(
                    "TRAFFIC07_WAIT {}",
                    serde_json::json!({
                        "minute": minute, "rank": rank + 1, "wait_min": waited,
                        "kind": match wait.kind { HoldKind::Node => "node", HoldKind::Attach => "attach" },
                        "key": wait.key, "since_minute": wait.since,
                        "plan_valid": (occ.agents().access_flags[agent] & ACCESS_PLAN_VALID) != 0,
                        "end": outcome.class(),
                        "chain": chain,
                    })
                );
            }
        }
        let mut root_cars: Vec<(usize, u32)> = root_cars.into_iter().collect();
        root_cars.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for &(car, count) in root_cars.iter().take(6) {
            eprintln!(
                "TRAFFIC07_ROOT {}",
                serde_json::json!({
                    "minute": minute, "waits_behind": count, "state": occ.root_state(car),
                })
            );
        }
        let mut nodes: Vec<(usize, Vec<(u64, usize)>)> = node_waits.into_iter().collect();
        nodes.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
        for (node, cars) in nodes.iter().take(8) {
            let mean = |list: &[usize]| {
                if list.is_empty() {
                    -1.0
                } else {
                    list.iter().sum::<usize>() as f64 / list.len() as f64
                }
            };
            let long_day: Vec<usize> = cars.iter().filter(|c| c.0 > 1_440).map(|c| c.1).collect();
            let fresh: Vec<usize> = cars.iter().filter(|c| c.0 <= 60).map(|c| c.1).collect();
            let all: Vec<usize> = cars.iter().map(|c| c.1).collect();
            eprintln!(
                "TRAFFIC07_NODE {}",
                serde_json::json!({
                    "minute": minute, "node": node, "held": cars.len(),
                    "over_24h": long_day.len(), "mean_index_over_24h": mean(&long_day),
                    "under_1h": fresh.len(), "mean_index_under_1h": mean(&fresh),
                    "min_index": all.iter().min(), "max_index": all.iter().max(),
                    "mean_index": mean(&all),
                })
            );
        }
        let mut top: Vec<(String, u32)> = roots.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        top.truncate(12);
        let mut cycles_seen = Vec::new();
        for cycle in cycles {
            let mut key = cycle.clone();
            key.sort_unstable();
            if !cycles_seen.contains(&key) {
                cycles_seen.push(key);
                eprintln!(
                    "TRAFFIC07_CYCLE {}",
                    serde_json::json!({"minute": minute, "cars": cycle_summary(&occ, &cycle)})
                );
            }
        }
        eprintln!(
            "TRAFFIC07_WAIT_SUMMARY {}",
            serde_json::json!({
                "minute": minute, "waits_over_1h": classes.values().sum::<u32>(),
                "ends": classes, "top_roots": top,
            })
        );
    }

    /// Lists the lanes whose head car stayed stopped at the same place since the previous
    /// checkpoint, with the head's blocker, and remembers this checkpoint's stopped heads.
    pub(super) fn standing_queues(&mut self, core: &SimCore, minute: u64) {
        let occ = Occupancy::new(core);
        let agents = occ.agents();
        let lanes = &core.transit_network.lane_system.lanes;
        let mut heads = HashMap::new();
        // Cars that report a speed but sit exactly where they sat at the previous checkpoint.
        let mut positions = HashMap::new();
        let mut unmoved_with_speed = 0_u32;
        let mut unmoved_total = 0_u32;
        // `TRAFFIC-14` question 1: where the unmoved cars that report a speed stand.
        let mut speed_classes = std::collections::BTreeMap::<&str, u32>::new();
        let mut speed_samples = std::collections::BTreeMap::<&str, Vec<u64>>::new();
        for (lane, bucket) in occ.buckets.iter().enumerate() {
            for &(d, car) in bucket {
                let id = agents.render_id[car];
                if let Some(&(prev_lane, prev_d)) = self.positions.get(&id)
                    && prev_lane == lane
                    && (prev_d - d).abs() < 0.1
                {
                    unmoved_total += 1;
                    if agents.speed[car] > MOVING_MS {
                        unmoved_with_speed += 1;
                        let class = occ.still_class(car);
                        *speed_classes.entry(class).or_default() += 1;
                        let samples = speed_samples.entry(class).or_default();
                        if samples.len() < 2 {
                            samples.push(id);
                        }
                    }
                }
                positions.insert(id, (lane, d));
            }
        }
        let mut standing = Vec::new();
        let mut steps = HashMap::new();
        let mut memo = HashMap::new();
        for (lane, bucket) in occ.buckets.iter().enumerate() {
            let Some(&(d, head)) = bucket.last() else {
                continue;
            };
            let id = agents.render_id[head];
            let first = match self.heads.get(&lane) {
                Some(&(prev_id, prev_d, first))
                    if prev_id == id && (prev_d - d).abs() < STACK_RADIUS_M =>
                {
                    standing.push((lane, head, first));
                    first
                }
                _ => minute,
            };
            heads.insert(lane, (id, d, first));
        }
        standing.sort_by_key(|&(lane, _, first)| (first, lane));
        // Group by where the chain ends; list a few lanes of each group and every cycle.
        let mut groups = std::collections::BTreeMap::<String, (u32, Vec<serde_json::Value>)>::new();
        let mut root_cars = HashMap::<usize, u32>::new();
        // Standing heads whose chain ends at a moving car, by that car's place and whether it
        // has moved since the previous checkpoint.
        let mut moving_roots = std::collections::BTreeMap::<String, u32>::new();
        let mut moving_heads = Vec::new();
        // Up to 24 more standing heads, oldest first, for the contest count.
        let long_standing: Vec<u64> = standing
            .iter()
            .take(24)
            .map(|&(_, head, _)| agents.render_id[head])
            .collect();
        for &(lane, head, first) in &standing {
            let outcome = resolve(&occ, &mut steps, &mut memo, head);
            let root = match &outcome {
                Outcome::Moving(car) => {
                    *root_cars.entry(*car).or_default() += 1;
                    let moved = !self
                        .positions
                        .get(&agents.render_id[*car])
                        .is_some_and(|&(lane, d)| {
                            lane == agents.current_lane_id[*car]
                                && (d - agents.lane_distance[*car]).abs() < 0.1
                        });
                    let key = format!("{} moved={moved}", occ.still_class(*car));
                    *moving_roots.entry(key).or_default() += 1;
                    if moving_heads.len() < 2 {
                        moving_heads.push(agents.render_id[head]);
                    }
                    format!("moving: {}", occ.place(*car))
                }
                Outcome::Unexplained(car, why) => {
                    *root_cars.entry(*car).or_default() += 1;
                    format!("unexplained: {} ({why})", occ.place(*car))
                }
                Outcome::Cycle(cycle) => format!("cycle of {} cars", cycle.len()),
            };
            let l = &lanes[lane];
            let entry = groups.entry(root).or_default();
            entry.0 += 1;
            if entry.1.len() < 3 {
                entry.1.push(serde_json::json!({
                    "lane": lane, "node": if l.edge_id == usize::MAX { l.node_id as i64 } else { -1 },
                    "edge": l.edge_id as i64, "cars": occ.buckets[lane].len(), "length": l.length,
                    "head": occ.describe(head), "stopped_since_minute": first,
                }));
            }
        }
        let mut root_cars: Vec<(usize, u32)> = root_cars.into_iter().collect();
        root_cars.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for &(car, count) in root_cars.iter().take(8) {
            eprintln!(
                "TRAFFIC07_STANDING_ROOT {}",
                serde_json::json!({
                    "minute": minute, "standing_heads_behind": count, "state": occ.root_state(car),
                    "unmoved_since_last_checkpoint": self
                        .positions
                        .get(&agents.render_id[car])
                        .is_some_and(|&(lane, d)| {
                            lane == agents.current_lane_id[car]
                                && (d - agents.lane_distance[car]).abs() < 0.1
                        }),
                })
            );
        }
        let mut rows: Vec<(String, u32, Vec<serde_json::Value>)> = groups
            .into_iter()
            .map(|(root, (n, ex))| (root, n, ex))
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        rows.truncate(15);
        eprintln!(
            "TRAFFIC07_STANDING {}",
            serde_json::json!({
                "minute": minute, "heads": heads.len(), "standing": standing.len(),
                "oldest_since_minute": standing.first().map_or(minute, |s| s.2),
                "unmoved_since_last_checkpoint": unmoved_total,
                "unmoved_but_speed_over_0_5": unmoved_with_speed, "groups": rows,
                "unmoved_speed_classes": speed_classes, "moving_root_classes": moving_roots,
            })
        );
        // METRUM_AUDIT_FOLLOW=<frames>: from the first checkpoint that has a previous one, follow
        // two cars of every unmoved-with-speed class and two standing heads chained to a moving car.
        if self.follow.is_empty()
            && !self.positions.is_empty()
            && let Ok(frames) = std::env::var("METRUM_AUDIT_FOLLOW")
        {
            self.follow = speed_samples.into_values().flatten().collect();
            self.follow.extend(moving_heads);
            self.follow.extend(long_standing);
            self.follow_left = frames.parse().expect("METRUM_AUDIT_FOLLOW must be an integer");
        }
        self.heads = heads;
        self.positions = positions;
    }

    /// Prints the followed cars' state for this frame while frames are left: lane, distance,
    /// speed, transit, claim mode, the car ahead and the first car of each next entry lane (a
    /// change of that car is an entry by someone else). O(agents) per frame while following.
    pub(super) fn trace_follow(&mut self, core: &SimCore, frame: u64) {
        if self.follow_left == 0 {
            return;
        }
        self.follow_left -= 1;
        let occ = Occupancy::new(core);
        let agents = occ.agents();
        let lanes = &core.transit_network.lane_system.lanes;
        // Per-frame lines for the first ten frames; contests for every frame.
        let print = self.follow_frames_printed < 10;
        self.follow_frames_printed += 1;
        for &id in &self.follow {
            let Some(&i) = occ.index_of.get(&id) else {
                continue;
            };
            let lane = agents.current_lane_id[i];
            let d = agents.lane_distance[i];
            let contests = self.follow_contests.entry(id).or_default();
            contests.agent = i;
            if let Some((prev_lane, prev_entries)) = self.follow_prev.get(&id) {
                for &(entry, was_clear, cars, first) in prev_entries {
                    if lane == entry {
                        contests.won += 1;
                    } else if was_clear && lane == *prev_lane {
                        contests.clear_frames += 1;
                        let now_first = occ.buckets[entry].first().map(|f| f.1);
                        if occ.buckets[entry].len() > cars
                            && let Some(other) = now_first
                            && agents.render_id[other] != first
                        {
                            contests.lost += 1;
                            contests.lost_to_lower_index += u32::from(other < i);
                        }
                    }
                }
            }
            let now_entries: Vec<EntryState> = if lane < lanes.len() {
                occ.next_entries(i, lane)
                    .into_iter()
                    .map(|entry| {
                        let bucket = &occ.buckets[entry];
                        (
                            entry,
                            occ.in_the_way(entry, 0.0).is_none(),
                            bucket.len(),
                            bucket.first().map_or(u64::MAX, |f| agents.render_id[f.1]),
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
            self.follow_prev.insert(id, (lane, now_entries));
            if !print {
                continue;
            }
            let (ahead, entries) = if lane < lanes.len() {
                let bucket = &occ.buckets[lane];
                let slot = bucket.partition_point(|entry| (entry.0, entry.1) < (d, i));
                let ahead = bucket.get(slot + 1).map(|&(ahead_d, car)| {
                    serde_json::json!({"car": agents.render_id[car], "d": ahead_d, "v": agents.speed[car]})
                });
                let entries: Vec<serde_json::Value> = occ
                    .next_entries(i, lane)
                    .into_iter()
                    .map(|entry| {
                        let first = occ.buckets[entry].first().copied();
                        let first_id = first.map(|f| agents.render_id[f.1]);
                        let previous = self.follow_first.insert(entry, first_id.unwrap_or(u64::MAX));
                        serde_json::json!({
                            "lane": entry, "cars": occ.buckets[entry].len(),
                            "first_car": first_id, "first_d": first.map(|f| f.0),
                            "first_v": first.map(|f| agents.speed[f.1]),
                            "entered": previous.is_some_and(|p| Some(p) != first_id),
                            "slot_clear": occ.in_the_way(entry, 0.0).is_none(),
                        })
                    })
                    .collect();
                (ahead, entries)
            } else {
                (None, Vec::new())
            };
            eprintln!(
                "TRAFFIC14_FOLLOW {}",
                serde_json::json!({
                    "frame": frame, "car": id, "agent": i, "class": occ.still_class(i),
                    "place": occ.place(i), "d": d, "speed": agents.speed[i],
                    "transit": agents.transit[i],
                    "claim_mode": core.agents.movement_claim_modes.get(i).map(|&m| m as u8),
                    "lane_change_from": agents.lane_change_from_lane_id[i],
                    "path_index": agents.current_path_index[i], "path_len": agents.current_path[i].len(),
                    "ahead": ahead, "entries": entries,
                })
            );
        }
        if self.follow_left == 0 {
            eprintln!(
                "TRAFFIC14_CONTESTS {}",
                serde_json::json!({"frames": self.follow_frames_printed, "cars": &self.follow_contests})
            );
        }
    }
}

/// One wait-for cycle and what option (a) of the round R14A brief could do with it: the cycle's
/// cars in chain order; how many of them head a road lane at its end (a car already on a
/// connector is committed to its turn); how many of those have another way out of their
/// junction now, a connector whose entry and exit-lane entry are clear; and those ways.
pub(super) struct CycleInfo {
    pub(super) cars: Vec<usize>,
    pub(super) road_heads: u32,
    pub(super) heads_with_room: u32,
    pub(super) ways_out: u32,
}

/// Every wait-for cycle among the cars now on lanes, found by following the blocker chain of
/// every lane's head car. O(cars on lanes), memoised; a diagnostic for `TRAFFIC-14`.
pub(super) fn wait_cycles(core: &SimCore) -> Vec<CycleInfo> {
    let occ = Occupancy::new(core);
    let mut steps = HashMap::new();
    let mut memo = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    let mut cycles = Vec::new();
    for bucket in &occ.buckets {
        let Some(&(_, head)) = bucket.last() else {
            continue;
        };
        if let Outcome::Cycle(cycle) = resolve(&occ, &mut steps, &mut memo, head) {
            let mut key = cycle.clone();
            key.sort_unstable();
            if seen.insert(key) {
                let (mut road_heads, mut heads_with_room, mut ways_out) = (0, 0, 0);
                for &car in &cycle {
                    if let Some(ways) = occ.ways_out(car) {
                        road_heads += 1;
                        heads_with_room += u32::from(ways > 0);
                        ways_out += ways;
                    }
                }
                cycles.push(CycleInfo {
                    cars: cycle,
                    road_heads,
                    heads_with_room,
                    ways_out,
                });
            }
        }
    }
    cycles
}

impl Occupancy<'_> {
    // For a car heading a road lane at its end, the connectors (or direct lanes) of that lane
    // other than its planned ones whose entry, and whose exit lane's entry, are clear now; None
    // for any other car.
    fn ways_out(&self, i: usize) -> Option<u32> {
        let agents = self.agents();
        let lanes = &self.core.transit_network.lane_system.lanes;
        let lane = agents.current_lane_id[i];
        let l = lanes.get(lane)?;
        if l.edge_id == usize::MAX || agents.lane_distance[i] + AT_END_M < l.length {
            return None;
        }
        let planned = self.next_entries(i, lane);
        let mut ways = 0;
        for &succ in &l.next_lanes {
            let s = &lanes[succ];
            if planned.contains(&succ) || s.lane_type != LaneType::Vehicle {
                continue;
            }
            let exit = if s.edge_id == usize::MAX {
                s.next_lanes.first().copied()
            } else {
                Some(succ)
            };
            let Some(exit) = exit else {
                continue;
            };
            let clear = self.in_the_way(succ, 0.0).is_none()
                && (exit == succ || self.in_the_way(exit, 0.0).is_none());
            ways += u32::from(clear);
        }
        Some(ways)
    }
}
