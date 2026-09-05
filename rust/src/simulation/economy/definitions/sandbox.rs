// SPDX-License-Identifier: GPL-2.0-only

//! Authored economy sandbox playback for editor-facing scenario diagnostics.

mod bottlenecks;
mod inventory;
mod playback;
mod pricing;
mod types;

pub(super) use playback::run_sandbox;
