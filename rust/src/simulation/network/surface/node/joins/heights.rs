// SPDX-License-Identifier: GPL-2.0-only

//! Side-join endpoint height-plane fitting.

use super::*;

impl SideJoinHeightPlane {
    pub(super) fn height_at_xz(self, point_xz: RoadVec2) -> f64 {
        self.origin.y
            + self.grade_x * (point_xz.x - self.origin.x)
            + self.grade_z * (point_xz.y - self.origin.z)
    }
}

pub(super) fn endpoint_height_plane_for_band_kind(
    mouths: &[NodeInputMouth],
    band_kind: RoadSurfaceBandKind,
) -> Result<Option<SideJoinHeightPlane>, SideJoinGenerationError> {
    let mut points = mouths
        .iter()
        .flat_map(|mouth| mouth.endpoint_rails.iter())
        .filter(|rail| rail.band_kind == band_kind)
        .flat_map(|rail| [rail.start_world, rail.end_world])
        .collect::<Vec<_>>();
    canonicalize_height_plane_points(&mut points);
    let Some(plane) = height_plane_from_points(&points) else {
        return Ok(None);
    };
    Ok(validate_height_plane(&points, plane).then_some(plane))
}

fn canonicalize_height_plane_points(points: &mut Vec<RoadVec3>) {
    points.sort_by_key(|point| {
        let key = SurfaceXzKey::from_road_xz(xz_from_road_vec3(*point));
        (
            key.x_key(),
            key.z_key(),
            SurfaceHeightMmKey::from_m_f64(point.y).as_i64(),
        )
    });
    points.dedup_by_key(|point| {
        let key = SurfaceXzKey::from_road_xz(xz_from_road_vec3(*point));
        (
            key.x_key(),
            key.z_key(),
            SurfaceHeightMmKey::from_m_f64(point.y).as_i64(),
        )
    });
}

fn height_plane_from_points(points: &[RoadVec3]) -> Option<SideJoinHeightPlane> {
    let mut selected: Option<(u128, SideJoinHeightPlane)> = None;
    for a_index in 0..points.len() {
        for b_index in a_index + 1..points.len() {
            for c_index in b_index + 1..points.len() {
                let area =
                    height_plane_triangle_area2(points[a_index], points[b_index], points[c_index]);
                if area == 0 {
                    continue;
                }
                let plane =
                    height_plane_from_triangle(points[a_index], points[b_index], points[c_index])?;
                if selected.is_none_or(|(selected_area, _)| area > selected_area) {
                    selected = Some((area, plane));
                }
            }
        }
    }
    selected.map(|(_, plane)| plane)
}

fn height_plane_triangle_area2(a: RoadVec3, b: RoadVec3, c: RoadVec3) -> u128 {
    let a = SurfaceXzKey::from_road_xz(xz_from_road_vec3(a)).raw_tuple();
    let b = SurfaceXzKey::from_road_xz(xz_from_road_vec3(b)).raw_tuple();
    let c = SurfaceXzKey::from_road_xz(xz_from_road_vec3(c)).raw_tuple();
    SurfaceXzKey::raw_tuple_triangle_area2(a, b, c).unsigned_abs()
}

fn height_plane_from_triangle(
    origin: RoadVec3,
    b: RoadVec3,
    c: RoadVec3,
) -> Option<SideJoinHeightPlane> {
    let ux = b.x - origin.x;
    let uz = b.z - origin.z;
    let uy = b.y - origin.y;
    let vx = c.x - origin.x;
    let vz = c.z - origin.z;
    let vy = c.y - origin.y;
    let denominator = ux * vz - uz * vx;
    if denominator.abs() <= f64::EPSILON {
        return None;
    }
    Some(SideJoinHeightPlane {
        origin,
        grade_x: (uy * vz - uz * vy) / denominator,
        grade_z: (ux * vy - uy * vx) / denominator,
    })
}

fn validate_height_plane(points: &[RoadVec3], plane: SideJoinHeightPlane) -> bool {
    for point in points {
        let expected_height_m = plane.height_at_xz(xz_from_road_vec3(*point));
        let expected_height_key = SurfaceHeightMmKey::from_m_f64(expected_height_m);
        let incoming_height_key = SurfaceHeightMmKey::from_m_f64(point.y);
        if (expected_height_key.as_i64() - incoming_height_key.as_i64()).abs()
            <= SIDE_JOIN_ENDPOINT_PLANE_HEIGHT_DUST_MM
        {
            continue;
        }
        return false;
    }
    true
}
