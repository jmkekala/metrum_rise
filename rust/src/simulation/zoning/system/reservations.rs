// SPDX-License-Identifier: GPL-2.0-only

//! Local external-placement queries against both authored parcels and reserved cell interiors.

use super::ZoningSystem;
use crate::simulation::agriculture::PolygonFootprint;
use crate::simulation::network::roadbed::{PieceMesh, RoadVec3};
use crate::simulation::zoning::cells::{CellBounds, road_contact_interior};
use glam::DVec2;

impl ZoningSystem {
    /// Tests arbitrary (including concave) field/site footprints against both zoning workflows.
    pub(crate) fn overlaps_reservation(&self, footprint: &PolygonFootprint) -> bool {
        self.parcels.overlaps_polygon(footprint) || self.cells_overlap_footprint(footprint, false)
    }

    /// Tests an external saved site against cell authority without changing legacy parcel policy.
    pub(crate) fn cells_overlap_site(&self, footprint: &PolygonFootprint) -> bool {
        self.cells_overlap_footprint(footprint, false)
    }

    /// Tests a proposed road corridor against painted cells and occupied grace claims locally.
    /// Edits test the pieces they install instead ([`Self::cells_overlap_road_piece`]); tests
    /// use the corridor to show what the pieces reach beyond it.
    #[cfg(test)]
    pub(crate) fn cells_overlap_road_corridor(
        &self,
        points: &[godot::prelude::Vector3],
        half_width_m: f32,
    ) -> bool {
        if !self.cells.has_reservations() {
            return false;
        }
        points.windows(2).any(|pair| {
            let a = DVec2::new(f64::from(pair[0].x), f64::from(pair[0].z));
            let b = DVec2::new(f64::from(pair[1].x), f64::from(pair[1].z));
            let normal = (b - a).normalize_or_zero().perp() * f64::from(half_width_m);
            normal.length_squared() > 0.0
                && self.cells_overlap_road_points(
                    [a - normal, b - normal, b + normal, a + normal]
                        .into_iter()
                        .map(|p| [p.x, p.y]),
                )
        })
    }

    /// Whether a road surface polygon given by its world points overlaps reserved cells.
    pub(crate) fn cells_overlap_road_points_world(&self, points: &[RoadVec3]) -> bool {
        if !self.cells.has_reservations() {
            return false;
        }
        self.cells_overlap_road_points(points.iter().map(|p| [p.x, p.z]))
    }

    /// Whether the top surface of one road piece overlaps reserved cells (`ROAD-68` open
    /// question 1): each candidate cell in the piece's bounds against its top triangles. Cells
    /// that only touch the road along their frontage do not overlap. O(candidate cells * top
    /// triangles), nothing beyond one bounds query when no cell is reserved near the piece.
    pub(crate) fn cells_overlap_road_piece(&self, mesh: &PieceMesh) -> bool {
        if !self.cells.has_reservations() {
            return false;
        }
        let [min_x, min_z, max_x, max_z] = mesh.bounds_xz();
        let bounds = CellBounds {
            min: DVec2::new(min_x, min_z),
            max: DVec2::new(max_x, max_z),
        };
        let top = &mesh.triangles()[..mesh.top_triangle_count()];
        self.reserved_cells_overlap(bounds, true, |corners| {
            let cell = PolygonFootprint::from_precise_points(corners.iter().map(|p| [p.x, p.y]));
            top.iter().any(|triangle| {
                let points = triangle.map(|vertex| {
                    let point = mesh.positions()[vertex as usize];
                    [point.x, point.z]
                });
                cell.overlaps_precise_points(&points)
            })
        })
    }

    fn cells_overlap_road_points(&self, points: impl Iterator<Item = [f64; 2]> + Clone) -> bool {
        let bounds = CellBounds::from_points(points.clone().map(DVec2::from_array));
        let mut footprint = None;
        // Most surface pieces have no local reservation candidate. Prepare their exact
        // overlay contour only after the existing block/mask index finds one.
        self.reserved_cells_overlap(bounds, true, |corners| {
            footprint
                .get_or_insert_with(|| PolygonFootprint::from_precise_points(points.clone()))
                .overlaps_precise_points(&corners.map(|p| [p.x, p.y]))
        })
    }

    fn cells_overlap_footprint(&self, footprint: &PolygonFootprint, road_contact: bool) -> bool {
        if !self.cells.has_reservations() {
            return false;
        }
        let bounds = CellBounds {
            min: DVec2::new(f64::from(footprint.min.x), f64::from(footprint.min.y)),
            max: DVec2::new(f64::from(footprint.max.x), f64::from(footprint.max.y)),
        };
        self.reserved_cells_overlap(bounds, road_contact, |corners| {
            footprint.overlaps_precise_points(&corners.map(|p| [p.x, p.y]))
        })
    }

    fn reserved_cells_overlap(
        &self,
        bounds: CellBounds,
        road_contact: bool,
        mut test: impl FnMut([DVec2; 4]) -> bool,
    ) -> bool {
        let mut overlaps = false;
        self.cells
            .visit_reserved_footprints_in_bounds(bounds, |_, corners| {
                if overlaps {
                    return;
                }
                let corners = if road_contact {
                    road_contact_interior(&corners)
                } else {
                    corners
                };
                overlaps = test(corners);
            });
        overlaps
    }
}
