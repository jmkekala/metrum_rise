// SPDX-License-Identifier: GPL-2.0-only

//! What a ring carries (`ROAD-62`, round R7c2): a saturating fixture with no city. Every entry
//! lane has a source that places a car at its start whenever the lane's entry slot is clear, so
//! a queue stands at every entry; a sink takes each car off near the end of its exit arm and
//! counts it there. Flows are cars per simulated minute after a warm-up, with a per-lane trace
//! of the junction's parts. Isolated subsystem setup: no routing, buildings or economy.
//!
//! `measure_capacity` (ignored) prints the table in `docs/traffic.md`:
//! `$TEST_BINARY simulation::economy::agents::tick::ring_capacity_tests::measure_capacity
//! --ignored --nocapture`, optionally `METRUM_CAPACITY_ONLY=<name part>` and
//! `METRUM_CAPACITY_TRACE=1` for the per-lane trace.

use super::super::TRANSIT_IMMIGRATING;
use super::super::data::AgentSystem;
use super::traffic::{connector_turn_speed, lane_attach_slot_clear, ring_entry_blocker};
use crate::config::{CAR_LENGTH, DEFAULT_URBAN_ROAD_SPEED_MS, IDM_S_MIN, IDM_T_HEAD};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::roadbed::fixtures::{
    connect, converted_ring, divided, finalize, hub_graph, point,
};
use crate::simulation::network::roundabouts::{
    RoundaboutApplied, RoundaboutLayout, RoundaboutRequest, RoundaboutSite,
};
use crate::simulation::network::types::NodeType;
use godot::prelude::Vector2;

// The agent step at game speed 1 and at the 32x of the day runs (`FRAME_DT_S` times the speed).
const DT_1X: f32 = 1.0 / 60.0;
const DT_32X: f32 = 32.0 / 60.0;
// The road control's length, and where its sink stands from the lane's end.
const ROAD_M: f32 = 900.0;
const SINK_BEFORE_END_M: f32 = 20.0;
const ANGLES: [f32; 4] = [0.0, 90.0, 180.0, 270.0];
// Starts per case; the table gives their median and range.
const PHASES: u32 = 9;

#[derive(Clone, Copy, Debug)]
enum Layout {
    Road { kmh: f32, lanes: u8 },
    Junction { lanes: u8 },
    Ring { diameter_m: f32, lanes: u8 },
    // A two-lane ring with arms 0 and 2 of 2+2 and arms 1 and 3 of 1+1, which fan onto both lanes.
    FanRing { diameter_m: f32 },
}

// Who drives where. Turns are counted in the ring's sense of travel: 1 = the first exit (a right
// turn), 2 = straight over, 3 = the third exit (a left turn).
#[derive(Clone, Copy, Debug)]
enum Demand {
    // Every arm to the arm `n` exits on.
    All(usize),
    // Only arm 0, to the arm `n` exits on.
    One(usize),
    // Every arm, cars in turn to the first, second and third exit (a two-lane arm's outer lane
    // first and second, its inner lane third and second: each car starts in the lane its turn
    // needs, so a queue needs no lane changes).
    Mixed,
    // A day's movements at a junction, `[arm][exit - 1]` cars, offered at `.1` cars a minute in
    // all: each arm's share at a steady rate, its exits in proportion.
    Offered(&'static Movements, f32),
}

type Movements = [[u32; 3]; 4];
// The trunk junction's movements in a ring-free day of the populated 10k city, arm 0 the border
// trunk, the rest in the ring's sense of travel (`METRUM_AUDIT_JUNCTION`, 32x).
const CITY_1_1: Movements = [[692, 3, 395], [15, 240, 727], [16, 3, 18], [424, 305, 15]];
const CITY_4_LANE: Movements = [[971, 0, 684], [6, 218, 888], [10, 6, 10], [733, 219, 6]];

struct Arm {
    far: u32,
    near: u32,
    entries: Vec<usize>,
    exits: Vec<usize>,
}

struct Site {
    graph: RegionGraph,
    network: TransitNetwork,
    arms: Vec<Arm>,
    // Ring nodes in the sense of travel from arm 0 on, one per arm; empty off a ring.
    ring_nodes: Vec<u32>,
    // Arm index after each arm in the sense of travel.
    next_arm: Vec<usize>,
}

// Arm `edge`'s vehicle lanes from `near`: toward the junction, then away, each from the outermost.
fn arm_lanes(
    graph: &RegionGraph,
    network: &TransitNetwork,
    edge: usize,
    near: u32,
) -> (Vec<usize>, Vec<usize>) {
    let inward = graph.edge(edge).end_node == near;
    let lanes = &network.lane_system.lanes;
    let mut own: Vec<usize> = network.lane_system.edge_lanes[&edge]
        .iter()
        .copied()
        .filter(|&lane| lanes[lane].lane_type == LaneType::Vehicle)
        .collect();
    own.sort_unstable_by_key(|&lane| std::cmp::Reverse(lanes[lane].lane_idx.abs()));
    own.iter().partition(|&&lane| lanes[lane].is_fwd == inward)
}

fn arm(graph: &RegionGraph, network: &TransitNetwork, edge: usize, near: u32) -> Arm {
    let data = graph.edge(edge);
    let far = if data.start_node == near {
        data.end_node
    } else {
        data.start_node
    };
    let (entries, exits) = arm_lanes(graph, network, edge, near);
    Arm {
        far,
        near,
        entries,
        exits,
    }
}

impl Site {
    fn new(layout: Layout) -> Self {
        match layout {
            Layout::Road { kmh, lanes } => Self::road(kmh, lanes),
            Layout::Junction { lanes } => Self::junction(lanes),
            Layout::Ring { diameter_m, lanes } => {
                let plan = converted_ring(diameter_m, &ANGLES, (lanes, lanes), false);
                Self::ring(plan.unwrap())
            }
            Layout::FanRing { diameter_m } => {
                let (mut graph, hub) = hub_graph(&ANGLES, (2, 2), false);
                for arm in [1, 3] {
                    *graph.edge_mut(arm) = divided(graph.edge(arm).clone(), 1, 1, None);
                }
                finalize(&mut graph);
                let site = RoundaboutSite::Junction(hub);
                let request = RoundaboutRequest { site, diameter_m };
                let applied = RoundaboutLayout::prepare(&graph, request)
                    .unwrap()
                    .apply(&mut graph);
                finalize(&mut graph);
                Self::ring((graph, applied))
            }
        }
    }

    fn road(kmh: f32, lanes: u8) -> Self {
        let mut graph = RegionGraph::new();
        let a = graph.add_node(point(0.0, 0.0, false), NodeType::Junction);
        let b = graph.add_node(point(ROAD_M, 0.0, false), NodeType::Junction);
        let id = connect(&mut graph, a, b, 7.0, false);
        *graph.edge_mut(id) = divided(graph.edge(id).clone(), lanes, lanes, None);
        graph.edge_mut(id).speed_limit = kmh / 3.6;
        finalize(&mut graph);
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        let arms = vec![arm(&graph, &network, id, b)];
        Self {
            graph,
            network,
            arms,
            ring_nodes: Vec::new(),
            next_arm: vec![0],
        }
    }

    // The sense of travel a ring on these arms has, for the plain junction's turns.
    fn sense(lanes: u8) -> Vec<usize> {
        let (graph, applied) = converted_ring(50.0, &ANGLES, (lanes, lanes), false).unwrap();
        (0..applied.arms.len())
            .map(|from| {
                let end = graph.edge(applied.ring[from]).end_node;
                let to = applied.nodes.iter().position(|&n| n == end).unwrap();
                (0..applied.arms.len())
                    .find(|&arm| applied.arms[arm] == applied.arms[to])
                    .unwrap()
            })
            .collect()
    }

    fn set_arm_speeds(graph: &mut RegionGraph, arms: &[usize]) {
        for &edge in arms {
            graph.edge_mut(edge).speed_limit = DEFAULT_URBAN_ROAD_SPEED_MS;
        }
    }

    fn junction(lanes: u8) -> Self {
        let (mut graph, hub) = hub_graph(&ANGLES, (lanes, lanes), false);
        let ids: Vec<usize> = (0..ANGLES.len()).collect();
        Self::set_arm_speeds(&mut graph, &ids);
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        let arms = ids
            .iter()
            .map(|&edge| arm(&graph, &network, edge, hub))
            .collect();
        Self {
            graph,
            network,
            arms,
            ring_nodes: Vec::new(),
            next_arm: Self::sense(lanes),
        }
    }

    fn ring((mut graph, applied): (RegionGraph, RoundaboutApplied)) -> Self {
        Self::set_arm_speeds(&mut graph, &applied.arms);
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        let arms = (0..applied.arms.len())
            .map(|index| arm(&graph, &network, applied.arms[index], applied.nodes[index]))
            .collect();
        let next_arm = (0..applied.arms.len())
            .map(|from| {
                let end = graph.edge(applied.ring[from]).end_node;
                applied.nodes.iter().position(|&n| n == end).unwrap()
            })
            .collect();
        Self {
            graph,
            network,
            arms,
            ring_nodes: applied.nodes,
            next_arm,
        }
    }

    // The arm `exits` exits after arm `from` in the sense of travel.
    fn exit_arm(&self, from: usize, exits: usize) -> usize {
        (0..exits).fold(from, |at, _| self.next_arm[at])
    }

    // The node path from arm `from` to arm `to`.
    fn path(&self, from: usize, to: usize) -> Vec<u32> {
        if self.arms.len() == 1 {
            return vec![self.arms[0].far, self.arms[0].near];
        }
        let mut path = vec![self.arms[from].far, self.arms[from].near];
        if !self.ring_nodes.is_empty() {
            let mut at = from;
            while at != to {
                at = self.next_arm[at];
                path.push(self.arms[at].near);
            }
        }
        path.push(self.arms[to].far);
        path
    }
}

// One entry lane's source.
struct Source {
    arm: usize,
    lane: usize,
    exits: Vec<usize>,
    turn: usize,
    // Offered demand: cars per second, the credit so far and the exits' weights and counts.
    rate: Option<f32>,
    credit: f32,
    weights: [u32; 3],
    given: [u32; 3],
    placed: u32,
    blocked_ticks: u64,
    min_queue: usize,
    queue_sum: u64,
}

#[derive(Clone, Default)]
struct LaneTrace {
    entries: u32,
    car_s: f64,
    metres: f64,
    standing_s: f64,
}

// What an entry's head stood for, in ticks at the line.
#[derive(Clone, Copy, Default)]
struct Holds {
    free: u64,
    connector_slot: u64,
    circulating: u64,
    arriving_head: u64,
    ring_slot: u64,
}

struct Run {
    per_exit: Vec<u32>,
    per_source: Vec<u32>,
    min_queue: usize,
    mean_queue: f64,
    lost: u32,
    minutes: f64,
    trace: Vec<LaneTrace>,
    holds: Holds,
}

fn sources(site: &Site, demand: Demand) -> Vec<Source> {
    let source = |arm: usize, lane: usize, exits: Vec<usize>| Source {
        arm,
        lane,
        exits,
        turn: 0,
        rate: None,
        credit: 0.0,
        weights: [0; 3],
        given: [0; 3],
        placed: 0,
        blocked_ticks: 0,
        min_queue: usize::MAX,
        queue_sum: 0,
    };
    let mut out = Vec::new();
    for (index, arm) in site.arms.iter().enumerate() {
        for (rank, &lane) in arm.entries.iter().enumerate() {
            match demand {
                // Only the outermost lane turns right and only the innermost left.
                Demand::All(1) if rank > 0 => {}
                Demand::All(3) if rank + 1 < arm.entries.len() => {}
                Demand::All(exits) => out.push(source(index, lane, vec![exits])),
                Demand::One(exits) if index == 0 => out.push(source(index, lane, vec![exits])),
                Demand::One(_) => {}
                Demand::Mixed if arm.entries.len() == 1 => {
                    out.push(source(index, lane, vec![1, 2, 3]))
                }
                Demand::Mixed if rank == 0 => out.push(source(index, lane, vec![1, 2])),
                Demand::Mixed => out.push(source(index, lane, vec![3, 2])),
                Demand::Offered(movements, per_min) => {
                    // A two-lane arm's outer lane takes the first exit, its inner lane the third
                    // and each half the second, as `Mixed`.
                    let [first, second, third] = movements[index];
                    let weights = match (arm.entries.len(), rank) {
                        (1, _) => [first, second, third],
                        (_, 0) => [first, second / 2, 0],
                        _ => [0, second - second / 2, third],
                    };
                    let total: u32 = movements.iter().flatten().sum();
                    let share = weights.iter().sum::<u32>() as f32 / total as f32;
                    let mut offered = source(index, lane, vec![1]);
                    offered.rate = Some(per_min * share / 60.0);
                    offered.weights = weights;
                    out.push(offered);
                }
            }
        }
    }
    out
}

// Places one car at `lane`'s start, standing, on the path; reuses a car the sink took off.
fn place(
    agents: &mut AgentSystem,
    free: &mut Vec<usize>,
    site: &Site,
    lane: usize,
    path: Vec<u32>,
) -> usize {
    let lanes = &site.network.lane_system;
    let id = match free.pop() {
        Some(id) => {
            agents.agents.enter_lane(id, lanes, lane, 0.0, path[0]);
            agents.agents.current_path[id] = path;
            agents.agents.current_path_index[id] = 1;
            agents.agents.next_replan_time[id] = 0.0;
            agents.agents.lane_change_from_lane_id[id] = u32::MAX;
            id
        }
        None => agents.spawn_debug_car(path, lanes, lane, Vector2::ZERO),
    };
    agents.agents.speed[id] = 0.0;
    id
}

fn take_off(agents: &mut AgentSystem, free: &mut Vec<usize>, id: usize) {
    agents.agents.transit[id] = TRANSIT_IMMIGRATING;
    agents.agents.current_lane_id[id] = usize::MAX;
    agents.agents.current_path[id].clear();
    agents.hold_departure(id);
    free.push(id);
}

// Tallies what the head of each arm's entry lanes stands for this tick.
fn tally_holds(site: &Site, agents: &AgentSystem, sources: &[Source], holds: &mut Holds) {
    let system = &site.network.lane_system;
    let buckets = &agents.lane_buckets;
    for source in sources {
        let Some(&(d, _)) = buckets[source.lane].last() else {
            continue;
        };
        let lane = &system.lanes[source.lane];
        if lane.length - d > 0.05 {
            continue;
        }
        let mut reason = None;
        for &connector in lane.next_lanes.iter() {
            if !lane_attach_slot_clear(&buckets[connector], 0.0) {
                reason.get_or_insert(0);
                continue;
            }
            let Some(entry) = system.ring_entry(connector) else {
                reason = Some(9);
                break;
            };
            match ring_entry_blocker(connector, system, buckets) {
                None => {
                    reason = Some(9);
                    break;
                }
                Some(lane) if lane == entry.circulating as usize => {
                    reason.get_or_insert(1);
                }
                Some(lane) if lane == entry.ring_in as usize => {
                    reason.get_or_insert(2);
                }
                Some(_) => {
                    reason.get_or_insert(3);
                }
            }
        }
        match reason {
            Some(0) => holds.connector_slot += 1,
            Some(1) => holds.circulating += 1,
            Some(2) => holds.arriving_head += 1,
            Some(3) => holds.ring_slot += 1,
            _ => holds.free += 1,
        }
    }
}

// Seconds source `index` waits before its first car in run `phase`: the fixture is symmetrical and
// deterministic, so one start is one periodic orbit, and flows differ between orbits.
fn start_delay_s(phase: u32, index: usize) -> f32 {
    if phase == 0 {
        return 0.0;
    }
    let mixed = (phase * 2_654_435_761).wrapping_add(index as u32 * 40_503);
    ((mixed >> 8) % 2_000) as f32 / 100.0
}

// The exit of a source's next car: its fixed turn, or the offered exit furthest behind its share.
fn next_exit(source: &mut Source) -> usize {
    if source.rate.is_none() {
        return source.exits[source.turn % source.exits.len()];
    }
    let total: u32 = source.weights.iter().sum();
    let placed: u32 = source.given.iter().sum();
    let behind = |k: usize| {
        i64::from(source.weights[k]) * i64::from(placed + 1)
            - i64::from(source.given[k]) * i64::from(total)
    };
    let exit = (0..3).max_by_key(|&k| behind(k)).unwrap();
    source.given[exit] += 1;
    exit + 1
}

fn run(layout: Layout, demand: Demand, dt: f32, phase: u32, warm_s: f32, measure_s: f32) -> Run {
    let mut site = Site::new(layout);
    let mut agents = AgentSystem::new();
    let mut sources = sources(&site, demand);
    let allocator = BuildingAllocator::new();
    let time = TimeSystem::new();
    let lane_count = site.network.lane_system.lanes.len();
    let mut trace = vec![LaneTrace::default(); lane_count];
    let mut live: Vec<(usize, usize, usize)> = Vec::new(); // car, exit arm, source
    let mut last_lane: Vec<usize> = Vec::new();
    let mut free = Vec::new();
    let mut per_exit = vec![0; site.arms.len()];
    let mut per_source = vec![0; sources.len()];
    let mut holds = Holds::default();
    let mut lost = 0;
    let ticks = ((warm_s + measure_s) / dt).ceil() as usize;
    let warm_ticks = (warm_s / dt).ceil() as usize;
    for tick in 0..ticks {
        let measuring = tick >= warm_ticks;
        for (index, source) in sources.iter_mut().enumerate() {
            if (tick as f32) * dt < start_delay_s(phase, index) {
                continue;
            }
            source.credit += source.rate.unwrap_or(0.0) * dt;
            let clear = lane_attach_slot_clear(
                agents
                    .lane_buckets
                    .get(source.lane)
                    .map_or(&[][..], Vec::as_slice),
                0.0,
            );
            if !clear {
                source.blocked_ticks += u64::from(measuring);
                continue;
            }
            if source.rate.is_some() {
                if source.credit < 1.0 {
                    continue;
                }
                source.credit -= 1.0;
            }
            let exits = next_exit(source);
            source.turn += 1;
            source.placed += 1;
            let to = site.exit_arm(source.arm, exits);
            let id = place(
                &mut agents,
                &mut free,
                &site,
                source.lane,
                site.path(source.arm, to),
            );
            live.push((id, to, index));
            if last_lane.len() <= id {
                last_lane.resize(id + 1, usize::MAX);
            }
            last_lane[id] = usize::MAX;
            agents.invalidate_lane_bucket_snapshot();
        }

        agents.tick(&allocator, &mut site.network, &mut site.graph, dt, &time);

        let lanes = &site.network.lane_system.lanes;
        let mut took_off = false;
        let mut at = 0;
        while at < live.len() {
            let (id, to, source) = live[at];
            let lane = agents.agents.current_lane_id[id];
            if lane >= lane_count {
                // A car that lost its path has nowhere to go: the fixture's routes do not fail.
                lost += u32::from(measuring);
                take_off(&mut agents, &mut free, id);
                took_off = true;
                live.swap_remove(at);
                continue;
            }
            let speed = agents.agents.speed[id];
            if measuring {
                let cell = &mut trace[lane];
                cell.entries += u32::from(last_lane[id] != lane);
                cell.car_s += f64::from(dt);
                cell.metres += f64::from(speed * dt);
                cell.standing_s += if speed < 0.1 { f64::from(dt) } else { 0.0 };
            }
            last_lane[id] = lane;
            let sink = site.arms[to].exits.contains(&lane)
                && agents.agents.lane_distance[id] >= lanes[lane].length - SINK_BEFORE_END_M;
            let road_sink = site.arms.len() == 1
                && site.arms[0].entries.contains(&lane)
                && agents.agents.lane_distance[id] >= lanes[lane].length - SINK_BEFORE_END_M;
            if sink || road_sink {
                if measuring {
                    per_exit[to] += 1;
                    per_source[source] += 1;
                }
                take_off(&mut agents, &mut free, id);
                took_off = true;
                live.swap_remove(at);
                continue;
            }
            at += 1;
        }
        if took_off {
            agents.invalidate_lane_bucket_snapshot();
        }
        if measuring {
            tally_holds(&site, &agents, &sources, &mut holds);
            for source in &mut sources {
                let queue = agents.lane_buckets[source.lane].len();
                source.min_queue = source.min_queue.min(queue);
                source.queue_sum += queue as u64;
            }
        }
    }
    let samples = (ticks - warm_ticks) as f64;
    Run {
        per_exit,
        per_source,
        min_queue: sources.iter().map(|s| s.min_queue).min().unwrap_or(0),
        mean_queue: sources
            .iter()
            .map(|s| s.queue_sum as f64 / samples)
            .sum::<f64>()
            / sources.len() as f64,
        lost,
        minutes: f64::from(measure_s) / 60.0,
        trace,
        holds,
    }
}

// The most a following lane carries at limit `v0`, cars per minute: the top of the model's steady
// state, bumper gap `(s0 + vT) / sqrt(1 - (v / v0)^4)`, in steps of 1 cm/s.
fn idm_lane_capacity_per_min(v0: f32) -> (f32, f32) {
    let (mut best, mut at) = (0.0, 0.0);
    let mut v = 0.01;
    while v < v0 {
        let gap = (IDM_S_MIN + v * IDM_T_HEAD) / (1.0 - (v / v0).powi(4)).sqrt();
        let flow = v / (CAR_LENGTH + gap);
        if flow > best {
            (best, at) = (flow, v);
        }
        v += 0.01;
    }
    (best * 60.0, at)
}

fn print_lanes(layout: Layout) {
    let site = Site::new(layout);
    let lanes = &site.network.lane_system.lanes;
    for (id, lane) in lanes.iter().enumerate() {
        if lane.lane_type != LaneType::Vehicle {
            continue;
        }
        let kind = if lane.edge_id == usize::MAX {
            match site.network.lane_system.ring_entry(id) {
                Some(_) => "entry-connector",
                None => "connector",
            }
        } else if site.graph.edge(lane.edge_id).roundabout.is_some() {
            "ring-lane"
        } else {
            "arm"
        };
        let cap = if lane.edge_id == usize::MAX {
            connector_turn_speed(lane)
        } else {
            site.graph.edge(lane.edge_id).speed_limit
        };
        let next: Vec<usize> = lane.next_lanes.iter().copied().collect();
        eprintln!(
            "LANE {layout:?} {id} {kind} len {:.2} cap {cap:.2} node {} edge {} next {next:?}",
            lane.length, lane.node_id as i64, lane.edge_id as i64
        );
    }
}

fn report(name: &str, layout: Layout, demand: Demand, dt: f32, trace: bool) {
    let (warm, measure) = (120.0, 600.0);
    let out = run(layout, demand, dt, 0, warm, measure);
    let total: u32 = out.per_exit.iter().sum();
    let per_min = |n: u32| f64::from(n) / out.minutes;
    let mut orbits: Vec<f64> = (0..PHASES)
        .map(|phase| {
            let other = run(layout, demand, dt, phase, warm, measure);
            f64::from(other.per_exit.iter().sum::<u32>()) / other.minutes
        })
        .collect();
    orbits.sort_unstable_by(f64::total_cmp);
    let per_source: Vec<String> = out
        .per_source
        .iter()
        .map(|&n| format!("{:.1}", per_min(n)))
        .collect();
    let h = out.holds;
    eprintln!(
        "CAPACITY {name} dt {dt:.4} total {:.1}/min range {:.1} {:.1} first {:.1} sources [{}] \
         queue min {} mean {:.1} lost {} holds free {} connector {} circulating {} head {} slot {}",
        orbits[orbits.len() / 2],
        orbits[0],
        orbits[orbits.len() - 1],
        per_min(total),
        per_source.join(" "),
        out.min_queue,
        out.mean_queue,
        out.lost,
        h.free,
        h.connector_slot,
        h.circulating,
        h.arriving_head,
        h.ring_slot,
    );
    if !trace {
        return;
    }
    let site = Site::new(layout);
    let lanes = &site.network.lane_system.lanes;
    let seconds = f64::from(measure);
    for (id, cell) in out.trace.iter().enumerate() {
        if cell.car_s == 0.0 {
            continue;
        }
        let lane = &lanes[id];
        let (kind, cap) = if lane.edge_id == usize::MAX {
            let kind = if site.network.lane_system.ring_entry(id).is_some() {
                "entry"
            } else {
                "conn"
            };
            (kind, connector_turn_speed(lane))
        } else if site.graph.edge(lane.edge_id).roundabout.is_some() {
            ("ring", site.graph.edge(lane.edge_id).speed_limit)
        } else {
            ("arm", site.graph.edge(lane.edge_id).speed_limit)
        };
        eprintln!(
            "TRACE {name} dt {dt:.4} lane {id} {kind} len {:.1} cap {cap:.2} flow {:.1}/min \
             speed {:.2} cars {:.2} standing {:.0}%",
            lane.length,
            f64::from(cell.entries) / out.minutes,
            cell.metres / cell.car_s,
            cell.car_s / seconds,
            100.0 * cell.standing_s / cell.car_s,
        );
    }
}

// The most of a junction's movements a layout carries: `per_min` offered in all, stepped up.
fn report_offered(name: &str, layout: Layout, movements: &'static Movements, dt: f32) {
    let (warm, measure) = (240.0, 600.0);
    for per_min in (20..=200).step_by(10) {
        let mut flows: Vec<f64> = (1..=5)
            .map(|phase| {
                let demand = Demand::Offered(movements, per_min as f32);
                let out = run(layout, demand, dt, phase, warm, measure);
                f64::from(out.per_exit.iter().sum::<u32>()) / out.minutes
            })
            .collect();
        flows.sort_unstable_by(f64::total_cmp);
        eprintln!(
            "OFFERED {name} dt {dt:.4} offered {per_min} carried {:.1} range {:.1} {:.1}",
            flows[2], flows[0], flows[4]
        );
    }
}

#[test]
#[ignore = "manual measurement: how much of a city junction's day a layout carries"]
fn measure_city_movements() {
    let ring_1 = Layout::Ring {
        diameter_m: 36.0,
        lanes: 1,
    };
    let ring_2 = Layout::Ring {
        diameter_m: 50.0,
        lanes: 2,
    };
    for dt in [DT_1X, DT_32X] {
        report_offered("ring-36-1-city-1+1", ring_1, &CITY_1_1, dt);
        report_offered("ring-50-2-city-4lane", ring_2, &CITY_4_LANE, dt);
        report_offered("junction-1+1", Layout::Junction { lanes: 1 }, &CITY_1_1, dt);
        report_offered(
            "junction-2+2",
            Layout::Junction { lanes: 2 },
            &CITY_4_LANE,
            dt,
        );
    }
}

#[test]
#[ignore = "manual measurement: prints the saturation flows of `docs/traffic.md`"]
fn measure_capacity() {
    use Demand::{All, Mixed, One};
    let only = std::env::var("METRUM_CAPACITY_ONLY").ok();
    let trace = std::env::var("METRUM_CAPACITY_TRACE").is_ok();
    // Game speeds to run, comma separated: the agent step is `FRAME_DT_S` times the speed.
    let steps: Vec<f32> =
        std::env::var("METRUM_CAPACITY_SPEEDS").map_or(vec![DT_1X, DT_32X], |text| {
            let speed = |part: &str| part.parse::<f32>().expect("a game speed");
            text.split(',').map(|part| speed(part) * DT_1X).collect()
        });
    for v0 in [DEFAULT_URBAN_ROAD_SPEED_MS, 20.0 / 3.6, 6.0] {
        let (flow, at) = idm_lane_capacity_per_min(v0);
        eprintln!("BOUND lane at {v0:.2} m/s: {flow:.1}/min at {at:.2} m/s");
    }
    let mut cases: Vec<(String, Layout, Demand)> = Vec::new();
    for lanes in [1, 2] {
        for kmh in [50.0, 20.0] {
            cases.push((
                format!("road-{lanes}+{lanes}-{kmh}"),
                Layout::Road { kmh, lanes },
                All(2),
            ));
        }
        let junction = Layout::Junction { lanes };
        let name = format!("junction-{lanes}+{lanes}");
        cases.push((format!("{name}-through-all"), junction, All(2)));
        cases.push((format!("{name}-through-one"), junction, One(2)));
        cases.push((format!("{name}-right-all"), junction, All(1)));
        cases.push((format!("{name}-left-all"), junction, All(3)));
        cases.push((format!("{name}-mixed"), junction, Mixed));
    }
    for (name, diameter_m, lanes) in [("ring-36-1", 36.0, 1), ("ring-50-2", 50.0, 2)] {
        let ring = Layout::Ring { diameter_m, lanes };
        cases.push((format!("{name}-through-all"), ring, All(2)));
        cases.push((format!("{name}-through-one"), ring, One(2)));
        cases.push((format!("{name}-first-exit-all"), ring, All(1)));
        cases.push((format!("{name}-third-exit-all"), ring, All(3)));
        cases.push((format!("{name}-mixed"), ring, Mixed));
    }
    let fan = Layout::FanRing { diameter_m: 50.0 };
    cases.push(("ring-50-fan-through-all".to_owned(), fan, All(2)));
    cases.push(("ring-50-fan-through-one-wide".to_owned(), fan, One(2)));
    cases.push(("ring-50-fan-mixed".to_owned(), fan, Mixed));
    if trace {
        print_lanes(Layout::Ring {
            diameter_m: 36.0,
            lanes: 1,
        });
        print_lanes(Layout::Junction { lanes: 1 });
    }
    for (name, layout, demand) in cases {
        if only.as_ref().is_some_and(|part| !name.contains(part)) {
            continue;
        }
        for &dt in &steps {
            report(&name, layout, demand, dt, trace);
        }
    }
}
