// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: boundary_fixture.rs
//  script_path: rust/tests/boundary_fixture.rs
//  module_name: boundary_fixture
//  version: 0.1.0
//  author: [BantedHam]
//  description: The Rust side of the simulation boundary drill. Consumes
//           the fixtures spike_boundary.gd records (batched samples and
//           a deposit grid) and validates the contract without the
//           engine running: schema, lengths, finiteness, and the
//           signed-16-bit grid read back to the values the engine wrote.
//  kind: spike
//  spec: none
//  internal_dependencies: []
//  external_dependencies: [serde_json]
//  features: [boundary-contract, fixtures]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-30
// =========================================================================

use std::fs;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("godot")
        .join("fixtures")
        .join("boundary")
}

#[test]
fn sample_fixture_holds_the_contract() {
    let raw = fs::read_to_string(fixture_dir().join("sample.json"))
        .expect("sample.json fixture missing; run spike_boundary.gd first");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("fixture is not JSON");

    let positions = v["positions"].as_array().expect("positions array");
    let samples = v["samples"].as_array().expect("samples array");
    assert_eq!(
        positions.len(),
        samples.len(),
        "one sample per position is the contract"
    );
    assert!(!positions.is_empty(), "an empty fixture drills nothing");
    assert!(v["seed"].is_i64(), "seed rides the fixture");
    assert!(v["footprint"].is_f64() || v["footprint"].is_i64(), "footprint rides the fixture");
    assert!(v["t"].is_f64() || v["t"].is_i64(), "t rides the fixture");

    for p in positions {
        let triple = p.as_array().expect("position is [x, y, z]");
        assert_eq!(triple.len(), 3, "position is [x, y, z]");
        for c in triple {
            assert!(c.as_f64().expect("coordinate is a number").is_finite());
        }
    }
    for s in samples {
        assert!(
            s.as_f64().expect("sample is a number").is_finite(),
            "a field sample is finite"
        );
    }
}

#[test]
fn deposit_fixture_reads_back_signed_metres() {
    let dir = fixture_dir();
    let meta_raw = fs::read_to_string(dir.join("deposit.json"))
        .expect("deposit.json fixture missing; run spike_boundary.gd first");
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).expect("sidecar is not JSON");

    let width = meta["width"].as_i64().expect("width") as usize;
    let height = meta["height"].as_i64().expect("height") as usize;
    assert!(width > 0 && height > 0, "a deposit covers ground");
    assert!(meta["origin_lon"].is_f64() || meta["origin_lon"].is_i64());
    assert!(meta["origin_lat"].is_f64() || meta["origin_lat"].is_i64());
    assert!(meta["pixel_deg_lon"].is_f64());
    assert!(meta["pixel_deg_lat"].is_f64());

    let bytes = fs::read(dir.join("deposit.raw")).expect("deposit.raw missing");
    assert_eq!(
        bytes.len(),
        width * height * 2,
        "row-major signed 16-bit cells, nothing else"
    );

    let cells: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    // The spike writes the format's full range on purpose; finding both
    // extremes proves sign handling end to end.
    assert!(cells.contains(&32767), "positive extreme survived the write");
    assert!(cells.contains(&-32768), "negative extreme survived the write");
}
