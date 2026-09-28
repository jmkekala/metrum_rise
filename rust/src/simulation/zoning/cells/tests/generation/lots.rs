// SPDX-License-Identifier: GPL-2.0-only

//! Generated-frontage lot derivation, blocked/rear paint and deterministic local publication.

use super::*;

mod curves;

fn painted_road(angle: f64) -> (RegionGraph, ZoningSystem) {
    let direction = DVec2::new(angle.cos(), angle.sin());
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(
        &mut graph,
        (direction.x * 120.0) as f32,
        (direction.y * 120.0) as f32,
    );
    road(&mut graph, a, b);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    zoning.generate_cells(&graph, extent(), |_| false);
    let point = direction * 5.0 + direction.perp() * 10.0;
    let selection = zoning.cells.select(CellSelectionShape::Fill, &[point]);
    zoning.paint_cells(&selection, 1, |_| false).unwrap();
    (graph, zoning)
}

fn size() -> CellLotSize {
    CellLotSize {
        profile: 1,
        width: 3,
        depth: 2,
    }
}

#[test]
fn generated_painted_frontages_create_disjoint_asset_sized_lots() {
    for angle in [0.0, 0.35, 0.8] {
        let (graph, mut zoning) = painted_road(angle);
        let paint = zoning.cells.saved_cells();
        let report: crate::simulation::zoning::CellLotGeneration =
            zoning.derive_cell_lots(&graph, extent(), &[size()], |_, _| false);
        assert_eq!(report.created.len(), 4, "angle {angle}");
        assert_eq!(zoning.cells.saved_cells(), paint);
        for id in report.created {
            let parcel = zoning.parcels.get(id).unwrap();
            assert_eq!(parcel.frontage_m(), 30.0);
            assert_eq!(parcel.depth_m(), 20.0);
            let lot = parcel.cell_lot().unwrap();
            assert_eq!(lot.cells().count(), 6);
            assert!(
                lot.cells()
                    .all(|key| zoning.cells.lot(key) == Some(id.raw()))
            );
            assert!(
                lot.frontage_cells()
                    .all(|key| !zoning.cells.frontages(key).is_empty())
            );
        }
        assert!(
            zoning
                .derive_cell_lots(&graph, extent(), &[size()], |_, _| false)
                .created
                .is_empty()
        );
    }
}

#[test]
fn splitting_an_unchanged_road_preserves_paint_and_frontage() {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::network::{TransitNetwork, topology};

    for (angle, split_m, existing_lots) in [0.0, 0.35].into_iter().flat_map(|angle| {
        [13.0, 43.0, 53.0, 65.0, 107.0]
            .into_iter()
            .flat_map(move |split| {
                [false, true]
                    .into_iter()
                    .map(move |existing| (angle, split, existing))
            })
    }) {
        let (mut graph, mut zoning) = painted_road(angle);
        if existing_lots {
            assert_eq!(
                zoning
                    .derive_cell_lots(&graph, extent(), &[size()], |_, _| false)
                    .created
                    .len(),
                4
            );
        }
        let lots: Vec<_> = zoning
            .parcels()
            .iter()
            .map(|p| (p.id(), p.corners(), p.build_generation()))
            .collect();
        let painted = zoning.cells.saved_cells();
        let frontages: Vec<_> = painted
            .iter()
            .filter_map(|&(key, _)| (!zoning.cells.frontages(key).is_empty()).then_some(key))
            .collect();
        let direction = DVec2::new(angle.cos(), angle.sin());
        let split = direction * split_m;
        let junction = node(&mut graph, split.x as f32, split.y as f32);
        let mut network = TransitNetwork::new();
        let mut allocator = BuildingAllocator::new();
        topology::split_edge(
            &mut network,
            &mut graph,
            0,
            0,
            split_m as f32 / 120.0,
            junction,
            &mut zoning,
            &mut allocator,
        );
        zoning.generate_cells(&graph, extent(), |_| false);
        assert_eq!(zoning.cells.saved_cells(), painted);
        assert!(
            frontages
                .iter()
                .all(|&key| !zoning.cells.frontages(key).is_empty()),
            "road split lost frontage at angle {angle}"
        );
        let report = zoning.derive_cell_lots(&graph, extent(), &[size()], |_, _| false);
        assert_eq!(
            report.created.len(),
            if existing_lots { 0 } else { 4 },
            "split {split_m} must retain four developable lots at angle {angle}"
        );
        assert!(report.retired.is_empty());
        if existing_lots {
            assert_eq!(
                zoning
                    .parcels()
                    .iter()
                    .map(|p| (p.id(), p.corners(), p.build_generation()))
                    .collect::<Vec<_>>(),
                lots
            );
        }
    }
}

#[test]
fn no_build_half_of_a_split_does_not_leave_complete_frontage_in_a_shared_cell() {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::network::{TransitNetwork, topology};
    let (mut graph, mut zoning) = painted_road(0.0);
    let junction = node(&mut graph, 43.0, 0.0);
    topology::split_edge(
        &mut TransitNetwork::new(),
        &mut graph,
        0,
        0,
        43.0 / 120.0,
        junction,
        &mut zoning,
        &mut BuildingAllocator::new(),
    );
    zoning.generate_cells(&graph, extent(), |_| false);
    assert_eq!(
        zoning
            .derive_cell_lots(&graph, extent(), &[size()], |_, _| false)
            .created
            .len(),
        4
    );
    graph.edge_mut(1).no_building_spawn = true;
    let report = zoning.derive_cell_lots(&graph, extent(), &[size()], |_, _| false);
    assert_eq!(report.retired.len(), 3);
    assert!(report.created.is_empty());
    assert_eq!(zoning.parcels().len(), 1);
}

#[test]
fn no_build_partial_frontage_cannot_generate_empty_cells_in_a_retained_strip() {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::network::{TransitNetwork, topology};
    let (mut graph, mut zoning) = painted_road(0.0);
    let all = zoning
        .cells
        .select(CellSelectionShape::Fill, &[DVec2::new(5.0, 10.0)]);
    zoning.paint_cells(&all, 0, |_| false).unwrap();
    let marker = zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::new(55.0, 20.0)]);
    zoning.paint_cells(&marker, 1, |_| false).unwrap();
    let junction = node(&mut graph, 43.0, 0.0);
    topology::split_edge(
        &mut TransitNetwork::new(),
        &mut graph,
        0,
        0,
        43.0 / 120.0,
        junction,
        &mut zoning,
        &mut BuildingAllocator::new(),
    );
    graph.edge_mut(1).no_building_spawn = true;
    zoning.generate_cells(&graph, extent(), |_| false);
    assert!(zoning.cells.pick(DVec2::new(35.0, 10.0)).is_some());
    assert_eq!(zoning.cells.pick(DVec2::new(45.0, 10.0)), None);
    assert_eq!(zoning.cells.pick(DVec2::new(55.0, 10.0)), None);
    assert_eq!(zoning.cells.profile(marker.cells[0]), Some(1));
}

#[test]
fn occupied_lots_follow_split_and_restored_roads_without_moving() {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::network::{TransitNetwork, topology};
    use crate::simulation::zoning::parcels;

    for (angle, erased, no_build) in [0.0, 0.35].into_iter().flat_map(|angle| {
        [false, true].into_iter().flat_map(move |erased| {
            [false, true]
                .into_iter()
                .map(move |no_build| (angle, erased, no_build))
        })
    }) {
        let (mut graph, mut zoning) = painted_road(angle);
        let lots = zoning
            .derive_cell_lots(&graph, extent(), &[size()], |_, _| false)
            .created;
        for (index, id) in lots.iter().enumerate() {
            assert!(zoning.occupy_parcel(id.raw(), index));
        }
        if erased {
            let direction = DVec2::new(angle.cos(), angle.sin());
            let selected = zoning.cells.select(
                CellSelectionShape::Fill,
                &[direction * 5.0 + direction.perp() * 10.0],
            );
            zoning.paint_cells(&selected, 0, |_| false).unwrap();
            assert!(!zoning.cells.has_paint());
        }
        let before: Vec<_> = lots
            .iter()
            .map(|&id| zoning.parcels.get(id).unwrap().clone())
            .collect();
        let restored_graph = graph.clone();
        let split = DVec2::new(angle.cos(), angle.sin()) * 53.0;
        let junction = node(&mut graph, split.x as f32, split.y as f32);
        topology::split_edge(
            &mut TransitNetwork::new(),
            &mut graph,
            0,
            0,
            53.0 / 120.0,
            junction,
            &mut zoning,
            &mut BuildingAllocator::new(),
        );
        if no_build {
            graph.edge_mut(0).no_building_spawn = true;
            graph.edge_mut(1).no_building_spawn = true;
        }
        for current_graph in [&graph, &restored_graph] {
            zoning.generate_cells(current_graph, extent(), |_| false);
            let report = zoning.derive_cell_lots(current_graph, extent(), &[], |_, _| false);
            assert!(report.created.is_empty() && report.retired.is_empty());
            for old in &before {
                let parcel = zoning.parcels.get(old.id()).unwrap();
                assert_eq!(parcel.corners(), old.corners());
                assert_eq!(parcel.occupied_building(), old.occupied_building());
                assert_eq!(parcel.build_generation(), old.build_generation());
                assert_eq!(parcel.cell_lot(), old.cell_lot());
                let expected_edge =
                    if current_graph.edge_count() == 1 || old.frontage_center_t() * 120.0 < 53.0 {
                        0
                    } else {
                        1
                    };
                assert_eq!(parcel.edge_idx(), expected_edge);
                let projected = parcels::project_point_to_edge(
                    current_graph,
                    expected_edge,
                    parcel.front_center(),
                )
                .unwrap();
                assert!(
                    (parcel.frontage_center_t() - projected.s_m / projected.edge_len_m).abs()
                        < 1e-6,
                    "occupied attachment stayed stale: angle={angle} erased={erased} no_build={no_build}"
                );
            }
        }
    }
}

#[test]
fn erased_split_grid_can_be_undone_after_empty_cells_are_regenerated() {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::network::{TransitNetwork, topology};
    let (mut graph, mut zoning) = painted_road(0.0);
    let before = zoning.cells.saved_cells();
    let junction = node(&mut graph, 43.0, 0.0);
    topology::split_edge(
        &mut TransitNetwork::new(),
        &mut graph,
        0,
        0,
        43.0 / 120.0,
        junction,
        &mut zoning,
        &mut BuildingAllocator::new(),
    );
    zoning.generate_cells(&graph, extent(), |_| false);
    let selected = zoning.cells.select(
        CellSelectionShape::Marquee,
        &[DVec2::new(45.0, 10.0), DVec2::new(75.0, 60.0)],
    );
    assert_eq!(selected.cells.len(), 24);
    let edit = zoning.paint_cells(&selected, 0, |_| false).unwrap();
    zoning.generate_cells(&graph, extent(), |_| false);
    assert!(
        selected
            .cells
            .iter()
            .any(|&key| zoning.cells.profile(key).is_none())
    );
    assert!(zoning.restore_cell_gesture(&edit, |_| false).is_some());
    assert_eq!(zoning.cells.saved_cells(), before);
    assert_disjoint(&zoning.cells, extent());
}

#[test]
fn retained_split_alignment_does_not_depend_on_query_bounds_containing_the_paint() {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::network::{TransitNetwork, topology};
    let (mut graph, mut zoning) = painted_road(0.0);
    let full = zoning
        .cells
        .select(CellSelectionShape::Fill, &[DVec2::new(5.0, 10.0)]);
    zoning.paint_cells(&full, 0, |_| false).unwrap();
    let marker = zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::new(85.0, 10.0)]);
    zoning.paint_cells(&marker, 1, |_| false).unwrap();
    let expected = zoning.cells.pick(DVec2::new(95.0, 10.0)).unwrap();
    let junction = node(&mut graph, 43.0, 0.0);
    topology::split_edge(
        &mut TransitNetwork::new(),
        &mut graph,
        0,
        0,
        43.0 / 120.0,
        junction,
        &mut zoning,
        &mut BuildingAllocator::new(),
    );
    let mut narrow = zoning.clone();
    zoning.generate_cells(&graph, extent(), |_| false);
    narrow.generate_cells(
        &graph,
        CellBounds {
            min: DVec2::new(91.0, 6.0),
            max: DVec2::new(99.0, 64.0),
        },
        |_| false,
    );
    assert_eq!(narrow.cells.pick(DVec2::new(95.0, 10.0)), Some(expected));
    assert_eq!(
        narrow.cells.frontages(expected),
        zoning.cells.frontages(expected)
    );
    assert!(!narrow.cells.frontages(expected).is_empty());
}

#[test]
fn disconnected_rear_paint_cannot_create_a_landlocked_lot() {
    let (graph, mut zoning) = painted_road(0.0);
    let front = zoning.cells.select(
        CellSelectionShape::Marquee,
        &[DVec2::new(5.0, 10.0), DVec2::new(115.0, 10.0)],
    );
    zoning.paint_cells(&front, 0, |_| false).unwrap();
    let painted = zoning.cells.saved_cells();
    assert!(!painted.is_empty());
    assert!(
        zoning
            .derive_cell_lots(&graph, extent(), &[size()], |_, _| false)
            .created
            .is_empty()
    );
    assert_eq!(zoning.cells.saved_cells(), painted);
}

#[test]
fn one_changed_rear_cell_can_enable_a_lot_with_frontage_outside_the_edit_bounds() {
    let (graph, mut zoning) = painted_road(0.0);
    let selection = zoning
        .cells
        .select(CellSelectionShape::Fill, &[DVec2::new(5.0, 10.0)]);
    zoning.paint_cells(&selection, 0, |_| false).unwrap();
    let selection = zoning.cells.select(
        CellSelectionShape::Marquee,
        &[DVec2::new(5.0, 10.0), DVec2::new(25.0, 20.0)],
    );
    zoning.paint_cells(&selection, 1, |_| false).unwrap();
    let missing = DVec2::new(15.0, 20.0);
    let selection = zoning.cells.select(CellSelectionShape::Cell, &[missing]);
    zoning.paint_cells(&selection, 0, |_| false).unwrap();
    let bounds = CellBounds {
        min: missing,
        max: missing,
    };
    assert!(
        zoning
            .derive_cell_lots(&graph, bounds, &[size()], |_, _| false)
            .created
            .is_empty()
    );
    let selection = zoning.cells.select(CellSelectionShape::Cell, &[missing]);
    zoning.paint_cells(&selection, 1, |_| false).unwrap();
    let report = zoning.derive_cell_lots(&graph, bounds, &[size()], |_, _| false);
    assert_eq!(report.created.len(), 1);
}

#[test]
fn orthogonal_corner_retains_both_road_frontage_options() {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    let c = node(&mut graph, 0.0, 120.0);
    road(&mut graph, a, b);
    road(&mut graph, a, c);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    zoning.generate_cells(&graph, extent(), |_| false);
    let corner = zoning.cells.pick(DVec2::splat(10.0)).unwrap();
    let links = zoning.cells.frontages(corner);
    assert_eq!(links.len(), 2);
    assert_ne!(links[0].boundary, links[1].boundary);
    let rear = zoning.cells.pick(DVec2::splat(30.0)).unwrap();
    assert!(zoning.cells.frontages(rear).is_empty());
}

#[test]
fn no_build_road_cannot_spawn_from_stale_frontage_cache() {
    let (mut graph, mut zoning) = painted_road(0.0);
    graph.edge_mut(0).no_building_spawn = true;
    assert!(
        zoning
            .derive_cell_lots(&graph, extent(), &[size()], |_, _| false)
            .created
            .is_empty()
    );
    let paint = zoning.cells.saved_cells();
    zoning.generate_cells(&graph, extent(), |_| false);
    assert_eq!(zoning.cells.saved_cells(), paint);
    assert!(
        paint
            .iter()
            .all(|&(key, _)| zoning.cells.frontages(key).is_empty())
    );
}

#[test]
fn widened_source_road_rejects_stale_frontage_even_for_a_millimetre_overlap() {
    let (mut graph, mut zoning) = painted_road(0.35);
    let paint = zoning.cells.saved_cells();
    graph.edge_mut(0).width += 0.002;
    assert!(
        zoning
            .derive_cell_lots(&graph, extent(), &[size()], |_, _| false)
            .created
            .is_empty()
    );
    assert_eq!(zoning.cells.saved_cells(), paint);
}

#[test]
fn lot_blockers_do_not_erase_paint_or_claim_blocked_land() {
    let (graph, mut zoning) = painted_road(0.0);
    let paint = zoning.cells.saved_cells();
    let report = zoning.derive_cell_lots(&graph, extent(), &[size()], |corners, _| {
        CellBounds::from_points(*corners).min.x < 60.0
    });
    assert_eq!(report.created.len(), 2);
    assert_eq!(zoning.cells.saved_cells(), paint);
    assert!(zoning.parcels().iter().all(|p| p.aabb_min().x >= 60.0));
}

#[test]
fn asset_order_and_worker_count_do_not_change_lot_products() {
    let (graph, zoning) = painted_road(0.35);
    let mut previous = None;
    for workers in [1, 4] {
        let mut zoning = zoning.clone();
        let mut sizes = vec![
            size(),
            CellLotSize {
                width: 2,
                depth: 3,
                ..size()
            },
            size(),
        ];
        if workers == 4 {
            sizes.reverse();
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let report =
            pool.install(|| zoning.derive_cell_lots(&graph, extent(), &sizes, |_, _| false));
        let products: Vec<_> = report
            .created
            .into_iter()
            .map(|id| {
                let p = zoning.parcels.get(id).unwrap();
                (id, p.cell_lot(), p.corners())
            })
            .collect();
        if let Some(previous) = previous {
            assert_eq!(products, previous);
        }
        previous = Some(products);
    }
}

#[test]
#[ignore = "matched release timing of local lot derivation and retirement, excluding setup"]
fn benchmark_cell_lot_derivation_locality() {
    use std::hint::black_box;
    use std::time::Instant;
    for remote in [0, 1_024, 10_000] {
        let (mut graph, mut zoning) = painted_road(0.35);
        let grid = zoning
            .cells
            .register_frame(GridFrame::new(DVec2::ZERO, DVec2::X, 10.0).unwrap());
        // Minimal background stores isolate indexed lot work from gameplay/road-surface setup.
        for index in 0..remote {
            let x = 200 + (index % 128) as i32 * 6;
            let y = 200 + (index / 128) as i32 * 6;
            let a = node(&mut graph, x as f32 * 10.0, y as f32 * 10.0 - 20.0);
            let b = node(&mut graph, x as f32 * 10.0 + 30.0, y as f32 * 10.0 - 20.0);
            let edge = road(&mut graph, a, b);
            let lot = CellLot::new(CellKey { grid, x, y }, 2, 3, CellFrontage::MinY).unwrap();
            for key in lot.cells() {
                assert!(zoning.cells.insert(key));
                zoning.cells.set_profile(key, 1);
            }
            zoning
                .install_prevalidated_cell_lot(lot, edge, 1, 0.5, 1)
                .unwrap();
        }
        let run = |zoning: &mut ZoningSystem| {
            let report = zoning.derive_cell_lots(&graph, extent(), &[size()], |_, _| false);
            assert_eq!(report.created.len(), 4);
            assert!(report.retired.is_empty());
            let retired = zoning.derive_cell_lots(&graph, extent(), &[], |_, _| false);
            assert_eq!(retired.retired, report.created);
            assert_eq!(zoning.parcels().len(), remote);
            black_box(report.candidates)
        };
        for _ in 0..16 {
            run(&mut zoning);
        }
        let mut samples = [0.0_f64; 21];
        let mut proposals = 0;
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..64 {
                proposals = run(black_box(&mut zoning));
            }
            *sample = start.elapsed().as_secs_f64() * 1e6 / 64.0;
        }
        samples.sort_unstable_by(f64::total_cmp);
        println!(
            "remote_roads={remote} remote_lots={remote} remote_cells={} proposals={proposals} derive_retire_us={:.3}",
            remote * 6,
            samples[10]
        );
    }
}

#[test]
#[ignore = "matched release timing of retained painted frontage after a split, excluding setup"]
fn benchmark_cell_split_frontage_locality() {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::network::{TransitNetwork, topology};
    use std::hint::black_box;
    use std::time::Instant;

    for (occupied, remote) in [false, true]
        .into_iter()
        .flat_map(|occupied| [0, 1_024, 10_000].map(|remote| (occupied, remote)))
    {
        let (mut graph, mut zoning) = painted_road(0.35);
        let original_edge = graph.edge(0).clone();
        let original = zoning
            .derive_cell_lots(&graph, extent(), &[size()], |_, _| false)
            .created;
        assert_eq!(original.len(), 4);
        if occupied {
            for (index, id) in original.iter().enumerate() {
                assert!(zoning.occupy_parcel(id.raw(), index));
            }
        }
        let direction = DVec2::new(0.35_f64.cos(), 0.35_f64.sin());
        let junction = node(
            &mut graph,
            (direction.x * 43.0) as f32,
            (direction.y * 43.0) as f32,
        );
        topology::split_edge(
            &mut TransitNetwork::new(),
            &mut graph,
            0,
            0,
            43.0 / 120.0,
            junction,
            &mut zoning,
            &mut BuildingAllocator::new(),
        );
        let paint = zoning.cells.saved_cells();
        let grid = zoning
            .cells
            .register_frame(GridFrame::new(DVec2::ZERO, DVec2::X, 10.0).unwrap());
        // All distant population and the topology edit stay outside the repeated query.
        for index in 0..remote {
            let x = 200 + (index % 128) as i32 * 6;
            let y = 200 + (index / 128) as i32 * 6;
            let a = node(&mut graph, x as f32 * 10.0, y as f32 * 10.0 - 20.0);
            let b = node(&mut graph, x as f32 * 10.0 + 30.0, y as f32 * 10.0 - 20.0);
            let edge = road(&mut graph, a, b);
            let lot = CellLot::new(CellKey { grid, x, y }, 2, 3, CellFrontage::MinY).unwrap();
            for key in lot.cells() {
                assert!(zoning.cells.insert(key));
                zoning.cells.set_profile(key, 1);
            }
            let id = zoning
                .install_prevalidated_cell_lot(lot, edge, 1, 0.5, 1)
                .unwrap();
            if occupied {
                assert!(zoning.occupy_parcel(id.raw(), original.len() + index));
            }
        }
        // Restore only the local source edge in a second immutable topology fixture. Keep
        // every distant edge id unchanged; graph cloning/index setup are outside timing.
        let restored = occupied.then(|| {
            let mut restored = graph.clone();
            restored.remove_from_spatial_index(0);
            restored.remove_from_spatial_index(1);
            *restored.edge_mut(0) = original_edge;
            restored.edge_mut(1).deleted = true;
            restored.add_to_spatial_index(0);
            restored.rebuild_adjacency_list();
            restored
        });
        let graphs: Vec<_> = std::iter::once(&graph).chain(restored.as_ref()).collect();
        // Production road edits prepare grid authority before generated cells. Distant road
        // choices are populated outside timing; each measured edit still visits only 0/1.
        zoning
            .cells
            .refresh_road_alignments(&graph, &zoning.config, 0..graph.edge_count());
        let expected: Vec<_> = graphs
            .iter()
            .map(|graph| {
                zoning
                    .cells
                    .refresh_road_alignments(graph, &zoning.config, [0, 1]);
                zoning.generate_cells(graph, extent(), |_| false)
            })
            .collect();
        let expected_keys = keys(&zoning.cells, extent());
        let run = |zoning: &mut ZoningSystem| {
            for (graph, expected) in graphs.iter().zip(&expected) {
                zoning
                    .cells
                    .refresh_road_alignments(graph, &zoning.config, [0, 1]);
                let generated = zoning.generate_cells(graph, extent(), |_| false);
                let lots = zoning.derive_cell_lots(graph, extent(), &[size()], |_, _| false);
                assert_eq!(generated, *expected);
                assert!(lots.created.is_empty());
                assert!(lots.retired.is_empty());
                if occupied {
                    assert_eq!(lots.reattached.len(), original.len());
                }
                black_box(generated);
            }
        };
        // Seed the last topology's attachment state before alternating split/restore refresh.
        zoning.derive_cell_lots(graphs[graphs.len() - 1], extent(), &[size()], |_, _| false);
        for _ in 0..16 {
            run(&mut zoning);
        }
        let mut samples = [0.0_f64; 21];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..16 {
                run(black_box(&mut zoning));
            }
            *sample = start.elapsed().as_secs_f64() * 1e6 / (16.0 * graphs.len() as f64);
        }
        samples.sort_unstable_by(f64::total_cmp);
        assert_eq!(keys(&zoning.cells, extent()), expected_keys);
        assert!(
            paint
                .iter()
                .all(|&(key, profile)| zoning.cells.profile(key) == Some(profile))
        );
        assert!(original.iter().all(|&id| zoning.parcels.get(id).is_some()));
        println!(
            "occupied={} remote_roads={remote} remote_lots={remote} remote_cells={} split_refresh_us={:.3} products={expected:?}",
            usize::from(occupied),
            remote * 6,
            samples[10]
        );
    }
}
