// SPDX-License-Identifier: GPL-3.0-or-later

//! The asset editor's lot check: the placed game's car and dock proofs over the asset alone,
//! on an assumed straight street flush with the lot (`PARK-18`, `PARK-19`).
//!
//! The placed check's support is the asset's paving, as the site derivation extends it, and the
//! frontage road's surface. On a straight street the derivation extends nothing (the front gap
//! is under its 1 cm floor) and the road surface starts at the lot's street edge, so the editor
//! takes the paving as authored and an open street beyond that edge. Every Driveway's road
//! connection lies where the placed one does: on the street edge straight out from the anchor,
//! `BUILDING_SITE_ROAD_SAMPLE_INSET_M` into the street. A curved street's verge and a real
//! road's width are left to the placed building's inspector.

use super::super::geometry::{overlaps, precise, rectangle};
use super::drive_lines::car_envelopes;
use super::route::{arc_waypoints, drive_envelopes, starts_at};
use super::{
    AnchorBay, DockWayInput, DriveLines, ProofClass, Rejection, STREET_APPROACH_M, dock_ways,
    driveway_points_inward, is_dock_class, is_freight_class, opens_onto_street, validate_bay,
    validate_car_route, validate_dock,
};
use crate::assets::asset::passage::{
    check_passages_in_bounds, class_structure_footprints, lot_axis_forward, passage_rectangle,
};
use crate::assets::asset::{MeshPart, Passage};
use crate::simulation::buildings::allocator::BUILDING_SITE_ROAD_SAMPLE_INSET_M;
use godot::prelude::Vector2;
use serde_json::{Map, Value, json};

// How far the assumed street reaches beyond the lot's street edge, and past its side edges. A
// corridor runs half a body length past its road connection (2.25 m for a car, 4 m for a
// truck); a real road with its sidewalk is wider than either.
const EDITOR_STREET_DEPTH_M: f32 = 8.0;

// A Driveway's width when the anchor gives none, as the placed proof reads it.
const DEFAULT_DRIVEWAY_WIDTH_M: f64 = 3.0;

// The asset as the editor sends it: anchors, paving and mesh parts in asset-local metres.
struct EditorLot<'a> {
    anchors: &'a [Value],
    support: Vec<Vec<[f64; 2]>>,
    // Lot axis pointing at the street and the lot's half width (x) and half depth (z).
    street: Option<(Vector2, Vector2)>,
    // The mesh parts with their passages and the bounds the editor imported, if any.
    parts: Vec<MeshPart>,
    // The structure rectangles each class meets, by `ProofClass` index.
    walls: [Vec<[Vector2; 4]>; 2],
    // Parts whose passages did not parse: (part index, error).
    passage_errors: Vec<(usize, String)>,
    // Whether the asset pins its cars to the straight path (`car_routes = "direct"`, `PARK-24`).
    direct: bool,
}

fn vector(value: &Value) -> Vector2 {
    Vector2::new(
        value[0].as_f64().unwrap_or(0.0) as f32,
        value[2].as_f64().unwrap_or(0.0) as f32,
    )
}

fn number(anchor: &Value, key: &str, fallback: f64) -> f32 {
    anchor[key].as_f64().unwrap_or(fallback) as f32
}

fn triple(value: &Value, fallback: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| value[i].as_f64().map_or(fallback[i], |v| v as f32))
}

fn point(p: Vector2) -> [f32; 2] {
    [p.x, p.y]
}

// An anchor's waypoints under `key` (`route_in` or `route_out`) as `[x, z]` in asset-local
// metres. O(waypoints).
fn waypoints(anchor: &Value, key: &str) -> Vec<Vector2> {
    anchor[key]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            Vector2::new(
                p[0].as_f64().unwrap_or(0.0) as f32,
                p[1].as_f64().unwrap_or(0.0) as f32,
            )
        })
        .collect()
}

// A car spot the lot check proves (`PARK-24`, `PARK-26`): its whole route from the road
// connection to the stop, how it was found and its Driveway's index among the usable ones (none
// for a street spot).
struct CarSpot {
    route: Vec<Vector2>,
    kind: &'static str,
    gate: Option<usize>,
}

impl<'a> EditorLot<'a> {
    // `part_bounds` holds six floats per mesh part, in part order: its imported mesh-root
    // bounds' minimum and maximum; a part without bounds has a box of no plan area. O(anchors +
    // support vertices + parts × P²).
    fn parse(params: &'a Value, part_bounds: &[f32]) -> Self {
        let anchors = params["anchors"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut support: Vec<Vec<[f64; 2]>> = params["site_surfaces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["material"] == "asphalt" || s["material"] == "concrete")
            .filter_map(|s| s["vertices"].as_array())
            .map(|vertices| {
                vertices
                    .iter()
                    .map(|p| [p[0].as_f64().unwrap_or(0.0), p[1].as_f64().unwrap_or(0.0)])
                    .collect()
            })
            .collect();
        let half_extents = params.get("lot_half_extents").map(|h| {
            Vector2::new(
                h[0].as_f64().unwrap_or(0.0) as f32,
                h[1].as_f64().unwrap_or(0.0) as f32,
            )
        });
        let street = params
            .get("frontage_forward")
            .map(|outward| vector(outward).normalized())
            .zip(half_extents);
        if let Some((outward, half)) = street {
            // The open street: from the lot's street edge outward, and past both side edges.
            let edge = outward.x.abs() * half.x + outward.y.abs() * half.y;
            let along = outward.y.abs() * half.x + outward.x.abs() * half.y;
            let road = rectangle(
                outward * (edge + EDITOR_STREET_DEPTH_M * 0.5),
                outward,
                2.0 * (along + EDITOR_STREET_DEPTH_M),
                EDITOR_STREET_DEPTH_M,
            );
            support.push(precise(&road));
        }
        let mut passage_errors = Vec::new();
        let parts: Vec<_> = params["mesh_parts"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, part)| {
                let passages = match part.get("passages") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(list) => serde_json::from_value::<Vec<Passage>>(list.clone())
                        .unwrap_or_else(|error| {
                            passage_errors.push((index, error.to_string()));
                            Vec::new()
                        }),
                };
                let bounds = part_bounds.get(index * 6..index * 6 + 6).and_then(|b| {
                    (b[3] - b[0] >= 0.001 && b[5] - b[2] >= 0.001)
                        .then(|| [[b[0], b[1], b[2]], [b[3], b[4], b[5]]])
                });
                MeshPart {
                    imported_bounds: bounds,
                    name: part["name"].as_str().unwrap_or_default().to_owned(),
                    position: triple(&part["position"], [0.0; 3]),
                    rotation_degrees: triple(&part["rotation_degrees"], [0.0; 3]),
                    scale: part["scale"].as_f64().unwrap_or(1.0) as f32,
                    pivot_offset: part
                        .get("pivot_offset")
                        .filter(|v| v.is_array())
                        .map(|v| triple(v, [0.0; 3])),
                    lods: Vec::new(),
                    passages,
                }
            })
            .collect();
        // Each class meets the parts' rectangles less the passages high enough for it, as the
        // site derivation cuts them.
        let walls = ProofClass::ALL.map(|class| {
            let mut local = Vec::new();
            for part in &parts {
                class_structure_footprints(part, class.proof().clear_height, &mut local);
            }
            local
                .into_iter()
                .map(|[a, b]| {
                    [
                        Vector2::new(a[0], a[1]),
                        Vector2::new(b[0], a[1]),
                        Vector2::new(b[0], b[1]),
                        Vector2::new(a[0], b[1]),
                    ]
                })
                .collect()
        });
        Self {
            anchors,
            support,
            street,
            parts,
            walls,
            passage_errors,
            direct: params["car_routes"] == "direct",
        }
    }

    // The anchors of `kind` passing `admits`, as (anchor index, centred bay), entry edge to
    // centre as `lot_proof` reads them.
    fn bays(&self, kind: &str, admits: impl Fn(&Value) -> bool) -> Vec<(usize, AnchorBay)> {
        self.anchors
            .iter()
            .enumerate()
            .filter(|(_, a)| a["anchor_type"] == kind && admits(a))
            .map(|(index, a)| {
                let forward = vector(&a["forward"]).normalized();
                let length = number(a, "length_m", 5.0);
                let bay = AnchorBay {
                    pos: vector(&a["position"]) + forward * (length * 0.5),
                    forward,
                    width: number(a, "width_m", 2.5),
                    length,
                };
                (index, bay)
            })
            .collect()
    }

    // The name, road connection and approach, and unit inward heading of every Driveway passing
    // `admits` that is at least the class's gate width and points into the lot. Without a street
    // the approach stands for the connection, as before `PARK-18`.
    fn driveway_paths(
        &self,
        class: ProofClass,
        admits: impl Fn(&Value) -> bool,
    ) -> Vec<(&'a str, [Vector2; 2], Vector2)> {
        let row = class.proof();
        self.anchors
            .iter()
            .filter(|a| a["anchor_type"] == "driveway" && admits(a))
            .filter(|a| number(a, "width_m", DEFAULT_DRIVEWAY_WIDTH_M) >= row.min_gate)
            .filter(|a| {
                self.street.is_none_or(|(outward, _)| {
                    driveway_points_inward(vector(&a["forward"]).normalized(), outward)
                })
            })
            .map(|a| {
                let position = vector(&a["position"]);
                let inward = vector(&a["forward"]).normalized();
                let approach = position + inward * row.lead;
                let connection = self.street.map_or(approach, |(outward, half)| {
                    let edge = outward.x.abs() * half.x + outward.y.abs() * half.y;
                    position
                        + outward
                            * (edge - position.dot(outward) + BUILDING_SITE_ROAD_SAMPLE_INSET_M)
                });
                let name = a["name"].as_str().unwrap_or_default().trim();
                (name, [connection, approach], inward)
            })
            .collect()
    }

    fn collision(&self, class: ProofClass) -> impl Fn(&[Vector2]) -> bool + '_ {
        move |foot| {
            self.walls[class as usize]
                .iter()
                .any(|wall| overlaps(foot, wall))
        }
    }

    // Every parking anchor's proof as the placed lot runs it, in anchor order: (anchor index,
    // bay, its spot or its rejection). A spot's kind is `street` from the street edge,
    // `straight` from a Driveway, `auto` along the drive lines (`PARK-24`) or `yours` along its
    // own route (`PARK-26`, on any lot: `car_routes` governs only the automatic routes). A
    // drive-line route whose points are the straight path's reads `straight`: it is drawn the
    // same.
    fn car_spots(&self) -> Vec<(usize, AnchorBay, Result<CarSpot, Rejection>)> {
        let parking = self.bays("parking", |_| true);
        let bays: Vec<_> = parking.iter().map(|&(_, bay)| bay).collect();
        let footprints: Vec<_> = bays
            .iter()
            .map(|b| rectangle(b.pos, b.forward, b.width, b.length))
            .collect();
        let has_driveway = self.anchors.iter().any(|a| a["anchor_type"] == "driveway");
        let (paths, inward): (Vec<_>, Vec<_>) = self
            .driveway_paths(ProofClass::Car, |_| true)
            .into_iter()
            .map(|(_, path, inward)| (path, inward))
            .unzip();
        // The editor proves in the lot's own frame, whose axes are x and z.
        let lines = (!self.direct).then_some(DriveLines {
            inward: &inward,
            axes: [Vector2::new(1.0, 0.0), Vector2::new(0.0, 1.0)],
        });
        let collision = self.collision(ProofClass::Car);
        parking
            .iter()
            .enumerate()
            .map(|(slot, &(index, bay))| {
                let own = waypoints(&self.anchors[index], "route_in");
                if !own.is_empty() {
                    let proof = validate_car_route(
                        slot,
                        &bays,
                        &footprints,
                        &self.support,
                        has_driveway,
                        &paths,
                        &inward,
                        &own,
                        &collision,
                    )
                    .map(|(route, gate)| CarSpot {
                        route,
                        kind: "yours",
                        gate: Some(gate),
                    });
                    return (index, bay, proof);
                }
                let entry = bay.pos - bay.forward * (bay.length * 0.5);
                // The street side of the edge is assumed open; the placed building's inspector
                // reports the real road connection.
                let street = self
                    .street
                    .filter(|&(outward, half)| opens_onto_street(entry, bay.forward, outward, half))
                    .map(|_| entry - bay.forward * STREET_APPROACH_M);
                let proof = validate_bay(
                    ProofClass::Car,
                    slot,
                    &bays,
                    &footprints,
                    &[],
                    &self.support,
                    has_driveway,
                    &paths,
                    street,
                    lines,
                    &collision,
                )
                .map(|(path, gate)| CarSpot {
                    route: path.to_vec(),
                    kind: match gate {
                        None => "street",
                        Some(gate) if path[1] == paths[gate][1] => "straight",
                        Some(_) => "auto",
                    },
                    gate,
                });
                (index, bay, proof)
            })
            .collect()
    }

    // The car payload: one rejection per parking anchor the placed proof would reject; and the
    // spots, one per parking anchor it accepts (`PARK-24`): its route, the line its car is drawn
    // along (`drawn`, `PARK-29`), its kind (`car_spots`),
    // its Driveway's index among the usable ones (none for a street spot) and the turn envelopes
    // its proof checked.
    fn car_payload(&self) -> (Map<String, Value>, Vec<Value>) {
        let mut spots = Vec::new();
        let mut rejected = Vec::new();
        for (index, bay, proof) in self.car_spots() {
            match proof {
                Ok(spot) => {
                    // A street spot is proven by its edge alone, with no envelope.
                    let envelopes: Vec<_> = (spot.kind != "street")
                        .then(|| car_envelopes(&spot.route, bay.forward, spot.kind == "straight"))
                        .into_iter()
                        .flatten()
                        .map(|shape| shape.map(point).to_vec())
                        .collect();
                    let route: Vec<_> = spot.route.iter().copied().map(point).collect();
                    // The line its car is drawn along, each bend an arc (`PARK-29`).
                    let drawn: Vec<_> = crate::simulation::parking::turns::rounded_line(
                        &spot.route,
                        crate::simulation::parking::turns::CAR_TURN_RADIUS_M,
                    )
                    .into_iter()
                    .map(point)
                    .collect();
                    spots.push(json!({"anchor":index, "route":route, "drawn":drawn,
                        "kind":spot.kind, "gate":spot.gate, "envelopes":envelopes}));
                }
                Err(rejection) => rejected.push(rejection_json(index, rejection)),
            }
        }
        let mut payload = Map::new();
        payload.insert("rejected".into(), Value::Array(rejected));
        payload.insert(
            "frontage_check".into(),
            "Road connection checked in the placed building inspector".into(),
        );
        (payload, spots)
    }

    // The dock payload, `None` for an asset with no truck's loading bay and no passage, so an
    // asset without either gets the car payload alone. Each loading bay is a dock with its
    // route, or a rejection that blocks export only when the asset has a gate; without one it
    // is a note. As `lot_proof`: no loading bay is proven without a freight Driveway, and every
    // dock arrives through the building's one gate.
    fn dock_payload(&self) -> Option<Map<String, Value>> {
        let docks = self.bays("loading_bay", |a| is_dock_class(a["vehicle_class"].as_str()));
        let has_passage = self.parts.iter().any(|p| !p.passages.is_empty());
        if docks.is_empty() && !has_passage && self.passage_errors.is_empty() {
            return None;
        }
        let freight = ProofClass::Freight;
        let freight_driveway = |a: &Value| {
            a["anchor_type"] == "driveway" && is_freight_class(a["vehicle_class"].as_str())
        };
        let has_freight_driveway = self.anchors.iter().any(freight_driveway);
        let has_gate = self.anchors.iter().any(|a| {
            freight_driveway(a)
                && number(a, "width_m", DEFAULT_DRIVEWAY_WIDTH_M) >= freight.proof().min_gate
        });
        let mut routes = Vec::new();
        let mut rejected = Vec::new();
        if has_freight_driveway {
            let bays: Vec<_> = docks.iter().map(|&(_, bay)| bay).collect();
            let footprints: Vec<_> = bays
                .iter()
                .map(|b| rectangle(b.pos, b.forward, b.width, b.length))
                .collect();
            // A truck's routes keep off the parking spots.
            let parking: Vec<_> = self
                .bays("parking", |_| true)
                .iter()
                .map(|(_, b)| rectangle(b.pos, b.forward, b.width, b.length))
                .collect();
            let (names, gates): (Vec<_>, Vec<_>) = self
                .driveway_paths(freight, |a| is_freight_class(a["vehicle_class"].as_str()))
                .into_iter()
                .map(|(name, path, _)| (name, path))
                .unzip();
            let ways: Vec<_> = docks
                .iter()
                .map(|&(index, _)| way_input(&self.anchors[index], &names))
                .collect();
            let prove = |slot: usize, only: Option<usize>| {
                validate_dock(
                    slot,
                    &bays,
                    &footprints,
                    &parking,
                    &self.support,
                    has_gate,
                    &gates,
                    only,
                    &ways[slot],
                    self.collision(freight),
                )
            };
            for (slot, (&(index, _), route)) in
                docks.iter().zip(dock_ways(docks.len(), prove)).enumerate()
            {
                match route {
                    Ok(dock) => {
                        let mut entry = json!({"anchor":index, "route":dock.path.map(point)});
                        // A drive-through's whole ways (`PARK-21`), for drawing and inspection,
                        // with the truck's envelope at each bend the proof checked.
                        if !dock.route_in.is_empty() {
                            entry["route_in"] = json!(dock.route_in.iter().copied().map(point).collect::<Vec<_>>());
                            entry["route_out"] = json!(dock.route_out.iter().copied().map(point).collect::<Vec<_>>());
                            entry["arrive"] = json!(names[dock.arrive]);
                            entry["leave"] = json!(names[dock.leave]);
                            let envelopes: Vec<_> = drive_envelopes(&dock, bays[slot].forward)
                                .into_iter()
                                .map(|shape| shape.map(point).to_vec())
                                .collect();
                            entry["envelopes"] = json!(envelopes);
                        }
                        routes.push(entry);
                    }
                    Err(rejection) => {
                        let mut entry = rejection_json(index, rejection);
                        entry["blocks"] = json!(has_gate);
                        rejected.push(entry);
                    }
                }
            }
        } else {
            rejected.extend(docks.iter().map(|&(index, _)| {
                json!({"anchor":index, "reason":"unused: no freight Driveway", "shape":null, "blocks":false})
            }));
        }
        let mut payload = Map::new();
        payload.insert("docks".into(), Value::Array(routes));
        payload.insert("dock_rejected".into(), Value::Array(rejected));
        payload.insert("passages".into(), Value::Array(self.passages_json()));
        payload.insert("passage_issues".into(), Value::Array(self.passage_issues()));
        Some(payload)
    }

    // Every passage's ground rectangle (no eaves margin), its mouth and far end, for drawing.
    // A passage off a lot axis has no rectangle; manifest validation names it.
    fn passages_json(&self) -> Vec<Value> {
        let mut list = Vec::new();
        for (part_index, part) in self.parts.iter().enumerate() {
            for (index, passage) in part.passages.iter().enumerate() {
                let (Some([min, max]), Some(forward)) = (
                    passage_rectangle(part, passage, false),
                    lot_axis_forward(part, passage),
                ) else {
                    continue;
                };
                let [x, _, z] = passage.position;
                let mouth = part
                    .local_transform()
                    .transform_point3(glam::Vec3::new(x, 0.0, z));
                let mouth = Vector2::new(mouth.x, mouth.z);
                let far = mouth + Vector2::new(forward[0], forward[1]) * passage.length_m;
                list.push(json!({
                    "part": part_index,
                    "index": index,
                    "name": passage.name,
                    "corners": [[min[0], min[1]], [max[0], min[1]], [max[0], max[1]], [min[0], max[1]]],
                    "mouth": point(mouth),
                    "far": point(far),
                    "through": passage.through,
                }));
            }
        }
        list
    }

    // Passages leaving their part's imported bounds by more than the eaves margin, as a pack
    // load refuses them, and passage lists that did not parse.
    fn passage_issues(&self) -> Vec<Value> {
        let bounds = self.parts.iter().enumerate().filter_map(|(index, part)| {
            check_passages_in_bounds(part)
                .err()
                .map(|message| json!({"part": index, "message": message}))
        });
        self.passage_errors
            .iter()
            .map(|(index, error)| {
                json!({"part": index, "message": format!("mesh part {} passages: {error}", index + 1)})
            })
            .chain(bounds)
            .collect()
    }
}

// A loading bay's drive-through as the editor sends it (`PARK-21`): `exit` by name, resolved
// among the usable gates' `names`, and `route_in` and `route_out` as `[x, z]` waypoints in
// asset-local metres. O(waypoints + gates).
fn way_input(anchor: &Value, names: &[&str]) -> DockWayInput {
    DockWayInput {
        route_in: waypoints(anchor, "route_in"),
        route_out: waypoints(anchor, "route_out"),
        exit: anchor["exit"].as_str().map(str::trim).map(|exit| {
            names.iter().position(|&name| name == exit).ok_or_else(|| exit.to_owned())
        }),
    }
}

fn rejection_json(anchor: usize, rejection: Rejection) -> Value {
    let shape = rejection.shape.map(|shape| shape.map(point).to_vec());
    let mut entry = json!({"anchor":anchor, "reason":rejection.reason, "shape":shape});
    // The leg of an authored way the reason names (`PARK-21`), or of the drive-line route a car
    // spot was last tried along (`PARK-24`), and that route itself.
    if let Some(leg) = rejection.leg {
        entry["leg"] = json!(leg.map(point));
    }
    if let Some(route) = rejection.route {
        entry["route"] = json!(route.into_iter().map(point).collect::<Vec<_>>());
    }
    entry
}

// The editor document's `params` as the lot check reads them: the document holds the lot in
// cells, the check its half extents. O(params).
fn document_lot(params: &Value) -> Value {
    let mut params = params.clone();
    if params.get("lot_half_extents").is_none() {
        let half = |key: &str| params[key].as_f64().unwrap_or(1.0) * 5.0;
        params["lot_half_extents"] = json!([half("lot_width_cells"), half("lot_depth_cells")]);
    }
    params
}

/// The waypoints the asset editor's arc helper inserts into loading bay `anchor`'s way in
/// (`way_in`) or way out (`PARK-21`), or into a parking spot's own route (`PARK-26`, a way in
/// only), after waypoint `after` (`None`: the last), as [`arc_waypoints`] lays them from that
/// point and the heading of the leg arriving at it, every bend under the class's limit (15° for
/// a truck, 30° for a car). The leg before the way's first waypoint comes from the stop along the
/// bay (way out), from the arrival gate's road connection to its approach (a loading bay's way
/// in: the first usable freight Driveway in anchor order that is not the bay's exit, else the
/// exit itself when it is the only one, on the assumed straight street), or from the road
/// connection of the car Driveway the spot's route starts at (the first usable one waypoint 1
/// lies straight ahead of, else the first usable one); with no waypoint a bay's arc starts at
/// the stop or the approach, and a spot without its own route has none to continue. `params` is
/// the editor document's. Returns the index to insert at and the points. O(anchors + waypoints +
/// chords); an edit boundary.
pub(crate) fn route_arc(
    params: &Value,
    anchor: usize,
    way_in: bool,
    after: Option<usize>,
    radius: f32,
    angle_deg: f32,
    left: bool,
) -> Result<(usize, Vec<[f32; 2]>), String> {
    let params = document_lot(params);
    let lot = EditorLot::parse(&params, &[]);
    let a = lot.anchors.get(anchor).ok_or("The loading bay no longer exists")?;
    let car = a["anchor_type"] == "parking";
    let waypoints = waypoints(a, if way_in { "route_in" } else { "route_out" });
    let at = after.map_or(waypoints.len(), |i| (i + 1).min(waypoints.len()));
    // The way's points before the inserted ones: what leads to the way's first waypoint, then
    // the waypoints up to `at`.
    let lead = if car {
        if !way_in || waypoints.is_empty() {
            return Err("Choose Edit this route first: an arc continues a parking spot's own way in".into());
        }
        let gates = lot.driveway_paths(ProofClass::Car, |_| true);
        let (_, [connection, _], _) = gates
            .iter()
            .find(|&&(_, path, inward)| starts_at(path, inward, waypoints[0]))
            .or(gates.first())
            .ok_or("Add a car Driveway at the street edge first: a spot's route starts at one")?;
        vec![*connection]
    } else if way_in {
        let exit = a["exit"].as_str().map(str::trim).unwrap_or_default();
        let freight = ProofClass::Freight;
        let gates = lot.driveway_paths(freight, |d| is_freight_class(d["vehicle_class"].as_str()));
        // The proof arrives through any usable gate, the exit included: a gate other than the
        // exit when the asset has one, else the exit itself (one gate both ways).
        let (_, [connection, approach], _) = gates
            .iter()
            .find(|(name, ..)| exit.is_empty() || *name != exit)
            .or(gates.first())
            .ok_or("Add a freight Driveway at the street edge first: a way in starts at its approach")?;
        vec![*connection, *approach]
    } else {
        let forward = vector(&a["forward"]).normalized();
        let stop = vector(&a["position"]) + forward * (number(a, "length_m", 5.0) * 0.5);
        vec![stop - forward, stop]
    };
    let mut points = lead;
    points.extend_from_slice(&waypoints[..at]);
    let n = points.len();
    let start = points[n - 1];
    let class = if car { ProofClass::Car } else { ProofClass::Freight };
    let limit = class.proof().bend_limit_deg;
    let arc = arc_waypoints(start, start - points[n - 2], radius, angle_deg, left, limit)?;
    if waypoints.len() + arc.len() > crate::assets::asset::validation::MAX_ROUTE_POINTS - 3 {
        return Err(format!(
            "A way holds at most {} waypoints",
            crate::assets::asset::validation::MAX_ROUTE_POINTS - 3
        ));
    }
    Ok((at, arc.into_iter().map(point).collect()))
}

/// The waypoints "Edit this route" gives parking spot `anchor` (`PARK-26`): the inner points of
/// the route the lot check shows for it (its proven route; when it is rejected, the drive-line
/// route last tried, else the straight path through the first usable car Driveway), rounded to
/// 0.1 mm as an authored TOML holds them. Refused for a spot that already has its own route, a
/// spot driven straight in from the street, and a lot without a usable car Driveway. `params` is
/// the editor document's, `part_bounds` six floats per mesh part. O(the lot check).
pub(crate) fn car_route_to_pin(
    params: &Value,
    part_bounds: &[f32],
    anchor: usize,
) -> Result<Vec<[f64; 2]>, String> {
    let params = document_lot(params);
    let lot = EditorLot::parse(&params, part_bounds);
    let a = lot.anchors.get(anchor).ok_or("The parking spot no longer exists")?;
    if a["anchor_type"] != "parking" {
        return Err("Only a parking spot's route is edited this way".into());
    }
    if !waypoints(a, "route_in").is_empty() {
        return Err("This spot already follows its own route".into());
    }
    let (_, bay, proof) = lot
        .car_spots()
        .into_iter()
        .find(|&(index, ..)| index == anchor)
        .ok_or("The parking spot no longer exists")?;
    let route = match proof {
        Ok(spot) if spot.kind == "street" => {
            return Err("This spot opens onto the street: a car drives straight in from the road, so it has no route to edit".into());
        }
        Ok(spot) => spot.route,
        Err(Rejection {
            route: Some(route), ..
        }) => route,
        Err(_) => {
            let row = ProofClass::Car.proof();
            let (_, [connection, approach], _) = *lot
                .driveway_paths(ProofClass::Car, |_| true)
                .first()
                .ok_or("Add a car Driveway at the street edge first: a spot's route starts at one")?;
            let aisle = bay.pos - bay.forward * (bay.length * 0.5 + row.lead);
            vec![connection, approach, aisle, bay.pos]
        }
    };
    let round = |v: f32| (f64::from(v) * 10_000.0).round() / 10_000.0;
    Ok(route[1..route.len() - 1]
        .iter()
        .map(|p| [round(p.x), round(p.y)])
        .collect())
}

/// The asset editor's car rejections (the car half of [`editor_lot_payload`], which the
/// fixtures' recorded digest reads): one `{anchor, reason, shape}` per parking anchor the
/// placed proof would reject, on an assumed straight street (`PARK-18`). `part_bounds` is six
/// floats per mesh part, its imported mesh-root bounds (none: no walls). O(A²D + AD × local
/// polygon overlay work) for A bays and D driveways, independent of city size.
#[cfg(test)]
pub(crate) fn editor_payload(params: &Value, part_bounds: &[f32]) -> Value {
    Value::Object(EditorLot::parse(params, part_bounds).car_payload().0)
}

/// The editor document's lot-check keys for a building `manifest`, as the editor sends them
/// (`car_routes` only on a pinned lot, `PARK-24`), or `None` for an asset that is no building.
/// O(anchors + surface vertices).
#[cfg(test)]
pub(crate) fn editor_params(manifest: &crate::assets::AssetManifest) -> Option<Value> {
    let building = manifest.building.as_ref()?;
    Some(json!({
        "car_routes": building.car_routes.map(|_| "direct"),
        "frontage_forward": manifest.building_frontage_forward(),
        "lot_half_extents": [
            f32::from(building.lot_width_cells) * 5.0,
            f32::from(building.lot_depth_cells) * 5.0,
        ],
        "anchors": manifest.anchors.iter().map(|a| json!({
            "anchor_type": match a.anchor_type {
                crate::assets::AnchorType::Driveway => "driveway",
                crate::assets::AnchorType::Parking => "parking",
                crate::assets::AnchorType::LoadingBay => "loading_bay",
                _ => "other",
            },
            "position": a.position,
            "forward": a.forward,
            "width_m": a.width_m,
            "length_m": a.length_m,
            "vehicle_class": a.vehicle_class.as_deref(),
        })).collect::<Vec<_>>(),
        "site_surfaces": manifest.site_surfaces.iter().map(|s| json!({
            "material": match s.material {
                crate::assets::SiteSurfaceMaterial::Asphalt => "asphalt",
                crate::assets::SiteSurfaceMaterial::Concrete => "concrete",
            },
            "vertices": s.vertices,
        })).collect::<Vec<_>>(),
    }))
}

/// The car rejections, the accepted spots' routes (`spots`, `PARK-24`) and, for an asset with a
/// truck's loading bay or a passage, its docks' routes, its loading bays' rejections (`blocks`
/// only when it has a freight gate), its passages' rectangles and their issues (`PARK-19`). Same
/// bound, with docks for bays.
pub(crate) fn editor_lot_payload(params: &Value, part_bounds: &[f32]) -> Value {
    let lot = EditorLot::parse(params, part_bounds);
    let (mut payload, spots) = lot.car_payload();
    payload.insert("spots".into(), Value::Array(spots));
    payload.extend(lot.dock_payload().into_iter().flatten());
    Value::Object(payload)
}

#[cfg(test)]
mod tests;
