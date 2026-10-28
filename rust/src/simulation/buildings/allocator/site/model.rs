// SPDX-License-Identifier: GPL-2.0-only

//! Building-site runtime data shared by derivation, queries, and terrain export.

use super::geometry::polygon_slice_bounds;
use crate::assets::SiteSurfaceMaterial;
use godot::prelude::Vector2;

/// Runtime surface polygon authored inside a building site.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BuildingSiteSurfaceClient {
    /// Material class for the surface.
    pub(crate) material: SiteSurfaceMaterial,
    /// Editor-authored label used by diagnostics.
    pub(crate) name: String,
    /// World-space polygon vertices.
    pub(crate) vertices_world: Vec<Vector2>,
}

/// Runtime client that owns the required flat building-site support surface.
#[derive(Clone, Debug)]
pub(crate) struct BuildingSiteClient {
    /// World-space flat support footprint corners.
    pub(crate) footprint_world: Vec<Vector2>,
    /// World-space lot reservation corners.
    pub(crate) lot_footprint_world: [Vector2; 4],
    /// Flat building/yard support height; edge paving outside the pad follows engineered ground.
    pub(crate) support_height_m: f32,
    /// Authored site surface polygons transformed into world space.
    pub(crate) surfaces: Vec<BuildingSiteSurfaceClient>,
    /// World-space footprint of each mesh part's imported bounds: the walls a yard plant keeps
    /// off. Empty when the asset has no imported bounds.
    pub(crate) structure_world: Vec<[Vector2; 4]>,
    /// World-space yard planting areas, where a yard tree may stand nearer the house than a
    /// tree's usual room allows.
    pub(crate) planting_world: Vec<Vec<Vector2>>,
}

/// One side of a site change for the graded terrain's dirty tiles (`TERRAIN-07-C`): the site
/// before or after it, by the bounds of its lot, support footprint and surfaces
/// (`[min_x, min_z, max_x, max_z]`, the legacy outbox's box) and its support height then. A
/// placement records the new site, a removal the old one, a rebuild both. Every pad seam lies
/// in the bounds at that height and every material edge in them, which is all the selection
/// reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SiteChange {
    /// XZ bounds of the lot, support footprint and surfaces.
    pub bounds: [f32; 4],
    /// Flat support height, metres.
    pub support_height_m: f32,
}

/// Minimal immutable building-site data needed by asynchronous terrain jobs.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BuildingSiteTerrainSnapshot {
    pub(super) sites: Vec<BuildingSiteTerrainClient>,
    // The allocator's largest lot radius when the snapshot was taken: the reach of its graded
    // terrain site queries (`TERRAIN-07-E`).
    pub(super) max_site_radius_m: f32,
}

/// One stable building-site footprint detached from the authoritative allocator.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BuildingSiteTerrainClient {
    pub(super) building_idx: usize,
    // The lot centre the graded terrain's site test reads (`TERRAIN-07-E`).
    pub(super) lot_center: Vector2,
    pub(super) footprint_world: Vec<Vector2>,
    pub(super) support_height_m: f32,
    pub(super) surfaces: Vec<BuildingSiteSurfaceClient>,
}

impl BuildingSiteClient {
    pub(crate) fn bounds(&self) -> (f32, f32, f32, f32) {
        site_surface_bounds(&self.footprint_world, &self.surfaces)
    }

    pub(crate) fn lot_bounds(&self) -> (f32, f32, f32, f32) {
        polygon_slice_bounds(&self.lot_footprint_world)
    }

    pub(super) fn overlaps_bounds(&self, min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> bool {
        let (site_min_x, site_min_z, site_max_x, site_max_z) = self.bounds();
        site_min_x <= max_x && site_max_x >= min_x && site_min_z <= max_z && site_max_z >= min_z
    }

    pub(crate) fn surface_debug_summary(&self) -> String {
        if self.surfaces.is_empty() {
            return "none".to_owned();
        }
        self.surfaces
            .iter()
            .map(|surface| {
                let material = match surface.material {
                    SiteSurfaceMaterial::Asphalt => "asphalt",
                    SiteSurfaceMaterial::Concrete => "concrete",
                };
                if surface.name.is_empty() {
                    material.to_owned()
                } else {
                    format!("{}:{}", material, surface.name)
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

pub(super) fn site_surface_bounds(
    footprint: &[Vector2],
    surfaces: &[BuildingSiteSurfaceClient],
) -> (f32, f32, f32, f32) {
    surfaces
        .iter()
        .fold(polygon_slice_bounds(footprint), |bounds, surface| {
            let other = polygon_slice_bounds(&surface.vertices_world);
            (
                bounds.0.min(other.0),
                bounds.1.min(other.1),
                bounds.2.max(other.2),
                bounds.3.max(other.3),
            )
        })
}
