// SPDX-License-Identifier: GPL-3.0-or-later

//! `RENDER-69`: the parked cars' tier batches on a prepared city. Every parked chunk of the
//! prepared `METRUM_PERF_RESIDENTS` city, with the bundled vehicles pack's catalogue, is installed
//! into the parked layer against the recorder once a cell split (cells a side for tiers 0, 1 and
//! 2), and the report gives each split's slots, rows uploaded and install time, and, over cameras
//! standing among the parked cars at four heights, the triangles the engine's ranges send to the
//! vertex stage (every car of an admitted batch is shaded, the material drops those outside its
//! band), the batches admitted, the triangles the cars' own tiers draw, and what the layer's
//! old rule (one batch a chunk, the tier by the distance to the batch's centre) drew. No frustum
//! culling: every direction counts, so the numbers compare splits, not frames.

use super::super::super::frame::FRAME_DT_S;
use super::super::super::snapshot::RenderSnapshot;
use super::super::contract::{cache_dir, residents, timing_mode};
use super::vehicle_rows::install_bundled_catalogue;
use crate::config::CAR_DRAW_DISTANCE_M;
use crate::nodes::sim::render::car_visual::ROW;
use crate::nodes::sim::render::stream::backend::Scenario;
use crate::nodes::sim::render::stream::backend::recorder::Recorder;
use crate::nodes::sim::render::stream::layers::parked::{ChunkKey, ParkedLayer};
use crate::nodes::sim::render::vegetation_product::godot_math::{Aabb, length3, sub3};
use crate::nodes::sim::render::vehicle_catalogue::TIERS;
use std::collections::BTreeMap;
use std::time::Instant;

// The pilot models' triangles a tier (`docs/vehicles.md` "Cost"): the estate, then the truck.
const ESTATE: [f64; TIERS] = [2_990.0, 880.0, 216.0, 40.0];
const TRUCK: [f64; TIERS] = [4_490.0, 1_398.0, 348.0, 60.0];
// Camera heights over a parked car, metres, and the cars a camera stands by (every n-th).
const HEIGHTS: [f32; 4] = [2.0, 20.0, 60.0, 200.0];
const CAMERA_EVERY: usize = 97;

// One look's bands and triangles a tier.
struct LookTiers {
    bands: [[f32; 2]; TIERS],
    triangles: [f64; TIERS],
}

#[test]
#[ignore = "RENDER-69 parked tier batches on a prepared city; release"]
fn parked_tier_batches_on_a_prepared_city() {
    timing_mode();
    let mut city = super::super::snapshot::load_snapshot(residents(), &cache_dir());
    let core = &mut city.core;
    install_bundled_catalogue(core);
    core.parking.invalidate_all_ground();
    core.step_frame(FRAME_DT_S);
    let snapshot = core.build_snapshot_reusing(RenderSnapshot::default());
    // Each chunk's newest rows by look: pose, then custom data, as the bridge packs them.
    let mut chunks: BTreeMap<ChunkKey, Vec<(u32, Vec<f32>)>> = BTreeMap::new();
    for update in snapshot.parked.updates.iter() {
        let looks = update
            .chunk
            .rows()
            .map(|(look, poses, custom, _)| {
                let rows = poses
                    .as_chunks::<12>()
                    .0
                    .iter()
                    .zip(custom.as_chunks::<4>().0)
                    .flat_map(|(pose, data)| pose.iter().chain(data).copied())
                    .collect();
                (u32::from(look), rows)
            })
            .collect();
        chunks.insert(update.key, looks);
    }
    let catalogue = &core.vehicle_catalogue;
    let looks: Vec<LookTiers> = catalogue
        .looks
        .iter()
        .map(|look| {
            let model = &catalogue.models[usize::from(look.model)];
            let starts = model.tier_starts_m;
            LookTiers {
                bands: std::array::from_fn(|t| [starts[t], starts.get(t + 1).copied().unwrap_or(f32::INFINITY)]),
                triangles: if model.size[0] < 6.0 { ESTATE } else { TRUCK },
            }
        })
        .collect();
    let cars: usize = chunks.values().flatten().map(|(_, rows)| rows.len() / ROW).sum();
    let mut cameras = Vec::new();
    for (index, origin) in chunks
        .values()
        .flatten()
        .flat_map(|(_, rows)| rows.as_chunks::<ROW>().0.iter().map(|row| [row[3], row[7], row[11]]))
        .enumerate()
        .filter(|(index, _)| index % CAMERA_EVERY == 0)
    {
        let height = HEIGHTS[index / CAMERA_EVERY % HEIGHTS.len()];
        cameras.push([origin[0] + 5.0, origin[1] + height, origin[2] + 3.0]);
    }
    // A car's bounds: the bundled models' boxes, close enough for the ranges' centres.
    let bounds = Aabb::new([-1.0, 0.0, -2.5], [2.0, 1.6, 5.0]);
    let mut report = serde_json::Map::new();
    for cells in [[1u16, 1, 1], [4, 4, 1], [8, 8, 1], [16, 16, 1], [8, 4, 1], [16, 8, 1], [8, 8, 4], [8, 8, 8]] {
        let mut layer = ParkedLayer::new(1 << 20, 16, CAR_DRAW_DISTANCE_M);
        let mut backend = Recorder::new();
        for look in 0..looks.len() as u32 {
            for tier in 0..TIERS as u32 {
                let bucket = look * TIERS as u32 + tier;
                let band = looks[look as usize].bands[tier as usize];
                layer.register(bucket, bucket, bounds, band, cells.get(tier as usize).copied().unwrap_or(1));
            }
        }
        let started = Instant::now();
        let mut uploaded = 0usize;
        for (&key, chunk) in &chunks {
            let named: Vec<(u32, &[f32])> = chunk
                .iter()
                .flat_map(|(look, rows)| (0..TIERS as u32).map(move |tier| (look * TIERS as u32 + tier, &rows[..])))
                .collect();
            uploaded += named.iter().map(|(_, rows)| rows.len()).sum::<usize>();
            assert!(layer.install(&mut backend, key, named.into_iter()));
        }
        let install_ms = started.elapsed().as_secs_f64() * 1e3;
        // Each drawn batch: its look and tier, range, bounds' centre, cars' origins.
        let batches: Vec<_> = backend
            .visible(Scenario::Main)
            .map(|(_, instance)| {
                let multimesh = &backend.multimeshes[&instance.base];
                let bucket = multimesh.mesh.unwrap() as usize;
                let count = multimesh.visible_count.unwrap_or(multimesh.capacity) as usize;
                let origins: Vec<[f32; 3]> = multimesh.rows[..count * ROW]
                    .as_chunks::<ROW>()
                    .0
                    .iter()
                    .map(|row| [row[3], row[7], row[11]])
                    .collect();
                (bucket / TIERS, bucket % TIERS, instance.range, multimesh.aabb.center(), origins)
            })
            .collect();
        let (mut shaded, mut admitted, mut exact) = (0.0f64, 0usize, 0.0f64);
        for camera in &cameras {
            for (look, tier, range, centre, origins) in &batches {
                let look = &looks[*look];
                let distance = length3(sub3(*camera, *centre));
                if (range[1] > 0.0 && distance > range[1]) || (range[0] > 0.0 && distance < range[0]) {
                    continue;
                }
                admitted += 1;
                shaded += origins.len() as f64 * look.triangles[*tier];
                let band = look.bands[*tier];
                exact += origins
                    .iter()
                    .map(|&origin| length3(sub3(*camera, origin)))
                    .filter(|&d| d >= band[0] && d < band[1] && d < CAR_DRAW_DISTANCE_M)
                    .count() as f64
                    * look.triangles[*tier];
            }
        }
        let n = cameras.len() as f64;
        let mut entry = serde_json::json!({
            "slots": batches.len(),
            "uploaded_floats": uploaded,
            "install_ms": install_ms,
            "shaded_triangles_mean": shaded / n,
            "admitted_batches_mean": admitted as f64 / n,
            "drawn_triangles_mean": exact / n,
        });
        if cells == [1, 1, 1] {
            // The old rule on the same chunk batches: one tier a batch, by the centre's distance.
            let mut old = 0.0f64;
            for camera in &cameras {
                for (look, tier, _, centre, origins) in &batches {
                    let look = &looks[*look];
                    let band = look.bands[*tier];
                    let distance = length3(sub3(*camera, *centre));
                    let reach = origins.iter().map(|&o| length3(sub3(*centre, o))).fold(0.0f32, f32::max);
                    if distance >= band[0] && distance <= band[1].min(CAR_DRAW_DISTANCE_M + reach) {
                        old += origins.len() as f64 * look.triangles[*tier];
                    }
                }
            }
            entry["old_rule_triangles_mean"] = (old / n).into();
        }
        report.insert(format!("{}/{}/{}", cells[0], cells[1], cells[2]), entry);
    }
    eprintln!(
        "PARKED_TIERS {}",
        serde_json::json!({
            "residents": residents(),
            "chunks": chunks.len(),
            "cars": cars,
            "looks": looks.len(),
            "cameras": cameras.len(),
            "splits": report,
        })
    );
}
