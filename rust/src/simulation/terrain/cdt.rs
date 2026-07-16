//! Deterministic constrained triangulation for road-touched terrain patches.
//!
//! The public data contract is kept in [`model`]. The remaining modules form a one-way
//! pipeline from canonical input through constrained triangulation, face ownership, and
//! diagnostics. No stage depends on Godot types.

#![cfg_attr(not(test), allow(dead_code))]

mod builder;
mod canonicalize;
mod constraints;
mod diagnostics;
mod face_classification;
mod loop_clip;
mod model;
mod seam_quality;

pub(crate) use builder::build_road_touched_terrain_patch;
pub(crate) use model::*;

use canonicalize::*;
use constraints::*;
use diagnostics::*;
use face_classification::*;
use loop_clip::*;
use seam_quality::*;

#[cfg(test)]
mod tests;
