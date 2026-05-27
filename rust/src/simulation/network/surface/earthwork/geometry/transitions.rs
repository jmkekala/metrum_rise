//! Earthwork transition point construction.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::earthwork) fn earthwork_edge_transition_points(
        &self,
        current: RoadVec3,
        next: RoadVec3,
        winding_ccw: bool,
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> Result<(RoadVec3, RoadVec3), RoadSurfaceEarthworkGeometryError> {
        let edge = RoadVec2::new(next.x - current.x, next.z - current.z);
        let Some(outward) = Self::edge_outward_normal_xz(edge, winding_ccw) else {
            return Err(
                RoadSurfaceEarthworkGeometryError::DegenerateOutwardDirection {
                    point_count: 2,
                    point_index: 0,
                },
            );
        };
        let Some(outer_current) = self.earthwork_transition_point(current, outward, terrain) else {
            return Err(
                RoadSurfaceEarthworkGeometryError::DegenerateOutwardDirection {
                    point_count: 2,
                    point_index: 0,
                },
            );
        };
        let Some(outer_next) = self.earthwork_transition_point(next, outward, terrain) else {
            return Err(
                RoadSurfaceEarthworkGeometryError::DegenerateOutwardDirection {
                    point_count: 2,
                    point_index: 1,
                },
            );
        };
        let Some(top_surface_shapes) = top_surface_shapes else {
            return Ok((outer_current, outer_next));
        };

        let Some(opposite_outer_current) =
            self.earthwork_transition_point(current, -outward, terrain)
        else {
            return Err(
                RoadSurfaceEarthworkGeometryError::DegenerateOutwardDirection {
                    point_count: 2,
                    point_index: 0,
                },
            );
        };
        let Some(opposite_outer_next) = self.earthwork_transition_point(next, -outward, terrain)
        else {
            return Err(
                RoadSurfaceEarthworkGeometryError::DegenerateOutwardDirection {
                    point_count: 2,
                    point_index: 1,
                },
            );
        };
        let Some(nominal_overlap) = Self::earthwork_candidate_top_overlap_area_m2(
            [current, next, outer_next, outer_current],
            top_surface_shapes,
        ) else {
            return Ok((outer_current, outer_next));
        };
        let Some(opposite_overlap) = Self::earthwork_candidate_top_overlap_area_m2(
            [current, next, opposite_outer_next, opposite_outer_current],
            top_surface_shapes,
        ) else {
            return Ok((outer_current, outer_next));
        };
        if opposite_overlap < nominal_overlap {
            Ok((opposite_outer_current, opposite_outer_next))
        } else {
            Ok((outer_current, outer_next))
        }
    }

    pub(in crate::simulation::network::surface) fn earthwork_transition_point(
        &self,
        road_point: RoadVec3,
        outward_xz: RoadVec2,
        terrain: &TerrainSystem,
    ) -> Option<RoadVec3> {
        if outward_xz.length_squared() <= f64::from(SAMPLE_EPSILON_M * SAMPLE_EPSILON_M) {
            return None;
        }
        let outward_xz = outward_xz.normalize();
        let distance_m = self.earthwork_transition_distance_m(road_point, outward_xz, terrain);
        let outer_xz = RoadVec2::new(road_point.x, road_point.z) + outward_xz * distance_m;
        let outer_height_m = terrain.sample_height_world(outer_xz.x as f32, outer_xz.y as f32)
            * config::HEIGHT_SCALE;
        Some(RoadVec3::new(
            outer_xz.x,
            f64::from(outer_height_m),
            outer_xz.y,
        ))
    }

    pub(in crate::simulation::network::surface::earthwork) fn earthwork_transition_distance_m(
        &self,
        road_point: RoadVec3,
        outward_xz: RoadVec2,
        terrain: &TerrainSystem,
    ) -> f64 {
        let source_height_at_edge = terrain
            .sample_height_world(road_point.x as f32, road_point.z as f32)
            * config::HEIGHT_SCALE;
        let cut_side = f64::from(source_height_at_edge) > road_point.y;
        let slope_rate = if cut_side {
            EARTHWORK_CUT_SLOPE_RATE
        } else {
            EARTHWORK_FILL_SLOPE_RATE
        };

        let mut distance_m = f64::from(EARTHWORK_MIN_MARGIN_M);
        while distance_m < f64::from(EARTHWORK_MAX_MARGIN_M) {
            let sample_x = road_point.x + outward_xz.x * distance_m;
            let sample_z = road_point.z + outward_xz.y * distance_m;
            let source_height = terrain.sample_height_world(sample_x as f32, sample_z as f32)
                * config::HEIGHT_SCALE;
            let transition_height = if cut_side {
                road_point.y + f64::from(slope_rate) * distance_m
            } else {
                road_point.y - f64::from(slope_rate) * distance_m
            };
            let rejoins_source = if cut_side {
                transition_height >= f64::from(source_height)
            } else {
                transition_height <= f64::from(source_height)
            };
            if rejoins_source {
                return distance_m;
            }
            distance_m += f64::from(EARTHWORK_MARGIN_SAMPLE_STEP_M);
        }

        f64::from(EARTHWORK_MAX_MARGIN_M)
    }
}
