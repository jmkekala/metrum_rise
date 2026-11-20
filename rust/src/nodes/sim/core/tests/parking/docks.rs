// SPDX-License-Identifier: GPL-3.0-or-later

//! PARK-19 docks: a yard's loading bay proven from its freight gate through a passage, kept out
//! of every supply index, reconciled, saved and derived for an older save.

use super::*;
use crate::simulation::parking::{Bay, PlaceSource};
use godot::prelude::Vector2;

// A 30 m by 40 m yard paved whole, placed as a service building (the parking class, a fixture's
// convenience: no economy): a drive-through bunker on the west with its lane at
// x -10..-5, a shed on the east, a 6 m freight gate above the lane, a loading bay under the
// bunker and one car spot opening onto the street.
const YARD: &str = r#"
asset_id = "building.industrial.freight_yard"
display_name = "Freight yard"

[building]
placement_mode = "explicit"
service_class = "parking"
lot_width_cells = 3
lot_depth_cells = 4
frontage_forward = [0, 0, 1]
worker_capacity = 0
household_capacity = 0

[[mesh_parts]]
name = "bunker"
position = [-7.5, 0, 0]
[[mesh_parts.lods]]
file = "bunker.glb"
distance_min_m = 0
PASSAGE

[[mesh_parts]]
name = "shed"
position = [5, 0, -10]
[[mesh_parts.lods]]
file = "shed.glb"
distance_min_m = 0

[[anchors]]
type = "entrance"
name = "main"
position = [10, 0, 10]
forward = [0, 0, 1]

[[anchors]]
type = "parking"
position = [8, 0, 20]
forward = [0, 0, -1]
width_m = 2.5
length_m = 5

[[anchors]]
type = "driveway"
name = "truck_in"
position = [-7.5, 0, 20]
forward = [0, 0, -1]
width_m = GATE_WIDTH
vehicle_class = "freight"

[[anchors]]
type = "loading_bay"
name = "bay"
position = [-7.5, 0, 6]
forward = [0, 0, -1]
width_m = 3.5
length_m = 8
vehicle_class = "freight"

[[site_surfaces]]
material = "asphalt"
vertices = [[-15, -20], [15, -20], [15, 20], [-15, 20]]
"#;

const LANE: &str = r#"
[[mesh_parts.passages]]
name = "lane"
position = [0, 0, 8]
forward = [0, 0, -1]
width_m = 5
length_m = 16
clear_height_m = 4.6
through = true
"#;

// Registers the yard, with its lane declared or not and its gate `gate_m` wide, with the bounds
// the game would import. Returns its id.
fn register_yard(core: &mut SimCore, lane: bool, gate_m: &str) -> String {
    let toml = YARD
        .replace("PASSAGE", if lane { LANE } else { "" })
        .replace("GATE_WIDTH", gate_m);
    let mut manifest = AssetManifest::from_str(&toml).unwrap();
    for part in &mut manifest.mesh_parts {
        part.imported_bounds = Some(match part.name.as_str() {
            "bunker" => [[-4.0, 0.0, -8.0], [4.0, 10.0, 8.0]],
            _ => [[-6.0, 0.0, -6.0], [6.0, 6.0, 6.0]],
        });
    }
    let id = manifest.qualified_id("freight");
    core.allocator
        .registry
        .register("freight", manifest, String::new());
    id
}

fn place_yard(lane: bool, gate_m: &str) -> (SimCore, u64) {
    let mut core = core();
    let id = register_yard(&mut core, lane, gate_m);
    let index = core
        .place_service_building_internal(&id, 0.0, -40.0)
        .unwrap();
    core.publish_pending_building_site_changes();
    let handle = core.allocator.building_handles.handle(index).to_bits();
    (core, handle)
}

fn dock(core: &SimCore, building: u64) -> Vec<(u64, Bay)> {
    core.parking.source_bays(PlaceSource::Dock {
        building,
        anchor: 3,
    })
}

#[test]
fn a_yard_publishes_its_dock_through_its_passage_and_into_no_supply() {
    let (mut core, building) = place_yard(true, "6");
    let docks = dock(&core, building);
    assert_eq!(docks.len(), 1, "{:?}", core.parking.dock_diagnostics);
    assert!(core.parking.dock_diagnostics.is_empty());
    // Straight in from the gate: every point of the route on the lane's centre line.
    let path = docks[0].1.path;
    let local = |p: [f32; 2]| {
        let b = &core.allocator.buildings[0];
        let d = [p[0] - b.center_x, p[1] - b.center_y];
        d[0] * b.facing_dir.y - d[1] * b.facing_dir.x
    };
    for point in path {
        assert!((local(point) - local(path[3])).abs() < 1e-3, "{path:?}");
    }
    // The car spot is supply; the dock is not.
    assert_eq!(core.parking.building_place_count(building), 1);
    assert_eq!(core.parking.counts().0, 1);
    core.parking.audit_docks();
    core.audit_every_parking_catchment();
    let gate = &core.allocator.freight_gates[&building];
    assert_eq!([gate.arrive.curb_pos.x, gate.arrive.curb_pos.y], path[0]);
    assert_eq!(gate.arrive, gate.leave);
    // A republication of the same lot keeps the dock under its handle.
    let manifest = core
        .allocator
        .registry
        .get(&core.allocator.buildings[0].asset_id)
        .unwrap();
    let manifest = manifest.manifest.clone();
    core.replace_asset_registry(vec![("freight".to_owned(), manifest, String::new())]);
    assert_eq!(dock(&core, building), docks);
    // So does a fleet fill: no car is put in a dock.
    core.parking.fill_test_fleet();
    assert_eq!(core.parking.counts().1, 1);
}

#[test]
fn a_dock_needs_its_passage_and_a_gate_four_metres_wide() {
    let (core, building) = place_yard(false, "6");
    assert!(dock(&core, building).is_empty());
    assert!(!core.allocator.freight_gates.contains_key(&building));
    assert_eq!(
        core.parking.dock_diagnostics[&building],
        [
            "freight:building.industrial.freight_yard loading bay 3: inside the building: mark the lane as a Passage on its mesh part"
        ]
    );
    let (core, building) = place_yard(true, "3.9");
    assert!(dock(&core, building).is_empty());
    assert_eq!(
        core.parking.dock_diagnostics[&building],
        [
            "freight:building.industrial.freight_yard loading bay 3: needs a freight Driveway at least 4 m wide: trucks use no car Driveway"
        ]
    );
    // The car spot is unaffected either way.
    assert_eq!(core.parking.building_place_count(building), 1);
}

#[test]
fn a_dock_goes_with_its_building_and_round_trips_a_save() {
    let (mut core, building) = place_yard(true, "6");
    let rows = core.parking.dock_rows();
    let gates = core.allocator.freight_gates.clone();
    let path = std::env::temp_dir().join(format!("metrum_docks_{}.sqlite", std::process::id()));
    let path_text = path.to_str().expect("a temporary path");
    core.save_game_settled_at(path_text, None, 0)
        .expect("the yard saves");
    core.load_game_internal(path_text).expect("the yard loads");
    let _ = std::fs::remove_file(&path);
    assert_eq!(core.parking.dock_rows(), rows);
    assert_eq!(core.allocator.freight_gates, gates);
    core.parking.audit_docks();
    assert!(core.bulldoze_building(0));
    core.publish_pending_building_site_changes();
    assert!(core.parking.dock_rows().is_empty());
    assert!(!core.allocator.freight_gates.contains_key(&building));
}

// A save of format 122 holds no dock: its load validates the yard once and publishes the dock a
// new city has. A current save is taken as it is, so one whose docks were dropped loads none.
#[test]
fn an_older_save_derives_the_docks_a_fresh_validation_gives() {
    use crate::simulation::save::schema::PRE_DOCKS_SAVE_VERSION;
    let (mut core, building) = place_yard(true, "6");
    let rows = core.parking.dock_rows();
    let gates = core.allocator.freight_gates.clone();
    assert_eq!(rows.len(), 1);
    // The save an older build wrote: no dock row.
    core.parking.replace_docks(building, &[]);
    core.allocator.freight_gates.clear();
    let path = std::env::temp_dir().join(format!("metrum_old_docks_{}.sqlite", std::process::id()));
    let path_text = path.to_str().expect("a temporary path");
    core.save_game_settled_at(path_text, None, 0)
        .expect("the yard saves");
    core.load_game_internal(path_text)
        .expect("a current save loads");
    assert!(
        core.parking.dock_rows().is_empty(),
        "a current save validates nothing"
    );
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(&format!(
            "UPDATE save_meta SET version = {PRE_DOCKS_SAVE_VERSION};"
        ))
        .unwrap();
    core.load_game_internal(path_text)
        .expect("format 122 loads");
    let _ = std::fs::remove_file(&path);
    assert_eq!(core.parking.dock_rows(), rows);
    assert_eq!(core.allocator.freight_gates, gates);
    core.parking.audit_docks();
}

// No asset of the test pack or the bundled packs has a freight Driveway, so none publishes a
// dock and no existing city changes (`PARK-19`, owner decision 11). Six carry a loading bay,
// which stays unused.
#[test]
fn no_fixture_pack_asset_publishes_a_dock() {
    use crate::simulation::parking::lots::has_freight_anchors;
    let mut core = core();
    crate::nodes::sim::core::scenario::install_bootstrap_assets(&mut core);
    let registry = &core.allocator.registry;
    let mut ids: Vec<_> = registry.qualified_ids().collect();
    ids.sort_unstable();
    let mut loading = Vec::new();
    for id in ids {
        let anchors = &registry.get(id).unwrap().manifest.anchors;
        assert!(!has_freight_anchors(anchors), "{id} could publish a dock");
        assert!(
            !anchors.iter().any(|a| a.anchor_type == AnchorType::Driveway
                && a.vehicle_class.as_deref() != Some("car")),
            "{id} has a Driveway that is not a car's"
        );
        if anchors
            .iter()
            .any(|a| a.anchor_type == AnchorType::LoadingBay)
        {
            loading.push(id);
        }
    }
    assert_eq!(
        loading,
        [
            "kenney:building.coal.mine",
            "kenney:building.commercial.small_grocery_store",
            "kenney:building.industrial.food_processing_plant",
            "kenney:building.industrial.milling_machines",
            "kenney:building.industrial.small_farm",
            "kenney:building.power.coal_power_plant",
        ]
    );
}

// The asset editor's payload of the yard as the editor sends it: anchors, paving and mesh parts
// with their passages, and the parts' imported bounds.
fn editor_payload_of(core: &SimCore, id: &str) -> serde_json::Value {
    use serde_json::json;
    let manifest = &core.allocator.registry.get(id).unwrap().manifest;
    let building = manifest.building.as_ref().unwrap();
    let anchor_type = |a: &crate::assets::Anchor| match a.anchor_type {
        AnchorType::Driveway => "driveway",
        AnchorType::Parking => "parking",
        AnchorType::LoadingBay => "loading_bay",
        _ => "other",
    };
    let params = json!({
        "frontage_forward": manifest.building_frontage_forward(),
        "lot_half_extents": [
            f32::from(building.lot_width_cells) * 5.0,
            f32::from(building.lot_depth_cells) * 5.0,
        ],
        "anchors": manifest.anchors.iter().map(|a| json!({
            "anchor_type": anchor_type(a), "position": a.position, "forward": a.forward,
            "width_m": a.width_m, "length_m": a.length_m,
            "vehicle_class": a.vehicle_class.as_deref(),
        })).collect::<Vec<_>>(),
        "site_surfaces": manifest.site_surfaces.iter().map(|s| json!({
            "material": "asphalt", "vertices": s.vertices,
        })).collect::<Vec<_>>(),
        "mesh_parts": manifest.mesh_parts.iter().map(|p| json!({
            "name": p.name, "position": p.position, "rotation_degrees": p.rotation_degrees,
            "scale": p.scale,
            "passages": p.passages.iter().map(|q| json!({
                "name": q.name, "position": q.position, "forward": q.forward,
                "width_m": q.width_m, "length_m": q.length_m,
                "clear_height_m": q.clear_height_m, "through": q.through,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let bounds: Vec<f32> = manifest
        .mesh_parts
        .iter()
        .flat_map(|p| p.imported_bounds.unwrap_or_default().into_iter().flatten())
        .collect();
    crate::simulation::parking::validation::editor_lot_payload(&params, &bounds)
}

// `PARK-18`: paving that stops 2 cm short of the lot's street edge leaves the way in from the
// Driveway unpaved. The placed lot rejects its spot, and the editor rejects it with the same
// words; paved to the edge, both accept it. The dock, proven from its gate, follows suit.
#[test]
fn the_editor_rejects_the_way_in_from_the_driveway_as_the_placed_lot_does() {
    for front in ["20", "19.98"] {
        let mut core = core();
        // The yard's spot moved off the street edge, reached through a car Driveway.
        let toml = YARD
            .replace("PASSAGE", LANE)
            .replace("GATE_WIDTH", "6")
            .replace("position = [8, 0, 20]", "position = [8, 0, 8]")
            .replace(
                "\n[[site_surfaces]]",
                "\n[[anchors]]\ntype = \"driveway\"\nposition = [8, 0, 20]\nforward = [0, 0, -1]\nwidth_m = 3\nvehicle_class = \"car\"\n\n[[site_surfaces]]",
            )
            .replace("[15, 20], [-15, 20]", &format!("[15, {front}], [-15, {front}]"));
        let mut manifest = AssetManifest::from_str(&toml).unwrap();
        for part in &mut manifest.mesh_parts {
            part.imported_bounds = Some(match part.name.as_str() {
                "bunker" => [[-4.0, 0.0, -8.0], [4.0, 10.0, 8.0]],
                _ => [[-6.0, 0.0, -6.0], [6.0, 6.0, 6.0]],
            });
        }
        let id = manifest.qualified_id("freight");
        core.allocator.registry.register("freight", manifest, String::new());
        let index = core.place_service_building_internal(&id, 0.0, -40.0).unwrap();
        core.publish_pending_building_site_changes();
        let handle = core.allocator.building_handles.handle(index).to_bits();
        let placed: Vec<_> = core.parking_rejections(index).into_owned();
        let editor = editor_payload_of(&core, &id);
        let edited: Vec<_> = editor["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| format!("{id} parking anchor {}: {}", r["anchor"], r["reason"].as_str().unwrap()))
            .collect();
        assert_eq!(edited, placed, "paving to z {front}");
        assert_eq!(placed.is_empty(), front == "20", "{placed:?}");
        if front != "20" {
            assert!(placed[0].ends_with("no room to drive in: the way in from the Driveway leaves the paved yard (the way needs about 1.8 m of clear paving)"), "{placed:?}");
        }
        // So is the dock's: its route from the gate is the editor's, point for point in lot
        // terms, and its way in from the gate is unpaved the same way.
        let docks = dock(&core, handle);
        if front != "20" {
            assert!(docks.is_empty());
            let edited: Vec<_> = editor["dock_rejected"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| format!("{id} loading bay {}: {}", r["anchor"], r["reason"].as_str().unwrap()))
                .collect();
            assert_eq!(edited, core.parking.dock_diagnostics[&handle]);
            continue;
        }
        assert_eq!(docks.len(), 1, "{:?}", core.parking.dock_diagnostics);
        let route = &editor["docks"][0]["route"];
        let b = &core.allocator.buildings[index];
        let (bx, bz) = crate::simulation::buildings::allocator::building_local_xz_basis(
            b.facing_dir,
            [0.0, 0.0, 1.0],
        );
        for (k, world) in docks[0].1.path.iter().enumerate() {
            let local = [0, 1].map(|i| route[k][i].as_f64().unwrap() as f32);
            let expected = Vector2::new(b.center_x, b.center_y) + bx * local[0] + bz * local[1];
            assert!(expected.distance_to(Vector2::new(world[0], world[1])) < 1e-3,
                "route point {k}: placed {world:?}, editor {local:?}");
        }
    }
}

// The inspector's dock lines (`dock_usable`, `dock_bays`, `dock_rejected`): a proven yard has
// its one dock usable and no reason; one without its passage none, with the reason, also after
// a load, which keeps no reason and derives it again.
#[test]
fn the_inspector_shows_a_buildings_docks_and_their_rejections() {
    let (core, _) = place_yard(true, "6");
    let (usable, bays, reasons) = core.dock_lines(0).expect("a yard with a gate");
    assert_eq!((usable, bays, reasons.len()), (1, 1, 0));
    let (mut core, building) = place_yard(false, "6");
    let reason = "freight:building.industrial.freight_yard loading bay 3: inside the building: mark the lane as a Passage on its mesh part";
    let lines = |core: &SimCore| {
        let (usable, bays, reasons) = core.dock_lines(0).expect("a yard with a gate");
        (usable, bays, reasons.into_owned())
    };
    assert_eq!(lines(&core), (0, 1, vec![reason.to_owned()]));
    let path = std::env::temp_dir().join(format!("metrum_dock_lines_{}.sqlite", std::process::id()));
    let path_text = path.to_str().expect("a temporary path");
    core.save_game_settled_at(path_text, None, 0).expect("the yard saves");
    core.load_game_internal(path_text).expect("the yard loads");
    let _ = std::fs::remove_file(&path);
    assert!(!core.parking.dock_diagnostics.contains_key(&building));
    assert_eq!(lines(&core), (0, 1, vec![reason.to_owned()]));
}
