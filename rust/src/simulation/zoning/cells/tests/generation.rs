// SPDX-License-Identifier: GPL-2.0-only

//! Road-generated reference arrangements, non-overlap, pinned paint and locality regressions.

mod conflicts;
mod lots;
mod queries;

use super::*;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::zoning::{ParcelPlacementError, ZoningSystem};
use godot::prelude::Vector3;

fn node(graph: &mut RegionGraph, x: f32, z: f32) -> u32 {
    graph.add_node(Vector3::new(x, 0.0, z), NodeType::Junction)
}

fn road(graph: &mut RegionGraph, a: u32, b: u32) -> usize {
    let points = vec![graph.node(a).pos, graph.node(b).pos];
    polyline(graph, a, b, points)
}

fn polyline(graph: &mut RegionGraph, a: u32, b: u32, points: Vec<Vector3>) -> usize {
    let physical_length = points
        .windows(2)
        .map(|pair| pair[0].distance_to(pair[1]))
        .sum();
    graph.add_edge(Edge {
        start_node: a,
        end_node: b,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        physical_length,
        geometry: points.clone(),
        physical_geometry: points,
        ..Default::default()
    })
}

fn extent() -> CellBounds {
    CellBounds {
        min: DVec2::splat(-300.0),
        max: DVec2::splat(300.0),
    }
}

fn keys(store: &CellStore, bounds: CellBounds) -> Vec<CellKey> {
    let mut result = Vec::new();
    store.visit_in_bounds(bounds, |key| result.push(key));
    result.sort_unstable();
    result
}

fn assert_disjoint(store: &CellStore, bounds: CellBounds) {
    let cells = keys(store, bounds);
    for (index, a) in cells.iter().enumerate() {
        let ac = store.frame(a.grid).unwrap().corners(a.x, a.y);
        for b in &cells[index + 1..] {
            let bc = store.frame(b.grid).unwrap().corners(b.x, b.y);
            assert!(
                !interiors_overlap(&ac, &bc),
                "overlap {a:?} {b:?}: {ac:?} {bc:?}"
            );
        }
    }
}

#[test]
fn straight_road_has_twelve_columns_and_six_rows_per_side() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    road(&mut graph, a, b);
    let mut store = CellStore::default();
    let report = store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
    assert_eq!(report.accepted, 12 * 6 * 2);
    let cells = keys(&store, extent());
    assert_eq!(cells.len(), report.accepted);
    for x in 0..12 {
        for row in 0..6 {
            for sign in [-1.0, 1.0] {
                assert!(
                    store
                        .pick(DVec2::new(
                            f64::from(x) * 10.0 + 5.0,
                            sign * (10.0 + f64::from(row) * 10.0)
                        ))
                        .is_some()
                );
            }
        }
    }
    let revision = store.revision();
    assert_eq!(
        store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false),
        report
    );
    assert_eq!(store.revision(), revision);
    assert_eq!(keys(&store, extent()), cells);
    assert_disjoint(&store, extent());
}

#[test]
fn orthogonal_l_and_t_corners_share_a_complete_grid() {
    for with_west in [false, true] {
        let mut graph = RegionGraph::new();
        let centre = node(&mut graph, 0.0, 0.0);
        let east = node(&mut graph, 120.0, 0.0);
        let north = node(&mut graph, 0.0, 120.0);
        road(&mut graph, centre, east);
        road(&mut graph, centre, north);
        if with_west {
            let west = node(&mut graph, -120.0, 0.0);
            road(&mut graph, west, centre);
        }
        let mut store = CellStore::default();
        store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
        let first = store.pick(DVec2::splat(10.0)).unwrap();
        for x in 0..6 {
            for y in 0..6 {
                let key = store
                    .pick(DVec2::new(
                        10.0 + f64::from(x) * 10.0,
                        10.0 + f64::from(y) * 10.0,
                    ))
                    .unwrap();
                assert_eq!(key.grid, first.grid);
            }
        }
        assert_disjoint(&store, extent());
    }
}

#[test]
fn rectangular_block_is_filled_without_duplicates_or_internal_seams() {
    let mut graph = RegionGraph::new();
    let corners = [(0.0, 0.0), (120.0, 0.0), (120.0, 120.0), (0.0, 120.0)]
        .map(|(x, z)| node(&mut graph, x, z));
    for i in 0..4 {
        road(&mut graph, corners[i], corners[(i + 1) % 4]);
    }
    let mut store = CellStore::default();
    store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
    let first = store.pick(DVec2::splat(10.0)).unwrap();
    for x in 0..11 {
        for y in 0..11 {
            let key = store
                .pick(DVec2::new(
                    10.0 + f64::from(x) * 10.0,
                    10.0 + f64::from(y) * 10.0,
                ))
                .unwrap();
            assert_eq!(key.grid, first.grid);
        }
    }
    let fill = store.select(CellSelectionShape::Fill, &[DVec2::splat(10.0)]);
    assert_eq!(fill.cells.len(), 121);
    assert_disjoint(&store, extent());
}

#[test]
fn t_junction_keeps_backside_cells_across_unpainted_straight_edge_splits() {
    for angle in [0.0_f64, 0.35] {
        for reversed in [false, true] {
            let u = DVec2::new(angle.cos(), angle.sin());
            let v = u.perp();
            let mut graph = RegionGraph::new();
            let ids = [u * -120.0, DVec2::ZERO, u * 120.0, v * 120.0]
                .map(|p| node(&mut graph, p.x as f32, p.y as f32));
            for (a, b) in [(ids[0], ids[1]), (ids[1], ids[2]), (ids[1], ids[3])] {
                road(
                    &mut graph,
                    if reversed { b } else { a },
                    if reversed { a } else { b },
                );
            }
            let mut store = CellStore::default();
            store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
            for row in 0..CELL_DEPTH {
                let point = -v * (10.0 + row as f64 * 10.0);
                let key = store
                    .pick(point)
                    .expect("backside frontage crosses the split");
                if row != 0 {
                    continue;
                }
                let frontages = store.frontages(key);
                assert_eq!(frontages.len(), 2);
                let mut spans: Vec<_> = frontages.iter().map(|f| (f.start, f.end)).collect();
                spans.sort_unstable();
                assert_eq!(spans[0].0, 0);
                assert!(spans[0].1.abs_diff(spans[1].0) <= 1);
                assert_eq!(spans[1].1, CELL_FRONTAGE_UNITS);
            }
            assert_disjoint(&store, extent());
            graph.edge_mut(1).no_building_spawn = true;
            store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
            assert!(
                store.pick(-v * 10.0).is_none(),
                "disabled continuation cannot authorize new cells"
            );
        }
    }
}

#[test]
fn adding_a_branch_preserves_the_existing_backside_grid() {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::network::{TransitNetwork, topology};

    for (angle, reversed) in [0.0_f64, 0.35, -0.55]
        .into_iter()
        .flat_map(|angle| [false, true].map(move |reversed| (angle, reversed)))
    {
        for branch_angle in [std::f64::consts::FRAC_PI_2, 1.1] {
            for centre in [DVec2::ZERO, DVec2::new(-5000.0, 2500.0)] {
                let u = DVec2::new(angle.cos(), angle.sin());
                let v = u.perp();
                let mut graph = RegionGraph::new();
                let a = node(&mut graph, centre.x as f32, centre.y as f32);
                let end = centre + u * 240.0;
                let b = node(&mut graph, end.x as f32, end.y as f32);
                road(
                    &mut graph,
                    if reversed { b } else { a },
                    if reversed { a } else { b },
                );
                let bounds = CellBounds::from_points([centre, end]).expanded(150.0);
                let config = WorldConfig::default();
                let mut zoning = ZoningSystem::new(&config);
                zoning.cells.refresh_road_alignments(&graph, &config, [0]);
                zoning.generate_cells(&graph, bounds, |_| false);
                let before: Vec<_> = keys(&zoning.cells, bounds)
                    .into_iter()
                    .filter(|key| {
                        let frame = zoning.cells.frame(key.grid).unwrap();
                        let p = frame.world(key.x as f64 + 0.5, key.y as f64 + 0.5);
                        (p - centre).dot(v) < 0.0
                    })
                    .collect();
                // Rounded endpoints may exclude one terminal column before the edit.
                assert!((23 * CELL_DEPTH..=24 * CELL_DEPTH).contains(&before.len()));
                let split = graph.node(a).pos.lerp(graph.node(b).pos, 0.47);
                let junction = node(&mut graph, split.x, split.z);
                topology::split_edge(
                    &mut TransitNetwork::new(),
                    &mut graph,
                    0,
                    0,
                    if reversed { 0.53 } else { 0.47 },
                    junction,
                    &mut zoning,
                    &mut BuildingAllocator::new(),
                );
                let branch = DVec2::new(f64::from(split.x), f64::from(split.z))
                    + (u * branch_angle.cos() + v * branch_angle.sin()) * 120.0;
                let c = node(&mut graph, branch.x as f32, branch.y as f32);
                road(&mut graph, junction, c);
                zoning
                    .cells
                    .refresh_road_alignments(&graph, &config, 0..graph.edge_count());
                zoning.generate_cells(&graph, bounds, |_| false);
                for key in before {
                    assert!(
                        zoning.cells.profile(key).is_some(),
                        "lost {key:?}, angle={angle}, reversed={reversed}, branch={branch_angle}, centre={centre:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn curves_acute_junctions_and_competing_roads_have_no_cell_overlap() {
    for scenario in 0..3 {
        let mut graph = RegionGraph::new();
        let a = node(&mut graph, -120.0, 0.0);
        let b = node(&mut graph, 120.0, 0.0);
        if scenario == 0 {
            let points = (0..=24)
                .map(|i| {
                    let x = -120.0 + i as f32 * 10.0;
                    Vector3::new(x, 0.0, (x / 50.0).sin() * 40.0)
                })
                .collect();
            polyline(&mut graph, a, b, points);
        } else {
            road(&mut graph, a, b);
            let c = if scenario == 1 {
                a
            } else {
                node(&mut graph, -120.0, 75.0)
            };
            let d = node(&mut graph, 120.0, if scenario == 1 { 150.0 } else { 45.0 });
            road(&mut graph, c, d);
        }
        let mut store = CellStore::default();
        let report = store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
        assert!(report.accepted > 100, "scenario {scenario}: {report:?}");
        assert!(report.comparisons > 0, "scenario {scenario}: {report:?}");
        assert_disjoint(&store, extent());
    }
}

#[test]
fn new_competing_road_preserves_existing_painted_geometry() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, -120.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    road(&mut graph, a, b);
    let mut store = CellStore::default();
    store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
    let paint = store.select(
        CellSelectionShape::Brush { radius_m: 25.0 },
        &[DVec2::new(0.0, 40.0)],
    );
    assert!(!paint.cells.is_empty());
    store.paint(&paint, 3).unwrap();
    let before: Vec<_> = paint
        .cells
        .iter()
        .map(|key| store.frame(key.grid).unwrap().corners(key.x, key.y))
        .collect();
    let c = node(&mut graph, -120.0, 100.0);
    let d = node(&mut graph, 120.0, 90.0);
    road(&mut graph, c, d);
    store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
    for (&key, corners) in paint.cells.iter().zip(before) {
        assert_eq!(store.profile(key), Some(3));
        assert_eq!(
            store.frame(key.grid).unwrap().corners(key.x, key.y),
            corners
        );
    }
    assert_disjoint(&store, extent());
}

#[test]
fn grade_samples_do_not_change_horizontal_cell_size_or_count() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    polyline(
        &mut graph,
        a,
        b,
        (0..=24)
            .map(|i| Vector3::new(i as f32 * 5.0, i as f32 * 2.0, 0.0))
            .collect(),
    );
    let mut store = CellStore::default();
    let report = store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
    assert_eq!(report.accepted, 144);
}

#[test]
fn distant_roads_do_not_change_local_generation_work_or_products() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    road(&mut graph, a, b);
    let mut store = CellStore::default();
    let report = store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
    let expected = keys(&store, extent());
    for i in 0..1000 {
        let a = node(
            &mut graph,
            2000.0 + (i % 20) as f32 * 100.0,
            2000.0 + (i / 20) as f32 * 100.0,
        );
        let b = node(
            &mut graph,
            2050.0 + (i % 20) as f32 * 100.0,
            2000.0 + (i / 20) as f32 * 100.0,
        );
        road(&mut graph, a, b);
    }
    assert_eq!(
        store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false),
        report
    );
    assert_eq!(keys(&store, extent()), expected);
}

#[test]
fn external_reservations_remove_whole_cells_and_no_build_roads_generate_none() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    let edge = road(&mut graph, a, b);
    let mut store = CellStore::default();
    store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |corners| {
        corners.iter().any(|point| point.x < 30.0)
    });
    assert!(store.pick(DVec2::new(25.0, 10.0)).is_none());
    assert!(store.pick(DVec2::new(35.0, 10.0)).is_some());
    graph.edge_mut(edge).no_building_spawn = true;
    store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
    assert!(keys(&store, extent()).is_empty());
}

#[test]
fn manual_parcels_and_cell_paint_reserve_land_in_both_directions() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    road(&mut graph, a, b);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    zoning.generate_cells(&graph, extent(), |_| false);
    let selection = zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::new(25.0, 10.0)]);
    zoning.paint_cells(&selection, 1, |_| false).unwrap();
    assert_eq!(
        zoning.place_or_rezone_parcel_at(25.0, 15.0, 0, 20.0, 20.0, &graph),
        Err(ParcelPlacementError::OverlapsExistingParcel)
    );
    let erase = zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::new(25.0, 10.0)]);
    zoning.paint_cells(&erase, 0, |_| false).unwrap();
    let pending = zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::new(25.0, 10.0)]);
    let parcel = zoning
        .place_or_rezone_parcel_at(25.0, 15.0, 0, 20.0, 20.0, &graph)
        .unwrap();
    assert!(zoning.paint_cells(&pending, 1, |_| false).is_none());
    zoning.generate_cells(&graph, extent(), |_| false);
    assert!(zoning.cells.pick(DVec2::new(25.0, 10.0)).is_none());
    assert_eq!(
        zoning
            .parcels
            .get(parcel)
            .unwrap()
            .zone_profile_runtime_id(),
        0
    );
    zoning.clear();
    assert!(keys(&zoning.cells, extent()).is_empty());
    assert!(zoning.parcels().is_empty());
}

#[test]
fn rotated_orthogonal_corners_share_phase_and_remain_disjoint() {
    for angle in [0.35_f32, 0.8, 1.4] {
        let mut graph = RegionGraph::new();
        let centre = node(&mut graph, 0.0, 0.0);
        let east = node(&mut graph, 120.0 * angle.cos(), 120.0 * angle.sin());
        let north = node(&mut graph, -120.0 * angle.sin(), 120.0 * angle.cos());
        road(&mut graph, centre, east);
        road(&mut graph, centre, north);
        let mut store = CellStore::default();
        store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
        let u = DVec2::new(f64::from(angle.cos()), f64::from(angle.sin()));
        let v = u.perp();
        let first = store.pick(u * 10.0 + v * 10.0).expect("first corner cell");
        for x in 0..6 {
            for y in 0..6 {
                let key = store
                    .pick(u * (10.0 + f64::from(x) * 10.0) + v * (10.0 + f64::from(y) * 10.0))
                    .expect("complete rotated corner");
                assert_eq!(key.grid, first.grid, "angle {angle}, {x},{y}");
            }
        }
        assert_disjoint(&store, extent());
    }
}

#[test]
fn generation_is_identical_across_rayon_worker_counts() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, -120.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    let c = node(&mut graph, -120.0, 90.0);
    let d = node(&mut graph, 120.0, 45.0);
    road(&mut graph, a, b);
    road(&mut graph, c, d);
    let run = |workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                let mut store = CellStore::default();
                store.refresh_road_alignments(
                    &graph,
                    &WorldConfig::default(),
                    0..graph.edge_count(),
                );
                let report =
                    store.generate_in_bounds(&graph, &WorldConfig::default(), extent(), |_| false);
                let products = keys(&store, extent())
                    .into_iter()
                    .map(|key| (key, store.frame(key.grid).unwrap()))
                    .collect::<Vec<_>>();
                (report, products, store.saved_road_alignments())
            })
    };
    assert_eq!(run(1), run(4));
}

#[test]
fn translated_sampled_orthogonal_roads_keep_one_prepared_grid() {
    for angle in [0.35_f64, -0.55, 0.8, 2.2] {
        let u = DVec2::new(angle.cos(), angle.sin());
        let v = u.perp();
        let centre = DVec2::new(-5000.0, 2500.0);
        let mut graph = RegionGraph::new();
        let origin = node(&mut graph, centre.x as f32, centre.y as f32);
        for direction in [u, v] {
            let points: Vec<_> = (0..=24)
                .map(|i| {
                    let p = centre + direction * (i as f64 * 5.0);
                    Vector3::new(p.x as f32, 0.0, p.y as f32)
                })
                .collect();
            let last = *points.last().unwrap();
            let end = node(&mut graph, last.x, last.z);
            polyline(&mut graph, origin, end, points);
        }
        let bounds = CellBounds::from_points([centre, centre + u * 120.0, centre + v * 120.0])
            .expanded(100.0);
        let mut store = CellStore::default();
        store.refresh_road_alignments(&graph, &WorldConfig::default(), 0..graph.edge_count());
        assert_eq!(store.saved_road_alignments().len(), 2);
        store.generate_in_bounds(&graph, &WorldConfig::default(), bounds, |_| false);
        let first = store.pick(centre + u * 10.0 + v * 10.0).unwrap();
        for x in 0..6 {
            for y in 0..6 {
                let key = store
                    .pick(centre + u * (10.0 + x as f64 * 10.0) + v * (10.0 + y as f64 * 10.0))
                    .expect("complete translated orthogonal corner");
                assert_eq!(key.grid, first.grid, "angle {angle}, {x},{y}");
            }
        }
        assert_disjoint(&store, bounds);
    }
}

#[test]
#[ignore = "matched release locality measurement; run without competing builds"]
fn benchmark_cell_zoning_locality() {
    use std::hint::black_box;
    use std::time::Instant;
    let config = WorldConfig::default();
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    road(&mut graph, a, b);
    let mut store = CellStore::default();
    let expected = store.generate_in_bounds(&graph, &config, extent(), |_| false);
    let expected_keys = keys(&store, extent());
    let frame = store.register_frame(GridFrame::new(DVec2::ZERO, DVec2::X, 10.0).unwrap());
    let mut previous_roads = 0;
    let mut previous_cells = 0;
    for (background_roads, background_cells) in [(0, 0), (1000, 16_384), (10_000, 131_072)] {
        // Setup is deliberately outside all repeated operation timings.
        for i in previous_roads..background_roads {
            let x = 2000.0 + (i % 100) as f32 * 50.0;
            let z = 2000.0 + (i / 100) as f32 * 50.0;
            let a = node(&mut graph, x, z);
            let b = node(&mut graph, x + 25.0, z);
            road(&mut graph, a, b);
        }
        for i in previous_cells..background_cells {
            store.insert(CellKey {
                grid: frame,
                x: 200 + i % 512,
                y: 200 + i / 512,
            });
        }
        previous_roads = background_roads;
        previous_cells = background_cells;
        for _ in 0..5 {
            assert_eq!(
                store.generate_in_bounds(&graph, &config, extent(), |_| false),
                expected
            );
        }
        let mut samples = Vec::new();
        for _ in 0..21 {
            let start = Instant::now();
            for _ in 0..16 {
                black_box(store.generate_in_bounds(&graph, &config, extent(), |_| false));
            }
            samples.push(start.elapsed().as_secs_f64() * 1e6 / 16.0);
        }
        samples.sort_by(f64::total_cmp);
        let generation_us = samples[samples.len() / 2];
        let start = Instant::now();
        for _ in 0..4096 {
            black_box(store.pick(black_box(DVec2::new(25.0, 10.0))));
        }
        let pick_us = start.elapsed().as_secs_f64() * 1e6 / 4096.0;
        let start = Instant::now();
        for _ in 0..512 {
            black_box(store.select(
                CellSelectionShape::Brush { radius_m: 20.0 },
                black_box(&[DVec2::new(25.0, 10.0), DVec2::new(65.0, 10.0)]),
            ));
        }
        let brush_us = start.elapsed().as_secs_f64() * 1e6 / 512.0;
        assert_eq!(keys(&store, extent()), expected_keys);
        println!(
            "roads={background_roads} cells={background_cells} generation_us={generation_us:.3} pick_us={pick_us:.3} brush_us={brush_us:.3} products={expected:?}"
        );
    }
}
