// SPDX-License-Identifier: GPL-2.0-only

//! Span region top quads rebuilt from compiled sections (`ROAD-44`).
//!
//! A span region is the strip of one band between two consecutive sections, so its top polygon
//! is a pure function of those sections. Span pieces keep their sections and rebuild each quad on
//! read, on the stack, instead of retaining a polygon and its triangles for every region.

use super::super::{
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceVisualPolygon, SurfaceTriangleGrid,
    WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2, backend::RoadVec3,
};
use super::{RoadSurfaceSpanOwnedRegion, RoadSurfaceSpanRegionRole, RoadSurfaceVisualSpanPiece};

/// A span top region or raised-step face, rebuilt from sections: up to four points and up to
/// two triangles.
#[derive(Clone, Copy, Debug)]
pub struct SpanQuad {
    points: [RoadVec3; 4],
    point_count: u8,
    triangles: [[RoadVec3; 3]; 2],
    triangle_count: u8,
}

impl PartialEq for SpanQuad {
    fn eq(&self, other: &Self) -> bool {
        self.points() == other.points() && self.triangles() == other.triangles()
    }
}

impl SpanQuad {
    // `make_visual_strip_polygon` for four corners, without allocating: drops consecutive and
    // closing duplicates, rejects degenerate or self-crossing strips, then fans from the first
    // point and keeps the triangles with XZ area.
    pub(in crate::simulation::network::surface) fn from_corners(
        corners: [RoadVec3; 4],
    ) -> Option<Self> {
        let duplicate = |a: RoadVec3, b: RoadVec3| {
            (a - b).length_squared() <= f64::from(WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2)
        };
        let mut points = [corners[0]; 4];
        let mut point_count = 1;
        for &corner in &corners[1..] {
            if !duplicate(corner, points[point_count - 1]) {
                points[point_count] = corner;
                point_count += 1;
            }
        }
        if point_count >= 2 && duplicate(points[0], points[point_count - 1]) {
            point_count -= 1;
        }
        if point_count < 3
            || RoadSurfaceSystem::polygon_has_strict_edge_crossing_xz(&points[..point_count])
        {
            return None;
        }
        let mut triangles = [[points[0]; 3]; 2];
        let mut triangle_count = 0;
        for index in 1..point_count - 1 {
            let triangle = [points[0], points[index], points[index + 1]];
            if RoadSurfaceSystem::triangle_has_area_xz(triangle) {
                triangles[triangle_count] = triangle;
                triangle_count += 1;
            }
        }
        (triangle_count > 0).then_some(Self {
            points,
            point_count: point_count as u8,
            triangles,
            triangle_count: triangle_count as u8,
        })
    }

    // `make_vertical_quad_polygon` without allocating.
    pub(in crate::simulation::network::surface) fn from_vertical_points(
        points: [RoadVec3; 4],
    ) -> Option<Self> {
        let (triangles, triangle_count) = RoadSurfaceSystem::vertical_quad_triangles(points)?;
        Some(Self {
            points,
            point_count: 4,
            triangles,
            triangle_count: triangle_count as u8,
        })
    }

    /// Ordered world-space polygon points.
    pub fn points(&self) -> &[RoadVec3] {
        &self.points[..usize::from(self.point_count)]
    }

    /// Deterministic fan triangles covering the polygon in world space.
    pub fn triangles(&self) -> &[[RoadVec3; 3]] {
        &self.triangles[..usize::from(self.triangle_count)]
    }

    /// Allocates the equivalent visual polygon for callers that need an owned one.
    pub fn to_polygon(&self) -> RoadSurfaceVisualPolygon {
        RoadSurfaceVisualPolygon::from_parts(self.points().to_vec(), self.triangles())
    }
}

impl RoadSurfaceSpanOwnedRegion {
    /// Start-left, end-left, end-right and start-right band corners from `sections`.
    pub(crate) fn corners(&self, sections: &[RoadSurfaceSection]) -> [RoadVec3; 4] {
        let start = &sections[self.start_section_index()];
        let end = &sections[self.end_section_index()];
        let band_index = self.owner().source_band_index;
        let band_start = &start.bands[band_index];
        let band_end = &end.bands[band_index];
        let point = RoadSurfaceSystem::section_boundary_world_point_static;
        [
            point(start, band_start.lateral_start_m, band_start.height_start_m),
            point(end, band_end.lateral_start_m, band_end.height_start_m),
            point(end, band_end.lateral_end_m, band_end.height_end_m),
            point(start, band_start.lateral_end_m, band_start.height_end_m),
        ]
    }

    /// The region's top quad, rebuilt from the sections it was resolved from.
    pub(crate) fn quad(&self, sections: &[RoadSurfaceSection]) -> SpanQuad {
        SpanQuad::from_corners(self.corners(sections))
            .expect("span regions are only resolved for valid strip quads")
    }
}

impl RoadSurfaceVisualSpanPiece {
    /// Top quad of one of this piece's owned or earthwork-support regions.
    pub(crate) fn region_quad(&self, region: &RoadSurfaceSpanOwnedRegion) -> SpanQuad {
        region.quad(&self.sections)
    }

    fn role_quads(
        &self,
        role: RoadSurfaceSpanRegionRole,
    ) -> impl ExactSizeIterator<Item = SpanQuad> + '_ {
        self.surface_polygon_order[usize::from(role.sort_key())]
            .iter()
            .map(|&index| self.region_quad(&self.span_owned_regions[index as usize]))
    }

    /// Asphalt top quads in render order.
    pub fn road_surface_polygons(&self) -> impl ExactSizeIterator<Item = SpanQuad> + '_ {
        self.role_quads(RoadSurfaceSpanRegionRole::Asphalt)
    }

    /// Curb or shoulder top quads in render order.
    pub fn curb_surface_polygons(&self) -> impl ExactSizeIterator<Item = SpanQuad> + '_ {
        self.role_quads(RoadSurfaceSpanRegionRole::CurbOrShoulder)
    }

    /// Sidewalk top quads in render order.
    pub fn sidewalk_surface_polygons(
        &self,
    ) -> impl ExactSizeIterator<Item = SpanQuad> + '_ {
        self.role_quads(RoadSurfaceSpanRegionRole::NonRoad)
    }

    // Grid over the top triangles, asphalt then curb then sidewalk in render order. Items
    // are `region_index << 1 | triangle_index`, so no triangle is stored twice.
    pub(super) fn surface_triangle_grid(&self) -> SurfaceTriangleGrid {
        let mut items = Vec::new();
        for order in &self.surface_polygon_order {
            for &region_index in order.iter() {
                let quad = self.region_quad(&self.span_owned_regions[region_index as usize]);
                for (triangle_index, &triangle) in quad.triangles().iter().enumerate() {
                    items.push((region_index << 1 | triangle_index as u32, triangle));
                }
            }
        }
        SurfaceTriangleGrid::from_triangles(items.iter().map(|item| item.1), |position| {
            items[position].0
        })
    }

    /// Resolves a `surface_query` item to its triangle and whether it is carriageway.
    pub(crate) fn surface_query_triangle(&self, item: u32) -> ([RoadVec3; 3], bool) {
        let region = &self.span_owned_regions[(item >> 1) as usize];
        (
            self.region_quad(region).triangles()[(item & 1) as usize],
            region.role() == RoadSurfaceSpanRegionRole::Asphalt,
        )
    }

    /// Top quads, asphalt then curb then sidewalk, whose four region corners pass `keep`.
    ///
    /// A quad's points are a subset of its corners, so a bounds test that rejects the corners
    /// also rejects the quad; this skips rebuilding quads outside a query's bounds.
    pub fn surface_polygons_where<'a>(
        &'a self,
        keep: impl Fn(&[RoadVec3; 4]) -> bool + 'a,
    ) -> impl Iterator<Item = SpanQuad> + 'a {
        self.surface_polygon_order
            .iter()
            .flat_map(|order| order.iter())
            .filter_map(move |&index| {
                let region = &self.span_owned_regions[index as usize];
                let corners = region.corners(&self.sections);
                keep(&corners).then(|| {
                    SpanQuad::from_corners(corners)
                        .expect("span regions are only resolved for valid strip quads")
                })
            })
    }

    /// Asphalt, then curb, then sidewalk top quads.
    pub fn surface_polygons(&self) -> impl Iterator<Item = SpanQuad> + '_ {
        self.road_surface_polygons()
            .chain(self.curb_surface_polygons())
            .chain(self.sidewalk_surface_polygons())
    }
}

impl RoadSurfaceSystem {
    // Render order of each role's regions, as the retained band lists were sorted: asphalt,
    // curb, then sidewalk, each stably by `visual_polygon_ordering`.
    pub(super) fn span_surface_polygon_order(
        sections: &[RoadSurfaceSection],
        regions: &[RoadSurfaceSpanOwnedRegion],
    ) -> [Box<[u32]>; 3] {
        let quads: Vec<_> = regions.iter().map(|region| region.quad(sections)).collect();
        [
            RoadSurfaceSpanRegionRole::Asphalt,
            RoadSurfaceSpanRegionRole::CurbOrShoulder,
            RoadSurfaceSpanRegionRole::NonRoad,
        ]
        .map(|role| {
            let mut order: Vec<u32> = (0..regions.len() as u32)
                .filter(|&index| regions[index as usize].role() == role)
                .collect();
            order.sort_by(|&a, &b| {
                Self::visual_points_ordering(
                    quads[a as usize].points(),
                    quads[b as usize].points(),
                )
            });
            order.into_boxed_slice()
        })
    }
}
