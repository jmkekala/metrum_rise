// SPDX-License-Identifier: GPL-2.0-only

//! The streaming core's layers: per renderer, the state that decides what it draws, its
//! product-to-install mapping and the display rules that are its own (`docs/terrain.md`
//! "Streaming core" 5.6). The graded tiles ([`graded`]) are the first; the vegetation patches
//! ([`vegetation`]) install through the backend's pools (`RENDER-32-E`), and so do the building
//! LOD batches ([`buildings`], `RENDER-32-L`).

/// Building LOD batches drawn through the backend's pools (`RENDER-32-L`).
pub(crate) mod buildings;
pub(crate) mod graded;
/// Vegetation patches installed through the backend's pools (`RENDER-32-E`).
pub(crate) mod vegetation;
