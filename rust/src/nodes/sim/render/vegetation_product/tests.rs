// SPDX-License-Identifier: GPL-2.0-only

//! Godot's `hash()` against vectors from the engine, the product's buffer layouts, and the cost
//! of a dense patch. Bitwise parity with the GDScript upload is `vegetation_api`'s
//! `product_parity` test, which needs a world.

use super::godot_math::{hash_int, hash_vector2};
use super::*;

// `hash_golden.txt`: Godot 4.7.2's `hash()` of `Vector2` (`v <x bits> <y bits> <hash>`: both
// zeros, extremes, a subnormal, random world positions and quarter-metre grid points) and of
// `int` (`i <value> <hash>`: edges, then appearance seeds xor the four jitter salts).
#[test]
fn hash_matches_godot_for_vector2_and_int() {
    let mut vectors = 0;
    let mut ints = 0;
    for line in include_str!("hash_golden.txt").lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "v" => {
                let x = f32::from_bits(f[1].parse().unwrap());
                let y = f32::from_bits(f[2].parse().unwrap());
                assert_eq!(hash_vector2(x, y), f[3].parse::<u32>().unwrap(), "{line}");
                vectors += 1;
            }
            "i" => {
                let value: i64 = f[1].parse().unwrap();
                assert_eq!(hash_int(value), f[2].parse::<u32>().unwrap(), "{line}");
                ints += 1;
            }
            _ => panic!("unknown golden line {line}"),
        }
    }
    assert!(
        vectors > 900 && ints > 1500,
        "{vectors} vectors, {ints} ints"
    );
}

// The catalogue the parity golden's renderer packed, cut to what these tests need: no impostor
// forms (a reach of zero) and unit near bounds.
fn catalogue() -> ProductCatalogue {
    let mut lanes = Vec::new();
    for &variants in &VARIANT_COUNTS[..CANOPY_SPECIES] {
        lanes.push(0.0);
        for _ in 0..variants {
            lanes.extend_from_slice(&[-0.5, 0.0, -0.5, 1.0, 1.0, 1.0]);
        }
    }
    ProductCatalogue::from_lanes(&lanes).unwrap()
}

const SETTINGS: ProductSettings = ProductSettings {
    density: 1.0,
    near_m: 200.0,
    crossfade_begin_m: 150.0,
    far_m: 4500.0,
    bush_range_m: 420.0,
    rock_range_m: 420.0,
};

fn inputs(placements: &[f32], near_band: bool) -> ProductInputs<'_> {
    ProductInputs {
        placements,
        cover: None,
        runs: &[],
        origin: [0.0, 0.0],
        near_band,
        caster: true,
        settings: SETTINGS,
    }
}

// Four pinned trees: conifer variants 0 and 2, broadleaf 0 and 2 (pins are biased by one).
const PINNED: [f32; 24] = [
    10.0, 0.0, 10.0, 0.0, 1.0, 4.0, 20.0, 0.0, 10.0, 0.0, 1.0, 12.0, 30.0, 0.0, 10.0, 0.0, 1.0,
    5.0, 40.0, 0.0, 10.0, 0.0, 1.0, 13.0,
];

#[test]
fn impostor_rows_carry_the_transform_rows_then_layer_and_tint() {
    for near_band in [true, false] {
        with_product(&inputs(&PINNED, near_band), &catalogue(), |product| {
            let impostors: Vec<_> = product
                .batches
                .iter()
                .filter(|b| b.lod == LOD_IMPOSTOR && !b.proxy)
                .collect();
            assert_eq!(impostors.len(), 2);
            for batch in impostors {
                let buffer = product.buffer(batch.buffer);
                assert_eq!(buffer.len(), 2 * 16);
                for row in buffer.as_chunks::<16>().0 {
                    // Origin x in the first row's last lane; the layer is the pinned variant.
                    let expected = if row[3] as i32 % 20 == 0 { 2.0 } else { 0.0 };
                    assert_eq!(row[12], expected);
                    assert!(row[13] > 0.7 && row[13] < 1.4 && row[15] > 0.7);
                }
                // The caster shares the impostor's buffer.
                assert!(
                    product
                        .batches
                        .iter()
                        .any(|b| b.proxy && b.buffer == batch.buffer)
                );
            }
            let near: Vec<_> = product
                .batches
                .iter()
                .filter(|b| b.lod == LOD_NEAR)
                .collect();
            assert_eq!(near.len(), if near_band { 4 } else { 0 });
            for batch in near {
                // Transform rows, the tint with an opaque alpha, the stand closure lane.
                let buffer = product.buffer(batch.buffer);
                assert_eq!(buffer.len(), 20);
                assert_eq!(buffer[15], 1.0);
                assert_eq!(&buffer[16..], &[0.0; 4]);
            }
        });
    }
}

// Round 1's dense forest patch: 4,096 placements, half canopy, half understory, in a 510 m
// patch. The GDScript loops took 5.6 ms on it (`docs/terrain.md` "Streaming core").
fn dense_patch() -> Vec<f32> {
    let mut data = vec![0.0f32; 4096 * PLACEMENT_STRIDE];
    for (i, record) in data
        .as_chunks_mut::<PLACEMENT_STRIDE>()
        .0
        .iter_mut()
        .enumerate()
    {
        record[0] = (i % 64) as f32 * 7.75 + 0.25;
        record[1] = (i % 13) as f32 * 0.25;
        record[2] = (i / 64) as f32 * 7.75 + 0.5;
        record[3] = (i % 31) as f32 * 0.2;
        record[4] = 0.8 + (i % 17) as f32 * 0.025;
        record[5] = [0.0, 1.0, 2.0, 2.0, 2.0, 2.0, 3.0, 3.0][i % 8];
    }
    data
}

#[test]
fn a_dense_patch_builds_every_level_and_reuses_its_storage() {
    let data = dense_patch();
    let catalogue = catalogue();
    let first = with_product(&inputs(&data, true), &catalogue, |p| {
        (p.tree_count, p.batches.len(), p.buffer(0).as_ptr())
    });
    assert_eq!(first.0, 4096);
    // The same patch again writes into the same storage.
    let second = with_product(&inputs(&data, true), &catalogue, |p| {
        (p.tree_count, p.batches.len(), p.buffer(0).as_ptr())
    });
    assert_eq!(first, second);
}

#[test]
#[ignore = "timing: cargo test --release --lib dense_patch_product_cost -- --ignored --nocapture"]
fn dense_patch_product_cost() {
    let data = dense_patch();
    let catalogue = catalogue();
    for near_band in [true, false] {
        let input = inputs(&data, near_band);
        for _ in 0..20 {
            with_product(&input, &catalogue, |p| p.tree_count);
        }
        let mut samples: Vec<f64> = (0..200)
            .map(|_| {
                let start = std::time::Instant::now();
                std::hint::black_box(with_product(&input, &catalogue, |p| p.tree_count));
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        samples.sort_by(f64::total_cmp);
        println!(
            "DENSE_PATCH near_band={near_band} placements=4096 p50_ms={:.4} p95_ms={:.4} max_ms={:.4}",
            samples[100], samples[190], samples[199]
        );
    }
}
