// SPDX-License-Identifier: GPL-2.0-only

//! The streaming core (`RENDER-32`, `docs/terrain.md` "Streaming core"): render-consumer state
//! that decides what the chunked renderers draw from the view, kept in Rust. Never simulation
//! authority, never saved: a world load or a renderer reset rebuilds it.
//!
//! It grows by its consumers. The graded tile renderer (`TERRAIN-13` part 1, round G1) is the
//! first: its levels and their travel buckets ([`levels`]) and its bake request queue
//! ([`requests`]), both measured from one [`view::StreamView`].

pub(crate) mod levels;
pub(crate) mod requests;
pub(crate) mod view;
