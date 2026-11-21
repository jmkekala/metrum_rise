// SPDX-License-Identifier: GPL-3.0-or-later

//! The asset editor's lot check against the placed proof's rules.

use super::*;

#[test]
fn editor_payload_reports_missing_driveway_and_accepts_clear_asset() {
    let mut params = json!({"anchors":[{"anchor_type":"parking", "position":[0,0,8],
        "forward":[0,0,1], "width_m":2.5, "length_m":5.0}],
        "site_surfaces":[{"material":"asphalt", "vertices":[[-20,-20],[20,-20],[20,20],[-20,20]]}]});
    assert_eq!(
        editor_payload(&params, &[])["rejected"][0]["reason"],
        "needs a Driveway point, or put the spot's front edge on the street edge pointing into the lot"
    );
    params["anchors"]
        .as_array_mut()
        .unwrap()
        .push(json!({"anchor_type":"driveway",
        "position":[0,0,-10], "forward":[0,0,1]}));
    assert_eq!(editor_payload(&params, &[])["rejected"], json!([]));
}

#[test]
fn only_a_real_turn_needs_a_turning_envelope_and_the_rejection_names_it() {
    // Two end-on spots beside a shallow forecourt (the Kenney pharmacy layout).
    let params = json!({"frontage_forward":[0,0,-1], "lot_half_extents":[10,10],
        "anchors":[
        {"anchor_type":"parking", "position":[-3.71,0,-7.89], "forward":[-1,0,0],
            "width_m":2.5, "length_m":5.0},
        {"anchor_type":"parking", "position":[-3.78,0,-4.53], "forward":[-1,0,0],
            "width_m":2.5, "length_m":5.0},
        {"anchor_type":"driveway", "position":[0.68,0,-10], "forward":[0,0,1], "width_m":3.0}],
        "site_surfaces":[{"material":"asphalt",
        "vertices":[[-10,-10],[5.39,-10],[5.53,-2.69],[-10,-2.77]]}]});
    let rejected = editor_payload(&params, &[])["rejected"].clone();
    // The first spot's way in bends by about 12° at the aisle: no envelope there, whose
    // square would cross the street edge.
    assert_eq!(rejected.as_array().unwrap().len(), 1, "{rejected}");
    assert_eq!(rejected[0]["anchor"], 1);
    // The second is reached diagonally right under the shop front.
    assert_eq!(
        rejected[0]["reason"],
        "no way to drive to this spot: straight from the Driveway, the way to the front of the spot leaves the paved yard, and no way along the row or straight down from the Driveway is clear (a car needs a 1.8 m strip of clear paving to drive on and about 5 m by 5 m of it where it turns): clear the way, move the spot, or draw this spot's own route (Edit this route)"
    );
    assert_eq!(rejected[0]["shape"].as_array().unwrap().len(), 4);
}

#[test]
fn a_spot_needs_room_in_front_to_line_up_and_the_rejection_says_so() {
    // A shop-front row (the K-market layout): spots face the shop, the street runs behind.
    let mut params = json!({"frontage_forward":[0,0,1], "lot_half_extents":[20,15],
        "anchors":[
        {"anchor_type":"parking", "position":[-2.5,0,11.42], "forward":[0,0,-1],
            "width_m":2.5, "length_m":5.0},
        {"anchor_type":"driveway", "position":[-18.5,0,15], "forward":[0,0,-1], "width_m":3.0}],
        "site_surfaces":[{"material":"asphalt",
        "vertices":[[-20,-15],[20,-15],[20,15],[-20,15]]}]});
    let reason = "no way to drive to this spot: straight from the Driveway, the way to the front of the spot leaves the paved yard, and no way along the row or straight down from the Driveway is clear (a car needs a 1.8 m strip of clear paving to drive on and about 5 m by 5 m of it where it turns): clear the way, move the spot, or draw this spot's own route (Edit this route)";
    // Its open end is 1 m from the street edge, with no paving for the car to line up on.
    assert_eq!(
        editor_payload(&params, &[])["rejected"][0]["reason"],
        reason
    );
    // Moved towards the shop, it still cannot be reached from a Driveway in the lot's corner.
    params["anchors"][0]["position"] = json!([-2.5, 0, 7.5]);
    assert_eq!(
        editor_payload(&params, &[])["rejected"][0]["reason"],
        reason
    );
    params["anchors"][1]["position"] = json!([-17.0, 0, 15]);
    assert_eq!(editor_payload(&params, &[])["rejected"], json!([]));
}

#[test]
fn a_bay_opening_onto_the_street_needs_no_driveway() {
    // A detached-house layout: the bay's front edge is the lot's street edge.
    let mut params = json!({"frontage_forward":[0,0,1], "lot_half_extents":[10,15],
        "anchors":[{"anchor_type":"parking", "position":[-4.2,0,15],
        "forward":[0,0,-1], "width_m":2.5, "length_m":4.6}],
        "site_surfaces":[{"material":"asphalt",
        "vertices":[[-6.9,8.07],[4.18,8],[4.2,14.98],[-6.86,14.98]]}]});
    assert_eq!(editor_payload(&params, &[])["rejected"], json!([]));
    // Set back from the street edge, the same bay needs a Driveway.
    params["anchors"][0]["position"] = json!([-4.2, 0, 13]);
    params["anchors"][0]["length_m"] = json!(4.5);
    assert_eq!(
        editor_payload(&params, &[])["rejected"][0]["reason"],
        "needs a Driveway point, or put the spot's front edge on the street edge pointing into the lot"
    );
}

// A 20 m by 30 m lot facing +z (street edge z = 15), a car Driveway at its street edge, one
// spot inside, paved up to `paving_front`.
fn driveway_lot(paving_front: f32) -> Value {
    json!({"frontage_forward":[0,0,1], "lot_half_extents":[10,15],
        "anchors":[
        {"anchor_type":"parking", "position":[0,0,0], "forward":[0,0,-1],
            "width_m":2.5, "length_m":5.0},
        {"anchor_type":"driveway", "position":[0,0,15], "forward":[0,0,-1], "width_m":3.0}],
        "site_surfaces":[{"material":"asphalt",
        "vertices":[[-10,-15],[10,-15],[10,paving_front],[-10,paving_front]]}]})
}

#[test]
fn paving_short_of_the_street_edge_rejects_the_way_in_from_the_driveway() {
    assert_eq!(editor_payload(&driveway_lot(15.0), &[])["rejected"], json!([]));
    // Two centimetres short, as authored surfaces often stop (`PARK-18`).
    let rejected = editor_payload(&driveway_lot(14.98), &[])["rejected"].clone();
    assert_eq!(
        rejected[0]["reason"],
        "no way to drive to this spot: straight from the Driveway, the way in from the Driveway leaves the paved yard, and no way along the row or straight down from the Driveway is clear (a car needs a 1.8 m strip of clear paving to drive on and about 5 m by 5 m of it where it turns): clear the way, move the spot, or draw this spot's own route (Edit this route)"
    );
}

// A 30 m by 40 m yard facing +z, paved whole: a 12 m by 16 m bunker on the west with its
// lane at x -10..-5, a loading bay under it reached from a gate above the lane, and a car
// spot opening onto the street. As the placed yard of the dock tests.
fn yard(gate: Value, passage: bool) -> (Value, Vec<f32>) {
    let mut bunker = json!({"name":"bunker", "position":[-7.5,0,0],
        "rotation_degrees":[0,0,0], "scale":1.0});
    if passage {
        bunker["passages"] = json!([{"name":"lane", "position":[0,0,8], "forward":[0,0,-1],
            "width_m":5.0, "length_m":16.0, "clear_height_m":4.6, "through":true}]);
    }
    let params = json!({"frontage_forward":[0,0,1], "lot_half_extents":[15,20],
        "mesh_parts":[bunker],
        "anchors":[
        {"anchor_type":"parking", "position":[8,0,20], "forward":[0,0,-1],
            "width_m":2.5, "length_m":5.0},
        gate,
        {"anchor_type":"loading_bay", "position":[-7.5,0,6], "forward":[0,0,-1],
            "width_m":3.5, "length_m":8.0, "vehicle_class":"freight"}],
        "site_surfaces":[{"material":"asphalt",
        "vertices":[[-15,-20],[15,-20],[15,20],[-15,20]]}]});
    (params, vec![-4.0, 0.0, -8.0, 4.0, 10.0, 8.0])
}

// Whether a JSON `[x, z]` lies within a millimetre of `expected`.
fn at(point: &Value, expected: [f64; 2]) -> bool {
    (0..2).all(|i| point[i].as_f64().is_some_and(|v| (v - expected[i]).abs() < 1e-3))
}

fn gate(width: f32, class: &str) -> Value {
    json!({"anchor_type":"driveway", "position":[-7.5,0,20], "forward":[0,0,-1],
        "width_m":width, "vehicle_class":class})
}

#[test]
fn a_loading_bay_blocks_export_only_with_a_gate_and_its_dock_route_is_drawn() {
    // No freight Driveway: a note, and the car payload is the payload of before.
    let (params, bounds) = yard(gate(6.0, "car"), true);
    let payload = editor_lot_payload(&params, &bounds);
    assert_eq!(payload["dock_rejected"][0]["reason"], "unused: no freight Driveway");
    assert_eq!(payload["dock_rejected"][0]["blocks"], false);
    assert_eq!(payload["rejected"], editor_payload(&params, &bounds)["rejected"]);
    // A freight Driveway under 4 m: the narrow gate is reported, still as a note.
    let (params, bounds) = yard(gate(3.5, "freight"), true);
    let payload = editor_lot_payload(&params, &bounds);
    assert_eq!(
        payload["dock_rejected"][0]["reason"],
        "needs a freight Driveway at least 4 m wide: trucks use no car Driveway"
    );
    assert_eq!(payload["dock_rejected"][0]["blocks"], false);
    // A gate without the passage: the bay is inside the bunker and blocks export.
    let (params, bounds) = yard(gate(6.0, "freight"), false);
    let payload = editor_lot_payload(&params, &bounds);
    assert_eq!(
        payload["dock_rejected"][0]["reason"],
        "inside the building: mark the lane as a Passage on its mesh part"
    );
    assert_eq!(payload["dock_rejected"][0]["blocks"], true);
    // With the passage the dock proves straight in from the gate's road connection.
    let (params, bounds) = yard(gate(6.0, "freight"), true);
    let payload = editor_lot_payload(&params, &bounds);
    assert_eq!(payload["dock_rejected"], json!([]), "{payload}");
    assert_eq!(payload["docks"][0]["anchor"], 2);
    let route = &payload["docks"][0]["route"];
    assert!(at(&route[0], [-7.5, 20.05]), "on the street edge, 5 cm into the street: {route}");
    assert!(at(&route[3], [-7.5, 2.0]), "the bay's centre: {route}");
    let passage = &payload["passages"][0];
    assert!(at(&passage["mouth"], [-7.5, 8.0]) && at(&passage["far"], [-7.5, -8.0]), "{passage}");
    assert_eq!(payload["passage_issues"], json!([]));
}

#[test]
fn an_asset_without_a_loading_bay_or_passage_gets_the_car_payload_alone() {
    let params = driveway_lot(15.0);
    let mut payload = editor_lot_payload(&params, &[]);
    // Beside the car payload, the accepted spot's route (`PARK-24`).
    let spots = payload.as_object_mut().unwrap().remove("spots").unwrap();
    assert_eq!(spots[0]["kind"], "straight");
    assert_eq!(payload, editor_payload(&params, &[]));
}

#[test]
fn a_passage_leaving_its_part_is_an_issue() {
    let (mut params, bounds) = yard(gate(6.0, "freight"), true);
    params["mesh_parts"][0]["passages"][0]["position"] = json!([0, 0, 10]);
    let payload = editor_lot_payload(&params, &bounds);
    let message = payload["passage_issues"][0]["message"].as_str().unwrap_or_default();
    assert!(message.contains("leaves the part's bounds"), "{payload}");
}


// `PARK-21`: the yard with an exit gate `truck_out` at x 7.5 and a way out laid as the editor's
// arc helper lays it: straight through the lane to its far end at z -8, then a half circle of
// 7.5 m turning right behind the bunker, which ends heading for the exit.
#[test]
fn the_arc_helper_lays_a_drive_through_that_proves_without_an_envelope() {
    let (mut params, bounds) = yard(gate(6.0, "freight"), true);
    let anchors = params["anchors"].as_array_mut().unwrap();
    anchors[1]["name"] = json!("truck_in");
    anchors.push(json!({"anchor_type":"driveway", "name":"truck_out", "position":[7.5,0,20],
        "forward":[0,0,-1], "width_m":6.0, "vehicle_class":"freight"}));
    // The car spot off the exit's line.
    anchors[0]["position"] = json!([12, 0, 20]);
    anchors[2]["exit"] = json!("truck_out");
    anchors[2]["route_out"] = json!([[-7.5, -8.0]]);
    let (at_index, arc) = route_arc(&params, 2, false, None, 7.5, 180.0, false).unwrap();
    assert_eq!((at_index, arc.len()), (1, 13), "13 chords for a half circle");
    let end = arc[12];
    assert!((end[0] - 7.5).abs() < 1e-3 && (end[1] + 8.0).abs() < 1e-3, "{end:?}");
    // Every point on the circle about (0, -8), the first chord turning half a step.
    for p in &arc {
        assert!((p[0].hypot(p[1] + 8.0) - 7.5).abs() < 1e-3, "{p:?}");
    }
    let mut out = params["anchors"][2]["route_out"].as_array().unwrap().clone();
    out.extend(arc.iter().map(|p| json!(p)));
    params["anchors"][2]["route_out"] = json!(out);
    let payload = editor_lot_payload(&params, &bounds);
    assert_eq!(payload["dock_rejected"], json!([]), "{payload}");
    let dock = &payload["docks"][0];
    assert_eq!((dock["arrive"].as_str(), dock["leave"].as_str()), (Some("truck_in"), Some("truck_out")));
    assert_eq!(dock["route_out"].as_array().unwrap().len(), 17, "stop, 14 waypoints, the exit's two");
    assert_eq!(dock["envelopes"], json!([]), "no bend reaches 15°");
    // A way in laid from the arrival gate's approach, not the exit's: a quarter turn of 7
    // chords left from the approach heading -z.
    let (at_index, arc) = route_arc(&params, 2, true, None, 10.0, 90.0, true).unwrap();
    assert_eq!((at_index, arc.len()), (0, 7));
    let approach = &dock["route_in"][1];
    let first = arc[0];
    assert!(
        (first[0] as f64 - approach[0].as_f64().unwrap()) < 0.0,
        "it turns left (to -x) from the approach at {approach}: {first:?}"
    );
    // An arc on a way out needs the exit; a sharp first leg is rejected on its leg.
    params["anchors"][2]["route_out"] = json!([[-3.0, -8.0]]);
    let payload = editor_lot_payload(&params, &bounds);
    let rejected = &payload["dock_rejected"][0];
    assert!(rejected["reason"].as_str().unwrap().contains("off the spot's direction"), "{rejected}");
    assert!(at(&rejected["leg"][1], [-3.0, -8.0]), "{rejected}");
}

// Found authoring the power plant: with one freight gate that is also the bay's exit, "Add arc"
// on the way in found no arrival gate. The proof arrives through any usable gate, the exit
// included, so the arc starts from that gate's approach.
#[test]
fn the_way_ins_arc_starts_at_the_one_gate_even_when_it_is_the_exit() {
    let (mut params, _) = yard(gate(6.0, "freight"), true);
    params["anchors"][1]["name"] = json!("truck_gate");
    let before = route_arc(&params, 2, true, None, 10.0, 90.0, true).unwrap();
    params["anchors"][2]["exit"] = json!("truck_gate");
    let after = route_arc(&params, 2, true, None, 10.0, 90.0, true)
        .expect("the exit is the only gate, so the way in arrives through it");
    assert_eq!(after, before, "the same gate's approach, whether or not it is the exit");
    // With a second gate, the way in still starts at the gate that is not the exit.
    params["anchors"].as_array_mut().unwrap().push(json!({"anchor_type":"driveway",
        "name":"truck_out", "position":[7.5,0,20], "forward":[0,0,-1], "width_m":6.0,
        "vehicle_class":"freight"}));
    params["anchors"][2]["exit"] = json!("truck_out");
    assert_eq!(route_arc(&params, 2, true, None, 10.0, 90.0, true).unwrap(), before);
    params["anchors"][2]["exit"] = json!("truck_gate");
    let other = route_arc(&params, 2, true, None, 10.0, 90.0, true).unwrap();
    assert_ne!(other, before, "the way in arrives by truck_out when truck_gate is the exit");
}

#[test]
fn an_arc_bends_under_fifteen_degrees_at_every_chord() {
    use super::super::route::arc_waypoints;
    let start = Vector2::new(0.0, 0.0);
    let heading = Vector2::new(0.0, -1.0);
    for (angle, chords) in [(180.0, 13), (90.0, 7), (15.0, 2), (14.9, 1), (360.0, 25)] {
        let arc = arc_waypoints(start, heading, 5.0, angle, true, 15);
        if chords > 21 {
            assert!(arc.is_err(), "{angle}");
            continue;
        }
        let arc = arc.unwrap();
        assert_eq!(arc.len(), chords, "{angle}");
        let mut points = vec![start - heading, start];
        points.extend(arc.iter().copied());
        let last = *points.last().unwrap();
        let turned = heading.rotated(-angle.to_radians());
        points.push(last + turned);
        for w in points.windows(3) {
            let (a, b) = ((w[1] - w[0]).normalized(), (w[2] - w[1]).normalized());
            assert!(a.dot(b) > 15f32.to_radians().cos(), "{angle}: {w:?}");
        }
    }
    assert!(arc_waypoints(start, heading, 0.0, 90.0, true, 15).is_err());
}

// A place in line behind a 3 m drive (`PARK-24`): the straight path's square just inside the
// gate leaves the drive, the drive-line route needs none. Pinned to the straight path
// (`car_routes = "direct"`), the editor rejects it with today's reason.
#[test]
fn the_editor_routes_along_the_drive_lines_unless_the_lot_is_pinned() {
    let mut params = json!({"frontage_forward":[0,0,1], "lot_half_extents":[10,15],
        "anchors":[
        {"anchor_type":"parking", "position":[0,0,0], "forward":[0,0,-1],
            "width_m":2.5, "length_m":5.0},
        {"anchor_type":"driveway", "position":[0,0,15], "forward":[0,0,-1], "width_m":3.0}],
        "site_surfaces":[{"material":"asphalt",
        "vertices":[[-1.5,-15],[1.5,-15],[1.5,15],[-1.5,15]]}]});
    let routed = editor_lot_payload(&params, &[]);
    assert_eq!(routed["rejected"], json!([]));
    assert_eq!(routed["spots"][0]["kind"], "auto");
    assert_eq!(routed["spots"][0]["gate"], 0);
    // Straight down the drive: no bend, no envelope.
    assert_eq!(routed["spots"][0]["envelopes"], json!([]));
    assert_eq!(routed["spots"][0]["route"][3], json!([0.0, -2.5]));
    params["car_routes"] = json!("direct");
    let pinned = editor_lot_payload(&params, &[]);
    assert_eq!(pinned["spots"], json!([]));
    assert_eq!(
        pinned["rejected"][0]["reason"],
        "no room to drive in: the turn off the Driveway leaves the paved yard (a car needs about 5 m by 5 m of clear paving just inside the Driveway to turn, so keep the Driveway about 2.5 m from the lot's side edges)"
    );
    assert!(pinned["rejected"][0].get("route").is_none());
    // A spot painted across the drive: no way reaches the first one. The rejection names the
    // straight way's failure and carries the drive-line route it was last tried along, with the
    // shape that blocked it.
    params["car_routes"] = Value::Null;
    params["anchors"].as_array_mut().unwrap().push(json!({"anchor_type":"parking",
        "position":[-1.25,0,8], "forward":[1,0,0], "width_m":2.5, "length_m":5.0}));
    let blocked = editor_lot_payload(&params, &[]);
    let first = &blocked["rejected"][0];
    assert_eq!(first["anchor"], 0);
    assert_eq!(
        first["reason"],
        "no way to drive to this spot: straight from the Driveway, the way to the front of the spot crosses another parking spot, and no way along the row or straight down from the Driveway is clear (a car needs a 1.8 m strip of clear paving to drive on and about 5 m by 5 m of it where it turns): clear the way, move the spot, or draw this spot's own route (Edit this route)"
    );
    assert_eq!(first["route"][1][0], 0.0, "down the drive: {first}");
    assert_eq!(first["shape"].as_array().unwrap().len(), 4);
}

// Every fixture-pack building (the test pack and the bundled packs), proven pinned to the
// straight path and routed along its drive lines (`PARK-24`): every spot the straight path proves
// keeps its route and gate bit for bit, and the spots only the drive lines reach are recorded.
// They are what an unpinned test pack would gain; the bundled packs gain none, so the pin on the
// test pack is what holds the fixtures still.
#[test]
fn every_straight_spot_keeps_its_route_and_the_drive_lines_only_add_spots() {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    fn manifests(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                manifests(&path, out);
            } else if path.file_name().is_some_and(|n| n == "asset.toml") {
                out.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../godot");
    let mut files = Vec::new();
    for pack in ["tests/mods", "bootstrap/mods"] {
        manifests(&root.join(pack), &mut files);
    }
    files.sort();
    let bits = |spot: &Value| -> Vec<u64> {
        spot["route"].as_array().unwrap().iter().flat_map(|p| p.as_array().unwrap().iter())
            .map(|x| x.as_f64().unwrap().to_bits())
            .chain(spot["gate"].as_u64())
            .collect()
    };
    let mut rescued = BTreeMap::new();
    let mut straight = 0;
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let manifest = crate::assets::AssetManifest::from_str(&text).unwrap();
        let Some(mut params) = editor_params(&manifest) else {
            continue;
        };
        params["car_routes"] = json!("direct");
        let pinned = editor_lot_payload(&params, &[]);
        params["car_routes"] = Value::Null;
        let routed = editor_lot_payload(&params, &[]);
        let spots = |payload: &Value| -> BTreeMap<u64, Value> {
            payload["spots"].as_array().unwrap().iter()
                .map(|s| (s["anchor"].as_u64().unwrap(), s.clone()))
                .collect()
        };
        let (before, after) = (spots(&pinned), spots(&routed));
        for (anchor, spot) in &before {
            assert_eq!(bits(spot), bits(&after[anchor]), "{} anchor {anchor}", manifest.asset_id);
            straight += 1;
        }
        let gained: Vec<_> = after.keys().filter(|a| !before.contains_key(a)).copied().collect();
        if !gained.is_empty() {
            rescued.insert(manifest.asset_id.clone(), gained);
        }
    }
    assert!(straight > 40, "{straight}");
    // Paper check of the plan: the grocery's fourth place turns on the gate line. Proven without
    // mesh walls, as the fixtures' editor digest proves them.
    let expected = BTreeMap::from([
        ("building.commercial.small_grocery_store".to_owned(), vec![4]),
        ("building.power.coal_power_plant".to_owned(), vec![6]),
    ]);
    assert_eq!(rescued, expected);
}

// "Edit this route" and "Back to automatic" (`PARK-26`) on every fixture-pack building: every
// spot reached from a Driveway pins its shown route as its own, which proves as `yours` along the
// same points (to the 0.1 mm the pin rounds to) on a lot pinned to the straight path or not; with
// the route removed again the payload is the automatic one exactly. A street spot has no route
// to pin.
#[test]
fn every_shown_route_pins_as_its_own_and_back_to_automatic_restores_it() {
    use std::path::Path;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../godot");
    let mut pinned = 0;
    for pack in ["tests/mods", "bootstrap/mods"] {
        let mut stack = vec![root.join(pack)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name().is_none_or(|n| n != "asset.toml") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let manifest = crate::assets::AssetManifest::from_str(&text).unwrap();
                let Some(params) = editor_params(&manifest) else {
                    continue;
                };
                let automatic = editor_lot_payload(&params, &[]);
                for spot in automatic["spots"].as_array().unwrap() {
                    let anchor = spot["anchor"].as_u64().unwrap() as usize;
                    let pin = car_route_to_pin(&params, &[], anchor);
                    if spot["kind"] == "street" {
                        assert!(pin.unwrap_err().contains("opens onto the street"));
                        continue;
                    }
                    let mut own = params.clone();
                    own["anchors"][anchor]["route_in"] = json!(pin.unwrap());
                    let payload = editor_lot_payload(&own, &[]);
                    let yours = payload["spots"].as_array().unwrap().iter()
                        .find(|s| s["anchor"] == anchor)
                        .unwrap_or_else(|| panic!("{} anchor {anchor}: {}", manifest.asset_id, payload["rejected"]));
                    assert_eq!(yours["kind"], "yours");
                    let points = |s: &Value| -> Vec<f64> {
                        s["route"].as_array().unwrap().iter()
                            .flat_map(|p| p.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()))
                            .collect()
                    };
                    let (a, b) = (points(spot), points(yours));
                    assert_eq!(a.len(), b.len());
                    assert!(a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1e-4), "{a:?} {b:?}");
                    assert!(car_route_to_pin(&own, &[], anchor).unwrap_err().contains("already"));
                    own["anchors"][anchor].as_object_mut().unwrap().remove("route_in");
                    assert_eq!(editor_lot_payload(&own, &[]), automatic);
                    pinned += 1;
                }
            }
        }
    }
    assert!(pinned > 40, "{pinned}");
}

// A drive lot: its own route of three waypoints proves as `yours` on a pinned lot too, drawn
// whole with its two squares; an arc continues it in 30° chords; a failing one is rejected with
// the hint and the whole route it was tried along.
#[test]
fn an_own_route_is_drawn_whole_and_its_arc_bends_under_thirty_degrees() {
    let mut params = json!({"frontage_forward":[0,0,1], "lot_half_extents":[10,15],
        "car_routes":"direct",
        "anchors":[
        {"anchor_type":"parking", "position":[5,0,0], "forward":[0,0,-1],
            "width_m":2.5, "length_m":5.0,
            "route_in":[[0,10],[0,4.0],[2.5,4.2],[5,4.0]]},
        {"anchor_type":"driveway", "position":[0,0,15], "forward":[0,0,-1], "width_m":3.0}],
        "site_surfaces":[{"material":"asphalt",
        "vertices":[[-10,-15],[10,-15],[10,15],[-10,15]]}]});
    let payload = editor_lot_payload(&params, &[]);
    assert_eq!(payload["rejected"], json!([]));
    let spot = &payload["spots"][0];
    assert_eq!((spot["kind"].as_str(), spot["route"].as_array().unwrap().len()), (Some("yours"), 6));
    assert_eq!(spot["envelopes"].as_array().unwrap().len(), 2, "{spot}");
    let (at, arc) = route_arc(&params, 0, true, Some(1), 5.0, 90.0, true).unwrap();
    assert_eq!((at, arc.len()), (2, 4), "a quarter turn in four chords");
    assert!(route_arc(&params, 0, false, None, 5.0, 90.0, true).is_err());
    params["anchors"][0]["route_in"] = json!([[0,10],[2,-2.5]]);
    let rejected = &editor_lot_payload(&params, &[])["rejected"][0];
    assert!(rejected["reason"].as_str().unwrap().ends_with("or choose Back to automatic"), "{rejected}");
    assert_eq!(rejected["route"].as_array().unwrap().len(), 4);
}
