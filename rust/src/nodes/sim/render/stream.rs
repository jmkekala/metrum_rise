// SPDX-License-Identifier: GPL-2.0-only

//! The streaming core (`RENDER-32`, `docs/terrain.md` "Streaming core"): render-consumer state
//! that decides what the chunked renderers draw from the view, kept in Rust. Never simulation
//! authority, never saved: a world load or a renderer reset rebuilds it.
//!
//! It grows by its consumers. The graded tile renderer (`TERRAIN-13` part 1, round G1) is the
//! first: its levels and their travel buckets ([`levels`]) and its bake request queue
//! ([`requests`]), both measured from one [`view::StreamView`]. The vegetation patches (round
//! V1b, `RENDER-32-D`) are the second: their queries run on the background pool from a
//! snapshot, tracked by the prepare queue ([`prepare`]) and held for the poll by the product
//! cache ([`cache`]). The graded renderer's remaining state (round G2a, `RENDER-32-I`) is a layer
//! of its own ([`layers`]).

pub(crate) mod cache;
pub(crate) mod layers;
pub(crate) mod levels;
pub(crate) mod prepare;
pub(crate) mod requests;
pub(crate) mod view;
