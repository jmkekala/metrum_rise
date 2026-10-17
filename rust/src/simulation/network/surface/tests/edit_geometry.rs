// SPDX-License-Identifier: GPL-2.0-only

//! Road placement refuses the illegal geometry a stroke adds (`ROAD-54`), and only that.

use super::*;

// Existing roads radiating from the origin at the given angles, each 30 m long.
fn star(angles_degrees: &[f32]) -> (RegionGraph, RoadSurfaceSystem, TerrainSystem) {
    let terrain = flat_terrain(128, 128);
    let mut graph = RegionGraph::new();
    let centre = graph.add_node(Vector3::ZERO, NodeType::Junction);
    for &angle in angles_degrees {
        let a = angle.to_radians();
        let end = Vector3::new(30.0 * a.cos(), 0.0, 30.0 * a.sin());
        let tip = graph.add_node(end, NodeType::Junction);
        graph.add_edge(crate::simulation::network::build_surface_edge(
            centre,
            tip,
            vec![Vector3::ZERO, end],
            1,
            1,
            EdgeClass::Standard,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    (graph, surface, terrain)
}

fn preview(
    points: &[Vector3],
    (graph, surface, terrain): &(RegionGraph, RoadSurfaceSystem, TerrainSystem),
) -> PreviewRoadSurfaceResult {
    surface
        .compile_preview_surface_mesh_only_with_existing_surface_snap_and_topology_reuse(
            points, 1, 1, terrain, graph, surface, true,
        )
        .0
}

fn arm_at(angle_degrees: f32) -> Vec<Vector3> {
    let a = angle_degrees.to_radians();
    vec![
        Vector3::ZERO,
        Vector3::new(30.0 * a.cos(), 0.0, 30.0 * a.sin()),
    ]
}

#[test]
fn a_road_joining_a_junction_too_close_to_another_arm_is_refused() {
    let world = star(&[0.0, 180.0]);
    let sharp = preview(&arm_at(15.0), &world);
    assert_eq!(sharp.validation.invalid_reason, "junction_arm_angle");
    assert!(
        (sharp.validation.geometry_measured - 15.0).abs() < 1.0,
        "{:?}",
        sharp.validation
    );
    assert_eq!(sharp.validation.geometry_limit, 25.0);
    assert!(preview(&arm_at(90.0), &world).is_valid);
}

#[test]
fn a_seventh_road_at_one_junction_is_refused() {
    let world = star(&[0.0, 60.0, 120.0, 180.0, 240.0, 300.0]);
    let seventh = preview(&arm_at(30.0), &world);
    // Thirty degrees from two arms is legal; seven arms is not.
    assert_eq!(seventh.validation.invalid_reason, "junction_degree");
    assert_eq!(seventh.validation.geometry_measured, 7.0);
}

#[test]
fn a_junction_already_too_sharp_still_takes_a_road_that_leaves_it_no_worse() {
    let world = star(&[0.0, 20.0, 180.0]);
    assert!(preview(&arm_at(270.0), &world).is_valid);
    assert_eq!(
        preview(&arm_at(10.0), &world).validation.invalid_reason,
        "junction_arm_angle"
    );
}

#[test]
fn a_curve_tighter_than_the_road_is_wide_is_refused() {
    let world = star(&[]);
    // A hairpin of 3 m radius on a 7 m road.
    let hairpin: Vec<_> = (0..=18)
        .map(|i| {
            let a = (i as f32 * 10.0).to_radians();
            Vector3::new(20.0 + 3.0 * a.sin(), 0.0, 3.0 - 3.0 * a.cos())
        })
        .chain([Vector3::new(0.0, 0.0, 6.0)])
        .collect();
    let mut points = vec![Vector3::new(0.0, 0.0, 0.0)];
    points.extend(hairpin);
    let refused = preview(&points, &world);
    assert_eq!(refused.validation.invalid_reason, "road_tight_curve");
    // A gentle 40 m curve is fine.
    let gentle: Vec<_> = (0..=9)
        .map(|i| {
            let a = (i as f32 * 5.0).to_radians();
            Vector3::new(40.0 * a.sin(), 0.0, 40.0 - 40.0 * a.cos())
        })
        .collect();
    assert!(preview(&gentle, &world).is_valid);
}
