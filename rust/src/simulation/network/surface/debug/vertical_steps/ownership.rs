//! Raised-step ownership and visibility debug helpers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::debug) fn debug_top_matches_form_raised_step_owner_pair(
        lower_matches: &[DebugTopBoundaryEdge],
        upper_matches: &[DebugTopBoundaryEdge],
    ) -> bool {
        lower_matches.iter().any(|lower| {
            upper_matches
                .iter()
                .any(|upper| Self::debug_owner_pair_forms_raised_step(lower.owner, upper.owner))
        })
    }

    pub(in crate::simulation::network::surface::debug) fn debug_boundary_owner_matches_band(
        owner: DebugBoundaryOwner,
        band_owner: NodeBandOwner,
    ) -> bool {
        owner.kind == band_owner.kind() && owner.owner_index == band_owner.owner_index()
    }

    pub(in crate::simulation::network::surface::debug) fn debug_owned_top_boundary_edges(
        piece: &RoadSurfaceVisualNodePiece,
    ) -> Vec<DebugTopBoundaryEdge> {
        let mut boundary_edges = Vec::new();
        for (region_index, region) in piece.owned_regions.iter().enumerate() {
            let owner = DebugBoundaryOwner {
                region_index,
                kind: region.kind,
                owner_index: region.owner_index,
            };
            let mut edge_counts: BTreeMap<
                DebugRenderEdgeKey,
                (usize, backend::RoadVec3, backend::RoadVec3),
            > = BTreeMap::new();
            if region.polygon.triangles_world.is_empty() {
                let points = &region.polygon.points_world;
                if points.len() >= 2 {
                    for index in 0..points.len() {
                        Self::record_debug_top_boundary_edge_count(
                            &mut edge_counts,
                            points[index],
                            points[(index + 1) % points.len()],
                        );
                    }
                }
            } else {
                for triangle in &region.polygon.triangles_world {
                    for edge_index in 0..3 {
                        Self::record_debug_top_boundary_edge_count(
                            &mut edge_counts,
                            triangle[edge_index],
                            triangle[(edge_index + 1) % 3],
                        );
                    }
                }
            }
            for (key, (count, start, end)) in edge_counts {
                if count != 1 {
                    continue;
                }
                boundary_edges.push(DebugTopBoundaryEdge {
                    owner,
                    start,
                    end,
                    key,
                    xz_key: key.xz(),
                    avg_y_m: ((start.y + end.y) * 0.5) as f32,
                });
            }
        }
        boundary_edges.sort_by(|a, b| {
            a.key
                .cmp(&b.key)
                .then(a.owner.region_index.cmp(&b.owner.region_index))
                .then(a.owner.kind.cmp(&b.owner.kind))
                .then(a.owner.owner_index.cmp(&b.owner.owner_index))
        });
        boundary_edges
    }

    pub(in crate::simulation::network::surface::debug) fn record_debug_top_boundary_edge_count(
        edge_counts: &mut BTreeMap<
            DebugRenderEdgeKey,
            (usize, backend::RoadVec3, backend::RoadVec3),
        >,
        start: backend::RoadVec3,
        end: backend::RoadVec3,
    ) {
        let Some(key) = DebugRenderEdgeKey::normalized(start, end) else {
            return;
        };
        edge_counts
            .entry(key)
            .and_modify(|entry| entry.0 += 1)
            .or_insert((1, start, end));
    }

    pub(in crate::simulation::network::surface::debug) fn debug_vertical_face_span_edges(
        polygon: &RoadSurfaceVisualPolygon,
    ) -> Option<DebugVerticalFaceSpanEdges> {
        if polygon.points_world.len() < 4 {
            return None;
        }
        let mut span_edges = Vec::new();
        for index in 0..polygon.points_world.len() {
            let start = polygon.points_world[index];
            let end = polygon.points_world[(index + 1) % polygon.points_world.len()];
            let start_key = DebugRenderVertexKey::from_point(start).xz();
            let end_key = DebugRenderVertexKey::from_point(end).xz();
            if start_key != end_key {
                span_edges.push((start, end, (start.y + end.y) * 0.5));
            }
        }
        if span_edges.len() != 2 {
            return None;
        }
        span_edges.sort_by(|a, b| a.2.total_cmp(&b.2));
        Some(DebugVerticalFaceSpanEdges {
            lower_start: span_edges[0].0,
            lower_end: span_edges[0].1,
            upper_start: span_edges[1].0,
            upper_end: span_edges[1].1,
        })
    }

    pub(in crate::simulation::network::surface::debug) fn debug_polygon_winding_normal(
        points: &[backend::RoadVec3],
    ) -> Option<backend::RoadVec3> {
        if points.len() < 3 {
            return None;
        }
        for index in 1..points.len().saturating_sub(1) {
            let normal = (points[index] - points[0]).cross(points[index + 1] - points[0]);
            if normal.length_squared() > 1e-8 {
                return Some(normal.normalize());
            }
        }
        None
    }

    pub(in crate::simulation::network::surface::debug) fn debug_visible_dot_to_lower_raised_step_owner(
        piece: &RoadSurfaceVisualNodePiece,
        face_midpoint: backend::RoadVec3,
        visible_direction: backend::RoadVec3,
        lower_matches: &[DebugTopBoundaryEdge],
        upper_matches: &[DebugTopBoundaryEdge],
    ) -> Option<f32> {
        let visible_xz = backend::RoadVec3::new(visible_direction.x, 0.0, visible_direction.z);
        if visible_xz.length_squared() <= 1e-8 {
            return None;
        }
        let visible_xz = visible_xz.normalize();
        let mut best: Option<f32> = None;
        for edge in lower_matches.iter().filter(|lower| {
            upper_matches
                .iter()
                .any(|upper| Self::debug_owner_pair_forms_raised_step(lower.owner, upper.owner))
        }) {
            let Some(centroid) = Self::debug_owned_region_centroid(piece, edge.owner.region_index)
            else {
                continue;
            };
            let owner_direction = backend::RoadVec3::new(
                centroid.x - face_midpoint.x,
                0.0,
                centroid.z - face_midpoint.z,
            );
            if owner_direction.length_squared() <= 1e-8 {
                continue;
            }
            let dot = visible_xz.dot(owner_direction.normalize()) as f32;
            best = Some(best.map_or(dot, |current| current.max(dot)));
        }
        best
    }

    pub(in crate::simulation::network::surface::debug) fn debug_visible_dot_to_lower_owner(
        piece: &RoadSurfaceVisualNodePiece,
        face_midpoint: backend::RoadVec3,
        visible_direction: backend::RoadVec3,
        lower_matches: &[DebugTopBoundaryEdge],
        lower_owner: NodeBandOwner,
    ) -> Option<f32> {
        let visible_xz = backend::RoadVec3::new(visible_direction.x, 0.0, visible_direction.z);
        if visible_xz.length_squared() <= 1e-8 {
            return None;
        }
        let visible_xz = visible_xz.normalize();
        let mut best: Option<f32> = None;
        for edge in lower_matches
            .iter()
            .filter(|edge| Self::debug_boundary_owner_matches_band(edge.owner, lower_owner))
        {
            let Some(centroid) = Self::debug_owned_region_centroid(piece, edge.owner.region_index)
            else {
                continue;
            };
            let owner_direction = backend::RoadVec3::new(
                centroid.x - face_midpoint.x,
                0.0,
                centroid.z - face_midpoint.z,
            );
            if owner_direction.length_squared() <= 1e-8 {
                continue;
            }
            let dot = visible_xz.dot(owner_direction.normalize()) as f32;
            best = Some(best.map_or(dot, |current| current.max(dot)));
        }
        best
    }

    pub(in crate::simulation::network::surface::debug) fn debug_owned_region_centroid(
        piece: &RoadSurfaceVisualNodePiece,
        region_index: usize,
    ) -> Option<backend::RoadVec3> {
        let region = piece.owned_regions.get(region_index)?;
        let mut sum = backend::RoadVec3::ZERO;
        let mut count = 0usize;
        if region.polygon.points_world.is_empty() {
            for point in region
                .polygon
                .triangles_world
                .iter()
                .flat_map(|triangle| triangle.iter().copied())
            {
                sum += point;
                count += 1;
            }
        } else {
            for point in &region.polygon.points_world {
                sum += *point;
                count += 1;
            }
        }
        (count > 0).then_some(sum * (1.0 / count as f64))
    }
}
