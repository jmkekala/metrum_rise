// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: engine_api.rs
//  script_path: rust/src/nodes/simulation_node/engine_api.rs
//  module_name: engine_api
//  version: 0.7.0
//  description: The engine boundary's intake API: the shell hands the
//           evaluated arrays and the probe grid's geometry across once
//           per delivery, and the store they land in lives at the
//           simulation layer where the grid systems read it. This file
//           is the Godot-facing surface and nothing else.
//  kind: module
//  spec: none
//  internal_dependencies: [rust/src/simulation/engine_inputs.rs]
//  external_dependencies: []
//  features: [engine-intake]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-30
// ========================================================================
//! Engine boundary intake API methods.

use super::*;
use crate::simulation::engine_inputs;

// ========================================================================
// ENGINE INTAKE API
// ========================================================================
#[godot_api(secondary)]
impl SimulationNode {
    /// One call per delivery from the shell: every evaluated array and
    /// the probe grid's world geometry. Returns the store's new revision.
    /// The policy channel is gone with the layer that produced it; minds
    /// are living instances the actor path consumes directly.
    #[allow(clippy::too_many_arguments)]
    #[func]
    fn set_engine_inputs(
        &mut self,
        desirability: PackedFloat64Array,
        iron: PackedFloat64Array,
        coal: PackedFloat64Array,
        stone: PackedFloat64Array,
        origin_x: f64,
        origin_z: f64,
        spacing: f64,
        side: i64,
    ) -> i64 {
        engine_inputs::store(
            desirability.to_vec(),
            iron.to_vec(),
            coal.to_vec(),
            stone.to_vec(),
            origin_x,
            origin_z,
            spacing,
            side.max(0) as usize,
        ) as i64
    }

    /// The store's revision, for the shell to confirm delivery.
    #[func]
    fn engine_inputs_revision(&self) -> i64 {
        engine_inputs::snapshot().revision as i64
    }

    /// Derives untouched terrain from the engine ground field, the exact
    /// evaluation the renderer draws, through the bit-exact twin; sculpted
    /// samples stay as measured overrides. Scale is the shell's metres per
    /// field unit, so the two sides can never fork. Returns samples filled.
    #[func]
    fn apply_engine_ground(
        &mut self,
        footprint: f64,
        t: f64,
        seed: i64,
        amplitude: f64,
        scale: f64,
    ) -> i64 {
        let filled = self
            .lock_core()
            .apply_engine_ground_internal(footprint, t, seed, amplitude, scale);
        if filled > 0 {
            self.refresh_snapshot_from_core();
        }
        filled as i64
    }

    /// The parcels' world bounding box, so the shell can aim the probe
    /// grid at the city instead of the camera. Count zero means no city
    /// yet and the shell keeps its listener-centred fallback.
    #[func]
    fn engine_parcel_bounds(&mut self) -> VarDictionary {
        let core = self.lock_core();
        let mut count: i64 = 0;
        let mut min_x = f32::INFINITY;
        let mut min_z = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_z = f32::NEG_INFINITY;
        for parcel in core.zoning.parcels.parcels() {
            let c = parcel.center();
            min_x = min_x.min(c.x);
            min_z = min_z.min(c.y);
            max_x = max_x.max(c.x);
            max_z = max_z.max(c.y);
            count += 1;
        }
        let mut d = VarDictionary::new();
        let _ = d.insert("count", count);
        if count > 0 {
            let _ = d.insert("min_x", f64::from(min_x));
            let _ = d.insert("min_z", f64::from(min_z));
            let _ = d.insert("max_x", f64::from(max_x));
            let _ = d.insert("max_z", f64::from(max_z));
        }
        d
    }

    /// Every committed extraction site's position and depletion, the
    /// city's testimony for the boundary's upward direction: the shell
    /// aggregates these into deposit grids the engine reads as measured
    /// rows.
    #[func]
    fn get_extractor_sites(&mut self) -> VarArray {
        let core = self.lock_core();
        let mut out = VarArray::new();
        for site in core.resource_extraction.sites() {
            let Some(building) = core.allocator.buildings.get(site.building_idx) else {
                continue;
            };
            let mut d = VarDictionary::new();
            d.set("x", f64::from(building.center_x));
            d.set("z", f64::from(building.center_y));
            d.set("resource", GString::from(site.resource_id.as_str()));
            d.set("extracted_units", f64::from(site.extracted_units));
            d.set(
                "remaining_units",
                f64::from(site.remaining_reserve_units()),
            );
            d.set("area_m2", f64::from(site.area_m2));
            out.push(&d.to_variant());
        }
        out
    }

    /// Counts and means, which is what a drill asserts on.
    #[func]
    fn engine_inputs_summary(&self) -> VarDictionary {
        let g = engine_inputs::snapshot();
        let mean = |v: &Vec<f64>| -> f64 {
            if v.is_empty() {
                0.0
            } else {
                v.iter().sum::<f64>() / v.len() as f64
            }
        };
        let mut d = VarDictionary::new();
        let _ = d.insert("revision", g.revision as i64);
        let _ = d.insert("parcels", g.desirability.len() as i64);
        let _ = d.insert("desirability_mean", mean(&g.desirability));
        let _ = d.insert("iron_mean", mean(&g.iron));
        let _ = d.insert("coal_mean", mean(&g.coal));
        let _ = d.insert("stone_mean", mean(&g.stone));
        let _ = d.insert("side", g.side as i64);
        d
    }
}
