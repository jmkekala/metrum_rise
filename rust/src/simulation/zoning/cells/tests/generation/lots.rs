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
fn releasing_an_occupied_lot_with_intact_paint_derives_without_regenerating_cells() {
    use crate::simulation::zoning::{CellLotGeneration, MAX_PARCEL_FRONTAGE_M};
    // Serves every queued region as the `cell lots` queue does; returns the roads that cell
    // generation queried (zero when no generation ran) and each region's derivation.
    fn serve(graph: &RegionGraph, zoning: &mut ZoningSystem) -> (usize, Vec<CellLotGeneration>) {
        let (mut roads, mut reports) = (0, Vec::new());
        while let Some(region) = zoning.pop_dirty_cell_lot_region() {
            let wide = region.expanded(f64::from(MAX_PARCEL_FRONTAGE_M));
            roads += zoning.prepare_generated_cells(graph, wide, |_| false).roads;
            reports.push(zoning.derive_cell_lots(graph, region, &[size()], |_, _| false));
        }
        (roads, reports)
    }
    let cells = |zoning: &ZoningSystem| {
        keys(&zoning.cells, extent().expanded(512.0))
            .into_iter()
            .map(|key| {
                let store = &zoning.cells;
                (
                    key,
                    store.profile(key),
                    store.lot(key),
                    store.frontages(key).to_vec(),
                )
            })
            .collect::<Vec<_>>()
    };
    for angle in [0.0, 0.35, 0.8] {
        let (graph, mut zoning) = painted_road(angle);
        let lots = zoning
            .derive_cell_lots(&graph, extent(), &[size()], |_, _| false)
            .created;
        for (index, id) in lots.iter().enumerate() {
            assert!(zoning.occupy_parcel(id.raw(), index));
        }
        serve(&graph, &mut zoning);
        zoning.prepare_generated_cells(&graph, extent().expanded(1024.0), |_| false);
        let released = lots[1];
        let lot = zoning.parcels.get(released).unwrap().cell_lot().unwrap();
        let frame = zoning.cells.frame(lot.origin().grid).unwrap();
        assert!(zoning.clear_parcel_occupancy(released.raw()));
        assert!(zoning.has_dirty_cell_lots());
        // Before `SIM-02-K` part 5 the release also invalidated the lot's halo of generated
        // cells, which the region then regenerated.
        let mut regenerated = zoning.clone();
        regenerated.cells.invalidate_generated_cells(
            CellBounds::from_points(lot.corners(frame))
                .expanded((CELL_DEPTH + 1) as f64 * f64::from(zoning.config.zone_cell_m)),
        );
        let (kept_roads, kept) = serve(&graph, &mut zoning);
        let (regenerated_roads, again) = serve(&graph, &mut regenerated);
        assert_eq!(
            kept_roads, 0,
            "angle {angle}: the release regenerated cells"
        );
        assert!(regenerated_roads > 0, "angle {angle}");
        assert_eq!(format!("{kept:?}"), format!("{again:?}"), "angle {angle}");
        assert_eq!(cells(&zoning), cells(&regenerated), "angle {angle}");
        assert_eq!(zoning.cells.revision(), regenerated.cells.revision());
        assert_eq!(
            format!("{:?}", zoning.parcels.parcels()),
            format!("{:?}", regenerated.parcels.parcels())
        );
        assert!(zoning.parcels.get(released).unwrap().is_available());
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

// `ROAD-109`: the paint on a side that lost its sidewalk keeps its cells and spawns nothing; the
// fixture paints the road's right, as a closed road's test above does.
#[test]
fn a_lost_sidewalk_cannot_spawn_from_stale_frontage_cache() {
    let (mut graph, mut zoning) = painted_road(0.0);
    graph
        .edge_mut(0)
        .set_sidewalks(crate::simulation::network::graph::SidewalkSides::LEFT);
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

// `ROAD-68` open question 32: one planner on the live store's plain maps and on the road tool
// view's persistent copy plans equal products, and the copy follows the live store's journal.
#[test]
fn a_plan_on_the_views_persistent_copy_equals_the_live_plan_and_the_copy_follows_it() {
    use crate::simulation::zoning::cells::{CellStore, SharedMaps};

    let (graph, mut zoning) = painted_road(0.35);
    let mut view = CellStore::copy_of(&zoning.cells);
    zoning.cells.clear_journal();
    assert_eq!(view.state_digest(), zoning.cells.state_digest());
    let mut planned: CellStore<SharedMaps> = view.clone();
    // Both regenerate every cell and derive lots on it.
    zoning.cells.invalidate_generated_cells(extent());
    planned.invalidate_generated_cells(extent());
    let mut live = std::mem::take(&mut zoning.cells);
    let (live_report, live_plan) = zoning.plan_cell_lots_on(&mut live, &graph, extent(), &[size()]);
    zoning.cells = live;
    let (copy_report, copy_plan) =
        zoning.plan_cell_lots_on(&mut planned, &graph, extent(), &[size()]);
    assert_eq!(live_report.created.len(), 4);
    assert_eq!(format!("{live_report:?}"), format!("{copy_report:?}"));
    assert_eq!(format!("{live_plan:?}"), format!("{copy_plan:?}"));
    assert_eq!(planned.state_digest(), zoning.cells.state_digest());
    assert_eq!(planned.digests(), zoning.cells.digests());
    // The view, untouched by the plan on its clone, follows the live store's journal.
    assert_ne!(view.state_digest(), zoning.cells.state_digest());
    view.follow(&mut zoning.cells);
    assert_eq!(view.state_digest(), zoning.cells.state_digest());
    // A clear makes the next follow copy the store whole.
    zoning.cells.clear();
    view.follow(&mut zoning.cells);
    assert_eq!(view.state_digest(), zoning.cells.state_digest());
}

// `ZONE-09` (PLAN-RZ 2.4), `ZONE-14` (PLAN-RL 2.2): a road on land an edit took. Its frontage cell
// `(x, y)` on the road's frame, the road's frame and a plan of `bounds` handing `sources` (erased
// cells and their paint) on to the cells it lays, on a zoning that has its cells already
// generated or not. With cells generated before, the road frame's cells at the lattice
// coordinates `covered` are the edit's covered cells: the plan's sites block them. Returns the
// zoning after the plan's installation and the paint handed on.
fn inherit_along_a_road(
    sources: impl FnOnce(&mut ZoningSystem, GridFrame, i32, i32) -> Vec<(CellKey, u16)>,
    generated_before: bool,
    covered: &[(i32, i32)],
) -> (ZoningSystem, Option<crate::simulation::zoning::RoadPaint>) {
    let mut graph = RegionGraph::new();
    let a = node(&mut graph, 0.0, 0.0);
    let b = node(&mut graph, 120.0, 0.0);
    road(&mut graph, a, b);
    // Where the road's cells fall, read from a generation on a copy.
    let mut probe = ZoningSystem::new(&WorldConfig::default());
    probe.generate_cells(&graph, extent(), |_| false);
    let key = probe.cells.pick(DVec2::new(55.0, 12.0)).unwrap();
    let frame = probe.cells.frame(key.grid).unwrap();
    assert_eq!(frame.axes().0, DVec2::X);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    if generated_before {
        zoning.generate_cells(&graph, extent(), |_| false);
    }
    let mut covered: Vec<_> = covered
        .iter()
        .map(|&(dx, dy)| road_cell(&zoning, frame, key.x + dx, key.y + dy))
        .collect();
    covered.sort_unstable();
    let footprints: Vec<_> = covered
        .iter()
        .map(|key| frame.corners(key.x, key.y))
        .collect();
    let paint = crate::simulation::zoning::RoadPaint {
        erase: 7,
        cells: sources(&mut zoning, frame, key.x, key.y),
        covered,
    };
    let blocked = |corners: &[DVec2; 4]| {
        footprints
            .iter()
            .any(|footprint| interiors_overlap(footprint, corners))
    };
    let mut planner = zoning.cell_lot_planner().inheriting(Some(paint));
    planner.plan_region(&graph, extent(), &[size()], &blocked);
    let inherited = planner.take_inherited();
    let plan = planner.finish();
    zoning.install_cell_lot_plan(plan);
    (zoning, inherited)
}

// The road frame's cell `(x, y)` in `zoning`, by its centre.
fn road_cell(zoning: &ZoningSystem, frame: GridFrame, x: i32, y: i32) -> CellKey {
    let key = zoning
        .cells
        .pick(frame.world(f64::from(x) + 0.5, f64::from(y) + 0.5))
        .unwrap();
    assert_eq!(zoning.cells.frame(key.grid), Some(frame));
    assert_eq!((key.x, key.y), (x, y));
    key
}

// The cell of `frame` (registered as `grid`) holding the point `along` cells along and half a
// cell into row `y` of the road's frame `road`.
fn cell_at(frame: GridFrame, grid: u64, road: GridFrame, along: f64, y: i32) -> CellKey {
    let local = frame.local(road.world(along, f64::from(y) + 0.5)).floor();
    CellKey {
        grid,
        x: local.x as i32,
        y: local.y as i32,
    }
}

// Rule 5: each new cell takes the paint of the erased cell it overlaps most; a tie goes to the
// lowest erased key; a new cell over no erased cell stays unpainted; lots derive on inherited
// paint in the same plan.
#[test]
fn new_frontage_cells_inherit_the_most_overlapped_removed_paint() {
    // Road cells by lattice coordinates, with the paint each must inherit.
    let mut expected = Vec::new();
    let (zoning, inherited) = inherit_along_a_road(
        |zoning, road, x, y| {
            let (x, yf) = (f64::from(x), y);
            // Erased cells 3 m along the road from its lattice: cell `x` overlaps the one 3 m
            // into it by 70 m² (profile 2) and the one before by 30 m² (profile 1); cell `x - 1`
            // takes the one before, cell `x + 1` the one into `x`, by 70 and 30 m².
            let shifted = road.moved(0, 3.0);
            let s = zoning.cells.register_frame(shifted);
            // Two zones meeting half way along cell `x + 5`, 5 m off the lattice: it overlaps
            // both by 50 m², and the lower key's paint (profile 2) wins; `x + 4` and `x + 6`
            // take the one each overlaps.
            let halved = road.moved(0, 5.0);
            let h = zoning.cells.register_frame(halved);
            let first = cell_at(halved, h, road, x + 4.55, yf);
            let second = cell_at(halved, h, road, x + 5.55, yf);
            assert!(first < second);
            let (x0, y0) = (x as i32, y);
            expected = vec![
                ((x0 - 1, y0), 1),
                ((x0, y0), 2),
                ((x0 + 1, y0), 2),
                ((x0 + 4, y0), 2),
                ((x0 + 5, y0), 2),
                ((x0 + 6, y0), 1),
            ];
            let mut sources = vec![
                (cell_at(shifted, s, road, x - 0.65, yf), 1),
                (cell_at(shifted, s, road, x + 0.35, yf), 2),
                (first, 2),
                (second, 1),
                // Far from the road, where no cell is generated.
                (cell_at(shifted, s, road, x, yf + 40), 1),
            ];
            // Across the road, two rows of three erased cells 1 m off the lattice: four columns
            // of two rows inherit (the fourth by 10 m²), deep enough for a 3 x 2 lot.
            let across = road.local(DVec2::new(55.0, -12.0)).floor();
            let (ax, ay) = (across.x as i32, across.y as i32);
            let nudged = road.moved(0, 1.0);
            let n = zoning.cells.register_frame(nudged);
            for row in [ay, ay - 1] {
                for column in ax - 1..=ax + 1 {
                    sources.push((cell_at(nudged, n, road, f64::from(column) + 0.15, row), 1));
                }
                expected.extend((ax - 1..=ax + 2).map(|column| ((column, row), 1)));
            }
            sources.sort_unstable();
            sources
        },
        false,
        &[],
    );
    let inherited = inherited.expect("new cells inherited paint");
    assert_eq!(inherited.erase, 7);
    let key = zoning.cells.pick(DVec2::new(55.0, 12.0)).unwrap();
    let frame = zoning.cells.frame(key.grid).unwrap();
    let mut want: Vec<_> = expected
        .iter()
        .map(|&((x, y), profile)| (road_cell(&zoning, frame, x, y), profile))
        .collect();
    want.sort_unstable();
    assert_eq!(inherited.cells, want);
    // Every other cell stays unpainted.
    let painted: Vec<_> = keys(&zoning.cells, extent())
        .into_iter()
        .filter_map(|key| Some((key, zoning.cells.profile(key).filter(|&p| p != 0)?)))
        .collect();
    assert_eq!(painted, want);
    // The two inherited rows across the road hold a 3 x 2 lot, derived in the same plan.
    let claimed = want
        .iter()
        .filter(|&&(key, _)| zoning.cells.lot(key).is_some_and(|lot| lot != 0))
        .count();
    assert_eq!(claimed, 6);
}

// `ZONE-14` (PLAN-RL 2.2): a cell standing when the plan began, released and regenerated in
// place, takes the paint of the erased cell it overlaps most, as a new one does; a covered cell
// never inherits (here blocked, as the road's pieces block it), and the cell beside it still
// inherits the erased cell's remainder.
#[test]
fn standing_cells_inherit_and_covered_cells_do_not() {
    // An erased cell 3 m along the road from the lattice: road cell `x` overlaps it by 70 m²,
    // cell `x + 1` by 30 m².
    let source = |zoning: &mut ZoningSystem, frame: GridFrame, x: i32, y: i32| {
        let shifted = zoning.cells.register_frame(frame.moved(0, 3.0));
        vec![(cell_at(frame.moved(0, 3.0), shifted, frame, f64::from(x) + 0.35, y), 2)]
    };
    let road_cells = |zoning: &ZoningSystem, inherited: &crate::simulation::zoning::RoadPaint| {
        // Road cell `x + 1`, which stands in both cases.
        let key = zoning.cells.pick(DVec2::new(65.0, 12.0)).unwrap();
        let frame = zoning.cells.frame(key.grid).unwrap();
        inherited
            .cells
            .iter()
            .map(|&(cell, profile)| {
                assert_eq!(zoning.cells.frame(cell.grid), Some(frame));
                ((cell.x - key.x + 1, cell.y - key.y), profile)
            })
            .collect::<Vec<_>>()
    };
    let (zoning, inherited) = inherit_along_a_road(source, true, &[]);
    let inherited = inherited.expect("the standing cells inherit");
    assert_eq!(road_cells(&zoning, &inherited), [((0, 0), 2), ((1, 0), 2)]);
    let (zoning, inherited) = inherit_along_a_road(source, true, &[(0, 0)]);
    let inherited = inherited.expect("the uncovered cell inherits");
    assert_eq!(road_cells(&zoning, &inherited), [((1, 0), 2)]);
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

// `ZONE-09` (PLAN-RZ section 5, test 8): a lot plan handing a road stroke's erased paint on to the
// new cells of a new road, its neighbourhood fixed while remote roads, lots and cells grow as in
// `benchmark_cell_lot_derivation_locality`. Each sample plans the new road's region on a copy
// made outside timing, with the erased cells and without (the control); the inherited cells and
// the plan's lots must not change with the background.
#[test]
#[ignore = "matched release timing of a lot plan with paint inheritance, excluding setup"]
fn benchmark_cell_inheritance_locality() {
    use std::hint::black_box;
    use std::time::Instant;
    let mut reference = None;
    for remote in [0_usize, 1_024, 10_000] {
        let (mut graph, mut zoning) = painted_road(0.35);
        let grid = zoning
            .cells
            .register_frame(GridFrame::new(DVec2::ZERO, DVec2::X, 10.0).unwrap());
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
        // The new road, west of the painted one, and its region, invalidated as a commit marks
        // it; its frame read from a generation on a copy.
        let a = node(&mut graph, -250.0, -200.0);
        let b = node(&mut graph, -130.0, -200.0);
        road(&mut graph, a, b);
        let region = CellBounds {
            min: DVec2::new(-300.0, -300.0),
            max: DVec2::new(-80.0, -100.0),
        };
        zoning.cells.invalidate_generated_cells(region);
        let mut probe = zoning.clone();
        probe.generate_cells(&graph, region, |_| false);
        let key = probe.cells.pick(DVec2::new(-195.0, -188.0)).unwrap();
        let frame = probe.cells.frame(key.grid).unwrap();
        drop(probe);
        // Two rows of six erased cells 3 m along the road from its lattice: seven columns of two
        // rows inherit, deep enough for 3 x 2 lots.
        let shifted = frame.moved(0, 3.0);
        let s = zoning.cells.register_frame(shifted);
        let mut sources: Vec<_> = (0..2)
            .flat_map(|row| {
                (0..6).map(move |column| {
                    let point = frame.world(f64::from(key.x + column) + 0.35, f64::from(key.y + row) + 0.5);
                    let local = shifted.local(point).floor();
                    (CellKey { grid: s, x: local.x as i32, y: local.y as i32 }, 1)
                })
            })
            .collect();
        sources.sort_unstable();
        let plan = |zoning: &mut ZoningSystem, inherit: bool| {
            let paint = inherit.then(|| crate::simulation::zoning::RoadPaint {
                erase: 1,
                cells: sources.clone(),
                covered: Vec::new(),
            });
            let start = Instant::now();
            let mut planner = zoning.cell_lot_planner().inheriting(paint);
            let report = planner.plan_region(&graph, region, &[size()], &|_| false);
            let inherited = planner.take_inherited();
            black_box(planner.finish());
            (start.elapsed().as_secs_f64() * 1e6, report.created.len(), inherited)
        };
        let mut inherit_us = Vec::new();
        let mut control_us = Vec::new();
        let mut products = None;
        for _ in 0..21 {
            let mut copy = zoning.clone();
            let (us, created, inherited) = plan(&mut copy, true);
            inherit_us.push(us);
            let mut copy = zoning.clone();
            control_us.push(plan(&mut copy, false).0);
            let inherited = inherited.expect("the new road's cells inherit").cells;
            let product = (created, format!("{inherited:?}"));
            assert!(products.as_ref().is_none_or(|previous| *previous == product));
            products = Some(product);
        }
        let (created, inherited) = products.unwrap();
        match &reference {
            None => reference = Some((created, inherited.clone())),
            Some(reference) => assert_eq!(reference, &(created, inherited.clone())),
        }
        inherit_us.sort_unstable_by(f64::total_cmp);
        control_us.sort_unstable_by(f64::total_cmp);
        println!(
            "INHERITANCE_LOCALITY remote_roads={remote} remote_lots={remote} remote_cells={} inherited={} lots_created={created} plan_inherit_us={:.1} plan_control_us={:.1}",
            remote * 6,
            inherited.matches("CellKey").count(),
            inherit_us[10],
            control_us[10]
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
