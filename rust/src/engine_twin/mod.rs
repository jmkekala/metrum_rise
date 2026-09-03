// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: mod.rs
//  script_path: rust/src/engine_twin/mod.rs
//  module_name: engine_twin
//  version: 0.1.1
//  author: [BantedHam]
//  description: Rust twins of 2.5D engine kernels, promoted one at a time
//           under the same gate the GLSL twins pass: bit-exact against
//           the GDScript reference on recorded fixtures. Metrum-side
//           code consuming the engine's published math; nothing here
//           ships into the addon.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: [kernel-twins]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-30
// =========================================================================
//! Rust twins of 2.5D engine kernels, promoted under the bit-exact gate.

pub mod fbm;
