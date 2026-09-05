// SPDX-License-Identifier: GPL-2.0-only

//! Graph metadata rebuild phases for adjacency, clips, profiles, and terrain sync.

mod adjacency;
mod clips;
mod compaction;
mod junction_profiles;
mod terrain_sync;

pub(crate) use junction_profiles::{JUNCTION_PROFILE_BLEND_ZONE_M, JunctionEndpointProfilePlane};

#[cfg(test)]
mod tests;
