//! Shared world-path canonical height interpolation helpers.

use super::{
    backend::{RoadPolyline, RoadVec2, RoadVec3, road_points_to_polyline, road_vec3_xz},
    keys::SurfaceXzKey,
};
use cavalier_contours::polyline::{PlineCreation, PlineSource};

/// Recovers a deterministic world height for a quantized XZ point on a source path.
pub(crate) fn height_on_world_path(point_xz: RoadVec2, path_world: &[RoadVec3]) -> Option<f64> {
    let key = SurfaceXzKey::from_road_xz(point_xz);
    for point_world in path_world {
        if SurfaceXzKey::from_road_xz(road_vec3_xz(*point_world)) == key {
            return Some(point_world.y);
        }
    }
    for segment in path_world.windows(2) {
        if let Some(height_m) = height_on_world_segment(point_xz, segment[0], segment[1]) {
            return Some(height_m);
        }
    }
    None
}

/// Interpolates height for an XZ key with canonical source-segment support.
pub(crate) fn height_on_world_segment(
    point_xz: RoadVec2,
    start_world: RoadVec3,
    end_world: RoadVec3,
) -> Option<f64> {
    let point = SurfaceXzKey::from_road_xz(point_xz);
    let start = SurfaceXzKey::from_road_xz(road_vec3_xz(start_world));
    let end = SurfaceXzKey::from_road_xz(road_vec3_xz(end_world));
    let parameter = point.overlay_segment_parameter(start, end)?;
    let t = parameter.as_f64();
    Some(start_world.y + (end_world.y - start_world.y) * t)
}

/// Rebuilds world points from cleaned XZ points with canonical source-path height support.
pub(crate) fn reheight_road_points_from_world_path(
    points_xz: impl IntoIterator<Item = RoadVec2>,
    source_path_world: &[RoadVec3],
) -> Option<Vec<RoadVec3>> {
    let mut points = points_xz
        .into_iter()
        .map(|point_xz| {
            let height_m = height_on_world_path(point_xz, source_path_world)?;
            Some(RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect::<Option<Vec<_>>>()?;
    remove_repeated_road_vec3_xz_points(&mut points);
    Some(points)
}

/// Cleans an open XZ path through the shared road-polyline representation.
pub(crate) fn cleaned_open_road_polyline(
    points_xz: impl IntoIterator<Item = RoadVec2>,
    point_equal_eps_m: f64,
    remove_redundant: bool,
) -> Option<RoadPolyline> {
    let raw = road_points_to_polyline(points_xz, false);
    let mut cleaned = RoadPolyline::create_from_remove_repeat(&raw, point_equal_eps_m);
    if remove_redundant {
        if let Some(reduced) = cleaned.remove_redundant(point_equal_eps_m) {
            cleaned = reduced;
        }
    }
    (cleaned.vertex_count() >= 2).then_some(cleaned)
}

/// Cleans an open world path by applying XZ-only road-polyline cleanup.
pub(crate) fn cleaned_open_world_path_polyline(
    path_world: &[RoadVec3],
    point_equal_eps_m: f64,
    remove_redundant: bool,
) -> Option<RoadPolyline> {
    cleaned_open_road_polyline(
        path_world.iter().copied().map(road_vec3_xz),
        point_equal_eps_m,
        remove_redundant,
    )
}

/// Reports whether a closed world contour has stable positive area and no self-intersection.
pub(crate) fn closed_world_contour_has_area(
    contour_world: &[RoadVec3],
    point_equal_eps_m: f64,
    min_area_m2: f64,
) -> bool {
    let raw = road_points_to_polyline(contour_world.iter().copied().map(road_vec3_xz), true);
    let contour = RoadPolyline::create_from_remove_repeat(&raw, point_equal_eps_m);
    contour.vertex_count() >= 3
        && contour.area().abs() > min_area_m2
        && !contour.scan_for_self_intersect()
}

/// Removes consecutive and closing duplicate XZ keys while preserving the first height.
pub(crate) fn remove_repeated_road_vec3_xz_points(points: &mut Vec<RoadVec3>) {
    points.dedup_by(|a, b| {
        SurfaceXzKey::from_road_xz(road_vec3_xz(*a)) == SurfaceXzKey::from_road_xz(road_vec3_xz(*b))
    });
    if points.len() > 1
        && SurfaceXzKey::from_road_xz(road_vec3_xz(points[0]))
            == SurfaceXzKey::from_road_xz(road_vec3_xz(
                *points.last().expect("points are non-empty"),
            ))
    {
        points.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_quantized_endpoint_height_wins_before_segment_projection() {
        let path = [
            RoadVec3::new(0.0, 3.0, 0.0),
            RoadVec3::new(1.0, 5.0, 0.0),
            RoadVec3::new(2.0, 9.0, 0.0),
        ];

        let height = height_on_world_path(RoadVec2::new(1.0, 0.0), &path)
            .expect("endpoint key should be heighted directly");

        assert_eq!(height, 5.0);
    }

    #[test]
    fn midpoint_height_interpolates_from_source_segment() {
        let path = [RoadVec3::new(0.0, 2.0, 0.0), RoadVec3::new(2.0, 6.0, 0.0)];

        let height = height_on_world_path(RoadVec2::new(1.0, 0.0), &path)
            .expect("midpoint should project onto source segment");

        assert_eq!(height, 4.0);
    }

    #[test]
    fn off_segment_point_outside_epsilon_has_no_height() {
        let path = [RoadVec3::new(0.0, 2.0, 0.0), RoadVec3::new(2.0, 6.0, 0.0)];

        assert!(height_on_world_path(RoadVec2::new(1.0, 0.01), &path).is_none());
    }

    #[test]
    fn off_segment_point_inside_old_epsilon_has_no_height() {
        let path = [RoadVec3::new(0.0, 2.0, 0.0), RoadVec3::new(2.0, 6.0, 0.0)];

        assert!(height_on_world_path(RoadVec2::new(1.0, 0.0005), &path).is_none());
    }

    #[test]
    fn reheighted_points_drop_repeated_xz_keys() {
        let path = [
            RoadVec3::new(0.0, 1.0, 0.0),
            RoadVec3::new(1.0, 2.0, 0.0),
            RoadVec3::new(0.0, 3.0, 0.0),
        ];

        let points = reheight_road_points_from_world_path(
            [
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(1.0, 0.0),
                RoadVec2::new(0.0, 0.0),
            ],
            &path,
        )
        .expect("source path should provide all requested heights");

        assert_eq!(
            points,
            vec![RoadVec3::new(0.0, 1.0, 0.0), RoadVec3::new(1.0, 2.0, 0.0)]
        );
    }

    #[test]
    fn cleaned_open_world_path_removes_repeats_and_optionally_redundant_points() {
        let path = [
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(1.0, 1.0, 0.0),
            RoadVec3::new(1.0, 2.0, 0.0),
            RoadVec3::new(2.0, 3.0, 0.0),
        ];

        let repeated_only = cleaned_open_world_path_polyline(&path, 0.001, false)
            .expect("repeated endpoint cleanup should preserve an open path");
        let reduced = cleaned_open_world_path_polyline(&path, 0.001, true)
            .expect("redundant collinear cleanup should preserve an open path");

        assert_eq!(repeated_only.vertex_count(), 3);
        assert_eq!(reduced.vertex_count(), 2);
    }

    #[test]
    fn closed_world_contour_area_rejects_degenerate_and_self_intersecting_contours() {
        let square = [
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 1.0),
            RoadVec3::new(0.0, 0.0, 1.0),
        ];
        let line = [
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
        ];
        let bowtie = [
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 1.0),
            RoadVec3::new(0.0, 0.0, 1.0),
            RoadVec3::new(1.0, 0.0, 0.0),
        ];

        assert!(closed_world_contour_has_area(&square, 0.001, 0.001));
        assert!(!closed_world_contour_has_area(&line, 0.001, 0.001));
        assert!(!closed_world_contour_has_area(&bowtie, 0.001, 0.001));
    }
}
