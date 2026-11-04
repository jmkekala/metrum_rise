// SPDX-License-Identifier: GPL-2.0-only

//! The drawn ground under roadside furniture that stands off the compiled bands (`ROAD-59`): a
//! street light on the verge of a road without sidewalks, a footpath's post-top light, a sign
//! beside a road without sidewalks. Such a pole stands on what is drawn there: the top of a
//! grounded road where one holds the point (the cursor's ground-level rule), else the graded
//! ground the terrain renders (`TERRAIN-07-F`). A pole on a sidewalk takes its band's height and
//! never asks.
//!
//! The ground is graded by the roadbed alone, without building sites: a road chunk is emitted
//! again when its roadbed or terrain changes, never when a building appears, so a height that
//! read the sites would depend on when the chunk was last emitted (a save and load would move
//! it), and the site index is not kept ready for the chunk emission. Where a building pad grades
//! the ground beside a verge, a pole there follows the road's grading, not the pad's.

use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::graded::{GradedSurface, NoSites, QueryScratch};

/// One placement pass's ground query: the graded surface over the pass's terrain and roadbed,
/// and one scratch, warm after its first query, so a pole allocates nothing. A query is one walk
/// of the roadbed's query cell plus one graded point query: `O(P + V / 8)` for the pieces and
/// chain segments met, `O(K log K)` for the seams kept, `O(K)` per lattice point. Without the
/// per-64 m query index (the bridge piers' choice): the gather reference gives the same value
/// bitwise and keeps the emission independent of the index's freshness.
pub(super) struct VergeGround<'a> {
    surface: GradedSurface<'a>,
    scratch: QueryScratch,
}

impl<'a> VergeGround<'a> {
    pub(super) fn new(terrain: &'a TerrainSystem, roadbed: &'a RoadbedSystem) -> Self {
        Self {
            surface: GradedSurface {
                terrain,
                roadbed,
                sites: &NoSites,
                query_index: None,
            },
            scratch: QueryScratch::default(),
        }
    }

    /// The drawn height at a world XZ point, metres.
    pub(super) fn height_at(&mut self, x: f64, z: f64) -> f32 {
        self.surface
            .roadbed
            .grounded_height_at(x as f32, z as f32)
            .unwrap_or_else(|| self.surface.ground_height_at(x, z, &mut self.scratch) as f32)
    }
}
