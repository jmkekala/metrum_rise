// SPDX-License-Identifier: GPL-2.0-only

//! Road edits in place over zoned land (`ZONE-10`, round RZ10a): a widening, a turn pocket and a
//! roundabout (its island included) take the zoned land their pieces newly cover as a road
//! stroke does, demolishing the buildings on the lots and erasing the covered cells' paint, and
//! re-lay the free paint of the region they regenerate (`ZONE-14`); the reverse edit covers
//! nothing; plotted land (a field, an extraction area, a placed building's
//! lot) refuses each of them with nothing changed. Their undo gives the road and the paint back,
//! not the buildings, and is refused whole while the paint's land is held. Each verdict (the
//! roundabout hover on the road tool's context, the windows') shows what its commit takes (round
//! RZ10b), and the other graph edits take land by the same rule (a bridge brought to the ground).

use super::preview::{handles, tool_context};
use super::*;
use crate::nodes::sim::core::RoadLandTake;
use crate::nodes::sim::core::tests::road_crosswalks::arm_towards;
use crate::simulation::network::graph::RoadParking;
use crate::simulation::network::pockets::PocketRequest;
use crate::simulation::network::road_types::RoadAttributes;
use crate::simulation::network::roadbed::EdgeEnd;
use crate::simulation::network::roundabouts::{RoundaboutLayout, RoundaboutRequest, RoundaboutSite};

// The houses' street (`street_with_houses`, houses on its -z curb from x -60 to 60) with the
// side street `SIDE` into its middle from the houses' side, which took the houses in its way:
// the arms' left turns lead into it. Returns the core and the T's junction node.
fn t_with_houses() -> (SimCore, u32) {
    let (mut core, _) = street_with_houses();
    assert_eq!(commit(&mut core, &SIDE), "");
    core.adopt_pending_cell_lots();
    let node = core
        .region_graph
        .find_node_within(Vector3::ZERO, 1.0)
        .expect("the T's junction");
    (core, node)
}

// The street's arm of the T towards -x.
fn west_arm(core: &SimCore, node: u32) -> usize {
    arm_towards(core, node, Vector3::new(-150.0, 0.0, 0.0))
}

// One of the three edits under test, applied to the T: whether it was applied.
#[derive(Clone, Copy, Debug)]
enum Edit {
    // The west arm from 1+1 to 2+2 lanes.
    Widen,
    // A 40 m left pocket on the west arm at the junction, for the left turn into the side
    // street: it widens towards -z, the houses' side.
    Pocket,
    // A one-lane ring of 60 m in place of the junction.
    Ring,
}

const EDITS: [Edit; 3] = [Edit::Widen, Edit::Pocket, Edit::Ring];

impl Edit {
    fn apply(self, core: &mut SimCore, node: u32) -> bool {
        match self {
            Self::Widen => widen(core, west_arm(core, node), 2),
            Self::Pocket => core.set_road_pocket_internal(pocket(core, node, true)),
            Self::Ring => core.place_roundabout_internal(ring(node)),
        }
    }

    // Its reverse on the edited T: what gives the land back, taking none (the ring has none).
    fn reverse(self, core: &mut SimCore, node: u32) -> Option<bool> {
        Some(match self {
            Self::Widen => widen(core, west_arm(core, node), 1),
            Self::Pocket => core.set_road_pocket_internal(pocket(core, node, false)),
            Self::Ring => return None,
        })
    }
}

// Sets `edge`'s lanes to `lanes` each way, keeping its attributes.
fn widen(core: &mut SimCore, edge: usize, lanes: u8) -> bool {
    let attributes = RoadAttributes::of_edge(core.region_graph.edge(edge));
    core.set_edge_cross_section_internal(edge as i32, lanes, lanes, None, RoadParking::NONE, attributes)
}

// The west arm's 40 m left pocket at the junction (`on`), or none.
fn pocket(core: &SimCore, node: u32, on: bool) -> PocketRequest {
    let edge = west_arm(core, node);
    let end = if core.region_graph.get_valid_node(core.region_graph.edge(edge).end_node) == node {
        EdgeEnd::End
    } else {
        EdgeEnd::Start
    };
    PocketRequest {
        edge,
        end,
        left: on,
        right: false,
        body_m: 40.0,
    }
}

fn ring(node: u32) -> RoundaboutRequest {
    RoundaboutRequest {
        site: RoundaboutSite::Junction(node),
        diameter_m: 60.0,
    }
}

// The paint the top undo record's land take erased and its new cells inherited.
fn record_paint(core: &SimCore) -> Option<(Paint, Paint)> {
    let cells = core.undo_stack.back()?.cells.as_ref()?;
    Some((cells.erased().to_vec(), cells.inherited().to_vec()))
}

// Every reserved cell in `AREA` whose road-contact interior a road piece covers.
fn reserved_under_road(core: &SimCore) -> Vec<CellKey> {
    let mut under = Vec::new();
    core.zoning
        .cells
        .visit_reserved_footprints_in_bounds(AREA, |key, _| {
            if under_road(core, key) {
                under.push(key);
            }
        });
    under
}

// The graph's live records, for the determinism digest.
fn graph_digest(core: &SimCore) -> u64 {
    let edges: Vec<_> = core
        .region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted)
        .map(|edge| {
            let points: Vec<_> = edge
                .physical_geometry
                .iter()
                .map(|p| [p.x.to_bits(), p.z.to_bits()])
                .collect();
            (edge.start_node, edge.end_node, edge.width.to_bits(), points)
        })
        .collect();
    crate::debug::work::debug_hash(&edges)
}

// Test 1 (`ZONE-10`, PLAN-RZ 10.5): each edit demolishes the houses on the lots its pieces newly
// cover, their families moved out, and erases the paint of the covered cells; no reserved cell
// is left under a road, and the Road Properties window's verdict counts the widening's
// demolitions. The free paint of the region it regenerates re-lays (`ZONE-14`, PLAN-RL test 6):
// no cell over the old paint is left unpainted, and the cell layout equals that of the same edit
// on the T without that paint. The reverse edit covers nothing and demolishes nothing; it re-lays
// the paint of the frontage it moves (D-removal).
#[test]
fn each_in_place_edit_takes_the_zoned_land_it_newly_covers() {
    for edit in EDITS {
        let (mut core, node) = t_with_houses();
        let twin = {
            let (mut twin, node) = t_with_houses();
            erase_free_paint(&mut twin);
            assert!(edit.apply(&mut twin, node), "{edit:?} without the free paint");
            layout(&twin)
        };
        let painted_before = painted(&core);
        let buildings = core.allocator.buildings.len();
        let agents = core.agents.agents.len();
        let island = matches!(edit, Edit::Ring).then(|| {
            RoundaboutLayout::prepare(&core.region_graph, ring(node))
                .expect("the ring's plan")
                .island_footprint()
        });
        if let Edit::Widen = edit {
            let west = west_arm(&core, node);
            let attributes = RoadAttributes::of_edge(core.region_graph.edge(west));
            let verdict = core
                .preview_edge_cross_section_internal(west as i32, 2, 2, None, RoadParking::NONE, attributes)
                .expect("the widening is accepted");
            assert!(verdict.buildings_removed > 0, "{verdict:?}");
            assert!(verdict.demolition_permanent);
            assert!(edit.apply(&mut core, node));
            assert_eq!(
                core.allocator.buildings.len(),
                buildings - verdict.buildings_removed,
                "the verdict counts the demolitions"
            );
        } else {
            assert!(edit.apply(&mut core, node), "{edit:?}");
        }
        core.precompute_road_mesh_data();
        assert!(core.allocator.buildings.len() < buildings, "{edit:?} demolishes houses");
        assert_eq!(core.agents.agents.len(), agents, "{edit:?}: no family is lost");
        assert_references_consistent(&core);
        let (erased, inherited) = record_paint(&core).expect("the record keeps the erased paint");
        eprintln!(
            "{edit:?}: demolished {} erased {} inherited {}",
            buildings - core.allocator.buildings.len(),
            erased.len(),
            inherited.len()
        );
        assert!(!erased.is_empty(), "{edit:?}");
        // The erase holds old paint only, and every painted cell the pieces cover; the ring also
        // takes its island's land (decision D-ring).
        let island = island.as_ref();
        for &(key, profile) in &painted_before {
            let on_island = island.is_some_and(|island| {
                let interior = road_contact_interior(&corners(&core, key)).map(|p| [p.x, p.y]);
                island.overlaps_precise_points(&interior)
            });
            if under_road(&core, key) || on_island {
                assert!(erased.contains(&(key, profile)), "{edit:?} {key:?} covered and erased");
            }
        }
        for paint in &erased {
            assert!(painted_before.contains(paint), "{edit:?} {paint:?} was painted");
        }
        assert_eq!(reserved_under_road(&core), Vec::new(), "{edit:?}");
        assert_relaid(&core, &painted_before, &format!("{edit:?}"));
        assert_eq!(layout(&core), twin, "{edit:?}: the re-laid cells are the unpainted ones");
        // The reverse edit covers nothing and demolishes nothing.
        let (buildings, painted_now) = (core.allocator.buildings.len(), painted(&core));
        if let Some(applied) = edit.reverse(&mut core, node) {
            assert!(applied, "{edit:?} reversed");
            assert_eq!(core.allocator.buildings.len(), buildings, "{edit:?} reversed");
            if let Some((erased, _)) = record_paint(&core) {
                for (key, _) in erased {
                    assert!(!under_road(&core, key), "{edit:?} reversed covers {key:?}");
                }
            }
            assert_relaid(&core, &painted_now, &format!("{edit:?} reversed"));
            assert_references_consistent(&core);
        }
    }
}

// Decision D-ring: the ring takes the land on its island too, which its pieces do not cover: no
// reserved cell is left on the disc it stands on, so no lot keeps an arm stub for its street.
#[test]
fn a_ring_takes_the_lots_on_its_island() {
    let (mut core, node) = t_with_houses();
    let island = RoundaboutLayout::prepare(&core.region_graph, ring(node))
        .expect("the ring's plan")
        .island_footprint();
    let on_island = |core: &SimCore| {
        let mut keys = Vec::new();
        core.zoning
            .cells
            .visit_reserved_footprints_in_bounds(AREA, |key, corners| {
                let interior = road_contact_interior(&corners).map(|p| [p.x, p.y]);
                if island.overlaps_precise_points(&interior) {
                    keys.push(key);
                }
            });
        keys
    };
    let before = on_island(&core);
    let beyond_pieces = before.iter().filter(|&&key| !under_road(&core, key)).count();
    assert!(beyond_pieces > 0, "painted cells on the island");
    assert!(Edit::Ring.apply(&mut core, node));
    core.precompute_road_mesh_data();
    assert_eq!(on_island(&core), Vec::new(), "nothing reserved is left on the island");
    let (erased, _) = record_paint(&core).unwrap();
    let off_pieces = erased.iter().filter(|&&(key, _)| !under_road(&core, key)).count();
    assert!(off_pieces > 0, "the island takes cells no ring piece covers");
}

// Test 1, plotted land: a field or an extraction area where each edit's pieces (or the ring's
// island) would newly reach refuses it with the stroke's reason, and the edit changes nothing.
#[test]
fn plotted_land_refuses_each_in_place_edit() {
    // Each edit's polygon on land it newly covers: beside the widened west arm far from the
    // houses, beside the pocket, on the ring's island across the street from the side street.
    let polygon = |edit: Edit| {
        let [x0, z0, x1, z1] = match edit {
            Edit::Widen => [-120.0, -8.0, -100.0, -6.0],
            Edit::Pocket => [-30.0, -8.0, -20.0, -6.0],
            Edit::Ring => [10.0, 10.0, 14.0, 14.0],
        };
        [
            Vector2::new(x0, z0),
            Vector2::new(x1, z0),
            Vector2::new(x1, z1),
            Vector2::new(x0, z1),
        ]
    };
    for edit in EDITS {
        for (kind, reason) in [("field", "field_overlap"), ("extraction", "extraction_area_overlap")] {
            let (mut core, node) = t_with_houses();
            let owner = core.allocator.buildings.len();
            match kind {
                "field" => core.allocator.field_clearance_mut().set(owner, &polygon(edit)),
                _ => core.allocator.extraction_clearance_mut().set(owner, &polygon(edit)),
            }
            let state = (land_state(&core), graph_digest(&core));
            assert!(!edit.apply(&mut core, node), "{edit:?} over a {kind}");
            let rejection = core.last_road_rejection.take();
            if !matches!(edit, Edit::Widen) {
                assert_eq!(rejection.map(|r| r.reason).as_deref(), Some(reason), "{edit:?}");
            }
            assert_eq!((land_state(&core), graph_digest(&core)), state, "{edit:?} over a {kind}");
        }
    }
}

// Test 1, a placed building's lot (`plotted_building_overlap`): a widening of the farm's road
// into the farm's lot is refused with nothing changed.
#[test]
fn a_placed_buildings_lot_refuses_a_widening() {
    let (mut core, farm, _) = super::super::fields::farm_fixture();
    core.benchmark_mode = false;
    core.treasury.balance = 1.0e9;
    let road = core.allocator.buildings[farm].edge_idx;
    let state = land_state(&core);
    let attributes = RoadAttributes::of_edge(core.region_graph.edge(road));
    let refusal = core
        .preview_edge_cross_section_internal(road as i32, 3, 3, None, RoadParking::NONE, attributes)
        .expect_err("the widening reaches the farm's lot");
    assert_eq!(refusal.reason, "plotted_building_overlap");
    assert!(!widen(&mut core, road, 3));
    assert_eq!(land_state(&core), state);
}

// Test 2 (decisions D5, D6): each edit's undo gives the road and the erased paint back, not the
// demolished houses, and lots derive again from the paint; while a field holds land an erased
// cell would take back, the undo is refused whole and changes nothing.
#[test]
fn undo_of_each_in_place_edit_restores_the_paint_not_the_buildings() {
    for edit in EDITS {
        let (mut core, node) = t_with_houses();
        let painted_before = painted(&core);
        let roads = graph_digest(&core);
        let depth = core.undo_stack.len();
        assert!(edit.apply(&mut core, node), "{edit:?}");
        let buildings = core.allocator.buildings.len();
        let (erased, _) = record_paint(&core).expect("the record keeps the erased paint");
        // A field on an erased cell's land refuses the undo.
        let frame = core.zoning.cells.frame(erased[0].0.grid).unwrap();
        let field = frame
            .corners(erased[0].0.x, erased[0].0.y)
            .map(|p| Vector2::new(p.x as f32, p.y as f32));
        let owner = core.allocator.buildings.len();
        core.allocator.field_clearance_mut().set(owner, &field);
        let state = land_state(&core);
        assert!(!core.undo_action_internal(), "{edit:?}: the undo is refused");
        assert_eq!(land_state(&core), state, "{edit:?}: a refused undo changes nothing");
        core.allocator.field_clearance_mut().set(owner, &[]);
        assert!(core.undo_action_internal(), "{edit:?}");
        assert_eq!(core.undo_stack.len(), depth, "{edit:?}");
        assert_eq!(graph_digest(&core), roads, "{edit:?}: the road is back");
        assert_eq!(painted(&core), painted_before, "{edit:?}: every erased cell is painted again");
        assert_eq!(core.allocator.buildings.len(), buildings, "{edit:?}: no house is rebuilt");
        core.prepare_cell_lots_internal();
        assert_references_consistent(&core);
    }
}

// Decision D-lot-plan: an in-place edit that erased paint plans its lots at once, so its new
// cells inherit its own paint into its own record before another edit erases any. Free paint
// beside each arm far from the houses, as the last paint around there: after both arms are
// widened in a row, each record holds the paint its own erase handed on to the new cells over
// that land (each inherited cell overlaps most an erased cell of the same record), the plan's
// paint slot is empty after each, and undoing both gives the land's paint back.
#[test]
fn two_erasing_edits_in_a_row_keep_their_own_inherited_paint() {
    use crate::simulation::zoning::cells::CellSelectionShape;
    let (mut core, node) = t_with_houses();
    for x in [-100.0, 100.0] {
        let brush = CellSelectionShape::Brush { radius_m: 6.0 };
        let preview = core.preview_cell_selection_internal(brush, &[DVec2::new(x, -9.0)]);
        assert!(core.apply_cell_selection_internal(&preview, 1));
    }
    core.prepare_cell_lots_internal();
    let painted_before = painted(&core);
    let mut records = Vec::new();
    for towards in [-150.0, 150.0] {
        let arm = arm_towards(&core, node, Vector3::new(towards, 0.0, 0.0));
        assert!(widen(&mut core, arm, 2));
        records.push(record_paint(&core).expect("the widening erases paint"));
        assert!(core.zoning.take_road_paint_sources().is_none(), "the edit's plan took them");
        assert!(!core.cell_lots_pending() && !core.zoning.has_dirty_cell_lots());
    }
    for (arm, (erased, inherited)) in records.iter().enumerate() {
        assert!(!inherited.is_empty(), "arm {arm}: the new cells inherit");
        for &(key, profile) in inherited {
            let footprint = corners(&core, key);
            let (mut best, mut best_area) = (None, 0.0);
            for &(source, paint) in erased {
                let area = convex_overlap_area(&footprint, &corners(&core, source));
                if area > best_area {
                    (best, best_area) = (Some(paint), area);
                }
            }
            assert_eq!(best, Some(profile), "arm {arm} {key:?} inherits its own record's paint");
        }
    }
    assert!(core.undo_action_internal());
    assert!(core.undo_action_internal());
    assert_eq!(painted(&core), painted_before);
}

// Test 3: a widening that also turns zoning off takes its lots before the road's other parcels
// are captured, so the parcel-removal undo holds no taken lot; undo restores the road, the other
// parcels and the paint.
#[test]
fn a_zoning_off_widening_captures_no_taken_lot() {
    use crate::simulation::zoning::ParcelGeometry;
    let (mut core, node) = t_with_houses();
    let west = arm_towards(&core, node, Vector3::new(-150.0, 0.0, 0.0));
    // An authored parcel on the west arm beyond the widened road's reach: zoning off removes it.
    let profile = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    let at = |x: f32, z: f32| Vector2::new(x, z);
    let far = core
        .zoning
        .parcels
        .insert_new(
            ParcelGeometry {
                edge_idx: west,
                side: -1,
                frontage_center_t: 0.15,
                frontage_m: 4.0,
                depth_m: 4.0,
                front_center: at(-128.0, -10.0),
                center: at(-128.0, -12.0),
                tangent: at(1.0, 0.0),
                normal: at(0.0, -1.0),
                corners: [at(-130.0, -10.0), at(-126.0, -10.0), at(-126.0, -14.0), at(-130.0, -14.0)],
                aabb_min: at(-130.0, -14.0),
                aabb_max: at(-126.0, -10.0),
            },
            profile,
        )
        .raw();
    let lot_of: std::collections::HashMap<CellKey, u64> = painted(&core)
        .into_iter()
        .filter_map(|(key, _)| Some((key, core.zoning.cells.lot(key).filter(|&lot| lot != 0)?)))
        .collect();
    let painted_before = painted(&core);
    let roads = graph_digest(&core);
    let attributes = RoadAttributes {
        zoning: false,
        ..RoadAttributes::of_edge(core.region_graph.edge(west))
    };
    assert!(core.set_edge_cross_section_internal(west as i32, 2, 2, None, RoadParking::NONE, attributes));
    let record = core.undo_stack.back().unwrap();
    let (erased, _) = record_paint(&core).unwrap();
    let taken: std::collections::BTreeSet<u64> =
        erased.iter().filter_map(|(key, _)| lot_of.get(key).copied()).collect();
    assert!(!taken.is_empty());
    let captured: Vec<u64> = record.zoning.as_ref().expect("zoning off captured").parcel_ids().collect();
    assert_eq!(captured, vec![far], "the capture holds the far parcel, no taken lot");
    assert!(core.zoning.parcel_by_raw_id(far).is_none());
    assert!(core.undo_action_internal());
    assert_eq!(graph_digest(&core), roads);
    assert!(core.zoning.parcel_by_raw_id(far).is_some(), "the other parcel is back");
    assert_eq!(painted(&core), painted_before);
    assert!(!core.region_graph.edge(west).no_building_spawn);
}

// The `before` rule (`ZONE-10`): land an edit's replaced pieces already cover is not newly
// covered. An authored parcel lying on the side street (an inconsistent state, set directly)
// stays when turning the side street's crossing off re-lays the junction and the side street
// with it: the toggle commits and takes nothing, where judged without the replaced pieces it
// would take the parcel.
#[test]
fn a_re_laid_junction_takes_none_of_the_land_it_already_covers() {
    use crate::simulation::zoning::ParcelGeometry;
    let (mut core, node) = t_with_houses();
    let side = arm_towards(&core, node, Vector3::new(0.0, 0.0, -70.0));
    let profile = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    // A 4 m square on the side street's carriageway, 30 m from the junction.
    let centre = Vector2::new(0.0, -30.0);
    let at = |dx: f32, dz: f32| centre + Vector2::new(dx, dz);
    let parcel = core
        .zoning
        .parcels
        .insert_new(
            ParcelGeometry {
                edge_idx: side,
                side: 1,
                frontage_center_t: 0.6,
                frontage_m: 4.0,
                depth_m: 4.0,
                front_center: at(0.0, 2.0),
                center: centre,
                tangent: Vector2::new(0.0, 1.0),
                normal: Vector2::new(1.0, 0.0),
                corners: [at(-2.0, -2.0), at(2.0, -2.0), at(2.0, 2.0), at(-2.0, 2.0)],
                aabb_min: at(-2.0, -2.0),
                aabb_max: at(2.0, 2.0),
            },
            profile,
        )
        .raw();
    let buildings = core.allocator.buildings.len();
    assert!(core.set_crosswalk_override_internal(node, side as i32, false));
    assert_eq!(core.region_graph.node(node).crosswalk_overrides.get(&side), Some(&false));
    assert!(record_paint(&core).is_none());
    assert!(core.zoning.parcel_by_raw_id(parcel).is_some(), "the parcel stays");
    assert_eq!(core.allocator.buildings.len(), buildings);
}

// Test 5: the products of the edits are a function of their inputs: each edit on two cities
// built alike takes the same land bit for bit (cells, parcels, buildings, families, the graph and
// the record's paint).
#[test]
fn in_place_edits_that_take_land_are_deterministic() {
    for edit in EDITS {
        let run = || {
            let (mut core, node) = t_with_houses();
            assert!(edit.apply(&mut core, node), "{edit:?}");
            (
                core.zoning.cells.state_digest(),
                land_state(&core),
                graph_digest(&core),
                record_paint(&core),
            )
        };
        assert_eq!(run(), run(), "{edit:?}");
    }
}


// The take's buildings named by their handles, descending, as the road tool's view names them.
fn take_handles(core: &SimCore, take: &RoadLandTake) -> Vec<u64> {
    handles(core, &take.buildings)
}

// The handles of the buildings that stand.
fn standing(core: &SimCore) -> std::collections::BTreeSet<u64> {
    (0..core.allocator.buildings.len())
        .map(|index| core.allocator.building_handles.handle(index).to_bits())
        .collect()
}

// Asserts that the commit just made demolished exactly `expected` (handles, descending) of the
// buildings `before` held, and erased the paint of `cells`' painted ones; any other cell it
// erased is released paint off the road (`ZONE-14`).
fn assert_commit_took(
    core: &SimCore,
    before: &std::collections::BTreeSet<u64>,
    expected: &[u64],
    cells: &[(CellKey, u16)],
    edit: &str,
) {
    let mut gone: Vec<u64> = before.difference(&standing(core)).copied().collect();
    gone.reverse();
    assert_eq!(gone, expected, "{edit}: the commit demolishes what its verdict showed");
    let painted: Vec<(CellKey, u16)> = cells.iter().copied().filter(|&(_, p)| p != 0).collect();
    let erased = record_paint(core).map(|(erased, _)| erased).unwrap_or_default();
    for paint in &painted {
        assert!(erased.contains(paint), "{edit}: the commit erases the verdict's {paint:?}");
    }
    for &(key, profile) in &erased {
        assert!(
            painted.contains(&(key, profile)) || !under_road(core, key),
            "{edit}: {key:?} is under the road but not in the verdict"
        );
    }
}

// Test 6 (`ZONE-10`, round RZ10b): each edit with a verdict shows what its commit takes. The
// roundabout tool's hover, on the road tool's context without the core, takes the same cells,
// lots, lot points and buildings (by handle) as the core's check, and its outlines are the lots'
// own corners; the Road Properties window's widening verdict and the Junction window's pocket
// verdict carry the take; each commit (the click, Apply) then demolishes exactly those buildings
// and erases exactly those cells.
#[test]
fn each_edit_verdict_shows_what_its_commit_takes() {
    // The ring: the hover on the context, then the click.
    let (mut core, node) = t_with_houses();
    let mut context = tool_context(&mut core);
    let view = context
        .road_edit_site()
        .try_roundabout_edit(ring(node))
        .expect("the hover accepts the ring")
        .verdict();
    let live = core
        .preview_roundabout_internal(ring(node))
        .expect("the core accepts it");
    assert!(!live.take.buildings.is_empty(), "the ring takes houses");
    assert_eq!(view.cost, live.cost);
    assert_eq!(view.ring, live.ring);
    assert_eq!(view.arms, live.arms);
    assert_eq!(view.take.plotted, live.take.plotted);
    assert_eq!(view.take.cells, live.take.cells, "covered cells");
    assert_eq!(view.take.lots, live.take.lots, "removed lots");
    assert_eq!(view.take.lot_points, live.take.lot_points, "lot points");
    assert_eq!(view.take.buildings, take_handles(&core, &live.take), "demolished buildings");
    for (&lot, &at) in view.take.lots.iter().zip(&view.take.lot_points) {
        let parcel = core.zoning.parcel_by_raw_id(lot).expect("a taken lot is a parcel");
        assert_eq!(context.land.lot_corners(lot, at), Some(parcel.corners()), "lot {lot}'s outline");
    }
    let before = standing(&core);
    assert!(core.place_roundabout_internal(ring(node)));
    assert_commit_took(&core, &before, &view.take.buildings, &view.take.cells, "ring");

    // The widening: the window's verdict, then Apply.
    let (mut core, node) = t_with_houses();
    let west = west_arm(&core, node);
    let attributes = RoadAttributes::of_edge(core.region_graph.edge(west));
    let verdict = core
        .preview_edge_cross_section_internal(west as i32, 2, 2, None, RoadParking::NONE, attributes)
        .expect("the widening is accepted");
    assert!(!verdict.take.buildings.is_empty());
    let expected = take_handles(&core, &verdict.take);
    let before = standing(&core);
    assert!(widen(&mut core, west, 2));
    assert_commit_took(&core, &before, &expected, &verdict.take.cells, "widening");

    // The pocket: the window's verdict, then Apply.
    let (mut core, node) = t_with_houses();
    let verdict = core
        .preview_road_pocket_internal(pocket(&core, node, true))
        .expect("the pocket is accepted");
    assert!(!verdict.take.buildings.is_empty());
    let expected = take_handles(&core, &verdict.take);
    let before = standing(&core);
    assert!(core.set_road_pocket_internal(pocket(&core, node, true)));
    assert_commit_took(&core, &before, &expected, &verdict.take.cells, "pocket");
}

// The bridge edge of a free bridge drawn over the houses' lots at `x` (`bridge_at`).
fn bridge_over_houses(core: &mut SimCore, x: f32) -> usize {
    use crate::simulation::network::types::EdgeClass;
    assert_eq!(commit(core, &super::bridge_at(x)), "");
    core.adopt_pending_cell_lots();
    (0..core.region_graph.edge_count())
        .find(|&edge| {
            let edge = core.region_graph.edge(edge);
            !edge.deleted && edge.class == EdgeClass::Bridge
        })
        .expect("the stroke is a bridge")
}

// The other graph-only edits take land by the same rule (`ZONE-10`, round RZ10b): a bridge over
// built lots takes none of them (decision D2), but the class change that brings its deck to the
// ground newly covers them, so it demolishes their houses and erases the covered paint into its
// own undo record, which gives the paint back, not the houses.
#[test]
fn a_bridge_brought_to_the_ground_takes_the_lots_under_it() {
    use crate::simulation::network::types::EdgeClass;
    let (mut core, houses) = street_with_houses();
    let x = core.allocator.buildings[houses[houses.len() / 2]].center_x;
    let bridge = bridge_over_houses(&mut core, x);
    let painted_before = painted(&core);
    let (buildings, depth) = (core.allocator.buildings.len(), core.undo_stack.len());
    core.set_edge_class_internal(bridge as i32, 0);
    assert_eq!(
        core.region_graph.edge(bridge).class,
        EdgeClass::Standard,
        "{:?}",
        core.last_road_rejection
    );
    assert_eq!(core.undo_stack.len(), depth + 1);
    assert!(core.allocator.buildings.len() < buildings, "the road on the ground demolishes houses");
    let (erased, _) = record_paint(&core).expect("the class change erases the covered paint");
    assert!(!erased.is_empty());
    for &(key, _) in &erased {
        assert!(under_road(&core, key), "{key:?}: only covered cells are erased");
    }
    assert_eq!(reserved_under_road(&core), Vec::new());
    assert_references_consistent(&core);
    let remaining = core.allocator.buildings.len();
    assert!(core.undo_action_internal());
    assert_eq!(core.region_graph.edge(bridge).class, EdgeClass::Bridge);
    assert_eq!(painted(&core), painted_before, "undo gives the paint back");
    assert_eq!(core.allocator.buildings.len(), remaining, "not the houses");
}

// A plotted reservation refuses the other edits as it refuses the four (`ZONE-10`): a field
// under the bridge refuses the class change that would bring the deck down on it, with nothing
// changed.
#[test]
fn a_field_refuses_a_bridge_brought_to_the_ground() {
    use crate::simulation::network::types::EdgeClass;
    let (mut core, houses) = street_with_houses();
    let x = core.allocator.buildings[houses[houses.len() / 2]].center_x;
    let bridge = bridge_over_houses(&mut core, x);
    let owner = core.allocator.buildings.len();
    let field = [
        Vector2::new(x - 1.0, -40.0),
        Vector2::new(x + 1.0, -40.0),
        Vector2::new(x + 1.0, -38.0),
        Vector2::new(x - 1.0, -38.0),
    ];
    core.allocator.field_clearance_mut().set(owner, &field);
    let state = (land_state(&core), graph_digest(&core));
    core.set_edge_class_internal(bridge as i32, 0);
    assert_eq!(core.region_graph.edge(bridge).class, EdgeClass::Bridge);
    let reason = core.last_road_rejection.take().map(|r| r.reason);
    assert_eq!(reason.as_deref(), Some("field_overlap"));
    assert_eq!((land_state(&core), graph_digest(&core)), state);
}

// The Godot road edit demolition test's fixture (`godot/tests/road_edit_demolition_test.gd`,
// PLAN-RZ 10.5 test 9): the T with houses, saved to `METRUM_ROAD_EDIT_DEMOLITION_FIXTURE`, and
// beside it, in `<save>.json`, the junction and a point on its west arm, by position (a save
// compacts the ids). The ring and the widening each take houses here.
#[test]
#[ignore = "writes the Godot road edit demolition test's fixture"]
fn record_road_edit_demolition_fixture() {
    let path = std::env::var("METRUM_ROAD_EDIT_DEMOLITION_FIXTURE")
        .expect("METRUM_ROAD_EDIT_DEMOLITION_FIXTURE: path of the fixture save to write");
    let (mut core, node) = t_with_houses();
    let ring_take = core.preview_roundabout_internal(ring(node)).expect("the ring").take;
    let west = west_arm(&core, node);
    let attributes = RoadAttributes::of_edge(core.region_graph.edge(west));
    let widening = core
        .preview_edge_cross_section_internal(west as i32, 2, 2, None, RoadParking::NONE, attributes)
        .expect("the widening");
    assert!(
        !ring_take.buildings.is_empty() && widening.buildings_removed > 0,
        "each edit takes houses"
    );
    let junction = core.region_graph.node(node).pos;
    core.save_game_settled_at(&path, None, 0).unwrap();
    let spec = serde_json::json!({
        "junction": [junction.x, junction.y, junction.z],
        "west_point": [-60.0, 0.0, 0.0],
        "ring_diameter_m": ring(node).diameter_m,
    });
    std::fs::write(format!("{path}.json"), spec.to_string()).unwrap();
    eprintln!(
        "ROAD_EDIT_DEMOLITION_FIXTURE {}",
        serde_json::json!({"path": path, "ring": ring_take.buildings.len(),
            "widening": widening.buildings_removed})
    );
}
