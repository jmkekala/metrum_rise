use godot::prelude::*;

pub mod config;
pub mod simulation;
pub mod nodes;
pub mod utils;

struct MetrumRiseExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MetrumRiseExtension {}
