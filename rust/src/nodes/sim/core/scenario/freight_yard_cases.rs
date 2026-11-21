// SPDX-License-Identifier: GPL-3.0-or-later

//! The freight yard liveness run (`PARK-20`): the producer town with a freight gate and a dock
//! on every fixture asset that carries a loading bay, run for 14 days beside the same town whose
//! gates are car Driveways (no dock), counting deliveries by yard and by door.
//!
//! The fixture packs publish no dock (`PARK-19`, owner decision 11), so the run builds its own
//! variants in memory: each of the six assets with a `loading_bay` gets its bay turned to face
//! into the lot and a 4.5 m Driveway in front of it, at the first place across the lot where the
//! asset editor's dock proof passes. Nothing is written to a pack.

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

// The registry with every loading-bay asset given its gate of `class`; the ids changed.
fn install_gates(core: &mut SimCore, class: &str) -> Vec<String> {
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
            manifest = with_gate(&manifest, class)
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
    // Deliveries to a building with a dock: stood in a dock; at the door after the plan to the
    // gate (the fallback: every dock held at the gate commit); at the door, planned there (the
    // building had no dock when the carrier was planned).
    delivered_in_yard: u32,
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
    held_dock_frames: u64,
    dock_frames: u64,
    most_held: usize,
    // Trip failure lines that name a yard.
    yard_failure_lines: Vec<String>,
    // Delivered shipments by destination dock building, by yard or door.
    by_building: BTreeMap<usize, (u32, u32)>,
}

// Runs the producer town with gates of `class` for `DAYS` days.
fn run(class: &str) -> YardRun {
    let mut scenario = GrowthScenario::producer_town();
    let changed = install_gates(&mut scenario.core, class);
    assert_eq!(changed.len(), 6, "{changed:?}");
    let core = &mut scenario.core;
    let mut result = YardRun::default();
    // Per shipment: its last status, and what its carrier was seen doing.
    // Per shipment: its last status; whether its carrier was seen standing in a dock, sent from a
    // gate to the door, pulling out of its source's dock, and bound for its destination's gate.
    let mut seen: BTreeMap<u64, (ShipmentStatus, bool, bool, bool, bool)> = BTreeMap::new();
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
                if let ShipmentEndpoint::Building(b) = shipment.destination
                    && trip.freight_gate
                    && trip.phase != ParkingTripPhase::PullOut
                    && trip.target == handles.handle(b).to_bits()
                {
                    entry.4 = true;
                }
                if trip.freight_gate {
                    match trip.phase {
                        ParkingTripPhase::AtDock => entry.1 = true,
                        ParkingTripPhase::GateToDoor => entry.2 = true,
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
                                } else if docked && (entry.2 || entry.4) {
                                    result.delivered_at_door_from_gate += 1;
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
        result.docks = docks;
        result.held_dock_frames += held as u64;
        result.dock_frames += docks as u64;
        result.most_held = result.most_held.max(held);
    }
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
