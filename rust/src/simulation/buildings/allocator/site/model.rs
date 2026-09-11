// SPDX-License-Identifier: GPL-2.0-only

//! Building-site runtime data shared by derivation, queries, and terrain export.

use super::geometry::{polygon_quad_bounds, polygon_slice_bounds};
use crate::assets::SiteSurfaceMaterial;
use godot::prelude::{Vector2, Vector3};
use std::sync::OnceLock;

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
    /// Derived once per site revision; rendering never retriangulates the city.
    pub(crate) foundation_mesh: OnceLock<Vec<(Option<SiteSurfaceMaterial>, [Vector3; 3])>>,
    /// World-space flat support footprint corners.
    pub(crate) footprint_world: Vec<Vector2>,
    /// World-space lot reservation corners.
    pub(crate) lot_footprint_world: [Vector2; 4],
    /// Flat building/yard support height; edge paving outside the pad follows engineered ground.
    pub(crate) support_height_m: f32,
    /// Authored site surface polygons transformed into world space.
    pub(crate) surfaces: Vec<BuildingSiteSurfaceClient>,
}

/// Minimal immutable building-site data needed by asynchronous terrain jobs.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BuildingSiteTerrainSnapshot {
    pub(super) sites: Vec<BuildingSiteTerrainClient>,
}

/// One stable building-site footprint detached from the authoritative allocator.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BuildingSiteTerrainClient {
    pub(super) building_idx: usize,
    pub(super) footprint_world: Vec<Vector2>,
    pub(super) support_height_m: f32,
    pub(super) surfaces: Vec<BuildingSiteSurfaceClient>,
}

impl BuildingSiteClient {
    pub(crate) fn bounds(&self) -> (f32, f32, f32, f32) {
        site_surface_bounds(&self.footprint_world, &self.surfaces)
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
