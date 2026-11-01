// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-61` round R8h (the user's play-test of 2026-11-01): a road over a tunnel is a
//! grade-separated crossing, refused only when its deck would stand within the bore's cover of
//! the tunnel's deck; a tunnel drawn under a road keeps that cover under the road's deck; and a
//! tunnel's bore leaves the zoning cells over it as they are.

use super::road_tunnels::{THROUGH_HILL, cone_hill, cone_hill_core};
use super::*;
use crate::nodes::sim::core::road_commit::{RoadCommitOutcome, RoadCommitRequest};
use crate::nodes::sim::core::road_preview::{RoadPreview, road_tool_snapshot_from_core};
use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::road_edit::RoadPreviewValidation;
use crate::simulation::network::roadbed::{PieceId, PieceMesh};
use crate::simulation::network::types::EdgeClass;
use std::collections::BTreeMap;
use std::sync::Arc;

// A road crossing the 1:4 hill north to south 50 m west of its top, over the bore of
// `tunnel_core`, where the hill stands 12.5 m over the tunnel's deck.
const OVER_THE_BORE: [Vector3; 2] = [
    Vector3::new(-50.0, 0.0, -30.0),
    Vector3::new(-50.0, 0.0, 30.0),
];

// The 1:4 hill 25 m high on the 256 m world.
fn hill() -> SimCore {
    cone_hill_core(100.0, 0.25)
}

fn commit(core: &mut SimCore, points: &[Vector3], tunnel: bool) -> RoadCommitOutcome {
    commit_with(core, points, tunnel, None)
}

fn commit_with(
    core: &mut SimCore,
    points: &[Vector3],
    tunnel: bool,
    preview: Option<Arc<RoadPreview>>,
) -> RoadCommitOutcome {
    core.precompute_road_mesh_data();
    core.refresh_road_terrain_inputs();
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            tunnel,
            preview,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    core.precompute_road_mesh_data();
    outcome
}

// The road tool's hover check and preview of a stroke, snapped to the roads as the tool snaps.
fn preview(
    core: &mut SimCore,
    points: &[Vector3],
    tunnel: bool,
) -> (RoadPreviewValidation, Arc<RoadPreview>) {
    core.precompute_road_mesh_data();
    core.refresh_road_terrain_inputs();
    let request = || RoadPreviewRequest {
        surface_generation: core.road_tool_surface_generation,
        points: points.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        tunnel,
    };
    let view = road_tool_snapshot_from_core(core).expect("current roadbed");
    let quick = view.validate(&request()).1;
    let preview = RoadPreviewContext::capture(view).preview_road(request());
    (quick, Arc::new(preview))
}

// `tunnel_core`'s hill with the straight tunnel through it: its bore from x = -76 to 76 at
// height 0, under 5 m or more of the hill.
fn tunnel_core() -> SimCore {
    let mut core = hill();
    let outcome = commit(&mut core, &THROUGH_HILL, true);
    assert!(outcome.committed, "{}", outcome.rejection);
    core
}

fn tunnel_edge(core: &SimCore) -> usize {
    let tunnels: Vec<usize> = (core.region_graph.edges().iter().enumerate())
        .filter(|(_, edge)| !edge.deleted && edge.class == EdgeClass::Tunnel)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(tunnels.len(), 1, "one tunnel edge");
    tunnels[0]
}

// Every published piece's vertex bits and triangles.
type PieceBits = BTreeMap<PieceId, (Vec<[u64; 3]>, Vec<[u32; 3]>)>;

fn pieces(core: &SimCore) -> PieceBits {
    let roadbed = &core.transit_network.roadbed;
    roadbed
        .pieces_in_bounds([f64::MIN, f64::MIN, f64::MAX, f64::MAX])
        .map(|(id, _)| {
            let mesh: &PieceMesh = roadbed.piece_mesh(id).expect("listed pieces are published");
            let positions = (mesh.positions().iter())
                .map(|p| [p.x, p.y, p.z].map(f64::to_bits))
                .collect();
            (id, (positions, mesh.triangles().to_vec()))
        })
        .collect()
}

// Lowers the source terrain to `height_m` along the strip `x0 <= x <= x1` of the 256 m world,
// where the hill stands higher: a cut dug across the hill.
fn dig(core: &mut SimCore, x0: f32, x1: f32, height_m: f32) {
    for z in 0..257 {
        for x in 0..257 {
            let wx = x as f32 - 128.0;
            if wx >= x0 && wx <= x1 {
                let natural = cone_hill(100.0, 0.25, wx, z as f32 - 128.0);
                core.heightmap.set_height(
                    x,
                    z,
                    natural.min(height_m) / crate::config::HEIGHT_SCALE,
                );
            }
        }
    }
}

// The hill restored along the strip `dig` lowered.
fn refill(core: &mut SimCore, x0: f32, x1: f32) {
    dig(core, x0, x1, f32::INFINITY);
}

// Acceptance 1 (the user's first report): a road over a tunnel commits as a grade-separated
// crossing. Before, the stroke's support read the tunnel's buried carriageway as a visible one
// and pinned the road's profile down to the bore's deck over the hill. The hover check, the
// preview and the commit agree; the commit installs the preview's pieces, equal to a fresh
// commit's; the tunnel keeps its edge, nodes and profile, with no node or split on either road;
// the road climbs over the bore on the hill; undo restores every piece.
#[test]
fn a_road_over_a_tunnel_crosses_it_grade_separated() {
    let mut core = tunnel_core();
    let tunnel = tunnel_edge(&core);
    let before = pieces(&core);
    let graph_before = (
        core.region_graph.node_count(),
        core.region_graph.edge(tunnel).clone(),
    );
    let (quick, preview) = preview(&mut core, &OVER_THE_BORE, false);
    assert!(quick.is_valid, "{}", quick.invalid_reason);
    assert!(
        preview.validation.is_valid,
        "{}",
        preview.validation.invalid_reason
    );
    let outcome = commit_with(&mut core, &OVER_THE_BORE, false, Some(preview));
    assert!(outcome.committed, "{}", outcome.rejection);
    assert!(core.last_road_edit_metrics.preview_plan_reused);
    let mut control = tunnel_core();
    assert!(commit(&mut control, &OVER_THE_BORE, false).committed);
    assert_eq!(pieces(&core), pieces(&control), "preview equals commit");

    let graph = &core.region_graph;
    let edge = graph.edge(tunnel);
    assert!(!edge.deleted && edge.class == EdgeClass::Tunnel);
    assert_eq!(edge.geometry, graph_before.1.geometry);
    assert_eq!(edge.physical_geometry, graph_before.1.physical_geometry);
    assert_eq!(
        [edge.start_node, edge.end_node],
        [graph_before.1.start_node, graph_before.1.end_node]
    );
    assert_eq!(
        graph.node_count(),
        graph_before.0 + 2,
        "the road's two ends only"
    );
    let roads: Vec<&crate::simulation::network::graph::Edge> = (graph.edges().iter())
        .filter(|edge| !edge.deleted && edge.geometry.iter().all(|p| (p.x + 50.0).abs() < 1.0))
        .collect();
    assert_eq!(roads.len(), 1, "one unsplit road over the bore");
    let road = roads[0];
    assert_eq!(road.class, EdgeClass::Standard);
    let crest = RegionGraph::profile_height_at(&road.physical_geometry, -50.0, 0.0);
    assert!(
        (crest - cone_hill(100.0, 0.25, -50.0, 0.0)).abs() < 1.0,
        "the road crosses on the hill: {crest}"
    );

    assert!(core.undo_action_internal());
    assert!(core.transit_network.roadbed.is_current());
    assert_eq!(pieces(&core), before, "undo restores every piece");
}

// The clearance rule: a road whose deck would stand less than the bore's cover (5 m) above the
// tunnel's deck is refused `tunnel_too_close`, with that height as the clearance, by the hover
// check, the preview and the commit alike, and leaves the network as it was. A cut 3 m above the
// deck is dug across the hill after the tunnel was built; the road follows it.
#[test]
fn a_road_too_close_above_a_tunnel_is_refused() {
    let mut core = tunnel_core();
    dig(&mut core, -55.0, -45.0, 3.0);
    let before = pieces(&core);
    let (quick, preview) = preview(&mut core, &OVER_THE_BORE, false);
    for validation in [&quick, &preview.validation] {
        assert_eq!(
            validation.invalid_reason, "tunnel_too_close",
            "{validation:?}"
        );
        assert!(
            (validation.clearance_m - 3.0).abs() < 0.5,
            "{}",
            validation.clearance_m
        );
        assert_eq!(validation.required_clearance_m, 5.0);
    }
    let outcome = commit(&mut core, &OVER_THE_BORE, false);
    assert_eq!(
        outcome.rejection_reason, "tunnel_too_close",
        "{}",
        outcome.rejection
    );
    assert_eq!(pieces(&core), before);
}

// The reverse order: a tunnel drawn under a road keeps its cover under the road's deck, not the
// source terrain. The road was built along a cut 3 m high across the hill, then the hill was
// restored around it, so the road runs 9.5 m below the hill over the would-be bore. The tunnel
// is refused for its cover under the road (3 m); before, it read the hill's 12.5 m and passed
// 3 m under the road. Without the road the same stroke commits.
#[test]
fn a_tunnel_under_a_road_keeps_its_cover_under_the_road() {
    let mut core = hill();
    dig(&mut core, -55.0, -45.0, 3.0);
    assert!(commit(&mut core, &OVER_THE_BORE, false).committed);
    refill(&mut core, -55.0, -45.0);
    let (quick, preview) = preview(&mut core, &THROUGH_HILL, true);
    for validation in [&quick, &preview.validation] {
        assert_eq!(validation.invalid_reason, "tunnel_cover", "{validation:?}");
        assert!(
            (validation.clearance_m - 3.0).abs() < 0.5,
            "{}",
            validation.clearance_m
        );
    }
    let outcome = commit(&mut core, &THROUGH_HILL, true);
    assert_eq!(
        outcome.rejection_reason, "tunnel_cover",
        "{}",
        outcome.rejection
    );
    let mut bare = hill();
    assert!(commit(&mut bare, &THROUGH_HILL, true).committed);
}
