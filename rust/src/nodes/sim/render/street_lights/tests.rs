// SPDX-License-Identifier: GPL-2.0-only

//! Parity of the street lights' passes with the control's GDScript (`street_lamp_grid.gd`,
//! `street_lights.gd` at `1279537c`): `golden.txt` holds what it built on the
//! `street_lamp_grid_test.gd` fixtures and a few more (written once on Godot 4.7.2), as SHA-256
//! digests and the non-zero floats. The 30k city's lamps compare with `METRUM_LAMP_DUMP`
//! pointing at a dump of the control's live heads (ignored test).

use super::*;
use sha2::{Digest, Sha256};

const GOLDEN: &str = include_str!("golden.txt");

fn golden(tag: &str) -> Vec<&'static str> {
    GOLDEN
        .lines()
        .find(|line| line.split(' ').next() == Some(tag))
        .unwrap_or_else(|| panic!("golden lacks {tag}"))
        .split(' ')
        .skip(1)
        .collect()
}

fn hex_bytes(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

fn hex_f32(text: &str) -> Vec<f32> {
    hex_bytes(text)
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

fn hex_f64(text: &str) -> f64 {
    f64::from_le_bytes(hex_bytes(text).try_into().expect("eight"))
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn floats_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

// Compares a texture's bytes with a golden `tag count sha [index:bits...]` line.
fn assert_golden(tag: &str, bytes: &[u8]) {
    let line = golden(tag);
    assert_eq!(bytes.len() / 4, line[0].parse::<usize>().expect("count"), "{tag} size");
    if sha(bytes) != line[1] {
        let mut wrong = Vec::new();
        for entry in &line[2..] {
            let (index, bits) = entry.split_once(':').expect("index:bits");
            let index: usize = index.parse().expect("index");
            let got = u32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().expect("four"));
            if format!("{got:08x}") != bits {
                wrong.push(format!("{index}: {got:08x} != {bits}"));
            }
        }
        panic!("{tag} differs from the control: {:?}", &wrong[..wrong.len().min(8)]);
    }
}

fn pack(heads: &[[f32; 3]], kinds: &[u8], reaches: &[f32], origin: [f32; 2]) -> Vec<u8> {
    let mut bytes = vec![0u8; CELL_BYTES];
    let mut counts = vec![0u8; SIDE * SIDE];
    for ((head, &kind), &reach) in heads.iter().zip(kinds).zip(reaches) {
        pack_head(&mut bytes, &mut counts, *head, f32::from(kind) + 1.0, reach, origin, CellRect::WINDOW);
    }
    bytes
}

fn triples(values: &[f32]) -> Vec<[f32; 3]> {
    values.as_chunks::<3>().0.to_vec()
}

#[test]
fn cells_equal_the_control() {
    let heads = [
        v3(-1025.0, 8.0, -2020.0),
        v3(-992.0, 11.15, -2016.0),
        v3(-1008.0, 4.4, -2032.0),
        v3(1030.0, 8.0, -8.0),
    ];
    let origin = [-1024.0, -2048.0];
    let reaches = [28.0, 31.15f64 as f32, 16.0, 28.0];
    assert_golden("cells_coverage", &pack(&heads, &[0, 1, 2, 3], &reaches, origin));
    assert_golden("cells_empty", &pack(&[], &[], &[], origin));
    let heads: Vec<[f32; 3]> = (0..21).map(|i| [8.0, i as f32, 8.0]).collect();
    let kinds: Vec<u8> = (0..21).collect();
    assert_golden("cells_overflow", &pack(&heads, &kinds, &[28.0; 21], [0.0, 0.0]));
    let odd = triples(&hex_f32(golden("odd_heads")[0]));
    let reaches = hex_f32(golden("odd_reach")[0]);
    let kinds: Vec<u8> = (0..odd.len()).map(|i| (i % 5) as u8).collect();
    assert_golden("cells_odd", &pack(&odd, &kinds, &reaches, [496.0, -3376.0]));
}

fn splat(texels: &mut [u8], heads: &[[f32; 3]], kinds: &[u8], light: &[[f32; 3]], origin: [f32; 2], side: i32, scale: f32) {
    for (head, &kind) in heads.iter().zip(kinds) {
        splat_head(texels, *head, light[usize::from(kind)], scale, origin, 16.0, side);
    }
}

#[test]
fn far_splats_equal_the_control() {
    let light = [v3(1.0, 0.5, 0.25), v3(2.0, 2.0, 2.0)];
    let heads = [v3(-38.0, 8.0, -35.0), v3(0.0, 11.0, 13.0), v3(500.0, 8.0, 0.0)];
    let kinds = [0, 1, 0];
    let origin = [-64.0, -64.0];
    let mut texels = vec![0u8; 8 * 8 * 16];
    splat(&mut texels, &heads, &kinds, &light, origin, 8, 1.0);
    assert_golden("far_splat", &texels);
    let mut centred = vec![0u8; 8 * 8 * 16];
    splat(&mut centred, &[v3(-56.0, 8.0, -56.0)], &[1], &light, origin, 8, 1.0);
    assert_golden("far_centred", &centred);
    splat(&mut texels, &heads, &kinds, &light, origin, 8, -1.0);
    assert_golden("far_removed", &texels);
    let odd = triples(&hex_f32(golden("far_odd_heads")[0]));
    let kinds: Vec<u8> = (0..odd.len()).map(|i| (i % 2) as u8).collect();
    let light = [v3(0.0137, 0.00731, 0.00213), v3(0.0219, 0.0213, 0.0201)];
    let mut texels = vec![0u8; 40 * 40 * 16];
    splat(&mut texels, &odd, &kinds, &light, [-624.0, -96.0], 40, 1.0);
    assert_golden("far_odd", &texels);
    splat(&mut texels, &odd, &kinds, &light, [-624.0, -96.0], 40, -1.0);
    assert_golden("far_odd_removed", &texels);
}

#[test]
fn flux_and_kind_light_equal_the_control() {
    assert_eq!(cone_cos().to_bits(), hex_f64(golden("cone_cos")[0]).to_bits());
    let lines: Vec<&str> = GOLDEN.lines().filter(|l| l.starts_with("flux ")).collect();
    assert_eq!(lines.len(), 6);
    for line in lines {
        let parts: Vec<f64> = line.split(' ').skip(1).map(hex_f64).collect();
        let flux = ground_flux(parts[0], parts[1], cone_cos());
        assert_eq!(flux.to_bits(), parts[2].to_bits(), "flux {} {}", parts[0], parts[1]);
    }
    let mut book = LampBook::default();
    book.set_kinds(&[[1.0; 4]; KINDS], &linear(), &[true; KINDS]);
    let expected = hex_f32(golden("kind_light")[0]);
    for kind in 0..KINDS {
        let got = book.kind_light[kind];
        assert_eq!(floats_bytes(&got), floats_bytes(&expected[kind * 4..kind * 4 + 3]), "kind {kind}");
    }
}

fn linear() -> Vec<[f32; 4]> {
    hex_f32(golden("linear")[0])
        .as_chunks::<4>()
        .0
        .to_vec()
}

fn book() -> LampBook {
    let mut book = LampBook::default();
    book.set_kinds(&[[1.0, 0.82, 0.62, 1.0]; KINDS], &linear(), &[true; KINDS]);
    book
}

#[test]
fn far_edits_equal_the_control() {
    let mut book = book();
    let mut far = vec![0u8; FAR_BYTES];
    book.configure_chunks(512.0, [-1024.0, -1024.0]);
    book.set_chunk((2, 2), [0.0; 3], &[10.0, 0.0, 20.0, 0.0, 0.0], &mut far);
    book.rebuild_far([0.0, 0.0], &mut far);
    book.set_chunk((2, 2), [0.0; 3], &[30.0, 0.0, 5.0, 1.0, 2.0, 90.0, 0.0, 40.0, 0.0, 4.0], &mut far);
    book.set_chunk((3, 2), [512.0, 0.0, 0.0], &[7.0, 0.0, 7.0, 0.0, 1.0], &mut far);
    assert_golden("edit_patched", &far);
    book.rebuild_far([0.0, 0.0], &mut far);
    assert_golden("edit_rebuilt", &far);
    book.clear(&mut far);
    assert_golden("edit_cleared", &far);
}

#[test]
fn chunk_heads_and_batches_equal_the_control() {
    let mut book = book();
    let mut far = vec![0u8; FAR_BYTES];
    let raw = hex_f32(golden("heads_raw")[0]);
    let origin = [1792.0, 0.0, -2304.0];
    let batches = book.set_chunk((5, -3), origin, &raw, &mut far);
    let chunk = &book.chunks[&(5, -3)];
    let heads: Vec<f32> = chunk.heads.iter().flatten().copied().collect();
    assert_eq!(floats_bytes(&heads), hex_bytes(golden("heads_points")[0]), "heads");
    assert_eq!(chunk.kinds, hex_bytes(golden("heads_kinds")[0]), "kinds");
    // The control made three level-of-detail nodes of each batch.
    let counts: Vec<String> = batches
        .transforms
        .iter()
        .flat_map(|t| [(t.len() / 12).to_string(), (t.len() / 12).to_string(), (t.len() / 12).to_string()])
        .collect();
    assert_eq!(counts.join(","), golden("heads_batches")[0]);
    // Each batch's transforms are its lights' in input order.
    let expected = hex_f32(golden("heads_transforms")[0]);
    let mut taken = vec![0usize; batches.transforms.len()];
    let mut buckets: Vec<(i64, i64, u8)> = Vec::new();
    for (i, light) in raw.as_chunks::<5>().0.iter().enumerate() {
        let bucket = (
            (f64::from(light[0]) / 64.0).floor() as i64,
            (f64::from(light[2]) / 64.0).floor() as i64,
            light[4] as u8,
        );
        let b = buckets.iter().position(|&k| k == bucket).unwrap_or_else(|| {
            buckets.push(bucket);
            buckets.len() - 1
        });
        assert_eq!(batches.kinds[b], bucket.2);
        let got = &batches.transforms[b][taken[b] * 12..taken[b] * 12 + 12];
        assert_eq!(floats_bytes(got), floats_bytes(&expected[i * 12..i * 12 + 12]), "light {i}");
        taken[b] += 1;
    }
    let glow = hex_f32(golden("heads_glow")[0]);
    assert_eq!(batches.glow.len() / 16, glow.len() / 12);
    for (i, instance) in batches.glow.as_chunks::<16>().0.iter().enumerate() {
        assert_eq!(floats_bytes(&instance[..12]), floats_bytes(&glow[i * 12..i * 12 + 12]), "glow {i}");
        assert_eq!(instance[12..], [1.0, 0.82, 0.62, 1.0]);
    }
}

// A synthetic town: rows of lamps along streets across several chunks, fractional positions.
fn town(book: &mut LampBook, far: &mut [u8]) {
    book.configure_chunks(512.0, [-2048.0, -2048.0]);
    for cz in 0..8 {
        for cx in 0..8 {
            let origin = [-2048.0 + cx as f32 * 512.0 + 256.0, 0.0, -2048.0 + cz as f32 * 512.0 + 256.0];
            let mut lights = Vec::new();
            for street in 0..6 {
                for step in 0..17 {
                    let x = -250.0 + step as f32 * 29.37 + street as f32 * 0.31;
                    let z = -240.0 + street as f32 * 83.1 + (step % 3) as f32 * 0.77;
                    let kind = ((cx + cz + street + step) % 5) as f32;
                    lights.extend_from_slice(&[x, 0.2 * street as f32, z, step as f32 * 0.37, kind]);
                }
            }
            book.set_chunk((cx, cz), origin, &lights, far);
        }
    }
}

#[test]
fn edits_repack_only_their_cells_and_equal_a_full_pack() {
    let mut book = book();
    let mut far = vec![0u8; FAR_BYTES];
    let mut cells = vec![0u8; CELL_BYTES];
    town(&mut book, &mut far);
    let eye = [131.7, 300.0, -77.2];
    let down = [0.0, -0.7, -0.7];
    // The grid first, then the far map: one texture a frame.
    assert_eq!(book.update(eye, down, 0.016, &mut cells, &mut far) & 3, CELLS_CHANGED);
    let full = book.packed_rows;
    assert_eq!(book.update(eye, down, 0.016, &mut cells, &mut far) & 3, FAR_CHANGED);
    assert_eq!(book.update(eye, down, 0.016, &mut cells, &mut far) & 3, 0);
    // Re-light one chunk: only the cells around it re-pack.
    let lights = [10.5, 0.0, 20.25, 0.3, 3.0, -100.0, 0.0, 200.0, 1.0, 0.0];
    book.set_chunk((4, 3), [256.0, 0.0, -256.0], &lights, &mut far);
    book.remove_chunk((3, 4), &mut far);
    assert_eq!(book.update(eye, down, 0.016, &mut cells, &mut far) & 3, CELLS_CHANGED);
    assert!(book.packed_rows > 0 && book.packed_rows < full / 2, "{} of {full}", book.packed_rows);
    let window = book.grid.expect("built");
    let mut fresh = vec![0u8; CELL_BYTES];
    book.recentre_grid(window.cell, &mut fresh);
    assert!(cells == fresh, "an edit's re-pack equals a full pack");
    // The far map's patch equals a rebuild up to rounding of the order of its sums.
    assert_eq!(book.update(eye, down, 0.016, &mut cells, &mut far) & 3, FAR_CHANGED);
}

#[test]
fn recentre_follows_sixteen_cells_and_the_far_map_a_kilometre() {
    let mut book = book();
    let mut far = vec![0u8; FAR_BYTES];
    let mut cells = vec![0u8; CELL_BYTES];
    town(&mut book, &mut far);
    let down = [0.0, -1.0, 0.0];
    book.update([0.0, 300.0, 0.0], down, 0.0, &mut cells, &mut far);
    book.update([0.0, 300.0, 0.0], down, 0.0, &mut cells, &mut far);
    assert_eq!(book.update([256.0, 300.0, 0.0], down, 0.0, &mut cells, &mut far) & 3, 0);
    assert_eq!(book.update([272.0, 300.0, 0.0], down, 0.0, &mut cells, &mut far) & 3, CELLS_CHANGED);
    assert_eq!(book.update([1030.0, 300.0, 0.0], down, 0.0, &mut cells, &mut far) & 3, CELLS_CHANGED);
    assert_eq!(book.update([1030.0, 300.0, 0.0], down, 0.0, &mut cells, &mut far) & 3, FAR_CHANGED);
}

// Every head within the radius, sorted by the pool's order: the brute-force nearest.
fn brute_pool(book: &LampBook, eye: [f32; 3]) -> Vec<Candidate> {
    let mut all: Vec<Candidate> = book
        .chunks
        .values()
        .flat_map(|chunk| chunk.heads.iter().zip(&chunk.kinds))
        .map(|(head, &kind)| {
            let d: [f32; 3] = std::array::from_fn(|i| eye[i] - head[i]);
            Candidate { distance_sq: d[0] * d[0] + d[1] * d[1] + d[2] * d[2], head: *head, kind }
        })
        .filter(|c| f64::from(c.distance_sq) < POOL_RADIUS_M * POOL_RADIUS_M)
        .collect();
    all.sort_by(candidate_order);
    all
}

#[test]
fn the_pool_is_the_nearest_thirty_two() {
    let mut book = book();
    let mut far = vec![0u8; FAR_BYTES];
    let mut cells = vec![0u8; CELL_BYTES];
    town(&mut book, &mut far);
    for eye in [[0.0, 10.0, 0.0], [-1795.3, 4.0, -1700.0], [511.9, 30.0, 0.1], [5000.0, 10.0, 0.0]] {
        book.pool_age_s = f64::INFINITY;
        book.update(eye, [0.0, -1.0, 0.0], 0.0, &mut cells, &mut far);
        let brute = brute_pool(&book, eye);
        let n = brute.len().min(POOL_SIZE);
        assert_eq!(book.pool[3] as usize, n);
        for (i, candidate) in brute[..n].iter().enumerate() {
            assert_eq!(book.pool[4 + i * 4..7 + i * 4], candidate.head);
        }
        let handoff = if brute.len() > POOL_SIZE {
            f64::from(brute[POOL_SIZE - 1].distance_sq).sqrt()
        } else {
            POOL_RADIUS_M
        };
        let fade = POOL_FADE_M.min(handoff * 0.5);
        assert_eq!(book.pool[..3], [(handoff - fade) as f32, fade as f32, handoff as f32]);
    }
}

// The 30k city's live heads from the control (`METRUM_LAMP_DUMP`, written by the L1 dump hook):
// the grid at five cells, the far map at four focus points and the pool at four eyes.
#[test]
#[ignore = "needs METRUM_LAMP_DUMP, a dump of the control's heads"]
fn city_lamps_equal_the_control() {
    let path = std::env::var("METRUM_LAMP_DUMP").expect("METRUM_LAMP_DUMP");
    let text = std::fs::read_to_string(path).expect("dump");
    let mut book = book();
    let mut far = vec![0u8; FAR_BYTES];
    let mut cells = vec![0u8; CELL_BYTES];
    let (mut grids, mut fars, mut pools) = (0, 0, 0);
    for line in text.lines() {
        let parts: Vec<&str> = line.split(' ').collect();
        match parts[0] {
            "span" => {
                let span: f32 = parts[1].parse().expect("span");
                let inner = &line[line.find("Vector2(").expect("origin") + 8..line.rfind(')').expect(")")];
                let origin: Vec<f32> = inner.split(", ").map(|p| p.parse().expect("origin")).collect();
                book.configure_chunks(span, [origin[0], origin[1]]);
            }
            "chunk" => {
                let key = (parts[1].parse().expect("x"), parts[2].parse().expect("z"));
                let heads = triples(&hex_f32(parts[3]));
                let kinds = hex_bytes(parts.get(4).copied().unwrap_or(""));
                book.insert_heads(key, heads, kinds, &mut far);
            }
            "grid" => {
                book.recentre_grid([parts[1].parse().expect("x"), parts[2].parse().expect("z")], &mut cells);
                assert_eq!(sha(&cells), parts[3], "grid at {} {}", parts[1], parts[2]);
                grids += 1;
            }
            "far" => {
                let focus = hex_f32(parts[1]);
                book.rebuild_far([focus[0], focus[1]], &mut far);
                assert_eq!(sha(&far), parts[2], "far at {focus:?}");
                fars += 1;
            }
            "pool" => {
                let eye = hex_f32(parts[1]);
                let fade = hex_f32(parts[2]);
                book.assign_pool([eye[0], eye[1], eye[2]]);
                assert_eq!(book.pool[..2], fade[..], "pool fade at {eye:?}");
                let key = |a: &[f32; 3], b: &[f32; 3]| {
                    a.iter().zip(b).map(|(x, y)| x.total_cmp(y)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
                };
                let mut expected = triples(&hex_f32(parts.get(3).copied().unwrap_or("")));
                let mut got: Vec<[f32; 3]> = book.pool[4..].as_chunks::<4>().0.iter().map(|l| [l[0], l[1], l[2]]).collect();
                expected.sort_by(key);
                got.sort_by(key);
                assert_eq!(got, expected, "pool at {eye:?}");
                pools += 1;
            }
            _ => {}
        }
    }
    assert!(grids == 5 && fars == 4 && pools == 4, "{grids} {fars} {pools}");
}
