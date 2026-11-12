// SPDX-License-Identifier: GPL-2.0-only

//! The streaming core's layers: per renderer, the state that decides what it draws, its
//! product-to-install mapping and the display rules that are its own (`docs/terrain.md`
//! "Streaming core" 5.6). The graded tiles ([`graded`]) are the first; the vegetation patches
//! ([`vegetation`]) install through the backend's pools (`RENDER-32-E`), and so do the building
//! LOD batches ([`buildings`], `RENDER-32-L`). The terrain and water patches ([`terrain`],
//! `RENDER-32-M`) decide here and install through the scripts' pooled nodes; so do a load's road
//! chunks ([`roads`], `RENDER-32-P`), through the road tool's own commit.

/// Building LOD batches drawn through the backend's pools (`RENDER-32-L`).
pub(crate) mod buildings;
pub(crate) mod graded;
/// Parked car chunks drawn through the backend's pools (`RENDER-32-P`).
pub(crate) mod parked;
/// Road chunk hydration at a load, nearest first under a budget (`RENDER-32-P`).
pub(crate) mod roads;
/// Terrain and water patch residency, levels of detail and prewarm (`RENDER-32-M`).
pub(crate) mod terrain;
/// The zoning ground masks' resident chunks on a 2 km ring (`RENDER-32-P`).
pub(crate) mod zoning;
/// Vegetation patches installed through the backend's pools (`RENDER-32-E`).
pub(crate) mod vegetation;
