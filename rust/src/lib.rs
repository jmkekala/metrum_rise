// SPDX-License-Identifier: GPL-2.0-only

#![warn(missing_docs)]
//! Metrum Rise — simulation backend, compiled as a Godot 4 GDExtension (`libmetrum_rise.so`).
//!
//! All simulation logic runs in Rust. Godot calls into it through
//! [`nodes::simulation_node::SimulationNode`], which exposes `#[func]` methods
//! on the Godot side. A background fixed-step loop owns simulation advancement;
//! Godot's render thread submits commands and reads published snapshots.
//!
//! **Reading order for new contributors / AI models:**
//! 1. [`config`] — global constants (map size, grid cell sizes, lane widths).
//! 2. [`simulation::network::graph`] — road graph schema (`Node`, `Edge`, `RegionGraph`).
//! 3. [`simulation::economy::agents`] — agent state machine and SoA layout.
//! 4. [`simulation::zoning`] — road-aligned zoning parcels.
//! 5. [`simulation::pathing::cch`] — CCH/CRP pathfinding (bidirectional Dijkstra on contracted graph).

use godot::prelude::*;

pub mod assets;
pub mod config;
pub mod debug;
pub mod nodes;
pub mod simulation;
mod utils;

struct MetrumRiseExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MetrumRiseExtension {
    fn on_level_init(level: InitLevel) {
        if level == InitLevel::Core {
            crate::debug::init();
        }
    }
}
