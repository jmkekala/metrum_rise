// SPDX-License-Identifier: GPL-2.0-only

//! Both-side road zoning correctness and isolated editor locality measurements.

use super::super::helpers::{
    add_vertical_road_at_x, make_quarter_arc_road, make_straight_road, make_straight_road_span,
    make_zoning,
};
use crate::simulation::zoning::{ParcelPlacementError, parcels};

#[test]
fn road_zoning_endpoint_rounding_does_not_cancel_an_acute_junction() {
    use crate::simulation::network::types::NodeType;
    use godot::prelude::Vector3;

    let (mut graph, edge) = make_straight_road_span(0.0, 140.17);
    let mut branch = graph.edge(edge).clone();
    let end = Vector3::new(140.0, 0.0, 50.0);
    branch.end_node = graph.add_node(end, NodeType::Junction);
    branch.geometry = vec![Vector3::ZERO, end];
    branch.physical_geometry = branch.geometry.clone();
    branch.physical_length = end.length();
    branch.base_cost = branch.physical_length;
    graph.add_edge(branch);

    let length = graph.edge(edge).physical_length;
    assert!(
        (5.0 / length) * length < 5.0,
        "fixture must exercise downward attachment rounding"
    );
    let blocked = parcels::geometry_from_attachment(&graph, edge, -1, 0.1, 10.0, 10.0);
    assert!(parcels::geometry_overlaps_road(&graph, &blocked));
    let mut manual = make_zoning();
    for (x, z) in [
        (20.0, -10.5),
        (40.0, -10.5),
        (60.0, -10.5),
        (80.0, -10.5),
        (100.0, -10.5),
        (70.0, 10.5),
        (90.0, 10.5),
        (110.0, 10.5),
        (130.0, 10.5),
    ] {
        let id = manual
            .place_or_rezone_parcel_at(x, z, 0, 10.0, 10.0, &graph)
            .unwrap();
        assert_eq!(manual.parcel_by_raw_id(id.raw()).unwrap().edge_idx(), edge);
    }
    assert_eq!(manual.parcels().len(), 9);

    let zoning = make_zoning();
    let lots = zoning
        .preview_parcels_on_road(edge, 10.0, 10.0, 10.0, &graph)
        .unwrap();
    assert!(
        lots.len() >= 9,
        "automatic fill must retain the usable road beyond the acute corner"
    );
    let mut restored = make_zoning();
    for (index, lot) in lots.iter().enumerate() {
        assert!(!parcels::geometry_overlaps_road(&graph, lot));
        restored
            .restore_parcel_from_attachment(
                index as u64 + 1,
                edge,
                lot.side,
                lot.frontage_center_t,
                10.0,
                10.0,
                0,
                &graph,
            )
            .unwrap();
    }
    assert!(lots.iter().any(|lot| lot.side == 1));
    assert!(lots.iter().any(|lot| lot.side == -1));
}

#[test]
fn run_spacing_endpoint_rounding_preserves_strict_saved_bounds() {
    for (length, frontage) in [(140.17, 20.0), (140.12, 30.0), (140.61, 40.0)] {
        let (graph, edge) = make_straight_road_span(0.0, length);
        let min_s = frontage * 0.5;
        let max_s = length - min_s;
        for direction in [1.0, -1.0] {
            let (start, end) = if direction > 0.0 {
                (min_s, max_s)
            } else {
                (max_s, min_s)
            };
            let (_, lot) = parcels::next_non_overlapping_run_geometry_with(
                &graph,
                edge,
                1,
                start,
                end,
                direction,
                length,
                frontage,
                10.0,
                |_| false,
            )
            .expect("a legal endpoint must not be discarded by normalized attachment rounding");
            let saved_s = lot.frontage_center_t * length;
            assert!(saved_s >= min_s && saved_s <= max_s);
            make_zoning()
                .restore_parcel_from_attachment(
                    1,
                    edge,
                    1,
                    lot.frontage_center_t,
                    frontage,
                    10.0,
                    0,
                    &graph,
                )
                .unwrap();
            let outside = if direction > 0.0 {
                min_s.next_down()
            } else {
                max_s.next_up()
            };
            assert!(
                parcels::next_non_overlapping_run_geometry_with(
                    &graph,
                    edge,
                    1,
                    outside,
                    end,
                    direction,
                    length,
                    frontage,
                    10.0,
                    |_| false,
                )
                .is_none(),
                "actually out-of-bounds requests must still fail"
            );
        }
    }
}

#[test]
fn road_zoning_uses_both_sides_options_and_matches_commit() {
    let (graph, edge) = make_straight_road();
    let mut zoning = make_zoning();
    let revision = zoning.overlay_revision();
    let preview = zoning
        .preview_parcels_on_road(edge, 20.0, 30.0, 10.0, &graph)
        .unwrap();
    assert_eq!(preview.len(), 8);
    assert_eq!(zoning.overlay_revision(), revision);
    assert!(zoning.parcels().is_empty());
    for (station, pair) in preview.chunks_exact(2).enumerate() {
        assert_eq!([pair[0].side, pair[1].side], [1, -1]);
        for lot in pair {
            assert_eq!(lot.edge_idx, edge);
            assert_eq!((lot.frontage_m, lot.depth_m), (20.0, 30.0));
            assert!((lot.front_center.x - (-50.0 + station as f32 * 30.0)).abs() < 0.001);
        }
    }
    let ids = zoning
        .place_prevalidated_parcel_geometries(preview.clone(), 0)
        .unwrap();
    let mut restored = make_zoning();
    for (id, lot) in ids.iter().zip(&preview) {
        assert_eq!(
            zoning.parcel_by_raw_id(id.raw()).unwrap().corners(),
            lot.corners
        );
        restored
            .restore_parcel_from_attachment(
                id.raw(),
                edge,
                lot.side,
                lot.frontage_center_t,
                lot.frontage_m,
                lot.depth_m,
                0,
                &graph,
            )
            .unwrap();
    }
    assert!(
        zoning
            .preview_parcels_on_road(edge, 20.0, 30.0, 10.0, &graph)
            .is_err()
    );
    assert_eq!(zoning.parcels().len(), 8);
}

#[test]
fn road_zoning_skips_existing_parcels_roads_and_world_bounds() {
    let (mut graph, edge) = make_straight_road();
    let mut zoning = make_zoning();
    zoning
        .place_or_rezone_parcel_at(-30.0, -20.0, 0, 20.0, 30.0, &graph)
        .unwrap();
    let existing = zoning.parcel_geometry_at(-30.0, -20.0).unwrap();
    add_vertical_road_at_x(&mut graph, 30.0);
    // The road crosses this small world's X boundary; only contained lots may survive.
    zoning.config.width_m = 100.0;
    let preview = zoning
        .preview_parcels_on_road(edge, 20.0, 30.0, 0.0, &graph)
        .unwrap();
    assert!(preview.iter().any(|lot| lot.side == 1));
    assert!(preview.iter().any(|lot| lot.side == -1));
    assert!(preview.len() < 12);
    for lot in &preview {
        assert!(!parcels::geometries_overlap(lot, &existing));
        assert!(!parcels::geometry_overlaps_road(&graph, lot));
        assert!(parcels::geometry_inside_world(
            lot,
            100.0,
            zoning.config.height_m
        ));
    }
}

fn grade_road(graph: &mut crate::simulation::network::graph::RegionGraph, edge: usize) {
    let road = graph.edge_mut(edge);
    road.physical_geometry[1].y = 18.0;
    road.geometry = road.physical_geometry.clone();
    road.physical_length = road.physical_geometry[0].distance_to(road.physical_geometry[1]);
}

#[test]
fn road_fill_extends_both_ends_of_manual_groups_without_phase_gaps() {
    for (curved, graded, gap) in [
        (false, false, 0.0),
        (false, true, 0.0),
        (true, false, 0.0),
        (false, false, 5.0),
    ] {
        let (mut graph, edge) = if curved {
            make_quarter_arc_road(240.0)
        } else {
            make_straight_road_span(-160.0, 160.0)
        };
        if graded {
            grade_road(&mut graph, edge);
        }
        let length = graph.edge(edge).physical_length;
        let mut zoning = make_zoning();
        for (side, station, count) in [(1, length * 0.37, 3), (-1, length * 0.31, 5)] {
            let mut center_s = station;
            let mut previous = Vec::new();
            for _ in 0..count {
                let (accepted_s, lot) = parcels::next_non_overlapping_run_geometry_with(
                    &graph,
                    edge,
                    side,
                    center_s,
                    length - 10.0,
                    1.0,
                    length,
                    20.0,
                    30.0,
                    |lot| {
                        previous
                            .iter()
                            .any(|old| parcels::geometries_overlap(lot, old))
                    },
                )
                .unwrap();
                previous.push(lot);
                center_s = accepted_s + 20.0;
            }
            zoning
                .place_prevalidated_parcel_geometries(previous, 1)
                .unwrap();
        }
        let existing: Vec<_> = zoning
            .parcels()
            .iter()
            .map(parcels::geometry_for_parcel)
            .collect();
        let preview = zoning
            .preview_parcels_on_road(edge, 20.0, 30.0, gap, &graph)
            .unwrap();
        for side in [1, -1] {
            let row: Vec<_> = existing.iter().filter(|lot| lot.side == side).collect();
            for (anchor, direction) in [(row[0], -1.0), (row[row.len() - 1], 1.0)] {
                let limit = if direction > 0.0 { length - 10.0 } else { 10.0 };
                let (_, expected) = parcels::next_non_overlapping_run_geometry_with(
                    &graph,
                    edge,
                    side,
                    anchor.frontage_center_t * length + direction * (20.0 + gap),
                    limit,
                    direction,
                    length,
                    20.0,
                    30.0,
                    |lot| {
                        existing
                            .iter()
                            .any(|old| parcels::geometries_overlap(lot, old))
                    },
                )
                .unwrap();
                assert!(
                    preview
                        .iter()
                        .any(|lot| lot.center.distance_to(expected.center) < 0.002),
                    "automatic fill must meet the manual extension at both group ends: curved={curved}, graded={graded}, gap={gap}, side={side}, direction={direction}"
                );
            }
        }
        for (index, lot) in preview.iter().enumerate() {
            assert!(
                existing
                    .iter()
                    .chain(&preview[..index])
                    .all(|old| !parcels::geometries_overlap(lot, old))
            );
        }
        let repeated = zoning
            .preview_parcels_on_road(edge, 20.0, 30.0, gap, &graph)
            .unwrap();
        assert!(
            preview
                .iter()
                .zip(&repeated)
                .all(|(a, b)| a.corners == b.corners)
        );
        let ids = zoning
            .place_prevalidated_parcel_geometries(preview.clone(), 2)
            .unwrap();
        for (stored, original) in zoning.parcels().iter().zip(&existing) {
            assert_eq!(stored.corners(), original.corners);
            assert_eq!(stored.zone_profile_runtime_id(), 1);
        }
        for (id, lot) in ids.iter().zip(&preview) {
            assert_eq!(
                zoning.parcel_by_raw_id(id.raw()).unwrap().corners(),
                lot.corners
            );
        }
        assert!(
            zoning
                .preview_parcels_on_road(edge, 20.0, 30.0, gap, &graph)
                .is_err()
        );
    }
}

#[test]
fn road_fill_anchors_mixed_widths_and_keeps_remainders_between_new_lots() {
    let (graph, edge) = make_straight_road_span(-160.0, 160.0);
    let mut reference: Option<Vec<parcels::ParcelGeometry>> = None;
    for reverse_insertion in [false, true] {
        let mut zoning = make_zoning();
        let mut anchors = [(-47.0, 30.0), (56.0, 40.0)];
        if reverse_insertion {
            anchors.reverse();
        }
        for (x, width) in anchors {
            zoning
                .place_or_rezone_parcel_at(x, -25.0, 1, width, 30.0, &graph)
                .unwrap();
        }
        let lots = zoning
            .preview_parcels_on_road(edge, 20.0, 30.0, 0.0, &graph)
            .unwrap();
        let row: Vec<_> = lots.iter().filter(|lot| lot.side == 1).collect();
        // Each side of each differently sized anchor must have a touching 20 m neighbor.
        for expected_x in [-72.0, -22.0, 26.0, 86.0] {
            assert!(
                row.iter()
                    .any(|lot| (lot.front_center.x - expected_x).abs() < 0.002)
            );
        }
        let mut interior: Vec<_> = row
            .iter()
            .filter(|lot| lot.front_center.x > -47.0 && lot.front_center.x < 56.0)
            .collect();
        interior.sort_by(|a, b| a.frontage_center_t.total_cmp(&b.frontage_center_t));
        assert_eq!(
            interior.len(),
            3,
            "three full 20 m lots fit between these anchors"
        );
        let remainder: f32 = interior
            .windows(2)
            .map(|p| p[1].front_center.x - p[0].front_center.x - 20.0)
            .sum();
        assert!(
            (remainder - 8.0).abs() < 0.002,
            "the unfillable eight meters belongs between new lots"
        );
        assert_eq!(
            lots.iter().filter(|lot| lot.side == -1).count(),
            16,
            "the empty opposite side keeps its endpoint layout"
        );
        // Both an extension across another group and a free-start drag use the road-fill layout.
        for (start_x, end_x) in [(-47.0_f32, 150.0_f32), (56.0, -150.0), (-150.0, 150.0)] {
            let manual = zoning
                .preview_parcel_run_at(start_x, -25.0, end_x, -25.0, 20.0, 30.0, 0.0, &graph)
                .unwrap();
            let expected: Vec<_> = row
                .iter()
                .filter(|lot| {
                    lot.front_center.x >= start_x.min(end_x) - 0.001
                        && lot.front_center.x <= start_x.max(end_x) + 0.001
                })
                .collect();
            assert_eq!(manual.len(), expected.len());
            assert!(
                manual.iter().all(|lot| expected
                    .iter()
                    .any(|automatic| lot.corners == automatic.corners)),
                "manual extension and automatic fill must use identical anchor geometry"
            );
        }
        for (index, lot) in lots.iter().enumerate() {
            assert!(!zoning.parcels.overlaps_existing(lot));
            assert!(
                lots[..index]
                    .iter()
                    .all(|old| !parcels::geometries_overlap(old, lot))
            );
        }
        if let Some(expected) = &reference {
            assert_eq!(lots.len(), expected.len());
            assert!(
                lots.iter()
                    .zip(expected)
                    .all(|(a, b)| a.corners == b.corners),
                "insertion order cannot change road fill"
            );
        }
        reference = Some(lots);
    }
}

#[test]
fn zero_gap_road_zoning_packs_neighbors_on_a_grade() {
    let (mut graph, edge) = make_straight_road();
    grade_road(&mut graph, edge);
    let zoning = make_zoning();
    let lots = zoning
        .preview_parcels_on_road(edge, 20.0, 30.0, 0.0, &graph)
        .unwrap();
    assert_eq!(
        lots.len(),
        12,
        "a small overlap must shift the next lot, not discard a whole frontage"
    );
    for side in [1, -1] {
        let row: Vec<_> = lots.iter().filter(|lot| lot.side == side).collect();
        for pair in row.windows(2) {
            let gap = (pair[1].front_center - pair[0].front_center).dot(pair[0].tangent) - 20.0;
            assert!(
                gap.abs() < 0.002,
                "zero-gap neighbors must touch within overlap/search tolerance: {gap}"
            );
            assert!(!parcels::geometries_overlap(pair[0], pair[1]));
        }
    }
}

#[test]
fn graded_manual_extension_keeps_the_last_lot_in_both_directions() {
    let (mut graph, edge) = make_straight_road();
    grade_road(&mut graph, edge);
    let length = graph.edge(edge).physical_length;
    for reverse in [false, true] {
        let mut zoning = make_zoning();
        let (start_s, end_s) = if reverse {
            (length - 10.0, 10.0)
        } else {
            (10.0, length - 10.0)
        };
        zoning
            .restore_parcel_from_attachment(1, edge, 1, start_s / length, 20.0, 30.0, 0, &graph)
            .unwrap();
        let start = zoning.parcel_by_raw_id(1).unwrap().center();
        let end =
            parcels::geometry_from_attachment(&graph, edge, 1, end_s / length, 20.0, 30.0).center;
        let extension = zoning
            .preview_parcel_run_at(start.x, start.y, end.x, end.y, 20.0, 30.0, 0.0, &graph)
            .unwrap();
        assert_eq!(
            extension.len(),
            5,
            "probe the remaining endpoint interval before rejecting the last lot; reverse={reverse}"
        );
    }
}

#[test]
fn zero_gap_curved_road_zoning_matches_manual_extension_density() {
    let (graph, edge) = make_quarter_arc_road(140.0);
    let zoning = make_zoning();
    let lots = zoning
        .preview_parcels_on_road(edge, 20.0, 30.0, 0.0, &graph)
        .unwrap();
    let length = graph.edge(edge).physical_length;
    for side in [1, -1] {
        let mut manual = make_zoning();
        manual
            .restore_parcel_from_attachment(1, edge, side, 10.0 / length, 20.0, 30.0, 0, &graph)
            .unwrap();
        let start = manual.parcel_by_raw_id(1).unwrap().center();
        let end = parcels::geometry_from_attachment(
            &graph,
            edge,
            side,
            (length - 10.0) / length,
            20.0,
            30.0,
        )
        .center;
        let extension = manual
            .preview_parcel_run_at(start.x, start.y, end.x, end.y, 20.0, 30.0, 0.0, &graph)
            .unwrap();
        let row: Vec<_> = lots.iter().filter(|lot| lot.side == side).collect();
        assert_eq!(
            row.len(),
            extension.len() + 1,
            "road fill must fit the same lots as manual extension on side {side}"
        );
        for (automatic, manual) in row.iter().skip(1).zip(&extension) {
            assert!(automatic.center.distance_to(manual.center) < 0.002);
        }
    }
}

#[test]
fn curved_road_zoning_has_no_overlaps_and_round_trips_attachments() {
    for radius in [60.0, 80.0, 140.0] {
        let (graph, edge) = make_quarter_arc_road(radius);
        let zoning = make_zoning();
        let lots = zoning
            .preview_parcels_on_road(edge, 20.0, 30.0, 0.0, &graph)
            .unwrap();
        let again = zoning
            .preview_parcels_on_road(edge, 20.0, 30.0, 0.0, &graph)
            .unwrap();
        assert_eq!(lots.len(), again.len());
        assert!(lots.iter().any(|lot| lot.side == 1));
        assert!(lots.iter().any(|lot| lot.side == -1));
        let mut restored = make_zoning();
        for (index, lot) in lots.iter().enumerate() {
            assert_eq!(lot.corners, again[index].corners);
            assert!(
                lots[..index]
                    .iter()
                    .all(|other| !parcels::geometries_overlap(lot, other))
            );
            restored
                .restore_parcel_from_attachment(
                    index as u64 + 1,
                    edge,
                    lot.side,
                    lot.frontage_center_t,
                    20.0,
                    30.0,
                    0,
                    &graph,
                )
                .unwrap();
        }
    }
}

#[test]
fn road_zoning_hover_and_invalid_input_rejection() {
    let (mut graph, edge) = make_straight_road();
    let zoning = make_zoning();
    let half_width = graph.edge(edge).width * 0.5 + crate::config::SIDEWALK_WIDTH;
    assert_eq!(zoning.road_edge_at(0.0, half_width, &graph), Some(edge));
    assert_eq!(zoning.road_edge_at(0.0, half_width + 0.01, &graph), None);
    assert_eq!(zoning.road_edge_at(f32::NAN, 0.0, &graph), None);
    add_vertical_road_at_x(&mut graph, 0.0);
    assert_eq!(zoning.road_edge_at(0.0, 0.0, &graph), Some(edge));
    for (frontage, depth, gap) in [(f32::NAN, 20.0, 0.0), (20.0, 0.0, 0.0), (20.0, 20.0, -1.0)] {
        assert!(
            zoning
                .preview_parcels_on_road(edge, frontage, depth, gap, &graph)
                .is_err()
        );
    }
    assert!(
        zoning
            .preview_parcels_on_road(usize::MAX, 20.0, 20.0, 0.0, &graph)
            .is_err()
    );
    graph.edge_mut(edge).no_building_spawn = true;
    assert_eq!(zoning.road_edge_at(-20.0, 0.0, &graph), Some(edge));
    assert!(matches!(
        zoning.preview_parcels_on_road(edge, 20.0, 20.0, 0.0, &graph),
        Err(ParcelPlacementError::NoRoadAttachment)
    ));
    graph.edge_mut(edge).deleted = true;
    assert_eq!(zoning.road_edge_at(-20.0, 0.0, &graph), None);
}

#[test]
#[ignore = "release locality measurement for road zoning hover and uncached geometry"]
fn benchmark_road_zoning_locality() {
    use crate::simulation::network::types::NodeType;
    use godot::prelude::Vector3;
    use std::{hint::black_box, time::Instant};

    for background in [0, 1_000, 10_000, 100_000] {
        let setup = Instant::now();
        let (mut graph, edge) = make_straight_road();
        let template = graph.edge(edge).clone();
        let mut zoning = make_zoning();
        for index in 0..background {
            let x = 2_048.0 + (index % 512) as f32 * 160.0;
            let z = 2_048.0 + (index / 512) as f32 * 160.0;
            let points = [Vector3::new(x, 0.0, z), Vector3::new(x + 120.0, 0.0, z)];
            let start = graph.add_node(points[0], NodeType::Junction);
            let end = graph.add_node(points[1], NodeType::Junction);
            let mut road = template.clone();
            road.start_node = start;
            road.end_node = end;
            road.geometry = points.to_vec();
            road.physical_geometry = points.to_vec();
            let remote = graph.add_edge(road);
            zoning.parcels.insert_new(
                parcels::geometry_from_attachment(&graph, remote, 1, 0.5, 20.0, 20.0),
                0,
            );
        }
        let setup_ms = setup.elapsed().as_secs_f64() * 1e3;
        for operation in ["hover", "preview", "graded_preview"] {
            if operation == "graded_preview" {
                grade_road(&mut graph, edge);
            }
            let mut samples = Vec::new();
            for sample in 0..12 {
                let start = Instant::now();
                for _ in 0..64 {
                    if operation == "hover" {
                        assert_eq!(
                            black_box(zoning.road_edge_at(black_box(0.0), 0.0, &graph)),
                            Some(edge)
                        );
                    } else {
                        let lots = black_box(
                            zoning
                                .preview_parcels_on_road(edge, 20.0, 20.0, 0.0, &graph)
                                .unwrap(),
                        );
                        assert_eq!(lots.len(), 12);
                        for (index, lot) in lots.iter().enumerate() {
                            assert_eq!(lot.side, if index % 2 == 0 { 1 } else { -1 });
                            if operation == "preview" {
                                assert!(
                                    (lot.front_center.x - (-50.0 + (index / 2) as f32 * 20.0))
                                        .abs()
                                        < 0.001
                                );
                            } else if index >= 2 {
                                assert!(
                                    (lot.front_center.x - lots[index - 2].front_center.x - 20.0)
                                        .abs()
                                        < 0.002
                                );
                            }
                        }
                    }
                }
                if sample >= 3 {
                    samples.push(start.elapsed().as_secs_f64() * 1e6 / 64.0);
                }
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "ROAD_ZONING_BENCH {}",
                serde_json::json!({
                    "background": background, "operation": operation, "median_us": samples[4], "setup_ms": setup_ms,
                })
            );
        }
    }
}
