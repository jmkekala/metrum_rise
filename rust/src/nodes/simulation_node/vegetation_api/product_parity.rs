// SPDX-License-Identifier: GPL-2.0-only

//! The vegetation product against the GDScript upload it replaced (`RENDER-32-C`): bitwise.
//!
//! `product_golden.txt` was written once, by the control's `vegetation.gd` `_upload_patch` on a
//! rendered Godot 4.7.2 (the near buffers and the engine's own near bounds read back only from a
//! real renderer), for every patch of two worlds at three camera distances: a 2040 m blank world
//! with its generated scatter, on the coarse grid and a third of the fine grid, and a 1020 m
//! blank world with brush stands of every preset, pinned plants, three hedge rows and a cleared
//! disc, on both grids. A `C` line holds the renderer's mesh catalogue as the bridge packs it;
//! each `P` line one upload: world, key, camera case, the bands the upload chose, the origin, the
//! tree count, the canopy box, the SHA-256 of its placements, stand cover and hedge runs, and the
//! SHA-256 of its instance records (per instance: species, level, variant, caster, count, colour
//! and custom lanes, both bounds, the range, and the SHA-256 of the buffer).

use super::land_cover::Coverage;
use super::*;
use crate::nodes::sim::render::vegetation_product::{
    ProductCatalogue, ProductInputs, ProductSettings, StandCover, VegetationProduct, with_product,
};
use sha2::{Digest, Sha256};

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn float_hex(values: &[f32]) -> String {
    hex(values
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>())
}

fn sha(bytes: impl AsRef<[u8]>) -> String {
    hex(Sha256::digest(bytes.as_ref()))
}

fn floats_of(hex_text: &str) -> Vec<f32> {
    (0..hex_text.len() / 8)
        .map(|i| {
            let word = &hex_text[i * 8..i * 8 + 8];
            let bytes: [u8; 4] =
                std::array::from_fn(|b| u8::from_str_radix(&word[b * 2..b * 2 + 2], 16).unwrap());
            f32::from_le_bytes(bytes)
        })
        .collect()
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

// The two worlds the golden writer built, by the same edits through the same kernels the
// bridge calls.
fn world(index: u32) -> SimCore {
    let mut core = super::super::tests::vegetation_test_core(Default::default());
    let size = if index == 0 { 2040.0 } else { 1020.0 };
    core.create_blank_world_internal(size, size, 10.0, 510.0, 20.0)
        .unwrap();
    if index == 1 {
        let options = (0..26).filter(|o| !(17..=19).contains(o));
        for (spot, option) in (0u32..).zip(options) {
            let at = Vector2::new(
                (-420.0 + f64::from(spot % 6) * 160.0) as f32,
                (-400.0 + f64::from(spot / 6) * 190.0) as f32,
            );
            let radius = if option < 11 { 40.0 } else { 12.0 };
            for _ in 0..5 {
                paint_at(
                    &mut core,
                    at,
                    radius,
                    option,
                    0,
                    stamp_limit(option, radius),
                );
            }
        }
        for option in [4i64, 5, 6, 7, 11, 15, 20] {
            let at = Vector2::new(
                (30.0 + option as f64 * 3.1) as f32,
                (70.5 - option as f64 * 2.3) as f32,
            );
            add_at(&mut core, at, option);
        }
        let lines = [
            ((-140.0, 3.0), (60.0, 3.0), 17),
            ((120.0, -150.0), (250.0, -20.0), 18),
            ((-300.0, 200.0), (-300.0, 380.0), 19),
        ];
        for ((fx, fz), (tx, tz), option) in lines {
            line_at(
                &mut core,
                Vector2::new(fx, fz),
                Vector2::new(tx, tz),
                option,
                0,
            );
        }
        remove_at(&mut core, Vector2::new(200.0, 250.0), 50.0, 0);
    }
    core.prepare_site_queries();
    core
}

// One instance record line, as the golden writer formats it.
fn record_lines(product: &VegetationProduct) -> String {
    let lines: Vec<String> = product
        .batches
        .iter()
        .map(|b| {
            format!(
                "I {} {} {} {} {} {} {} {} {} {} {}",
                b.species,
                b.lod,
                b.variant,
                u8::from(b.proxy),
                b.count,
                u8::from(b.colors),
                u8::from(b.custom),
                float_hex(&b.mesh_bounds.lanes()),
                float_hex(&b.shared_bounds.lanes()),
                float_hex(&b.range),
                sha(f32_bytes(product.buffer(b.buffer))),
            )
        })
        .collect();
    lines.join("\n")
}

const SETTINGS: ProductSettings = ProductSettings {
    density: 1.0,
    near_m: 200.0,
    crossfade_begin_m: 150.0,
    far_m: 4500.0,
    bush_range_m: 420.0,
    rock_range_m: 420.0,
};

#[test]
fn vegetation_product_matches_the_gdscript_upload_bitwise() {
    let golden = include_str!("product_golden.txt");
    let catalogue_line = golden.lines().next().unwrap();
    let catalogue =
        ProductCatalogue::from_lanes(&floats_of(catalogue_line.strip_prefix("C ").unwrap()))
            .expect("catalogue lanes");
    let worlds = [world(0), world(1)];
    let mut checked = 0;
    let mut instances = 0;
    let mut mismatches = Vec::new();
    for line in golden.lines().skip(1) {
        let f: Vec<&str> = line.split(' ').collect();
        assert_eq!(f[0], "P");
        let num = |i: usize| f[i].parse::<i64>().unwrap();
        let core = &worlds[num(1) as usize];
        let (understory, near_band, caster) = (num(6) == 1, num(7) == 1, num(8) != 0);
        let origin = floats_of(f[9]);
        let origin = Vector2::new(origin[0], origin[1]);
        let span = 510.0 / num(4) as f32;
        let placements = decorative_tree_patch(core, origin, span, understory);
        assert_eq!(sha(f32_bytes(&placements)), f[12], "placements of {line}");
        let margin = STAND_RADIUS_TEXELS as f32 * land_cover::TEXEL_M;
        let (bytes, width, height, bounds) =
            Coverage::build(core, origin - Vector2::splat(margin), span + 2.0 * margin)
                .stand_parts(STAND_RADIUS_TEXELS);
        assert_eq!(sha(&bytes), f[13], "stand cover of {line}");
        let runs = hedge_runs(core, origin, span);
        assert_eq!(sha(f32_bytes(&runs)), f[14], "hedge runs of {line}");
        let inputs = ProductInputs {
            placements: &placements,
            cover: (!bytes.is_empty()).then_some(StandCover {
                bytes: &bytes,
                width: width as i64,
                height: height as i64,
                bounds,
            }),
            runs: if near_band { &[] } else { &runs },
            origin: [origin.x, origin.y],
            near_band,
            caster,
            settings: SETTINGS,
        };
        let (count, canopy, records, batches) = with_product(&inputs, &catalogue, |product| {
            (
                product.tree_count,
                float_hex(&product.canopy_box.lanes()),
                record_lines(product),
                product.batches.len(),
            )
        });
        instances += batches;
        if count != num(10) || canopy != f[11] || sha(records.as_bytes()) != f[15] {
            mismatches.push(format!("{line}\n  rust: {count} {canopy}\n{records}"));
        }
        checked += 1;
    }
    assert!(
        mismatches.is_empty(),
        "{} of {checked} uploads differ; first:\n{}",
        mismatches.len(),
        mismatches.first().map_or("", |m| m.as_str())
    );
    assert!(
        checked >= 500 && instances > 5000,
        "{checked} uploads, {instances} batches"
    );
}
