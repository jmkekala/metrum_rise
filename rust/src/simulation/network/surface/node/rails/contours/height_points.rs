//! Height-carrier alignment for generated rail contours.

use super::*;

pub(in crate::simulation::network::surface::node::rails) fn align_height_points_to_contour(
    contour_points_xz: &[RoadVec2],
    source_points_world: &[RoadVec3],
) -> Option<Vec<RoadVec3>> {
    let mut height_by_key = BTreeMap::<NodeRailPointKey, f64>::new();
    for point in source_points_world {
        let key = road_point_key(xz(*point));
        if let Some(existing_height_m) = height_by_key.get(&key)
            && (*existing_height_m - point.y).abs() > f64::EPSILON
        {
            return None;
        }
        height_by_key.insert(key, point.y);
    }
    contour_points_xz
        .iter()
        .copied()
        .map(|point_xz| {
            height_by_key
                .get(&road_point_key(point_xz))
                .copied()
                .map(|height_m| RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect()
}

pub(in crate::simulation::network::surface::node::rails) fn align_height_points_to_source_contours(
    contour_points_xz: &[RoadVec2],
    source_contours_world: &[&[RoadVec3]],
) -> Option<Vec<RoadVec3>> {
    contour_points_xz
        .iter()
        .copied()
        .map(|point_xz| {
            height_on_source_contours(point_xz, source_contours_world)
                .map(|height_m| RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect()
}

fn height_on_source_contours(
    point_xz: RoadVec2,
    source_contours_world: &[&[RoadVec3]],
) -> Option<f64> {
    let key = road_point_key(point_xz);
    let mut height_m: Option<f64> = None;
    for source_contour_world in source_contours_world {
        if let Some(candidate_height_m) = height_on_source_contour_edge(key, source_contour_world) {
            if let Some(existing_height_m) = height_m
                && (existing_height_m - candidate_height_m).abs() > f64::EPSILON
            {
                return None;
            }
            height_m = Some(candidate_height_m);
        }
    }
    height_m
}

fn height_on_source_contour_edge(
    key: NodeRailPointKey,
    source_points_world: &[RoadVec3],
) -> Option<f64> {
    if source_points_world.is_empty() {
        return None;
    }
    for point in source_points_world {
        if road_point_key(xz(*point)) == key {
            return Some(point.y);
        }
    }
    for index in 0..source_points_world.len() {
        let next = (index + 1) % source_points_world.len();
        let start = road_point_key(xz(source_points_world[index]));
        let end = road_point_key(xz(source_points_world[next]));
        if start == end || !generated_point_key_lies_on_segment(key, start, end) {
            continue;
        }
        if let Some(height_m) = height_for_key_on_generated_edge(
            key,
            start,
            end,
            source_points_world[index].y,
            source_points_world[next].y,
        ) {
            return Some(height_m);
        }
    }
    None
}

pub(in crate::simulation::network::surface::node::rails) fn height_for_key_on_generated_edge(
    point: NodeRailPointKey,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    start_height_m: f64,
    end_height_m: f64,
) -> Option<f64> {
    if start == end || !generated_point_key_lies_on_segment(point, start, end) {
        return None;
    }
    let dx = end.0 - start.0;
    let dz = end.1 - start.1;
    let denominator = if dx.abs() >= dz.abs() { dx } else { dz };
    if denominator == 0 {
        return None;
    }
    let numerator = if dx.abs() >= dz.abs() {
        point.0 - start.0
    } else {
        point.1 - start.1
    };
    let t = numerator as f64 / denominator as f64;
    Some(start_height_m + (end_height_m - start_height_m) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_height_points_rejects_planar_source_contour_interior_vertex() {
        let source = vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(2.0, 2.0, 0.0),
            RoadVec3::new(2.0, 4.0, 2.0),
            RoadVec3::new(0.0, 2.0, 2.0),
        ];
        assert!(
            align_height_points_to_source_contours(
                &[RoadVec2::new(1.0, 1.0)],
                &[source.as_slice()],
            )
            .is_none(),
            "generated contour height ownership must come from explicit vertices or source edges"
        );
    }

    #[test]
    fn align_height_points_rejects_conflicting_source_contour_planes() {
        let lower = vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 2.0),
            RoadVec3::new(0.0, 0.0, 2.0),
        ];
        let raised = vec![
            RoadVec3::new(0.0, 1.0, 0.0),
            RoadVec3::new(2.0, 1.0, 0.0),
            RoadVec3::new(2.0, 1.0, 2.0),
            RoadVec3::new(0.0, 1.0, 2.0),
        ];

        assert!(
            align_height_points_to_source_contours(
                &[RoadVec2::new(1.0, 1.0)],
                &[lower.as_slice(), raised.as_slice()],
            )
            .is_none()
        );
    }
}
