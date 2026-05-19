//! Runtime road-surface query module wiring.
//!
//! Query-facing code is split by consumer contract: terrain/CDT loop export,
//! visible-surface sampling/raycasting, visibility policy, and shared triangle
//! traversal helpers.

mod policy;
mod terrain_cdt;
mod traversal;
mod visible;
