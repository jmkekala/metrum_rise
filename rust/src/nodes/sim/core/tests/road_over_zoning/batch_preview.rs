// SPDX-License-Identifier: GPL-2.0-only

//! Batch preview (`ROAD-112` round G1, plan 6.2): tier 1's verdict is each stroke's hover check;
//! tier 2's finished preview is the batch commit's pieces bitwise, refuses the stroke the commit
//! refuses, leaves the replica equal to the view's graph after every frame and across a restart;
//! a batch committed with tier 2's plans equals one committed through the dry run; batches over
//! the caps are refused.

use super::batch::{DRAFT, products, stroke};
use super::preview::tool_context;
use super::*;
use crate::debug::work::debug_hash;
use crate::nodes::sim::core::road_batch::{BATCH_MAX_LENGTH_M, BATCH_MAX_STROKES};
use crate::nodes::sim::core::{
    BATCH_TOO_LARGE_REASON, BatchVerdict, RoadBatchRequest, RoadPreviewContext,
};
use crate::simulation::network::graph::{RegionGraph, RoadParking};
use crate::simulation::network::road_types::RoadAttributes;
use crate::simulation::network::roadbed::PieceId;

fn request(context: &RoadPreviewContext, points: &[Vector3]) -> RoadPreviewRequest {
    RoadPreviewRequest {
        surface_generation: context.view.surface_generation,
        points: points.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        profile: crate::simulation::network::road_edit::RoadProfile::FollowGround,
        median: None,
        parking: RoadParking::NONE,
        attributes: RoadAttributes::DEFAULT,
    }
}

fn requests(context: &RoadPreviewContext, strokes: &[[Vector3; 2]]) -> Vec<RoadPreviewRequest> {
    strokes
        .iter()
        .map(|points| request(context, points))
        .collect()
}

// Every value of a graph's records the preview's plans and pieces read, with the spatial index's
// answer around the draft.
fn graph_digest(graph: &RegionGraph) -> u64 {
    let edges: Vec<_> = (0..graph.edge_count())
        .map(|edge| {
            let edge = graph.edge(edge);
            (
                edge.deleted,
                edge.start_node,
                edge.end_node,
                edge.width.to_bits(),
                edge.fwd_lanes,
                edge.bkw_lanes,
                format!("{:?}{:?}", edge.start_clip, edge.end_clip),
                format!("{:?}{:?}", edge.geometry, edge.physical_geometry),
            )
        })
        .collect();
    let nodes: Vec<_> = (0..graph.node_count() as u32)
        .map(|node| {
            let record = graph.node(node);
            (
                format!("{:?}", record.pos),
                graph.get_valid_node(node),
                (node as usize) < graph.node_adjacency_count(),
            )
        })
        .collect();
    let adjacency: Vec<_> = (0..graph.node_adjacency_count() as u32)
        .map(|node| graph.node_adjacency(node).to_vec())
        .collect();
    let mut near: Vec<_> = graph
        .get_edges_near_aabb(Vector3::new(-100.0, 0.0, -120.0), Vector3::new(150.0, 0.0, 50.0))
        .into_iter()
        .collect();
    near.sort_unstable();
    debug_hash(&(edges, nodes, adjacency, near))
}

// Runs tier 2 to its verdict, checking after every frame that the replica equals the view's
// graph again.
fn finish_tier_two(context: &mut RoadPreviewContext, strokes: &[[Vector3; 2]]) -> BatchVerdict {
    let view = graph_digest(&context.view.region_graph);
    for _ in 0..=strokes.len() {
        let requests = requests(context, strokes);
        let (batch, _) = context.preview_batch_step(requests);
        let verdict = batch.verdict.clone();
        assert_eq!(graph_digest(&context.replica), view, "the replica is restored");
        if let Some(verdict) = verdict {
            return verdict;
        }
    }
    panic!("tier 2 runs one stroke a frame");
}

// Tier 1 is each stroke's hover check, the invalid ones included.
#[test]
fn tier_one_is_each_strokes_hover_check() {
    let (mut core, _) = street_with_houses();
    let context = tool_context(&mut core);
    let mut strokes = DRAFT.to_vec();
    // Too short a stroke, which the hover check refuses.
    strokes.push([Vector3::new(60.0, 0.0, -60.0), Vector3::new(60.2, 0.0, -60.0)]);
    let requests = requests(&context, &strokes);
    let hover = context.view.validate_batch(&requests).expect("within the caps");
    assert_eq!(hover.len(), strokes.len());
    for (request, (prepared, validation)) in requests.iter().zip(&hover) {
        let (alone, alone_validation) = context.view.validate(request);
        assert_eq!(prepared.points, alone.points);
        assert_eq!(prepared.class, alone.class);
        assert_eq!(*validation, alone_validation);
    }
    assert!(hover[..DRAFT.len()].iter().all(|(_, v)| v.is_valid));
    assert!(!hover[DRAFT.len()].1.is_valid, "the short stroke is refused");
}

// Tier 2's finished preview holds the batch commit's pieces bitwise, and the replica is the
// view's graph after every frame.
#[test]
fn tier_two_previews_the_batch_commits_pieces() {
    let (mut core, _) = street_with_houses();
    let mut context = tool_context(&mut core);
    assert_eq!(
        finish_tier_two(&mut context, &DRAFT),
        BatchVerdict::Accepted
    );
    let batch = context.batch().expect("the batch stands");
    assert_eq!(batch.strokes.len(), DRAFT.len());
    assert_eq!(batch.frames.len(), DRAFT.len(), "one stroke a frame");
    let outcome = core.commit_road_batch(
        RoadBatchRequest {
            strokes: DRAFT.iter().map(|points| stroke(points)).collect(),
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.outcome.committed);
    let live = &core.transit_network.roadbed;
    let mut owners = 0;
    for stroke in &batch.strokes {
        for (id, _) in stroke.delta.pieces() {
            owners += 1;
            assert_eq!(
                debug_hash(&batch.roadbed().piece_mesh(id)),
                debug_hash(&live.piece_mesh(id)),
                "{id:?}"
            );
        }
    }
    assert!(owners > DRAFT.len());
    let every = |roadbed: &crate::simulation::network::roadbed::RoadbedSystem| {
        let spans: Vec<_> = (0..core.region_graph.edge_count())
            .map(|edge| roadbed.piece_mesh(PieceId::Span(edge)))
            .collect();
        let nodes: Vec<_> = (0..core.region_graph.node_count() as u32)
            .map(|node| roadbed.piece_mesh(PieceId::Node(node)))
            .collect();
        debug_hash(&(spans, nodes))
    };
    assert_eq!(every(batch.roadbed()), every(live));
    assert_eq!(batch.roadbed().generation(), live.generation());
}

// Tier 2 refuses the stroke the commit refuses, for the commit's reason: the last stroke over a
// field.
#[test]
fn tier_two_refuses_the_stroke_the_commit_refuses() {
    let (mut core, _) = street_with_houses();
    let field = [
        Vector2::new(100.0, -90.0),
        Vector2::new(130.0, -90.0),
        Vector2::new(130.0, -60.0),
        Vector2::new(100.0, -60.0),
    ];
    let owner = core.allocator.buildings.len();
    core.allocator.field_clearance_mut().set(owner, &field);
    let mut context = tool_context(&mut core);
    let strokes = [
        DRAFT[0],
        DRAFT[1],
        [
            Vector3::new(115.0, 0.0, -100.0),
            Vector3::new(115.0, 0.0, -40.0),
        ],
    ];
    assert_eq!(
        finish_tier_two(&mut context, &strokes),
        BatchVerdict::Refused {
            stroke: Some(2),
            reason: "field_overlap".to_owned()
        }
    );
    assert!(context.take_batch_products(&requests(&context, &strokes)).is_none());
}

// A new pose restarts tier 2 with the replica bitwise the view's graph; the restarted batch
// still previews the commit's pieces.
#[test]
fn a_new_pose_restarts_tier_two_on_a_restored_replica() {
    let (mut core, _) = street_with_houses();
    let mut context = tool_context(&mut core);
    let view = graph_digest(&context.view.region_graph);
    for _ in 0..2 {
        let requests = requests(&context, &DRAFT);
        let (batch, frame) = context.preview_batch_step(requests);
        assert!(frame.is_some() && batch.verdict.is_none());
    }
    assert_eq!(context.batch().map(|batch| batch.strokes.len()), Some(2));
    assert_eq!(graph_digest(&context.replica), view);
    // The pointer moved: the last stroke now ends elsewhere.
    let mut moved = DRAFT;
    moved[2][0].x = -40.0;
    let requests = requests(&context, &moved);
    let (batch, _) = context.preview_batch_step(requests);
    assert_eq!(batch.strokes.len(), 1, "restarted");
    assert_eq!(graph_digest(&context.replica), view);
    assert_eq!(finish_tier_two(&mut context, &moved), BatchVerdict::Accepted);
    // A single stroke's preview after the batch reads the view's graph.
    let single = context.preview_road(request(&context, &DRAFT[0]));
    assert!(single.validation.is_valid);
    assert_eq!(graph_digest(&context.replica), view);
}

// A batch committed with tier 2's plans and pieces (its dry run keeping only the checks) leaves
// the same products bitwise as one committed through the dry run.
#[test]
fn a_batch_committed_with_tier_twos_plans_equals_one_through_the_dry_run() {
    let (mut planned, _) = street_with_houses();
    planned.precompute_road_mesh_data();
    let outcome = planned.commit_road_batch(
        RoadBatchRequest {
            strokes: DRAFT.iter().map(|points| stroke(points)).collect(),
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.outcome.committed);
    assert_eq!(outcome.times.preview_reused, 0);
    planned.adopt_pending_cell_lots();

    let (mut previewed, _) = street_with_houses();
    let mut context = tool_context(&mut previewed);
    assert_eq!(
        finish_tier_two(&mut context, &DRAFT),
        BatchVerdict::Accepted
    );
    let products_of = context.take_batch_products(&requests(&context, &DRAFT));
    assert!(products_of.is_some());
    assert!(context.batch().is_none(), "the commit takes the batch");
    let outcome = previewed.commit_road_batch(
        RoadBatchRequest {
            strokes: DRAFT.iter().map(|points| stroke(points)).collect(),
            preview: products_of,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.outcome.committed);
    assert_eq!(outcome.times.preview_reused, DRAFT.len());
    assert_eq!(outcome.times.reused, DRAFT.len());
    previewed.adopt_pending_cell_lots();
    assert_eq!(products(&previewed), products(&planned));
    assert_references_consistent(&previewed);
}

// Products of another stroke list, or of a stale view, are not installed: the dry run plans.
#[test]
fn tier_twos_plans_for_other_strokes_are_not_installed() {
    let (mut core, _) = street_with_houses();
    let mut context = tool_context(&mut core);
    assert_eq!(
        finish_tier_two(&mut context, &DRAFT),
        BatchVerdict::Accepted
    );
    let products_of = context.take_batch_products(&requests(&context, &DRAFT));
    let mut moved = DRAFT;
    moved[2][0].x = -40.0;
    let outcome = core.commit_road_batch(
        RoadBatchRequest {
            strokes: moved.iter().map(|points| stroke(points)).collect(),
            preview: products_of,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.outcome.committed);
    assert_eq!(outcome.times.preview_reused, 0);
}

// Batches over 40 strokes or 8 km are refused as `batch_too_large` by both tiers; the caps
// themselves pass.
#[test]
fn batches_over_the_caps_are_refused() {
    let (mut core, _) = street_with_houses();
    let mut context = tool_context(&mut core);
    let short = |i: usize| {
        let x = -300.0 + 10.0 * i as f32;
        [Vector3::new(x, 0.0, 200.0), Vector3::new(x, 0.0, 205.0)]
    };
    let at_cap: Vec<_> = (0..BATCH_MAX_STROKES).map(short).collect();
    assert!(context.view.validate_batch(&requests(&context, &at_cap)).is_some());
    let over: Vec<_> = (0..=BATCH_MAX_STROKES).map(short).collect();
    assert!(context.view.validate_batch(&requests(&context, &over)).is_none());
    let refused = BatchVerdict::Refused {
        stroke: None,
        reason: BATCH_TOO_LARGE_REASON.to_owned(),
    };
    assert_eq!(finish_tier_two(&mut context, &over), refused);
    // Two strokes just over 8 km together.
    let half = (BATCH_MAX_LENGTH_M * 0.5) as f32 + 1.0;
    let long = [
        [Vector3::new(0.0, 0.0, 300.0), Vector3::new(half, 0.0, 300.0)],
        [Vector3::new(0.0, 0.0, 320.0), Vector3::new(half, 0.0, 320.0)],
    ];
    assert!(context.view.validate_batch(&requests(&context, &long)).is_none());
    assert_eq!(finish_tier_two(&mut context, &long), refused);
}
