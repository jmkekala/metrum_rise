// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-112` round H: the stroke profile field and the automatic bridge/tunnel segmentation of a
//! draft through the existing preparation, validation and batch commit. Today's strokes keep
//! their products through the field (digests recorded from the branch point's binary), raised
//! ends give the inferred bridge, and a draft with a bridge and a tunnel commits as one undo
//! group.

use super::road_tunnels::{THROUGH_HILL, cone_hill_core, hill_core};
use super::*;
use crate::nodes::sim::core::RoadEditPlan;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::nodes::sim::road_tool::height::{DraftSection, draft_batch, segment_draft};
use crate::simulation::network::road_edit::RoadProfile;
use crate::simulation::network::types::EdgeClass;

const SECTION: DraftSection = DraftSection {
    fwd_lanes: 1,
    bkw_lanes: 1,
    snap_to_existing_roads: true,
    median: None,
    parking: crate::simulation::network::graph::RoadParking::NONE,
    attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
};

// A 256 m world, flat at height 0, with no roads.
fn flat_core() -> SimCore {
    let mut core = test_core();
    core.treasury.balance = 1.0e9;
    core.benchmark_mode = false;
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    core.precompute_road_mesh_data();
    core
}

fn request(core: &SimCore, points: &[Vector3], profile: RoadProfile) -> RoadPreviewRequest {
    RoadPreviewRequest {
        surface_generation: core.road_tool_surface_generation,
        points: points.to_vec(),
        fwd_lanes: SECTION.fwd_lanes,
        bkw_lanes: SECTION.bkw_lanes,
        snap_to_existing_roads: SECTION.snap_to_existing_roads,
        profile,
        median: None,
        parking: SECTION.parking,
        attributes: SECTION.attributes,
    }
}

fn commit(core: &mut SimCore, points: &[Vector3], profile: RoadProfile) -> bool {
    core.commit_road(
        RoadCommitRequest {
            points: points.to_vec(),
            fwd_lanes: SECTION.fwd_lanes,
            bkw_lanes: SECTION.bkw_lanes,
            snap_to_existing_roads: SECTION.snap_to_existing_roads,
            profile,
            median: None,
            parking: SECTION.parking,
            attributes: SECTION.attributes,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    )
    .committed
}

// FNV-1a over 32-bit words: a digest both binaries compute alike.
#[derive(Default)]
struct Digest(u64);

impl Digest {
    fn word(&mut self, word: u32) {
        for byte in word.to_le_bytes() {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn points(&mut self, points: &[Vector3]) {
        self.word(points.len() as u32);
        for point in points {
            for value in [point.x, point.y, point.z] {
                self.word(value.to_bits());
            }
        }
    }

    fn plan(&mut self, plan: &RoadEditPlan) {
        let prepared = plan.prepared();
        self.points(&prepared.points);
        self.word(prepared.class as u32);
        self.word(u32::from(prepared.elevated_ends[0]) | u32::from(prepared.elevated_ends[1]) << 1);
        self.word(u32::from(plan.validation().is_valid));
        for byte in plan.validation().invalid_reason.bytes() {
            self.word(u32::from(byte));
        }
    }

    // Every live edge of the graph in id order: class and both profiles.
    fn graph(&mut self, core: &SimCore) {
        for (id, edge) in core.region_graph.edges().iter().enumerate() {
            if edge.deleted {
                continue;
            }
            self.word(id as u32);
            self.word(edge.class as u32);
            self.points(&edge.geometry);
            self.points(&edge.physical_geometry);
        }
    }
}

// The plan of a stroke drawn as `points` with both ends raised the same height above flat
// ground: in the branch, the draft's grade line segmented into strokes (one bridge) and planned
// with their profile; the branch point inferred the bridge from the follow-ground solve.
fn raised_plans(core: &SimCore, points: &[Vector3]) -> Vec<RoadEditPlan> {
    let path: Vec<_> = points.iter().map(|p| Vector3::new(p.x, 0.0, p.z)).collect();
    let heights = [points[0].y, points[points.len() - 1].y];
    let strokes = segment_draft(
        &path,
        heights,
        SECTION,
        &core.heightmap,
        &core.region_graph,
        &core.transit_network.roadbed,
    )
    .unwrap();
    strokes
        .iter()
        .map(|stroke| {
            RoadEditPlan::compile(
                core,
                request(core, &stroke.points, stroke.profile),
            )
        })
        .collect()
}

const RAISED: [f32; 2] = [2.5, 5.0];

fn raised_stroke(raise: f32) -> [Vector3; 2] {
    [
        Vector3::new(-50.0, raise, 0.0),
        Vector3::new(50.0, raise, 0.0),
    ]
}

// The follow-ground strokes, including a curve over the hill's flank and one ending on it.
fn follow_ground_strokes() -> Vec<Vec<Vector3>> {
    vec![
        THROUGH_HILL.to_vec(),
        (0..=12)
            .map(|step| {
                let angle = step as f32 * 0.25;
                Vector3::new(60.0 * angle.cos(), 0.0, 60.0 * angle.sin())
            })
            .collect(),
        vec![
            Vector3::new(-110.0, 0.0, 70.0),
            Vector3::new(10.0, 0.0, 20.0),
        ],
    ]
}

// Products of today's strokes through the profile field: follow-ground plans and commits (or
// refusals) on a 1:4 hill, raised ends on flat ground, and the tunnel fixtures. The digests were recorded by
// running this test's twin on the branch point (`dev` cd49a275), which drew the same strokes
// with the tunnel flag and inferred the bridges from the follow-ground solve.
#[test]
fn todays_strokes_keep_the_branch_points_products() {
    let mut follow = Digest::default();
    let mut core = cone_hill_core(100.0, 0.25);
    for stroke in follow_ground_strokes() {
        follow.plan(&RoadEditPlan::compile(
            &core,
            request(&core, &stroke, RoadProfile::FollowGround),
        ));
        let committed = commit(&mut core, &stroke, RoadProfile::FollowGround);
        follow.word(u32::from(committed));
    }
    follow.graph(&core);

    let mut raised = Digest::default();
    for raise in RAISED {
        let mut core = flat_core();
        let stroke = raised_stroke(raise);
        let plans = raised_plans(&core, &stroke);
        assert_eq!(plans.len(), 1, "one bridge");
        assert_eq!(plans[0].profile(), RoadProfile::Bridge);
        assert_eq!(plans[0].request().points, stroke, "the draft's own points");
        raised.plan(&plans[0]);
        let batch = draft_batch(
            vec![crate::nodes::sim::road_tool::height::DraftStroke {
                points: stroke.to_vec(),
                profile: RoadProfile::Bridge,
            }],
            SECTION,
        );
        assert!(
            core.commit_road_batch(batch, std::time::Instant::now(), Default::default())
                .outcome
                .committed
        );
        raised.graph(&core);
    }

    let mut tunnels = Digest::default();
    for mut core in [hill_core(80.0), cone_hill_core(100.0, 0.25)] {
        tunnels.plan(&RoadEditPlan::compile(
            &core,
            request(&core, &THROUGH_HILL, RoadProfile::Tunnel),
        ));
        assert!(commit(&mut core, &THROUGH_HILL, RoadProfile::Tunnel));
        tunnels.graph(&core);
    }
    assert_eq!(
        [follow.0, raised.0, tunnels.0],
        BRANCH_POINT_DIGESTS,
        "follow-ground, raised, tunnel"
    );
}

// Recorded on `dev` cd49a275 (see the test above). The follow-ground digest re-recorded by
// `ROAD-118` round JS4a: the arc's end stands 8.5 m beside the through-hill road, its cap on that
// road's side, and now runs on to it as a T.
const BRANCH_POINT_DIGESTS: [u64; 3] = [
    0x9a7d_e29e_1f58_20fb,
    0x6d30_1655_d3ee_62e0,
    0x32c0_e185_fa25_f9d8,
];

// Raised ends: the bridge the follow-ground solve infers, the same points, class, ends and
// validation through the segmented draft's explicit bridge.
#[test]
fn raised_ends_give_the_inferred_bridge() {
    let core = flat_core();
    for raise in RAISED {
        let stroke = raised_stroke(raise);
        let inferred =
            RoadEditPlan::compile(&core, request(&core, &stroke, RoadProfile::FollowGround));
        let plans = raised_plans(&core, &stroke);
        assert_eq!(plans.len(), 1);
        let (a, b) = (inferred.prepared(), plans[0].prepared());
        assert_eq!(a.class, EdgeClass::Bridge);
        assert_eq!(
            (&a.points, a.class, a.elevated_ends),
            (&b.points, b.class, b.elevated_ends)
        );
        assert_eq!(
            format!("{:?}", inferred.validation()),
            format!("{:?}", plans[0].validation())
        );
    }
}

// The ground of `cone_hill_core(40, 0.5)` (a hill 20 m high at the origin) with a valley 6 m deep
// across the draft between x = -100 and -60.
fn valley_and_hill_core() -> SimCore {
    let mut core = cone_hill_core(40.0, 0.5);
    for z in 0..257 {
        for x in 0..257 {
            let wx = x as f32 - 128.0;
            if (-100.0..=-60.0).contains(&wx) && (z as f32 - 128.0).abs() < 40.0 {
                let depth = 6.0 * ((wx + 100.0) * std::f32::consts::PI / 40.0).sin();
                core.heightmap
                    .set_height(x, z, -depth / crate::config::HEIGHT_SCALE);
            }
        }
    }
    core.precompute_road_mesh_data();
    core
}

// A level draft at the ground's height over the valley and through the hill: held ground, a
// bridge landing to landing, held ground, a tunnel and held ground, committed as one batch;
// one undo takes back all of it.
#[test]
fn a_draft_with_a_bridge_then_a_tunnel_commits_as_one_undo_group() {
    let mut core = valley_and_hill_core();
    let path = [
        Vector3::new(-120.0, 0.0, 0.0),
        Vector3::new(110.0, 0.0, 0.0),
    ];
    let strokes = segment_draft(
        &path,
        [0.0, 0.0],
        SECTION,
        &core.heightmap,
        &core.region_graph,
        &core.transit_network.roadbed,
    )
    .unwrap();
    let profiles: Vec<_> = strokes.iter().map(|stroke| stroke.profile).collect();
    assert_eq!(
        profiles,
        [
            RoadProfile::HeldGround,
            RoadProfile::Bridge,
            RoadProfile::HeldGround,
            RoadProfile::Tunnel,
            RoadProfile::HeldGround,
        ]
    );
    for pair in strokes.windows(2) {
        assert_eq!(pair[0].points.last(), pair[1].points.first());
    }
    let outcome = core.commit_road_batch(
        draft_batch(strokes, SECTION),
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(
        outcome.outcome.committed,
        "{} at stroke {:?}",
        outcome.outcome.rejection, outcome.refused_stroke
    );
    let live = |class: EdgeClass| {
        core.region_graph
            .edges()
            .iter()
            .filter(|edge| !edge.deleted && edge.class == class)
            .count()
    };
    assert_eq!(live(EdgeClass::Bridge), 1);
    assert_eq!(live(EdgeClass::Tunnel), 1);
    // The joints stand at the grade line's height: the ground under them.
    for edge in core
        .region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted)
    {
        for end in [edge.geometry[0], edge.geometry[edge.geometry.len() - 1]] {
            assert!(end.y.abs() < 1.0e-3, "{end:?}");
        }
    }
    assert!(core.undo_action_internal());
    assert!(core.region_graph.edges().iter().all(|edge| edge.deleted));
}

// A draft 6 m under a ridge only 2 m wide across its section: no knot has bore cover (the
// shell's corners stand under open ground), and 6 m is deeper than a cut may go.
#[test]
fn a_deep_cut_without_bore_cover_is_refused() {
    let mut core = flat_core();
    // A ridge 6 m high along the draft from x = -10 to 10, 2 m wide across it.
    for z in 0..257 {
        for x in 0..257 {
            let (wx, wz) = (x as f32 - 128.0, z as f32 - 128.0);
            if wx.abs() <= 10.0 && wz.abs() <= 1.0 {
                core.heightmap
                    .set_height(x, z, 6.0 / crate::config::HEIGHT_SCALE);
            }
        }
    }
    core.precompute_road_mesh_data();
    let path = [Vector3::new(-40.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)];
    let segmented = segment_draft(
        &path,
        [0.0, 0.0],
        SECTION,
        &core.heightmap,
        &core.region_graph,
        &core.transit_network.roadbed,
    );
    assert_eq!(
        segmented,
        Err(crate::nodes::sim::road_tool::height::CUT_TOO_DEEP_REASON)
    );
}

// The portal rows' x of every published span piece of `core` (round I's flat-ground tunnels run
// along x), each checked to carry its collar (the headwall) and a lined bore beside it.
fn portal_rows_x(core: &SimCore) -> Vec<f64> {
    use crate::simulation::network::roadbed::PieceId;
    let roadbed = &core.transit_network.roadbed;
    let mut rows = Vec::new();
    for mesh in (0..core.region_graph.edge_count())
        .filter_map(|edge| roadbed.piece_mesh(PieceId::Span(edge)))
    {
        let before = rows.len();
        rows.extend(mesh.portal_rows().map(|row| row[0].x));
        if rows.len() > before {
            assert!(!mesh.collar_triangles().is_empty(), "a portal row without its collar");
            assert!(!mesh.lining_triangles().is_empty(), "a portal row without a lining");
        }
    }
    rows.sort_by(f64::total_cmp);
    rows
}

fn live_edges(core: &SimCore, class: EdgeClass) -> usize {
    core.region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.class == class)
        .count()
}

fn commit_draft(core: &mut SimCore, path: &[Vector3], heights: [f32; 2]) -> Vec<RoadProfile> {
    let strokes = segment_draft(
        path,
        heights,
        SECTION,
        &core.heightmap,
        &core.region_graph,
        &core.transit_network.roadbed,
    )
    .unwrap();
    let profiles = strokes.iter().map(|stroke| stroke.profile).collect();
    let outcome = core.commit_road_batch(
        draft_batch(strokes, SECTION),
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(
        outcome.outcome.committed,
        "{} at stroke {:?}",
        outcome.outcome.rejection, outcome.refused_stroke
    );
    core.precompute_road_mesh_data();
    profiles
}

// Round I, on flat ground (the user's question): a draft from the ground whose far end is
// lowered 10 m is one tunnel stroke, its approach a cut ramp from the start and its bore from
// the first lattice knot with the 5 m cover (x = -48, 52 m along at 10 %), the end buried.
// Committed as one batch it builds the approach, the bore and one portal row there. A second
// draft back up from the buried end to the ground mirrors it: a second bore and portal, and one
// undo takes back only that draft.
#[test]
fn a_lowered_draft_on_flat_ground_ramps_through_a_portal() {
    let mut core = flat_core();
    let down = [Vector3::new(-100.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)];
    assert_eq!(
        commit_draft(&mut core, &down, [0.0, -10.0]),
        [RoadProfile::Tunnel]
    );
    assert_eq!(live_edges(&core, EdgeClass::Tunnel), 1);
    assert!(live_edges(&core, EdgeClass::Standard) >= 1, "the approach");
    let rows = portal_rows_x(&core);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!((rows[0] + 48.0).abs() <= 4.0, "portal at {rows:?}");

    let up = [Vector3::new(0.0, -10.0, 0.0), Vector3::new(100.0, 0.0, 0.0)];
    assert_eq!(
        commit_draft(&mut core, &up, [-10.0, 0.0]),
        [RoadProfile::Tunnel]
    );
    assert_eq!(live_edges(&core, EdgeClass::Tunnel), 2);
    let rows = portal_rows_x(&core);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!((rows[1] - 48.0).abs() <= 4.0, "portal at {rows:?}");
    assert!(core.undo_action_internal());
    assert_eq!(live_edges(&core, EdgeClass::Tunnel), 1);
}
