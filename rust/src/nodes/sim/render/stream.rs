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
//! of its own ([`layers`]). Round V2a (`RENDER-32-E`) adds the install side: the
//! `RenderingServer` backend and its slot pools ([`backend`]), the frame's streaming budget and
//! install order ([`schedule`]), each layer's status ([`status`]) and the vegetation layer that
//! installs through them, run each frame by the `StreamNode`. Round V2b (`RENDER-32-F`) adds the
//! rings and travel buckets ([`rings`]) and moves the vegetation's residency into its layer.
//! Round T1 (`RENDER-32-M`) decides the terrain and water patches' residency, levels of detail and
//! prewarm in a layer ([`layers`]) whose units the scripts execute on their pooled nodes.
//! Round C1 (`RENDER-32-N`) adds the stream benchmark's drawn probe ([`probe`]): entered-to-drawn
//! and in-view holes per layer, benchmark only.

pub(crate) mod backend;
pub(crate) mod cache;
pub(crate) mod layers;
pub(crate) mod levels;
pub(crate) mod prepare;
pub(crate) mod probe;
pub(crate) mod requests;
pub(crate) mod rings;
pub(crate) mod schedule;
pub(crate) mod status;
pub(crate) mod view;
