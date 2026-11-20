// SPDX-License-Identifier: GPL-3.0-or-later

//! Graph metadata rebuild phases for adjacency, clips, profiles, and terrain sync.

mod adjacency;
mod clips;
mod junction_profiles;
mod terrain_sync;

pub(crate) use junction_profiles::{JUNCTION_PROFILE_BLEND_ZONE_M, JunctionEndpointProfilePlane};

#[cfg(test)]
mod tests;
