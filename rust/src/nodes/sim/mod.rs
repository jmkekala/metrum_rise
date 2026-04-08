//! Simulation node submodules for modular Godot-Rust bridge organization.

// pub mod api; (Moved to simulation_node.rs for macro sanity)
pub mod asset_export;
pub mod benchmark;
pub mod bridge;
pub mod core;
pub mod editing;
pub mod query;
/// Rendering bridge sub-modules for Godot-Rust interaction.
pub mod render;
pub mod save_load;
pub mod undo;
