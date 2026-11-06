// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-62` round R7c: what a day does at the city's roundabouts, on one `ROUNDABOUT_DAY` line at
//! the audit's end. The gate's two numbers, the `TRAFFIC-14` saturation reports and the trip
//! failures on a ring, with the measurements around them: every car an entry lets in, how long
//! heads stand at an entry and the longest queues. A city without rings prints the gridlock and
//! failure counts alone, the ring-free run's half of the comparison. Also a hash of every car's
//! lane, distance and speed bits, which 1, 4 and 8 workers must share.

use crate::nodes::sim::core::state::SimCore;
use crate::simulation::economy::agents::tick::FailurePlaces;
use crate::simulation::economy::logistics::{ShipmentEndpoint, ShipmentStatus};
use crate::simulation::network::lanes::LaneType;
use std::collections::{BTreeMap, BTreeSet};

const AT_END_M: f32 = 0.05;

// One arm's entry lane at a ring: its connectors onto the ring and the head standing at its end.
struct EntryWatch {
    lane: usize,
    node: usize,
    connectors: Vec<usize>,
    head: usize,
    since_s: f32,
    last_in: Vec<usize>,
    entered: u64,
    queue_peak: usize,
    waits_s: Vec<f32>,
}

pub(super) struct RingDay {
    ring_nodes: BTreeSet<u32>,
    ring_edges: BTreeSet<usize>,
    entries: Vec<EntryWatch>,
    failures: u64,
    ring_failures: BTreeMap<&'static str, u64>,
    ring_failure_samples: Vec<String>,
    cycles_peak: u32,
    confirmed_peak: u32,
    saturated_peak: u32,
    saturated_ring_junctions: BTreeSet<u32>,
    junction: Option<JunctionFlows>,
}

// `METRUM_AUDIT_JUNCTION=<node,node,...>` (`ROAD-62` round R7c2): what crosses one junction, a plain
// node or the four nodes of a ring. Cars into each connector there by game hour, the deepest queue
// on each lane feeding it, and of the day's deliveries those whose carrier used it, by kind.
struct JunctionFlows {
    connectors: Vec<ConnectorFlow>,
    feeders: BTreeMap<usize, [usize; 24]>,
    start_s: f32,
    day_s: f32,
    crossed: BTreeSet<u64>,
    settled: BTreeMap<u64, &'static str>,
}

struct ConnectorFlow {
    lane: usize,
    from: usize,
    last_in: usize,
    hours: [u32; 24],
}

impl JunctionFlows {
    fn new(core: &SimCore) -> Option<Self> {
        let nodes: BTreeSet<usize> = std::env::var("METRUM_AUDIT_JUNCTION")
            .ok()?
            .split(',')
            .map(|node| node.parse().expect("METRUM_AUDIT_JUNCTION: node ids"))
            .collect();
        let lanes = &core.transit_network.lane_system.lanes;
        let mut connectors = Vec::new();
        let mut feeders = BTreeMap::new();
        for (from, lane) in lanes.iter().enumerate() {
            if lane.lane_type != LaneType::Vehicle {
                continue;
            }
            for &next in lane.next_lanes.iter() {
                let connector = &lanes[next];
                if connector.edge_id == usize::MAX && nodes.contains(&connector.node_id) {
                    connectors.push(ConnectorFlow {
                        lane: next,
                        from,
                        last_in: usize::MAX,
                        hours: [0; 24],
                    });
                    feeders.insert(from, [0; 24]);
                }
            }
        }
        // What was settled before the day is not the day's.
        let settled = core
            .logistics
            .shipments
            .iter()
            .filter(|shipment| !matches!(shipment.status, ShipmentStatus::InTransit))
            .map(|shipment| (shipment.id, "before"))
            .collect();
        Some(Self {
            connectors,
            feeders,
            start_s: core.agents.sim_time,
            day_s: core.time.seconds_per_day as f32,
            crossed: BTreeSet::new(),
            settled,
        })
    }

    fn after_frame(&mut self, core: &SimCore) {
        let buckets = &core.agents.lane_buckets;
        let agents = &core.agents.agents;
        let elapsed = (core.agents.sim_time - self.start_s) / self.day_s;
        let hour = ((elapsed.fract() * 24.0) as usize).min(23);
        for flow in &mut self.connectors {
            let bucket = &buckets[flow.lane];
            for &(_, car) in bucket {
                if agents.freight_shipment_id[car] != 0 {
                    self.crossed.insert(agents.freight_shipment_id[car]);
                }
            }
            let newest = bucket.first().map_or(usize::MAX, |car| car.1);
            if newest != usize::MAX {
                flow.hours[hour] += u32::from(newest != flow.last_in);
                flow.last_in = newest;
            }
        }
        for (&lane, peaks) in &mut self.feeders {
            peaks[hour] = peaks[hour].max(buckets[lane].len());
        }
        for shipment in &core.logistics.shipments {
            if !matches!(
                shipment.status,
                ShipmentStatus::InTransit | ShipmentStatus::Failed | ShipmentStatus::Expired
            ) {
                let kind = match (shipment.source, shipment.destination) {
                    (ShipmentEndpoint::OwaBorder(_), _) => "import",
                    (_, ShipmentEndpoint::OwaBorder(_)) => "export",
                    _ => "local",
                };
                self.settled.entry(shipment.id).or_insert(kind);
            }
        }
    }

    fn report(&self, core: &SimCore) {
        let lanes = &core.transit_network.lane_system.lanes;
        let edge_of = |lane: usize| lanes[lanes[lane].next_lanes[0]].edge_id;
        let mut kinds: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
        for (id, &kind) in &self.settled {
            let row = kinds.entry(kind).or_default();
            row.0 += 1;
            row.1 += u32::from(self.crossed.contains(id));
        }
        let movements: Vec<_> = self
            .connectors
            .iter()
            .map(|flow| {
                serde_json::json!({
                    "connector": flow.lane, "node": lanes[flow.lane].node_id,
                    "from_lane": flow.from, "from_edge": lanes[flow.from].edge_id as i64,
                    "to_edge": edge_of(flow.lane), "length_m": lanes[flow.lane].length,
                    "ring_entry": core.transit_network.lane_system.ring_entry(flow.lane).is_some(),
                    "total": flow.hours.iter().sum::<u32>(), "hours": flow.hours,
                })
            })
            .collect();
        let feeders: Vec<_> = self
            .feeders
            .iter()
            .map(|(lane, peaks)| serde_json::json!({"lane": lane, "edge": lanes[*lane].edge_id as i64, "peaks": peaks}))
            .collect();
        eprintln!(
            "JUNCTION_FLOWS {}",
            serde_json::json!({"deliveries_by_kind": kinds, "movements": movements, "feeders": feeders})
        );
    }
}

impl RingDay {
    pub(super) fn new(core: &mut SimCore) -> Self {
        core.agents.trip_failures.kept = Some(Vec::new());
        let graph = &core.region_graph;
        let mut ring_nodes = BTreeSet::new();
        let mut ring_edges = BTreeSet::new();
        for id in 0..graph.edge_count() {
            let edge = graph.edge(id);
            if !edge.deleted && edge.roundabout.is_some() {
                ring_edges.insert(id);
                ring_nodes.extend([edge.start_node, edge.end_node]);
            }
        }
        let system = &core.transit_network.lane_system;
        let lanes = &system.lanes;
        let mut entries = Vec::new();
        for (id, lane) in lanes.iter().enumerate() {
            if lane.edge_id == usize::MAX || lane.lane_type != LaneType::Vehicle {
                continue;
            }
            let connectors: Vec<usize> = lane
                .next_lanes
                .iter()
                .copied()
                .filter(|&next| system.ring_entry(next).is_some())
                .collect();
            if let Some(&first) = connectors.first() {
                entries.push(EntryWatch {
                    lane: id,
                    node: lanes[first].node_id,
                    last_in: vec![usize::MAX; connectors.len()],
                    connectors,
                    head: usize::MAX,
                    since_s: 0.0,
                    entered: 0,
                    queue_peak: 0,
                    waits_s: Vec::new(),
                });
            }
        }
        Self {
            ring_nodes,
            ring_edges,
            entries,
            failures: 0,
            ring_failures: BTreeMap::new(),
            ring_failure_samples: Vec::new(),
            cycles_peak: 0,
            confirmed_peak: 0,
            saturated_peak: 0,
            saturated_ring_junctions: BTreeSet::new(),
            junction: JunctionFlows::new(core),
        }
    }

    // Whether a failure's cause, or the place the agent stood, is on a ring: a ring node, a ring
    // road, one of its lanes or a connector at a ring node.
    fn on_ring(&self, core: &SimCore, failure: &FailurePlaces) -> bool {
        let lanes = &core.transit_network.lane_system.lanes;
        let node_on_ring = |node: u32| self.ring_nodes.contains(&node);
        let edge_on_ring = |edge: usize| self.ring_edges.contains(&edge);
        let lane_on_ring = |lane: usize| {
            lanes.get(lane).is_some_and(|lane| {
                edge_on_ring(lane.edge_id)
                    || (lane.edge_id == usize::MAX && node_on_ring(lane.node_id as u32))
            })
        };
        let (node, edge, lane) = failure.agent;
        failure.nodes.iter().any(|&node| node_on_ring(node))
            || failure.edges.iter().any(|&edge| edge_on_ring(edge))
            || failure.lanes.iter().any(|&lane| lane_on_ring(lane))
            || node_on_ring(node)
            || edge_on_ring(edge)
            || lane_on_ring(lane)
    }

    pub(super) fn after_frame(&mut self, core: &mut SimCore) {
        if let Some(junction) = &mut self.junction {
            junction.after_frame(core);
        }
        let kept = core.agents.trip_failures.kept.take().unwrap_or_default();
        self.failures += kept.len() as u64;
        for failure in &kept {
            if self.on_ring(core, failure) {
                *self.ring_failures.entry(failure.cause).or_default() += 1;
                if self.ring_failure_samples.len() < 24 {
                    self.ring_failure_samples.push(format!("{failure:?}"));
                }
            }
        }
        core.agents.trip_failures.kept = Some(Vec::with_capacity(kept.capacity()));

        let report = &core.agents.gridlock.report();
        self.cycles_peak = self.cycles_peak.max(report.cycles);
        self.confirmed_peak = self.confirmed_peak.max(report.confirmed);
        self.saturated_peak = self.saturated_peak.max(report.saturated);
        self.saturated_ring_junctions.extend(
            report
                .junctions
                .iter()
                .filter(|node| self.ring_nodes.contains(node)),
        );

        let now = core.agents.sim_time;
        let buckets = &core.agents.lane_buckets;
        let lanes = &core.transit_network.lane_system.lanes;
        for watch in &mut self.entries {
            let Some(bucket) = buckets.get(watch.lane) else {
                continue;
            };
            watch.queue_peak = watch.queue_peak.max(bucket.len());
            let head = bucket
                .last()
                .filter(|head| head.0 + AT_END_M >= lanes[watch.lane].length)
                .map_or(usize::MAX, |head| head.1);
            if head != watch.head {
                if watch.head != usize::MAX {
                    watch.waits_s.push(now - watch.since_s);
                }
                watch.head = head;
                watch.since_s = now;
            }
            for (slot, &connector) in watch.connectors.iter().enumerate() {
                let newest = buckets[connector].first().map_or(usize::MAX, |car| car.1);
                if newest != usize::MAX && newest != watch.last_in[slot] {
                    watch.entered += 1;
                }
                if newest != usize::MAX {
                    watch.last_in[slot] = newest;
                }
            }
        }
    }

    // A diagnostic: every car on the rings and at their connectors, node by node.
    fn dump(&self, core: &SimCore) {
        let lanes = &core.transit_network.lane_system.lanes;
        let buckets = &core.agents.lane_buckets;
        let agents = &core.agents.agents;
        let describe = |lane: usize| {
            let cars: Vec<String> = buckets[lane]
                .iter()
                .map(|&(d, i)| {
                    let idx = agents.current_path_index[i];
                    let path = &agents.current_path[i];
                    let next: Vec<u32> = path.iter().skip(idx).take(3).copied().collect();
                    format!(
                        "{i}@{d:.2} v{:.2} tr{} next{next:?} tgt{}",
                        agents.speed[i], agents.transit[i], agents.target_building[i] as i64
                    )
                })
                .collect();
            format!(
                "lane {lane} edge {} node {} len {:.2} entry {:?} next {:?}: {}",
                lanes[lane].edge_id as i64,
                lanes[lane].node_id as i64,
                lanes[lane].length,
                core.transit_network.lane_system.ring_entry(lane),
                &lanes[lane].next_lanes[..],
                cars.join(" | ")
            )
        };
        for cycle in super::waits::wait_cycles(core) {
            eprintln!(
                "RING_CYCLE {} cars, {} road heads, {} with room",
                cycle.cars.len(),
                cycle.road_heads,
                cycle.heads_with_room
            );
            for &i in &cycle.cars {
                let idx = agents.current_path_index[i];
                let next: Vec<u32> = agents.current_path[i]
                    .iter()
                    .skip(idx)
                    .take(4)
                    .copied()
                    .collect();
                let lane = agents.current_lane_id[i];
                eprintln!(
                    "  car {i} lane {lane} d {:.2}/{:.2} v {:.2} node {} next {next:?} detach ({} {}) edge {} conn_node {}",
                    agents.lane_distance[i],
                    lanes.get(lane).map_or(0.0, |l| l.length),
                    agents.speed[i],
                    agents.current_node[i],
                    agents.planned_detach_node[i],
                    agents.planned_detach_lane_id[i],
                    lanes.get(lane).map_or(0, |l| l.edge_id as i64),
                    lanes.get(lane).map_or(0, |l| l.node_id as i64)
                );
            }
        }
        for &node in &self.ring_nodes {
            eprintln!("RING_NODE {node}");
            for &edge in core.region_graph.node_adjacency(node) {
                for &lane in core
                    .transit_network
                    .lane_system
                    .edge_lanes
                    .get(&edge)
                    .into_iter()
                    .flatten()
                {
                    if lanes[lane].lane_type == LaneType::Vehicle {
                        eprintln!("  road {}", describe(lane));
                    }
                }
            }
            for &lane in core
                .transit_network
                .lane_system
                .node_lanes
                .get(&(node as usize))
                .into_iter()
                .flatten()
            {
                if lanes[lane].lane_type == LaneType::Vehicle {
                    eprintln!("  conn {}", describe(lane));
                }
            }
        }
    }

    pub(super) fn report(mut self, core: &SimCore) {
        if let Some(junction) = &self.junction {
            junction.report(core);
        }
        if std::env::var_os("METRUM_AUDIT_RINGS").is_some() {
            self.dump(core);
        }
        let now = core.agents.sim_time;
        let mut waits = Vec::new();
        let mut standing = 0;
        let mut rings: BTreeMap<usize, (u64, usize, f32, u32)> = BTreeMap::new();
        for watch in &mut self.entries {
            if watch.head != usize::MAX {
                watch.waits_s.push(now - watch.since_s);
                standing += 1;
            }
            let ring = rings.entry(watch.node).or_default();
            ring.0 += watch.entered;
            ring.1 = ring.1.max(watch.queue_peak);
            ring.2 = watch.waits_s.iter().copied().fold(ring.2, f32::max);
            ring.3 += 1;
            waits.extend_from_slice(&watch.waits_s);
        }
        waits.sort_unstable_by(f32::total_cmp);
        let at = |share: f32| {
            waits
                .get(((waits.len() as f32 - 1.0) * share).round().max(0.0) as usize)
                .copied()
                .unwrap_or(0.0)
        };
        let mean = waits.iter().sum::<f32>() / (waits.len().max(1) as f32);
        let by_node: Vec<_> = rings
            .iter()
            .map(|(node, ring)| {
                serde_json::json!({"node": node, "entered": ring.0, "queue_peak": ring.1,
                    "wait_max_s": ring.2})
            })
            .collect();
        let report = &core.agents.gridlock.report();
        eprintln!(
            "ROUNDABOUT_DAY {}",
            serde_json::json!({
                "rings_edges": self.ring_edges.len(), "ring_nodes": self.ring_nodes.len(),
                "entries": self.entries.len(),
                "gridlock": {
                    "checks": report.checks, "detours": report.detours,
                    "saturation_reports": report.saturation_reports,
                    "cycles_peak": self.cycles_peak, "confirmed_peak": self.confirmed_peak,
                    "saturated_peak": self.saturated_peak,
                    "ring_junctions_reported": self.saturated_ring_junctions,
                },
                "trip_failures": self.failures,
                "ring_trip_failures": self.ring_failures.values().sum::<u64>(),
                "ring_trip_failures_by_cause": self.ring_failures,
                "ring_failure_samples": self.ring_failure_samples,
                "entered": rings.values().map(|ring| ring.0).sum::<u64>(),
                "waits": {"count": waits.len(), "standing_at_end": standing, "mean_s": mean,
                    "p50_s": at(0.5), "p95_s": at(0.95), "max_s": at(1.0)},
                "by_entry_node": by_node,
                "state_hash": format!("{:016x}", state_hash(core)),
            })
        );
    }
}

// Every agent's lane, distance and speed bits, FNV-1a: equal across worker counts.
fn state_hash(core: &SimCore) -> u64 {
    let agents = &core.agents.agents;
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut mix = |value: u64| {
        hash = (hash ^ value).wrapping_mul(0x0000_0100_0000_01b3);
    };
    for i in 0..agents.len() {
        mix(agents.current_lane_id[i] as u64);
        mix(u64::from(agents.lane_distance[i].to_bits()));
        mix(u64::from(agents.speed[i].to_bits()));
    }
    hash
}
