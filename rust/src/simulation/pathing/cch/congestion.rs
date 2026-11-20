// SPDX-License-Identifier: GPL-3.0-or-later

//! The car-congestion metric (`TRAFFIC-17`): a second car metric on the same filled topology,
//! customized from directed edge weights (free-flow cost plus measured lane delay) and adopted
//! beside the free-flow profiles.
//!
//! Only car route materialization reads it ([`RouteMetric::Congested`]); every economy quote,
//! sweep, bound and bucket query keeps the free-flow profiles and their `metric_revision`, so
//! adopting new weights reprices nothing. Its own `congestion_revision` keys only the retained
//! route sweeps. A customization is a pure function of the build's [`CongestionCustomizer`] and
//! the weights, so a background job and a synchronous call give the same bits.

use super::*;
use std::sync::Arc;

/// Which customized metric a route query reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RouteMetric {
    /// Free-flow edge costs: every economy quote, sweep, bound and planning query.
    FreeFlow,
    /// A car route's measured lane delay on top of free flow, once a congestion metric is
    /// adopted; free flow for other modes or before the first adoption.
    Congested,
}

/// Customization inputs fixed for one build, shared by the hierarchy and its congestion jobs.
pub(super) struct CustomizeInputs {
    pub(super) base_arcs: Vec<BaseArc>,
    pub(super) triangles: Vec<[usize; 3]>,
    // Physical length per edge at build, for the congestion metric's distances.
    pub(super) lengths: Vec<f32>,
}

/// A customized car-congestion metric and the directed weights it was customized from.
pub(crate) struct CongestionMetric {
    pub(super) metric: Metric,
    weights: Vec<f32>,
    build_generation: u32,
}

impl CongestionMetric {
    /// The directed weights, two per edge (forward, then backward): the free-flow cost plus
    /// the measured delay, the save's record of the adopted metric.
    pub(crate) fn weights(&self) -> &[f32] {
        &self.weights
    }

    /// Lends the weight storage to the next snapshot; the metric keeps its own storage for the
    /// next customization ([`CongestionCustomizer::customize`]'s `spare`). O(1).
    pub(crate) fn take_weights(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.weights)
    }
}

/// What a congestion customization reads besides its weights: one build's base transitions,
/// lower triangles and edge lengths, shared with the hierarchy in O(1).
#[derive(Clone)]
pub(crate) struct CongestionCustomizer {
    inputs: Arc<CustomizeInputs>,
    mask: u8,
    arcs: usize,
    build_generation: u32,
}

impl CongestionCustomizer {
    /// Number of directed weights a snapshot gives: two per edge (forward, backward).
    pub(crate) fn weight_count(&self) -> usize {
        self.inputs.lengths.len() * 2
    }

    /// Customizes the car metric from `weights` in O(base arcs + triangles), serially, so the
    /// result never depends on the thread or pool that runs it. `spare`, a metric this
    /// customizer's hierarchy displaced, lends its storage: with it no storage is allocated.
    pub(crate) fn customize(
        &self,
        weights: Vec<f32>,
        spare: Option<CongestionMetric>,
    ) -> CongestionMetric {
        assert_eq!(
            weights.len(),
            self.weight_count(),
            "one weight per edge direction"
        );
        let mut metric = match spare {
            Some(spare) if spare.metric.costs.len() == self.arcs => spare.metric,
            _ => Metric::empty(self.mask, self.arcs),
        };
        metric.mask = self.mask;
        let inputs = &self.inputs;
        metric.customize(inputs, |base| {
            (
                weights[2 * base.edge + usize::from(!base.forward)],
                inputs.lengths[base.edge],
            )
        });
        CongestionMetric {
            metric,
            weights,
            build_generation: self.build_generation,
        }
    }
}

impl CchGraph {
    /// What a congestion customization of this hierarchy reads, or `None` for a hierarchy with
    /// no car metric (unbuilt). O(1).
    pub(crate) fn congestion_customizer(&self) -> Option<CongestionCustomizer> {
        let car = self
            .profiles
            .get(self.profile_for_mask[TransitFlags::CAR as usize] as usize)?;
        Some(CongestionCustomizer {
            inputs: Arc::clone(&self.inputs),
            mask: car.mask,
            arcs: car.costs.len(),
            build_generation: self.build_generation,
        })
    }

    /// Makes `metric` the car-congestion metric car routes read, advancing only the
    /// congestion revision; no free-flow revision or economy key moves. Returns the displaced
    /// metric for reuse, or `metric` itself, unadopted, when it was customized on another
    /// build of the hierarchy (a routing rebuild superseded it). O(1).
    pub(crate) fn adopt_congestion(
        &mut self,
        metric: CongestionMetric,
    ) -> Option<CongestionMetric> {
        if metric.build_generation != self.build_generation {
            return Some(metric);
        }
        self.congestion_revision = self.congestion_revision.wrapping_add(1);
        self.congested.replace(metric)
    }

    /// The adopted congestion metric's directed weights, if any (save, tests).
    pub(crate) fn congestion_weights(&self) -> Option<&[f32]> {
        self.congested.as_ref().map(CongestionMetric::weights)
    }

    // The metric a query reads, the revision its retained sweeps key on, and the directed
    // weights a restricted origin seeds from (`None`: free-flow base costs).
    pub(super) fn selected(&self, mask: u8, route: RouteMetric) -> Selected<'_> {
        let profile = self.profile_for_mask[mask as usize];
        if route == RouteMetric::Congested
            && profile == self.profile_for_mask[TransitFlags::CAR as usize]
            && let Some(congested) = &self.congested
        {
            return Selected {
                metric: &congested.metric,
                key: (true, self.congestion_revision),
                weights: Some(&congested.weights),
            };
        }
        Selected {
            metric: &self.profiles[profile as usize],
            key: (false, self.metric_revision),
            weights: None,
        }
    }
}

/// A query's metric: see [`CchGraph::selected`].
pub(super) struct Selected<'a> {
    pub(super) metric: &'a Metric,
    // (congested, revision): retained sweeps of one metric never answer another's query.
    pub(super) key: (bool, u64),
    pub(super) weights: Option<&'a [f32]>,
}
