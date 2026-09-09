// SPDX-License-Identifier: GPL-2.0-only

//! Planned stamp parity, neighboring ownership, exact dependency rejection and one-shot reuse.

use super::*;

fn assert_visual_terrain_equal(a: &TerrainSystem, b: &TerrainSystem) {
    assert_eq!(a.grid_dimensions(), b.grid_dimensions());
    let (width, height) = a.grid_dimensions();
    for z in 0..height {
        for x in 0..width {
            let (wx, wz) = a.grid_to_world_coords(x, z);
            assert_eq!(a.get_height(x, z), b.get_height(x, z), "source ({x}, {z})");
            assert_eq!(
                a.sample_visual_height_world(wx, wz),
                b.sample_visual_height_world(wx, wz),
                "visual ({x}, {z})"
            );
        }
    }
}

fn stamp_and_compare(planned: &mut Fixture, cold: &mut Fixture) -> usize {
    let source_generation = planned.terrain.source_generation();
    let chunks = planned
        .network
        .road_surface
        .rebuild_dirty_earthworks(&planned.graph, &mut planned.terrain);
    cold.network
        .road_surface
        .rebuild_dirty_earthworks(&cold.graph, &mut cold.terrain);
    assert_visual_terrain_equal(&planned.terrain, &cold.terrain);
    assert_eq!(planned.terrain.source_generation(), source_generation);
    assert!(!chunks.is_empty());
    assert_eq!(
        planned
            .network
            .road_surface
            .last_reused_earthwork_chunk_count,
        chunks.len()
    );
    chunks.len()
}

#[test]
fn planned_stamps_match_cold_crossing_close_t_and_terminal_extension() {
    for strokes in [
        vec![((-60.0, 0.0), (60.0, 0.0)), ((0.0, -48.0), (0.0, 48.0))],
        vec![
            ((-60.0, 0.0), (60.0, 0.0)),
            ((-16.0, 48.0), (-16.0, 0.0)),
            ((16.0, -48.0), (16.0, 0.0)),
        ],
        vec![((-60.0, 0.0), (0.0, 0.0)), ((0.0, 0.0), (60.0, 0.0))],
    ] {
        let mut planned = Fixture::new(true);
        let mut cold = Fixture::new(true);
        for ((sx, sz), (ex, ez)) in strokes {
            let before = planned.terrain.clone();
            let (prepared, plan, reuse) =
                planned.plan(&[planned.point(sx, sz), planned.point(ex, ez)]);
            assert!(plan.earthworks.is_some());
            assert_visual_terrain_equal(&before, &planned.terrain); // Planning is read-only.
            cold.cold_add(&prepared);
            planned.adopt(&plan, reuse);
            stamp_and_compare(&mut planned, &mut cold);
            // Publication consumes the offer; no stale products leak into the next rebuild.
            planned
                .network
                .road_surface
                .rebuild_dirty_earthworks(&planned.graph, &mut planned.terrain);
            assert_eq!(
                planned
                    .network
                    .road_surface
                    .last_reused_earthwork_chunk_count,
                0
            );
        }
    }
}

#[test]
fn planned_stamps_keep_unmapped_neighbors_in_shared_chunks() {
    let mut planned = Fixture::new(true);
    let mut cold = Fixture::new(true);
    for fixture in [&mut planned, &mut cold] {
        let (prepared, _, _) =
            fixture.plan(&[fixture.point(-60.0, 100.0), fixture.point(60.0, 100.0)]);
        fixture.cold_add(&prepared);
        fixture
            .network
            .road_surface
            .rebuild_dirty_earthworks(&fixture.graph, &mut fixture.terrain);
    }
    let neighbor_before = planned.terrain.sample_visual_height_world(0.0, 100.0);
    let (prepared, plan, reuse) =
        planned.plan(&[planned.point(-48.0, 0.0), planned.point(48.0, 0.0)]);
    assert!(
        !plan.source_topology_ids().0.contains(&0),
        "neighbor must be outside local graph"
    );
    cold.cold_add(&prepared);
    planned.adopt(&plan, reuse);
    stamp_and_compare(&mut planned, &mut cold);
    assert_eq!(
        planned.terrain.sample_visual_height_world(0.0, 100.0),
        neighbor_before
    );
}

#[test]
fn stamp_offer_rejects_source_changes_and_changed_support_with_same_ids() {
    for source_changed in [true, false] {
        let mut fixture = Fixture::new(false);
        let (_, plan, reuse) = fixture.plan(&[fixture.point(-48.0, 0.0), fixture.point(48.0, 0.0)]);
        fixture.adopt(&plan, reuse);
        if source_changed {
            fixture.terrain.set_height(2, 2, 1.0);
        } else {
            // Even if the outer caller errs and keeps a plan across a geometry change, the
            // stamper must compare the actual support inputs and use fresh results.
            let edge = fixture.graph.edge_mut(0);
            edge.class = EdgeClass::Tunnel; // Grounded roads have CDT-only support, not stamps.
            for point in &mut edge.physical_geometry {
                point.y += 0.5;
            }
            for point in &mut edge.geometry {
                point.y += 0.5;
            }
            let nodes = [edge.start_node, edge.end_node];
            for id in nodes {
                let mut pos = fixture.graph.node(id).pos;
                pos.y += 0.5;
                fixture.graph.set_node_pos(id, pos);
            }
            fixture
                .network
                .road_surface
                .mark_edge_dirty(&fixture.graph, 0);
        }
        let mut cold_surface = fixture.network.road_surface.clone();
        cold_surface.enqueue_planned_earthworks(None);
        let mut cold_terrain = fixture.terrain.clone();
        fixture
            .network
            .road_surface
            .rebuild_dirty_earthworks(&fixture.graph, &mut fixture.terrain);
        cold_surface.rebuild_dirty_earthworks(&fixture.graph, &mut cold_terrain);
        assert_eq!(
            fixture
                .network
                .road_surface
                .last_reused_earthwork_chunk_count,
            0
        );
        assert_visual_terrain_equal(&fixture.terrain, &cold_terrain);
    }
}

#[test]
fn planned_road_queries_match_committed_owners_and_keep_unmapped_neighbors() {
    use crate::simulation::network::surface::RoadSurfaceView;

    let mut fixture = Fixture::new(true);
    for z in [100.0, 0.0] {
        let (input, _, _) = fixture.plan(&[fixture.point(-60.0, z), fixture.point(60.0, z)]);
        fixture.cold_add(&input);
    }
    let (_, plan, reuse) = fixture.plan(&[fixture.point(0.0, -48.0), fixture.point(0.0, 48.0)]);
    assert!(!plan.source_topology_ids().0.contains(&0));
    // Test-only independent source copy: production borrows the already-pinned world context.
    let source_graph = fixture.graph.clone();
    let source_surface = fixture.network.road_surface.clone();
    let planned = plan
        .earthworks()
        .unwrap()
        .roads
        .view(&source_graph, &source_surface);
    fixture.adopt(&plan, reuse);
    let actual = RoadSurfaceView::new(&fixture.graph, &fixture.network.road_surface);
    let mut covered = 0;
    for z in (-56..=108).step_by(2) {
        for x in (-68..=68).step_by(2) {
            let (x, z) = (x as f32, z as f32);
            let expected = actual.sample_visible_height(&fixture.terrain, x, z);
            covered += usize::from(expected.is_some());
            assert_eq!(
                planned.sample_visible_height(&fixture.terrain, x, z),
                expected,
                "({x}, {z})"
            );
        }
    }
    assert!(covered > 100);
    // Both split/new and unaffected road candidates use the IDs the live query sees, exactly once.
    for pos in [Vector3::ZERO, Vector3::new(0.0, 0.0, 100.0)] {
        let candidates = |view: RoadSurfaceView<'_>| {
            let mut values = Vec::new();
            view.visit_edges_near_point(pos, 16.0, |graph, local, live| {
                let edge = graph.edge(local);
                if !edge.deleted {
                    values.push((live, edge.physical_geometry.clone()));
                }
            });
            values.sort_by_key(|(live, _)| *live);
            values
        };
        let expected = candidates(actual);
        assert!(!expected.is_empty());
        assert_eq!(candidates(planned), expected);
    }
}
