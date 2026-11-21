// SPDX-License-Identifier: GPL-3.0-or-later

//! Public anchor bays with fixed driveway–aisle–bay swept-footprint proofs.

use super::{
    Bay, DockRoute, DockWay,
    geometry::{precise, rectangle, structure_collision},
    validation::{
        AnchorBay, DockWayInput, ProofClass, dock_ways, driveway_points_inward, is_dock_class,
        is_freight_class, opens_onto_street, validate_bay, validate_dock,
    },
};
use crate::assets::{Anchor, AnchorType};
use crate::assets::asset::validation::ZONE_CELL_M;
use crate::simulation::buildings::allocator::{BuildingAllocator, building_local_xz_basis};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::network::types::TransitFlags;
use godot::prelude::Vector2;

/// A building's lot as its anchors prove it (`PARK-19`): the car bays, the docks and the
/// reasons each rejected anchor gives.
#[derive(Default)]
pub(crate) struct LotProof {
    /// One place per valid parking anchor, by anchor index.
    pub bays: Vec<(u32, Bay)>,
    /// One reason per rejected parking anchor, or the lot's one reason.
    pub issues: Vec<String>,
    /// One dock per valid loading bay, by anchor index, all through the building's one gate:
    /// the gate its first dock proves through.
    pub docks: Vec<(u32, Bay)>,
    /// One reason per rejected loading bay. Empty for an asset without a freight Driveway,
    /// whose loading bays are not validated at all.
    pub dock_issues: Vec<String>,
    /// The authored drive of each dock that has one (`PARK-21`), by anchor index.
    pub dock_ways: Vec<(u32, DockWay)>,
}

/// [`lot_proof`]'s car half, for the callers that read no dock. Same bound.
pub(crate) fn lot_bays(
    allocator: &BuildingAllocator,
    index: usize,
    graph: &RegionGraph,
    roads: &RoadbedSystem,
) -> (Vec<(u32, Bay)>, Vec<String>) {
    let proof = lot_proof(allocator, index, graph, roads);
    (proof.bays, proof.issues)
}

/// Whether `anchors` hold a freight Driveway and a loading bay a truck may stop at: the assets
/// whose lots `lot_proof` proves docks for. O(anchors).
pub(crate) fn has_freight_anchors(anchors: &[Anchor]) -> bool {
    anchors.iter().any(is_freight_driveway) && anchors.iter().any(is_dock_anchor)
}

/// How many loading bays of `anchors` a truck may stop at: the docks a building of the asset
/// could publish, as the inspector counts them. O(anchors).
pub(crate) fn dock_anchor_count(anchors: &[Anchor]) -> usize {
    anchors.iter().filter(|a| is_dock_anchor(a)).count()
}

// A Driveway that admits freight: a gate once it is wide enough (owner decision 11: freight
// never uses a `car` Driveway). O(1).
fn is_freight_driveway(anchor: &Anchor) -> bool {
    anchor.anchor_type == AnchorType::Driveway && is_freight_class(anchor.vehicle_class.as_deref())
}

// A loading bay a truck may stop at: its class is `freight` or absent. O(1).
fn is_dock_anchor(anchor: &Anchor) -> bool {
    anchor.anchor_type == AnchorType::LoadingBay && is_dock_class(anchor.vehicle_class.as_deref())
}

/// One place per valid parking anchor and one dock per valid loading bay, at asset/edit
/// boundaries only. For A bays and docks, D driveways, S support vertices and L local
/// structure vertices: O(A² D + A D (S + L + K overlay cost)), K only AABB-overlapping
/// support/structure contours per tested footprint. Bay, sweep and turn checks share this
/// filter; disjoint support never runs an overlay. Unsupported sweeps return diagnostics and
/// publish no bay. Loading bays are validated only when the asset has a freight Driveway, so
/// an asset without one costs what it did before docks. No work depends on the city's total
/// building count.
pub(crate) fn lot_proof(
    allocator: &BuildingAllocator,
    index: usize,
    graph: &RegionGraph,
    roads: &RoadbedSystem,
) -> LotProof {
    let building = &allocator.buildings[index];
    let Some(asset) = allocator.registry.get(&building.asset_id) else {
        return LotProof::default();
    };
    let Some(site) = allocator.building_sites.get(index) else {
        return LotProof::default();
    };
    let anchors = &asset.manifest.anchors;
    let has_parking = anchors.iter().any(|a| a.anchor_type == AnchorType::Parking);
    let has_freight = has_freight_anchors(anchors);
    if !has_parking && !has_freight {
        return LotProof::default();
    }
    // Cars reach every bay from the frontage edge; a walkway carries none. Placement refuses a
    // parking lot there; this covers any other building with bays on a walkway, a lot whose
    // street became one, and saves from before the refusal. Trucks are no different.
    if graph
        .get_edge(building.edge_idx)
        .is_some_and(|edge| edge.allowed_types & TransitFlags::CAR == 0)
    {
        let reason = "parking needs a road cars can drive on, not a walkway";
        let issue = vec![format!("{}: {reason}", building.asset_id)];
        return LotProof {
            issues: if has_parking { issue.clone() } else { Vec::new() },
            dock_issues: if has_freight { issue } else { Vec::new() },
            ..LotProof::default()
        };
    }
    let frontage = asset.manifest.building_frontage_forward();
    let local_outward = Vector2::new(frontage[0], frontage[2]).normalized();
    let (bx, bz) = building_local_xz_basis(building.facing_dir, frontage);
    let center = Vector2::new(building.center_x, building.center_y);
    let position = |p: [f32; 3]| center + bx * p[0] + bz * p[2];
    let direction = |p: [f32; 3]| (bx * p[0] + bz * p[2]).normalized();
    // The anchor marks the bay's entry edge and the bay runs along `forward`, as asset
    // validation and the editor draw it; bay geometry below is centred.
    let centred = |(i, a): (usize, &Anchor)| {
        let forward = direction(a.forward);
        let length = a.length_m.unwrap_or(5.0);
        (
            i as u32,
            position(a.position) + forward * (length * 0.5),
            forward,
            a.width_m.unwrap_or(2.5),
            length,
        )
    };
    let bays: Vec<_> = anchors
        .iter()
        .enumerate()
        .filter(|(_, a)| a.anchor_type == AnchorType::Parking)
        .map(centred)
        .collect();
    let footprints: Vec<_> = bays
        .iter()
        .map(|(_, p, f, w, l)| rectangle(*p, *f, *w, *l))
        .collect();
    let mut support: Vec<_> = site
        .surfaces
        .iter()
        .map(|s| precise(&s.vertices_world))
        .collect();
    // Driveway access may cross its frontage's sidewalk/carriageway, never another building.
    if let Some(span) = roads.span_piece(building.edge_idx) {
        let mesh = span.mesh();
        support.extend(mesh.triangles().iter().map(|tri| {
            tri.iter()
                .map(|&i| {
                    let p = mesh.positions()[i as usize];
                    [p.x, p.z]
                })
                .collect()
        }));
    }
    let geometry = |list: &[(u32, Vector2, Vector2, f32, f32)]| -> Vec<_> {
        list.iter()
            .map(|&(_, pos, forward, width, length)| AnchorBay {
                pos,
                forward,
                width,
                length,
            })
            .collect()
    };
    // A Driveway's road connection and its approach one lead inside it, when it is at least
    // the class's gate width and points into the lot from a live frontage.
    let driveway_path = |driveway: &Anchor, class: ProofClass| {
        let row = class.proof();
        if driveway.width_m.unwrap_or(3.0) < row.min_gate {
            return None;
        }
        let start = position(driveway.position);
        let inward = direction(driveway.forward);
        if !driveway_points_inward(inward, building.facing_dir) {
            return None;
        }
        let edge = graph.get_edge(building.edge_idx).filter(|e| !e.deleted)?;
        let station =
            BuildingAllocator::project_point_to_polyline_s(&edge.physical_geometry, start);
        let connection = BuildingAllocator::claimed_road_side_connection_pos(
            graph,
            building.edge_idx,
            building.side,
            station / edge.physical_length,
        )?;
        // Placement already validates frontage support; repeat its exact connection proof.
        roads.height_at(connection.x, connection.y)?;
        let approach = start + inward * row.lead;
        Some([connection, approach])
    };
    let place = |anchor: u32, path: [Vector2; 4], pos: Vector2, forward: Vector2, width, length| {
        (
            anchor,
            Bay {
                identity: [0, u64::from(anchor)],
                position: [pos.x, site.support_height_m + 0.02, pos.y],
                path: path.map(|p| [p.x, p.y]),
                heading: (-forward.x).atan2(-forward.y),
                width,
                length,
            },
        )
    };
    let mut proof = LotProof::default();
    let car_geometry = geometry(&bays);
    for (slot, &(anchor, pos, forward, width, length)) in bays.iter().enumerate() {
        let fail = |reason: &str| {
            format!(
                "{} parking anchor {}: {}",
                building.asset_id, anchor, reason
            )
        };
        let paths: Vec<_> = anchors
            .iter()
            .filter(|a| a.anchor_type == AnchorType::Driveway)
            .filter_map(|driveway| driveway_path(driveway, ProofClass::Car))
            .collect();
        // A bay whose entry edge is the lot's street edge is reached straight from the road.
        let local = &anchors[anchor as usize];
        let local_forward = Vector2::new(local.forward[0], local.forward[2]).normalized();
        let local_entry = Vector2::new(local.position[0], local.position[2]);
        let half_extents = Vector2::new(
            f32::from(building.width_cells),
            f32::from(building.depth_cells),
        ) * (ZONE_CELL_M * 0.5);
        let street = opens_onto_street(local_entry, local_forward, local_outward, half_extents)
            .then(|| {
                let entry = pos - forward * (length * 0.5);
                let edge = graph.get_edge(building.edge_idx).filter(|e| !e.deleted)?;
                let station =
                    BuildingAllocator::project_point_to_polyline_s(&edge.physical_geometry, entry);
                let connection = BuildingAllocator::claimed_road_side_connection_pos(
                    graph,
                    building.edge_idx,
                    building.side,
                    station / edge.physical_length,
                )?;
                roads.height_at(connection.x, connection.y)?;
                Some(connection)
            })
            .flatten();
        let checked = validate_bay(
            ProofClass::Car,
            slot,
            &car_geometry,
            &footprints,
            &[],
            &support,
            anchors
                .iter()
                .any(|a| a.anchor_type == AnchorType::Driveway),
            &paths,
            street,
            |foot| structure_collision(allocator, ProofClass::Car, foot),
        );
        match checked {
            Ok((path, _)) => proof.bays.push(place(anchor, path, pos, forward, width, length)),
            Err(rejection) => proof.issues.push(fail(&rejection.reason)),
        }
    }
    if has_freight {
        let docks: Vec<_> = anchors
            .iter()
            .enumerate()
            .filter(|(_, a)| is_dock_anchor(a))
            .map(centred)
            .collect();
        let dock_geometry = geometry(&docks);
        let dock_footprints: Vec<_> = docks
            .iter()
            .map(|(_, p, f, w, l)| rectangle(*p, *f, *w, *l))
            .collect();
        let freight = ProofClass::Freight;
        // The usable gates with their names, for the exits that name them (`PARK-21`).
        let (names, gates): (Vec<_>, Vec<_>) = anchors
            .iter()
            .filter(|a| is_freight_driveway(a))
            .filter_map(|gate| Some((gate.name.trim(), driveway_path(gate, freight)?)))
            .unzip();
        let has_gate = anchors.iter().any(|a| {
            is_freight_driveway(a) && a.width_m.unwrap_or(3.0) >= freight.proof().min_gate
        });
        let lot_point = |[x, z]: [f32; 2]| position([x, 0.0, z]);
        let ways: Vec<_> = docks
            .iter()
            .map(|&(anchor, ..)| {
                let anchor = &anchors[anchor as usize];
                DockWayInput {
                    route_in: anchor.route_in.iter().copied().map(lot_point).collect(),
                    route_out: anchor.route_out.iter().copied().map(lot_point).collect(),
                    exit: anchor.exit.as_deref().map(|exit| {
                        let exit = exit.trim();
                        names.iter().position(|&name| name == exit).ok_or_else(|| exit.to_owned())
                    }),
                }
            })
            .collect();
        // A dock never opens onto the street, and its routes also keep off the parking spots.
        let prove = |slot: usize, only: Option<usize>| {
            validate_dock(
                slot,
                &dock_geometry,
                &dock_footprints,
                &footprints,
                &support,
                has_gate,
                &gates,
                only,
                &ways[slot],
                |foot| structure_collision(allocator, freight, foot),
            )
        };
        let routes = dock_ways(docks.len(), prove);
        for (&(anchor, pos, forward, width, length), route) in docks.iter().zip(routes) {
            match route {
                Ok(dock) => {
                    if !dock.route_in.is_empty() {
                        let points = |route: &[Vector2]| {
                            let points: Vec<_> = route.iter().map(|p| [p.x, p.y]).collect();
                            DockRoute::new(&points).expect("a proven way fits its row")
                        };
                        let way = DockWay {
                            route_in: points(&dock.route_in),
                            route_out: points(&dock.route_out),
                        };
                        proof.dock_ways.push((anchor, way));
                    }
                    proof.docks.push(place(anchor, dock.path, pos, forward, width, length));
                }
                Err(rejection) => proof.dock_issues.push(format!(
                    "{} loading bay {}: {}",
                    building.asset_id, anchor, rejection.reason
                )),
            }
        }
    }
    proof
}
