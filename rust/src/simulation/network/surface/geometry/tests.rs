// SPDX-License-Identifier: GPL-2.0-only

//! Low-level geometry regression tests.

use super::*;

#[test]
fn road_triangle_double_area_uses_xz_plane() {
    let area = RoadSurfaceSystem::road_triangle_double_area_xz_m2([
        RoadVec3::new(0.0, 8.0, 0.0),
        RoadVec3::new(2.0, -4.0, 0.0),
        RoadVec3::new(0.0, 2.0, 3.0),
    ]);

    assert_eq!(area, 6.0);
}

#[test]
fn triangle_has_area_xz_rejects_area_positive_needle_triangle() {
    let triangle = [
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(3.687, 0.0, 0.0),
        RoadVec3::new(0.0, 0.0, 0.000002826),
    ];

    assert!(
        RoadSurfaceSystem::road_triangle_double_area_xz_m2(triangle)
            > f64::from(NODE_OVERLAY_MIN_AREA_M2)
    );
    assert!(!RoadSurfaceSystem::triangle_has_area_xz(triangle));
}

#[test]
fn triangle_has_area_xz_accepts_stable_triangle() {
    assert!(RoadSurfaceSystem::triangle_has_area_xz([
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(2.0, 0.0, 0.0),
        RoadVec3::new(0.0, 0.0, 2.0),
    ]));
}

// A span outline: one side out along the road, the other back, with a gentle wave.
fn long_outline(points_per_side: usize, along_x: bool) -> Vec<RoadVec3> {
    let side = |index: usize, offset: f64| {
        let along = index as f64 * 2.0;
        let across = offset + (along * 0.05).sin() * 0.3;
        if along_x {
            RoadVec3::new(along, 0.0, across)
        } else {
            RoadVec3::new(across, 0.0, along)
        }
    };
    (0..points_per_side)
        .map(|index| side(index, -4.0))
        .chain((0..points_per_side).rev().map(|index| side(index, 4.0)))
        .collect()
}

#[test]
fn swept_strict_crossing_matches_all_pairs_on_long_loops() {
    let check = |points: &[RoadVec3]| {
        let swept = RoadSurfaceSystem::polygon_has_strict_edge_crossing_xz_swept(points);
        assert_eq!(
            swept,
            RoadSurfaceSystem::polygon_has_strict_edge_crossing_xz_all_pairs(points)
        );
        swept
    };
    for along_x in [true, false] {
        let mut outline = long_outline(300, along_x);
        assert!(!check(&outline));
        // Pull one point of the far side across the near side, 400 m along the span.
        let pulled = 2 * 300 - 1 - 200;
        if along_x {
            outline[pulled].z = -10.0;
        } else {
            outline[pulled].x = -10.0;
        }
        assert!(check(&outline));
    }
    // Random loops cross many times; random stars do not.
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    for _ in 0..20 {
        let scattered: Vec<RoadVec3> = (0..200)
            .map(|_| RoadVec3::new(next() * 100.0, 0.0, next() * 30.0))
            .collect();
        check(&scattered);
        let star: Vec<RoadVec3> = (0..200)
            .map(|index| {
                let angle = index as f64 / 200.0 * std::f64::consts::TAU;
                let radius = 20.0 + next() * 30.0;
                RoadVec3::new(radius * angle.cos(), 0.0, radius * angle.sin())
            })
            .collect();
        assert!(!check(&star));
    }
}
