// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: fbm.rs
//  script_path: rust/src/engine_twin/fbm.rs
//  module_name: fbm_twin
//  version: 0.1.1
//  author: [BantedHam]
//  description: Bit-exact Rust twin of the engine's section 3.1 fBm
//           kernel (fbm_node.gd), transcribed operation for operation:
//           the trilinear nesting order, the hash chain, and the
//           fractional-octave loop are the contract, because float
//           addition is not associative and a different order is a
//           different rounding path. No transcendentals anywhere, so
//           bit-exact f64 is the policy, not a tolerance.
//  kind: module
//  spec: fBm over value noise (Musgrave 1993 fractional octaves)
//  internal_dependencies: []
//  external_dependencies: []
//  features: [fbm, value-noise, bit-exact]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-30
// =========================================================================
//! Bit-exact twin of the engine's fBm kernel; operation order is contract.

const GAIN: f64 = 0.5;
const LACUNARITY: f64 = 2.0;
const FADE_LAST_OCTAVE: bool = false;
const MAX_OCTAVES: i32 = 16;
const INV_U32: f64 = 1.0 / 4294967296.0;

fn hash_u32(x: u32) -> u32 {
    let mut x = x;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    x
}

fn hash3(ix: i64, iy: i64, iz: i64, seed: i64) -> u32 {
    // GDScript masks with & 0xFFFFFFFF at every step; truncating i64 to
    // u32 is the same low-32 operation, wrapping adds included.
    let mut h = hash_u32(seed as u32);
    h = hash_u32(h.wrapping_add(ix as u32));
    h = hash_u32(h.wrapping_add(iy as u32));
    h = hash_u32(h.wrapping_add(iz as u32));
    h
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn value_noise(px: f64, py: f64, pz: f64, seed: i64) -> f64 {
    let fx = px.floor();
    let fy = py.floor();
    let fz = pz.floor();
    let ix = fx as i64;
    let iy = fy as i64;
    let iz = fz as i64;
    let tx = px - fx;
    let ty = py - fy;
    let tz = pz - fz;

    let ux = fade(tx);
    let uy = fade(ty);
    let uz = fade(tz);

    let v000 = f64::from(hash3(ix, iy, iz, seed)) * INV_U32;
    let v100 = f64::from(hash3(ix + 1, iy, iz, seed)) * INV_U32;
    let v010 = f64::from(hash3(ix, iy + 1, iz, seed)) * INV_U32;
    let v110 = f64::from(hash3(ix + 1, iy + 1, iz, seed)) * INV_U32;
    let v001 = f64::from(hash3(ix, iy, iz + 1, seed)) * INV_U32;
    let v101 = f64::from(hash3(ix + 1, iy, iz + 1, seed)) * INV_U32;
    let v011 = f64::from(hash3(ix, iy + 1, iz + 1, seed)) * INV_U32;
    let v111 = f64::from(hash3(ix + 1, iy + 1, iz + 1, seed)) * INV_U32;

    // x, then y, then z: the reference's nesting order is the contract.
    let x00 = lerp(v000, v100, ux);
    let x10 = lerp(v010, v110, ux);
    let x01 = lerp(v001, v101, ux);
    let x11 = lerp(v011, v111, ux);
    let y0 = lerp(x00, x10, uy);
    let y1 = lerp(x01, x11, uy);
    let v = lerp(y0, y1, uz);

    v * 2.0 - 1.0
}

/// The section 3.1 signature: evaluate(p, footprint, t, seed) in [-1, 1].
pub fn evaluate(px: f64, py: f64, pz: f64, footprint: f64, t: f64, seed: i64) -> f64 {
    let x = px + t;
    let mut value: f64 = 0.0;
    let mut amplitude: f64 = 1.0;
    let mut frequency: f64 = 1.0;
    let mut total_amplitude: f64 = 0.0;

    let fade_last = footprint > 0.0 && FADE_LAST_OCTAVE;
    for octave in 0..MAX_OCTAVES {
        if amplitude <= footprint {
            break;
        }
        let mut w: f64 = 1.0;
        if fade_last && amplitude < footprint * LACUNARITY {
            w = (amplitude / footprint.max(1.0e-30) - 1.0).clamp(0.0, 1.0);
        }
        let octave_seed =
            i64::from(hash_u32((seed ^ (i64::from(octave) * 0x9E37_79B9)) as u32));
        value += amplitude * w * value_noise(x * frequency, py * frequency, pz * frequency, octave_seed);
        total_amplitude += amplitude * w;
        amplitude *= GAIN;
        frequency *= LACUNARITY;
    }
    if total_amplitude <= 0.0 {
        return 0.0;
    }
    value * (1.0 - GAIN)
}
