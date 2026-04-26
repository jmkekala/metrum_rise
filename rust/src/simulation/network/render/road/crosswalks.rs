//! Intersection crosswalk markings and pedestrian path rendering.

use super::*;

pub(super) fn emit_crosswalk_markings(
    mesh: &mut NetworkMeshData,
    lane_system: &crate::simulation::network::lanes::LaneSystem,
) {
    use crate::simulation::network::lanes::LaneType;
    for lane in &lane_system.lanes {
        if lane.edge_id == usize::MAX && lane.lane_type == LaneType::Foot && lane.is_crosswalk {
            if lane.geometry.len() >= 2 {
                emit_zebra_stripes(mesh, lane);
            }
        }
    }
}

const CROSSWALK_STRIPE_WIDTH: f32 = 0.5;
const CROSSWALK_STRIPE_LEN: f32 = 2.0;
const CROSSWALK_STRIPE_GAP: f32 = 0.4;

fn emit_zebra_stripes(mesh: &mut NetworkMeshData, lane: &crate::simulation::network::lanes::Lane) {
    let color = Color::from_rgb(1.0, 1.0, 1.0);
    let step = CROSSWALK_STRIPE_WIDTH + CROSSWALK_STRIPE_GAP;
    let mut travelled = 0.0;
    while travelled + CROSSWALK_STRIPE_WIDTH <= lane.length {
        let t_param = (travelled + CROSSWALK_STRIPE_WIDTH * 0.5) / lane.length;
        let (p, tangent) = sample_polyline_pos_tangent(&lane.geometry, t_param);
        let normal = Vector3::new(-tangent.z, 0.0, tangent.x).normalized();
        let hw = CROSSWALK_STRIPE_WIDTH * 0.5;
        let hl = CROSSWALK_STRIPE_LEN * 0.5;
        let v0 = p - tangent * hw - normal * hl;
        let v1 = p + tangent * hw - normal * hl;
        let v2 = p + tangent * hw + normal * hl;
        let v3 = p - tangent * hw + normal * hl;
        push_quad(
            mesh,
            MeshLayer::Marking,
            [
                v0 + Vector3::new(0.0, MARKING_RENDER_Z_BIAS_M, 0.0),
                v1 + Vector3::new(0.0, MARKING_RENDER_Z_BIAS_M, 0.0),
                v2 + Vector3::new(0.0, MARKING_RENDER_Z_BIAS_M, 0.0),
                v3 + Vector3::new(0.0, MARKING_RENDER_Z_BIAS_M, 0.0),
            ],
            [
                Vector2::new(0.0, 0.0),
                Vector2::new(1.0, 0.0),
                Vector2::new(1.0, 1.0),
                Vector2::new(0.0, 1.0),
            ],
            color,
        );
        travelled += step;
    }
}

fn sample_polyline_pos_tangent(points: &[Vector3], t: f32) -> (Vector3, Vector3) {
    if points.is_empty() {
        return (Vector3::ZERO, Vector3::ZERO);
    }
    if points.len() == 1 {
        return (points[0], Vector3::FORWARD);
    }
    let t = t.clamp(0.0, 1.0);
    let mut total_len = 0.0;
    for i in 0..points.len() - 1 {
        total_len += points[i].distance_to(points[i + 1]);
    }
    let target_len = t * total_len;
    let mut current = 0.0;
    for i in 0..points.len() - 1 {
        let seg_len = points[i].distance_to(points[i + 1]);
        if current + seg_len >= target_len || i == points.len() - 2 {
            let local_t = (target_len - current) / seg_len;
            let pos = points[i].lerp(points[i + 1], local_t.clamp(0.0, 1.0));
            let tangent = (points[i + 1] - points[i]).normalized();
            return (pos, tangent);
        }
        current += seg_len;
    }
    (
        points[points.len() - 1],
        (points[points.len() - 1] - points[points.len() - 2]).normalized(),
    )
}
