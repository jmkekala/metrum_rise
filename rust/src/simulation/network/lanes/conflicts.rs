// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: conflicts.rs
//  script_path: rust/src/simulation/network/lanes/conflicts.rs
//  module_name: conflicts
//  version: 0.1.0
//  description: Which connector movements through a junction cross each
//  kind: module
//  spec: docs/roads.md
//  internal_dependencies: [simulation/network/lanes/mod.rs, config]
//  external_dependencies: [godot]
//  features: [junction-conflicts, co-entrants, permissive-turns]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Which connector movements through a junction conflict with each other.
//!
//! Every permitted turn at a node is its own connector lane, and each lane is
//! separated only along its own length. Two connectors whose paths cross are
//! different lanes, so nothing tests one against the other: a left-turning car
//! and the oncoming through car each see their own lane clear and both proceed.
//! That is a left turn driven into traffic, and at volume it is two cars
//! occupying the same point.
//!
//! A conflict is geometric. Two connectors conflict when their paths pass
//! within a car's width of each other somewhere other than a shared endpoint.
//! Sharing a start means they leave the same approach lane, where ordinary
//! car-following already applies; sharing an end means they merge, which is a
//! different problem with a different rule.
//!
//! The result is a per-node table computed once when lanes are rebuilt, so the
//! movement hot path asks a question rather than doing geometry.

use super::Lane;
use crate::config::CAR_LENGTH;
use std::collections::HashMap;

/// Lateral clearance below which two connector paths are treated as crossing.
///
/// A car is [`CAR_LENGTH`] long and roughly two thirds that wide. Two paths
/// closer than this cannot both be occupied at the same longitudinal position
/// without the vehicles overlapping.
pub const CONFLICT_CLEARANCE_M: f32 = CAR_LENGTH * 0.75;

/// Endpoint agreement below which two connectors are treated as sharing it.
const SHARED_ENDPOINT_M: f32 = 0.5;

// ========================================================================
// THE TABLE
// ========================================================================

/// The conflicting movements at one junction.
///
/// Keyed by lane id, holding the lane ids that movement must not overlap with.
/// Symmetric: if A conflicts with B then B conflicts with A.
///
/// Two kinds are recorded. `conflicts` are crossing paths, where a car already
/// inside the box blocks a movement that would cut across it. `co_entrants` are
/// movements that begin at the same point: every turn out of one approach lane
/// starts where that lane ends, so a car sitting at distance zero on any of them
/// occupies the same ground as a car at distance zero on its neighbor. Without
/// that second table, three cars choosing three different turns out of one lane
/// all park on the identical point and render inside each other.
#[derive(Clone, Debug, Default)]
pub struct JunctionConflicts {
    conflicts: HashMap<usize, Vec<usize>>,
    co_entrants: HashMap<usize, Vec<usize>>,
}

impl JunctionConflicts {
    /// Lanes that conflict with `lane_id`, or an empty slice when none do.
    #[inline]
    pub fn conflicting(&self, lane_id: usize) -> &[usize] {
        self.conflicts
            .get(&lane_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Movements starting at the same point as `lane_id`.
    ///
    /// A car at distance zero on any of these stands on the same ground, so
    /// entry must be tested against all of them rather than one bucket.
    #[inline]
    pub fn co_entrants(&self, lane_id: usize) -> &[usize] {
        self.co_entrants
            .get(&lane_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Whether any conflicting movement exists for `lane_id`.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.conflicts.is_empty() && self.co_entrants.is_empty()
    }

    /// Number of lanes carrying at least one conflict.
    #[inline]
    pub fn len(&self) -> usize {
        self.conflicts.len()
    }
}

// ========================================================================
// GEOMETRY
// ========================================================================

/// Squared distance from point `p` to segment `ab`, in the XZ plane.
///
/// Height is ignored because two connectors at different heights are a bridge
/// or a tunnel, and those are separate `EdgeClass` values that do not conflict.
fn point_segment_dist2_xz(px: f32, pz: f32, ax: f32, az: f32, bx: f32, bz: f32) -> f32 {
    let abx = bx - ax;
    let abz = bz - az;
    let len2 = abx * abx + abz * abz;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((px - ax) * abx + (pz - az) * abz) / len2).clamp(0.0, 1.0)
    };
    let cx = ax + abx * t;
    let cz = az + abz * t;
    let dx = px - cx;
    let dz = pz - cz;
    dx * dx + dz * dz
}

/// Whether two segments properly cross in the XZ plane.
///
/// Vertex-to-segment distance alone misses this case entirely. Two straight
/// connectors meeting in an X have every vertex far from the other's path, and
/// only the interiors touch, so measuring vertices reports them as clear when
/// they intersect head on.
fn segments_cross_xz(
    a0: godot::prelude::Vector3,
    a1: godot::prelude::Vector3,
    b0: godot::prelude::Vector3,
    b1: godot::prelude::Vector3,
) -> bool {
    let orient = |px: f32, pz: f32, qx: f32, qz: f32, rx: f32, rz: f32| -> f32 {
        (qx - px) * (rz - pz) - (qz - pz) * (rx - px)
    };
    let d1 = orient(a0.x, a0.z, a1.x, a1.z, b0.x, b0.z);
    let d2 = orient(a0.x, a0.z, a1.x, a1.z, b1.x, b1.z);
    let d3 = orient(b0.x, b0.z, b1.x, b1.z, a0.x, a0.z);
    let d4 = orient(b0.x, b0.z, b1.x, b1.z, a1.x, a1.z);
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

/// Closest approach between two polylines in the XZ plane.
///
/// Returns 0 when the paths actually intersect, otherwise the smallest gap
/// between them.
fn polyline_min_dist_xz(a: &[godot::prelude::Vector3], b: &[godot::prelude::Vector3]) -> f32 {
    for wa in a.windows(2) {
        for wb in b.windows(2) {
            if segments_cross_xz(wa[0], wa[1], wb[0], wb[1]) {
                return 0.0;
            }
        }
    }

    let mut best = f32::MAX;
    for pa in a {
        for wb in b.windows(2) {
            let d2 = point_segment_dist2_xz(pa.x, pa.z, wb[0].x, wb[0].z, wb[1].x, wb[1].z);
            if d2 < best {
                best = d2;
            }
        }
    }
    for pb in b {
        for wa in a.windows(2) {
            let d2 = point_segment_dist2_xz(pb.x, pb.z, wa[0].x, wa[0].z, wa[1].x, wa[1].z);
            if d2 < best {
                best = d2;
            }
        }
    }
    best.sqrt()
}

// ========================================================================
// STREET GROUPING
// ========================================================================

/// Heading a connector enters and leaves the junction on, normalized in XZ.
fn entry_exit_bearings(l: &Lane) -> Option<((f32, f32), (f32, f32))> {
    let g = &l.geometry;
    if g.len() < 2 {
        return None;
    }
    let norm = |dx: f32, dz: f32| -> Option<(f32, f32)> {
        let m = (dx * dx + dz * dz).sqrt();
        if m < 1e-4 { None } else { Some((dx / m, dz / m)) }
    };
    let entry = norm(g[1].x - g[0].x, g[1].z - g[0].z)?;
    let n = g.len();
    let exit = norm(g[n - 1].x - g[n - 2].x, g[n - 1].z - g[n - 2].z)?;
    Some((entry, exit))
}

/// Whether two movements arrive on the same street.
///
/// A signal gives green to one street, and every movement *arriving* on it goes:
/// through traffic both ways, plus the turns off it. A northbound left turn is
/// green at the same time as the southbound through lane it crosses, and the
/// turning driver yields by gap acceptance rather than by being held out of the
/// junction. Grouping by the approach bearing captures exactly that set.
///
/// Movements arriving on the crossing street are the ones a signal separates,
/// and the ones that produce cars driving through each other when nothing does.
fn same_street(a: &Lane, b: &Lane) -> bool {
    let (Some((ae, _)), Some((be, _))) = (entry_exit_bearings(a), entry_exit_bearings(b)) else {
        return false;
    };
    // Parallel or antiparallel approach headings have |dot| near 1, so a
    // northbound and a southbound approach group together.
    (ae.0 * be.0 + ae.1 * be.1).abs() > 0.85
}

fn near_xz(p: &godot::prelude::Vector3, q: &godot::prelude::Vector3) -> bool {
    let dx = p.x - q.x;
    let dz = p.z - q.z;
    (dx * dx + dz * dz).sqrt() < SHARED_ENDPOINT_M
}

/// Whether two connectors begin at the same point.
///
/// Every turn out of one approach lane starts where that lane ends, so all of
/// them share a start. Cars waiting to take different turns queue on the same
/// ground.
fn shares_start(a: &Lane, b: &Lane) -> bool {
    match (a.geometry.first(), b.geometry.first()) {
        (Some(x), Some(y)) => near_xz(x, y),
        _ => false,
    }
}

/// Whether two connectors end at the same point, which is a merge.
fn shares_end(a: &Lane, b: &Lane) -> bool {
    match (a.geometry.last(), b.geometry.last()) {
        (Some(x), Some(y)) => near_xz(x, y),
        _ => false,
    }
}

// ========================================================================
// BUILDING THE TABLE
// ========================================================================

/// Builds the conflict table for the connectors at one junction.
///
/// `connector_lane_ids` are the vehicle connectors at the node.
///
/// The rule a real signal follows: one road runs at a time, both of its
/// directions together. So two movements that belong to the same road never
/// conflict, even where their paths cross inside the box. A northbound left
/// turn crosses the southbound through lane on paper, and in a real
/// intersection both are green in the same phase; the turning driver yields by
/// gap acceptance rather than by being held out of the junction.
///
/// What does conflict is two movements from *different* roads. That is the
/// pair a signal separates and the pair that produces cars driving through each
/// other when nothing separates them.
pub fn build_junction_conflicts(connector_lane_ids: &[usize], lanes: &[Lane]) -> JunctionConflicts {
    let mut conflicts: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut co_entrants: HashMap<usize, Vec<usize>> = HashMap::new();

    for (i, &la) in connector_lane_ids.iter().enumerate() {
        let Some(a) = lanes.get(la) else { continue };
        if a.geometry.len() < 2 {
            continue;
        }
        for &lb in connector_lane_ids.iter().skip(i + 1) {
            let Some(b) = lanes.get(lb) else { continue };
            if b.geometry.len() < 2 {
                continue;
            }

            // Movements leaving the same point stand on the same ground while
            // they wait, whatever else is true about them. Recorded first,
            // because it applies even to same-street turns a signal runs
            // together: two cars cannot both sit at the mouth of one lane.
            if shares_start(a, b) {
                co_entrants.entry(la).or_default().push(lb);
                co_entrants.entry(lb).or_default().push(la);
                continue;
            }

            // Same street, either direction: a signal runs these together.
            //
            // A cross junction splits one street into two edges at the node, so
            // edge id cannot identify a street. Bearing can: the two arms of one
            // street leave the junction on opposite headings, and a movement
            // between them stays on that street whichever way it runs.
            if same_street(a, b) {
                continue;
            }

            // Two movements into the same exit merge, which is governed by gap
            // acceptance on the exit lane rather than by yielding in the box.
            if shares_end(a, b) {
                continue;
            }

            if polyline_min_dist_xz(&a.geometry, &b.geometry) < CONFLICT_CLEARANCE_M {
                conflicts.entry(la).or_default().push(lb);
                conflicts.entry(lb).or_default().push(la);
            }
        }
    }

    for v in conflicts.values_mut() {
        v.sort_unstable();
        v.dedup();
    }
    for v in co_entrants.values_mut() {
        v.sort_unstable();
        v.dedup();
    }

    JunctionConflicts {
        conflicts,
        co_entrants,
    }
}

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::lanes::LaneType;
    use godot::prelude::Vector3;

    fn connector(id_geom: Vec<Vector3>) -> Lane {
        let mut l = Lane {
            edge_id: usize::MAX,
            node_id: 0,
            lane_type: LaneType::Vehicle,
            ..Default::default()
        };
        l.geometry = id_geom;
        l
    }

    fn v(x: f32, z: f32) -> Vector3 {
        Vector3::new(x, 0.0, z)
    }

    #[test]
    fn crossing_movements_from_different_roads_conflict() {
        // East-west through and north-south through, from two different roads.
        // This is the pair a signal separates.
        let lanes = vec![
            connector(vec![v(-10.0, 0.0), v(10.0, 0.0)]),
            connector(vec![v(0.0, -10.0), v(0.0, 10.0)]),
        ];
        let c = build_junction_conflicts(&[0, 1], &lanes);
        assert_eq!(c.conflicting(0), &[1]);
        assert_eq!(c.conflicting(1), &[0]);
    }

    #[test]
    fn both_directions_of_one_road_run_together() {
        // The rule a real signal follows: green goes to one road, both of its
        // directions at once. A northbound left turn crosses the southbound
        // through lane on paper, and both are still green in the same phase.
        // Treating that as a conflict makes opposing lanes of one street wait
        // on each other, which no traffic light does.
        let left = connector(vec![v(1.5, -10.0), v(1.5, 0.0), v(-10.0, 1.5)]);
        let oncoming = connector(vec![v(-1.5, 10.0), v(-1.5, -10.0)]);
        let lanes = vec![left, oncoming];
        let c = build_junction_conflicts(&[0, 1], &lanes);
        assert!(
            c.conflicting(0).is_empty(),
            "same-road movements must not block each other"
        );
        assert!(c.conflicting(1).is_empty());
    }

    #[test]
    fn a_turn_conflicts_with_the_crossing_street() {
        // The same northbound left turn, now against traffic arriving on the
        // east-west street. Different approach, so the signal holds one of them
        // and this must be a conflict.
        let turning = connector(vec![v(1.5, -10.0), v(1.5, 0.0), v(-10.0, 1.5)]);
        let crossing = connector(vec![v(-10.0, -1.5), v(10.0, -1.5)]);
        let lanes = vec![turning, crossing];
        let c = build_junction_conflicts(&[0, 1], &lanes);
        assert_eq!(
            c.conflicting(0),
            &[1],
            "a turn must be held against the crossing street"
        );
    }

    #[test]
    fn parallel_movements_do_not_conflict() {
        // Both directions of one street, never crossing.
        let lanes = vec![
            connector(vec![v(-10.0, 1.8), v(10.0, 1.8)]),
            connector(vec![v(10.0, -1.8), v(-10.0, -1.8)]),
        ];
        let c = build_junction_conflicts(&[0, 1], &lanes);
        assert!(c.conflicting(0).is_empty());
        assert!(c.conflicting(1).is_empty());
    }

    #[test]
    fn movements_out_of_the_same_lane_are_co_entrants_not_crossings() {
        // A right turn and a through movement leaving the same approach lane
        // diverge, so neither has to yield to the other inside the box.
        let lanes = vec![
            connector(vec![v(0.0, -10.0), v(0.0, 10.0)]),
            connector(vec![v(0.0, -10.0), v(0.0, 0.0), v(10.0, 0.5)]),
        ];
        let c = build_junction_conflicts(&[0, 1], &lanes);
        assert!(
            c.conflicting(0).is_empty(),
            "a shared start is a diverge, not a crossing"
        );
        // But they start on the same ground, so only one car may wait there.
        assert_eq!(
            c.co_entrants(0),
            &[1],
            "two cars cannot both sit at the mouth of one lane"
        );
        assert_eq!(c.co_entrants(1), &[0]);
    }

    #[test]
    fn every_turn_out_of_one_lane_is_a_co_entrant_of_the_others() {
        // The stacking case, watched on screen: three cars pick three different
        // turns out of one approach and all park on the identical point,
        // rendering inside each other. Each connector must know about the other
        // two.
        let lanes = vec![
            connector(vec![v(0.0, -10.0), v(0.0, 10.0)]),
            connector(vec![v(0.0, -10.0), v(0.0, 0.0), v(10.0, 1.0)]),
            connector(vec![v(0.0, -10.0), v(0.0, 0.0), v(-10.0, 1.0)]),
        ];
        let c = build_junction_conflicts(&[0, 1, 2], &lanes);
        assert_eq!(c.co_entrants(0), &[1, 2]);
        assert_eq!(c.co_entrants(1), &[0, 2]);
        assert_eq!(c.co_entrants(2), &[0, 1]);
    }

    #[test]
    fn movements_into_the_same_exit_do_not_conflict() {
        // Two approaches merging into one exit. That is a merge, governed by
        // gap acceptance rather than by yielding inside the box.
        let lanes = vec![
            connector(vec![v(-10.0, 0.0), v(10.0, 0.0)]),
            connector(vec![v(0.0, -10.0), v(5.0, -2.0), v(10.0, 0.0)]),
        ];
        let c = build_junction_conflicts(&[0, 1], &lanes);
        assert!(c.conflicting(0).is_empty());
    }

    #[test]
    fn an_empty_junction_has_no_conflicts() {
        let c = build_junction_conflicts(&[], &[]);
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        assert!(c.conflicting(7).is_empty());
    }
}
