// SPDX-License-Identifier: GPL-2.0-only

//! Endpoint deformation that preserves both independently sampled road profiles.

use super::{Edge, RegionGraph};
use godot::prelude::Vector3;

/// Moves one endpoint with a smooth displacement over horizontal road distance.
/// Mismatched profiles first share their original support stations in O(C + P) time.
pub(super) fn deform_edge(edge: &mut Edge, at_start: bool, delta: Vector3) {
    if edge.geometry.len() < 2 {
        return;
    }
    align_profile_supports(edge);
    let length = stations(&edge.geometry).last().unwrap().0;
    let mut distance = 0.0;
    let mut previous = edge.geometry[0];
    let last_index = edge.geometry.len() - 1;
    for (index, (control, physical)) in edge
        .geometry
        .iter_mut()
        .zip(&mut edge.physical_geometry)
        .enumerate()
    {
        distance += RegionGraph::edge_profile_point_distance_m(previous, *control);
        previous = *control;
        let fraction = if length > 0.0 {
            distance / length
        } else {
            index as f32 / last_index as f32
        };
        let weight = if at_start { 1.0 - fraction } else { fraction };
        let displacement = delta * (weight * weight * (3.0 - 2.0 * weight));
        *control += displacement;
        *physical += displacement;
    }
}

fn stations(points: &[Vector3]) -> impl Iterator<Item = (f32, Vector3)> + '_ {
    let mut distance = 0.0;
    points.iter().enumerate().map(move |(index, &point)| {
        if index > 0 {
            distance += RegionGraph::edge_profile_point_distance_m(points[index - 1], point);
        }
        (distance, point)
    })
}

fn profile_height(previous: (f32, Vector3), next: Option<(f32, Vector3)>, distance: f32) -> f32 {
    let Some((end_distance, end)) = next else {
        return previous.1.y;
    };
    let run = end_distance - previous.0;
    let fraction = if run > 0.0 {
        ((distance - previous.0) / run).clamp(0.0, 1.0)
    } else {
        0.0
    };
    previous.1.y + (end.y - previous.1.y) * fraction
}

fn align_profile_supports(edge: &mut Edge) {
    if edge.geometry.len() == edge.physical_geometry.len()
        && edge
            .geometry
            .iter()
            .zip(&edge.physical_geometry)
            .all(|(control, physical)| control.x == physical.x && control.z == physical.z)
    {
        return;
    }

    let capacity = edge.geometry.len() + edge.physical_geometry.len();
    let mut controls = Vec::with_capacity(capacity);
    let mut physicals = Vec::with_capacity(capacity);
    {
        let mut control_stations = stations(&edge.geometry).peekable();
        let mut physical_stations = stations(&edge.physical_geometry).peekable();
        let mut previous_control = (0.0, edge.geometry[0]);
        let mut previous_physical = (0.0, edge.physical_geometry[0]);
        loop {
            let control = control_stations.peek().copied();
            let physical = physical_stations.peek().copied();
            let (distance, point, take_control, take_physical) = match (control, physical) {
                (Some((distance, point)), Some((_, other)))
                    if point.x == other.x && point.z == other.z =>
                {
                    (distance, point, true, true)
                }
                (Some((control_distance, _)), Some((distance, point)))
                    if distance < control_distance =>
                {
                    (distance, point, false, true)
                }
                (Some((distance, point)), _) => (distance, point, true, false),
                (None, Some((distance, point))) => (distance, point, false, true),
                (None, None) => break,
            };
            let control_y = if take_control {
                previous_control = control_stations.next().unwrap();
                previous_control.1.y
            } else {
                profile_height(previous_control, control, distance)
            };
            let physical_y = if take_physical {
                previous_physical = physical_stations.next().unwrap();
                previous_physical.1.y
            } else {
                profile_height(previous_physical, physical, distance)
            };
            controls.push(Vector3::new(point.x, control_y, point.z));
            physicals.push(Vector3::new(point.x, physical_y, point.z));
        }
    }
    controls.shrink_to_fit();
    physicals.shrink_to_fit();
    edge.geometry = controls;
    edge.physical_geometry = physicals;
}
