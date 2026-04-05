use super::*;
use crate::simulation::core::config::MapConfig;

/// Build a ZoningSystem with one edge grid of the given length (zone_cell_m = 10.0).
fn make_zoning(edge_idx: usize, length: f32) -> ZoningSystem {
    let cfg = MapConfig::default(); // zone_cell_m = 10.0
    let mut z = ZoningSystem::new(&cfg);
    z.update_edge_grid_size(edge_idx, length);
    z
}

#[test]
fn zone_range_full_zones_all_columns() {
    let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
    z.set_zone_range(0, 1, 0.0, 1.0, 4, ZoneType::Residential, &graph);
    for x in 0..10 {
        for y in 0..4 {
            assert_eq!(z.get_cell(0, 1, x, y), ZoneType::Residential, "col={x} row={y}");
        }
        assert_eq!(z.get_cell(0, 1, x, 4), ZoneType::None, "col={x} row=4 should be empty");
    }
}

#[test]
fn zone_range_partial_zones_correct_columns() {
    let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
    z.set_zone_range(0, 1, 0.3, 0.7, crate::config::DEFAULT_ZONING_DEPTH, ZoneType::Commercial, &graph);
    for x in 0..3 { assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::None); }
    for x in 3..7 { assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::Commercial); }
    for x in 7..10 { assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::None); }
}

#[test]
fn zone_range_sides_are_independent() {
    let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
    z.set_zone_range(0, 1, 0.0, 1.0, 1, ZoneType::Residential, &graph);
    z.set_zone_range(0, -1, 0.0, 1.0, 1, ZoneType::Commercial, &graph);
    assert_eq!(z.get_cell(0, 1, 0, 0), ZoneType::Residential);
    assert_eq!(z.get_cell(0, -1, 0, 0), ZoneType::Commercial);
}

#[test]
fn zone_range_reversed_t_still_zones_range() {
    let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
    z.set_zone_range(0, 1, 0.7, 0.3, 1, ZoneType::Industrial, &graph);
    for x in 3..7 { assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::Industrial); }
}

#[test]
fn zone_range_clear_with_none() {
    let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
    z.set_zone_range(0, 1, 0.0, 1.0, 1, ZoneType::Residential, &graph);
    z.set_zone_range(0, 1, 0.0, 0.5, 1, ZoneType::None, &graph);
    for x in 0..5 { assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::None); }
    for x in 5..10 { assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::Residential); }
}

#[test]
fn get_cell_out_of_bounds_returns_none() {
    let z = make_zoning(0, 100.0);
    assert_eq!(z.get_cell(0, 1, 10, 0), ZoneType::None);
    assert_eq!(z.get_cell(99, 1, 0, 0), ZoneType::None);
}

#[test]
fn is_occupied_out_of_bounds_returns_true() {
    let z = make_zoning(0, 100.0);
    assert!(z.is_occupied(0, 1, 100, 0));
    assert!(z.is_occupied(99, 1, 0, 0));
}

#[test]
fn split_produces_correct_column_counts() {
    let (_graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
    z.split_edge_grid(0, 1, 4);
    assert_eq!(z.edge_grids[&0].cells_long, 4);
    assert_eq!(z.edge_grids[&1].cells_long, 6);
}

#[test]
fn split_assigns_zone_data_to_correct_half() {
    let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
    z.set_zone_range(0, 1, 0.0, 0.4, 1, ZoneType::Residential, &graph);
    z.set_zone_range(0, 1, 0.4, 1.0, 1, ZoneType::Commercial, &graph);
    z.split_edge_grid(0, 1, 4);
    for x in 0..4 { assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::Residential); }
    for x in 0..6 { assert_eq!(z.get_cell(1, 1, x, 0), ZoneType::Commercial); }
}

#[test]
fn split_at_zero_produces_empty_old_full_new() {
    let (_graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
    z.split_edge_grid(0, 1, 0);
    assert_eq!(z.edge_grids[&0].cells_long, 0);
    assert_eq!(z.edge_grids[&1].cells_long, 10);
}

#[test]
fn split_at_end_produces_full_old_empty_new() {
    let (_graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
    z.split_edge_grid(0, 1, 10);
    assert_eq!(z.edge_grids[&0].cells_long, 10);
    assert_eq!(z.edge_grids[&1].cells_long, 0);
}

#[test]
fn split_copies_occupancy_to_new_half() {
    let (_graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
    z.set_occupied(0, 1, 7, 0, true);
    z.split_edge_grid(0, 1, 4);
    assert!(z.is_occupied(1, 1, 3, 0));
    assert!(!z.is_occupied(0, 1, 3, 0));
}

#[test]
fn merge_combines_column_counts_and_removes_second() {
    let (_graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(40.0, 0.0, 0.0)], 7.0);
    z.update_edge_grid_size(1, 60.0);
    z.merge_edge_grids(0, 1);
    assert_eq!(z.edge_grids[&0].cells_long, 10);
    assert!(!z.edge_grids.contains_key(&1));
}

#[test]
fn merge_preserves_zone_data_order() {
    let (_graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(40.0, 0.0, 0.0)], 7.0);
    z.update_edge_grid_size(1, 60.0);
    // Grow both sides to depth 1 so left_side is non-empty before filling.
    if let Some(g0) = z.edge_grids.get_mut(&0) { g0.grow_left_depth(1); for i in 0..g0.left_side.len() { g0.left_side[i] = ZoneType::Residential; } }
    if let Some(g1) = z.edge_grids.get_mut(&1) { g1.grow_left_depth(1); for i in 0..g1.left_side.len() { g1.left_side[i] = ZoneType::Commercial; } }
    z.merge_edge_grids(0, 1);
    for x in 0..4 { assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::Residential); }
    for x in 4..10 { assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::Commercial); }
}

// ── Step 1: dynamic depth tests ──────────────────────────────────────────────

#[test]
fn grow_depth_preserves_existing_cells() {
    // Paint a 3-column edge to depth 1 (Residential on all cols), then call
    // grow_left_depth(3) directly. Verify the y=0 row survives the reformat and
    // the new y=1, y=2 rows are None.
    let (graph, mut z) = make_edge_graph(
        vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(30.0, 0.0, 0.0)],
        7.0,
    );
    z.set_zone_range(0, 1, 0.0, 1.0, 1, ZoneType::Residential, &graph);
    assert_eq!(z.edge_grids[&0].left_depth, 1);

    z.edge_grids.get_mut(&0).unwrap().grow_left_depth(3);
    assert_eq!(z.edge_grids[&0].left_depth, 3);

    for x in 0..3 {
        assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::Residential, "col {x} y=0 should survive grow");
        assert_eq!(z.get_cell(0, 1, x, 1), ZoneType::None,        "col {x} y=1 new row should be None");
        assert_eq!(z.get_cell(0, 1, x, 2), ZoneType::None,        "col {x} y=2 new row should be None");
    }
}

#[test]
fn set_cell_autogrow_paints_correct_cell() {
    // Edge at depth 0. set_cell at y=2 must grow depth to 3, leave y=0..1 as None.
    let (graph, mut z) = make_edge_graph(
        vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)],
        7.0,
    );
    assert_eq!(z.edge_grids[&0].left_depth, 0);
    z.set_cell(0, 1, 3, 2, ZoneType::Residential, &graph);
    assert_eq!(z.edge_grids[&0].left_depth, 3, "depth should have grown to 3");
    assert_eq!(z.get_cell(0, 1, 3, 2), ZoneType::Residential);
    assert_eq!(z.get_cell(0, 1, 3, 0), ZoneType::None);
    assert_eq!(z.get_cell(0, 1, 3, 1), ZoneType::None);
}

#[test]
fn set_occupied_autogrow() {
    let (_graph, mut z) = make_edge_graph(
        vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)],
        7.0,
    );
    assert_eq!(z.edge_grids[&0].right_depth, 0);
    z.set_occupied(0, -1, 2, 3, true);
    assert_eq!(z.edge_grids[&0].right_depth, 4, "depth should have grown to 4");
    assert!(z.is_occupied(0, -1, 2, 3));
    assert!(!z.is_occupied(0, -1, 2, 0));
}

#[test]
fn split_with_asymmetric_depths_preserves_both_sides() {
    // Paint the left side to depth 3, leave right at depth 1.
    let (graph, mut z) = make_edge_graph(
        vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)],
        7.0,
    );
    z.set_zone_range(0, 1,  0.0, 1.0, 3, ZoneType::Residential, &graph);
    z.set_zone_range(0, -1, 0.0, 1.0, 1, ZoneType::Commercial,  &graph);
    assert_eq!(z.edge_grids[&0].left_depth,  3);
    assert_eq!(z.edge_grids[&0].right_depth, 1);

    z.split_edge_grid(0, 1, 4);

    // Depths propagate to both halves.
    assert_eq!(z.edge_grids[&0].left_depth,  3);
    assert_eq!(z.edge_grids[&0].right_depth, 1);
    assert_eq!(z.edge_grids[&1].left_depth,  3);
    assert_eq!(z.edge_grids[&1].right_depth, 1);

    // Left side data in both halves.
    for x in 0..4 { for y in 0..3 { assert_eq!(z.get_cell(0, 1,  x, y), ZoneType::Residential, "old x={x} y={y}"); } }
    for x in 0..6 { for y in 0..3 { assert_eq!(z.get_cell(1, 1,  x, y), ZoneType::Residential, "new x={x} y={y}"); } }
    // Right side data in both halves.
    for x in 0..4 { assert_eq!(z.get_cell(0, -1, x, 0), ZoneType::Commercial, "old right x={x}"); }
    for x in 0..6 { assert_eq!(z.get_cell(1, -1, x, 0), ZoneType::Commercial, "new right x={x}"); }
}

#[test]
fn merge_with_mismatched_depths_normalises_to_max() {
    // First grid: left_depth=3, right_depth=1.
    // Second grid: left_depth=1, right_depth=2.
    // After merge: left_depth=3, right_depth=2 in the combined grid.
    let (graph, mut z) = make_edge_graph(
        vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(40.0, 0.0, 0.0)],
        7.0,
    );
    z.update_edge_grid_size(1, 60.0);

    // Paint first grid.
    z.set_zone_range(0, 1,  0.0, 1.0, 3, ZoneType::Residential, &graph);
    z.set_zone_range(0, -1, 0.0, 1.0, 1, ZoneType::Commercial,  &graph);
    // Paint second grid — use set_zone_range via a temporary graph (same edge 1).
    // We have no graph for edge 1, so drive depth via set_cell directly.
    let (graph2, mut z2) = make_edge_graph(
        vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(60.0, 0.0, 0.0)],
        7.0,
    );
    z2.set_zone_range(0, 1,  0.0, 1.0, 1, ZoneType::Industrial, &graph2);
    z2.set_zone_range(0, -1, 0.0, 1.0, 2, ZoneType::Office,     &graph2);
    // Copy the painted grid into z under index 1.
    let painted = z2.edge_grids.remove(&0).unwrap();
    z.edge_grids.insert(1, painted);

    assert_eq!(z.edge_grids[&0].left_depth,  3);
    assert_eq!(z.edge_grids[&0].right_depth, 1);
    assert_eq!(z.edge_grids[&1].left_depth,  1);
    assert_eq!(z.edge_grids[&1].right_depth, 2);

    z.merge_edge_grids(0, 1);

    let merged = &z.edge_grids[&0];
    assert_eq!(merged.left_depth,  3, "merged left_depth should be max(3,1)=3");
    assert_eq!(merged.right_depth, 2, "merged right_depth should be max(1,2)=2");
    assert_eq!(merged.cells_long,  10, "4 + 6 columns");

    // First grid left side: all 3 depths painted Residential.
    for x in 0..4 { for y in 0..3 { assert_eq!(z.get_cell(0, 1,  x, y), ZoneType::Residential, "first left x={x} y={y}"); } }
    // Second grid left side: depth 1 painted Industrial, depths 1-2 should be None.
    for x in 4..10 { assert_eq!(z.get_cell(0, 1,  x, 0), ZoneType::Industrial, "second left x={x} y=0"); }
    for x in 4..10 { assert_eq!(z.get_cell(0, 1,  x, 1), ZoneType::None,       "second left x={x} y=1 expanded"); }
    // First grid right side: depth 1.
    for x in 0..4 { assert_eq!(z.get_cell(0, -1, x, 0), ZoneType::Commercial, "first right x={x}"); }
    // Second grid right side: depth 2 painted Office.
    for x in 4..10 { assert_eq!(z.get_cell(0, -1, x, 0), ZoneType::Office, "second right x={x} y=0"); }
    for x in 4..10 { assert_eq!(z.get_cell(0, -1, x, 1), ZoneType::Office, "second right x={x} y=1"); }
}

#[test]
fn zone_range_beyond_default_depth_is_accepted() {
    // Depth 20 is larger than DEFAULT_ZONING_DEPTH (12) — must work without capping.
    let (graph, mut z) = make_edge_graph(
        vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)],
        7.0,
    );
    z.set_zone_range(0, 1, 0.0, 1.0, 20, ZoneType::Residential, &graph);
    assert_eq!(z.edge_grids[&0].left_depth, 20);
    assert_eq!(z.get_cell(0, 1, 0, 19), ZoneType::Residential);
    assert_eq!(z.get_cell(0, 1, 5, 15), ZoneType::Residential);
}

fn make_edge_graph(pts: Vec<godot::prelude::Vector3>, width: f32) -> (crate::simulation::network::graph::RegionGraph, ZoningSystem) {
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(*pts.first().unwrap(), NodeType::Junction);
    let n1 = graph.add_node(*pts.last().unwrap(), NodeType::Junction);
    let mut length = 0.0f32;
    for i in 0..pts.len() - 1 { length += (pts[i+1] - pts[i]).length(); }
    graph.add_edge(Edge {
        start_node: n0, end_node: n1, primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR, width, fwd_lanes: 1, bkw_lanes: 1,
        speed_limit: 50.0, physical_length: length, geometry: pts.clone(),
        physical_geometry: pts, class: EdgeClass::Standard, deleted: false, ..Default::default()
    });
    graph.rebuild_adjacency_list();
    let mut zoning = ZoningSystem::new(&MapConfig::default());
    zoning.update_edge_grid_size(0, length);
    (graph, zoning)
}

fn arc_pts(radius: f32) -> Vec<godot::prelude::Vector3> {
    (0..=8).map(|k| {
        let theta = std::f32::consts::PI - (k as f32 / 8.0) * std::f32::consts::FRAC_PI_2;
        godot::prelude::Vector3::new(radius + radius * theta.cos(), 0.0, radius * theta.sin())
    }).collect()
}

#[test]
fn splay_y0_not_blocked_on_straight_road() {
    let pts = vec![godot::prelude::Vector3::new(0.0, 0.0, 0.0), godot::prelude::Vector3::new(100.0, 0.0, 0.0)];
    let (graph, zoning) = make_edge_graph(pts, 7.0);
    assert!(!zoning.is_cell_obstructed(0, -1, 1, 0, &graph, Some(&[])));
}

#[test]
fn splay_y0_blocked_on_tight_curve() {
    let (graph, zoning) = make_edge_graph(arc_pts(20.0), 7.0);
    assert!(zoning.is_cell_obstructed(0, -1, 1, 0, &graph, Some(&[])));
}

#[test]
fn splay_y1_not_blocked_on_tight_curve() {
    let (graph, zoning) = make_edge_graph(arc_pts(20.0), 7.0);
    assert!(!zoning.is_cell_obstructed(0, -1, 1, 1, &graph, Some(&[])));
}

fn make_t_junction_graph(angle_rad: f32, width: f32) -> (crate::simulation::network::graph::RegionGraph, ZoningSystem) {
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let n_left = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
    let n_right = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let dir = Vector3::new(angle_rad.cos(), 0.0, angle_rad.sin());
    let n_arm = graph.add_node(dir * 100.0, NodeType::Junction);
    let e0_pts = vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)];
    graph.add_edge(Edge {
        start_node: n_left, end_node: n_right, width, class: EdgeClass::Standard,
        primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
        physical_length: 200.0, geometry: e0_pts.clone(), physical_geometry: e0_pts,
        fwd_lanes: 1, bkw_lanes: 1, deleted: false, ..Default::default()
    });
    let e1_pts = vec![Vector3::ZERO, dir * 100.0];
    graph.add_edge(Edge {
        start_node: center, end_node: n_arm, width, class: EdgeClass::Standard,
        primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
        physical_length: 100.0, geometry: e1_pts.clone(), physical_geometry: e1_pts,
        fwd_lanes: 1, bkw_lanes: 1, deleted: false, ..Default::default()
    });
    graph.rebuild_adjacency_list();
    let mut zoning = ZoningSystem::new(&MapConfig::default());
    zoning.update_edge_grid_size(0, 200.0);
    zoning.update_edge_grid_size(1, 100.0);
    (graph, zoning)
}

#[test]
fn junction_zoning_90_deg_non_destructive() {
    let (graph, mut zoning) = make_t_junction_graph(std::f32::consts::FRAC_PI_2, 7.0);
    zoning.set_zone_range(0, -1, 0.0, 1.0, 3, ZoneType::Residential, &graph);
    let initial_count = zoning.edge_grids[&0].right_side.iter().filter(|&&z| z != ZoneType::None).count();
    zoning.set_zone_range(1, 1, 0.0, 1.0, 3, ZoneType::Industrial, &graph);
    let final_count = zoning.edge_grids[&0].right_side.iter().filter(|&&z| z != ZoneType::None).count();
    assert_eq!(initial_count, final_count);
    assert_eq!(zoning.get_cell(1, 1, 0, 0), ZoneType::None);
}

fn make_star_graph(angles: &[f32], length: f32, width: f32) -> (crate::simulation::network::graph::RegionGraph, ZoningSystem) {
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
    for &angle in angles {
        let dir = Vector3::new(angle.cos(), 0.0, angle.sin());
        let n_arm = graph.add_node(dir * length, NodeType::Junction);
        let pts = vec![Vector3::ZERO, dir * length];
        let edge_idx = graph.add_edge(Edge {
            start_node: center, end_node: n_arm, width, class: EdgeClass::Standard,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
            physical_length: length, geometry: pts.clone(), physical_geometry: pts,
            fwd_lanes: 1, bkw_lanes: 1, deleted: false, ..Default::default()
        });
        graph.add_to_spatial_index(edge_idx);
    }
    graph.rebuild_adjacency_list();
    let mut zoning = ZoningSystem::new(&MapConfig::default());
    for i in 0..angles.len() { zoning.update_edge_grid_size(i, length); }
    (graph, zoning)
}

#[test]
fn junction_zoning_8way() {
    let mut angles = Vec::new();
    for i in 0..8 { angles.push((i as f32) * 0.7854); }
    verify_junction_nongrowth(&angles, "8-way");
}

fn verify_junction_nongrowth(angles: &[f32], name: &str) {
    let (graph, mut zoning) = make_star_graph(angles, 100.0, 7.0);
    for i in 0..angles.len() {
        let mut before_counts = Vec::new();
        for j in 0..angles.len() {
            let l = zoning.edge_grids[&j].left_side.iter().filter(|&&z| z != ZoneType::None).count();
            let r = zoning.edge_grids[&j].right_side.iter().filter(|&&z| z != ZoneType::None).count();
            before_counts.push((l, r));
        }
        zoning.set_zone_range(i, 1, 0.0, 1.0, 3, ZoneType::Residential, &graph);
        zoning.set_zone_range(i, -1, 0.0, 1.0, 3, ZoneType::Residential, &graph);
        for j in 0..angles.len() {
            if i == j { continue; }
            let l = zoning.edge_grids[&j].left_side.iter().filter(|&&z| z != ZoneType::None).count();
            let r = zoning.edge_grids[&j].right_side.iter().filter(|&&z| z != ZoneType::None).count();
            assert_eq!(l, before_counts[j].0, "Sibling {} corruption in {}", j, name);
            assert_eq!(r, before_counts[j].1, "Sibling {} corruption in {}", j, name);
        }
    }
}

fn make_split_t_junction(clip: f32) -> (crate::simulation::network::graph::RegionGraph, ZoningSystem) {
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;
    let mut graph = RegionGraph::new();
    let n_left  = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
    let center  = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let n_right = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let n_north = graph.add_node(Vector3::new(0.0, 0.0, -100.0), NodeType::Junction);
    let make_edge = |sn, en, pts: Vec<Vector3>, sc, ec| Edge {
        start_node: sn, end_node: en, width: 7.0, primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR, class: EdgeClass::Standard, fwd_lanes: 1, bkw_lanes: 1,
        physical_length: 100.0, geometry: pts.clone(), physical_geometry: pts,
        start_clip: sc, end_clip: ec, deleted: false, ..Default::default()
    };
    graph.add_edge(make_edge(n_left, center, vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::ZERO], 0.0, clip));
    graph.add_edge(make_edge(center, n_right, vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)], clip, 0.0));
    graph.add_edge(make_edge(center, n_north, vec![Vector3::ZERO, Vector3::new(0.0, 0.0, -100.0)], clip, 0.0));
    graph.rebuild_adjacency_list();
    for idx in 0..3 { graph.add_to_spatial_index(idx); }
    let mut zoning = ZoningSystem::new(&MapConfig::default());
    for i in 0..3 { zoning.update_edge_grid_size(i, 100.0); }
    (graph, zoning)
}

#[test]
fn split_junction_first_zoned_wins_col_boundary() {
    let clip = 12.0_f32;
    let (graph, mut zoning) = make_split_t_junction(clip);
    zoning.set_zone_range(0, -1, 0.0, 1.0, 3, ZoneType::Residential, &graph);
    zoning.set_zone_range(2, -1, 0.0, 1.0, 3, ZoneType::Commercial, &graph);
    assert_eq!(zoning.get_cell(2, -1, 0, 0), ZoneType::None);
    assert_eq!(zoning.get_cell(2, -1, 5, 0), ZoneType::Commercial);
}

#[test]
fn test_zoning_grid_created_for_sidewalk_road() {
    for &(fwd, bkw, _) in crate::simulation::LANE_CONFIGS {
        let mut z = ZoningSystem::new(&MapConfig::default());
        z.update_edge_grid_size(0, 100.0);
        let _ = (fwd, bkw);
        assert!(z.edge_grids.contains_key(&0));
        assert_eq!(z.edge_grids[&0].cells_long, 10);
    }
}
