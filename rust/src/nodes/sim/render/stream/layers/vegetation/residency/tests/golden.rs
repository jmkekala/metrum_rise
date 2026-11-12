// SPDX-License-Identifier: GPL-2.0-only

//! The residency against the control's `vegetation.gd`, recorded on the stream benchmark's camera
//! paths (`dev` `f1c93527`, `RENDER-32-F`): every phase of the 30k city and Kuopio, from the
//! script's own camera, terrain residency and decisions. The 30k file has every frame: the
//! residency must evaluate its tiers on exactly the frames the script did and arrive at the same
//! wanted set (count and digest). Kuopio's has the evaluations only. Both carry a sample of the
//! band decisions (one in 24): the distance of the request or check, bit for bit, and the bands
//! the script chose with the record's hysteresis. The whole recordings (every band decision) are
//! replayed by the ignored test from `$METRUM_VEG_GOLDEN` (a directory of `full-*.txt`).
//!
//! Lines: `H` world, terrain span, subdivision, far, near, shadow range; `F x z y revision` a
//! frame (`E` the same for an evaluation without the frames between); `R +x,z -x,z` the terrain
//! residency changes since the last evaluation; `W count digest` the wanted set after it; `Q key
//! camera distance has_record record_bands chosen_bands` a request; `B key camera distance
//! record_bands` a band check that found the record out of its bands.

use super::*;

const PATHS_30K: &str = include_str!("paths_30k_golden.txt");
const PATHS_KUOPIO: &str = include_str!("paths_kuopio_golden.txt");

// The FNV-1a of the sorted keys' little-endian lanes (the recorder's `wanted_digest`).
fn wanted_digest(keys: impl Iterator<Item = PatchKey>) -> u64 {
    let mut keys: Vec<PatchKey> = keys.collect();
    keys.sort_unstable();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for key in keys {
        for lane in key {
            for byte in (lane as u32).to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    hash
}

struct Frame {
    eye: [f32; 3],
    revision: i64,
    evaluation_only: bool,
    resident: Vec<(Cell, bool)>,
    wanted: Option<(usize, u64)>,
}

#[derive(Default, Debug)]
struct Replayed {
    frames: usize,
    evaluations: usize,
    requests: usize,
    checks: usize,
}

fn bands(text: &str) -> Bands {
    let bit = |at: usize| text.as_bytes()[at] == b'1';
    Bands {
        understory: bit(0),
        near_band: bit(1),
        caster: bit(2),
    }
}

fn replay(text: &str) -> Replayed {
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap().split(' ').collect();
    assert_eq!(header[0], "H");
    let f32_at = |at: usize| header[at].parse::<f32>().unwrap();
    let f64_at = |at: usize| header[at].parse::<f64>().unwrap();
    let world = [f32_at(1), f32_at(2)];
    let terrain_span = f64_at(3);
    let (far, near, shadow) = (f64_at(5), f64_at(6), f64_at(7));
    let settings = ResidencySettings::from_lanes(&[
        1.0,
        near,
        near - 50.0,
        far,
        420.0,
        420.0,
        f64_at(4),
        0.0,
        shadow,
        f64::from(world[0]),
        f64::from(world[1]),
    ])
    .unwrap();
    let mut residency = VegetationResidency::default();
    let mut layer = VegetationLayer::new(64, 16);
    let mut backend = Recorder::new();
    let mut jobs = Jobs {
        full: true,
        ..Jobs::default()
    };
    residency.set_terrain_span(terrain_span);
    residency.reset(settings, &mut layer, &mut backend, &mut jobs);
    let mut out = Replayed::default();
    let mut pending: Option<Frame> = None;
    let mut last_revision = None;
    let mut run = |frame: Frame, residency: &mut VegetationResidency, out: &mut Replayed| {
        for &(block, resident) in &frame.resident {
            residency.set_resident(block, resident);
        }
        if last_revision != Some(frame.revision) || frame.evaluation_only {
            residency.revise_residency();
        }
        last_revision = Some(frame.revision);
        residency.frame(Some(frame.eye), &mut layer, &mut backend, &mut jobs);
        let work = residency.work();
        out.frames += 1;
        assert_eq!(
            work.evaluated,
            frame.wanted.is_some(),
            "frame {} at {:?}: the script evaluated the tiers on exactly these frames",
            out.frames,
            frame.eye
        );
        if let Some((count, digest)) = frame.wanted {
            out.evaluations += 1;
            let wanted: Vec<PatchKey> = residency.wanted().collect();
            assert_eq!(
                (wanted.len(), wanted_digest(wanted.into_iter())),
                (count, digest),
                "evaluation {} at {:?}: the wanted set",
                out.evaluations,
                frame.eye
            );
        }
    };
    for line in lines {
        let fields: Vec<&str> = line.split(' ').collect();
        let key = |at: usize| -> PatchKey {
            [0, 1, 2].map(|lane| fields[at + lane].parse::<i32>().unwrap())
        };
        match fields[0] {
            "F" | "E" => {
                if let Some(frame) = pending.take() {
                    run(frame, &mut residency, &mut out);
                }
                let at = |i: usize| fields[i].parse::<f32>().unwrap();
                pending = Some(Frame {
                    eye: [at(1), at(3), at(2)],
                    revision: fields[4].parse().unwrap(),
                    evaluation_only: fields[0] == "E",
                    resident: Vec::new(),
                    wanted: None,
                });
            }
            "R" => {
                let frame = pending.as_mut().unwrap();
                for change in fields[1..].iter().filter(|field| !field.is_empty()) {
                    let (x, z) = change[1..].split_once(',').unwrap();
                    frame
                        .resident
                        .push(([x.parse().unwrap(), z.parse().unwrap()], change.starts_with('+')));
                }
            }
            "W" => {
                let digest = u64::from_str_radix(fields[2], 16).unwrap();
                pending.as_mut().unwrap().wanted = Some((fields[1].parse().unwrap(), digest));
            }
            "Q" | "B" => {
                let key = key(1);
                let camera = [fields[4].parse::<f32>().unwrap(), fields[5].parse::<f32>().unwrap()];
                let distance = fields[6].parse::<f32>().unwrap();
                assert_eq!(
                    key_distance(key, terrain_span, world, camera).to_bits(),
                    f64::from(distance).to_bits(),
                    "{line}: the patch distance"
                );
                let span = key_span(key, terrain_span);
                let distance = f64::from(distance);
                if fields[0] == "Q" {
                    let previous = (fields[7] == "1").then(|| bands(fields[8]));
                    assert_eq!(settings.bands(distance, span, previous), bands(fields[9]), "{line}");
                    out.requests += 1;
                } else {
                    let held = bands(fields[7]);
                    assert_ne!(settings.bands(distance, span, Some(held)), held, "{line}");
                    out.checks += 1;
                }
            }
            other => panic!("unknown record {other}"),
        }
    }
    if let Some(frame) = pending.take() {
        run(frame, &mut residency, &mut out);
    }
    out
}

#[test]
fn the_residency_equals_the_control_on_the_30k_benchmark_paths() {
    let replayed = replay(PATHS_30K);
    assert!(replayed.frames > 3000 && replayed.evaluations > 250, "{replayed:?}");
    assert!(replayed.requests > 200 && replayed.checks > 500, "{replayed:?}");
}

#[test]
fn the_residency_equals_the_control_on_the_kuopio_benchmark_paths() {
    let replayed = replay(PATHS_KUOPIO);
    assert!(replayed.evaluations > 400, "{replayed:?}");
    assert!(replayed.requests > 400 && replayed.checks > 400, "{replayed:?}");
}

#[test]
#[ignore = "reads the whole recordings from $METRUM_VEG_GOLDEN"]
fn the_residency_equals_the_control_on_the_whole_recordings() {
    let dir = std::env::var("METRUM_VEG_GOLDEN").expect("METRUM_VEG_GOLDEN");
    for name in ["full-30k.txt", "full-kuopio.txt"] {
        let text = std::fs::read_to_string(std::path::Path::new(&dir).join(name)).unwrap();
        eprintln!("{name}: {:?}", replay(&text));
    }
}
