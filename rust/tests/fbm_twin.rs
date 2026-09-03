// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: fbm_twin.rs
//  script_path: rust/tests/fbm_twin.rs
//  module_name: fbm_twin_test
//  version: 0.1.0
//  author: [BantedHam]
//  description: The twin gate: the Rust fBm twin evaluates the boundary
//           fixture's positions and must match the GDScript reference's
//           recorded samples BIT FOR BIT, the same promotion standard
//           the GLSL twins pass. A tolerance here would hide the exact
//           rounding drift the policy exists to catch.
//  kind: spike
//  spec: none
//  internal_dependencies: [rust/src/engine_twin/fbm.rs]
//  external_dependencies: [serde_json]
//  features: [twin-gate, bit-exact]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-30
// =========================================================================

use metrum_rise::engine_twin::fbm;
use std::fs;
use std::path::PathBuf;

fn fixture() -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("godot")
        .join("fixtures")
        .join("boundary")
        .join("sample.json");
    let raw = fs::read_to_string(path)
        .expect("sample.json fixture missing; run spike_boundary.gd first");
    serde_json::from_str(&raw).expect("fixture is not JSON")
}

#[test]
fn twin_matches_reference_bit_for_bit() {
    let v = fixture();
    let footprint = v["footprint"].as_f64().expect("footprint");
    let t = v["t"].as_f64().expect("t");
    let seed = v["seed"].as_i64().expect("seed");
    let positions = v["positions"].as_array().expect("positions");
    let bits = v["sample_bits"]
        .as_array()
        .expect("sample_bits missing; re-run spike_boundary.gd");
    assert_eq!(positions.len(), bits.len());

    for (p, b) in positions.iter().zip(bits.iter()) {
        let triple = p.as_array().expect("position triple");
        // The engine hands positions across as 32-bit vector components,
        // so the reference evaluated f32-quantized coordinates; the twin
        // evaluates the same quantized values.
        let px = triple[0].as_f64().unwrap() as f32 as f64;
        let py = triple[1].as_f64().unwrap() as f32 as f64;
        let pz = triple[2].as_f64().unwrap() as f32 as f64;
        let hex = b.as_str().expect("bit pattern is hex text");
        let mut raw = [0u8; 8];
        for i in 0..8 {
            raw[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                .expect("bit pattern parses");
        }
        let reference = f64::from_le_bytes(raw);
        let twin = fbm::evaluate(px, py, pz, footprint, t, seed);
        assert_eq!(
            twin.to_bits(),
            reference.to_bits(),
            "twin diverged at ({px}, {py}, {pz}): twin {twin} vs reference {reference}"
        );
    }
}
