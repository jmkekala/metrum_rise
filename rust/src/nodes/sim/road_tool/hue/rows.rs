// SPDX-License-Identifier: GPL-2.0-only

//! The paused preview rows' worlds and poses in Rust (`ROAD-123`, round HF): round L1's
//! `road_preview_replay.gd` cases (`sloped_t`, `sloped_branch`, `flat_cross`, `sloped_cross`,
//! `dense`) built on a blank 512 m world with 8 m terrain cells, their roads committed through
//! the gameplay path, each pose's preview made outside timing, then the footprint hue built as
//! the session builds it (`road_tool_hue_of_preview`: the preview's grounded pieces, the drawn
//! graded surface read through the view's shared index). The bench times that build per pose;
//! the digest is the hue's vertices, bitwise, over every pose.

use super::*;
use crate::nodes::sim::core::{
    CityTreasury, RoadCommitRequest, RoadPreview, RoadPreviewContext, RoadPreviewRequest,
    RoadToolQuerySnapshot, SimCore,
};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::RoadParking;
use crate::simulation::network::road_edit::{DraftGround, RoadProfile};
use crate::simulation::network::road_types::RoadAttributes;
use godot::prelude::Vector2;
use std::time::Instant;

// Poses per row: evenly over one sweep or one orbit of the replay's pointer.
const POSES: usize = 48;
// The hue's half width outside the pieces (refused poses' strips): a two-lane road's ghost.
const HALF_WIDTH_M: f32 = 6.0;

// One row: its roads, the stroke's start and the pointer's path.
struct Row {
    name: &'static str,
    sloped: bool,
    roads: Vec<[Vector2; 2]>,
    start: Vector2,
    pointer: Pointer,
}

enum Pointer {
    Sweep(Vector2, Vector2),
    Orbit(Vector2, f32),
}

impl Pointer {
    // `road_preview_replay.gd`'s `_pose`.
    fn at(&self, u: f32) -> Vector2 {
        match *self {
            Self::Sweep(a, b) => a.lerp(b, u),
            Self::Orbit(centre, radius) => {
                centre + Vector2::from_angle(std::f32::consts::TAU * u) * radius
            }
        }
    }
}

fn rows() -> Vec<Row> {
    let v = Vector2::new;
    let main = [v(-110.0, 0.0), v(110.0, 0.0)];
    let t = |name, sloped, roads| Row {
        name,
        sloped,
        roads,
        start: v(0.0, -80.0),
        pointer: Pointer::Sweep(v(-40.0, 0.0), v(90.0, 0.0)),
    };
    let mut dense = vec![main];
    dense.extend(
        (32..=112)
            .step_by(16)
            .map(|z| [v(-100.0, z as f32), v(100.0, z as f32)]),
    );
    let cross = |name, sloped| Row {
        name,
        sloped,
        roads: [-45.0, -15.0, 15.0, 45.0]
            .map(|x| [v(x, -90.0), v(x, 90.0)])
            .to_vec(),
        start: v(-90.0, -30.0),
        pointer: Pointer::Orbit(v(90.0, 30.0), 20.0),
    };
    vec![
        t("sloped_t", true, vec![main]),
        Row {
            name: "sloped_branch",
            sloped: true,
            roads: vec![main, [v(-65.0, 0.0), v(-65.0, 60.0)]],
            start: v(-65.0, 0.0),
            pointer: Pointer::Orbit(v(-60.0, -55.0), 15.0),
        },
        cross("flat_cross", false),
        cross("sloped_cross", true),
        t("dense", false, dense),
    ]
}

// A row's world: the replay's blank world, its slope and its roads, and the published view.
struct RowWorld {
    core: SimCore,
    view: RoadToolQuerySnapshot,
}

impl RowWorld {
    fn build(row: &Row) -> Self {
        let mut core = SimCore {
            treasury: CityTreasury::new(1.0e9),
            ..SimCore::new(WorldConfig::new(512.0, 512.0, 40.0, 10.0))
        };
        core.create_blank_world_internal(512.0, 512.0, 8.0, 128.0, 0.0)
            .expect("the replay's world");
        if row.sloped {
            core.slope_terrain_internal(
                Vector2::ZERO,
                256.0,
                Vector2::new(-128.0, 0.0),
                0.0,
                Vector2::new(128.0, 0.0),
                6.4,
                1.0,
            );
        }
        for [a, b] in &row.roads {
            let points = vec![surface_point(&core, *a), surface_point(&core, *b)];
            let outcome = core.commit_road(
                RoadCommitRequest {
                    points,
                    fwd_lanes: 1,
                    bkw_lanes: 1,
                    snap_to_existing_roads: true,
                    profile: RoadProfile::FollowGround,
                    median: None,
                    parking: RoadParking::NONE,
                    attributes: RoadAttributes::DEFAULT,
                    preview: None,
                },
                Instant::now(),
                Default::default(),
            );
            assert!(outcome.committed, "{}: {}", row.name, outcome.rejection);
        }
        let view = core
            .publish_road_tool_view()
            .expect("a current roadbed")
            .view;
        Self { core, view }
    }
}

fn surface_point(core: &SimCore, p: Vector2) -> Vector3 {
    let y = core.heightmap.sample_height_world(p.x, p.y) * crate::config::HEIGHT_SCALE;
    Vector3::new(p.x, y, p.y)
}

// One pose's preview and the hue the session builds of it.
struct PoseHue {
    preview: RoadPreview,
}

impl PoseHue {
    fn of(world: &RowWorld, context: &mut RoadPreviewContext, row: &Row, u: f32) -> Self {
        let points = vec![
            surface_point(&world.core, row.start),
            surface_point(&world.core, row.pointer.at(u)),
        ];
        let preview = context.preview_road(RoadPreviewRequest {
            surface_generation: world.view.surface_generation,
            points,
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            profile: RoadProfile::FollowGround,
            median: None,
            parking: RoadParking::NONE,
            attributes: RoadAttributes::DEFAULT,
        });
        Self { preview }
    }

    // `road_tool_hue_of_preview`'s build; with `graded` off the drawn ground is a constant
    // below everything (the build without its read).
    fn build(&self, world: &RowWorld, hue: &mut FootprintHue, graded: bool) {
        self.build_with(world, hue, |surface, scratch, hue, ground| {
            if graded {
                hue.finish(
                    HALF_WIDTH_M,
                    |x, z| ground.height_at(x, z),
                    graded_ground(surface, scratch),
                    HueState::Valid,
                );
            } else {
                hue.finish(
                    HALF_WIDTH_M,
                    |x, z| ground.height_at(x, z),
                    |_, _| -1.0e3,
                    HueState::Valid,
                );
            }
        });
    }

    // The session's pushes, then `finish` inside the view's graded surface.
    fn build_with(
        &self,
        world: &RowWorld,
        hue: &mut FootprintHue,
        finish: impl FnOnce(
            &crate::simulation::terrain::graded::GradedSurface<'_>,
            &mut crate::simulation::terrain::graded::QueryScratch,
            &mut FootprintHue,
            &DraftGround<'_>,
        ),
    ) {
        let prepared = self.preview.plan.prepared();
        let view = &world.view;
        let ground = DraftGround::new(&view.terrain, &view.roadbed);
        let kind = HueKind::of(RoadProfile::FollowGround, prepared.class);
        hue.begin();
        hue.push_stroke(&prepared.points, kind);
        if kind == HueKind::Road
            && let Some(pieces) = &self.preview.pieces
        {
            hue.push_pieces(
                (pieces.delta.pieces())
                    .filter_map(|(_, mesh)| mesh)
                    .filter(|mesh| mesh.grounded() && !mesh.buried()),
            );
        }
        view.with_graded_surface(|surface, scratch| finish(surface, scratch, hue, &ground));
    }
}

// Round HU3's reader (one `gather_near` per 8 m block, then `ground_height_near` at every read),
// the reference the lattice reader is bitwise equal to.
fn hu3_ground<'a>(
    surface: &'a crate::simulation::terrain::graded::GradedSurface<'a>,
    scratch: &'a mut crate::simulation::terrain::graded::QueryScratch,
) -> impl FnMut(f64, f64) -> f64 + 'a {
    const BLOCK_M: f64 = 8.0;
    let mut gathered: Option<[f64; 2]> = None;
    move |x, z| {
        let inside = gathered.is_some_and(|[x0, z0]| {
            (x0..=x0 + BLOCK_M).contains(&x) && (z0..=z0 + BLOCK_M).contains(&z)
        });
        if !inside {
            let [x0, z0] = [x, z].map(|value| (value / BLOCK_M).floor() * BLOCK_M);
            surface.gather_near(
                x0 + BLOCK_M * 0.5,
                z0 + BLOCK_M * 0.5,
                BLOCK_M * 0.5,
                scratch,
            );
            gathered = Some([x0, z0]);
        }
        surface.ground_height_near(x, z, scratch)
    }
}

// `ROAD-123`: on every row's poses the hue the session builds through the lattice reader is
// bitwise the hue built through round HU3's per-point reader (fill bytes and outline), and every
// pose has pieces, so the graded read is exercised. Sixteen poses a row in debug.
#[test]
fn the_lattice_reader_builds_hu3s_hue_bitwise_on_the_rows() {
    let (mut hue, mut reference) = (FootprintHue::default(), FootprintHue::default());
    for row in rows() {
        let world = RowWorld::build(&row);
        let mut context = RoadPreviewContext::capture(world.view.clone());
        for k in 0..16 {
            let pose = PoseHue::of(&world, &mut context, &row, (k as f32 + 0.5) / 16.0);
            assert!(
                pose.preview.pieces.is_some(),
                "{} pose {k}: no pieces",
                row.name
            );
            pose.build(&world, &mut hue, true);
            pose.build_with(&world, &mut reference, |surface, scratch, hue, ground| {
                hue.finish(
                    HALF_WIDTH_M,
                    |x, z| ground.height_at(x, z),
                    hu3_ground(surface, scratch),
                    HueState::Valid,
                )
            });
            assert!(hue.fill_used > 0, "{} pose {k}: an empty hue", row.name);
            assert_eq!(
                hue.fill_bytes, reference.fill_bytes,
                "{} pose {k}",
                row.name
            );
            assert_eq!(
                digest(&hue, 0),
                digest(&reference, 0),
                "{} pose {k}",
                row.name
            );
        }
    }
}

// FNV-1a over the hue's fill bytes (padding included) and its outline's bits.
fn digest(hue: &FootprintHue, mut hash: u64) -> u64 {
    let mut eat = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    };
    hue.fill_bytes.iter().for_each(|&byte| eat(byte));
    for point in &hue.outline {
        [point.x, point.y, point.z]
            .iter()
            .flat_map(|value| value.to_bits().to_le_bytes())
            .for_each(&mut eat);
    }
    hash
}

fn rows_with_poses() -> Vec<(Row, RowWorld, Vec<PoseHue>)> {
    // `METRUM_HUE_ROWS` (comma-separated names) keeps only those rows, for a profile.
    let only = std::env::var("METRUM_HUE_ROWS").unwrap_or_default();
    rows()
        .into_iter()
        .filter(|row| only.is_empty() || only.split(',').any(|name| name == row.name))
        .map(|row| {
            let world = RowWorld::build(&row);
            let mut context = RoadPreviewContext::capture(world.view.clone());
            let poses = (0..POSES)
                .map(|k| PoseHue::of(&world, &mut context, &row, (k as f32 + 0.5) / POSES as f32))
                .collect();
            (row, world, poses)
        })
        .collect()
}

// The hue build per pose on the rows' poses, release: per row the poses with pieces, the hue's
// triangles, its digest, and p50 / p99 / max over every pose of 9 rounds (one warm-up round
// first), with the drawn ground read and with a constant ground (the build without the read). The digest is the round's bitwise record: equal on the base and the branch.
#[test]
#[ignore = "controlled release timing: cargo test --release --lib benchmark_hue_rows -- --ignored --nocapture"]
fn benchmark_hue_rows() {
    // `METRUM_HUE_ROUNDS` sets the timed rounds (9), for a profile.
    let rounds = std::env::var("METRUM_HUE_ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(9);
    let mut hue = FootprintHue::default();
    for (row, world, poses) in rows_with_poses() {
        let mut hash = 0xcbf2_9ce4_8422_2325;
        let (mut with_pieces, mut triangles) = (0, 0);
        for pose in &poses {
            pose.build(&world, &mut hue, true);
            hash = digest(&hue, hash);
            with_pieces += usize::from(pose.preview.pieces.is_some());
            triangles += hue.fill_used;
        }
        let mut time = |graded: bool| {
            let mut times = Vec::with_capacity(poses.len() * rounds);
            for _ in 0..rounds {
                for pose in &poses {
                    let start = Instant::now();
                    pose.build(&world, &mut hue, graded);
                    times.push(start.elapsed().as_secs_f64() * 1.0e3);
                }
            }
            times.sort_by(f64::total_cmp);
            let at = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
            format!(
                "p50 {:.3} ms p99 {:.3} ms max {:.3} ms",
                at(0.5),
                at(0.99),
                at(1.0)
            )
        };
        let (graded, constant) = (time(true), time(false));
        println!(
            "hue-rows {:<14} poses {} pieces {} triangles/pose {} digest {:016x} {graded} | constant ground {constant}",
            row.name,
            poses.len(),
            with_pieces,
            triangles / poses.len(),
            hash,
        );
    }
}
