// SPDX-License-Identifier: GPL-2.0-only

//! The illegal geometry a road edit adds, which placement refuses (`ROAD-54`).
//!
//! Placement judges what the edit draws, not what it touches: a pre-existing bad junction does
//! not block an edit that leaves it no worse, so the player can still repair or extend it.

use super::*;

// Illegal checks placement refuses. Grade above the 1:3 maximum is refused (`ROAD-68` open
// question 37); grade change waits for the hill policy (`docs/roads.md`): the profile solver
// still follows bumpy ground, so refusing it would refuse ordinary roads over it. A sharp corner
// where two roads meet end to end is a street corner the bend piece rounds, so a direction kink
// stays a design warning.
fn refused(violation: &RoadGeometryViolation) -> bool {
    use RoadGeometryViolation::*;
    matches!(
        violation,
        Grade {
            tier: RoadGeometryTier::Illegal,
            ..
        } | HorizontalRadius {
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
/// drawn road's pieces); an illegal grade on `rederived`, the other edges whose profiles the
/// edit derives (the arms its node planes re-pin), each paired with the edge of `before` it
/// came from, only where that profile is steeper than before; and at `nodes`, each paired with
/// its id in `before` when it existed there, only where that node was not already as bad.
/// Deterministic: edges, re-derived edges, then nodes, each in the given order, the validator's
/// check order within each. O(points on `edges` and `rederived` and their sources + local
/// incidence of `nodes`).
pub fn edit_violation(
    after: &RegionGraph,
    edges: &[usize],
    rederived: &[(usize, Option<usize>)],
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
    for &(edge, source) in rederived {
        let Some(violation) = grade_violation(after, edge, limits) else {
            continue;
        };
        let RoadGeometryViolation::Grade {
            measured,
            tier: RoadGeometryTier::Illegal,
            ..
        } = violation
        else {
            continue;
        };
        // A profile that was already as steep is the existing road's, not this edit's.
        let old = source.and_then(|source| steepest_grade(centreline(before.edge(source))));
        if old.is_none_or(|old| measured > old.grade) {
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
