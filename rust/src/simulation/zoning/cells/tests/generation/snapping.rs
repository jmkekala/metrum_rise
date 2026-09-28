// SPDX-License-Identifier: GPL-2.0-only

//! Zoning-aware road construction, curb spacing, and bounded cursor query costs.

use super::*;

#[test]
fn snapped_roads_close_a_complete_rectangular_grid() {
    for angle in [0.0_f64, 0.35, -0.55] {
        for width in [7.0, 10.5, 14.0] {
            for cell_m in [8.0, 10.0] {
                let config = WorldConfig {
                    zone_cell_m: cell_m,
                    ..Default::default()
                };
                let origin = DVec2::new(-5000.0, 2500.0);
                let u = DVec2::new(angle.cos(), angle.sin());
                let v = u.perp();
                let half = width * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH);
                let span = 12.0 * f64::from(cell_m) + 2.0 * half;
                let mut graph = RegionGraph::new();
                let mut store = CellStore::default();
                let start = node(&mut graph, origin.x as f32, origin.y as f32);
                let mut previous = start;
                for (index, desired) in [
                    origin + u * span,
                    origin + (u + v) * span,
                    origin + v * span,
                    origin,
                ]
                .into_iter()
                .enumerate()
                {
                    let previous_pos = graph.node(previous).pos;
                    // Rotated fixtures start from a freely drawn reference road; the cardinal
                    // fixtures exercise snapped placement from the very first edge.
                    let cursor = if index == 0 && angle != 0.0 {
                        desired
                    } else {
                        store.road_grid_snap(f64::from(cell_m)).snap_road_cursor(
                            &graph,
                            desired + u * 0.7 + v * 0.4,
                            Some(DVec2::new(
                                f64::from(previous_pos.x),
                                f64::from(previous_pos.z),
                            )),
                            width,
                        )
                    };
                    assert!(
                        cursor.distance(desired) < 0.003,
                        "corner {index}: {cursor:?} != {desired:?}, angle={angle}, width={width}, cell={cell_m}"
                    );
                    let next = if index == 3 {
                        start
                    } else {
                        node(&mut graph, cursor.x as f32, cursor.y as f32)
                    };
                    let id = road(&mut graph, previous, next);
                    graph.edge_mut(id).width = width as f32;
                    store.refresh_road_alignments(&graph, &config, [id]);
                    previous = next;
                }
                let bounds =
                    CellBounds::from_points([origin, origin + (u + v) * span]).expanded(span);
                store.generate_in_bounds(&graph, &config, bounds, |_| false);
                for x in 0..12 {
                    for y in 0..12 {
                        let centre = origin
                            + u * (half + (x as f64 + 0.5) * f64::from(cell_m))
                            + v * (half + (y as f64 + 0.5) * f64::from(cell_m));
                        assert!(
                            store.pick(centre).is_some(),
                            "missing interior cell {x},{y}, angle={angle}, width={width}, cell={cell_m}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn zoning_snap_accounts_for_both_road_widths_and_keeps_branch_start_on_road() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, -200.0, 0.0);
    let b = node(&mut graph, 200.0, 0.0);
    road(&mut graph, a, b);
    let config = WorldConfig::default();
    let mut store = CellStore::default();
    store.refresh_road_alignments(&graph, &config, [0]);
    for width in [3.5, 7.0, 14.0] {
        let half = width * 0.5 + f64::from(crate::config::SIDEWALK_WIDTH);
        let branch =
            store
                .road_grid_snap(10.0)
                .snap_road_cursor(&graph, DVec2::new(63.2, 1.0), None, width);
        assert_eq!(branch.y, 0.0);
        let nearest_curb = (branch.x - half) / 10.0;
        let other_curb = (branch.x + half) / 10.0;
        assert!(
            (nearest_curb - nearest_curb.round()).abs() < 1e-6
                || (other_curb - other_curb.round()).abs() < 1e-6
        );
        for sign in [-1.0, 1.0] {
            let expected = sign * (120.0 + 5.0 + half);
            let snapped = store.road_grid_snap(10.0).snap_road_cursor(
                &graph,
                DVec2::new(branch.x + 0.4, expected + 0.8),
                Some(branch),
                width,
            );
            assert!((snapped.y - expected).abs() < 1e-6);
            assert!((snapped.x - branch.x).abs() < 1e-6);
        }
    }
}

#[test]
fn zoning_snap_free_terrain_uses_configured_cell_length() {
    let graph = RegionGraph::new();
    let store = CellStore::default();
    let start = DVec2::new(3.0, 7.0);
    assert_eq!(
        store
            .road_grid_snap(8.0)
            .snap_road_cursor(&graph, start, None, 7.0),
        start
    );
    assert_eq!(
        store.road_grid_snap(8.0).snap_road_cursor(
            &graph,
            start + DVec2::new(81.0, 2.0),
            Some(start),
            7.0
        ),
        start + DVec2::new(82.0, 0.0)
    );
}

#[test]
fn zoning_snap_snapshot_retains_its_grid_after_road_edits() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, -200.0, 0.0);
    let b = node(&mut graph, 200.0, 0.0);
    road(&mut graph, a, b);
    let config = WorldConfig::default();
    let mut store = CellStore::default();
    store.refresh_road_alignments(&graph, &config, [0]);
    let old = store.road_grid_snap(10.0);
    let old_graph = graph.clone();
    graph.edge_mut(0).width = 14.0;
    store.refresh_road_alignments(&graph, &config, [0]);
    let cursor = DVec2::new(65.0, 133.0);
    let start = Some(DVec2::new(65.0, 0.0));
    assert_eq!(
        old.snap_road_cursor(&old_graph, cursor, start, 7.0).y,
        130.0
    );
    assert_eq!(
        store
            .road_grid_snap(10.0)
            .snap_road_cursor(&graph, cursor, start, 7.0)
            .y,
        133.5
    );
}

#[test]
fn zoning_snap_captures_near_axes_and_releases_other_angles() {
    for reference_angle in [None, Some(0.0_f64), Some(0.35), Some(-0.55)] {
        let angle = reference_angle.unwrap_or(0.0);
        let u = DVec2::new(angle.cos(), angle.sin());
        let v = u.perp();
        let mut graph = RegionGraph::new();
        let mut store = CellStore::default();
        if reference_angle.is_some() {
            let a = node(&mut graph, (-200.0 * u.x) as f32, (-200.0 * u.y) as f32);
            let b = node(&mut graph, (200.0 * u.x) as f32, (200.0 * u.y) as f32);
            road(&mut graph, a, b);
            store.refresh_road_alignments(&graph, &WorldConfig::default(), [0]);
        }
        let snap = store.road_grid_snap(10.0);
        let start = u * 65.0;
        for quarter in 0..4 {
            let axis = [u, v, -u, -v][quarter];
            let across = axis.perp();
            for sign in [-1.0, 1.0] {
                let near =
                    start + axis * 40.0 + across * (40.0 * 4.0_f64.to_radians().tan() * sign);
                let result = snap.snap_road_cursor(&graph, near, Some(start), 7.0);
                assert!(
                    result.distance(start + axis * 40.0) < 0.001,
                    "near-axis capture: {reference_angle:?}, {quarter}, {sign}"
                );
                for degrees in [6.0_f64, 15.0, 30.0, 45.0, 60.0, 75.0, 84.0] {
                    let radians = degrees.to_radians() * sign;
                    let cursor = start + (axis * radians.cos() + across * radians.sin()) * 40.0;
                    assert_eq!(
                        snap.snap_road_cursor(&graph, cursor, Some(start), 7.0),
                        cursor,
                        "free angle: {reference_angle:?}, {quarter}, {degrees}, {sign}"
                    );
                }
                // Long strokes must not jump sideways even within the angular capture cone.
                let far = start + axis * 1000.0 + across * (5.1 * sign);
                assert_eq!(snap.snap_road_cursor(&graph, far, Some(start), 7.0), far);
            }
        }
    }
}

#[test]
#[ignore = "release-only locality diagnostic; run without competing workloads"]
fn benchmark_zoning_snap_locality() {
    use std::{hint::black_box, time::Instant};
    let config = WorldConfig::default();
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, -200.0, 0.0);
    let b = node(&mut graph, 200.0, 0.0);
    road(&mut graph, a, b);
    let mut store = CellStore::default();
    store.refresh_road_alignments(&graph, &config, [0]);
    let mut previous = 0;
    let mut expected = None;
    for count in [0, 1000, 10_000] {
        for i in previous..count {
            let x = 2000.0 + (i % 100) as f32 * 50.0;
            let z = 2000.0 + (i / 100) as f32 * 50.0;
            let a = node(&mut graph, x, z);
            let b = node(&mut graph, x + 25.0, z);
            let id = road(&mut graph, a, b);
            store.refresh_road_alignments(&graph, &config, [id]);
        }
        previous = count;
        let snapshot = store.road_grid_snap(10.0);
        let mut samples = Vec::new();
        for _ in 0..21 {
            let timer = Instant::now();
            let mut sum = DVec2::ZERO;
            for i in 0..4096 {
                sum += black_box(snapshot.snap_road_cursor(
                    black_box(&graph),
                    DVec2::new(65.0 + (i % 7) as f64 * 0.1, 130.0 + (i % 11) as f64 * 0.1),
                    Some(DVec2::new(65.0, 0.0)),
                    7.0,
                ));
            }
            samples.push(timer.elapsed().as_secs_f64() * 1e6 / 4096.0);
            assert_eq!(*expected.get_or_insert(sum), sum);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "zoning_snap background_roads={count} median_us={:.3} product={:?}",
            samples[10],
            expected.unwrap()
        );
    }
}
