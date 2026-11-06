// SPDX-License-Identifier: GPL-2.0-only

//! Public anchor bays with fixed driveway–aisle–bay swept-footprint proofs.

use super::{
    Bay,
    geometry::{precise, rectangle, structure_collision},
    validation::{AnchorBay, driveway_points_inward, opens_onto_street, validate_bay},
};
use crate::assets::AnchorType;
use crate::assets::asset::validation::ZONE_CELL_M;
use crate::simulation::buildings::allocator::{BuildingAllocator, building_local_xz_basis};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::RoadbedSystem;
use godot::prelude::Vector2;

/// One place per valid parking anchor, at asset/edit boundaries only. For A bays, D driveways,
/// S support vertices and L local structure vertices: O(A² D + A D (S + L + K overlay cost)),
/// K only AABB-overlapping support/structure contours per tested footprint. Bay, sweep and
/// turn checks share this filter; disjoint support never runs an overlay. Unsupported sweeps
/// return diagnostics and publish no bay. No work depends on the city's total building count.
pub(crate) fn lot_bays(
    allocator: &BuildingAllocator,
    index: usize,
    graph: &RegionGraph,
    roads: &RoadbedSystem,
) -> (Vec<(u32, Bay)>, Vec<String>) {
    let building = &allocator.buildings[index];
    let Some(asset) = allocator.registry.get(&building.asset_id) else {
        return (Vec::new(), Vec::new());
    };
    let Some(site) = allocator.building_sites.get(index) else {
        return (Vec::new(), Vec::new());
    };
    let anchors = &asset.manifest.anchors;
    if !anchors.iter().any(|a| a.anchor_type == AnchorType::Parking) {
        return (Vec::new(), Vec::new());
    }
    let frontage = asset.manifest.building_frontage_forward();
    let local_outward = Vector2::new(frontage[0], frontage[2]).normalized();
    let (bx, bz) = building_local_xz_basis(building.facing_dir, frontage);
    let center = Vector2::new(building.center_x, building.center_y);
    let position = |p: [f32; 3]| center + bx * p[0] + bz * p[2];
    let direction = |p: [f32; 3]| (bx * p[0] + bz * p[2]).normalized();
    let bays: Vec<_> = anchors
        .iter()
        .enumerate()
        .filter(|(_, a)| a.anchor_type == AnchorType::Parking)
        .map(|(i, a)| {
            // The anchor marks the bay's entry edge and the bay runs along `forward`, as asset
            // validation and the editor draw it; bay geometry below is centred.
            let forward = direction(a.forward);
            let length = a.length_m.unwrap_or(5.0);
            (
                i as u32,
                position(a.position) + forward * (length * 0.5),
                forward,
                a.width_m.unwrap_or(2.5),
                length,
            )
        })
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
    let geometry: Vec<_> = bays
        .iter()
        .map(|&(_, pos, forward, width, length)| AnchorBay {
            pos,
            forward,
            width,
            length,
        })
        .collect();
    let mut result = Vec::new();
    let mut issues = Vec::new();
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
            .filter_map(|driveway| {
                if driveway.width_m.unwrap_or(3.0) < 1.8 {
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
                let approach = start + inward * 2.5;
                Some([connection, approach])
            })
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
            slot,
            &geometry,
            &footprints,
            &support,
            anchors
                .iter()
                .any(|a| a.anchor_type == AnchorType::Driveway),
            &paths,
            street,
            |foot| structure_collision(allocator, foot),
        );
        if let Ok(path) = checked {
            let swept_path = path.map(|p| [p.x, p.y]);
            result.push((
                anchor,
                Bay {
                    identity: [0, u64::from(anchor)],
                    position: [pos.x, site.support_height_m + 0.02, pos.y],
                    path: swept_path,
                    heading: (-forward.x).atan2(-forward.y),
                    width,
                    length,
                },
            ));
        } else {
            issues.push(fail(&checked.unwrap_err().reason));
        }
    }
    (result, issues)
}
