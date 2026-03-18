use godot::prelude::*;

pub mod config;
mod simulation;
mod nodes;
mod utils;

struct MetrumRiseExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MetrumRiseExtension {}
