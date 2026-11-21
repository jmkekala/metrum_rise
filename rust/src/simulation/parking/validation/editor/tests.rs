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
        "no room to drive in: the way to the front of the spot leaves the paved yard (a car drives straight from just inside the Driveway to about 2.5 m in front of the spot, on a 1.8 m strip of clear paving reaching half a car length past both ends)"
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
    let reason = "no room to drive in: the way to the front of the spot leaves the paved yard (a car drives straight from just inside the Driveway to about 2.5 m in front of the spot, on a 1.8 m strip of clear paving reaching half a car length past both ends)";
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
        "no room to drive in: the way in from the Driveway leaves the paved yard (the way needs about 1.8 m of clear paving)"
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
    assert_eq!(editor_lot_payload(&params, &[]), editor_payload(&params, &[]));
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

#[test]
fn an_arc_bends_under_fifteen_degrees_at_every_chord() {
    use super::super::route::arc_waypoints;
    let start = Vector2::new(0.0, 0.0);
    let heading = Vector2::new(0.0, -1.0);
    for (angle, chords) in [(180.0, 13), (90.0, 7), (15.0, 2), (14.9, 1), (360.0, 25)] {
        let arc = arc_waypoints(start, heading, 5.0, angle, true);
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
    assert!(arc_waypoints(start, heading, 0.0, 90.0, true).is_err());
}
