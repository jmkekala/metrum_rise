// SPDX-License-Identifier: GPL-2.0-only

//! Roads over zoned land (`ZONE-09`, rounds RZ1 and RZ2): a road stroke takes the zoned land its
//! pieces newly cover, demolishing the buildings on the lots it crosses, removing those lots and
//! erasing the covered cells' paint, which the new cells along the road inherit; a plotted
//! reservation (a placed building's lot, a field, an extraction area) refuses it and changes
//! nothing; a bridge's deck takes no land. Undo gives the road and the erased paint back and takes
//! the inherited paint away, not the buildings, and is refused whole while the paint's land is
//! taken.

use super::fields::farm_fixture;
use super::road_pockets::buildings::{grow, register_houses};
use super::road_terrain_plan::commit_ready;
use super::*;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::simulation::agriculture::PolygonFootprint;
use crate::simulation::buildings::allocator::BuildingSiteEnvironment;
use crate::simulation::zoning::cells::{
    CellBounds, CellKey, CellSelection, convex_overlap_area, road_contact_interior,
};
use glam::DVec2;
use godot::prelude::Vector2;

// The side street the strokes draw: from the south into the street's middle, a T.
const SIDE: [Vector3; 2] = [Vector3::new(0.0, 0.0, -70.0), Vector3::new(0.0, 0.0, 0.0)];

// A side street drawn diagonally into the street's middle from the south-west: its cells lie on
// a frame of their own, so its new frontage cells overlap the land of the cells it erases.
const DIAGONAL: [Vector3; 2] = [Vector3::new(-50.0, 0.0, -70.0), Vector3::new(0.0, 0.0, 0.0)];

// The area the tests compare cells in: the side street and the lots along the street.
const AREA: CellBounds = CellBounds {
    min: DVec2::new(-120.0, -120.0),
    max: DVec2::new(120.0, 120.0),
};

// Commits one stroke through the player's path (`commit_road`, no preview) and returns its
// rejection reason, empty when the stroke was committed.
fn commit(core: &mut SimCore, points: &[Vector3]) -> String {
    core.precompute_road_mesh_data();
    core.commit_road(
        RoadCommitRequest {
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            tunnel: false,
            median: None,
            parking: crate::simulation::network::graph::RoadParking::NONE,
            attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    )
    .rejection_reason
}

// A street along +x with houses grown on its south curb from x -60 to 60 and a family of two in
// every house. Undo records are kept. Returns the core and the houses.
fn street_with_houses() -> (SimCore, Vec<usize>) {
    let mut core = test_core();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e9;
    register_houses(&mut core);
    commit_ready(
        &mut core,
        vec![
            Vector3::new(-150.0, 0.0, 0.0),
            Vector3::new(150.0, 0.0, 0.0),
        ],
    );
    let street = (0..core.region_graph.edge_count())
        .find(|&edge| !core.region_graph.edge(edge).deleted)
        .expect("the street");
    let houses = grow(&mut core, street, -1.0, [-60.0, 60.0]);
    assert!(houses.len() >= 4, "houses along the street: {houses:?}");
    for &house in &houses {
        move_in(&mut core, house);
    }
    (core, houses)
}

// A family of two moves into `home`.
fn move_in(core: &mut SimCore, home: usize) {
    use crate::simulation::economy::agents::{
        household_age_composition, household_member_age_group,
    };
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
    let composition = household_age_composition(home, core.households.households.len(), 2);
    let household =
        core.households
            .admit_immigrant_household(&catalog, &tuning, home, 2, composition);
    core.allocator.claim_vacancy(home);
    let building = &core.allocator.buildings[home];
    let (x, z) = (building.center_x, building.center_y);
    for member in 0..2 {
        let age = household_member_age_group(home, household, member, 2);
        let agent = core
            .agents
            .spawn_housed_agent_with_age_group(home, x, z, age);
        core.agents.assign_household_id(agent, household);
    }
}

// Each house's parcel id and its lot's cells.
fn lots_of(core: &SimCore, houses: &[usize]) -> Vec<(u64, Vec<CellKey>)> {
    houses
        .iter()
        .map(|&house| {
            let id = core.allocator.buildings[house].parcel_id;
            let lot = core
                .zoning
                .parcel_by_raw_id(id)
                .and_then(|parcel| parcel.cell_lot())
                .expect("a grown house stands on a cell lot");
            (id, lot.cells().collect())
        })
        .collect()
}

// Whether a cell's road-contact interior lies under an exposed road piece.
fn under_road(core: &SimCore, key: CellKey) -> bool {
    let frame = core.zoning.cells.frame(key.grid).unwrap();
    let points = road_contact_interior(&frame.corners(key.x, key.y)).map(|p| [p.x, p.y]);
    PolygonFootprint::from_precise_slice(&points)
        .overlaps_exposed_roads(&core.transit_network.roadbed)
}

// The roads standing.
fn live_edges(core: &SimCore) -> usize {
    (0..core.region_graph.edge_count())
        .filter(|&edge| !core.region_graph.edge(edge).deleted)
        .count()
}

// Every painted cell in `AREA` with its paint.
fn painted(core: &SimCore) -> Vec<(CellKey, u16)> {
    let mut cells = Vec::new();
    core.zoning.cells.visit_in_bounds(AREA, |key| {
        if let Some(profile) = core.zoning.cells.profile(key).filter(|&p| p != 0) {
            cells.push((key, profile));
        }
    });
    cells.sort_unstable();
    cells.dedup();
    cells
}

// What a refused stroke must leave as it was: paint and claims, parcels with their occupants,
// buildings, agents, households, the graph and the undo history.
fn land_state(core: &SimCore) -> Vec<u64> {
    use crate::debug::work::debug_hash;
    let parcels: Vec<_> = core
        .zoning
        .parcels
        .parcels()
        .iter()
        .map(|parcel| (parcel.id().raw(), parcel.occupied_building()))
        .collect();
    let buildings: Vec<_> = core
        .allocator
        .buildings
        .iter()
        .map(|b| {
            (
                b.asset_id.clone(),
                b.parcel_id,
                b.center_x.to_bits(),
                b.center_y.to_bits(),
            )
        })
        .collect();
    let homes: Vec<_> = core
        .households
        .households
        .iter()
        .map(|household| household.home_building_id)
        .collect();
    vec![
        core.zoning
            .cells
            .digests_by(|_| true, |lot| lot, |road| road)
            .0,
        debug_hash(&parcels),
        debug_hash(&buildings),
        debug_hash(&homes),
        debug_hash(&core.agents.agents.home_building),
        core.agents.agents.len() as u64,
        core.region_graph.edge_count() as u64,
        core.undo_stack.len() as u64,
    ]
}

// Every building on a parcel is its parcel's occupant; every household and agent names a
// standing house or none.
fn assert_references_consistent(core: &SimCore) {
    let buildings = &core.allocator.buildings;
    for (index, building) in buildings.iter().enumerate() {
        if building.parcel_id != 0 {
            let parcel = core.zoning.parcel_by_raw_id(building.parcel_id);
            assert_eq!(
                parcel.and_then(|parcel| parcel.occupied_building()),
                Some(index),
                "building {index} occupies its parcel"
            );
        }
    }
    for parcel in core.zoning.parcels.parcels() {
        if let Some(index) = parcel.occupied_building() {
            assert_eq!(buildings[index].parcel_id, parcel.id().raw());
        }
    }
    let homes = &core.agents.agents.home_building;
    for (agent, &home) in homes.iter().enumerate() {
        assert!(
            home == usize::MAX || home < buildings.len(),
            "agent {agent}"
        );
        let household = core.agents.agents.household_id[agent];
        assert_eq!(
            core.households.households[household].home_building_id, home,
            "agent {agent} lives where its household does"
        );
    }
}

// Rules 1, 3, 4, 6 and 9: a stroke through built lots commits; the houses on the lots it covers
// are demolished with their lots, their families moved out; only the covered cells lose their
// paint, the rest of a taken lot stays painted and free; the other houses stand on their lots.
#[test]
fn a_road_over_a_growable_lot_demolishes_it_and_removes_only_covered_cells() {
    let (mut core, houses) = street_with_houses();
    let lots = lots_of(&core, &houses);
    let painted_before = painted(&core);
    let buildings_before = core.allocator.buildings.len();
    let agents_before = core.agents.agents.len();

    assert_eq!(commit(&mut core, &SIDE), "");
    assert!(core.last_road_rejection.is_none());
    // Read before the commit's deferred lot plan is adopted, which derives lots again.
    assert!(core.cell_lots_pending());
    let (taken, kept): (Vec<_>, Vec<_>) = lots
        .iter()
        .partition(|(id, _)| core.zoning.parcel_by_raw_id(*id).is_none());
    assert!(!taken.is_empty(), "the side street crosses a lot");
    assert!(!kept.is_empty(), "lots away from the side street stay");
    assert_eq!(
        core.allocator.buildings.len(),
        buildings_before - taken.len(),
        "one house demolished per taken lot"
    );
    for (id, _) in &kept {
        let parcel = core.zoning.parcel_by_raw_id(*id).unwrap();
        let house = parcel
            .occupied_building()
            .expect("a kept lot keeps its house");
        assert_eq!(core.allocator.buildings[house].parcel_id, *id);
    }
    // The families of the demolished houses were evacuated, none was lost.
    assert_eq!(core.agents.agents.len(), agents_before);
    assert_references_consistent(&core);

    // No reserved cell is left under the new road; a taken lot's other cells keep their paint
    // and are free; every other cell's paint is unchanged.
    let mut reserved_under_road = Vec::new();
    core.zoning
        .cells
        .visit_reserved_footprints_in_bounds(AREA, |key, _| {
            if under_road(&core, key) {
                reserved_under_road.push(key);
            }
        });
    assert!(reserved_under_road.is_empty(), "{reserved_under_road:?}");
    let mut erased = 0;
    let mut leftover = 0;
    for (_, cells) in &taken {
        for &key in cells {
            let profile = core.zoning.cells.profile(key);
            if profile.is_none_or(|profile| profile == 0) {
                erased += 1;
                assert!(
                    under_road(&core, key),
                    "{key:?}: only covered cells are erased"
                );
            } else {
                leftover += 1;
                assert_eq!(core.zoning.cells.lot(key), Some(0), "{key:?} is free");
            }
        }
    }
    assert!(
        erased > 0 && leftover > 0,
        "erased {erased}, leftover {leftover}"
    );
    let painted_after = painted(&core);
    for &(key, profile) in &painted_before {
        if !under_road(&core, key) {
            assert!(
                painted_after.contains(&(key, profile)),
                "{key:?} keeps its paint"
            );
        }
    }
    let after = core.adopt_pending_cell_lots();
    assert!(after.adopted);
    assert_references_consistent(&core);
}

// Rule 8 (decisions D5, D6): undo removes the road and gives back every erased cell's paint,
// not the demolished houses; their lots derive again from the paint.
#[test]
fn undo_of_a_demolishing_road_restores_cells_and_paint_not_buildings() {
    let (mut core, _) = street_with_houses();
    let painted_before = painted(&core);
    let roads_before = live_edges(&core);
    let buildings_before = core.allocator.buildings.len();
    let undo_depth = core.undo_stack.len();
    assert_eq!(commit(&mut core, &SIDE), "");
    core.adopt_pending_cell_lots();
    let buildings_after = core.allocator.buildings.len();
    assert!(
        buildings_after < buildings_before,
        "the stroke demolished houses"
    );
    assert_ne!(painted(&core), painted_before, "the stroke erased paint");
    assert_eq!(core.undo_stack.len(), undo_depth + 1);
    assert!(
        core.undo_stack
            .back()
            .is_some_and(|record| record.cells.is_some()),
        "the record keeps the erased paint"
    );

    assert!(core.undo_action_internal());
    assert_eq!(core.undo_stack.len(), undo_depth);
    assert_eq!(live_edges(&core), roads_before, "the side street is gone");
    assert_eq!(
        painted(&core),
        painted_before,
        "every erased cell is painted again"
    );
    assert_eq!(
        core.allocator.buildings.len(),
        buildings_after,
        "the demolished houses are not rebuilt"
    );
    core.prepare_cell_lots_internal();
    assert_references_consistent(&core);
    // The restored paint derives lots again: no painted cell is left without one.
    for (key, _) in painted(&core) {
        assert_ne!(
            core.zoning.cells.lot(key),
            Some(0),
            "{key:?} has a lot again"
        );
    }
}

// Decision D5: while a plotted reservation (here a field) holds land an erased cell would take
// back, the road's undo is refused whole and changes nothing; once the land is free, it runs.
#[test]
fn undo_is_refused_while_a_field_holds_restored_land() {
    let (mut core, _) = street_with_houses();
    let painted_before = painted(&core);
    let undo_depth = core.undo_stack.len();
    assert_eq!(commit(&mut core, &SIDE), "");
    core.adopt_pending_cell_lots();
    let erased = painted_before
        .iter()
        .find(|(key, _)| core.zoning.cells.profile(*key).is_none_or(|p| p == 0))
        .map(|&(key, _)| key)
        .expect("the stroke erased paint");
    let frame = core.zoning.cells.frame(erased.grid).unwrap();
    let field = frame
        .corners(erased.x, erased.y)
        .map(|p| Vector2::new(p.x as f32, p.y as f32));
    let owner = core.allocator.buildings.len();
    core.allocator.field_clearance_mut().set(owner, &field);
    let state = land_state(&core);
    assert!(!core.undo_action_internal(), "the undo is refused");
    assert_eq!(land_state(&core), state, "a refused undo changes nothing");
    assert_eq!(core.undo_stack.len(), undo_depth + 1, "the record stays");
    core.allocator.field_clearance_mut().set(owner, &[]);
    assert!(core.undo_action_internal());
    assert_eq!(painted(&core), painted_before);
}

// Rule 2 (decision D4): a stroke over a placed building's lot is refused before anything is
// written: the graph, parcels, buildings, agents, cells and undo history stay as they were.
#[test]
fn a_road_over_a_plotted_building_is_refused_and_changes_nothing() {
    let (mut core, farm, _) = farm_fixture();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e9;
    let state = land_state(&core);
    let lot = core.allocator.building_sites[farm].lot_footprint_world;
    let x = (lot[0].x + lot[2].x) * 0.5;
    let stroke = [Vector3::new(x, 0.0, 80.0), Vector3::new(x, 0.0, 0.0)];
    assert_eq!(commit(&mut core, &stroke), "plotted_building_overlap");
    assert!(!core.transit_network.road_edit_is_staged());
    assert_eq!(land_state(&core), state);
    assert!(core.transit_network.roadbed.failed_nodes().is_empty());
}

// Rule 2: a stroke over a mine's extraction area is refused and changes nothing.
#[test]
fn an_extraction_area_refuses_the_road() {
    let (mut core, farm, _) = farm_fixture();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e9;
    let area = [
        Vector2::new(140.0, 30.0),
        Vector2::new(180.0, 30.0),
        Vector2::new(180.0, 70.0),
        Vector2::new(140.0, 70.0),
    ];
    core.allocator.extraction_clearance_mut().set(farm, &area);
    let state = land_state(&core);
    let stroke = [
        Vector3::new(160.0, 0.0, 90.0),
        Vector3::new(160.0, 0.0, 0.0),
    ];
    assert_eq!(commit(&mut core, &stroke), "extraction_area_overlap");
    assert_eq!(land_state(&core), state);
    core.allocator.extraction_clearance_mut().clear();
    assert_eq!(
        commit(&mut core, &stroke),
        "",
        "the same stroke beside no area"
    );
}

// The commit's products are a function of its inputs: the same stroke on two cities built alike
// takes the same land, bit for bit, and its lot plan derives the same lots and hands on the same
// paint; the deferred plan's products equal those of the plan made in the commit.
#[test]
fn a_demolishing_commit_is_deterministic() {
    type Record = Option<(Paint, Paint)>;
    let run = |stroke: &[Vector3], in_commit: bool| {
        let (mut core, _) = street_with_houses();
        core.cell_lot_hooks.in_commit = in_commit;
        assert_eq!(commit(&mut core, stroke), "");
        let committed = (core.zoning.cells.state_digest(), land_state(&core));
        core.adopt_pending_cell_lots();
        let record: Record = core
            .undo_stack
            .back()
            .and_then(|record| record.cells.as_ref())
            .map(|cells| (cells.erased().to_vec(), cells.inherited().to_vec()));
        let adopted = (core.zoning.cells.state_digest(), land_state(&core), record);
        (committed, adopted)
    };
    for stroke in [&SIDE[..], &DIAGONAL[..]] {
        let first = run(stroke, false);
        assert_eq!(run(stroke, false), first);
        assert_eq!(run(stroke, true).1, first.1, "in-commit plan");
    }
}

// Decision D8: the one-shot stroke checks (`ROAD-102`) find a placed building's lot and an
// extraction area in the stroke's corridor; a stroke beside them finds neither.
#[test]
fn the_one_shot_corridor_check_finds_plotted_land() {
    use crate::nodes::sim::core::RoadCorridorClearance;
    use crate::simulation::network::graph::RoadParking;
    use crate::simulation::network::road_types::RoadAttributes;
    let (mut core, farm, _) = farm_fixture();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let clearance = |core: &SimCore, x: f32| {
        RoadCorridorClearance::evaluate(
            core,
            &[Vector3::new(x, 0.0, 90.0), Vector3::new(x, 0.0, 0.0)],
            (1, 1),
            None,
            RoadParking::NONE,
            RoadAttributes::DEFAULT,
        )
    };
    let lot = core.allocator.building_sites[farm].lot_footprint_world;
    let x = (lot[0].x + lot[2].x) * 0.5;
    assert!(clearance(&core, x).plotted_overlap);
    assert!(!clearance(&core, x + 160.0).plotted_overlap);
    assert!(!clearance(&core, x + 160.0).extraction_overlap);
    let area = [
        Vector2::new(x + 140.0, 30.0),
        Vector2::new(x + 180.0, 30.0),
        Vector2::new(x + 180.0, 70.0),
        Vector2::new(x + 140.0, 70.0),
    ];
    core.allocator.extraction_clearance_mut().set(farm, &area);
    assert!(clearance(&core, x + 160.0).extraction_overlap);
}

// A cell's footprint.
fn corners(core: &SimCore, key: CellKey) -> [DVec2; 4] {
    core.zoning
        .cells
        .frame(key.grid)
        .unwrap()
        .corners(key.x, key.y)
}

// Cells with their paint.
type Paint = Vec<(CellKey, u16)>;

// Commits the diagonal side street and adopts its lots; returns the cells it erased and those its
// lot plan painted, as the stroke's undo record keeps them.
fn commit_diagonal(core: &mut SimCore) -> (Paint, Paint) {
    assert_eq!(commit(core, &DIAGONAL), "");
    assert!(core.adopt_pending_cell_lots().adopted);
    let record = core
        .undo_stack
        .back()
        .and_then(|record| record.cells.as_ref())
        .expect("the record keeps the erased paint");
    (record.erased().to_vec(), record.inherited().to_vec())
}

// Rule 5: the new frontage cells along a stroke fill the land its erased cells leave, each taking
// the paint of the erased cell it overlaps most (ties to the lowest key); no new cell over erased
// land stays unpainted; the inherited paint gets lots in the commit's own lot plan.
#[test]
fn new_cells_along_a_stroke_inherit_the_erased_paint() {
    let (mut core, _) = street_with_houses();
    let (erased, inherited) = commit_diagonal(&mut core);
    assert!(!erased.is_empty());
    assert!(!inherited.is_empty(), "the side street's cells inherit");
    for &(key, profile) in &inherited {
        assert_eq!(core.zoning.cells.profile(key), Some(profile), "{key:?}");
        let footprint = corners(&core, key);
        // The oracle: the largest shared area, the lowest key among equals.
        let (mut best, mut best_area) = (None, 0.0);
        for &(source, paint) in &erased {
            let area = convex_overlap_area(&footprint, &corners(&core, source));
            if area > best_area {
                (best, best_area) = (Some(paint), area);
            }
        }
        assert_eq!(best, Some(profile), "{key:?}");
        assert_ne!(key.grid, erased[0].0.grid, "a new frame's cell");
    }
    // Every cell standing over erased land is painted: none is left unpainted.
    let mut unpainted = Vec::new();
    core.zoning.cells.visit_in_bounds(AREA, |key| {
        if core.zoning.cells.profile(key) == Some(0)
            && erased
                .iter()
                .any(|&(source, _)| convex_overlap_area(&corners(&core, key), &corners(&core, source)) > 0.0)
        {
            unpainted.push(key);
        }
    });
    assert!(unpainted.is_empty(), "{unpainted:?}");
    assert_references_consistent(&core);
}

// Rule 8, the inherited half: undo takes the inherited paint and its empty lots away and gives
// the erased paint back, so the land's paint is as before the stroke.
#[test]
fn undo_takes_the_inherited_paint_and_its_lots_away() {
    let (mut core, _) = street_with_houses();
    let painted_before = painted(&core);
    let (_, inherited) = commit_diagonal(&mut core);
    assert!(!inherited.is_empty());
    assert!(core.undo_action_internal());
    assert_eq!(painted(&core), painted_before);
    for (key, _) in inherited {
        assert!(core.zoning.cells.profile(key).is_none_or(|p| p == 0));
        assert!(core.zoning.cells.lot(key).is_none_or(|lot| lot == 0));
    }
    core.prepare_cell_lots_internal();
    assert_references_consistent(&core);
    for (key, _) in painted(&core) {
        assert_ne!(core.zoning.cells.lot(key), Some(0), "{key:?} has a lot again");
    }
}

// Decision D5 over another grid's cells: a house grown on the inherited cells holds land the
// erased cells would take back, so the road's undo is refused whole and changes nothing; once the
// house is gone (here outside the undo history), the undo runs and the erased paint is back.
#[test]
fn undo_is_refused_while_a_new_building_stands_on_restored_land() {
    let (mut core, _) = street_with_houses();
    let undo_depth = core.undo_stack.len();
    let (erased, inherited) = commit_diagonal(&mut core);
    // The inherited front row is one cell deep; the row behind it is zoned (outside the undo
    // history), and a lot derives over both.
    let path: Vec<_> = inherited
        .iter()
        .map(|&(key, _)| {
            let c = corners(&core, key);
            (c[0] + c[2]) * 0.5
        })
        .collect();
    let brush = crate::simulation::zoning::cells::CellSelectionShape::Brush { radius_m: 15.0 };
    let selection = core.zoning.cells.select(brush, &path);
    assert!(core.zoning.paint_cells(&selection, 1, |_| false).is_some());
    core.prepare_cell_lots_internal();
    let lot = inherited
        .iter()
        .find_map(|&(key, _)| core.zoning.cells.lot(key).filter(|&lot| lot != 0))
        .expect("an inherited lot");
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
        .find(|action| action.parcel_id == lot)
        .expect("a house fits the inherited lot")
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
    for (key, profile) in erased {
        assert_eq!(core.zoning.cells.profile(key), Some(profile), "{key:?}");
    }
}

// Decision D5: an inherited cell whose paint changed outside the undo history (another zone) is
// another grid's paint over the restored land: the undo is refused whole and changes nothing,
// and runs once the inherited paint is back.
#[test]
fn undo_is_refused_while_another_grids_paint_covers_restored_land() {
    let (mut core, _) = street_with_houses();
    let painted_before = painted(&core);
    let (_, inherited) = commit_diagonal(&mut core);
    let (key, profile) = inherited[0];
    let repaint = |core: &mut SimCore, profile: u16| {
        let selection = CellSelection {
            revision: core.zoning.cells.revision(),
            cells: vec![key],
        };
        assert!(core.zoning.paint_cells(&selection, profile, |_| false).is_some());
    };
    repaint(&mut core, profile + 1);
    let state = land_state(&core);
    assert!(!core.undo_action_internal(), "the undo is refused");
    assert_eq!(land_state(&core), state, "a refused undo changes nothing");
    repaint(&mut core, profile);
    assert!(core.undo_action_internal());
    assert_eq!(painted(&core), painted_before);
}

// Rule 5 when the stroke erases the last paint in the world: no reservation is left, yet the
// commit's lot plan still runs and its new cells inherit the paint.
#[test]
fn new_cells_inherit_the_last_paint_in_the_world() {
    let mut core = test_core();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e9;
    commit_ready(
        &mut core,
        vec![
            Vector3::new(-150.0, 0.0, 0.0),
            Vector3::new(150.0, 0.0, 0.0),
        ],
    );
    // A few cells where the diagonal side street crosses the street's first rows, and nothing
    // else painted.
    let brush = crate::simulation::zoning::cells::CellSelectionShape::Brush { radius_m: 6.0 };
    let preview = core.preview_cell_selection_internal(brush, &[DVec2::new(-21.0, -29.0)]);
    assert!(core.apply_cell_selection_internal(&preview, 1));
    core.prepare_cell_lots_internal();
    let painted_before = painted(&core);
    assert!(!painted_before.is_empty());
    let (erased, inherited) = commit_diagonal(&mut core);
    assert_eq!(erased, painted_before, "the stroke erases every painted cell");
    assert!(!inherited.is_empty(), "the new cells inherit the paint");
    assert_eq!(painted(&core), inherited);
}

// A free bridge 6 m over the flat ground, north to south across the houses' lots at `x`.
fn bridge_at(x: f32) -> [Vector3; 2] {
    [Vector3::new(x, 6.0, -70.0), Vector3::new(x, 6.0, -12.0)]
}

// The land a bridge must leave as it was: paint and claims, parcels and their occupants,
// buildings, agents and households.
fn land(core: &SimCore) -> Vec<u64> {
    land_state(core)[..6].to_vec()
}

// Decision D2: a bridge's deck takes no land: a deck over built lots leaves their paint, lots,
// houses and families as they were, before and after its lot plan.
#[test]
fn a_bridge_deck_covers_no_cell() {
    use crate::simulation::network::types::EdgeClass;
    let (mut core, houses) = street_with_houses();
    let x = core.allocator.buildings[houses[houses.len() / 2]].center_x;
    let before = land(&core);
    assert_eq!(commit(&mut core, &bridge_at(x)), "");
    assert!(
        core.region_graph
            .edges()
            .iter()
            .any(|edge| !edge.deleted && edge.class == EdgeClass::Bridge),
        "the stroke is a bridge"
    );
    assert_eq!(land(&core), before);
    core.adopt_pending_cell_lots();
    assert_eq!(land(&core), before);
}

// Decision D2: a bridge's deck over a farm field is no field overlap.
#[test]
fn a_bridge_deck_over_a_field_is_no_field_overlap() {
    let (mut core, _) = street_with_houses();
    let owner = core.allocator.buildings.len();
    let field = [
        Vector2::new(80.0, -60.0),
        Vector2::new(120.0, -60.0),
        Vector2::new(120.0, -20.0),
        Vector2::new(80.0, -20.0),
    ];
    core.allocator.field_clearance_mut().set(owner, &field);
    assert_eq!(commit(&mut core, &bridge_at(100.0)), "");
}
