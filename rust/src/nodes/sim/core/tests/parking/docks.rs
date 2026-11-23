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
    core.parking.replace_docks(building, &[], &[], &[]);
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

// `PARK-21`: the yard with a second gate, `truck_out`, east of the bunker, and the bay's way out
// through the lane's far end, a half circle of radius 7.5 m in 13 chords (each bend under 15°)
// behind the bunker, and north to `truck_out`. The shed is small and moved out of the way, and
// the car spot off the exit's line.
fn place_drive_through() -> (SimCore, u64, String) {
    let manifest = drive_through_manifest(true);
    let id = manifest.qualified_id("freight");
    let mut core = core();
    core.allocator.registry.register("freight", manifest, String::new());
    let index = core.place_service_building_internal(&id, 0.0, -40.0).unwrap();
    core.publish_pending_building_site_changes();
    let handle = core.allocator.building_handles.handle(index).to_bits();
    (core, handle, id)
}

// The drive-through yard's asset ([`place_drive_through`]), its bay naming the exit and the way
// out, or (`exit` false) neither: then a stage A dock backed out of through `truck_in`.
fn drive_through_manifest(exit: bool) -> AssetManifest {
    drive_through_manifest_at(exit, "7.5")
}

// [`drive_through_manifest`] with `truck_out` at local x `exit_x`.
fn drive_through_manifest_at(exit: bool, exit_x: &str) -> AssetManifest {
    let arc: Vec<_> = (0..=13)
        .map(|k| {
            let angle = std::f32::consts::PI * (1.0 - k as f32 / 13.0);
            format!("[{}, {}]", 7.5 * angle.cos(), -8.0 - 7.5 * angle.sin())
        })
        .collect();
    let drive = if exit {
        format!("length_m = 8\nvehicle_class = \"freight\"\nexit = \"truck_out\"\nroute_out = [{}]", arc.join(", "))
    } else {
        "length_m = 8\nvehicle_class = \"freight\"".to_owned()
    };
    let toml = YARD
        .replace("PASSAGE", LANE)
        .replace("GATE_WIDTH", "6")
        .replace("position = [5, 0, -10]", "position = [12, 0, 10]")
        .replace("position = [8, 0, 20]", "position = [12, 0, 20]")
        .replace(
            "vehicle_class = \"freight\"\n\n[[anchors]]\ntype = \"loading_bay\"",
            "vehicle_class = \"freight\"\n\n[[anchors]]\ntype = \"driveway\"\nname = \"truck_out\"\nposition = [EXIT_X, 0, 20]\nforward = [0, 0, -1]\nwidth_m = 6\nvehicle_class = \"freight\"\n\n[[anchors]]\ntype = \"loading_bay\"",
        )
        .replace("length_m = 8\nvehicle_class = \"freight\"", &drive)
        .replace("EXIT_X", exit_x);
    let mut manifest = AssetManifest::from_str(&toml).unwrap();
    for part in &mut manifest.mesh_parts {
        part.imported_bounds = Some(match part.name.as_str() {
            "bunker" => [[-4.0, 0.0, -8.0], [4.0, 10.0, 8.0]],
            _ => [[-1.0, 0.0, -1.0], [1.0, 2.0, 1.0]],
        });
    }
    manifest
}

#[test]
fn a_drive_through_dock_keeps_its_ways_on_its_row_and_in_a_save() {
    use crate::simulation::core::handles::Handle;
    let (mut core, building, id) = place_drive_through();
    let source = PlaceSource::Dock { building, anchor: 4 };
    let docks = core.parking.source_bays(source);
    assert_eq!(docks.len(), 1, "{:?}", core.parking.dock_diagnostics);
    let (handle, bay) = docks[0];
    let way = *core.parking.dock_way(Handle::from_bits(handle)).expect("the drive");
    // The derived way in is the row's route; the way out leaves the stop and ends at the exit's
    // road connection: the stop, 14 arc points, the exit's approach and connection.
    assert_eq!(way.route_in.as_slice(), bay.path);
    assert_eq!(way.route_out.as_slice().len(), 17);
    assert_eq!(way.route_out.as_slice()[0], bay.path[3]);
    // The editor proves the same drive over the asset alone.
    let editor = editor_payload_of(&core, &id);
    let dock = &editor["docks"][0];
    assert_eq!((dock["arrive"].as_str(), dock["leave"].as_str()), (Some("truck_in"), Some("truck_out")));
    assert_eq!(dock["route_out"].as_array().map(Vec::len), Some(17), "{editor:#}");
    // Saved with its row and loaded as saved; a save without the drive's format (123) loads its
    // stage A docks validating nothing.
    let path = std::env::temp_dir().join(format!("metrum_drive_{}.sqlite", std::process::id()));
    let path_text = path.to_str().expect("a temporary path");
    core.save_game_settled_at(path_text, None, 0).expect("the yard saves");
    core.load_game_internal(path_text).expect("the yard loads");
    assert_eq!(core.parking.source_bays(source), docks);
    assert_eq!(core.parking.dock_way(Handle::from_bits(handle)), Some(&way));
    let _ = std::fs::remove_file(&path);
    let (mut core, building) = place_yard(true, "6");
    let rows = core.parking.dock_rows();
    core.save_game_settled_at(path_text, None, 0).expect("the yard saves");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(&format!(
            "UPDATE save_meta SET version = {};",
            crate::simulation::save::schema::PRE_ROUTES_SAVE_VERSION
        ))
        .unwrap();
    core.parking.replace_docks(building, &[], &[], &[]);
    core.load_game_internal(path_text).expect("format 123 loads");
    let _ = std::fs::remove_file(&path);
    assert_eq!(core.parking.dock_rows(), rows);
    let (handle, _) = core.parking.source_bays(PlaceSource::Dock { building, anchor: 3 })[0];
    assert!(core.parking.dock_way(Handle::from_bits(handle)).is_none());
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
            "anchor_type": anchor_type(a), "name": a.name, "position": a.position,
            "forward": a.forward, "width_m": a.width_m, "length_m": a.length_m,
            "vehicle_class": a.vehicle_class.as_deref(), "exit": a.exit,
            "route_in": a.route_in, "route_out": a.route_out,
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
            assert!(placed[0].ends_with("no way to drive to this spot: straight from the Driveway, the way in from the Driveway leaves the paved yard, and no way along the row or straight down from the Driveway is clear (a car needs a 1.8 m strip of clear paving to drive on and about 5 m by 5 m of it where it turns): clear the way, move the spot, or draw this spot's own route (Edit this route)"), "{placed:?}");
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

// A carrier from a second yard 60 m east to the first, just dispatched: standing in its
// source's dock, which it has taken (`PARK-20`, owner decision 10).
fn dispatched_truck() -> (SimCore, u64, usize) {
    let (mut core, building) = place_yard(true, "6");
    let id = core.allocator.buildings[0].asset_id.clone();
    let source = core.place_service_building_internal(&id, 60.0, -40.0).unwrap();
    core.publish_pending_building_site_changes();
    let carrier = dispatch_shipment(&mut core, 0, source);
    (core, building, carrier)
}

// Loads and dispatches shipment `id` of packaged food from building `source` to the first yard,
// returning its truck.
fn dispatch_shipment(core: &mut SimCore, id: u64, source: usize) -> usize {
    use crate::simulation::economy::logistics::{
        CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus,
    };
    let catalog = load_runtime_economy_catalog().unwrap();
    core.logistics.shipments.push(Shipment {
        id,
        resource_runtime_id: catalog.resource_runtime_id_for_id("packaged_food").unwrap(),
        amount: 40.0,
        source: ShipmentEndpoint::Building(source),
        destination: ShipmentEndpoint::Building(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier: crate::simulation::economy::agents::AgentHandle::NONE,
        total_cost: 600.0,
        freight_cost: 50.0,
        eta_hours: 1,
        leg_started_hour: 0,
        trip_event_slot: 0,
        cargo_loaded: true,
    });
    core.logistics.next_shipment_id = id + 1;
    let mut treasury = 0.0;
    assert!(core.logistics.dispatch_shipment(
        id,
        &mut core.allocator,
        &mut core.agents,
        &mut core.parking,
        &core.transit_network,
        &core.region_graph,
        &mut treasury,
    ));
    let index = core.logistics.shipment_index(id).unwrap();
    core.agents
        .resolve_agent(core.logistics.shipments[index].carrier)
        .unwrap()
}

// One agent tick of `core`'s city.
fn tick_agents(core: &mut SimCore) {
    crate::thread_pools::install(|| {
        core.agents.tick_with_parking(
            &core.allocator,
            &mut core.transit_network,
            &mut core.region_graph,
            0.1,
            &core.time,
            &mut core.parking,
        )
    });
}

// [`dispatched_truck`] once it stands in the first yard's dock.
fn truck_at_dock() -> (SimCore, u64, usize) {
    use crate::simulation::parking::ParkingTripPhase;
    let (mut core, building, carrier) = dispatched_truck();
    for _ in 0..6_000 {
        if core.agents.parking_trip[carrier].phase == ParkingTripPhase::AtDock {
            return (core, building, carrier);
        }
        tick_agents(&mut core);
    }
    panic!("the truck never stood in the dock");
}

// Turns shipment 0's truck, standing in the first yard's dock, back to its source, as a
// delivering check does (`start_return_trip`): these yards have no economy to deliver to.
fn check_truck(core: &mut SimCore) -> bool {
    use crate::simulation::economy::logistics::ShipmentEndpoint;
    let ShipmentEndpoint::Building(source) = core.logistics.shipments[0].source else {
        panic!("a yard sent it");
    };
    let carrier = core
        .agents
        .resolve_agent(core.logistics.shipments[0].carrier)
        .unwrap();
    core.agents.route_freight_carrier_building_to_building(
        carrier,
        0,
        source,
        &core.allocator,
        &core.transit_network,
        &core.region_graph,
    )
}

// Shipment `index`'s truck over the next `ticks` agent ticks: its pose, transit and trip, and the
// holder of every dock, each tick.
fn shipment_trace(
    core: &mut SimCore,
    ticks: usize,
    index: usize,
) -> Vec<(u32, u32, u8, String, Vec<u64>)> {
    (0..ticks)
        .map(|_| {
            tick_agents(core);
            let i = core
                .agents
                .resolve_agent(core.logistics.shipments[index].carrier)
                .expect("the truck is under way");
            let holders = core
                .parking
                .dock_buildings()
                .into_iter()
                .flat_map(|b| core.parking.building_docks(b).collect::<Vec<_>>())
                .map(|dock| core.parking.dock_holder(dock).to_bits())
                .collect();
            let a = &core.agents;
            (
                a.pos_x[i].to_bits(),
                a.pos_y[i].to_bits(),
                a.transit[i],
                format!("{:?}", a.parking_trip[i]),
                holders,
            )
        })
        .collect()
}

// Saves `core`, runs `ticks` ticks, loads the save and runs them again: both runs are equal, and
// the truck has left the yard by their end.
fn assert_save_continues_equally(mut core: SimCore, name: &str, ticks: usize) {
    let live = save_and_continue(&mut core, name, ticks, 0, |_| {});
    assert!(live[0].3.contains("PullOut") && !live[ticks - 1].3.contains("PullOut"), "{live:?}");
}

// Saves `core`, runs `ticks` ticks following shipment `index`'s truck, applies `between` (an edit
// made after the save, before the load), loads the save and runs them again: both runs are equal.
// Returns the live run.
fn save_and_continue(
    core: &mut SimCore,
    name: &str,
    ticks: usize,
    index: usize,
    between: impl FnOnce(&mut SimCore),
) -> Vec<(u32, u32, u8, String, Vec<u64>)> {
    let path = std::env::temp_dir().join(format!("metrum_{name}_{}.sqlite", std::process::id()));
    let path_text = path.to_str().expect("a temporary path");
    core.save_game_settled_at(path_text, None, 0).expect("the yard saves");
    let live = shipment_trace(core, ticks, index);
    between(core);
    core.load_game_internal(path_text).expect("the yard loads");
    let _ = std::fs::remove_file(&path);
    assert_eq!(shipment_trace(core, ticks, index), live);
    live
}

// A save made while a truck stands in a dock loads with the truck still standing there and the
// dock still its own; the next tick leaves both as they were.
#[test]
fn a_truck_standing_in_a_dock_round_trips_a_save() {
    let (mut core, building, carrier) = truck_at_dock();
    let dock = core.parking.building_docks(building).next().unwrap();
    let trip = format!("{:?}", core.agents.parking_trip[carrier]);
    let pose = (core.agents.pos_x[carrier], core.agents.pos_y[carrier]);
    let path = std::env::temp_dir().join(format!("metrum_truck_dock_{}.sqlite", std::process::id()));
    let path_text = path.to_str().expect("a temporary path");
    core.save_game_settled_at(path_text, None, 0).expect("the yard saves");
    core.load_game_internal(path_text).expect("the yard loads");
    let _ = std::fs::remove_file(&path);
    let carrier = core
        .agents
        .resolve_agent(core.logistics.shipments[0].carrier)
        .unwrap();
    assert_eq!(format!("{:?}", core.agents.parking_trip[carrier]), trip);
    assert_eq!((core.agents.pos_x[carrier], core.agents.pos_y[carrier]), pose);
    assert_eq!(core.parking.building_docks(building).next(), Some(dock));
    assert_eq!(core.parking.dock_holder(dock), core.agents.agent_handle(carrier));
    tick_agents(&mut core);
    assert_eq!((core.agents.pos_x[carrier], core.agents.pos_y[carrier]), pose);
}

// A save made while a delivered truck backs out of the dock (`PARK-20`) continues as the live
// city does, through its lane attach, where the dock is freed, and on along the road.
#[test]
fn a_truck_backing_out_of_a_dock_round_trips_a_save() {
    use crate::simulation::parking::ParkingTripPhase;
    let (mut core, _, carrier) = truck_at_dock();
    assert!(check_truck(&mut core));
    for _ in 0..20 {
        tick_agents(&mut core);
    }
    assert_eq!(core.agents.parking_trip[carrier].phase, ParkingTripPhase::PullOut);
    assert_save_continues_equally(core, "truck_backing_out", 200);
}

// A save made right after a dispatch spawned its truck in its source's dock continues as the
// live city does, the truck backing out and taking the road.
#[test]
fn a_truck_spawned_in_a_dock_round_trips_a_save() {
    use crate::simulation::parking::ParkingTripPhase;
    let (mut core, _, carrier) = dispatched_truck();
    let trip = core.agents.parking_trip[carrier];
    assert_eq!(trip.phase, ParkingTripPhase::PullOut);
    assert_eq!(core.parking.dock_holder(trip.source), core.agents.agent_handle(carrier));
    assert_save_continues_equally(core, "truck_spawned_in_dock", 200);
}

// A truck turned back faces as it stood, nose into the bay, and backs out (owner decision 2):
// the pose the snapshot draws points against its travel.
#[test]
fn a_truck_backs_out_of_its_dock_facing_into_the_bay() {
    let (mut core, _, carrier) = truck_at_dock();
    let facing = |core: &SimCore| {
        let a = &core.agents;
        crate::nodes::sim::core::snapshot::access_phase_direction(
            core,
            carrier,
            a.pos_x[carrier],
            a.pos_y[carrier],
        )
        .expect("a pose")
    };
    let standing = facing(&core);
    assert!(check_truck(&mut core));
    assert!((facing(&core) - standing).length() < 1e-5, "it turned on the spot");
    let from = Vector2::new(core.agents.pos_x[carrier], core.agents.pos_y[carrier]);
    for _ in 0..10 {
        tick_agents(&mut core);
    }
    let to = Vector2::new(core.agents.pos_x[carrier], core.agents.pos_y[carrier]);
    let travel = (to - from).normalized();
    let nose = facing(&core);
    assert!(travel.dot(Vector2::new(nose.x, nose.z)) < -0.999, "travel {travel:?}, nose {nose:?}");
}

// Bulldozing a yard while a truck backs out of its dock cancels the shipment and removes the
// truck, as for one standing there; the dock goes with the building.
#[test]
fn bulldozing_a_yard_with_a_truck_backing_out_removes_both() {
    let (mut core, building, _) = truck_at_dock();
    assert!(check_truck(&mut core));
    for _ in 0..5 {
        tick_agents(&mut core);
    }
    let agents = core.agents.len();
    assert!(core.bulldoze_building(0));
    core.publish_pending_building_site_changes();
    assert_eq!(core.agents.len(), agents - 1, "the truck is removed");
    assert!(core.logistics.shipments.is_empty());
    assert!(
        core.parking
            .dock_rows()
            .iter()
            .all(|(source, _, _)| *source != PlaceSource::Dock { building, anchor: 3 })
    );
}

// Bulldozing a yard with a truck in its dock cancels the shipment and removes the truck, as for
// a truck inside; the dock goes with the building and holds nobody.
#[test]
fn bulldozing_a_yard_with_a_truck_in_its_dock_removes_both() {
    let (mut core, building, _) = truck_at_dock();
    let agents = core.agents.len();
    assert!(core.bulldoze_building(0));
    core.publish_pending_building_site_changes();
    assert_eq!(core.agents.len(), agents - 1, "the truck is removed");
    assert!(core.logistics.shipments.is_empty());
    assert!(dock(&core, building).is_empty());
    assert!(
        core.parking
            .dock_rows()
            .iter()
            .all(|(source, _, _)| *source != PlaceSource::Dock { building, anchor: 3 })
    );
    assert!(!core.allocator.freight_gates.contains_key(&building));
}

// `PARK-29`: truck `i`'s manoeuvre while it pulls in or out off the road: where it is, the
// snapshot's drawn point and facing, the polyline it walks and the end of the leg it is on.
type DrawnTruck = (Vector2, Vector2, Option<Vector2>, Vec<Vector2>, usize);
fn drawn_truck(core: &SimCore, i: usize) -> Option<DrawnTruck> {
    use crate::simulation::parking::ParkingTripPhase::{PullIn, PullOut};
    let trip = core.agents.parking_trip[i];
    let egress = match trip.phase {
        PullOut => true,
        PullIn => false,
        _ => return None,
    };
    if core.agents.current_lane_id[i] != usize::MAX {
        return None;
    }
    let (place, lane, distance) = if egress {
        (trip.source, core.agents.planned_attach_lane_id[i], core.agents.planned_attach_lane_d[i])
    } else {
        (trip.destination, core.agents.planned_detach_lane_id[i], core.agents.planned_detach_lane_d[i])
    };
    let path = core
        .parking
        .maneuver_path(place, lane as usize, distance, &core.transit_network, !egress, trip.freight_gate)?
        .as_slice()
        .to_vec();
    let stored = Vector2::new(core.agents.pos_x[i], core.agents.pos_y[i]);
    let target = crate::simulation::economy::agents::tick::parking_path_target_segment(stored, &path)?;
    let (x, z, facing) =
        crate::nodes::sim::core::snapshot::off_lane_car_pose(core, i, stored.x, stored.y);
    Some((stored, Vector2::new(x, z), facing.map(|f| Vector2::new(f.x, f.z)), path, target))
}

// Ticks `core` until `done` holds of truck `i`, at most 6,000 ticks.
fn tick_truck_until(core: &mut SimCore, i: usize, done: impl Fn(&SimCore, usize) -> bool) {
    for _ in 0..6_000 {
        if done(core, i) {
            return;
        }
        tick_agents(core);
    }
    panic!("not reached in 6,000 ticks: {:?}", core.agents.parking_trip[i]);
}

fn phase_of(core: &SimCore, i: usize) -> crate::simulation::parking::ParkingTripPhase {
    core.agents.parking_trip[i].phase
}

// A truck between its source's gate and the first yard's: on the road, its plan ending at the
// yard's gate.
fn truck_on_the_road() -> (SimCore, u64, usize) {
    use crate::simulation::economy::agents::TRANSIT_NETWORK;
    use crate::simulation::parking::ParkingTripPhase;
    let (mut core, building, carrier) = dispatched_truck();
    tick_truck_until(&mut core, carrier, |core, i| {
        let trip = core.agents.parking_trip[i];
        trip.freight_gate
            && trip.phase == ParkingTripPhase::None
            && core.agents.transit[i] == TRANSIT_NETWORK
    });
    assert_eq!(core.agents.parking_trip[carrier].target, building);
    (core, building, carrier)
}

// Save and reload at every phase of a yard trip (`PARK-20`): the live city and the loaded one
// continue tick for tick alike. On the road to the gate, pulling in, standing (`AtDock`, here
// through 100 ticks), refused at the end of the line before the gate with the dock held by
// another and pulling over to its place at the kerb there, and queued there (`PARK-36`), and
// backing
// out (`a_truck_backing_out_of_a_dock_round_trips_a_save`, `a_truck_spawned_in_a_dock_round_trips_a_save`).
#[test]
fn a_truck_round_trips_a_save_at_every_phase_of_its_yard_trip() {
    use crate::simulation::parking::ParkingTripPhase;
    let (mut core, _, _) = truck_on_the_road();
    let live = save_and_continue(&mut core, "truck_on_the_road", 400, 0, |_| {});
    assert!(live.iter().any(|t| t.3.contains("PullIn")), "it reached the gate");
    assert!(live[399].3.contains("AtDock"), "{:?}", live[399]);

    let (mut core, _, carrier) = dispatched_truck();
    tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) == ParkingTripPhase::PullIn);
    let live = save_and_continue(&mut core, "truck_pulling_in", 200, 0, |_| {});
    assert!(live[0].3.contains("PullIn") && live[199].3.contains("AtDock"), "{live:?}");

    let (mut core, _, _) = truck_at_dock();
    let live = save_and_continue(&mut core, "truck_standing", 100, 0, |_| {});
    assert!(live.iter().all(|t| t == &live[0]), "it stands");

    // A second truck, from the source's door (its dock is free again, but the first yard's is
    // held), refused at the gate.
    let (mut core, _, _) = truck_at_dock();
    let crate::simulation::economy::logistics::ShipmentEndpoint::Building(source) =
        core.logistics.shipments[0].source
    else {
        panic!("a yard sent it");
    };
    let second = dispatch_shipment(&mut core, 1, source);
    tick_truck_until(&mut core, second, |core, i| phase_of(core, i) == ParkingTripPhase::AtKerb);
    let left_lane_at = (core.agents.pos_x[second].to_bits(), core.agents.pos_y[second].to_bits());
    let live = save_and_continue(&mut core, "truck_into_line", 200, 1, |_| {});
    assert_ne!((live[199].0, live[199].1), left_lane_at, "it pulls over");
    assert!(live.iter().all(|t| {
        t.2 != crate::simulation::economy::agents::TRANSIT_IN_BUILDING && t.3.contains("AtKerb")
    }));

    // Queued for the dock at the kerb before the gate (`PARK-36`): it waits there, saved and
    // loaded alike.
    let live = save_and_continue(&mut core, "truck_queued_at_kerb", 100, 1, |_| {});
    assert!(live.iter().all(|t| t == &live[0] && t.3.contains("AtKerb")), "it waits");
}

// A truck a save from before the line moved to the gate (`PARK-36`) left waiting at the kerb past
// the gate (the door's kerb stop lay there) is served after the load: it stays where it stands
// while it waits and, granted the dock, drives to the gate as it was planned then (round the
// block once at most); the loaded city continues as the live one.
#[test]
fn an_older_save_with_a_truck_waiting_past_the_gate_is_served() {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::parking::ParkingTripPhase;
    let (mut core, building, _) = truck_at_dock();
    let crate::simulation::economy::logistics::ShipmentEndpoint::Building(source) =
        core.logistics.shipments[0].source
    else {
        panic!("a yard sent it");
    };
    let second = dispatch_shipment(&mut core, 1, source);
    tick_truck_until(&mut core, second, |core, i| phase_of(core, i) == ParkingTripPhase::AtKerb);
    // The older layout: at the kerb 15 m past the gate's lane point, its lane point beside it.
    let gate = core
        .allocator
        .freight_gate_entrance(building, &core.region_graph, &core.transit_network.lane_system)
        .unwrap();
    let lane = &core.transit_network.lane_system.lanes[core.agents.planned_detach_lane_id[second] as usize];
    let length = core.region_graph.edge(gate.edge_idx).physical_length;
    let gate_point =
        BuildingAllocator::sample_pos_on_edge(&core.region_graph, gate.edge_idx, gate.entrance_s_m / length);
    let gate_d = BuildingAllocator::project_point_to_polyline_s(&lane.geometry, gate_point);
    let old_d = gate_d + 15.0;
    assert!(old_d < lane.length - 4.0, "{gate_d} of {}", lane.length);
    let point = BuildingAllocator::sample_pos_on_lane(lane, old_d);
    let stop = crate::simulation::economy::agents::tick::kerb_stop_beside(&gate, &core.region_graph, point)
        .unwrap()
        .door_pos;
    let a = &mut core.agents;
    (a.pos_x[second], a.pos_y[second]) = (stop.x, stop.y);
    a.planned_detach_lane_d[second] = old_d;
    assert!(check_truck(&mut core), "the first turns back");
    let live = save_and_continue(&mut core, "truck_waiting_past_gate", 3_000, 1, |_| {});
    let waited = live.iter().take_while(|t| t.3.contains("AtKerb")).count();
    assert!(waited > 0 && live[..waited].iter().all(|t| (t.0, t.1) == (live[0].0, live[0].1)), "it stays");
    assert!(live.iter().any(|t| t.3.contains("AtDock")), "it is served: {:?}", live.last());
}

// The yard asset with its lane declared or not (without it the dock stops proving) and its
// loading bay at local z `bay_z`, with the parts' imported bounds.
fn yard_manifest(lane: bool, bay_z: &str) -> AssetManifest {
    let toml = YARD
        .replace("PASSAGE", if lane { LANE } else { "" })
        .replace("GATE_WIDTH", "6")
        .replace("position = [-7.5, 0, 6]", &format!("position = [-7.5, 0, {bay_z}]"));
    let mut manifest = AssetManifest::from_str(&toml).unwrap();
    for part in &mut manifest.mesh_parts {
        part.imported_bounds = Some(match part.name.as_str() {
            "bunker" => [[-4.0, 0.0, -8.0], [4.0, 10.0, 8.0]],
            _ => [[-6.0, 0.0, -6.0], [6.0, 6.0, 6.0]],
        });
    }
    manifest
}

// An asset reload with the yard as `manifest`: every yard's lot and docks are validated again.
fn reload_yard(core: &mut SimCore, manifest: AssetManifest) {
    core.replace_asset_registry(vec![("freight".to_owned(), manifest, String::new())]);
}

// The first yard's dock rows, in handle order: each handle and whether it is active.
fn yard_docks(
    core: &SimCore,
    building: u64,
) -> Vec<(crate::simulation::core::handles::Handle<crate::simulation::parking::Place>, bool)> {
    let active: Vec<_> = core.parking.building_docks(building).collect();
    core.parking
        .source_bays(PlaceSource::Dock { building, anchor: 3 })
        .into_iter()
        .map(|(bits, _)| {
            let dock = crate::simulation::core::handles::Handle::from_bits(bits);
            (dock, active.contains(&dock))
        })
        .collect()
}

// A dock an edit to its asset retires while a truck holds it, in each phase that holds one
// (`PARK-20`): the row goes inactive and stays with its route, the building's gate record goes
// with its last active dock, and the truck finishes its yard trip on the row. A truck turned back
// from it with no gate record waits in its bay for the parking phase to plan its way out from the
// dock's own access. The next reconcile after it has left removes the row.
#[test]
fn a_dock_retired_under_a_truck_stays_for_it_until_it_leaves() {
    use crate::simulation::parking::ParkingTripPhase::{AtDock, PullIn, PullOut};
    for phase in [PullIn, AtDock, PullOut] {
        let (mut core, building, carrier) = dispatched_truck();
        let reached = if phase == PullOut { AtDock } else { phase };
        tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) == reached);
        if phase == PullOut {
            assert!(check_truck(&mut core));
            tick_agents(&mut core);
        }
        let docks = yard_docks(&core, building);
        let [(dock, true)] = docks[..] else {
            panic!("{docks:?}");
        };
        let route = core.parking.dock_route(dock).unwrap();
        let holds = |core: &SimCore| {
            core.agents
                .holds_freight_dock(core.agents.handles.handle(carrier), dock)
        };
        reload_yard(&mut core, yard_manifest(false, "6"));
        assert_eq!(yard_docks(&core, building), [(dock, false)], "{phase:?}");
        assert!(!core.allocator.freight_gates.contains_key(&building));
        assert!(holds(&core), "{phase:?}");
        assert_eq!(phase_of(&core, carrier), phase);
        if phase != PullOut {
            tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) == AtDock);
            let a = &core.agents;
            assert_eq!([a.pos_x[carrier], a.pos_y[carrier]], route[3]);
            assert!(check_truck(&mut core), "turned back with no gate record");
            assert_eq!(phase_of(&core, carrier), PullOut);
            tick_agents(&mut core);
            let trip = core.agents.parking_trip[carrier];
            assert!(!trip.ready && !trip.retry_pending, "planned by the commit: {trip:?}");
            assert_eq!([core.agents.pos_x[carrier], core.agents.pos_y[carrier]], route[3]);
        }
        tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) != PullOut);
        assert!(!holds(&core));
        assert_eq!(
            core.agents.transit[carrier],
            crate::simulation::economy::agents::TRANSIT_NETWORK,
            "it left by the road"
        );
        reload_yard(&mut core, yard_manifest(false, "6"));
        assert!(yard_docks(&core, building).is_empty(), "{phase:?}");
    }
}

// A revalidation that leaves a dock's pose keeps it under its handle with its truck; one that
// moves it adds the new row and keeps the old one, inactive, for the truck standing in it, which
// leaves along its old route. The next truck is granted the new dock.
#[test]
fn a_dock_moved_under_a_standing_truck_keeps_its_old_row_until_the_truck_leaves() {
    use crate::simulation::parking::ParkingTripPhase::{AtDock, PullIn, PullOut};
    let (mut core, building, carrier) = truck_at_dock();
    let docks = yard_docks(&core, building);
    let [(old, true)] = docks[..] else {
        panic!("{docks:?}");
    };
    let holds = |core: &SimCore, dock| {
        core.agents
            .holds_freight_dock(core.agents.handles.handle(carrier), dock)
    };
    reload_yard(&mut core, yard_manifest(true, "6"));
    assert_eq!(yard_docks(&core, building), [(old, true)]);
    assert!(holds(&core, old));
    let stop = core.parking.dock_route(old).unwrap()[3];
    reload_yard(&mut core, yard_manifest(true, "4"));
    let docks = yard_docks(&core, building);
    assert!(docks.contains(&(old, false)) && docks.len() == 2, "{docks:?}");
    let new = docks.iter().find(|(_, active)| *active).unwrap().0;
    assert_ne!(core.parking.dock_route(new).unwrap()[3], stop);
    assert!(holds(&core, old));
    tick_agents(&mut core);
    let a = &core.agents;
    assert_eq!(([a.pos_x[carrier], a.pos_y[carrier]], phase_of(&core, carrier)), (stop, AtDock));
    assert!(check_truck(&mut core));
    tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) != PullOut);
    assert!(!holds(&core, old));
    // The next truck through the gate takes the new dock.
    let crate::simulation::economy::logistics::ShipmentEndpoint::Building(source) =
        core.logistics.shipments[0].source
    else {
        panic!("a yard sent it");
    };
    let second = dispatch_shipment(&mut core, 1, source);
    tick_truck_until(&mut core, second, |core, i| phase_of(core, i) == PullIn);
    assert_eq!(core.agents.parking_trip[second].destination, new);
    reload_yard(&mut core, yard_manifest(true, "4"));
    assert_eq!(yard_docks(&core, building), [(new, true)]);
}

// A stage A dock whose pose stays while its route changes under a truck holding it (`truck_in`
// moved 0.5 m west, so the gate's connection, approach and aisle move): the held row is retired
// with its route, as a moved dock is (`PARK-21` round 3; before, the row's route was overwritten
// under the truck, which jumped to its end), and a new row takes the new route. The truck drives
// its old route on in every holding phase, never moving farther in a tick than it drives, leaves
// through the old gate, and the next reconcile removes the retired row.
#[test]
fn a_stage_a_dock_whose_route_changes_under_a_truck_keeps_its_old_route_for_it() {
    use crate::simulation::parking::ParkingTripPhase::{AtDock, PullIn, PullOut};
    let moved_gate = || {
        let toml = YARD
            .replace("PASSAGE", LANE)
            .replace("GATE_WIDTH", "6")
            .replace("position = [-7.5, 0, 20]", "position = [-8, 0, 20]");
        let mut manifest = AssetManifest::from_str(&toml).unwrap();
        for part in &mut manifest.mesh_parts {
            part.imported_bounds = Some(match part.name.as_str() {
                "bunker" => [[-4.0, 0.0, -8.0], [4.0, 10.0, 8.0]],
                _ => [[-6.0, 0.0, -6.0], [6.0, 6.0, 6.0]],
            });
        }
        manifest
    };
    for phase in [PullIn, AtDock, PullOut] {
        let (mut core, building, carrier) = dispatched_truck();
        let reached = if phase == PullOut { AtDock } else { phase };
        tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) == reached);
        if phase == PullIn {
            for _ in 0..10 {
                tick_agents(&mut core);
            }
            assert_eq!(phase_of(&core, carrier), PullIn, "mid-route");
        }
        if phase == PullOut {
            assert!(check_truck(&mut core));
            for _ in 0..10 {
                tick_agents(&mut core);
            }
        }
        let docks = yard_docks(&core, building);
        let [(dock, true)] = docks[..] else {
            panic!("{docks:?}");
        };
        let route = core.parking.dock_route(dock).unwrap();
        reload_yard(&mut core, moved_gate());
        let docks = yard_docks(&core, building);
        assert!(docks.contains(&(dock, false)) && docks.len() == 2, "{phase:?}: {docks:?}");
        assert_eq!(core.parking.dock_route(dock).unwrap(), route, "{phase:?}: its route kept");
        let new = docks.iter().find(|(_, active)| *active).unwrap().0;
        let new_route = core.parking.dock_route(new).unwrap();
        assert_eq!(new_route[3], route[3], "{phase:?}: the same stop");
        assert_ne!(new_route[0], route[0], "{phase:?}: a new gate");
        let holds = |core: &SimCore| {
            core.agents
                .holds_freight_dock(core.agents.handles.handle(carrier), dock)
        };
        assert!(holds(&core), "{phase:?}");
        // Never farther in a tick than 3 m/s for 0.1 s: no jump to the route's end.
        let step = |core: &mut SimCore| {
            let before = Vector2::new(core.agents.pos_x[carrier], core.agents.pos_y[carrier]);
            tick_agents(core);
            let now = Vector2::new(core.agents.pos_x[carrier], core.agents.pos_y[carrier]);
            assert!(now.distance_to(before) < 0.31, "{phase:?}: jumped {before:?} -> {now:?}");
        };
        if phase != PullOut {
            while phase_of(&core, carrier) == PullIn {
                step(&mut core);
            }
            assert_eq!(phase_of(&core, carrier), AtDock);
            let a = &core.agents;
            assert_eq!([a.pos_x[carrier], a.pos_y[carrier]], route[3], "{phase:?}");
            assert!(check_truck(&mut core));
        }
        let mut last = [0.0; 2];
        while phase_of(&core, carrier) == PullOut {
            last = [core.agents.pos_x[carrier], core.agents.pos_y[carrier]];
            step(&mut core);
        }
        assert!(!holds(&core), "{phase:?}");
        let old_gate = Vector2::new(route[0][0], route[0][1]);
        let new_gate = Vector2::new(new_route[0][0], new_route[0][1]);
        let left = Vector2::new(last[0], last[1]);
        assert!(
            left.distance_to(old_gate) < left.distance_to(new_gate),
            "{phase:?}: it left through the old gate at {left:?}"
        );
        // One active row again, held by nobody, with the new route: the first row by handle
        // whose pose matches is taken up (the A2 rule), the other removed.
        reload_yard(&mut core, moved_gate());
        let docks = yard_docks(&core, building);
        let [(row, true)] = docks[..] else {
            panic!("{phase:?}: {docks:?}");
        };
        assert_eq!(core.parking.dock_route(row).unwrap(), new_route, "{phase:?}");
        assert!(core.parking.dock_holder(row).is_none(), "{phase:?}");
    }
}

// A truck on the road bound for a yard whose gate goes (its last dock retired by an asset edit)
// loses its plan in the gate pass over the shipments and plans again to the door's lane point,
// where it stops at the kerb as a truck without a yard (`PARK-36`).
#[test]
fn a_truck_bound_for_a_gate_that_goes_plans_again_to_the_door() {
    use crate::simulation::economy::agents::{ACCESS_PLAN_VALID, TRANSIT_ACCESS_INGRESS};
    let (mut core, building, carrier) = truck_on_the_road();
    let gate = core.allocator.freight_gates[&building].arrive.curb_pos;
    reload_yard(&mut core, yard_manifest(false, "6"));
    assert!(!core.allocator.freight_gates.contains_key(&building));
    assert_eq!(core.agents.access_flags[carrier] & ACCESS_PLAN_VALID, 0, "its plan went");
    tick_truck_until(&mut core, carrier, |core, i| {
        core.agents.transit[i] == TRANSIT_ACCESS_INGRESS
    });
    let a = &core.agents;
    let lane = &core.transit_network.lane_system.lanes[a.planned_detach_lane_id[carrier] as usize];
    let detach = crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_lane(
        lane,
        a.planned_detach_lane_d[carrier],
    );
    let door = core.allocator.entrances[0].curb_pos;
    assert!(detach.distance_to(door) < detach.distance_to(gate), "{detach:?}: door {door:?}, gate {gate:?}");
    tick_truck_until(&mut core, carrier, |core, i| {
        core.agents.parking_trip[i].phase == crate::simulation::parking::ParkingTripPhase::AtKerb
    });
    let trip = core.agents.parking_trip[carrier];
    assert_eq!(trip.target, core.allocator.building_handles.handle(0).to_bits());
    assert!(!trip.freight_gate);
}

// A save taken with a truck in a yard loads as saved after an edit to the asset retired its dock
// (`PARK-19`'s rule: a current save validates nothing, its dock rows and holders are taken as they
// are), so the loaded city continues as the live one; the edit takes effect at the next
// revalidation of the lot (an asset reload), which retires the row under the truck as any edit.
#[test]
fn a_save_with_a_truck_in_a_yard_loads_after_an_edit_retired_its_dock() {
    use crate::simulation::parking::ParkingTripPhase::PullOut;
    let (mut core, building, carrier) = truck_at_dock();
    let docks = yard_docks(&core, building);
    let live = save_and_continue(&mut core, "truck_dock_edited", 100, 0, |core| {
        core.allocator
            .registry
            .register("freight", yard_manifest(false, "6"), String::new());
    });
    assert!(live.iter().all(|t| t == &live[0]), "it stands");
    assert_eq!(yard_docks(&core, building), docks, "loaded as saved");
    let dock = docks[0].0;
    assert!(core.agents.holds_freight_dock(core.agents.handles.handle(carrier), dock));
    reload_yard(&mut core, yard_manifest(false, "6"));
    assert_eq!(yard_docks(&core, building), [(dock, false)]);
    assert!(check_truck(&mut core));
    tick_truck_until(&mut core, carrier, |core, i| phase_of(core, i) != PullOut);
    assert!(!core.agents.holds_freight_dock(core.agents.handles.handle(carrier), dock));
}

// The rendered evidence's city (`PARK-20`): two copies of a yard asset with a dock 120 m apart
// on a straight street, and one loaded shipment between them whose dispatch the trip wheel runs
// at the next slot, so a game that loads the save shows its truck back out of the source's dock,
// drive to the other yard, pull into its dock, stand there until its check and back
// out. The asset is read from `METRUM_FREIGHT_STILL_ASSET` (a directory holding `asset.toml`, for
// example a copy of a pack's coal mine with its passage declared; nothing is copied anywhere)
// and registered as `METRUM_FREIGHT_STILL_ID` (`pack:asset`, the id the game's packs give it).
// Writes `yard.sqlite` and `yard.json` (each dock's route and heading, world x and z) into
// `METRUM_FREIGHT_STILL_OUT`. A save loads without validating its docks, so the game draws the
// rows proven here. The shipment carries `METRUM_FREIGHT_STILL_RESOURCE`, an input the asset
// takes (default `machinery`, the coal mine's; `grain` for a food processor, `PARK-23`).
// `METRUM_FREIGHT_STILL_TRUCKS` (default 1) loaded shipments leave at once, or
// `METRUM_FREIGHT_STILL_STAGGER` slots apart (`PARK-36`: with one dock, the next ones wait on a
// drive-through way in or at the kerb), and an asset without a dock is taken as one when
// `METRUM_FREIGHT_STILL_DOCKS` is `0` (`PARK-36`: kerb deliveries). `METRUM_FREIGHT_STILL_TRACE`
// seconds prints the loaded save's run at 1x.
#[test]
#[ignore = "manual fixture for rendered freight yard stills; requires METRUM_FREIGHT_STILL_ASSET, _ID and _OUT"]
fn write_the_freight_yard_still_save() {
    let var = |name| std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"));
    let (dir, id, out) = (
        var("METRUM_FREIGHT_STILL_ASSET"),
        var("METRUM_FREIGHT_STILL_ID"),
        var("METRUM_FREIGHT_STILL_OUT"),
    );
    let text = std::fs::read_to_string(std::path::Path::new(&dir).join("asset.toml")).unwrap();
    let manifest = AssetManifest::from_str(&text).unwrap();
    let (pack, _) = id.split_once(':').expect("pack:asset");
    let mut core = SimCore::new(
        WorldConfig::new(512.0, 512.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    core.treasury.balance = 1_000_000.0;
    commit(&mut core, (-200.0, 0.0), (200.0, 0.0));
    core.allocator.registry.register(pack, manifest, dir.clone());
    // An explicit asset (the coal mine) as industry, any other (a shop given `placement_mode =
    // "explicit"`, `PARK-36`) as a service building.
    let mut place = |x, z| {
        core.place_industry_building_internal(&id, x, z)
            .or_else(|_| core.place_service_building_internal(&id, x, z))
            .unwrap()
    };
    // `METRUM_FREIGHT_STILL_MIRROR`: the yard east of the source, so its trucks arrive driving
    // east and meet its gate before its door (the coal mine's gate lies west of its door).
    // `METRUM_FREIGHT_STILL_SOURCE` "x,z" moves the source (`PARK-32`: across the road, so its
    // trucks cross into the yard's gate).
    let mirror = std::env::var("METRUM_FREIGHT_STILL_MIRROR").is_ok();
    let (source_x, source_z) = std::env::var("METRUM_FREIGHT_STILL_SOURCE").map_or(
        (if mirror { -60.0 } else { 60.0 }, -40.0),
        |v| {
            let (x, z) = v.split_once(',').expect("x,z");
            (x.parse().unwrap(), z.parse().unwrap())
        },
    );
    let (yard, source) = if mirror {
        let source = place(source_x, source_z);
        (place(60.0, -40.0), source)
    } else {
        (place(-60.0, -40.0), place(source_x, source_z))
    };
    core.publish_pending_building_site_changes();
    let handles: Vec<u64> = [yard, source]
        .map(|b| core.allocator.building_handles.handle(b).to_bits())
        .into();
    let docks: usize = std::env::var("METRUM_FREIGHT_STILL_DOCKS").map_or(1, |n| n.parse().unwrap());
    let trucks: u64 = std::env::var("METRUM_FREIGHT_STILL_TRUCKS").map_or(1, |n| n.parse().unwrap());
    for handle in &handles {
        assert_eq!(
            core.parking.building_docks(*handle).count(),
            docks,
            "{:?}",
            core.parking.dock_diagnostics
        );
    }
    use crate::simulation::economy::logistics::{
        CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus,
    };
    // `METRUM_FREIGHT_STILL_STAGGER` slots between the dispatches (default 0: all at once).
    let stagger: u64 = std::env::var("METRUM_FREIGHT_STILL_STAGGER").map_or(0, |n| n.parse().unwrap());
    let first_slot = core.time.absolute_slot() + 1;
    let catalog = load_runtime_economy_catalog().unwrap();
    for id in 0..trucks {
        core.logistics.shipments.push(Shipment {
            id,
            // An input the yard's asset takes (the coal mine's machinery), so the check delivers.
            resource_runtime_id: catalog
                .resource_runtime_id_for_id(
                    &std::env::var("METRUM_FREIGHT_STILL_RESOURCE").unwrap_or_else(|_| "machinery".into()),
                )
                .unwrap(),
            amount: 4.0,
            source: ShipmentEndpoint::Building(source),
            destination: ShipmentEndpoint::Building(yard),
            carrier_class: CarrierClass::Truck,
            status: ShipmentStatus::InTransit,
            carrier: crate::simulation::economy::agents::AgentHandle::NONE,
            total_cost: 100.0,
            freight_cost: 10.0,
            eta_hours: 1,
            leg_started_hour: core.logistics.hour_ticks,
            trip_event_slot: if stagger == 0 { 0 } else { first_slot + id * stagger },
            cargo_loaded: true,
        });
    }
    core.logistics.next_shipment_id = trucks;
    core.resume_restored_freight(&(0..trucks).collect::<Vec<_>>());
    let out = std::path::Path::new(&out);
    std::fs::create_dir_all(out).unwrap();
    let save = out.join("yard.sqlite");
    core.save_game_settled_at(save.to_str().unwrap(), None, 0)
        .expect("the still city saves");
    let docks: Vec<_> = handles
        .iter()
        .flat_map(|&handle| core.parking.building_docks(handle).collect::<Vec<_>>())
        .map(|dock| {
            let route = core.parking.dock_route(dock).unwrap();
            serde_json::json!({ "route": route })
        })
        .collect();
    let kerbs: Vec<_> = [yard, source]
        .iter()
        .map(|&b| {
            let kerb = crate::simulation::economy::agents::tick::kerb_stop_entrance(
                &core.allocator.entrances[b],
                &core.region_graph,
                crate::simulation::economy::agents::tick::FREIGHT_KERB_INSET_M,
            )
            .unwrap()
            .door_pos;
            [kerb.x, kerb.y]
        })
        .collect();
    let json = serde_json::json!({
        "docks": docks, "yard": handles[0], "source": handles[1], "kerbs": kerbs,
    });
    std::fs::write(out.join("yard.json"), json.to_string()).unwrap();
    // `METRUM_FREIGHT_STILL_TRACE` seconds: the save loaded and run at 1x as the game runs it,
    // 30 frames a second, printing each second every shipment and truck (`PARK-36`).
    let Ok(seconds) = std::env::var("METRUM_FREIGHT_STILL_TRACE") else {
        return;
    };
    core.load_game_internal(save.to_str().unwrap()).expect("the still city loads");
    core.time.speed_multiplier = 1.0;
    for frame in 0..seconds.parse::<usize>().unwrap() * 30 {
        core.step_frame(1.0 / 30.0);
        if frame % 30 != 29 {
            continue;
        }
        let shipments: Vec<_> = core
            .logistics
            .shipments
            .iter()
            .map(|s| format!("{}:{:?}", s.id, s.status))
            .collect();
        let a = &core.agents;
        let trucks: Vec<_> = (0..a.pos_x.len())
            .filter(|&i| a.freight_shipment_id[i] != u64::MAX)
            .map(|i| {
                format!(
                    "({:.1},{:.1}) transit {} {:?}",
                    a.pos_x[i], a.pos_y[i], a.transit[i], a.parking_trip[i].phase
                )
            })
            .collect();
        println!("t={}s shipments {shipments:?} trucks {trucks:?}", frame / 30 + 1);
    }
}

mod backs_in;
mod kerb_freight;
mod drive_through;
