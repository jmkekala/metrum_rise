// SPDX-License-Identifier: GPL-2.0-only

//! Yard planting contract for building assets: lawn areas a spawned building plants.
//!
//! The areas are asset-local polygons; the simulation fills them when the building spawns, from
//! a seed of the building's own, so two copies of one asset grow different yards.

use serde::Deserialize;

/// What a yard planting area grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YardPlantKind {
    /// Yard trees: birch-led broadleaf with some conifers, standing apart.
    Trees,
    /// Yard shrubs: lilac, spirea, rose, cotoneaster, mugo pine and juniper.
    Bushes,
    /// A few trees among shrubs and flowers.
    Mixed,
    /// A flower bed: peony, lupine, phlox, hydrangea, rhododendron and summer bedding plants.
    Flowers,
}

impl YardPlantKind {
    /// Every kind, in editor order.
    pub const ALL: [Self; 4] = [Self::Trees, Self::Bushes, Self::Mixed, Self::Flowers];

    /// Parses the manifest and editor spelling.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// Manifest and editor spelling.
    pub fn name(self) -> &'static str {
        match self {
            Self::Trees => "trees",
            Self::Bushes => "bushes",
            Self::Mixed => "mixed",
            Self::Flowers => "flowers",
        }
    }
}

/// One authored planting area (`[[building.yard_planting]]`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YardPlanting {
    /// What the area grows.
    pub plants: YardPlantKind,
    /// Editor label; not used by the simulation.
    #[serde(default)]
    pub name: String,
    /// Asset-local `[x, z]` polygon, in the same frame as site surfaces.
    pub vertices: Vec<[f32; 2]>,
}
