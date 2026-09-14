// SPDX-License-Identifier: GPL-2.0-only

//! Vegetation parameter metadata for menus that run before a world exists.
//!
//! The main menu holds no [`crate::nodes::simulation_node::SimulationNode`], so the new-game
//! dialog cannot ask a loaded simulation what the shipped defaults or the renderer's density
//! ceiling are. This class carries those numbers across the boundary so GDScript does not keep
//! a second copy of them that drifts when `RENDER-04` raises the ceiling.

use crate::simulation::vegetation::{VegetationConfig, canopy_cell_m_for_density};
use godot::prelude::*;

/// Read-only source of authored vegetation defaults and bounds for menu code.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct VegetationOptions;

#[godot_api]
impl VegetationOptions {
    /// Returns the parameters a new game starts with unless the player changes them.
    #[func]
    pub fn get_default_config(&self) -> VarDictionary {
        let config = VegetationConfig::default();
        let mut dict = VarDictionary::new();
        dict.set("enabled", config.enabled);
        dict.set("seed", i64::from(config.seed));
        dict.set("coverage", f64::from(config.coverage));
        dict.set("canopy_stems_per_ha", f64::from(config.canopy_stems_per_ha));
        dict
    }

    /// Returns the inclusive canopy density range, in stems per hectare, that is accepted.
    ///
    /// The ceiling is a renderer limit rather than a generator one, so a dialog must read it
    /// here instead of assuming the value that shipped.
    #[func]
    pub fn get_density_bounds(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set(
            "min_canopy_stems_per_ha",
            f64::from(VegetationConfig::MIN_CANOPY_STEMS_PER_HA),
        );
        dict.set(
            "max_canopy_stems_per_ha",
            f64::from(VegetationConfig::MAX_CANOPY_STEMS_PER_HA),
        );
        dict
    }

    /// Returns the canopy grid spacing in metres that a density produces, for menu readouts.
    #[func]
    pub fn get_canopy_spacing_m(&self, canopy_stems_per_ha: f32) -> f32 {
        canopy_cell_m_for_density(canopy_stems_per_ha)
    }
}
