// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: reroute.rs
//  script_path: rust/src/simulation/economy/agents/tick/planning/reroute.rs
//  module_name: reroute
//  version: 0.1.0
//  description: En-route rerouting against observed congestion. A route
//  kind: module
//  spec: docs/traffic.md
//  internal_dependencies: [simulation/network/graph/data.rs]
//  external_dependencies: []
//  features: [congestion-reroute, improvement-threshold, rate-limit]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! En-route rerouting against observed congestion.
//!
//! A route chosen at departure is a prediction. Without a way to revise it, a
//! jam cannot influence the traffic that feeds it, so congestion never pushes
//! back and the network has no negative feedback. `traffic.md` names that as
//! the single most important requirement in the traffic model, and this module
//! is the half of the loop that was missing: congestion was already measured
//! per tick and already priced into the router's metric, but a vehicle holding
//! a valid path never asked the question again.
//!
//! Two rules keep this from replacing one failure with another.
//!
//! A switch requires a real improvement. [`REROUTE_IMPROVEMENT_FRACTION`] is
//! the margin the new route must beat the old one by. Without it, two routes
//! within rounding distance of each other trade vehicles back and forth every
//! time they are compared, and the oscillation is worse than the jam.
//!
//! A vehicle reconsiders at a bounded rate. [`REROUTE_INTERVAL_S`] spaces the
//! attempts out, so the cost is a periodic comparison rather than a pathfind
//! per vehicle per tick.

use crate::simulation::network::graph::RegionGraph;

// ========================================================================
// THRESHOLDS
// ========================================================================

/// Fractional improvement a candidate route must show before a vehicle takes it.
///
/// A vehicle switches when the new route costs less than 85% of the remainder of
/// its current one. The retrofit mod that added this behavior to Cities:
/// Skylines used roughly the same margin, and the shape matters more than the
/// exact figure: too low and vehicles oscillate between equivalent routes, too
/// high and they sit in jams they could have avoided.
pub(in crate::simulation::economy::agents::tick) const REROUTE_IMPROVEMENT_FRACTION: f32 = 0.85;

/// Seconds between rerouting attempts for one vehicle.
pub(in crate::simulation::economy::agents::tick) const REROUTE_INTERVAL_S: f32 = 15.0;

// ========================================================================
// PRICING
// ========================================================================

/// Prices a node path against the live graph, congestion included.
///
/// Uses the same metric the contraction hierarchy customizes with,
/// `base_cost * (1.0 + current_congestion)`, so a path priced here and a path
/// costed by the router are comparable. Pricing the remainder with the
/// free-flow cost instead would make every congested route look cheap and the
/// comparison meaningless.
///
/// Returns `None` when any leg of the path has no edge between its endpoints,
/// which means the path has been invalidated by an edit and is not a candidate
/// for comparison at all.
pub(in crate::simulation::economy::agents::tick) fn price_node_path(
    path: &[u32],
    from_idx: usize,
    graph: &RegionGraph,
) -> Option<f32> {
    if from_idx >= path.len() {
        return Some(0.0);
    }

    let mut total = 0.0_f32;
    for pair in path[from_idx..].windows(2) {
        let edge_id = graph.get_edge_between_nodes(pair[0], pair[1])?;
        let edge = graph.edge(edge_id);
        if edge.deleted {
            return None;
        }
        total += edge.base_cost * (1.0 + edge.current_congestion);
    }
    Some(total)
}

/// Whether `candidate_cost` beats `current_cost` by enough to be worth taking.
///
/// A current cost of zero or less admits nothing: there is no remaining route
/// to improve on, so switching cannot help.
#[inline]
pub(in crate::simulation::economy::agents::tick) fn reroute_is_worthwhile(
    current_cost: f32,
    candidate_cost: f32,
) -> bool {
    if !(current_cost > 0.0) || !candidate_cost.is_finite() {
        return false;
    }
    candidate_cost < current_cost * REROUTE_IMPROVEMENT_FRACTION
}

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marginal_gain_does_not_trigger_a_switch() {
        // 5% better is inside the margin: taking it would let two near-equal
        // routes trade vehicles back and forth.
        assert!(!reroute_is_worthwhile(100.0, 95.0));
        assert!(!reroute_is_worthwhile(100.0, 86.0));
    }

    #[test]
    fn a_real_gain_triggers_a_switch() {
        assert!(reroute_is_worthwhile(100.0, 84.0));
        assert!(reroute_is_worthwhile(100.0, 10.0));
    }

    #[test]
    fn a_worse_route_is_never_taken() {
        assert!(!reroute_is_worthwhile(100.0, 100.0));
        assert!(!reroute_is_worthwhile(100.0, 250.0));
    }

    #[test]
    fn a_finished_route_admits_nothing() {
        assert!(!reroute_is_worthwhile(0.0, 0.0));
        assert!(!reroute_is_worthwhile(-1.0, 0.5));
    }

    #[test]
    fn nonfinite_candidates_are_rejected() {
        assert!(!reroute_is_worthwhile(100.0, f32::NAN));
        assert!(!reroute_is_worthwhile(100.0, f32::INFINITY));
    }

    #[test]
    fn the_margin_is_the_documented_one() {
        // The threshold is a stated requirement, not a tuning constant to drift.
        assert_eq!(REROUTE_IMPROVEMENT_FRACTION, 0.85);
        assert!(reroute_is_worthwhile(1.0, REROUTE_IMPROVEMENT_FRACTION - 0.001));
        assert!(!reroute_is_worthwhile(1.0, REROUTE_IMPROVEMENT_FRACTION + 0.001));
    }
}
