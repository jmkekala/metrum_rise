// SPDX-License-Identifier: GPL-2.0-only

//! The illegal geometry a road edit adds, which placement refuses (`ROAD-54`).
//!
//! Placement judges what the edit draws, not what it touches: a pre-existing bad junction does
//! not block an edit that leaves it no worse, so the player can still repair or extend it.

use super::*;

// Illegal checks placement refuses. Grade and grade change wait for the hill policy
// (`docs/roads.md`): the profile solver still follows the terrain, so refusing them would refuse
// every steep hillside and every road over bumpy ground. A sharp corner where two roads meet end
// to end is a street corner the bend piece rounds, so a direction kink stays a design warning.
fn refused(violation: &RoadGeometryViolation) -> bool {
    use RoadGeometryViolation::*;
    matches!(
        violation,
        HorizontalRadius {
            tier: RoadGeometryTier::Illegal,
            ..
        } | JunctionArmAngle {
            tier: RoadGeometryTier::Illegal,
            ..
        } | JunctionDegree {
            tier: RoadGeometryTier::Illegal,
            ..
        }
    )
}

// Whether `after` at a node is worse than `before` was at that node's pre-edit self.
fn worse_than(after: &RoadGeometryViolation, before: &[RoadGeometryViolation]) -> bool {
    use RoadGeometryViolation::*;
    !before.iter().any(|old| match (after, old) {
        (JunctionArmAngle { measured: new, .. }, JunctionArmAngle { measured: old, .. }) => {
            // Rounding of the same arms' directions must not read as a new violation.
            new + 0.01 >= *old
        }
        (JunctionDegree { measured: new, .. }, JunctionDegree { measured: old, .. }) => new <= old,
        _ => false,
    })
}

/// The first geometry `after` refuses that the edit adds: any on the edit's own `edges` (the
/// drawn road's pieces), and at `nodes`, each paired with its id in `before` when it existed
/// there, only where that node was not already as bad. Deterministic: edges, then nodes, each in
/// the given order, the validator's check order within each. O(points on `edges` + local
/// incidence of `nodes`).
pub fn edit_violation(
    after: &RegionGraph,
    edges: &[usize],
    nodes: &[(u32, Option<u32>)],
    before: &RegionGraph,
    limits: &RoadGeometryLimits,
) -> Option<RoadGeometryViolation> {
    let mut found = Vec::new();
    for &edge in edges {
        validate_edge(after, edge, limits, &mut found);
        if let Some(violation) = found.drain(..).find(refused) {
            return Some(violation);
        }
    }
    let mut old = Vec::new();
    for &(node, source) in nodes {
        validate_node(after, node, limits, &mut found);
        old.clear();
        if let Some(source) = source {
            validate_node(before, source, limits, &mut old);
        }
        if let Some(violation) = found.drain(..).find(|v| refused(v) && worse_than(v, &old)) {
            return Some(violation);
        }
    }
    None
}
