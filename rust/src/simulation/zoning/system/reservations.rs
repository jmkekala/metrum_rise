// SPDX-License-Identifier: GPL-2.0-only

//! Local external-placement queries against both authored parcels and reserved cell interiors.

use super::ZoningSystem;
use crate::simulation::agriculture::PolygonFootprint;
use crate::simulation::network::roadbed::{PieceMesh, RoadVec3};
use crate::simulation::zoning::cells::{CellBounds, CellReservations, road_contact_interior};
use glam::DVec2;

// Whether a piece's top triangles overlap a polygon with positive area.
fn piece_covers(mesh: &PieceMesh, polygon: &PolygonFootprint) -> bool {
    let bounds = [polygon.min.x, polygon.min.y, polygon.max.x, polygon.max.y].map(f64::from);
    mesh.any_top_triangle_in_bounds(bounds, |triangle| {
        let points = mesh
            .triangle_points(triangle)
            .map(|point| [point.x, point.z]);
        polygon.overlaps_precise_points(&points)
    })
}

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
    /// Edits test the pieces they install instead ([`Self::cells_newly_covered_by_road_piece`]); tests
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

    /// Whether the top surface of one road piece covers a reserved cell of `cells` that none of
    /// `before` covered (`ROAD-68` open question 1): each candidate cell in the piece's bounds
    /// against its top triangles, then against the pieces the edit replaces. Cells are laid out
    /// against the straight road corridors, so cells beside an existing junction corner or road
    /// end may already lie under it; an edit is refused only for land it newly covers. Cells that
    /// only touch the road along their frontage are not covered. The commit passes the live
    /// store's index and the road tool's preview the view's copy, so both decide alike
    /// (open question 25). O(candidate cells * top triangles of the piece and of `before`), one
    /// bounds query when no cell is reserved near the piece.
    pub(crate) fn cells_newly_covered_by_road_piece(
        cells: &CellReservations,
        mesh: &PieceMesh,
        before: &[&PieceMesh],
    ) -> bool {
        if cells.is_empty() {
            return false;
        }
        let [min_x, min_z, max_x, max_z] = mesh.bounds_xz();
        let bounds = CellBounds {
            min: DVec2::new(min_x, min_z),
            max: DVec2::new(max_x, max_z),
        };
        let mut covered = false;
        cells.visit_footprints_in_bounds(bounds, |_, corners| {
            if covered {
                return;
            }
            let corners = road_contact_interior(&corners);
            let cell = PolygonFootprint::from_precise_points(corners.iter().map(|p| [p.x, p.y]));
            covered =
                piece_covers(mesh, &cell) && !before.iter().any(|old| piece_covers(old, &cell));
        });
        covered
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
