// SPDX-License-Identifier: GPL-2.0-only

//! The graded tile renderer's state in Rust (`TERRAIN-13` part 2, `RENDER-32-I`, round G2a),
//! ported from `terrain_graded_tiles.gd` and `terrain_graded_pages.gd` by record and replay (the
//! GDScript renderer's traces, `terrain_graded_trace.gd`): first the page allocator ([`pages`]),
//! then the copies, growth and installed tiles. [`ordered`] keeps the GDScript dictionaries'
//! iteration order, which the traces' outputs follow.

pub(crate) mod book;
pub(crate) mod copies;
pub(crate) mod ops;
pub(crate) mod ordered;
pub(crate) mod pages;
pub(crate) mod state;
pub(crate) mod tiles;
