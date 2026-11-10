// SPDX-License-Identifier: GPL-2.0-only

//! Painted land re-lays around a road edit as unpainted land does (`ZONE-14`, round RL1,
//! PLAN-RL section 4): the edit's land take releases the free paint of the region it regenerates
//! (`ZoningSystem::release_painted_cells`), the lot plan lays that region's cells as on unpainted
//! land, and each cell over the old paint takes the paint it overlaps most. An occupied lot stays
//! pinned with its cells; paint without a road column vanishes; undo gives the old cells and
//! paint back exactly, and is refused whole while a building stands anywhere on the re-laid land.
//! A ring over a zoned crossroads leaves no cell on its island (`ZONE-13`).

use super::*;
use crate::simulation::network::roundabouts::{
    RoundaboutLayout, RoundaboutRequest, RoundaboutSite,
};
use crate::simulation::zoning::cells::CellSelectionShape;

// The chunks `AREA` touches.
const AREA_CHUNKS: [(i32, i32); 4] = [(-1, -1), (-1, 0), (0, -1), (0, 0)];

// The end of a frame after an edit: the road tool's view published (an edit's offered graph
// copy is consumed before the next edit, as the frame loop does), and every cell chunk of `AREA`
// generated, as the overlay would show it: cold chunks of a core that never painted are
// generated from the roads.
fn settle(core: &mut SimCore) {
    core.precompute_road_mesh_data();
    core.publish_road_tool_view();
    core.prepare_cell_lots_internal();
    for chunk in AREA_CHUNKS {
        assert!(core.prepare_cell_chunk_internal(chunk));
    }
}

// Applies one cell tool gesture of `shape` along `path` with `profile`.
fn paint(core: &mut SimCore, shape: CellSelectionShape, path: &[DVec2], profile: u16) {
    let preview = core.preview_cell_selection_internal(shape, path);
    assert!(!preview.selection.cells.is_empty(), "{shape:?} at {path:?}");
    core.apply_cell_selection_internal(&preview, profile);
}

// The street of `street_on` with both curbs painted whole (profile 1) by the fill tool, and,
// given `east`, the south curb east of x = -25 painted with it: the screenshot's block.
fn curbs_painted(east: Option<u16>) -> SimCore {
    let (mut core, _) = street_on(|_| {});
    for z in [15.0, -15.0] {
        paint(
            &mut core,
            CellSelectionShape::Fill,
            &[DVec2::new(0.0, z)],
            1,
        );
    }
    if let Some(profile) = east {
        let corners = [DVec2::new(-25.0, -6.0), DVec2::new(160.0, -80.0)];
        paint(&mut core, CellSelectionShape::Marquee, &corners, profile);
    }
    settle(&mut core);
    core
}

// The plain street of `street_on` with nothing painted after the diagonal: the unpainted case.
fn unpainted_diagonal() -> SimCore {
    let (mut core, _) = street_on(|_| {});
    assert_eq!(commit(&mut core, &DIAGONAL), "");
    core.adopt_pending_cell_lots();
    settle(&mut core);
    core
}

// Every painted cell in `AREA` carries the paint of the cell of `before` it overlaps most
// (PLAN-RL test 1 (b)): the oracle of the stroke's inheritance over the land's old paint.
fn assert_paint_follows(core: &SimCore, before: &[(CellKey, u16)], label: &str) {
    for (key, profile) in painted(core) {
        let footprint = corners(core, key);
        let (mut best, mut best_area) = (None, 0.0);
        for &(source, paint) in before {
            let area = convex_overlap_area(&footprint, &corners(core, source));
            if area > best_area {
                (best, best_area) = (Some(paint), area);
            }
        }
        assert_eq!(best, Some(profile), "{label}: {key:?}");
    }
}

// Every cell in `AREA`.
fn cell_keys(core: &SimCore) -> Vec<CellKey> {
    let mut keys = Vec::new();
    core.zoning
        .cells
        .visit_in_bounds(AREA, |key| keys.push(key));
    keys.sort_unstable();
    keys.dedup();
    keys
}

// The painted cells in `AREA` claimed by a lot.
fn claimed(core: &SimCore) -> Vec<CellKey> {
    painted(core)
        .into_iter()
        .filter(|&(key, _)| core.zoning.cells.lot(key).is_some_and(|lot| lot != 0))
        .map(|(key, _)| key)
        .collect()
}

// Test 1, the screenshot (rules 1, 2, 4, 7): a diagonal through a block painted whole leaves no
// sliver and no gap: the cells in the area are those of the same stroke on the unpainted street,
// each painted one carries the paint it overlaps most, no cell over the old paint is unpainted,
// and the stroke released paint beyond the cells it covers.
#[test]
fn a_diagonal_through_a_painted_block_re_lays_it_as_unpainted_land() {
    let mut core = curbs_painted(None);
    let before = painted(&core);
    let (erased, inherited) = commit_diagonal(&mut core);
    settle(&mut core);
    assert!(
        erased.iter().any(|&(key, _)| !under_road(&core, key)),
        "the stroke releases paint it does not cover"
    );
    assert!(!inherited.is_empty());
    assert_eq!(
        layout(&core),
        layout(&unpainted_diagonal()),
        "the re-laid cells"
    );
    assert_paint_follows(&core, &before, "diagonal");
    assert_relaid(&core, &before, "diagonal");
    assert_references_consistent(&core);
}

// Test 7: the re-lay is a function of its inputs: two cities built alike end bit for bit equal
// (cells, land, the record), and the plan made in the commit equals the deferred one.
#[test]
fn the_re_lay_is_deterministic() {
    let run = |in_commit: bool| {
        let mut core = curbs_painted(Some(2));
        core.cell_lot_hooks.in_commit = in_commit;
        assert_eq!(commit(&mut core, &DIAGONAL), "");
        core.adopt_pending_cell_lots();
        let record = core
            .undo_stack
            .back()
            .and_then(|record| record.cells.as_ref())
            .map(|cells| (cells.erased().to_vec(), cells.inherited().to_vec()));
        (core.zoning.cells.state_digest(), land_state(&core), record)
    };
    let first = run(false);
    assert_eq!(run(false), first);
    assert_eq!(run(true), first, "in-commit plan");
}

// Test 2, an occupied lot (rule 3): the houses the stroke does not take keep their lots and
// cells, pinned as their claims pin unpainted land: the cells are those of the same stroke on the
// street with the same houses and no other paint.
#[test]
fn occupied_lots_stay_pinned_while_the_free_paint_re_lays() {
    let (mut core, houses) = street_with_houses();
    let mut twin = street_with_houses().0;
    erase_free_paint(&mut twin);
    for point in [DVec2::new(0.0, 15.0), DVec2::new(0.0, -35.0)] {
        paint(&mut core, CellSelectionShape::Fill, &[point], 1);
    }
    settle(&mut core);
    let lots = lots_of(&core, &houses);
    let before = painted(&core);
    commit_diagonal(&mut core);
    settle(&mut core);
    assert_eq!(commit(&mut twin, &DIAGONAL), "");
    twin.adopt_pending_cell_lots();
    settle(&mut twin);
    let mut kept = 0;
    for (id, cells) in &lots {
        let Some(parcel) = core.zoning.parcel_by_raw_id(*id) else {
            continue;
        };
        kept += 1;
        assert!(!parcel.is_available(), "lot {id} keeps its house");
        assert_eq!(
            parcel
                .cell_lot()
                .map(|lot| lot.cells().collect::<Vec<_>>())
                .as_ref(),
            Some(cells)
        );
        for &key in cells {
            assert_eq!(core.zoning.cells.profile(key), Some(1), "{key:?}");
            assert_eq!(core.zoning.cells.lot(key), Some(*id), "{key:?}");
        }
    }
    assert!(
        kept > 0 && kept < lots.len(),
        "the stroke takes some houses: {kept}"
    );
    assert_eq!(layout(&core), layout(&twin), "the re-laid cells");
    assert_relaid(&core, &before, "beside houses");
    assert_references_consistent(&core);
}

// Test 3 (rule 5): undo gives the old cells and their paint back exactly, and the lots derive on
// the same cells again.
#[test]
fn undo_of_a_re_lay_restores_the_cells_and_paint_exactly() {
    let mut core = curbs_painted(Some(2));
    let (paint_before, keys_before, claimed_before) =
        (painted(&core), cell_keys(&core), claimed(&core));
    commit_diagonal(&mut core);
    assert_ne!(
        cell_keys(&core),
        keys_before,
        "the stroke re-laid the block"
    );
    assert!(core.undo_action_internal());
    settle(&mut core);
    assert_eq!(painted(&core), paint_before);
    assert_eq!(cell_keys(&core), keys_before);
    assert_eq!(claimed(&core), claimed_before);
    assert_references_consistent(&core);
}

// Test 3, decision D5 extended (D5-ext (a)): a house grown on a lot of the re-laid block, on the
// old street's own grid away from the new road, holds land the undo would give back, so the undo
// is refused whole and changes nothing; once the house is gone the undo runs.
#[test]
fn a_house_anywhere_on_the_re_laid_land_refuses_the_undo() {
    let mut core = curbs_painted(None);
    let paint_before = painted(&core);
    let undo_depth = core.undo_stack.len();
    let (erased, inherited) = commit_diagonal(&mut core);
    let old_grid = erased
        .iter()
        .find(|&&(key, _)| !under_road(&core, key))
        .expect("released paint")
        .0
        .grid;
    let relaid = |core: &SimCore, lot: u64| {
        core.zoning
            .parcel_by_raw_id(lot)
            .and_then(|parcel| parcel.cell_lot())
            .is_some_and(|lot| {
                lot.cells().all(|key| {
                    key.grid == old_grid
                        && inherited
                            .binary_search_by_key(&key, |&(cell, _)| cell)
                            .is_ok()
                })
            })
    };
    let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
        &core.zoning,
        &core.region_graph,
        core.demand.runtime_catalog(),
        &[],
        BuildingSiteEnvironment {
            road_surface: &core.transit_network.roadbed,
            terrain: &core.heightmap,
        },
    );
    let action = candidates
        .residential
        .iter()
        .map(|candidate| &candidate.action)
        .find(|action| relaid(&core, action.parcel_id))
        .expect("a house fits a re-laid lot of the old grid")
        .clone();
    let house = core
        .allocator
        .execute_demand_spawn_action(
            &action,
            &mut core.zoning,
            &core.region_graph,
            &core.transit_network.roadbed,
            &core.heightmap,
            core.demand.runtime_catalog(),
            core.demand.runtime_tuning(),
        )
        .expect("the house grows");
    core.rebuild_building_entrances_internal();
    let state = land_state(&core);
    assert!(!core.undo_action_internal(), "the undo is refused");
    assert_eq!(land_state(&core), state, "a refused undo changes nothing");
    assert_eq!(core.undo_stack.len(), undo_depth + 1, "the record stays");
    let core = &mut core;
    assert!(core.allocator.remove_building_for_bulldoze(
        house,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance,
    ));
    core.publish_pending_building_site_changes();
    assert!(core.undo_action_internal());
    assert_eq!(painted(core), paint_before);
}

// Test 4 (rule 4, D-removal (a)): removing the only street of a painted strip leaves no paint,
// as no cell keeps a road column; undo gives every painted cell back.
#[test]
fn removing_a_strips_only_street_leaves_no_paint_and_undo_restores_it() {
    let mut core = curbs_painted(None);
    let saved = core.zoning.cells.saved_cells();
    assert!(!saved.is_empty());
    let street = (0..core.region_graph.edge_count())
        .find(|&edge| !core.region_graph.edge(edge).deleted)
        .unwrap();
    let depth = core.undo_stack.len();
    assert!(core.bulldoze_road_edge_for_test(street));
    assert_eq!(core.undo_stack.len(), depth + 1);
    core.adopt_pending_cell_lots();
    assert!(
        !core.zoning.cells.has_reservations(),
        "no paint without a road"
    );
    assert!(core.undo_action_internal());
    core.prepare_cell_lots_internal();
    assert_eq!(core.zoning.cells.saved_cells(), saved);
    assert_references_consistent(&core);
}

// Test 5, mixed zones (D-mixed (a)): across a block painted with two zones, every cell carries the
// zone it overlaps most, both zones meet the new road, and each cell re-laid in place keeps its
// own. (Ties go to the lowest key: the planner's
// `new_frontage_cells_inherit_the_most_overlapped_removed_paint`.)
#[test]
fn mixed_zones_re_lay_by_the_largest_share() {
    let mut core = curbs_painted(Some(2));
    let before = painted(&core);
    assert!(before.iter().any(|&(_, p)| p == 1) && before.iter().any(|&(_, p)| p == 2));
    let (erased, inherited) = commit_diagonal(&mut core);
    settle(&mut core);
    assert_paint_follows(&core, &before, "two zones");
    let old_grids: std::collections::BTreeSet<_> = erased.iter().map(|(key, _)| key.grid).collect();
    let new_zones: std::collections::BTreeSet<_> = inherited
        .iter()
        .filter(|(key, _)| !old_grids.contains(&key.grid))
        .map(|&(_, profile)| profile)
        .collect();
    assert_eq!(new_zones, [1, 2].into(), "both zones front the new road");
    assert_relaid(&core, &before, "two zones");
}

// A cross street through the street's middle, every quadrant of the crossroads painted, and its
// junction.
fn zoned_crossroads() -> (SimCore, u32) {
    let (mut core, _) = street_on(|_| {});
    assert_eq!(
        commit(
            &mut core,
            &[
                Vector3::new(0.0, 0.0, -150.0),
                Vector3::new(0.0, 0.0, 150.0)
            ]
        ),
        ""
    );
    core.adopt_pending_cell_lots();
    for (x, z) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        for point in [
            DVec2::new(40.0 * x, 15.0 * z),
            DVec2::new(15.0 * x, 40.0 * z),
        ] {
            paint(&mut core, CellSelectionShape::Fill, &[point], 1);
        }
    }
    settle(&mut core);
    let node = core
        .region_graph
        .find_node_within(Vector3::ZERO, 1.0)
        .expect("the crossroads");
    (core, node)
}

fn ring_at(node: u32) -> RoundaboutRequest {
    RoundaboutRequest {
        site: RoundaboutSite::Junction(node),
        diameter_m: 60.0,
    }
}

// Every cell, painted or not, whose road-contact interior the island overlaps.
fn on_island(
    core: &SimCore,
    island: &crate::simulation::agriculture::PolygonFootprint,
) -> Vec<CellKey> {
    cell_keys(core)
        .into_iter()
        .filter(|&key| {
            let interior = road_contact_interior(&corners(core, key)).map(|p| [p.x, p.y]);
            island.overlaps_precise_points(&interior)
        })
        .collect()
}

// Test 8 (`ZONE-13`): a ring over a zoned crossroads leaves no cell on its island, which is road
// without sidewalks: after its plan, after undo (the crossroads' cells and paint back exactly)
// and the ring again, and after a save and load.
#[test]
fn a_ring_over_a_zoned_crossroads_leaves_no_cell_on_its_island() {
    let (mut core, node) = zoned_crossroads();
    let island = RoundaboutLayout::prepare(&core.region_graph, ring_at(node))
        .expect("the ring's plan")
        .island_footprint();
    assert!(
        !on_island(&core, &island).is_empty(),
        "cells on the island's land"
    );
    let (paint_before, keys_before) = (painted(&core), cell_keys(&core));
    assert!(core.place_roundabout_internal(ring_at(node)));
    settle(&mut core);
    assert_eq!(
        on_island(&core, &island),
        Vec::new(),
        "after the ring's plan"
    );
    assert_relaid(&core, &paint_before, "ring");

    assert!(core.undo_action_internal());
    settle(&mut core);
    assert_eq!(painted(&core), paint_before, "undo gives the paint back");
    assert_eq!(cell_keys(&core), keys_before, "and the cells");

    assert!(core.place_roundabout_internal(ring_at(node)));
    settle(&mut core);
    assert_eq!(
        on_island(&core, &island),
        Vec::new(),
        "after the ring again"
    );

    let path = super::super::temp_save_path("ring_island");
    core.save_game_settled(path.to_str().unwrap(), None)
        .unwrap();
    let mut loaded = test_core();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    settle(&mut loaded);
    assert_eq!(
        on_island(&loaded, &island),
        Vec::new(),
        "after a save and load"
    );
    assert_eq!(painted(&loaded), painted(&core), "the save keeps the paint");
}

// Where the long street and its stroke lie: four chunks along x, two along z.
const LONG: CellBounds = CellBounds {
    min: DVec2::new(-1000.0, -200.0),
    max: DVec2::new(1000.0, 200.0),
};

// A street from x -900 to 900 with both curbs painted whole, or with none, its cells generated.
fn long_street(painted: bool) -> SimCore {
    let mut core = test_core();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e9;
    register_houses(&mut core);
    commit_ready(
        &mut core,
        vec![
            Vector3::new(-900.0, 0.0, 0.0),
            Vector3::new(900.0, 0.0, 0.0),
        ],
    );
    if painted {
        for z in [15.0, -15.0] {
            paint(
                &mut core,
                CellSelectionShape::Fill,
                &[DVec2::new(0.0, z)],
                1,
            );
        }
    }
    settle_long(&mut core);
    core
}

// [`settle`] over every chunk of `LONG`.
fn settle_long(core: &mut SimCore) {
    core.precompute_road_mesh_data();
    core.publish_road_tool_view();
    core.prepare_cell_lots_internal();
    for x in -2..=1 {
        for z in -1..=0 {
            assert!(core.prepare_cell_chunk_internal((x, z)));
        }
    }
}

// A stroke 1.4 km long across the south block: its lot plan serves several regions, and the
// released cells of the later ones must not inherit before those regions regenerate them, or
// their paint would pin the old grid there (`CellStore::generation_current`). The block re-lays
// along the whole stroke as unpainted land does.
#[test]
fn a_long_stroke_re_lays_every_chunk_it_crosses() {
    let stroke = [
        Vector3::new(-700.0, 0.0, -75.0),
        Vector3::new(700.0, 0.0, -30.0),
    ];
    let mut twin = long_street(false);
    assert_eq!(commit(&mut twin, &stroke), "");
    twin.adopt_pending_cell_lots();
    settle_long(&mut twin);
    let mut core = long_street(true);
    let before = painted_in(&core, LONG);
    assert_eq!(commit(&mut core, &stroke), "");
    assert!(core.adopt_pending_cell_lots().adopted);
    settle_long(&mut core);
    assert_eq!(
        layout_in(&core, LONG),
        layout_in(&twin, LONG),
        "the re-laid cells"
    );
    assert_relaid_in(&core, LONG, &before, "long stroke");
    assert_references_consistent(&core);
}
