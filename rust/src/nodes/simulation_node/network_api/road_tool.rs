// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool preview, cursor snapping, and surface-debug Godot API methods.

mod cursor;
mod ghost;
mod preview;
pub(super) use preview::{core_lot_outlines, view_lot_outlines};
mod surface_debug;
