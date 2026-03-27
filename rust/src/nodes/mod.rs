//! Godot node wrappers — the bridge between GDScript and the Rust simulation.
//!
//! [`simulation_node::SimulationNode`] exposes all `#[func]` API methods called by GDScript.
//! It owns all simulation subsystems and drives the tick loop from Godot's `_process` callback.
//!
//! **Do not add simulation logic here.** All game-state decisions belong in `simulation/`.
//! This module is thin glue: marshal data in, call into Rust, marshal results out.

pub mod camera_node;
pub mod sim;
pub mod simulation_node;
