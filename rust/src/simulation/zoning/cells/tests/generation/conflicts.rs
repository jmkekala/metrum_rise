// SPDX-License-Identifier: GPL-2.0-only

//! Conflict pruning preserves reservations, publication seams and precise road contacts.

use super::*;

#[test]
fn partial_generation_keeps_untouched_empty_and_reserved_conflicts() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    road(&mut graph, a, b);
    for reserved in [false, true] {
        let mut store = CellStore::default();
        let grid =
            store.register_frame(GridFrame::new(DVec2::new(34.0, 5.0), DVec2::X, 10.0).unwrap());
        let retained = CellKey { grid, x: 3, y: 0 };
        store.insert(retained);
        if reserved {
            let selection = store.select(CellSelectionShape::Cell, &[DVec2::new(39.0, 10.0)]);
            store.paint(&selection, 1).unwrap();
        }
        // Unreserved geometry is outside this tiny publication area, but intersects the
        // crossing candidate [30,40] x [5,15]. The larger request exercises the mask path.
        let bounds = if reserved {
            extent()
        } else {
            CellBounds {
                min: DVec2::new(30.5, 6.0),
                max: DVec2::new(31.0, 14.0),
            }
        };
        store.generate_in_bounds(&graph, &WorldConfig::default(), bounds, |_| false);
        assert_eq!(store.profile(retained), Some(u16::from(reserved)));
        // Retained paint may align replacement candidates to its own frame. The
        // original conflicting square must be absent; compatible neighbors may remain.
        let rejected_frame = GridFrame::new(DVec2::new(0.0, 5.0), DVec2::X, 10.0).unwrap();
        assert!(keys(&store, extent()).into_iter().all(|key| {
            store.frame(key.grid) != Some(rejected_frame) || key.x != 3 || key.y != 0
        }));
        assert_disjoint(&store, extent());
    }
}

#[test]
fn corridor_bounds_pruning_matches_all_segment_contacts() {
    use crate::simulation::zoning::cells::generation::overlaps_road_corridors;
    use crate::simulation::zoning::cells::geometry::road_source_precision;

    for angle in [0.0_f64, 0.35, 1.2] {
        let u = DVec2::new(angle.cos(), angle.sin());
        let v = u.perp();
        for offset in [DVec2::ZERO, DVec2::new(8192.0, -8192.0)] {
            let points: Vec<_> = (-20..=20)
                .map(|i| {
                    let p = offset + u * (i as f64 * 8.0) + v * (i as f64 * 0.05).sin() * 20.0;
                    Vector3::new(p.x as f32, 0.0, p.y as f32)
                })
                .collect();
            let mut graph = RegionGraph::new();
            let a = node(&mut graph, points[0].x, points[0].z);
            let last = points.last().unwrap();
            let b = node(&mut graph, last.x, last.z);
            let id = polyline(&mut graph, a, b, points);
            let edge = graph.edge(id);
            let half_width = f64::from(edge.width) * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH);
            for x in -18..=18 {
                for distance in [
                    -30.0, -5.000001, -5.0, -4.999999, 0.0, 4.999999, 5.0, 5.000001, 30.0,
                ] {
                    let origin = offset + u * (x as f64 * 10.0) + v * distance;
                    let frame = GridFrame::new(origin, u, 10.0).unwrap();
                    let local = frame.local(origin).round();
                    let corners = frame.corners(local.x as i32, local.y as i32);
                    // Independent exhaustive traversal intentionally has no spatial rejection.
                    let expected = edge.physical_geometry.windows(2).any(|segment| {
                        let a = DVec2::new(f64::from(segment[0].x), f64::from(segment[0].z));
                        let b = DVec2::new(f64::from(segment[1].x), f64::from(segment[1].z));
                        let precision = road_source_precision(CellBounds::from_points(
                            corners.iter().copied().chain([a, b]),
                        ));
                        let normal =
                            (b - a).normalize_or_zero().perp() * (half_width - precision).max(0.0);
                        normal.length_squared() > 0.0
                            && interiors_overlap(
                                &corners,
                                &[a - normal, b - normal, b + normal, a + normal],
                            )
                    });
                    assert_eq!(
                        overlaps_road_corridors(&graph, &corners),
                        expected,
                        "angle={angle} offset={offset:?} x={x} distance={distance}"
                    );
                }
            }
        }
    }
}
