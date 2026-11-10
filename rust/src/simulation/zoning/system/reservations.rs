// SPDX-License-Identifier: GPL-2.0-only

//! Local external-placement queries against both authored parcels and reserved cell interiors.

use super::ZoningSystem;
use crate::simulation::agriculture::PolygonFootprint;
use crate::simulation::network::roadbed::{PieceMesh, RoadVec3};
use crate::simulation::zoning::cells::{
    CellBounds, CellKey, CellMaps, CellStore, road_contact_interior,
};
use glam::DVec2;

// Whether a piece's top triangles overlap a polygon with positive area. Only a piece on the
// ground covers land: a buried piece's top lies in the hill under the cells (`ROAD-61` round
// R8h), a bridge's deck above them (`ZONE-09`, decision D2), and neither covers any.
fn piece_covers(mesh: &PieceMesh, polygon: &PolygonFootprint) -> bool {
    if !mesh.grounded() {
        return false;
    }
    let bounds = [polygon.min.x, polygon.min.y, polygon.max.x, polygon.max.y].map(f64::from);
    mesh.any_top_triangle_in_bounds(bounds, |triangle| {
        let points = mesh
            .triangle_points(triangle)
            .map(|point| [point.x, point.z]);
        polygon.overlaps_precise_points(&points)
    })
}

/// Whether one of an edit's road pieces newly covers a four-cornered footprint (a cell, a lot or
/// a parcel): its top triangles overlap the footprint's road-contact interior
/// ([`road_contact_interior`]) and those of no piece in `before`, the pieces the edit replaces,
/// do. A footprint that only touches the road along its frontage is not covered.
/// O(top triangles of `mesh` and `before` in the footprint's bounds).
pub(crate) fn road_piece_newly_covers(
    mesh: &PieceMesh,
    before: &[&PieceMesh],
    corners: &[DVec2; 4],
) -> bool {
    let interior = road_contact_interior(corners);
    let footprint = PolygonFootprint::from_precise_points(interior.iter().map(|p| [p.x, p.y]));
    piece_covers(mesh, &footprint) && !before.iter().any(|old| piece_covers(old, &footprint))
}

/// [`road_piece_newly_covers`] for land an edit takes beyond its pieces (`ZONE-10`, a
/// roundabout's island): `footprint` overlaps the four-cornered footprint's road-contact interior
/// with positive area and no piece in `before` covers it. O(`footprint` vertices + top triangles
/// of `before` in the footprint's bounds).
pub(crate) fn footprint_newly_covers(
    footprint: &PolygonFootprint,
    before: &[&PieceMesh],
    corners: &[DVec2; 4],
) -> bool {
    let interior = road_contact_interior(corners).map(|p| [p.x, p.y]);
    footprint.overlaps_precise_points(&interior) && {
        let interior = PolygonFootprint::from_precise_slice(&interior);
        !before.iter().any(|old| piece_covers(old, &interior))
    }
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
    /// Edits test the pieces they install instead
    /// ([`Self::reserved_cells_newly_covered_by_road_pieces`]); tests use the corridor to show
    /// what the pieces reach beyond it.
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

    /// Every reserved cell of `cells` that one of `pieces` newly covers, sorted by key and
    /// without repeats: the cells a road edit takes (`ZONE-09`, `ZONE-10`). Each candidate cell
    /// in a piece's bounds is tested against its top triangles, then against the pieces the edit
    /// replaces ([`road_piece_newly_covers`]). Cells are laid out against the straight road
    /// corridors, so cells beside an existing junction corner or road end may already lie under
    /// it; only land the edit newly covers is taken. Cells that only touch the road along their
    /// frontage are not covered. The commit passes the live store and the road tool's preview
    /// the view's persistent copy of it, so both decide alike (`ROAD-68` open question 25).
    /// O(pieces * candidate cells * top triangles), one bounds query per piece when no cell is
    /// reserved near it.
    pub(crate) fn reserved_cells_newly_covered_by_road_pieces<'a, M: CellMaps>(
        cells: &CellStore<M>,
        pieces: impl IntoIterator<Item = &'a PieceMesh>,
        before: &[&PieceMesh],
    ) -> Vec<CellKey> {
        let mut covered = Vec::new();
        if !cells.has_reservations() {
            return covered;
        }
        for mesh in pieces {
            cells.visit_reserved_footprints_in_bounds(piece_bounds(mesh), |key, corners| {
                if road_piece_newly_covers(mesh, before, &corners) {
                    covered.push(key);
                }
            });
        }
        covered.sort_unstable();
        covered.dedup();
        covered
    }

    fn cells_overlap_road_points(&self, points: impl Iterator<Item = [f64; 2]> + Clone) -> bool {
        let bounds = CellBounds::from_points(points.clone().map(DVec2::from_array));
        let mut footprint = None;
        // Most surface pieces have no local reservation candidate. Prepare their exact
        // overlay contour only after the existing block/mask index finds one.
        Self::reserved_cells_overlap(&self.cells, bounds, true, |corners| {
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
        Self::reserved_cells_overlap(&self.cells, bounds, road_contact, |corners| {
            footprint.overlaps_precise_points(&corners.map(|p| [p.x, p.y]))
        })
    }

    fn reserved_cells_overlap<M: CellMaps>(
        cells: &CellStore<M>,
        bounds: CellBounds,
        road_contact: bool,
        mut test: impl FnMut([DVec2; 4]) -> bool,
    ) -> bool {
        let mut overlaps = false;
        cells
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

// A piece's XZ bounds as a cell query box.
fn piece_bounds(mesh: &PieceMesh) -> CellBounds {
    let [min_x, min_z, max_x, max_z] = mesh.bounds_xz();
    CellBounds {
        min: DVec2::new(min_x, min_z),
        max: DVec2::new(max_x, max_z),
    }
}
