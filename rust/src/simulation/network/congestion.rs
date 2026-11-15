// SPDX-License-Identifier: GPL-2.0-only

//! Periodic recustomization of the car-congestion metric (`TRAFFIC-17`).
//!
//! At its launch slot the `congestion routing` system snapshots one directed weight per edge
//! direction (the free-flow cost plus the mean routing delay of that direction's lanes) and
//! customizes the hierarchy's car-congestion metric from it as a [`PureJob`]; the result is
//! adopted a fixed number of slots later (`docs/simulation.md#pure-jobs`). A job whose adoption
//! slot the launching frame itself reaches, or a launch outside a frame, customizes in the slot
//! instead: the result is the same pure function of the snapshot. The frame waits for a late
//! job before it enters the pools ([`TransitNetwork::congestion_adoption_completion`]).
//!
//! A routing rebuild leaves the new hierarchy without a congestion metric and makes a pending
//! job's result stale (it is rejected at adoption by build generation); the next running frame
//! customizes one from the lanes' current delays ([`TransitNetwork::refresh_congestion`]).
//! Adoption moves only the congestion revision: no economy cache or quote reads it, and no
//! agent replans because of it.

use super::TransitNetwork;
use super::graph::RegionGraph;
use super::lanes::LaneType;
use crate::simulation::core::pure_job::{JobCompletion, PureJob};
use crate::simulation::pathing::cch::CongestionMetric;
use rayon::prelude::*;

/// The congestion customization in flight between its launch and adoption slots.
enum Pending {
    Running(PureJob<CongestionMetric>),
    Done(CongestionMetric),
}

/// A launched customization: the job, its adoption slot and the build it customizes.
struct Launched {
    job: Pending,
    adopt_slot: u64,
    build_generation: u32,
}

/// The congestion routing state a network carries: the pending customization and the storage
/// the next one reuses.
#[derive(Default)]
pub(crate) struct CongestionRouting {
    // The customization launched last.
    pending: Option<Launched>,
    // A copy of the pending customization's weights, so a save records it without waiting
    // for the job.
    snapshot: Vec<f32>,
    // A displaced metric whose storage the next customization reuses.
    spare: Option<CongestionMetric>,
}

impl TransitNetwork {
    /// Writes the directed congestion weights into `weights`, two per edge (forward,
    /// backward): the edge's free-flow cost plus the mean `route_delay_s` of its live vehicle
    /// lanes in that direction (0 without lanes). Edges in id order, lanes in their indexed
    /// order, so the sums do not depend on the worker count. O((edges + lanes) / p); no
    /// allocation once `weights` holds two per edge.
    pub(crate) fn congestion_weights_into(&self, graph: &RegionGraph, weights: &mut Vec<f32>) {
        weights.resize(graph.edge_count() * 2, 0.0);
        let lanes = &self.lane_system;
        weights
            .par_chunks_mut(2)
            .zip(graph.edges().par_iter())
            .enumerate()
            .for_each(|(id, (pair, edge))| {
                let mut sums = [(0.0_f32, 0_u32); 2];
                for &lane in lanes.edge_lanes.get(&id).into_iter().flatten() {
                    let lane = &lanes.lanes[lane];
                    if lane.lane_type == LaneType::Vehicle {
                        let sum = &mut sums[usize::from(!lane.is_fwd)];
                        sum.0 += lane.route_delay_s;
                        sum.1 += 1;
                    }
                }
                for (weight, (delay, count)) in pair.iter_mut().zip(sums) {
                    *weight = if count == 0 {
                        edge.base_cost
                    } else {
                        edge.base_cost + delay / count as f32
                    };
                }
            });
    }

    /// Snapshots the congestion weights and starts customizing them, to be adopted at
    /// `adopt_slot`; a customization still pending is superseded. With `synchronous` it
    /// customizes here (the launching frame reaches the adoption slot, or no frame runs).
    /// O(edges + lanes) for the snapshot and O(1) to start; nothing for a hierarchy without a
    /// car metric.
    pub(crate) fn launch_congestion(
        &mut self,
        graph: &RegionGraph,
        adopt_slot: u64,
        synchronous: bool,
    ) {
        self.congestion.pending = None;
        let Some(customizer) = self.cch_graph.congestion_customizer() else {
            return;
        };
        let mut spare = self.congestion.spare.take();
        let mut weights = spare
            .as_mut()
            .map(CongestionMetric::take_weights)
            .unwrap_or_default();
        self.congestion_weights_into(graph, &mut weights);
        self.congestion.snapshot.clone_from(&weights);
        let job = if synchronous {
            Pending::Done(customizer.customize(weights, spare))
        } else {
            Pending::Running(PureJob::spawn(move || customizer.customize(weights, spare)))
        };
        self.congestion.pending = Some(Launched {
            job,
            adopt_slot,
            build_generation: self.cch_graph.build_generation,
        });
    }

    /// Adopts the customization launched for `slot`, if any. O(1); a job not yet finished is
    /// finished here, so the frame's caller waits for it outside the pools first
    /// ([`Self::congestion_adoption_completion`]). A result customized on a hierarchy a rebuild
    /// replaced is dropped.
    pub(crate) fn adopt_congestion(&mut self, slot: u64) {
        if !matches!(&self.congestion.pending, Some(launched) if launched.adopt_slot == slot) {
            return;
        }
        let Some(launched) = self.congestion.pending.take() else {
            return;
        };
        let metric = match launched.job {
            Pending::Running(job) => job.take(),
            Pending::Done(metric) => metric,
        };
        self.congestion.spare = self.cch_graph.adopt_congestion(metric);
    }

    /// The pending customization's completion when its adoption slot is at most
    /// `through_slot` and it may not have finished: the frame's caller waits for it before
    /// entering the pools. O(1).
    pub(crate) fn congestion_adoption_completion(
        &self,
        through_slot: u64,
    ) -> Option<JobCompletion> {
        match &self.congestion.pending {
            Some(Launched {
                job: Pending::Running(job),
                adopt_slot,
                ..
            }) if *adopt_slot <= through_slot => Some(job.completion()),
            _ => None,
        }
    }

    /// The pending customization's weights and adoption slot (save); none for one customized
    /// on a hierarchy a rebuild has replaced, which its adoption would drop. O(1).
    pub(crate) fn pending_congestion(&self) -> Option<(&[f32], u64)> {
        let launched = self.congestion.pending.as_ref()?;
        (launched.build_generation == self.cch_graph.build_generation)
            .then_some((&self.congestion.snapshot[..], launched.adopt_slot))
    }

    /// Restores a saved pending customization: customizes `weights` here, to be adopted at
    /// `adopt_slot` as the saved game would have. O(customize).
    pub(crate) fn restore_pending_congestion(&mut self, weights: Vec<f32>, adopt_slot: u64) {
        let Some(customizer) = self.cch_graph.congestion_customizer() else {
            return;
        };
        self.congestion.snapshot.clone_from(&weights);
        let metric = customizer.customize(weights, None);
        self.congestion.pending = Some(Launched {
            job: Pending::Done(metric),
            adopt_slot,
            build_generation: self.cch_graph.build_generation,
        });
    }

    /// Adopts a congestion metric customized here from `weights` (a load's saved metric, or a
    /// refresh's snapshot). O(customize); nothing for a hierarchy without a car metric.
    pub(crate) fn install_congestion(&mut self, weights: Vec<f32>) {
        if let Some(customizer) = self.cch_graph.congestion_customizer() {
            let metric = customizer.customize(weights, self.congestion.spare.take());
            self.congestion.spare = self.cch_graph.adopt_congestion(metric);
        }
    }

    /// Gives a hierarchy without a congestion metric (a new game, or the first running frame
    /// after a routing rebuild) one customized from the lanes' current routing delays; edited
    /// roads' new lanes read 0. A frame calls it after its routing rebuild, so the game routes
    /// the same way whichever thread or moment installed the rebuild. O(1) when the hierarchy
    /// has one; otherwise O(edges + lanes) plus one customize, synchronously.
    pub(crate) fn refresh_congestion(&mut self, graph: &RegionGraph) {
        if self.cch_graph.congestion_weights().is_some()
            || self.cch_graph.congestion_customizer().is_none()
        {
            return;
        }
        let mut weights = self
            .congestion
            .spare
            .as_mut()
            .map(CongestionMetric::take_weights)
            .unwrap_or_default();
        self.congestion_weights_into(graph, &mut weights);
        self.install_congestion(weights);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::{NodeType, TransitFlags};
    use crate::simulation::pathing::cch::{CchGraph, CchPathScratch, RouteMetric};
    use godot::prelude::Vector3;

    // Two ways from A (node 0) to B (node 2): over T (node 1, shorter) and over U (node 3).
    fn two_ways() -> (RegionGraph, TransitNetwork) {
        let mut graph = RegionGraph::new();
        for (x, z) in [(0.0, 0.0), (100.0, -60.0), (200.0, 0.0), (100.0, 75.0)] {
            graph.add_node(Vector3::new(x, 0.0, z), NodeType::Junction);
        }
        for (a, b) in [(0, 1), (1, 2), (0, 3), (3, 2)] {
            let points = vec![graph.node(a).pos, graph.node(b).pos];
            let length = points[0].distance_to(points[1]);
            graph.add_edge(Edge {
                start_node: a,
                end_node: b,
                allowed_types: TransitFlags::CAR,
                speed_limit: 14.0,
                base_cost: length / 14.0,
                physical_length: length,
                fwd_lanes: 1,
                bkw_lanes: 1,
                width: 7.0,
                geometry: points.clone(),
                physical_geometry: points,
                ..Default::default()
            });
        }
        graph.rebuild_adjacency_list();
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = CchGraph::build(&graph);
        (graph, network)
    }

    fn car_route(network: &TransitNetwork, graph: &RegionGraph) -> Vec<u32> {
        let mut scratch = CchPathScratch::default();
        let (_, _, nodes) = network
            .cch_graph
            .find_path_in(
                0,
                2,
                usize::MAX,
                usize::MAX,
                graph,
                TransitFlags::CAR,
                RouteMetric::Congested,
                &mut scratch,
                None,
            )
            .unwrap();
        nodes.to_vec()
    }

    // Sets the routing delay of every vehicle lane of `edge` in direction `forward`.
    fn delay(network: &mut TransitNetwork, edge: usize, forward: bool, seconds: f32) {
        let lanes = &mut network.lane_system;
        for &lane in &lanes.edge_lanes[&edge] {
            let lane = &mut lanes.lanes[lane];
            if lane.lane_type == LaneType::Vehicle && lane.is_fwd == forward {
                lane.route_delay_s = seconds;
            }
        }
    }

    #[test]
    fn cars_route_around_a_queue_the_lanes_measured() {
        let (graph, mut network) = two_ways();
        network.refresh_congestion(&graph);
        assert_eq!(car_route(&network, &graph), [0, 1, 2]);
        // A queue towards B over T; the opposite lanes stay free.
        delay(&mut network, 1, true, 40.0);
        delay(&mut network, 0, false, 0.0);
        network.launch_congestion(&graph, 10, true);
        assert_eq!(
            car_route(&network, &graph),
            [0, 1, 2],
            "adopted only at its slot"
        );
        let revisions = (
            network.cch_graph.build_generation,
            network.cch_graph.metric_revision,
        );
        network.adopt_congestion(10);
        assert_eq!(car_route(&network, &graph), [0, 3, 2]);
        assert_eq!(
            (
                network.cch_graph.build_generation,
                network.cch_graph.metric_revision
            ),
            revisions
        );
        let weights = network.cch_graph.congestion_weights().unwrap();
        assert_eq!(weights[2], graph.edge(1).base_cost + 40.0);
        assert_eq!(weights[3], graph.edge(1).base_cost);
    }

    #[test]
    fn a_job_and_a_synchronous_customization_adopt_the_same_bits() {
        let (graph, mut network) = two_ways();
        network.refresh_congestion(&graph);
        delay(&mut network, 1, true, 17.5);
        delay(&mut network, 2, false, 3.25);
        let mut adopted = Vec::new();
        for synchronous in [true, false] {
            network.launch_congestion(&graph, 7, synchronous);
            if let Some(completion) = network.congestion_adoption_completion(7) {
                completion.wait();
            }
            network.adopt_congestion(7);
            adopted.push(network.cch_graph.congestion_weights().unwrap().to_vec());
            assert!(network.pending_congestion().is_none());
        }
        assert_eq!(adopted[0], adopted[1]);
    }

    #[test]
    fn a_rebuild_drops_a_pending_customization_and_refreshes_from_the_lanes() {
        let (graph, mut network) = two_ways();
        network.refresh_congestion(&graph);
        delay(&mut network, 1, true, 40.0);
        network.launch_congestion(&graph, 5, false);
        network.cch_graph = CchGraph::build(&graph);
        let revision = network.cch_graph.congestion_revision;
        network.adopt_congestion(5);
        assert_eq!(network.cch_graph.congestion_revision, revision);
        assert!(network.cch_graph.congestion_weights().is_none());
        network.refresh_congestion(&graph);
        assert_eq!(car_route(&network, &graph), [0, 3, 2]);
    }
}
