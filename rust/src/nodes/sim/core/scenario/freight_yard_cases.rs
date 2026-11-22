// SPDX-License-Identifier: GPL-3.0-or-later

//! The freight yard liveness run (`PARK-20`): the producer town with a freight gate and a dock
//! on every fixture asset that carries a loading bay, run for 14 days beside the same town whose
//! gates are car Driveways (no dock), counting deliveries by yard and by door.
//!
//! The fixture packs publish no dock (`PARK-19`, owner decision 11), so the run builds its own
//! variants in memory: each of the six assets with a `loading_bay` gets its bay turned to face
//! into the lot and a 4.5 m Driveway in front of it, at the first place across the lot where the
//! asset editor's dock proof passes. Nothing is written to a pack. The drive-through run
//! (`PARK-21`) first tries each asset as a drive-through: a second gate 14 m across the street
//! edge as the bay's exit, and a way out straight on past the bay and round a half circle of 7 m
//! in 13 chords to it; an asset where no such drive proves keeps its stage A dock.

use super::{FRAME_DT_S, GrowthScenario};
use crate::assets::{AnchorType, AssetManifest};
use crate::nodes::sim::core::state::SimCore;
use crate::simulation::economy::logistics::{ShipmentEndpoint, ShipmentStatus};
use crate::simulation::parking::ParkingTripPhase;
use std::collections::BTreeMap;

const DAYS: u32 = 14;
const GATE_WIDTH_M: f32 = 4.5;

// The asset editor's dock proof of `manifest` on a straight street: its proven docks, and the
// rejections.
fn proven_docks(manifest: &AssetManifest) -> (usize, String) {
    use serde_json::json;
    let building = manifest.building.as_ref().unwrap();
    let anchor_type = |a: &crate::assets::Anchor| match a.anchor_type {
        AnchorType::Driveway => "driveway",
        AnchorType::Parking => "parking",
        AnchorType::LoadingBay => "loading_bay",
        _ => "other",
    };
    let route = |points: &[[f32; 2]]| (!points.is_empty()).then(|| json!(points));
    let params = json!({
        "frontage_forward": manifest.building_frontage_forward(),
        "lot_half_extents": [
            f32::from(building.lot_width_cells) * 5.0,
            f32::from(building.lot_depth_cells) * 5.0,
        ],
        "anchors": manifest.anchors.iter().map(|a| json!({
            "anchor_type": anchor_type(a), "position": a.position, "forward": a.forward,
            "width_m": a.width_m, "length_m": a.length_m,
            "vehicle_class": a.vehicle_class.as_deref(), "name": a.name,
            "exit": a.exit.as_deref(), "route_in": route(&a.route_in),
            "route_out": route(&a.route_out),
        })).collect::<Vec<_>>(),
        "site_surfaces": manifest.site_surfaces.iter().map(|s| json!({
            "material": "asphalt", "vertices": s.vertices,
        })).collect::<Vec<_>>(),
        "mesh_parts": manifest.mesh_parts.iter().map(|p| json!({
            "name": p.name, "position": p.position, "rotation_degrees": p.rotation_degrees,
            "scale": p.scale, "passages": [],
        })).collect::<Vec<_>>(),
    });
    let bounds: Vec<f32> = manifest
        .mesh_parts
        .iter()
        .flat_map(|p| p.imported_bounds.unwrap_or_default().into_iter().flatten())
        .collect();
    let payload = crate::simulation::parking::validation::editor_lot_payload(&params, &bounds);
    (
        payload["docks"].as_array().map_or(0, Vec::len),
        payload["dock_rejected"].to_string(),
    )
}

// `manifest` with a freight gate of class `class` (`freight`: a dock; `car`: none, the control)
// straight in front of its loading bay, the bay turned to face into the lot, at the first lateral
// and depth position, scanning the lot in 0.5 m steps, where the freight variant's dock proves.
// The error names the reasons at the lot's middle when no position proves.
fn with_gate(manifest: &AssetManifest, class: &str) -> Result<AssetManifest, String> {
    with_dock(manifest, class, false)
}

// `PARK-21`: the half circle's radius, so the exit stands twice that across from the arrival gate.
const DRIVE_RADIUS_M: f32 = 7.0;

// [`with_gate`], or with `through` a drive-through dock: an exit gate `truck_exit` of the same
// class 14 m across the street edge from the arrival gate (either side), and the bay's way out
// straight on to a turning point, then a half circle of 7 m in 13 chords (the editor's arc
// helper's chords) ending in line with the exit. The turning point is scanned past the bay in
// 0.5 m steps. The control keeps the same geometry with car gates.
fn with_dock(manifest: &AssetManifest, class: &str, through: bool) -> Result<AssetManifest, String> {
    let building = manifest.building.as_ref().ok_or("no building")?;
    let mut reasons = String::new();
    let forward = manifest.building_frontage_forward();
    // Into the lot from the street, and across it.
    let inward = [-forward[0], 0.0, -forward[2]];
    let across = [inward[2], 0.0, -inward[0]];
    let half_width = f32::from(building.lot_width_cells) * 5.0;
    let half_depth = f32::from(building.lot_depth_cells) * 5.0;
    let at = |lateral: f32, depth: f32| {
        [
            across[0] * lateral + inward[0] * depth,
            0.0,
            across[2] * lateral + inward[2] * depth,
        ]
    };
    // First as authored; then with the car spots within 5 m of the gate's line removed, for a
    // lot whose spots leave no straight way in; then also paved whole, for a lot whose paving
    // leaves no room for a truck's bay (the control is the same variant).
    for (clear, pave) in [(false, false), (true, false), (true, true)] {
        let mut lateral = -half_width + GATE_WIDTH_M;
        while lateral <= half_width - GATE_WIDTH_M {
            let mut depth = -half_depth + 4.5;
            while depth <= half_depth - 4.0 {
                let mut variant = manifest.clone();
                if pave {
                    let (w, d) = (half_width, half_depth);
                    variant.site_surfaces.truncate(1);
                    variant.site_surfaces[0].vertices = vec![[-w, -d], [w, -d], [w, d], [-w, d]];
                }
                if clear {
                    variant.anchors.retain(|a| {
                        let p = a.position;
                        a.anchor_type != AnchorType::Parking
                            || (p[0] * across[0] + p[2] * across[2] - lateral).abs() >= 5.0
                    });
                }
                let bay = variant
                    .anchors
                    .iter()
                    .position(|a| a.anchor_type == AnchorType::LoadingBay)
                    .unwrap();
                let anchor = &mut variant.anchors[bay];
                anchor.position = at(lateral, depth);
                anchor.forward = inward;
                let mut gate = variant
                    .anchors
                    .iter()
                    .find(|a| a.anchor_type == AnchorType::Driveway)
                    .ok_or("no driveway")?
                    .clone();
                gate.name = "truck_gate".to_owned();
                gate.position = at(lateral, -half_depth);
                gate.forward = inward;
                gate.width_m = Some(GATE_WIDTH_M);
                gate.vehicle_class = Some("freight".to_owned());
                variant.anchors.push(gate);
                if through {
                    if let Some(mut drive) = drive_through(&variant, bay, at, inward, lateral, half_width) {
                        for anchor in &mut drive.anchors {
                            if anchor.vehicle_class.as_deref() == Some("freight")
                                && anchor.anchor_type == AnchorType::Driveway
                            {
                                anchor.vehicle_class = Some(class.to_owned());
                            }
                        }
                        return Ok(drive);
                    }
                    depth += 0.5;
                    continue;
                }
                let (docks, rejected) = proven_docks(&variant);
                if docks == 1 {
                    variant.anchors.last_mut().unwrap().vehicle_class = Some(class.to_owned());
                    return Ok(variant);
                }
                if lateral.abs() < 0.25 && reasons.len() < 2_000 {
                    reasons += &format!("\n  at depth {depth}: {rejected}");
                }
                depth += 0.5;
            }
            lateral += 0.5;
        }
    }
    Err(reasons)
}

// `variant` (its bay `bay` placed, its arrival gate last) made a drive-through that proves, if
// any exit side and turning point does. `at(lateral, depth)` places a lot point.
fn drive_through(
    variant: &AssetManifest,
    bay: usize,
    at: impl Fn(f32, f32) -> [f32; 3],
    inward: [f32; 3],
    lateral: f32,
    half_width: f32,
) -> Option<AssetManifest> {
    use godot::prelude::Vector2;
    let building = variant.building.as_ref()?;
    let half_depth = f32::from(building.lot_depth_cells) * 5.0;
    let anchor = &variant.anchors[bay];
    let length = anchor.length_m.unwrap_or(5.0);
    let stop_depth = {
        let p = anchor.position;
        // The bay's depth from the street edge inward: its position along `inward`, plus half
        // its length.
        let along = |q: [f32; 3]| q[0] * inward[0] + q[2] * inward[2];
        along(p) + length * 0.5
    };
    for side in [1.0f32, -1.0] {
        let exit_lateral = lateral + side * 2.0 * DRIVE_RADIUS_M;
        if exit_lateral.abs() > half_width - GATE_WIDTH_M {
            continue;
        }
        let mut turn = stop_depth + 2.0;
        while turn + DRIVE_RADIUS_M <= half_depth - 2.0 {
            let mut drive = variant.clone();
            let mut exit = drive.anchors.last().unwrap().clone();
            exit.name = "truck_exit".to_owned();
            exit.position = at(exit_lateral, -half_depth);
            drive.anchors.push(exit);
            let start = at(lateral, turn);
            let start = Vector2::new(start[0], start[2]);
            let heading = Vector2::new(inward[0], inward[2]);
            // Left of the heading, as a driver sees it, is (h.z, -h.x).
            let across = at(side, 0.0);
            let left = Vector2::new(heading.y, -heading.x).dot(Vector2::new(across[0], across[2])) > 0.0;
            let arc = crate::simulation::parking::validation::arc_waypoints(
                start,
                heading,
                DRIVE_RADIUS_M,
                180.0,
                left,
                crate::simulation::parking::validation::ProofClass::Freight.proof().bend_limit_deg,
            )
            .ok()?;
            let round = |v: f32| (v * 10_000.0).round() / 10_000.0;
            let anchor = &mut drive.anchors[bay];
            anchor.exit = Some("truck_exit".to_owned());
            anchor.route_out = std::iter::once(start)
                .chain(arc)
                .map(|p| [round(p.x), round(p.y)])
                .collect();
            if proven_docks(&drive).0 == 1 {
                return Some(drive);
            }
            turn += 0.5;
        }
    }
    None
}

// The registry with every loading-bay asset given its gate of `class`, with `through` a
// drive-through where one proves; the ids changed.
fn install_docks(core: &mut SimCore, class: &str, through: bool) -> Vec<String> {
    let mut changed = Vec::new();
    let mut entries = Vec::new();
    let mut ids: Vec<String> = core
        .allocator
        .registry
        .qualified_ids()
        .map(str::to_owned)
        .collect();
    ids.sort_unstable();
    for id in ids {
        let entry = core.allocator.registry.get(&id).unwrap();
        let pack = entry.pack_id.clone();
        let mut manifest = entry.manifest.clone();
        if manifest
            .anchors
            .iter()
            .any(|a| a.anchor_type == AnchorType::LoadingBay)
        {
            manifest = with_dock(&manifest, class, through)
                .or_else(|_| with_gate(&manifest, class))
                .unwrap_or_else(|reasons| panic!("{id}: no gate position proves a dock:{reasons}"));
            changed.push(id.clone());
        }
        entries.push((pack, manifest, entry.asset_dir.clone()));
    }
    core.replace_asset_registry(entries);
    changed
}

/// What one 14-day run delivered, and how.
#[derive(Debug, Default, PartialEq)]
struct YardRun {
    // Deliveries to a building with a dock: stood in a dock; at the kerb stop after the plan to
    // the gate (since `PARK-36` only when the building lost its docks while the truck waited);
    // at the kerb stop, planned there (the building had no dock when the carrier was planned).
    delivered_in_yard: u32,
    // Of those, at a drive-through dock (`PARK-21`): the truck left forward by the exit.
    delivered_in_drive_through: u32,
    delivered_at_door_from_gate: u32,
    delivered_at_door_planned: u32,
    // Deliveries to a building without a dock, and to the border.
    delivered_at_door: u32,
    delivered_at_border: u32,
    // Carriers that started standing in their source's dock.
    loaded_in_dock: u32,
    // Shipments that failed, expired, or vanished while in transit.
    failed: u32,
    expired: u32,
    vanished: u32,
    // Dock buildings, docks, and dock-frames held (summed over frames) out of dock-frames.
    dock_buildings: usize,
    docks: usize,
    // Dock buildings whose docks are drive-throughs, at the end.
    drive_through_buildings: usize,
    held_dock_frames: u64,
    dock_frames: u64,
    most_held: usize,
    // Trucks that found every dock held and waited at the kerb stop (`PARK-36`), the ones of
    // them later granted a dock, and their waits from the gate to the grant, in seconds.
    queued: u32,
    queued_granted: u32,
    queue_wait_max_s: f32,
    queue_wait_mean_s: f32,
    // Trip failure lines that name a yard.
    yard_failure_lines: Vec<String>,
    // Delivered shipments by destination dock building, by yard or door.
    by_building: BTreeMap<usize, (u32, u32)>,
}

// Runs the producer town with gates of `class` for `DAYS` days.
fn run(class: &str) -> YardRun {
    run_with(class, false)
}

// [`run`], with `through` drive-through docks where one proves.
fn run_with(class: &str, through: bool) -> YardRun {
    let mut scenario = GrowthScenario::producer_town();
    let changed = install_docks(&mut scenario.core, class, through);
    if through {
        let registry = &scenario.core.allocator.registry;
        let drives: Vec<_> = changed
            .iter()
            .filter(|id| {
                let manifest = &registry.get(id.as_str()).unwrap().manifest;
                manifest.anchors.iter().any(|a| a.exit.is_some())
            })
            .collect();
        eprintln!("{class}: drive-through assets {drives:?} of {changed:?}");
    }
    assert_eq!(changed.len(), 6, "{changed:?}");
    let core = &mut scenario.core;
    let mut result = YardRun::default();
    // Per shipment: its last status, and what its carrier was seen doing.
    // Per shipment: its last status; whether its carrier was seen standing in a dock, sent from a
    // gate to the door, pulling out of its source's dock, and bound for its destination's gate.
    let mut seen: BTreeMap<u64, (ShipmentStatus, bool, bool, bool, bool)> = BTreeMap::new();
    // Per shipment whose truck waits at a kerb stop for a dock: when it reached the gate.
    let mut queued_since: BTreeMap<u64, f32> = BTreeMap::new();
    let mut queue_wait_sum_s = 0.0_f64;
    // Per shipment: the distinct (phase, yard flag, transit, ready) states its carrier was seen
    // in while in transit, printed for a delivery to a dock building that did not stand in a dock.
    let mut history: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    let mut days = 0;
    while days < DAYS {
        if core.step_frame(FRAME_DT_S).daily_ticks > 0 {
            days += 1;
        }
        let handles = &core.allocator.building_handles;
        let mut live = BTreeMap::new();
        for shipment in &core.logistics.shipments {
            let entry =
                seen.entry(shipment.id)
                    .or_insert((shipment.status, false, false, false, false));
            if let Some(i) = core.agents.resolve_agent(shipment.carrier)
                && shipment.status == ShipmentStatus::InTransit
            {
                let trip = core.agents.parking_trip[i];
                let seen = history.entry(shipment.id).or_default();
                let state = format!(
                    "{:?}/{}/{}/{}",
                    trip.phase, trip.freight_gate, core.agents.transit[i], trip.ready
                );
                if seen.last() != Some(&state) {
                    seen.push(state);
                }
                if let ShipmentEndpoint::Building(b) = shipment.destination
                    && trip.freight_gate
                    && trip.phase != ParkingTripPhase::PullOut
                    && trip.target == handles.handle(b).to_bits()
                {
                    entry.4 = true;
                }
                if trip.freight_gate {
                    if matches!(trip.phase, ParkingTripPhase::ToKerb | ParkingTripPhase::AtKerb) {
                        queued_since.entry(shipment.id).or_insert_with(|| {
                            result.queued += 1;
                            core.agents.journey_start_time[i]
                        });
                    } else if trip.phase == ParkingTripPhase::PullIn
                        && let Some(since) = queued_since.remove(&shipment.id)
                    {
                        let wait = core.agents.sim_time - since;
                        result.queued_granted += 1;
                        result.queue_wait_max_s = result.queue_wait_max_s.max(wait);
                        queue_wait_sum_s += f64::from(wait);
                    }
                    match trip.phase {
                        // Pulling in counts: a frame of the fastest speed can hold the rest of
                        // the pull-in, the stand and the delivering check (`PARK-36`'s run).
                        ParkingTripPhase::PullIn | ParkingTripPhase::AtDock => entry.1 = true,
                        ParkingTripPhase::ToKerb | ParkingTripPhase::AtKerb => entry.2 = true,
                        ParkingTripPhase::PullOut
                            if Some(core.agents.current_building[i])
                                == match shipment.source {
                                    ShipmentEndpoint::Building(b) => Some(b),
                                    _ => None,
                                } =>
                        {
                            entry.3 = true
                        }
                        _ => {}
                    }
                }
            }
            if entry.0 == ShipmentStatus::InTransit && shipment.status != ShipmentStatus::InTransit
            {
                match shipment.status {
                    ShipmentStatus::Returning | ShipmentStatus::Fulfilled => {
                        match shipment.destination {
                            ShipmentEndpoint::Building(b) => {
                                let handle = handles.handle(b).to_bits();
                                let docked = core.parking.building_docks(handle).next().is_some();
                                if entry.1 {
                                    result.delivered_in_yard += 1;
                                    let forward = core
                                        .parking
                                        .building_docks(handle)
                                        .next()
                                        .is_some_and(|dock| core.parking.dock_leaves_forward(dock));
                                    result.delivered_in_drive_through += u32::from(forward);
                                } else if docked && (entry.2 || entry.4) {
                                    result.delivered_at_door_from_gate += 1;
                                    eprintln!(
                                        "delivered at a dock building outside its docks: shipment {} seen {:?}",
                                        shipment.id,
                                        history.get(&shipment.id)
                                    );
                                } else if docked {
                                    result.delivered_at_door_planned += 1;
                                } else {
                                    result.delivered_at_door += 1;
                                }
                                if docked {
                                    let row = result.by_building.entry(b).or_default();
                                    if entry.1 {
                                        row.0 += 1;
                                    } else {
                                        row.1 += 1;
                                    }
                                }
                            }
                            _ => result.delivered_at_border += 1,
                        }
                        if entry.3 {
                            result.loaded_in_dock += 1;
                        }
                    }
                    ShipmentStatus::Failed => result.failed += 1,
                    ShipmentStatus::Expired => result.expired += 1,
                    ShipmentStatus::InTransit => {}
                }
            }
            entry.0 = shipment.status;
            live.insert(shipment.id, ());
        }
        // A shipment gone from the list while it was still in transit.
        seen.retain(|id, entry| {
            let kept = live.contains_key(id);
            if !kept && entry.0 == ShipmentStatus::InTransit {
                result.vanished += 1;
            }
            kept
        });
        let buildings = core.parking.dock_buildings();
        let mut held = 0;
        let mut docks = 0;
        for building in &buildings {
            for dock in core.parking.building_docks(*building) {
                docks += 1;
                held += usize::from(
                    core.agents
                        .holds_freight_dock(core.parking.dock_holder(dock), dock),
                );
            }
        }
        result.dock_buildings = buildings.len();
        result.drive_through_buildings = buildings
            .iter()
            .filter(|&&b| {
                core.parking
                    .building_docks(b)
                    .next()
                    .is_some_and(|dock| core.parking.dock_leaves_forward(dock))
            })
            .count();
        result.docks = docks;
        result.held_dock_frames += held as u64;
        result.dock_frames += docks as u64;
        result.most_held = result.most_held.max(held);
    }
    result.queue_wait_mean_s = (queue_wait_sum_s / f64::from(result.queued_granted.max(1))) as f32;
    result.yard_failure_lines = core
        .agents
        .trip_failures
        .lines
        .iter()
        .filter(|line| line.contains("yard") || line.contains("loading bay"))
        .cloned()
        .collect();
    result
}

// The plan's A2 liveness gate (`PARK-20`): 14 days of the producer town with a dock on the coal
// mine, the farm, the power plant, both food industries and the grocery store, beside the same
// town with car gates. Deliveries stay alive, trucks use the docks, no shipment vanishes in
// transit and no trip failure names a yard; the failed and expired counts of both runs are
// printed for comparison. Manual: run with `--ignored --nocapture`.
#[test]
#[ignore = "manual 14-day freight yard liveness run (PARK-20); prints delivery counts"]
fn freight_yards_keep_deliveries_alive_for_14_days() {
    let yards = run("freight");
    let control = run("car");
    eprintln!("with docks: {yards:#?}");
    eprintln!("control, car gates: {control:#?}");
    assert!(yards.dock_buildings > 0 && yards.docks > 0, "{yards:#?}");
    assert_eq!(control.docks, 0);
    assert!(yards.delivered_in_yard > 0, "trucks use the docks");
    assert!(
        yards.yard_failure_lines.is_empty(),
        "{:#?}",
        yards.yard_failure_lines
    );
    assert_eq!(yards.vanished, 0);
    // No truck takes the door of a building with a dock (`PARK-36`).
    assert_eq!(yards.delivered_at_door_from_gate, 0, "{yards:#?}");
    let total = |r: &YardRun| {
        r.delivered_in_yard
            + r.delivered_at_door_from_gate
            + r.delivered_at_door_planned
            + r.delivered_at_door
            + r.delivered_at_border
    };
    eprintln!(
        "deliveries: with docks {}, control {}",
        total(&yards),
        total(&control)
    );
}

// `PARK-21`'s liveness run: the A2 run with every asset where a drive-through proves given one
// (the others keep their stage A dock), beside the same geometry with car gates. Deliveries stay
// alive, trucks drive through, no shipment vanishes and no trip failure names a yard. Manual:
// run with `--ignored --nocapture`.
#[test]
#[ignore = "manual 14-day drive-through freight yard liveness run (PARK-21); prints delivery counts"]
fn drive_through_yards_keep_deliveries_alive_for_14_days() {
    let yards = run_with("freight", true);
    let control = run_with("car", true);
    eprintln!("with drive-through docks: {yards:#?}");
    eprintln!("control, car gates: {control:#?}");
    assert!(yards.drive_through_buildings > 0, "{yards:#?}");
    assert!(yards.delivered_in_drive_through > 0, "trucks drive through");
    assert_eq!(control.docks, 0);
    assert!(
        yards.yard_failure_lines.is_empty(),
        "{:#?}",
        yards.yard_failure_lines
    );
    assert_eq!(yards.vanished, 0);
}
