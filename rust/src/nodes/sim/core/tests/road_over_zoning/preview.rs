// SPDX-License-Identifier: GPL-2.0-only

//! Roads over zoned land in the road tool's hover preview (`ZONE-09`, round RZ3): the preview
//! takes, without the core, what the commit takes for the same stroke and state (the same cells,
//! lots and buildings, or the same plotted refusal, and nothing under a bridge's deck); its land
//! follows the city's buildings, sites and reservations; the one-shot stroke checks judge plotted
//! land on the stroke's pieces.

use super::*;
use crate::nodes::sim::core::road_edit_plan::PlottedOverlap;
use crate::nodes::sim::core::road_preview::RoadPreview;
use crate::nodes::sim::core::{
    RoadCorridorClearance, RoadEditPlan, RoadLandTake, RoadPreviewContext,
};
use crate::simulation::network::graph::RoadParking;
use crate::simulation::network::road_types::RoadAttributes;

// The road tool's context as the simulation publishes it: the view, and the land it takes
// (the site query index prepared, as frames leave it before the land refreshes its sites).
fn tool_context(core: &mut SimCore) -> RoadPreviewContext {
    core.precompute_road_mesh_data();
    core.refresh_road_terrain_inputs();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let view = road_tool_snapshot_from_core(core).expect("current roadbed");
    let mut context = RoadPreviewContext::capture(view);
    context.land = core.current_road_tool_land();
    context
}

// A 1 + 1 stroke as the road tool asks for it.
fn request(context: &RoadPreviewContext, points: &[Vector3]) -> RoadPreviewRequest {
    RoadPreviewRequest {
        surface_generation: context.view.surface_generation,
        points: points.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        tunnel: false,
        median: None,
        parking: RoadParking::NONE,
        attributes: RoadAttributes::DEFAULT,
    }
}

fn preview(context: &mut RoadPreviewContext, points: &[Vector3]) -> RoadPreview {
    let request = request(context, points);
    context.preview_road(request)
}

// The preview's take of `points`.
fn view_take(context: &mut RoadPreviewContext, points: &[Vector3]) -> RoadLandTake<u64> {
    let preview = preview(context, points);
    preview.pieces.expect("the stroke compiles").take
}

// The commit's take of the same pieces: `RoadLandTake::in_core` on the live core, with the
// site query index prepared as the commit prepares it.
fn commit_take(
    context: &mut RoadPreviewContext,
    core: &SimCore,
    points: &[Vector3],
) -> RoadLandTake {
    let plan = RoadEditPlan::compile_from(
        &context.view.terrain,
        &context.replica,
        &context.view.roadbed,
        request(context, points),
    );
    context
        .with_plan_pieces(&plan, |(installed, replaced)| {
            RoadLandTake::in_core(core, installed, replaced)
        })
        .expect("the stroke compiles")
}

// The commit's buildings by their handles, as the preview names them, descending.
fn handles(core: &SimCore, buildings: &[usize]) -> Vec<u64> {
    let mut handles: Vec<u64> = buildings
        .iter()
        .map(|&index| core.allocator.building_handles.handle(index).to_bits())
        .collect();
    handles.sort_unstable_by(|a, b| b.cmp(a));
    handles
}

// The preview's and the commit's take of `points` are the same products.
fn assert_takes_equal(
    context: &mut RoadPreviewContext,
    core: &SimCore,
    points: &[Vector3],
) -> RoadLandTake<u64> {
    let view = view_take(context, points);
    let live = commit_take(context, core, points);
    assert_eq!(view.plotted, live.plotted, "plotted land");
    assert_eq!(view.cells, live.cells, "covered cells");
    assert_eq!(view.lots, live.lots, "removed lots");
    assert_eq!(view.lot_points, live.lot_points, "lot points");
    assert_eq!(
        view.buildings,
        handles(core, &live.buildings),
        "demolished buildings"
    );
    view
}

// PLAN-RZ test 5: a stroke through built lots previews what its commit takes, the outlines are
// the removed parcels' own, and the commit then demolishes exactly the previewed houses.
#[test]
fn the_preview_takes_what_the_commit_takes() {
    let (mut core, houses) = street_with_houses();
    let mut context = tool_context(&mut core);
    for stroke in [&SIDE[..], &DIAGONAL[..]] {
        let take = assert_takes_equal(&mut context, &core, stroke);
        assert!(!take.buildings.is_empty(), "the stroke demolishes houses");
        assert!(take.lots.len() >= take.buildings.len());
        for (&lot, &at) in take.lots.iter().zip(&take.lot_points) {
            let parcel = core.zoning.parcel_by_raw_id(lot).expect("a taken lot");
            assert_eq!(context.land.lot_corners(lot, at), Some(parcel.corners()));
        }
        let previewed = preview(&mut context, stroke);
        assert!(previewed.validation.is_valid, "zoned land does not refuse");
    }

    let take = view_take(&mut context, &SIDE);
    let all = handles(&core, &houses);
    assert_eq!(commit(&mut core, &SIDE), "");
    for handle in all {
        let standing = core
            .allocator
            .building_handles
            .resolve(crate::simulation::core::handles::Handle::from_bits(handle))
            .is_some();
        assert_eq!(
            standing,
            !take.buildings.contains(&handle),
            "house {handle:#x}"
        );
    }
    for lot in &take.lots {
        assert!(
            core.zoning.parcel_by_raw_id(*lot).is_none(),
            "lot {lot} removed"
        );
    }
}

// PLAN-RZ tests 5 and 9: the preview refuses what the commit refuses, with its reason, for a
// placed building's lot (placed after the road tool's land was first published), a farm field
// and a mine's extraction area, and the commit then refuses it too.
#[test]
fn the_preview_refuses_plotted_land_as_the_commit_does() {
    let (mut core, farm, _) = crate::nodes::sim::core::tests::fields::farm_fixture_with(|core| {
        // The land as the simulation published it before the farm was placed.
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        core.refresh_road_tool_land();
    });
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e9;
    let lot = core.allocator.building_sites[farm].lot_footprint_world;
    let x = (lot[0].x + lot[2].x) * 0.5;
    let over_farm = [Vector3::new(x, 0.0, 80.0), Vector3::new(x, 0.0, 0.0)];
    let mut context = tool_context(&mut core);
    let take = assert_takes_equal(&mut context, &core, &over_farm);
    assert_eq!(take.plotted, Some(PlottedOverlap::Building(farm)));
    let previewed = preview(&mut context, &over_farm);
    assert!(!previewed.validation.is_valid);
    assert_eq!(
        previewed.validation.invalid_reason,
        "plotted_building_overlap"
    );
    assert_eq!(commit(&mut core, &over_farm), "plotted_building_overlap");

    let beside = [
        Vector3::new(160.0, 0.0, 90.0),
        Vector3::new(160.0, 0.0, 0.0),
    ];
    let area = [
        Vector2::new(140.0, 30.0),
        Vector2::new(180.0, 30.0),
        Vector2::new(180.0, 70.0),
        Vector2::new(140.0, 70.0),
    ];
    for (reason, set) in [("extraction_area_overlap", true), ("field_overlap", false)] {
        core.allocator.extraction_clearance_mut().clear();
        core.allocator.field_clearance_mut().clear();
        let mut context = tool_context(&mut core);
        assert_eq!(preview(&mut context, &beside).validation.invalid_reason, "");
        if set {
            core.allocator.extraction_clearance_mut().set(farm, &area);
        } else {
            core.allocator.field_clearance_mut().set(farm, &area);
        }
        // The land follows the reservation that changed.
        context.land = core.current_road_tool_land();
        assert_takes_equal(&mut context, &core, &beside);
        assert_eq!(
            preview(&mut context, &beside).validation.invalid_reason,
            reason
        );
        assert_eq!(commit(&mut core, &beside), reason);
    }
}

// Decision D2 in the preview: a bridge's deck over built lots takes nothing, as its commit.
#[test]
fn a_bridge_deck_previews_no_take() {
    let (mut core, houses) = street_with_houses();
    let x = core.allocator.buildings[houses[houses.len() / 2]].center_x;
    let mut context = tool_context(&mut core);
    let take = assert_takes_equal(&mut context, &core, &bridge_at(x));
    assert_eq!(take, RoadLandTake::default());
    assert!(preview(&mut context, &bridge_at(x)).validation.is_valid);
}

// The road tool's land follows the city between edits: a demolished house leaves the preview's
// count; an unchanged city publishes nothing.
#[test]
fn the_preview_land_follows_the_buildings() {
    let (mut core, _) = street_with_houses();
    let mut context = tool_context(&mut core);
    assert!(core.refresh_road_tool_land().is_none(), "nothing changed");
    let take = view_take(&mut context, &SIDE);
    assert!(!take.buildings.is_empty());
    let first = core
        .allocator
        .building_handles
        .resolve(crate::simulation::core::handles::Handle::from_bits(
            take.buildings[0],
        ))
        .expect("a previewed house");
    assert!(core.bulldoze_building(first));
    context.land = core
        .refresh_road_tool_land()
        .expect("a demolition changes the land");
    let after = assert_takes_equal(&mut context, &core, &SIDE);
    assert_eq!(after.buildings.len(), take.buildings.len() - 1);
}

// The one-shot clearance on the stroke's pieces (`ROAD-102`, decisions D2 and D8): with them, a
// bridge's deck over a field or a placed building's lot is no overlap, as its commit accepts;
// the corridor alone would refuse it.
#[test]
fn the_one_shot_check_follows_a_bridge_decks_pieces() {
    let clearance = |context: &mut RoadPreviewContext, core: &SimCore, points: &[Vector3]| {
        let plan = RoadEditPlan::compile_from(
            &context.view.terrain,
            &context.replica,
            &context.view.roadbed,
            request(context, points),
        );
        let stroke = (None, RoadParking::NONE, RoadAttributes::DEFAULT);
        let pieces = context
            .with_plan_pieces(&plan, |pieces| {
                RoadCorridorClearance::evaluate(core, points, (1, 1), stroke, Some(pieces))
            })
            .expect("the stroke compiles");
        let corridor = RoadCorridorClearance::evaluate(core, points, (1, 1), stroke, None);
        (pieces, corridor)
    };

    let (mut core, _) = street_with_houses();
    let owner = core.allocator.buildings.len();
    let field = [
        Vector2::new(80.0, -60.0),
        Vector2::new(120.0, -60.0),
        Vector2::new(120.0, -20.0),
        Vector2::new(80.0, -20.0),
    ];
    core.allocator.field_clearance_mut().set(owner, &field);
    let mut context = tool_context(&mut core);
    let (pieces, corridor) = clearance(&mut context, &core, &bridge_at(100.0));
    assert!(corridor.field_overlap, "the corridor crosses the field");
    assert!(!pieces.field_overlap, "the deck takes no field");

    let (mut core, farm, _) = crate::nodes::sim::core::tests::fields::farm_fixture();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e9;
    let (min, max) = lot_bounds(&core.allocator.building_sites[farm].lot_footprint_world);
    let z = (min.y + max.y) * 0.5;
    let bridge = [
        Vector3::new(min.x - 40.0, 6.0, z),
        Vector3::new(max.x + 40.0, 6.0, z),
    ];
    let mut context = tool_context(&mut core);
    let (pieces, corridor) = clearance(&mut context, &core, &bridge);
    assert!(
        corridor.plotted_overlap,
        "the corridor crosses the farm's lot"
    );
    assert!(!pieces.plotted_overlap, "the deck takes no lot");
    assert_eq!(
        commit(&mut core, &bridge),
        "",
        "the commit accepts the deck"
    );
}

// A lot's bounding box.
fn lot_bounds(lot: &[Vector2; 4]) -> (Vector2, Vector2) {
    lot.iter().fold(
        (
            Vector2::splat(f32::INFINITY),
            Vector2::splat(f32::NEG_INFINITY),
        ),
        |(min, max), &p| {
            (
                Vector2::new(min.x.min(p.x), min.y.min(p.y)),
                Vector2::new(max.x.max(p.x), max.y.max(p.y)),
            )
        },
    )
}

// The land the preview takes is shared, not copied, between publications that change nothing
// in a chunk: a refresh after one demolition rebuilds only its parcel chunk.
#[test]
fn a_land_refresh_rebuilds_only_changed_chunks() {
    let (mut core, houses) = street_with_houses();
    let before = core.current_road_tool_land();
    assert!(core.bulldoze_building(houses[0]));
    let after = core.refresh_road_tool_land().expect("changed");
    assert!(after.sequence() > before.sequence());
    let shared = after.shares_chunks_with(&before);
    assert!(shared.0 >= 1, "unchanged chunks are shared: {shared:?}");
    assert!(
        shared.1 >= 1,
        "the demolition's chunk is rebuilt: {shared:?}"
    );
}

// The Godot road tool's demolition test fixture (`godot/tests/road_tool_demolition_test.gd`,
// PLAN-RZ test 9): a street with one house on its cell lot, saved to
// `METRUM_ROAD_DEMOLITION_FIXTURE`, and beside it, in `<save>.json`, a side street stroke over
// the house that the preview here takes exactly that house with.
#[test]
#[ignore = "writes the Godot road demolition test's fixture"]
fn record_road_demolition_fixture() {
    use crate::nodes::sim::core::tests::road_pockets::buildings::{grow, register_houses};
    use crate::nodes::sim::core::tests::road_terrain_plan::commit_ready;
    let path = std::env::var("METRUM_ROAD_DEMOLITION_FIXTURE")
        .expect("METRUM_ROAD_DEMOLITION_FIXTURE: path of the fixture save to write");
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
    let houses = grow(&mut core, street, -1.0, [-12.0, 12.0]);
    assert!(!houses.is_empty(), "houses along the street");
    let ground = |core: &SimCore, x: f32, z: f32| {
        Vector3::new(
            x,
            core.get_world_surface_height_internal(Vector2::new(x, z)),
            z,
        )
    };
    // The first house whose side street, through its centre, takes it alone.
    let mut context = tool_context(&mut core);
    let (stroke, take) = houses
        .iter()
        .find_map(|&house| {
            let x = core.allocator.buildings[house].center_x;
            let stroke = [ground(&core, x, -70.0), ground(&core, x, 0.0)];
            let take = view_take(&mut context, &stroke);
            (take.buildings.len() == 1).then_some((stroke, take))
        })
        .expect("a stroke that takes one house");
    let x = stroke[0].x;
    core.save_game_settled_at(&path, None, 0).unwrap();
    let strokes = serde_json::json!({"house_stroke": stroke.map(|p| [p.x, p.y, p.z])});
    std::fs::write(format!("{path}.json"), strokes.to_string()).unwrap();
    eprintln!(
        "ROAD_DEMOLITION_FIXTURE {}",
        serde_json::json!({"path": path, "house": houses[0], "x": x, "lots": take.lots})
    );
}
