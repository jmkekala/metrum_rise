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
mod thread_pools;
mod utils;

struct MetrumRiseExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MetrumRiseExtension {
    fn on_stage_init(stage: InitStage) {
        if stage == InitStage::Core {
            // First, before any extension code runs Rayon work: from here on that work runs in
            // the shared pool, which the Scene deinit hook joins (CODE-17).
            if !crate::thread_pools::disable_global_pool() {
                godot_error!(
                    "Rayon's global pool started before the extension disabled it; its threads \
                     can outlive the library (CODE-17)"
                );
            }
            crate::debug::init();
        }
        if stage == InitStage::Scene {
            refuse_tests_in_the_players_profile();
        }
    }

    // Godot deinitializes Scene after deleting the scene tree and unloads the library soon
    // after, so a thread the extension started that still runs then executes unmapped code
    // (CODE-17). Nodes join their own threads when freed; the shared Rayon pool and the
    // process-wide watchdog stop here.
    fn on_stage_deinit(stage: InitStage) {
        if stage == InitStage::Scene {
            crate::thread_pools::stop();
            crate::debug::shutdown();
        }
    }
}

// `TOOLS-19` part 2: a test script, benchmark matrix or probe started in the player's real profile
// exits non-zero here, before any script runs and before it writes to user://
// (`debug::profile_guard`). The Scene stage runs before Godot creates the main loop or a
// `--script` main loop.
fn refuse_tests_in_the_players_profile() {
    use crate::debug::profile_guard::{LaunchFacts, default_data_dir, refusal};
    use godot::classes::{Os, ProjectSettings};
    let mut os = Os::singleton();
    let (given, user) = (os.get_cmdline_args(), os.get_cmdline_user_args());
    let args: Vec<String> = given
        .as_slice()
        .iter()
        .chain(user.as_slice())
        .map(|arg| arg.to_string())
        .collect();
    let project_name = ProjectSettings::singleton()
        .get_setting("application/config/name")
        .to_string();
    let data_dir = os.get_data_dir().to_string();
    let facts = LaunchFacts {
        args: &args,
        project_name: &project_name,
        data_dir: &data_dir,
        default_data_dir: default_data_dir(&os.get_name().to_string(), |name| {
            std::env::var(name).ok()
        }),
    };
    if let Some(reason) = refusal(&facts) {
        godot_error!("{reason}");
        std::process::exit(3);
    }
}
