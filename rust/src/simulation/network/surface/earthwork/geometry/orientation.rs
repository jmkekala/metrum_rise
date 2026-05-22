//! Earthwork boundary orientation helpers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::earthwork) fn closed_loop_vertex_outward_xz(
        boundary_points: &[Vector3],
        index: usize,
    ) -> Option<Vector2> {
        if boundary_points.len() < 3 {
            return None;
        }

        let len = boundary_points.len();
        let prev = boundary_points[(index + len - 1) % len];
        let current = boundary_points[index];
        let next = boundary_points[(index + 1) % len];
        let incoming = Vector2::new(current.x - prev.x, current.z - prev.z);
        let outgoing = Vector2::new(next.x - current.x, next.z - current.z);
        let winding_ccw = Self::signed_polygon_area_xz(boundary_points) > 0.0;
        let outward_incoming = Self::edge_outward_normal_xz(incoming, winding_ccw)?;
        let outward_outgoing = Self::edge_outward_normal_xz(outgoing, winding_ccw)?;
        let outward = outward_incoming + outward_outgoing;
        if outward.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            None
        } else {
            Some(outward.normalized())
        }
    }

    pub(in crate::simulation::network::surface::earthwork) fn edge_outward_normal_xz(
        edge_xz: Vector2,
        winding_ccw: bool,
    ) -> Option<Vector2> {
        if edge_xz.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return None;
        }
        let tangent = edge_xz.normalized();
        if winding_ccw {
            Some(Vector2::new(tangent.y, -tangent.x))
        } else {
            Some(Vector2::new(-tangent.y, tangent.x))
        }
    }
}
