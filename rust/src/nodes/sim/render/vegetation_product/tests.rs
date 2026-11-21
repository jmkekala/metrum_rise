// SPDX-License-Identifier: GPL-3.0-or-later

//! Godot's `hash()` against vectors from the engine, the product's buffer layouts, and the cost
//! of a dense patch. The full-patch draw golden is in `vegetation_api`'s
//! `product_parity` tests, which need a world.

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
fn a_product_counts_the_storage_it_holds_not_the_rows_in_use() {
    // AUDIT-stream-2 finding 2: storage of a dense patch reused for a sparse one still holds
    // the dense patch's capacity; a cap counting lengths held a multiple of what it said.
    let dense = dense_patch();
    let sparse = &dense[..8 * PLACEMENT_STRIDE];
    let catalogue = catalogue();
    let mut product = VegetationProduct::default();
    build_product(&inputs(&dense, true), &catalogue, &mut product);
    let held = product.bytes();
    let rows: usize = (0..product.buffer_count()).map(|i| product.buffer(i).len() * 4).sum();
    assert!(held >= rows, "at least the rows in use");
    build_product(&inputs(sparse, true), &catalogue, &mut product);
    assert!(product.tree_count < 16);
    assert_eq!(product.bytes(), held, "the reused storage is what it holds");
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

#[test]
fn unmarked_placements_keep_the_original_rows_and_tint() {
    let records = [
        12.5, 0.25, -7.0, 0.8, 0.8, 9.0, -2.0, 0.25, 9.0, 1.2, 0.8, 0.0,
    ];
    let mut placed = Vec::new();
    place(&inputs(&records, true), &mut placed);
    // Below the knee, the original formula and recorded seed-picked row still match bitwise.
    assert_eq!(placed.len(), 2);
    assert_eq!(placed[0].variant, 1);
    let pinned = &records[..PLACEMENT_STRIDE];
    let seed = i64::from(hash_vector2(pinned[0], pinned[2]));
    let scale = f64::from(pinned[4]);
    let width = scale * (0.80 + jitter_unit(seed, 53) * 0.48);
    let height = scale * (0.82 + jitter_unit(seed, 17) * 0.42);
    assert_eq!(
        placed[0].rows.map(f32::to_bits),
        instance_rows(pinned, seed, [0.0; 2], width as f32, height as f32).map(f32::to_bits),
    );
    assert_eq!(
        placed[0].tint.map(f32::to_bits),
        [1064286112, 1065682043, 1066544422]
    );
    assert_eq!(placed[1].variant, 7);
    assert_eq!(
        placed[1].rows.map(f32::to_bits),
        [
            1049246076, 1025932646, 1060234178, 3221225472, 3164496755, 1063735238, 3169397158,
            1048576000, 3207726885, 3155758670, 1049265595, 1091567616
        ],
    );
    assert_eq!(
        placed[1].tint.map(f32::to_bits),
        [1065745057, 1066720133, 1067695209]
    );
}

fn height_band(kind: CanopyKind) -> (f64, f64) {
    match kind {
        CanopyKind::Birch | CanopyKind::Aspen => (24.0, 28.0),
        CanopyKind::Pine => (28.0, 32.0),
        CanopyKind::Spruce => (28.0, 35.0),
    }
}

#[test]
fn canopy_soft_cap_preserves_the_knee_and_rises_toward_the_limit() {
    for (species, &variants) in VARIANT_COUNTS[..CANOPY_SPECIES].iter().enumerate() {
        for variant in 0..variants {
            let (kind, model_height) = canopy_model(species, variant).unwrap();
            let (knee, max) = height_band(kind);
            let cap = |height: f64| {
                height * canopy_height_ratio(species, variant, height / f64::from(model_height))
            };
            for h in [0.0, knee - 0.1, knee] {
                assert_eq!(cap(h).to_bits(), h.to_bits());
            }
            let epsilon = 0.000_01;
            assert!(((cap(knee + epsilon) - knee) / epsilon - 1.0).abs() < 0.000_003);
            let mut previous = knee;
            for excess in 1..=1000 {
                let h = knee + f64::from(excess) * 0.1;
                let capped = cap(h);
                assert!(previous < capped && capped < max && capped < h);
                previous = capped;
            }
        }
    }
}

#[test]
fn canopy_products_share_calibrated_proportions_at_every_distance() {
    let mut placements = Vec::new();
    for (species, &variants) in VARIANT_COUNTS[..CANOPY_SPECIES].iter().enumerate() {
        for variant in 0..variants {
            for i in 0..128 {
                let pin = ((variant + 1) << SPECIES_BITS) | species;
                let scale = [0.75, 0.9, 1.05, 1.2, 1.35][i % 5];
                let mut record = [
                    0.25 + (i as f32 * 0.7),
                    0.0,
                    1.5 + (i / 5) as f32,
                    0.8,
                    scale,
                    pin as f32,
                ];
                placements.extend_from_slice(&record);
                if variant == 0 {
                    record[5] = species as f32;
                    placements.extend_from_slice(&record);
                }
            }
        }
    }
    let mut placed = Vec::new();
    place(&inputs(&placements, true), &mut placed);
    let mut capped_count = 0;
    let mut generated_variants = [0; CANOPY_SPECIES];
    for (p, record) in placed
        .iter()
        .zip(placements.as_chunks::<PLACEMENT_STRIDE>().0)
    {
        let species = usize::from(p.species);
        let variant = usize::from(p.variant);
        if record[5] < CANOPY_SPECIES as f32 {
            generated_variants[species] |= 1 << variant;
        }
        let (kind, model_height) = canopy_model(species, variant).unwrap();
        let (knee, max) = height_band(kind);
        let seed = i64::from(hash_vector2(record[0], record[2]));
        let scale = f64::from(record[4]);
        let height = scale * (0.82 + jitter_unit(seed, 17) * 0.42);
        let width = scale * (0.80 + jitter_unit(seed, 53) * 0.48);
        let basis = basis_of(&p.rows);
        let drawn_height = f64::from(column_length_squared(&basis, 1).sqrt());
        assert!(drawn_height * f64::from(model_height) < max);
        if height * f64::from(model_height) <= knee {
            assert_eq!(
                p.rows.map(f32::to_bits),
                instance_rows(record, seed, [0.0; 2], width as f32, height as f32)
                    .map(f32::to_bits),
            );
        } else {
            capped_count += 1;
            let width_ratio = f64::from(column_length_squared(&basis, 0).sqrt()) / width;
            assert!((drawn_height / height - width_ratio).abs() < 0.000_000_3);
            let plant = AuthoredPlant {
                x: record[0],
                z: record[2],
                yaw: record[3],
                scale: record[4],
                species: p.species,
                variant: p.variant + 1,
            };
            assert!(
                (f64::from(canopy_cover_scale(&plant)) / scale - width_ratio).abs() < 0.000_000_3
            );
        }
    }
    assert!(capped_count > 100);
    for (species, seen) in generated_variants.into_iter().enumerate() {
        assert_eq!(seen, (1 << VARIANT_COUNTS[species]) - 1);
    }

    // Seed-picked (unbiased pin), tall broadleaf. One placement makes batch order immaterial.
    let record = [12.5, 0.25, -7.0, 0.8, 1.35, 1.0];
    let mut catalogue = catalogue();
    catalogue.impostor_reach[1] = 30.0;
    let near = with_product(&inputs(&record, true), &catalogue, Clone::clone);
    let far = with_product(&inputs(&record, false), &catalogue, Clone::clone);
    let [mesh, impostor, caster] = near.batches.as_slice() else {
        panic!("expected near mesh, impostor and caster");
    };
    assert_eq!(mesh.shared_bounds, impostor.shared_bounds);
    assert_eq!(caster.shared_bounds, impostor.shared_bounds);
    assert_eq!(caster.buffer, impostor.buffer);
    assert_eq!(far.batches[0].mesh_bounds, impostor.mesh_bounds);
    assert_eq!(far.buffer(0), near.buffer(impostor.buffer));
    assert_eq!(&near.buffer(mesh.buffer)[..12], &far.buffer(0)[..12]);
}

#[test]
fn fixed_look_uses_the_stored_scale_and_yaw_independently_of_position() {
    let packed = f32::from((u16::from(FIXED_LOOK | 2) << SPECIES_BITS) | 1);
    let scale = 1.3;
    let records = [
        0.0, 0.0, 0.0, 0.8, scale, packed, 12.5, 0.0, -7.0, 0.8, scale, packed, 20.0, 0.0, -7.0,
        1.2, scale, packed,
    ];
    let mut placed = Vec::new();
    place(&inputs(&records, true), &mut placed);
    assert_eq!(placed.len(), 3);
    let [ghost, planted, other_pick] = placed.as_slice() else {
        unreachable!()
    };
    assert_eq!(ghost.variant, 1);
    assert_eq!(planted.variant, ghost.variant);
    assert_eq!((planted.rows[3], planted.rows[11]), (12.5, -7.0));
    let basis = |rows: [f32; 12]| {
        [
            [rows[0], rows[1], rows[2]],
            [rows[4], rows[5], rows[6]],
            [rows[8], rows[9], rows[10]],
        ]
    };
    let planted_basis = basis(planted.rows);
    assert_eq!(
        planted_basis.map(|r| r.map(f32::to_bits)),
        basis(ghost.rows).map(|r| r.map(f32::to_bits))
    );
    assert_eq!(planted.tint.map(f32::to_bits), ghost.tint.map(f32::to_bits));
    for axis in 0..3 {
        // Orthonormal rotation in float32 introduces rounding in the recovered length.
        assert!(
            (column_length_squared(&planted_basis, axis).sqrt() - scale).abs()
                <= 2.0 * f32::EPSILON
        );
    }
    assert_ne!(other_pick.tint, ghost.tint);
    assert_ne!(
        basis(other_pick.rows)[0][1],
        planted_basis[0][1],
        "yaw also changes the lean"
    );
}
