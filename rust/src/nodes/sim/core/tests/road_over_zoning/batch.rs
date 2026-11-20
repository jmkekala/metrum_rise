// SPDX-License-Identifier: GPL-3.0-or-later

//! Batch commits (`ROAD-112` round F, plan 11 F): a batch of strokes leaves the graph, roadbed,
//! cells and parcels bitwise as the same strokes committed one by one; one undo takes the whole
//! batch back; a refused stroke anywhere changes nothing and demolishes nothing; the undo
//! history's cap counts groups; undo refunds what an edit charged (decision U5).

use super::*;
use crate::debug::work::debug_hash;
use crate::nodes::sim::core::RoadBatchRequest;
use crate::simulation::network::graph::RoadParking;
use crate::simulation::network::road_types::RoadAttributes;

// A draft from the south into the street's middle, as consecutive segments, and a cross street
// joining it at the segments' joint: the second segment and the cross street take houses.
pub(super) const DRAFT: [[Vector3; 2]; 3] = [
    [Vector3::new(0.0, 0.0, -70.0), Vector3::new(0.0, 0.0, -35.0)],
    [Vector3::new(0.0, 0.0, -35.0), Vector3::new(0.0, 0.0, 0.0)],
    [
        Vector3::new(-50.0, 0.0, -35.0),
        Vector3::new(0.0, 0.0, -35.0),
    ],
];

pub(super) fn stroke(points: &[Vector3]) -> RoadCommitRequest {
    RoadCommitRequest {
        points: points.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
        median: None,
        parking: RoadParking::NONE,
        attributes: RoadAttributes::DEFAULT,
        preview: None,
    }
}

// Commits `strokes` as one batch; the refusing stroke's index and reason, `None` when committed.
fn commit_batch(core: &mut SimCore, strokes: &[[Vector3; 2]]) -> Option<(usize, String)> {
    core.precompute_road_mesh_data();
    let batch = core.commit_road_batch(
        RoadBatchRequest {
            strokes: strokes.iter().map(|points| stroke(points)).collect(),
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert_eq!(batch.outcome.committed, batch.refused_stroke.is_none());
    batch
        .refused_stroke
        .map(|index| (index, batch.outcome.rejection_reason))
}

// Every edge's and node's published piece.
fn roadbed_digest(core: &SimCore) -> u64 {
    use crate::simulation::network::roadbed::PieceId;
    let roadbed = &core.transit_network.roadbed;
    let spans: Vec<_> = (0..core.region_graph.edge_count())
        .map(|edge| roadbed.piece_mesh(PieceId::Span(edge)))
        .collect();
    let nodes: Vec<_> = (0..core.region_graph.node_count() as u32)
        .map(|node| roadbed.piece_mesh(PieceId::Node(node)))
        .collect();
    debug_hash(&(spans, nodes))
}

// The products a batch must share bitwise with its strokes committed one by one: the live graph
// (ends, widths, geometry and profiles), the roadbed's pieces, the cells (paint, claims and lots),
// parcels with occupants, buildings, agents and households (`land_state`, without the undo
// history's length), and the treasury.
pub(super) fn products(core: &SimCore) -> Vec<u64> {
    let edges: Vec<_> = core
        .region_graph
        .edges()
        .iter()
        .map(|edge| {
            let points: Vec<_> = edge
                .physical_geometry
                .iter()
                .map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()])
                .collect();
            (
                edge.deleted,
                edge.start_node,
                edge.end_node,
                edge.width.to_bits(),
                points,
            )
        })
        .collect();
    let mut state = land_state(core);
    state.pop();
    state.extend([
        debug_hash(&edges),
        roadbed_digest(core),
        core.zoning.cells.state_digest(),
        core.treasury.balance.to_bits(),
        core.treasury.lifetime_build_cost.to_bits(),
    ]);
    state
}

// Test 1: the batch equals its strokes committed one by one, in order, bitwise; its records
// share one undo group and the one-by-one strokes' records one group each.
#[test]
fn a_batch_equals_its_strokes_committed_one_by_one() {
    let (mut single, _) = street_with_houses();
    let depth = single.undo_stack.len();
    let buildings = single.allocator.buildings.len();
    for points in &DRAFT {
        assert_eq!(commit(&mut single, points), "");
        single.adopt_pending_cell_lots();
    }
    let (mut batch, _) = street_with_houses();
    batch.precompute_road_mesh_data();
    let outcome = batch.commit_road_batch(
        RoadBatchRequest {
            strokes: DRAFT.iter().map(|points| stroke(points)).collect(),
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.outcome.committed);
    // Every stroke installed its dry run's plan and pieces: the batch's chain held.
    assert_eq!(outcome.times.reused, DRAFT.len());
    batch.adopt_pending_cell_lots();
    assert!(
        batch.allocator.buildings.len() < buildings,
        "the batch takes houses"
    );
    assert_eq!(products(&batch), products(&single));
    assert_references_consistent(&batch);
    let groups = |core: &SimCore| {
        core.undo_stack
            .iter()
            .skip(depth)
            .map(|record| record.group)
            .collect::<Vec<_>>()
    };
    let batch_groups = groups(&batch);
    assert_eq!(batch_groups.len(), DRAFT.len(), "one record a stroke");
    assert!(batch_groups.iter().all(|&group| group == batch_groups[0]));
    let single_groups = groups(&single);
    assert!(single_groups.windows(2).all(|pair| pair[0] != pair[1]));
}

// Round F2: the cores installing their dry run's plan, pieces and plotted verdict leave the same
// products bitwise as cores that plan, compile and judge every stroke afresh.
#[test]
fn a_batch_reusing_its_dry_run_equals_one_planned_afresh() {
    let request = || RoadBatchRequest {
        strokes: DRAFT.iter().map(|points| stroke(points)).collect(),
        preview: None,
    };
    let (mut fresh, _) = street_with_houses();
    fresh.precompute_road_mesh_data();
    let outcome = fresh.commit_road_batch_without_reuse(request());
    assert!(outcome.outcome.committed);
    assert_eq!(outcome.times.reused, 0, "every core planned afresh");
    fresh.adopt_pending_cell_lots();
    let (mut reused, _) = street_with_houses();
    reused.precompute_road_mesh_data();
    let outcome =
        reused.commit_road_batch(request(), std::time::Instant::now(), Default::default());
    assert!(outcome.outcome.committed);
    assert_eq!(outcome.times.reused, DRAFT.len());
    reused.adopt_pending_cell_lots();
    assert_eq!(products(&reused), products(&fresh));
    assert_references_consistent(&reused);
}

// Test 2: one undo takes the whole batch back: the graph, roadbed, paint and the treasury (the
// build cost refunded, U5) are as before it; the demolished houses stay demolished.
#[test]
fn one_undo_restores_the_pre_batch_state() {
    let (mut core, _) = street_with_houses();
    let depth = core.undo_stack.len();
    let painted_before = painted(&core);
    let edges_before = live_edges(&core);
    let pieces_before = roadbed_digest(&core);
    let balance = core.treasury.balance;
    let lifetime = core.treasury.lifetime_build_cost;
    assert_eq!(commit_batch(&mut core, &DRAFT), None);
    core.adopt_pending_cell_lots();
    assert!(core.treasury.balance < balance, "the batch is charged");
    let charged: f64 = core
        .undo_stack
        .iter()
        .skip(depth)
        .map(|record| record.charged)
        .sum();
    assert!((balance - core.treasury.balance - charged).abs() <= 1e-6);
    assert!(core.undo_action_internal());
    assert_eq!(core.undo_stack.len(), depth, "one undo takes every stroke");
    assert_eq!(live_edges(&core), edges_before);
    assert_eq!(painted(&core), painted_before);
    core.precompute_road_mesh_data();
    assert_eq!(roadbed_digest(&core), pieces_before);
    assert!(
        (core.treasury.balance - balance).abs() <= 1e-6,
        "{} refunded to {balance}",
        core.treasury.balance
    );
    assert!((core.treasury.lifetime_build_cost - lifetime).abs() <= 1e-6);
    core.prepare_cell_lots_internal();
    assert_references_consistent(&core);
}

// Test 3: a batch whose last stroke covers plotted land (a field) is refused before any stroke
// takes land: nothing is demolished, erased, charged or recorded, and the refusal names the
// stroke.
#[test]
fn a_batch_whose_last_stroke_is_refused_changes_nothing() {
    let (mut core, _) = street_with_houses();
    let field = [
        Vector2::new(100.0, -90.0),
        Vector2::new(130.0, -90.0),
        Vector2::new(130.0, -60.0),
        Vector2::new(100.0, -60.0),
    ];
    let owner = core.allocator.buildings.len();
    core.allocator.field_clearance_mut().set(owner, &field);
    let state = land_state(&core);
    let products_before = products(&core);
    let painted_before = painted(&core);
    let strokes = [
        DRAFT[0],
        DRAFT[1],
        [
            Vector3::new(115.0, 0.0, -100.0),
            Vector3::new(115.0, 0.0, -40.0),
        ],
    ];
    assert_eq!(
        commit_batch(&mut core, &strokes),
        Some((2, "field_overlap".to_string()))
    );
    assert!(!core.transit_network.road_edit_is_staged());
    assert_eq!(land_state(&core), state);
    assert_eq!(products(&core), products_before);
    assert_eq!(painted(&core), painted_before);
    assert!(core.transit_network.roadbed.failed_nodes().is_empty());
    // The first two strokes alone commit.
    assert_eq!(commit_batch(&mut core, &strokes[..2]), None);
}

// Test 4: the cap of 30 counts groups: a batch's records leave the history together.
#[test]
fn the_undo_cap_counts_groups() {
    let mut core = test_core();
    core.benchmark_mode = false;
    let group = core.next_undo_group();
    core.undo_batch_group = Some(group);
    for _ in 0..5 {
        core.push_network_undo_for_polyline(&[], 0.0);
    }
    core.undo_batch_group = None;
    for _ in 0..29 {
        core.push_network_undo_for_polyline(&[], 0.0);
    }
    assert_eq!(
        core.undo_stack.len(),
        34,
        "30 groups, the batch's five records among them"
    );
    core.push_network_undo_for_polyline(&[], 0.0);
    assert_eq!(core.undo_stack.len(), 30, "the batch left whole");
    assert!(core.undo_stack.iter().all(|record| record.group != group));
}

// Test 5 (U5): another edit that charges, a widening, is refunded by its undo.
#[test]
fn undo_refunds_a_widening() {
    let (mut core, street) = street_on(|_| {});
    let balance = core.treasury.balance;
    let lifetime = core.treasury.lifetime_build_cost;
    let attributes = RoadAttributes::of_edge(core.region_graph.edge(street));
    assert!(core.set_edge_cross_section_internal(
        street as i32,
        2,
        2,
        None,
        RoadParking::NONE,
        attributes
    ));
    let charged = core.undo_stack.back().map_or(0.0, |record| record.charged);
    assert!(charged > 0.0, "the widening is charged");
    assert!((balance - core.treasury.balance - charged).abs() <= 1e-6);
    assert!(core.undo_action_internal());
    assert!((core.treasury.balance - balance).abs() <= 1e-6);
    assert!((core.treasury.lifetime_build_cost - lifetime).abs() <= 1e-6);
}
