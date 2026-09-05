// SPDX-License-Identifier: GPL-2.0-only

//! Building-site runtime data shared by derivation, queries, and terrain export.

use super::geometry::{polygon_quad_bounds, polygon_slice_bounds};
use crate::assets::SiteSurfaceMaterial;
use godot::prelude::Vector2;

/// Runtime surface polygon authored inside a building site.
#[derive(Clone, Debug)]
pub(crate) struct BuildingSiteSurfaceClient {
    /// Material class for the surface.
    pub(crate) material: SiteSurfaceMaterial,
    /// Editor-authored label used by diagnostics.
    pub(crate) name: String,
    /// World-space height of the surface top.
    pub(crate) height_m: f32,
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
    /// Flat support height shared by the building and authored site surfaces.
    pub(crate) support_height_m: f32,
    /// Authored site surface polygons transformed into world space.
    pub(crate) surfaces: Vec<BuildingSiteSurfaceClient>,
}

/// Minimal immutable building-site data needed by asynchronous terrain jobs.
#[derive(Clone, Debug, Default)]
pub(crate) struct BuildingSiteTerrainSnapshot {
    pub(super) sites: Vec<BuildingSiteTerrainClient>,
}

/// One stable building-site footprint detached from the authoritative allocator.
#[derive(Clone, Debug)]
pub(super) struct BuildingSiteTerrainClient {
    pub(super) building_idx: usize,
    pub(super) footprint_world: Vec<Vector2>,
    pub(super) support_height_m: f32,
}

impl BuildingSiteClient {
    pub(crate) fn bounds(&self) -> (f32, f32, f32, f32) {
        polygon_slice_bounds(&self.footprint_world)
    }

    pub(crate) fn lot_bounds(&self) -> (f32, f32, f32, f32) {
        polygon_quad_bounds(self.lot_footprint_world)
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
