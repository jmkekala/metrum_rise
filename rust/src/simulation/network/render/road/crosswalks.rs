//! Intersection crosswalk markings and pedestrian path rendering.

use super::*;

pub(super) fn emit_crosswalk_markings(
    mesh: &mut NetworkMeshData,
    graph: &crate::simulation::network::graph::RegionGraph,
    lane_system: &crate::simulation::network::lanes::LaneSystem,
    terrain: &crate::simulation::terrain::TerrainSystem,
    road_surface: &crate::simulation::network::surface::RoadSurfaceSystem,
    coverage: &super::standard_surface::CompiledSurfaceCoverage,
) {
    use crate::simulation::network::lanes::LaneType;
    for &node_id in &coverage.node_ids {
        let Some(lane_ids) = lane_system.node_lanes.get(&(node_id as usize)) else {
            continue;
        };
        for &lane_id in lane_ids {
            let Some(lane) = lane_system.lanes.get(lane_id) else {
                continue;
            };
            if lane.edge_id == usize::MAX
                && lane.lane_type == LaneType::Foot
                && let Some(marking) = lane.crosswalk_marking
            {
                let surface_query = road_surface.lane_owner_surface_query(
                    graph,
                    terrain,
                    marking.edge_id,
                    node_id as usize,
                    true,
                );
                emit_zebra_stripes(mesh, graph, marking, terrain, road_surface, &surface_query);
            }
        }
    }
}

pub(super) const CROSSWALK_STRIPE_WIDTH: f32 = 0.5;
pub(super) const CROSSWALK_STRIPE_LEN: f32 = 2.0;
const CROSSWALK_STRIPE_GAP: f32 = 0.4;

fn emit_zebra_stripes(
    mesh: &mut NetworkMeshData,
    graph: &crate::simulation::network::graph::RegionGraph,
    marking: crate::simulation::network::lanes::CrosswalkMarking,
    terrain: &crate::simulation::terrain::TerrainSystem,
    road_surface: &crate::simulation::network::surface::RoadSurfaceSystem,
    surface_query: &RoadLaneSurfaceQuery<'_>,
) {
    let geometry = [marking.start, marking.end];
    let length = marking.start.distance_to(marking.end);
    if length <= f32::EPSILON {
        return;
    }
    let color = Color::from_rgb(1.0, 1.0, 1.0);
    let step = CROSSWALK_STRIPE_WIDTH + CROSSWALK_STRIPE_GAP;
    let mut travelled = 0.0;
    while travelled + CROSSWALK_STRIPE_WIDTH <= length {
        let t_param = (travelled + CROSSWALK_STRIPE_WIDTH * 0.5) / length;
        let (p, tangent) = sample_polyline_pos_tangent(&geometry, t_param);
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
                crosswalk_marking_vertex(v0, graph, terrain, road_surface, surface_query),
                crosswalk_marking_vertex(v1, graph, terrain, road_surface, surface_query),
                crosswalk_marking_vertex(v2, graph, terrain, road_surface, surface_query),
                crosswalk_marking_vertex(v3, graph, terrain, road_surface, surface_query),
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

fn crosswalk_marking_vertex(
    vertex: Vector3,
    graph: &crate::simulation::network::graph::RegionGraph,
    terrain: &crate::simulation::terrain::TerrainSystem,
    road_surface: &crate::simulation::network::surface::RoadSurfaceSystem,
    surface_query: &RoadLaneSurfaceQuery<'_>,
) -> Vector3 {
    let height_m = surface_query
        .sample_height(vertex.x, vertex.z)
        .or_else(|| {
            road_surface.sample_visible_carriageway_height(graph, terrain, vertex.x, vertex.z)
        })
        .or_else(|| road_surface.sample_visible_surface_height(graph, terrain, vertex.x, vertex.z))
        .unwrap_or(vertex.y);
    Vector3::new(vertex.x, height_m + MARKING_RENDER_Z_BIAS_M, vertex.z)
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
