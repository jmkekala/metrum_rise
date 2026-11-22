// SPDX-License-Identifier: GPL-3.0-or-later

//! The parked layer against the recorder: a chunk drawn as its bucket nodes drew it, a chunk
//! updated whole, a full pool refusing a chunk without sending anything, the teardown that
//! frees every object the layer made, the cars' draw distance (`RENDER-48`) as the engine
//! applies a bucket's visibility range, and each car's tier (`RENDER-69`): the batch ranges
//! never cut a car in its tier's band, and the near tiers' cells.

use super::super::super::backend::Scenario;
use super::super::super::backend::recorder::{RecordedInstance, Recorder};
use super::*;

fn poses(count: usize, x: f32) -> Vec<f32> {
    (0..count)
        .flat_map(|i| {
            [1.0, 0.0, 0.0, x + i as f32, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 0.0, 0.0, 0.0, 0.0]
        })
        .collect()
}

// A car's bounds: 2 m wide, 1.5 m high, 4.5 m long, standing on its origin.
const CAR: Aabb = Aabb {
    position: [-1.0, 0.0, -2.25],
    size: [2.0, 1.5, 4.5],
};

fn layer() -> ParkedLayer {
    let mut layer = ParkedLayer::new(64, 16, CAR_DRAW_DISTANCE_M);
    layer.register(10, 1, CAR, [0.0, f32::INFINITY], 1);
    layer.register(21, 2, CAR, [0.0, f32::INFINITY], 1);
    layer
}

// Every visible main instance: (mesh, its poses), sorted.
fn drawn(backend: &Recorder) -> Vec<(u32, Vec<f32>)> {
    let mut out: Vec<_> = backend
        .visible(Scenario::Main)
        .map(|(_, instance)| {
            let multimesh = &backend.multimeshes[&instance.base];
            let count = multimesh.visible_count.unwrap_or(multimesh.capacity) as usize;
            (multimesh.mesh.unwrap(), multimesh.rows[..count * STRIDE].to_vec())
        })
        .collect();
    out.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out
}

#[test]
fn a_chunk_draws_each_bucket_as_its_node_did() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let (a, b) = (poses(3, 0.0), poses(5, 100.0));
    assert!(layer.install(&mut backend, (0, 0), [(10, &a[..]), (21, &b[..])].into_iter()));
    assert_eq!(drawn(&backend), vec![(1, a.clone()), (2, b.clone())]);
    for (_, instance) in backend.visible(Scenario::Main) {
        assert_eq!(instance.shadow, Shadow::On);
        assert_eq!(instance.extra_margin, 12.0);
        assert_eq!(instance.material, None);
        assert_eq!(instance.origin, [0.0; 3]);
        assert!(!instance.dynamic_gi);
        let multimesh = &backend.multimeshes[&instance.base];
        // Instance custom data carries each car's paint (`RENDER-65`).
        assert!(!multimesh.colors && multimesh.custom);
        let rows = &multimesh.rows[..multimesh.visible_count.unwrap() as usize * STRIDE];
        assert_eq!(multimesh.aabb, CAR.through_instances(rows, rows.len() / STRIDE), "the engine's bounds of the poses");
    }
    assert_eq!(backend.visible(Scenario::Reflection).count(), 0);
}

#[test]
fn a_chunk_update_hides_the_buckets_that_left_and_reuses_its_objects() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let (a, b, c) = (poses(3, 0.0), poses(5, 100.0), poses(4, 50.0));
    layer.install(&mut backend, (0, 0), [(10, &a[..]), (21, &b[..])].into_iter());
    let objects = backend.multimeshes.len();
    // Bucket 21 leaves (absent), bucket 10 changes its poses.
    assert!(layer.install(&mut backend, (0, 0), [(10, &c[..])].into_iter()));
    assert_eq!(drawn(&backend), vec![(1, c.clone())]);
    // Bucket 21 again in another chunk: the spare slot, no new object.
    layer.install(&mut backend, (1, 0), [(21, &b[..]), (10, &[][..])].into_iter());
    assert_eq!(backend.multimeshes.len(), objects);
    assert_eq!(drawn(&backend), vec![(1, c.clone()), (2, b.clone())]);
    // An empty install hides the chunk.
    layer.install(&mut backend, (0, 0), std::iter::empty());
    assert_eq!(drawn(&backend), vec![(2, b)]);
}

#[test]
fn a_chunk_the_full_pool_cannot_take_is_refused_whole_until_room_frees() {
    let mut backend = Recorder::new();
    let mut layer = ParkedLayer::new(2, 2, CAR_DRAW_DISTANCE_M);
    layer.register(10, 1, CAR, [0.0, f32::INFINITY], 1);
    layer.register(21, 2, CAR, [0.0, f32::INFINITY], 1);
    let (a, b, c) = (poses(3, 0.0), poses(5, 100.0), poses(2, 7.0));
    layer.install(&mut backend, (0, 0), [(10, &a[..]), (21, &b[..])].into_iter());
    let before = (backend.commands.len(), drawn(&backend));
    assert!(!layer.install(&mut backend, (1, 0), [(10, &c[..])].into_iter()));
    assert_eq!((backend.commands.len(), drawn(&backend)), before, "nothing of it sent");
    // A bucket outgrowing its slot moves within its own room, even in a full pool.
    let big = poses(40, 0.0);
    assert!(layer.install(&mut backend, (0, 0), [(10, &big[..]), (21, &b[..])].into_iter()));
    assert_eq!(drawn(&backend), vec![(1, big), (2, b)]);
    // The first chunk leaves: the refused one fits.
    layer.install(&mut backend, (0, 0), std::iter::empty());
    assert!(layer.install(&mut backend, (1, 0), [(10, &c[..])].into_iter()));
    assert_eq!(drawn(&backend), vec![(1, c)]);
}

#[test]
fn a_reset_frees_every_object_the_layer_made() {
    // The teardown (`RENDER-41`'s lesson): a world replaced or the stream node leaving frees
    // the pool's every multimesh and instance, spares included.
    let mut backend = Recorder::new();
    let mut layer = layer();
    let (a, b) = (poses(3, 0.0), poses(5, 100.0));
    layer.install(&mut backend, (0, 0), [(10, &a[..]), (21, &b[..])].into_iter());
    layer.install(&mut backend, (1, 1), [(10, &b[..])].into_iter());
    layer.install(&mut backend, (0, 0), std::iter::empty());
    assert!(layer.pool().spares() > 0 && layer.pool().live() > 0);
    layer.reset(&mut backend);
    assert!(backend.multimeshes.is_empty(), "every multimesh freed");
    assert!(backend.instances.is_empty(), "every instance freed");
    assert_eq!((layer.pool().live(), layer.pool().spares()), (0, 0));
    assert_eq!(layer.drawn().count(), 0);
}

#[test]
fn the_cast_toggle_reaches_drawn_and_later_chunks() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let (a, b) = (poses(3, 0.0), poses(5, 100.0));
    assert!(layer.install(&mut backend, (0, 0), [(10, &a[..])].into_iter()));
    layer.set_cast_shadows(&mut backend, false);
    assert!(layer.install(&mut backend, (1, 0), [(21, &b[..])].into_iter()));
    assert_eq!(backend.visible(Scenario::Main).count(), 2, "the toggle hides nothing");
    assert!(backend.visible(Scenario::Main).all(|(_, car)| car.shadow == Shadow::Off));
    layer.set_cast_shadows(&mut backend, true);
    assert!(backend.visible(Scenario::Main).all(|(_, car)| car.shadow == Shadow::On));
}

// Whether the engine draws a batch for a camera at `camera`, in the main and the shadow passes
// alike: `RendererSceneCull::_visibility_range_check` (Godot 4.7) with no margins, measured from
// the centre of the batch's bounds (its instance stands at the origin).
fn engine_draws(backend: &Recorder, instance: &RecordedInstance, camera: [f32; 3]) -> bool {
    let centre = backend.multimeshes[&instance.base].aabb.center();
    let [begin, end] = instance.range;
    let distance = length3(sub3(camera, centre));
    !(end > 0.0 && distance > end) && !(begin > 0.0 && distance < begin)
}

#[test]
fn a_bucket_draws_while_any_of_its_cars_is_within_the_draw_distance() {
    // The cars stand from x = 0 to 300 on a row and from z = 0 to 30 across it.
    let rows: Vec<f32> = (0..40)
        .flat_map(|i| {
            let (x, z) = ((i % 10) as f32 * 33.3, (i / 10) as f32 * 10.0);
            [0.0, 0.0, 1.0, x, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 0.0]
        })
        .collect();
    let mut backend = Recorder::new();
    let mut layer = layer();
    assert!(layer.install(&mut backend, (0, 0), [(10, &rows[..])].into_iter()));
    let (_, car) = backend.visible(Scenario::Main).next().expect("drawn");
    let car = car.clone();
    let origins: Vec<[f32; 3]> = rows.as_chunks::<STRIDE>().0.iter().map(|p| [p[3], p[7], p[11]]).collect();
    let nearest = |camera: [f32; 3]| {
        origins.iter().map(|&o| length3(sub3(camera, o))).fold(f32::INFINITY, f32::min)
    };
    // Cameras on rings about the bucket, from inside it to far beyond the draw distance.
    let mut cut = 0;
    let mut straddling = 0;
    for ring in 0..60 {
        let radius = 50.0 + 60.0 * ring as f32;
        for step in 0..64 {
            let angle = step as f32 * std::f32::consts::TAU / 64.0;
            let height = [2.0, 80.0, 900.0][step % 3];
            let camera = [150.0 + radius * angle.cos(), height, 15.0 + radius * angle.sin()];
            let drawn = engine_draws(&backend, &car, camera);
            let near = nearest(camera);
            if near < CAR_DRAW_DISTANCE_M {
                // A car not yet faded out whole: the batch must draw.
                assert!(drawn, "a car {near} m away is cut at {camera:?}");
            }
            if !drawn {
                cut += 1;
            }
            let far = origins.iter().map(|&o| length3(sub3(camera, o))).fold(0.0, f32::max);
            straddling += usize::from(drawn && near < CAR_DRAW_DISTANCE_M && far > CAR_DRAW_DISTANCE_M);
        }
    }
    assert!(cut > 0, "a bucket beyond the draw distance draws nothing");
    assert!(straddling > 0, "some poses straddle the draw distance (the material fades those cars)");
    // The range ends no farther out than the bucket's own extent requires.
    let centre = backend.multimeshes[&car.base].aabb.center();
    let reach = origins.iter().map(|&o| length3(sub3(centre, o))).fold(0.0, f32::max);
    assert!(car.range[1] >= CAR_DRAW_DISTANCE_M + reach);
    assert!(car.range[1] <= CAR_DRAW_DISTANCE_M + reach + 20.0, "{:?}", car.range);
    // A camera 1 m past the range from the centre, straight up: the batch is skipped.
    let above = [centre[0], centre[1] + car.range[1] + 1.0, centre[2]];
    assert!(!engine_draws(&backend, &car, above));
    assert!(nearest(above) > CAR_DRAW_DISTANCE_M);
}

#[test]
fn the_paint_layer_draws_pose_rows_without_custom_data() {
    // `PARK-30` beside `RENDER-65`: a paint line's row is its pose alone, so the paint batch
    // counts 12 floats an instance and carries no custom data, while the cars' rows keep theirs.
    let line = Aabb {
        position: [-0.05, 0.0, -2.5],
        size: [0.1, 0.01, 5.0],
    };
    let rows: Vec<f32> = (0..6)
        .flat_map(|i| [1.0, 0.0, 0.0, i as f32 * 2.5, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0])
        .collect();
    let mut backend = Recorder::new();
    let mut layer = ParkedLayer::paint();
    layer.register(PAINT_BUCKET, 7, line, [0.0, f32::INFINITY], 1);
    assert!(layer.install(&mut backend, (0, 0), std::iter::once((PAINT_BUCKET, &rows[..]))));
    let (_, paint) = backend.visible(Scenario::Main).next().expect("drawn");
    assert_eq!(paint.shadow, Shadow::Off);
    let multimesh = &backend.multimeshes[&paint.base];
    assert!(!multimesh.colors && !multimesh.custom);
    assert_eq!(multimesh.visible_count, Some(6));
    assert_eq!(multimesh.rows[..rows.len()], rows[..]);
    assert_eq!(multimesh.aabb, line.through_instances(&rows, 6));
    // The range: the paint's draw distance plus the farthest line origin from the centre.
    let centre = multimesh.aabb.center();
    let reach = (0..6)
        .map(|i| length3(sub3(centre, [i as f32 * 2.5, 0.0, 3.0])))
        .fold(0.0f32, f32::max);
    assert!(paint.range[1] >= PAINT_DRAW_DISTANCE_M + reach - 1e-3);
    assert!(paint.range[1] <= PAINT_DRAW_DISTANCE_M + reach + 1.0, "{:?}", paint.range);
}

// The estate's tier bands (`tier_starts` of a 4.48 m body), each a mesh of its own: tier `t` is
// bucket `t` and mesh `t + 1`.
const BANDS: [[f32; 2]; 4] = [[0.0, 30.0], [30.0, 100.0], [100.0, 398.0], [398.0, f32::INFINITY]];

fn tier_layer(limit: usize) -> ParkedLayer {
    let mut layer = ParkedLayer::new(limit, 16, CAR_DRAW_DISTANCE_M);
    for (tier, band) in BANDS.into_iter().enumerate() {
        layer.register(tier as u32, tier as u32 + 1, CAR, band, tier_cells(tier as u32));
    }
    layer
}

// `count` cars spread over chunk (1, -1) (x 512 to 1,024, z -512 to 0) on a 7.3 m pitch, three
// of them a few metres past the chunk's edges, as a car whose bay is in the chunk can stand.
fn chunk_cars(count: usize) -> Vec<f32> {
    (0..count)
        .flat_map(|i| {
            let (x, z) = match i {
                0 => (510.0, -40.0),
                1 => (700.0, 3.0),
                2 => (1_026.0, -511.0),
                _ => (512.0 + (i * 73 % 700) as f32 * 0.731, -512.0 + (i * 31 % 70) as f32 * 7.31),
            };
            [1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, 0.5, 0.0, 0.0, 1.0, z, 0.0, 0.0, i as f32, 0.0]
        })
        .collect()
}

// Every drawn batch: its mesh (tier + 1), its instance and its cars' origins.
fn batches(backend: &Recorder) -> Vec<(u32, RecordedInstance, Vec<[f32; 3]>)> {
    backend
        .visible(Scenario::Main)
        .map(|(_, instance)| {
            let multimesh = &backend.multimeshes[&instance.base];
            let count = multimesh.visible_count.unwrap_or(multimesh.capacity) as usize;
            let origins = multimesh.rows[..count * STRIDE]
                .as_chunks::<STRIDE>()
                .0
                .iter()
                .map(|row| [row[3], row[7], row[11]])
                .collect();
            (multimesh.mesh.unwrap(), instance.clone(), origins)
        })
        .collect()
}

#[test]
fn every_car_draws_at_the_tier_of_its_own_distance_through_its_batch() {
    // `RENDER-69`: the material keeps a car only within its tier's band of the car's own camera
    // distance, so exactly one tier holds each car; the engine must draw the batch holding it in
    // that tier for every camera, in the main and the shadow passes alike.
    let rows = chunk_cars(600);
    let mut backend = Recorder::new();
    let mut layer = tier_layer(4_096);
    let named: Vec<(u32, &[f32])> = (0..4).map(|tier| (tier, &rows[..])).collect();
    assert!(layer.install(&mut backend, (1, -1), named.iter().copied()));
    let drawn = batches(&backend);
    // Every car once in each tier.
    for tier in 0..4u32 {
        let cars: usize = drawn.iter().filter(|b| b.0 == tier + 1).map(|b| b.2.len()).sum();
        assert_eq!(cars, 600, "tier {tier} holds every car once");
    }
    let mut cameras = Vec::new();
    for (cx, cz) in [(512.0, -512.0), (768.0, -256.0), (1_030.0, 10.0), (600.0, -100.0), (300.0, 200.0)] {
        for radius in [0.0, 3.0, 12.0, 29.0, 31.0, 55.0, 99.0, 101.0, 180.0, 397.0, 399.0, 700.0, 1_400.0, 2_600.0] {
            for step in 0..12 {
                let angle = step as f32 * std::f32::consts::TAU / 12.0;
                let height = [1.6, 25.0, 150.0][step % 3];
                cameras.push([cx + radius * angle.cos(), height, cz + radius * angle.sin()]);
            }
        }
    }
    let (mut checked, mut skipped) = (0, 0);
    for camera in cameras {
        for (mesh, instance, origins) in &drawn {
            let band = BANDS[*mesh as usize - 1];
            let admitted = engine_draws(&backend, instance, camera);
            skipped += usize::from(!admitted);
            for &origin in origins {
                let distance = length3(sub3(camera, origin));
                if distance >= band[0] && distance < band[1] && distance < CAR_DRAW_DISTANCE_M {
                    assert!(admitted, "a car {distance} m away is cut from tier {} at {camera:?}", mesh - 1);
                    checked += 1;
                }
            }
        }
        // Exactly one tier's band holds any distance.
        let distance = length3(sub3(camera, [700.0, 0.5, -300.0]));
        assert_eq!(BANDS.iter().filter(|b| distance >= b[0] && distance < b[1]).count(), 1);
    }
    assert!(checked > 10_000, "{checked} cars checked in their band");
    assert!(skipped > 0, "the ranges still skip batches none of whose cars can be in the band");
}

#[test]
fn the_near_tiers_split_a_chunk_by_cell_and_the_far_tiers_keep_one_batch() {
    let rows = chunk_cars(600);
    let mut backend = Recorder::new();
    let mut layer = tier_layer(4_096);
    let named: Vec<(u32, &[f32])> = (0..4).map(|tier| (tier, &rows[..])).collect();
    assert!(layer.install(&mut backend, (1, -1), named.iter().copied()));
    let drawn = batches(&backend);
    let size = 512.0 / f32::from(NEAR_CELLS);
    let cell = |o: [f32; 3]| ((o[0] / size).floor() as i64, (o[2] / size).floor() as i64);
    for tier in 0..4u32 {
        let tier_batches: Vec<_> = drawn.iter().filter(|b| b.0 == tier + 1).collect();
        if tier >= NEAR_TIERS {
            assert_eq!(tier_batches.len(), 1, "far tier {tier}: one batch a chunk");
            continue;
        }
        let cells = usize::from(NEAR_CELLS * NEAR_CELLS);
        assert!(tier_batches.len() > cells / 2, "near tier {tier}: {} cells", tier_batches.len());
        for (_, instance, origins) in tier_batches {
            // One cell of the world's grid a batch (modulo the chunk: the cars past its edges
            // join the cell on the far side), and its range no wider than the band plus a reach
            // of that.
            let (cx, cz) = cell(origins[0]);
            let side = i64::from(NEAR_CELLS);
            assert!(origins.iter().all(|&o| {
                let (x, z) = cell(o);
                (x - cx).rem_euclid(side) == 0 && (z - cz).rem_euclid(side) == 0
            }));
            let local = origins.iter().all(|&o| cell(o) == (cx, cz));
            if local {
                assert!(instance.range[1] <= BANDS[tier as usize][1] + size * 1.5 + 2.0, "{:?}", instance.range);
            }
        }
    }
    // The cars keep the chunk's order within a cell: a second install of the same rows sends
    // the same rows to the same slots.
    let before: Vec<_> = batches(&backend).into_iter().map(|b| (b.0, b.2)).collect();
    assert!(layer.install(&mut backend, (1, -1), named.iter().copied()));
    let after: Vec<_> = batches(&backend).into_iter().map(|b| (b.0, b.2)).collect();
    assert_eq!(before, after);
}

#[test]
fn a_split_chunk_installs_whole_or_not_at_all() {
    let rows = chunk_cars(600);
    let mut backend = Recorder::new();
    let mut probe = tier_layer(4_096);
    let named: Vec<(u32, &[f32])> = (0..4).map(|tier| (tier, &rows[..])).collect();
    assert!(probe.install(&mut backend, (1, -1), named.iter().copied()));
    let needed = probe.pool().live();
    // One slot short: nothing sent, the chunk refused; with the room it installs.
    let mut backend = Recorder::new();
    let mut layer = tier_layer(needed - 1);
    assert!(!layer.install(&mut backend, (1, -1), named.iter().copied()));
    assert!(backend.commands.is_empty() && layer.drawn().count() == 0, "nothing of it sent");
    let mut layer = tier_layer(needed);
    assert!(layer.install(&mut backend, (1, -1), named.iter().copied()));
    assert_eq!(layer.drawn().count(), needed);
    // The cars move into fewer cells: the chunk's batches that left free their slots.
    let few = chunk_cars(3);
    let named: Vec<(u32, &[f32])> = (0..4).map(|tier| (tier, &few[..])).collect();
    assert!(layer.install(&mut backend, (1, -1), named.iter().copied()));
    assert_eq!(layer.drawn().count(), 2 * 3 + 2, "three cells for each near tier, a batch for each far one");
    assert_eq!(backend.visible(Scenario::Main).count(), 8);
}
