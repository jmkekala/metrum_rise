// SPDX-License-Identifier: GPL-2.0-only

//! The culled, parallel render snapshot reproduces the serial full-pose snapshot bit for bit
//! (`SIM-02-K` part 2), on populated cities, frame after frame.

use std::collections::HashMap;
use std::time::Duration;

use super::super::super::frame::FRAME_DT_S;
use super::super::super::snapshot::RenderSnapshot;
use super::super::super::state::SimCore;
use super::super::PopulatedCity;
use super::super::contract::{cache_dir, residents, timing_mode};
use godot::prelude::{Vector2, Vector3};

use crate::simulation::economy::agents::{
    MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
    transit_is_visible,
};

type Camera = (f32, f32, f32, f32);

// Agents whose pose differs most from their stored position, or which have no lane pose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    LaneChange,
    Junction,
    Egress,
    Ingress,
    OffLaneCar,
}

const KINDS: [Kind; 5] = [
    Kind::LaneChange,
    Kind::Junction,
    Kind::Egress,
    Kind::Ingress,
    Kind::OffLaneCar,
];

fn kind(core: &SimCore, i: usize) -> Option<Kind> {
    let agents = &core.agents;
    let transit = agents.transit[i];
    let lanes = &core.transit_network.lane_system.lanes;
    let on_lane = agents.current_lane_id[i] < lanes.len();
    if agents.transit_mode[i] == MODE_CAR && !on_lane && transit_is_visible(transit) {
        return Some(Kind::OffLaneCar);
    }
    match transit {
        TRANSIT_ACCESS_EGRESS => Some(Kind::Egress),
        TRANSIT_ACCESS_INGRESS => Some(Kind::Ingress),
        TRANSIT_INTERSECTION => Some(Kind::Junction),
        TRANSIT_NETWORK
            if agents.transit_mode[i] == MODE_CAR
                && on_lane
                && (agents.lane_change_from_lane_id[i] as usize) < lanes.len()
                && agents.lane_change_length_m[i] > 1e-5
                && agents.lane_distance[i]
                    < agents.lane_change_start_d[i] + agents.lane_change_length_m[i] =>
        {
            Some(Kind::LaneChange)
        }
        _ => None,
    }
}

// Camera boxes for this frame: the city's centre third, the whole city, and boxes whose edge
// runs through the stored position of one agent of every kind, rotating with the frame.
fn cameras(core: &SimCore, frame: usize) -> (Vec<Camera>, HashMap<Kind, Vec<usize>>) {
    let mut by_kind: HashMap<Kind, Vec<usize>> = HashMap::new();
    for i in 0..core.agents.len() {
        if let Some(kind) = kind(core, i) {
            by_kind.entry(kind).or_default().push(i);
        }
    }
    let (mut min_x, mut min_z, mut max_x, mut max_z) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for building in &core.allocator.buildings {
        min_x = min_x.min(building.center_x);
        max_x = max_x.max(building.center_x);
        min_z = min_z.min(building.center_y);
        max_z = max_z.max(building.center_y);
    }
    let (third_x, third_z) = ((max_x - min_x) / 3.0, (max_z - min_z) / 3.0);
    let mut boxes = vec![
        (
            min_x + third_x,
            max_x - third_x,
            min_z + third_z,
            max_z - third_z,
        ),
        (min_x - 100.0, max_x + 100.0, min_z - 100.0, max_z + 100.0),
    ];
    for kind in KINDS {
        let Some(candidates) = by_kind.get(&kind) else {
            continue;
        };
        let agent = candidates[frame % candidates.len()];
        let (x, z) = (core.agents.pos_x[agent], core.agents.pos_y[agent]);
        boxes.push((x - 0.25, x + 60.0, z - 60.0, z + 60.0));
        boxes.push((x - 60.0, x + 60.0, z - 60.0, z + 0.25));
    }
    (boxes, by_kind)
}

fn non_empty<T: Clone>(buckets: &HashMap<u8, Vec<T>>) -> Vec<(u8, Vec<T>)> {
    let mut out: Vec<_> = buckets
        .iter()
        .filter(|(_, bucket)| !bucket.is_empty())
        .map(|(&key, bucket)| (key, bucket.clone()))
        .collect();
    out.sort_by_key(|(key, _)| *key);
    out
}

fn bits(buckets: &HashMap<u8, Vec<f32>>) -> Vec<(u8, Vec<u32>)> {
    non_empty(buckets)
        .into_iter()
        .map(|(key, bucket)| (key, bucket.iter().map(|v| v.to_bits()).collect()))
        .collect()
}

// Returns the number of instances after asserting both snapshots hold the same ones.
fn assert_same(context: &str, culled: &RenderSnapshot, full: &RenderSnapshot) -> usize {
    assert_eq!(
        bits(&culled.pedestrian_transforms),
        bits(&full.pedestrian_transforms),
        "pedestrians differ: {context}"
    );
    assert_eq!(
        bits(&culled.car_transforms),
        bits(&full.car_transforms),
        "car transforms differ: {context}"
    );
    assert_eq!(
        non_empty(&culled.car_render_ids),
        non_empty(&full.car_render_ids),
        "car identities differ: {context}"
    );
    assert_eq!(
        non_empty(&culled.car_ground_flags),
        non_empty(&full.car_ground_flags),
        "car ground flags differ: {context}"
    );
    culled
        .pedestrian_transforms
        .values()
        .map(Vec::len)
        .sum::<usize>()
        / 16
        + culled.car_render_ids.values().map(Vec::len).sum::<usize>()
}

#[derive(Default)]
struct Run {
    comparisons: usize,
    instances: usize,
    // Frames in which some agent of the kind stood inside a compared box.
    framed: HashMap<Kind, usize>,
    culled: Vec<Duration>,
    full: Vec<Duration>,
}

fn count_framed(
    core: &SimCore,
    camera: Camera,
    by_kind: &HashMap<Kind, Vec<usize>>,
    run: &mut Run,
) {
    let (x0, x1, z0, z1) = camera;
    for (&kind, agents) in by_kind {
        if agents.iter().any(|&i| {
            let (x, z) = (core.agents.pos_x[i], core.agents.pos_y[i]);
            x >= x0 && x <= x1 && z >= z0 && z <= z1
        }) {
            *run.framed.entry(kind).or_default() += 1;
        }
    }
}

// Compares both paths for every camera of `frames` frames at `speed`, recycling snapshots
// between frames as the simulation thread does.
fn compare_frames(core: &mut SimCore, frames: usize, speed: f32, run: &mut Run) {
    core.time.speed_multiplier = speed;
    let mut culled = RenderSnapshot::default();
    let mut full = RenderSnapshot::default();
    for frame in 0..frames {
        core.step_frame(FRAME_DT_S);
        let (boxes, by_kind) = cameras(core, frame);
        for (index, &camera) in boxes.iter().enumerate() {
            core.camera_aabb = camera;
            core.agent_instances_both_ways(&mut culled, &mut full);
            let context = format!(
                "speed {speed} frame {frame} camera {index} {camera:?} day {} minute {}",
                core.time.day_index, core.time.minute_of_day
            );
            run.instances += assert_same(&context, &culled, &full);
            run.comparisons += 1;
            count_framed(core, camera, &by_kind, run);
        }
    }
    // The default "show all" camera poses every visible agent.
    core.camera_aabb = (0.0, 0.0, 0.0, 0.0);
    core.agent_instances_both_ways(&mut culled, &mut full);
    run.instances += assert_same("show all", &culled, &full);
    run.comparisons += 1;
}

// Site answers and vehicle poses agree with the unfiltered, per-point queries: on a grid over
// the city's centre, at lot corners, and along every `stride`-th site's footprint edges, inside
// and outside the polygon test's 1 mm edge tolerance. Returns (points, points on a site).
fn assert_surface_queries_match(core: &SimCore, stride: usize) -> (usize, usize) {
    let allocator = &core.allocator;
    let mut points = Vec::new();
    for site in allocator.building_sites.iter().step_by(stride) {
        let polygon = &site.footprint_world;
        for k in 0..polygon.len() {
            let (a, b) = (polygon[k], polygon[(k + 1) % polygon.len()]);
            let edge = b - a;
            let length = edge.length();
            if length <= 1e-6 {
                points.push(a);
                continue;
            }
            let normal = Vector2::new(-edge.y, edge.x) / length;
            for t in [0.0, 0.31, 0.5, 1.0] {
                for offset in [-0.0015, -0.001, -0.0005, 0.0, 0.0005, 0.001, 0.0015, 0.3] {
                    points.push(a + edge * t + normal * offset);
                }
            }
        }
        points.extend(site.lot_footprint_world);
    }
    let centre = allocator
        .buildings
        .iter()
        .map(|building| Vector2::new(building.center_x, building.center_y))
        .sum::<Vector2>()
        / allocator.buildings.len().max(1) as f32;
    for row in 0..160 {
        for column in 0..160 {
            points.push(
                centre + Vector2::new(column as f32, row as f32) * 1.7 - Vector2::ONE * 136.0,
            );
        }
    }
    let answer_bits = |(height, blocks, planting): (Option<f32>, bool, bool)| {
        (height.map(f32::to_bits), blocks, planting)
    };
    let mut on_site = 0;
    for &point in &points {
        let [filtered, unfiltered] = allocator.site_answers_both_ways(point);
        assert_eq!(
            answer_bits(filtered),
            answer_bits(unfiltered),
            "site answers differ at {point:?}"
        );
        on_site += usize::from(filtered.0.is_some());
    }
    let headings = [
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 1.0),
        Vector3::new(0.6, 0.0, -0.8),
        Vector3::new(-0.28, 0.0, 0.96),
    ];
    let pose_bits = |(height, basis): (f32, [Vector3; 3])| {
        let mut bits = vec![height.to_bits()];
        for axis in basis {
            bits.extend([axis.x.to_bits(), axis.y.to_bits(), axis.z.to_bits()]);
        }
        bits
    };
    for (k, &point) in points.iter().enumerate().step_by(3) {
        let heading = headings[k % headings.len()];
        assert_eq!(
            pose_bits(core.vehicle_ground_pose(0, point, heading)),
            pose_bits(core.vehicle_ground_pose_per_point(0, point, heading)),
            "vehicle pose differs at {point:?} heading {heading:?}"
        );
    }
    (points.len(), on_site)
}

#[test]
#[ignore = "SIM-02-K render snapshot equivalence on a populated city; release only"]
fn culled_snapshot_matches_full_pose_on_a_populated_city() {
    timing_mode();
    let mut city = PopulatedCity::build(2_000);
    let core = &mut city.core;
    let mut run = Run::default();
    compare_frames(core, 300, 1.0, &mut run);
    compare_frames(core, 300, 32.0, &mut run);
    let (points, on_site) = assert_surface_queries_match(core, 1);
    eprintln!(
        "K2_SNAPSHOT_EQUIVALENCE comparisons={} instances={} framed={:?} surface_points={points} on_site={on_site}",
        run.comparisons, run.instances, run.framed
    );
    assert!(on_site > 0, "no compared point lay on a building site");
    // No car of this fixture changes lanes in these frames; the core tests cover lane changes.
    for kind in [
        Kind::Junction,
        Kind::Egress,
        Kind::Ingress,
        Kind::OffLaneCar,
    ] {
        assert!(
            run.framed.get(&kind).copied().unwrap_or(0) > 0,
            "no {kind:?} agent stood in a compared camera"
        );
    }
}

fn percentile_ms(samples: &mut [Duration], fraction: f64) -> f64 {
    samples.sort();
    let rank = ((fraction * samples.len() as f64).ceil() as usize).clamp(1, samples.len());
    samples[rank - 1].as_secs_f64() * 1000.0
}

fn mean_ms(samples: &[Duration]) -> f64 {
    samples.iter().map(Duration::as_secs_f64).sum::<f64>() * 1000.0 / samples.len() as f64
}

/// Diagnostic and proof at scale on a prepared `PERF-07` snapshot (`METRUM_PERF_RESIDENTS`,
/// `METRUM_PERF_CACHE`): `METRUM_K2_FRAMES` frames at `METRUM_PERF07_SPEED`, every frame
/// compared under the `PERF-07` camera and the straddling cameras, plus the agent-instance
/// time of both paths under the `PERF-07` camera in a second pass. The full-pose path runs
/// second on warm caches, so its time is a lower bound.
#[test]
#[ignore = "SIM-02-K render snapshot equivalence and agent-instance timing on a prepared snapshot"]
fn culled_snapshot_matches_full_pose_on_a_prepared_city() {
    timing_mode();
    let frames: usize = std::env::var("METRUM_K2_FRAMES")
        .map(|value| value.parse().expect("METRUM_K2_FRAMES must be an integer"))
        .unwrap_or(600);
    let speed: f32 = std::env::var("METRUM_PERF07_SPEED")
        .map(|value| value.parse().expect("METRUM_PERF07_SPEED must be a number"))
        .unwrap_or(1.0);
    let mut city = super::super::snapshot::load_snapshot(residents(), &cache_dir());
    let core = &mut city.core;
    let mut run = Run::default();
    compare_frames(core, frames, speed, &mut run);
    let (points, on_site) = assert_surface_queries_match(core, 97);
    let mut timed = Run::default();
    // The PERF-07 camera box.
    core.camera_aabb = (-1000.0, 1000.0, -1000.0, 1000.0);
    compare_frames_timed(core, frames, speed, &mut timed);
    eprintln!(
        "K2_SNAPSHOT_PREPARED {}",
        serde_json::json!({
            "residents": residents(), "speed": speed, "frames": frames,
            "comparisons": run.comparisons + timed.comparisons,
            "instances": run.instances, "framed": format!("{:?}", run.framed),
            "surface_points": points, "surface_points_on_site": on_site,
            "perf07_camera_instances_per_frame":
                timed.instances as f64 / timed.comparisons.max(1) as f64,
            "culled_mean_ms": mean_ms(&timed.culled),
            "culled_p99_ms": percentile_ms(&mut timed.culled, 0.99),
            "full_pose_mean_ms": mean_ms(&timed.full),
            "full_pose_p99_ms": percentile_ms(&mut timed.full, 0.99),
        })
    );
}

fn compare_frames_timed(core: &mut SimCore, frames: usize, speed: f32, run: &mut Run) {
    core.time.speed_multiplier = speed;
    let camera = core.camera_aabb;
    let mut culled = RenderSnapshot::default();
    let mut full = RenderSnapshot::default();
    for frame in 0..frames {
        core.step_frame(FRAME_DT_S);
        core.camera_aabb = camera;
        let (culled_time, full_time) = core.agent_instances_both_ways(&mut culled, &mut full);
        run.culled.push(culled_time);
        run.full.push(full_time);
        run.instances += assert_same(&format!("timed frame {frame}"), &culled, &full);
        run.comparisons += 1;
    }
}
