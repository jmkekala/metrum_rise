// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: mod.rs
//  script_path: rust/src/simulation/buildings/mod.rs
//  module_name: mod
//  version: 0.1.0
//  description: Building placement and lifecycle. See
//           [`allocator::BuildingAllocator`].
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Building placement and lifecycle. See [`allocator::BuildingAllocator`].

// ========================================================================
// SUBMODULES
// ========================================================================

pub mod allocator;
/// Frontage roles and which edge classes accept them.
pub mod frontage;
