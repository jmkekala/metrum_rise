// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: engine_inputs.rs
//  script_path: rust/src/simulation/engine_inputs.rs
//  module_name: engine_inputs
//  version: 0.4.0
//  description: The engine boundary's store, at the simulation layer so
//           grid systems read it without upward dependencies: the
//           evaluated arrays the shell delivers, the probe grid's
//           geometry, and bilinear sampling with None outside coverage,
//           which is what keeps each consumer's old default the honest
//           fallback. Desirability scales to land value; coal reads as
//           a richness fraction for the deposit reserve.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: [engine-store, probe-sampling, revisioned]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-30
// ========================================================================
//! The engine boundary's revisioned store and probe sampling.

use std::sync::RwLock;

/// What the engine hands across each delivery. One game, one store; the
/// revision says when anything changed.
#[derive(Default, Clone)]
pub struct EngineInputs {
    /// Evaluated habitability-by-buildability per probe sample, 0-1.
    pub desirability: Vec<f64>,
    /// Evaluated iron richness fraction per probe sample.
    pub iron: Vec<f64>,
    /// Evaluated coal richness fraction per probe sample.
    pub coal: Vec<f64>,
    /// Evaluated stone richness fraction per probe sample.
    pub stone: Vec<f64>,
    /// World X of the probe grid's first sample.
    pub origin_x: f64,
    /// World Z of the probe grid's first sample.
    pub origin_z: f64,
    /// Metres between probe samples.
    pub spacing: f64,
    /// Probe samples per side; the grid is side by side, row-major.
    pub side: usize,
    /// Bumps once per delivery, so consumers can tell stale from fresh.
    pub revision: u64,
}

static STORE: RwLock<EngineInputs> = RwLock::new(EngineInputs {
    desirability: Vec::new(),
    iron: Vec::new(),
    coal: Vec::new(),
    stone: Vec::new(),
    origin_x: 0.0,
    origin_z: 0.0,
    spacing: 0.0,
    side: 0,
    revision: 0,
});

/// Lands one delivery in the store and returns the new revision.
#[allow(clippy::too_many_arguments)]
pub fn store(
    desirability: Vec<f64>,
    iron: Vec<f64>,
    coal: Vec<f64>,
    stone: Vec<f64>,
    origin_x: f64,
    origin_z: f64,
    spacing: f64,
    side: usize,
) -> u64 {
    let mut g = STORE.write().expect("engine inputs poisoned");
    g.desirability = desirability;
    g.iron = iron;
    g.coal = coal;
    g.stone = stone;
    g.origin_x = origin_x;
    g.origin_z = origin_z;
    g.spacing = spacing;
    g.side = side;
    g.revision += 1;
    g.revision
}

/// A whole-store clone, taken once per tick so no lock outlives a call.
pub fn snapshot() -> EngineInputs {
    STORE.read().expect("engine inputs poisoned").clone()
}

impl EngineInputs {
    /// The delivered desirability at a world position, bilinear over the
    /// probe grid, scaled to the sim's 0-100 land value. None outside
    /// coverage, which is what keeps the flat default the fallback.
    pub fn desirability_base(&self, x: f64, z: f64) -> Option<f64> {
        Some(self.bilinear(&self.desirability, x, z)? * 100.0)
    }

    /// The delivered coal richness fraction at a world position, clamped
    /// to 0-1 so a hot channel can never mint reserve past full richness.
    /// None outside coverage, so an unpainted, undelivered cell stays the
    /// zero it always was.
    pub fn coal_fraction(&self, x: f64, z: f64) -> Option<f64> {
        Some(self.bilinear(&self.coal, x, z)?.clamp(0.0, 1.0))
    }

    /// The delivered iron richness fraction, same law as coal.
    pub fn iron_fraction(&self, x: f64, z: f64) -> Option<f64> {
        Some(self.bilinear(&self.iron, x, z)?.clamp(0.0, 1.0))
    }

    /// The delivered stone richness fraction, same law as coal.
    pub fn stone_fraction(&self, x: f64, z: f64) -> Option<f64> {
        Some(self.bilinear(&self.stone, x, z)?.clamp(0.0, 1.0))
    }

    fn bilinear(&self, values: &[f64], x: f64, z: f64) -> Option<f64> {
        if self.side < 2 || self.spacing <= 0.0 {
            return None;
        }
        if values.len() != self.side * self.side {
            return None;
        }
        let gx = (x - self.origin_x) / self.spacing;
        let gz = (z - self.origin_z) / self.spacing;
        if gx < 0.0 || gz < 0.0 {
            return None;
        }
        let max = (self.side - 1) as f64;
        if gx > max || gz > max {
            return None;
        }
        let x0 = gx.floor() as usize;
        let z0 = gz.floor() as usize;
        let x1 = (x0 + 1).min(self.side - 1);
        let z1 = (z0 + 1).min(self.side - 1);
        let tx = gx - x0 as f64;
        let tz = gz - z0 as f64;
        let v00 = values[z0 * self.side + x0];
        let v10 = values[z0 * self.side + x1];
        let v01 = values[z1 * self.side + x0];
        let v11 = values[z1 * self.side + x1];
        let a = v00 + (v10 - v00) * tx;
        let b = v01 + (v11 - v01) * tx;
        Some(a + (b - a) * tz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_samples_inside_and_refuses_outside() {
        store(
            vec![0.0, 1.0, 0.0, 1.0],
            vec![],
            vec![],
            vec![],
            100.0,
            200.0,
            10.0,
            2,
        );
        let s = snapshot();
        // Corners are the delivered values, scaled to land value.
        assert_eq!(s.desirability_base(100.0, 200.0), Some(0.0));
        assert_eq!(s.desirability_base(110.0, 200.0), Some(100.0));
        // The middle interpolates.
        let mid = s.desirability_base(105.0, 205.0).expect("inside coverage");
        assert!((mid - 50.0).abs() < 1.0e-9);
        // Outside coverage refuses, so the flat default stays the fallback.
        assert_eq!(s.desirability_base(0.0, 0.0), None);
        assert_eq!(s.desirability_base(500.0, 200.0), None);
    }

    #[test]
    fn coal_fraction_clamps_and_refuses_outside() {
        // Built directly, not through the global store, so this test never
        // races the store test on another thread.
        let s = EngineInputs {
            coal: vec![0.25, 3.0, 0.25, 3.0],
            origin_x: 0.0,
            origin_z: 0.0,
            spacing: 10.0,
            side: 2,
            ..EngineInputs::default()
        };
        assert_eq!(s.coal_fraction(0.0, 0.0), Some(0.25));
        // A hot channel clamps to full richness.
        assert_eq!(s.coal_fraction(10.0, 0.0), Some(1.0));
        // Outside coverage refuses; the unpainted zero stays the fallback.
        assert_eq!(s.coal_fraction(-1.0, 0.0), None);
        // An undelivered channel refuses everywhere.
        assert_eq!(EngineInputs::default().coal_fraction(0.0, 0.0), None);
    }
}
