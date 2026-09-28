// SPDX-License-Identifier: GPL-2.0-only

//! Conflict pruning preserves reservations, publication seams and precise road contacts.

use super::*;

#[test]
fn blocked_middle_row_removes_rear_cells_but_preserves_paint() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    road(&mut graph, a, b);
    let config = WorldConfig::default();
    let mut store = CellStore::default();
    store.generate_in_bounds(&graph, &config, extent(), |_| false);
    let painted = store.select(CellSelectionShape::Cell, &[DVec2::new(25.0, 50.0)]);
    store.paint(&painted, 1).unwrap();
    let blocked = |corners: &[DVec2; 4]| {
        let centre = corners.iter().copied().sum::<DVec2>() * 0.25;
        centre.y > 25.0 && centre.y < 35.0
    };
    store.generate_in_bounds(&graph, &config, extent(), blocked);
    assert!(store.pick(DVec2::new(45.0, 20.0)).is_some());
    for y in [30.0, 40.0, 50.0, 60.0] {
        assert!(store.pick(DVec2::new(45.0, y)).is_none(), "row at {y}");
    }
    assert_eq!(store.profile(painted.cells[0]), Some(1));
    assert!(store.frontages(painted.cells[0]).is_empty());
    store.generate_in_bounds(&graph, &config, extent(), |_| false);
    assert!(store.pick(DVec2::new(45.0, 60.0)).is_some());
}

#[test]
fn competing_roads_offer_only_cells_in_frontage_connected_rectangles() {
    use std::collections::HashSet;
    for skew in [0.1, 40.0, -40.0] {
        let mut graph = RegionGraph::new();
        let a = node(&mut graph, -120.0, 0.0);
        let b = node(&mut graph, 120.0, 0.0);
        let c = node(&mut graph, skew, 120.0);
        road(&mut graph, a, b);
        road(&mut graph, a, c);
        let config = WorldConfig::default();
        let mut store = CellStore::default();
        store.generate_in_bounds(&graph, &config, extent(), |_| false);
        let cells = keys(&store, extent());
        let mut usable = HashSet::new();
        for &seed in &cells {
            for link in store.frontages(seed) {
                for depth in 1..=CELL_DEPTH as u8 {
                    let lot = CellLot::from_frontage(seed, 1, depth, link.boundary).unwrap();
                    if lot.cells().all(|key| store.profile(key).is_some()) {
                        usable.extend(lot.cells());
                    }
                }
            }
        }
        assert!(!usable.is_empty());
        assert!(cells.iter().all(|key| usable.contains(key)), "skew={skew}");
        assert_disjoint(&store, extent());
        // A cold, narrow request must inspect supporting rows outside its publication area.
        let bounds = CellBounds {
            min: DVec2::new(-45.0, 35.0),
            max: DVec2::new(-25.0, 55.0),
        };
        let mut local = CellStore::default();
        local.generate_in_bounds(&graph, &config, bounds, |_| false);
        let geometry = |store: &CellStore| {
            let mut result: Vec<_> = keys(store, bounds)
                .into_iter()
                .map(|key| (store.frame(key.grid).unwrap(), key.x, key.y))
                .collect();
            result.sort_unstable();
            result
        };
        assert_eq!(geometry(&local), geometry(&store));
    }
}

#[test]
fn painting_near_orthogonal_corner_keeps_the_displayed_layout() {
    for skew in [0.1, 1.0, -1.0] {
        let mut graph = RegionGraph::new();
        let centre = node(&mut graph, 0.0, 0.0);
        let east = node(&mut graph, 120.0, 0.0);
        let north = node(&mut graph, skew, 120.0);
        let edges = [
            road(&mut graph, centre, east),
            road(&mut graph, centre, north),
        ];
        let config = WorldConfig::default();
        let mut original = CellStore::default();
        original.refresh_road_alignments(&graph, &config, edges);
        original.generate_in_bounds(&graph, &config, extent(), |_| false);
        for (point, edge) in [
            (DVec2::new(40.0, 10.0), edges[0]),
            (DVec2::new(10.0, 40.0), edges[1]),
        ] {
            let key = original
                .pick(point)
                .expect("both roads retain front-row cells");
            assert!(
                original
                    .frontages(key)
                    .iter()
                    .any(|frontage| frontage.edge == edge),
                "each road retains its own frontage in the overlapping area"
            );
        }
        let before = keys(&original, extent());
        for grid in before
            .iter()
            .map(|key| key.grid)
            .collect::<std::collections::BTreeSet<_>>()
        {
            let mut store = original.clone();
            let selection = CellSelection {
                revision: store.revision(),
                cells: before
                    .iter()
                    .copied()
                    .filter(|key| key.grid == grid)
                    .collect(),
            };
            store.paint(&selection, 1).unwrap();
            store.generate_in_bounds(&graph, &config, extent(), |_| false);
            assert_eq!(keys(&store, extent()), before, "skew={skew}, grid={grid}");
            assert_disjoint(&store, extent());
            let erase = CellSelection {
                revision: store.revision(),
                cells: selection.cells,
            };
            store.paint(&erase, 0).unwrap();
            store.generate_in_bounds(&graph, &config, extent(), |_| false);
            assert_eq!(
                keys(&store, extent()),
                before,
                "erase skew={skew}, grid={grid}"
            );
        }
    }
}

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
