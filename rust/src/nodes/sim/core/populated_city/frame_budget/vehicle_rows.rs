// SPDX-License-Identifier: GPL-3.0-or-later

//! `RENDER-66`: the drawn car rows uploaded a frame on a prepared city, with the brake lamps,
//! indicators, reverse lamp, steer and roll, against the same frames drawn as stage 1a drew
//! them (steer and roll zero, the lamp word "lights on" only), through the agents' frame job;
//! and the city's lane cars on their gate turn (`PARK-31`) with the gate test's time. Also the
//! hashes of every snapshot's drawn products, the proof that a performance change draws the same.

use std::collections::HashMap;

use super::super::super::frame::FRAME_DT_S;
use super::super::super::snapshot::RenderSnapshot;
use super::super::super::state::SimCore;
use super::super::contract::{cache_dir, residents, timing_mode};
use crate::assets::{AssetRegistry, scan_pack_dir};
use crate::nodes::sim::render::agent_frame::{AgentFrame, CAR_ROW, FrameView};
use crate::nodes::sim::render::car_visual::CarVisualSmoother;
use crate::nodes::sim::render::vehicle_catalogue::VehicleCatalogue;
use godot::prelude::{Vector2, Vector3};

#[derive(Default)]
struct Counts {
    frames: usize,
    cars: usize,
    // Buckets and rows uploaded: 1b, then as 1a.
    buckets: [usize; 2],
    rows: [usize; 2],
    // Drawn cars with the brake, an indicator, the reverse lamp, a steer.
    brake: usize,
    indicator: usize,
    reverse: usize,
    steer: usize,
    // `PARK-31`: the city's lane cars, those on their gate turn, and the gate test's time over
    // them, nanoseconds.
    lane_cars: usize,
    gate_cars: usize,
    gate_test_ns: u128,
}

// `PARK-31`: every visible car on a road lane in the city, its lane pose sampled as the snapshot
// does; then the gate turn's test over them, timed alone. Returns (lane cars, on the turn, ns).
fn gate_turns(core: &SimCore, poses: &mut Vec<(usize, Vector3, Vector3)>) -> (usize, usize, u128) {
    use crate::nodes::sim::render::lane_pose::sample_lane_pose;
    use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_NETWORK};
    poses.clear();
    let lanes = &core.transit_network.lane_system.lanes;
    for i in 0..core.agents.len() {
        if core.agents.transit_mode[i] != MODE_CAR || core.agents.transit[i] != TRANSIT_NETWORK {
            continue;
        }
        if let Some((pos, tangent)) = lanes
            .get(core.agents.current_lane_id[i])
            .and_then(|lane| sample_lane_pose(lane, core.agents.lane_distance[i]))
        {
            poses.push((i, pos, tangent));
        }
    }
    let start = std::time::Instant::now();
    let on_turn = poses
        .iter()
        .filter(|&&(i, pos, tangent)| {
            super::super::super::snapshot::lane_car_gate_pose(core, i, false, pos, tangent).is_some()
        })
        .count();
    (poses.len(), on_turn, start.elapsed().as_nanos())
}

// The snapshot's cars as stage 1a gave them: no steer, no wheel radius, every moving car's word
// "lights on" with its paint and blink.
fn as_stage_1a(snapshot: &RenderSnapshot, into: &mut RenderSnapshot) {
    into.car_transforms.clone_from(&snapshot.car_transforms);
    into.car_render_ids.clone_from(&snapshot.car_render_ids);
    into.car_ground_flags.clone_from(&snapshot.car_ground_flags);
    into.vehicle_ground_key = snapshot.vehicle_ground_key;
    into.vehicle_catalogue_revision = snapshot.vehicle_catalogue_revision;
    into.car_custom.clone_from(&snapshot.car_custom);
    for custom in into.car_custom.values_mut() {
        for car in custom.as_chunks_mut::<4>().0 {
            car[0] = 0.0;
            car[2] = ((car[2] as u32 & !0xff) | 1) as f32;
        }
    }
}

// One frame job over `snapshot` as the game runs it; the uploaded buckets and their rows.
fn draw(frame: &mut AgentFrame, snapshot: &RenderSnapshot, core: &SimCore) -> (usize, usize) {
    frame.take_snapshot(snapshot);
    frame.view = FrameView {
        alpha: CarVisualSmoother::alpha(FRAME_DT_S),
        camera: core
            .render_camera_position
            .map(|camera| [camera.x, camera.y]),
        eye: None,
        candidate_radius_m: 0.0,
        watch: -1,
    };
    frame.smooth_cars();
    let camera = core.render_camera_position;
    frame.smoother.solve_pending(|slot, poses, solve| {
        core.ground_vehicle_transforms(core.vehicle_bucket_model(slot), poses, solve, camera)
    });
    frame.finish();
    let rows = frame
        .smoother
        .changed()
        .iter()
        .map(|&key| frame.smoother.drawn_bucket(key).len() / CAR_ROW)
        .sum();
    (frame.smoother.changed().len(), rows)
}

// Installs the bundled vehicles pack's catalogue on `core`, as the game's pack load does.
pub(super) fn install_bundled_catalogue(core: &mut SimCore) {
    let mods = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../godot/bootstrap/mods");
    let mut registry = AssetRegistry::new();
    for pack in scan_pack_dir(&mods).packs {
        for (manifest, directory) in pack.assets {
            registry.register(&pack.pack.pack_id, manifest, directory);
        }
    }
    core.vehicle_catalogue = VehicleCatalogue::from_registry(&registry);
    core.vehicle_catalogue.revision = 1;
    core.vehicle_ground_support = core.vehicle_catalogue.ground_supports();
    core.vehicle_ground_generation += 1;
}

fn frames_from_env(default: usize) -> usize {
    std::env::var("METRUM_ROWS_FRAMES")
        .map(|value| {
            value
                .parse()
                .expect("METRUM_ROWS_FRAMES must be an integer")
        })
        .unwrap_or(default)
}

// A hash of every drawn product of `snapshot`'s agents: each non-empty car bucket's key, row
// bits, custom data bits, render ids and ground flags, each walker bucket's rows, the draw cut
// counts, and the parked chunks published since serial `parked_seen` (their rows and custom
// data). Returns (hash, car instances).
fn drawn_products_hash(snapshot: &RenderSnapshot, parked_seen: &mut u64) -> (u64, usize) {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    let mut cars = 0;
    let mut keys: Vec<u8> = snapshot.car_transforms.keys().copied().collect();
    keys.sort_unstable();
    for key in keys {
        let rows = &snapshot.car_transforms[&key];
        if rows.is_empty() {
            continue;
        }
        cars += rows.len() / 12;
        key.hash(&mut hasher);
        rows.iter().for_each(|v| v.to_bits().hash(&mut hasher));
        snapshot.car_custom[&key]
            .iter()
            .for_each(|v| v.to_bits().hash(&mut hasher));
        snapshot.car_render_ids[&key].hash(&mut hasher);
        snapshot.car_ground_flags[&key].hash(&mut hasher);
    }
    let mut keys: Vec<u8> = snapshot.pedestrian_transforms.keys().copied().collect();
    keys.sort_unstable();
    for key in keys {
        let rows = &snapshot.pedestrian_transforms[&key];
        if !rows.is_empty() {
            key.hash(&mut hasher);
            rows.iter().for_each(|v| v.to_bits().hash(&mut hasher));
        }
    }
    snapshot.agent_draw_cut.hash(&mut hasher);
    for update in snapshot.parked.updates.iter() {
        if update.serial <= *parked_seen {
            continue;
        }
        update.key.hash(&mut hasher);
        for (bucket, poses, custom, identities) in update.chunk.rows() {
            bucket.hash(&mut hasher);
            poses.iter().for_each(|v| v.to_bits().hash(&mut hasher));
            custom.iter().for_each(|v| v.to_bits().hash(&mut hasher));
            identities.hash(&mut hasher);
        }
    }
    *parked_seen = snapshot.parked.serial;
    (hasher.finish(), cars)
}

/// Behaviour-preserving proof of the snapshot's drawn products (`RENDER-65`/`66` performance
/// round): `METRUM_ROWS_FRAMES` frames (default 300) at 32x of the prepared
/// `METRUM_PERF_RESIDENTS` city, each frame's snapshot built from four views (the `PERF-07`
/// camera with no eye, a street eye, a district eye, play height over the centre), first with the
/// catalogue the city loads (the bundled pack's, as `PERF-07` runs), then with none (the
/// procedural fallback, every car in one bucket). Prints every snapshot's
/// hash of its drawn products; two builds draw the same when the `VEHICLE_PRODUCTS` lines match.
#[test]
#[ignore = "drawn product hashes on a prepared city; release"]
fn vehicle_products_on_a_prepared_city() {
    timing_mode();
    let frames = frames_from_env(300);
    let mut city = super::super::snapshot::load_snapshot(residents(), &cache_dir());
    let core = &mut city.core;
    core.time.speed_multiplier = 32.0;
    let views: [(&str, f32, Option<Vector3>); 4] = [
        ("perf07", 1_000.0, None),
        ("street", 150.0, Some(Vector3::new(0.0, 25.0, 45.0))),
        ("district", 1_000.0, Some(Vector3::new(0.0, 300.0, 300.0))),
        ("play", 1_000.0, Some(Vector3::new(0.0, 300.0, 0.0))),
    ];
    let mut snapshots: Vec<RenderSnapshot> =
        views.iter().map(|_| RenderSnapshot::default()).collect();
    let mut parked_seen = vec![0_u64; views.len()];
    let mut results = serde_json::Map::new();
    for catalogue in ["city", "empty"] {
        if catalogue == "empty" {
            let revision = core.vehicle_catalogue.revision + 1;
            core.vehicle_catalogue = VehicleCatalogue::default();
            core.vehicle_catalogue.revision = revision;
            core.vehicle_ground_support = core.vehicle_catalogue.ground_supports();
            core.vehicle_ground_generation += 1;
            core.parking.invalidate_all_ground();
        }
        let mut hashes = vec![Vec::with_capacity(frames); views.len()];
        let mut cars = vec![0_usize; views.len()];
        for _ in 0..frames {
            core.step_frame(FRAME_DT_S);
            for (v, &(_, half, eye)) in views.iter().enumerate() {
                core.camera_aabb = (-half, half, -half, half);
                core.render_camera_position = Some(Vector2::ZERO);
                core.agent_draw_eye = eye;
                snapshots[v] = core.build_snapshot_reusing(std::mem::take(&mut snapshots[v]));
                super::acknowledge_uploads(core, &snapshots[v]);
                let (hash, drawn) = drawn_products_hash(&snapshots[v], &mut parked_seen[v]);
                hashes[v].push(format!("{hash:016x}"));
                cars[v] += drawn;
            }
        }
        for (v, &(view, ..)) in views.iter().enumerate() {
            results.insert(
                format!("{catalogue}/{view}"),
                serde_json::json!({ "cars": cars[v], "hashes": hashes[v] }),
            );
        }
    }
    eprintln!(
        "VEHICLE_PRODUCTS {}",
        serde_json::json!({ "residents": residents(), "frames": frames, "views": results })
    );
}

/// The rows a populated city's car frames upload with `RENDER-66`'s values and as stage 1a drew
/// them, over `METRUM_ROWS_FRAMES` frames (default 1,200, 20 s at normal speed) from a street
/// view (eye 25 m up, a 300 m box) and a district view (300 m up, 2 km): the prepared
/// `METRUM_PERF_RESIDENTS` city with the bundled vehicles pack's catalogue.
#[test]
#[ignore = "RENDER-66 rows uploaded a frame on a prepared city; release"]
fn vehicle_rows_uploaded_on_a_prepared_city() {
    timing_mode();
    let frames = frames_from_env(1_200);
    let mut city = super::super::snapshot::load_snapshot(residents(), &cache_dir());
    let core = &mut city.core;
    install_bundled_catalogue(core);
    core.time.speed_multiplier = 1.0;
    let mut results = serde_json::Map::new();
    for (view, height, half) in [("street", 25.0, 150.0), ("district", 300.0, 1_000.0)] {
        core.camera_aabb = (-half, half, -half, half);
        core.render_camera_position = Some(Vector2::ZERO);
        core.agent_draw_eye = Some(Vector3::new(0.0, height, half * 0.3));
        let (mut frame, mut stage_1a) = (AgentFrame::default(), AgentFrame::default());
        let (mut snapshot, mut old) = (RenderSnapshot::default(), RenderSnapshot::default());
        let mut counts = Counts::default();
        let mut lane_poses = Vec::new();
        for index in 0..frames {
            core.step_frame(FRAME_DT_S);
            snapshot = core.build_snapshot_reusing(snapshot);
            as_stage_1a(&snapshot, &mut old);
            let (buckets, rows) = draw(&mut frame, &snapshot, core);
            let (old_buckets, old_rows) = draw(&mut stage_1a, &old, core);
            // The first frames upload everything either way.
            if index < 10 {
                continue;
            }
            counts.frames += 1;
            counts.buckets[0] += buckets;
            counts.rows[0] += rows;
            counts.buckets[1] += old_buckets;
            counts.rows[1] += old_rows;
            let (lane_cars, gate_cars, gate_ns) = gate_turns(core, &mut lane_poses);
            counts.lane_cars += lane_cars;
            counts.gate_cars += gate_cars;
            counts.gate_test_ns += gate_ns;
            for custom in snapshot.car_custom.values() {
                for car in custom.as_chunks::<4>().0 {
                    let lamps = car[2] as u32;
                    counts.cars += 1;
                    counts.brake += usize::from(lamps & 2 != 0);
                    counts.indicator += usize::from(lamps & 12 != 0);
                    counts.reverse += usize::from(lamps & 16 != 0);
                    counts.steer += usize::from(car[0] != 0.0);
                }
            }
        }
        let per_frame = |n: usize| n as f64 / counts.frames.max(1) as f64;
        let by_tier: HashMap<u8, usize> =
            snapshot
                .car_render_ids
                .iter()
                .fold(HashMap::new(), |mut tiers, (key, ids)| {
                    *tiers.entry(key % 4).or_default() += ids.len();
                    tiers
                });
        results.insert(
            view.into(),
            serde_json::json!({
                "frames": counts.frames,
                "cars_per_frame": per_frame(counts.cars),
                "last_frame_cars_by_tier": format!("{by_tier:?}"),
                "buckets_uploaded_per_frame": per_frame(counts.buckets[0]),
                "rows_uploaded_per_frame": per_frame(counts.rows[0]),
                "stage_1a_buckets_uploaded_per_frame": per_frame(counts.buckets[1]),
                "stage_1a_rows_uploaded_per_frame": per_frame(counts.rows[1]),
                "brake_per_frame": per_frame(counts.brake),
                "indicator_per_frame": per_frame(counts.indicator),
                "reverse_per_frame": per_frame(counts.reverse),
                "steering_per_frame": per_frame(counts.steer),
                "city_lane_cars_per_frame": per_frame(counts.lane_cars),
                "city_gate_turn_cars_per_frame": per_frame(counts.gate_cars),
                "gate_test_ns_per_lane_car": counts.gate_test_ns as f64 / counts.lane_cars.max(1) as f64,
            }),
        );
    }
    eprintln!(
        "VEHICLE_ROWS {}",
        serde_json::json!({ "residents": residents(), "frames": frames, "views": results })
    );
}
