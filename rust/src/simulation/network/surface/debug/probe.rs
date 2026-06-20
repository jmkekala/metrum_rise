//! Point probe debug dump for final road-surface triangles.

use super::*;

const PROBE_TOP_NEAR_RADIUS_M: f64 = 0.05;
const PROBE_VERTICAL_NEAR_RADIUS_M: f64 = 0.25;
const PROBE_EPSILON_M: f64 = 0.001;

impl RoadSurfaceSystem {
    pub(crate) fn build_road_surface_probe_debug_dump(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        world_pos: Vector3,
    ) -> String {
        let point = backend::godot_vec3_to_road(world_pos);
        let point_xz = backend::road_vec3_xz(point);
        let chunk = self.chunk_coords_for_world(point.x, point.z);
        let (edge_indices, node_ids) = self.debug_probe_contributors_for_chunk(chunk);
        let source_terrain_y_m =
            terrain.sample_height_world(world_pos.x, world_pos.z) * config::HEIGHT_SCALE;
        let visual_terrain_y_m =
            terrain.sample_visual_height_world(world_pos.x, world_pos.z) * config::HEIGHT_SCALE;

        let mut dump = String::new();
        let _ = writeln!(dump, "ROAD_SURFACE_PROBE_BEGIN");
        dump.push('{');
        dump.push_str("\"probe_world\":");
        Self::append_vector3_precise_literal(&mut dump, point);
        let _ = write!(
            dump,
            ",\"probe_xz\":[{:.6}, {:.6}],\"chunk\":[{}, {}],\"source_terrain_y_m\":{:.3},\"visual_terrain_y_m\":{:.3},\"top_near_radius_m\":{:.3},\"vertical_near_radius_m\":{:.3}",
            point.x,
            point.z,
            chunk.0,
            chunk.1,
            source_terrain_y_m,
            visual_terrain_y_m,
            PROBE_TOP_NEAR_RADIUS_M,
            PROBE_VERTICAL_NEAR_RADIUS_M
        );
        dump.push_str(",\"contributor_edges\":");
        Self::append_usize_list_literal(&mut dump, &edge_indices);
        dump.push_str(",\"contributor_nodes\":[");
        for (index, node_id) in node_ids.iter().enumerate() {
            if index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(dump, "{node_id}");
        }
        dump.push_str("],\"matches\":[");

        let mut first_match = true;
        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if self.node_uses_visible_surface(graph, terrain, node_id) {
                self.append_node_probe_top_matches(
                    &mut dump,
                    &mut first_match,
                    node_id,
                    piece,
                    point_xz,
                );
            }
            self.append_node_probe_raised_step_matches(
                &mut dump,
                &mut first_match,
                node_id,
                piece,
                point_xz,
            );
            if self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
                self.append_node_probe_earthwork_matches(
                    &mut dump,
                    &mut first_match,
                    graph,
                    terrain,
                    node_id,
                    piece,
                    point_xz,
                );
            }
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            self.append_span_probe_top_matches(&mut dump, &mut first_match, piece, point_xz);
            self.append_span_probe_raised_step_matches(
                &mut dump,
                &mut first_match,
                piece,
                point_xz,
            );
            if self.span_piece_uses_visible_earthwork(piece) {
                self.append_span_probe_earthwork_matches(
                    &mut dump,
                    &mut first_match,
                    piece,
                    point_xz,
                );
            }
        }

        dump.push_str("]}");
        let _ = write!(dump, "\nROAD_SURFACE_PROBE_END");
        dump
    }

    fn debug_probe_contributors_for_chunk(&self, chunk: SurfaceChunkKey) -> (Vec<usize>, Vec<u32>) {
        let mut edge_indices = Vec::new();
        let mut node_ids = Vec::new();
        for cx in (chunk.0 - 1)..=(chunk.0 + 1) {
            for cz in (chunk.1 - 1)..=(chunk.1 + 1) {
                let key = (cx, cz);
                if let Some(entry) = self.surface_chunk_cache.get(&key) {
                    edge_indices.extend(entry.edge_indices.iter().copied());
                    node_ids.extend(entry.node_ids.iter().copied());
                }
                if let Some(entry) = self.earthwork_chunk_cache.get(&key) {
                    edge_indices.extend(entry.edge_indices.iter().copied());
                    node_ids.extend(entry.node_ids.iter().copied());
                }
            }
        }
        edge_indices.sort_unstable();
        edge_indices.dedup();
        node_ids.sort_unstable();
        node_ids.dedup();
        (edge_indices, node_ids)
    }

    fn append_node_probe_top_matches(
        &self,
        dump: &mut String,
        first_match: &mut bool,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        point_xz: backend::RoadVec2,
    ) {
        for (region_index, region) in piece.owned_regions.iter().enumerate() {
            for (triangle_index, triangle) in region.polygon.triangles_world.iter().enumerate() {
                let Some(probe) = DebugProbeTriangleHit::from_top_triangle(*triangle, point_xz)
                else {
                    continue;
                };
                if !probe.contains_xz && probe.xz_distance_m > PROBE_TOP_NEAR_RADIUS_M {
                    continue;
                }
                Self::append_probe_match_separator(dump, first_match);
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"node_top\",\"node_id\":{},\"piece_kind\":\"{:?}\",\"region\":{},\"kind\":\"{:?}\",\"material\":\"{}\",\"owner_index\":{},\"triangle_index\":{}",
                    node_id,
                    piece.kind,
                    region_index,
                    region.kind,
                    Self::debug_material_for_band_kind(region.kind),
                    region.owner_index,
                    triangle_index
                );
                Self::append_probe_hit_fields(dump, probe);
                dump.push('}');
            }
        }
    }

    fn append_node_probe_raised_step_matches(
        &self,
        dump: &mut String,
        first_match: &mut bool,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        point_xz: backend::RoadVec2,
    ) {
        for (face_index, polygon) in piece.raised_step_face_polygons.iter().enumerate() {
            let source = piece.raised_step_face_sources.get(face_index).copied();
            for (triangle_index, triangle) in polygon.triangles_world.iter().enumerate() {
                let Some(probe) = DebugProbeTriangleHit::from_near_triangle(*triangle, point_xz)
                else {
                    continue;
                };
                if probe.xz_distance_m > PROBE_VERTICAL_NEAR_RADIUS_M {
                    continue;
                }
                Self::append_probe_match_separator(dump, first_match);
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"node_raised_step_face\",\"node_id\":{},\"piece_kind\":\"{:?}\",\"face_index\":{},\"triangle_index\":{},\"source\":",
                    node_id, piece.kind, face_index, triangle_index
                );
                if let Some(source) = source {
                    let _ = write!(dump, "\"{:?}\"", source);
                } else {
                    dump.push_str("null");
                }
                Self::append_probe_hit_fields(dump, probe);
                dump.push('}');
            }
        }
    }

    fn append_node_probe_earthwork_matches(
        &self,
        dump: &mut String,
        first_match: &mut bool,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        point_xz: backend::RoadVec2,
    ) {
        for (face_index, face) in piece.render_earthwork_faces.iter().enumerate() {
            if !self
                .node_earthwork_face_uses_visible_earthwork(graph, terrain, node_id, piece, face)
            {
                continue;
            }
            for (triangle_index, triangle) in face.polygon.triangles_world.iter().enumerate() {
                let Some(probe) = DebugProbeTriangleHit::from_top_triangle(*triangle, point_xz)
                    .or_else(|| DebugProbeTriangleHit::from_near_triangle(*triangle, point_xz))
                else {
                    continue;
                };
                if !probe.contains_xz && probe.xz_distance_m > PROBE_VERTICAL_NEAR_RADIUS_M {
                    continue;
                }
                Self::append_probe_match_separator(dump, first_match);
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"node_earthwork_face\",\"node_id\":{},\"piece_kind\":\"{:?}\",\"face_index\":{},\"face_kind\":\"{:?}\",\"triangle_index\":{},\"source\":",
                    node_id, piece.kind, face_index, face.kind, triangle_index
                );
                Self::append_earthwork_face_source_debug_literal(dump, face.source);
                Self::append_probe_hit_fields(dump, probe);
                dump.push('}');
            }
        }
    }

    fn append_span_probe_top_matches(
        &self,
        dump: &mut String,
        first_match: &mut bool,
        piece: &RoadSurfaceVisualSpanPiece,
        point_xz: backend::RoadVec2,
    ) {
        for (region_index, region) in piece.span_owned_regions.iter().enumerate() {
            for (triangle_index, triangle) in region.polygon.triangles_world.iter().enumerate() {
                let Some(probe) = DebugProbeTriangleHit::from_top_triangle(*triangle, point_xz)
                else {
                    continue;
                };
                if !probe.contains_xz && probe.xz_distance_m > PROBE_TOP_NEAR_RADIUS_M {
                    continue;
                }
                Self::append_probe_match_separator(dump, first_match);
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"span_top\",\"edge_idx\":{},\"region\":{},\"role\":\"{}\",\"source_band_index\":{},\"band_kind\":\"{:?}\",\"material\":\"{}\",\"start_section_index\":{},\"end_section_index\":{},\"start_s_m\":{:.3},\"end_s_m\":{:.3},\"triangle_index\":{}",
                    piece.edge_idx,
                    region_index,
                    Self::span_region_role_debug_name(region.role),
                    region.owner.source_band_index,
                    region.owner.kind,
                    Self::debug_material_for_span_region_role(region.role),
                    region.start_section_index,
                    region.end_section_index,
                    region.start_s_m,
                    region.end_s_m,
                    triangle_index
                );
                Self::append_probe_hit_fields(dump, probe);
                dump.push('}');
            }
        }
    }

    fn append_span_probe_raised_step_matches(
        &self,
        dump: &mut String,
        first_match: &mut bool,
        piece: &RoadSurfaceVisualSpanPiece,
        point_xz: backend::RoadVec2,
    ) {
        for (face_index, polygon) in piece.raised_step_face_polygons.iter().enumerate() {
            let source = piece.span_raised_step_sources.get(face_index).copied();
            for (triangle_index, triangle) in polygon.triangles_world.iter().enumerate() {
                let Some(probe) = DebugProbeTriangleHit::from_near_triangle(*triangle, point_xz)
                else {
                    continue;
                };
                if probe.xz_distance_m > PROBE_VERTICAL_NEAR_RADIUS_M {
                    continue;
                }
                Self::append_probe_match_separator(dump, first_match);
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"span_raised_step_face\",\"edge_idx\":{},\"face_index\":{},\"triangle_index\":{}",
                    piece.edge_idx, face_index, triangle_index
                );
                if let Some(source) = source {
                    dump.push_str(",\"lower_owner\":");
                    Self::append_span_band_owner_debug_literal(dump, source.lower_owner);
                    dump.push_str(",\"raised_owner\":");
                    Self::append_span_band_owner_debug_literal(dump, source.raised_owner);
                    let _ = write!(
                        dump,
                        ",\"start_section_index\":{},\"end_section_index\":{},\"start_s_m\":{:.3},\"end_s_m\":{:.3}",
                        source.start_section_index,
                        source.end_section_index,
                        source.start_s_m,
                        source.end_s_m
                    );
                }
                Self::append_probe_hit_fields(dump, probe);
                dump.push('}');
            }
        }
    }

    fn append_span_probe_earthwork_matches(
        &self,
        dump: &mut String,
        first_match: &mut bool,
        piece: &RoadSurfaceVisualSpanPiece,
        point_xz: backend::RoadVec2,
    ) {
        for (face_index, face) in piece.render_earthwork_faces.iter().enumerate() {
            if !self.span_earthwork_face_uses_visible_earthwork(face) {
                continue;
            }
            for (triangle_index, triangle) in face.polygon.triangles_world.iter().enumerate() {
                let Some(probe) = DebugProbeTriangleHit::from_top_triangle(*triangle, point_xz)
                    .or_else(|| DebugProbeTriangleHit::from_near_triangle(*triangle, point_xz))
                else {
                    continue;
                };
                if !probe.contains_xz && probe.xz_distance_m > PROBE_VERTICAL_NEAR_RADIUS_M {
                    continue;
                }
                Self::append_probe_match_separator(dump, first_match);
                let _ = write!(
                    dump,
                    "{{\"source_kind\":\"span_earthwork_face\",\"edge_idx\":{},\"face_index\":{},\"face_kind\":\"{:?}\",\"triangle_index\":{},\"source\":",
                    piece.edge_idx, face_index, face.kind, triangle_index
                );
                Self::append_earthwork_face_source_debug_literal(dump, face.source);
                Self::append_probe_hit_fields(dump, probe);
                dump.push('}');
            }
        }
    }

    fn append_probe_match_separator(dump: &mut String, first_match: &mut bool) {
        if !*first_match {
            dump.push_str(", ");
        }
        *first_match = false;
    }

    fn append_probe_hit_fields(dump: &mut String, probe: DebugProbeTriangleHit) {
        let relation = if probe.contains_xz {
            "contains"
        } else {
            "near_xz"
        };
        let _ = write!(
            dump,
            ",\"xz_relation\":\"{}\",\"xz_distance_m\":{:.6},\"height_at_probe_m\":",
            relation, probe.xz_distance_m
        );
        Self::append_optional_f32_precise_literal(dump, probe.height_at_probe_m);
        dump.push_str(",\"barycentric\":");
        if let Some((wa, wb, wc)) = probe.barycentric {
            let _ = write!(dump, "[{:.6}, {:.6}, {:.6}]", wa, wb, wc);
        } else {
            dump.push_str("null");
        }
        dump.push_str(",\"triangle_world\":");
        Self::append_vector3_triangle_precise_literal(dump, probe.triangle);
    }
}

#[derive(Clone, Copy)]
struct DebugProbeTriangleHit {
    triangle: [backend::RoadVec3; 3],
    contains_xz: bool,
    xz_distance_m: f64,
    height_at_probe_m: Option<f32>,
    barycentric: Option<(f64, f64, f64)>,
}

impl DebugProbeTriangleHit {
    fn from_top_triangle(
        triangle: [backend::RoadVec3; 3],
        point_xz: backend::RoadVec2,
    ) -> Option<Self> {
        let barycentric = probe_triangle_barycentric_weights_xz(triangle, point_xz);
        let xz_distance_m = if barycentric.is_some() {
            0.0
        } else {
            probe_point_to_triangle_xz_distance_m(triangle, point_xz)
        };
        Some(Self {
            triangle,
            contains_xz: barycentric.is_some(),
            xz_distance_m,
            height_at_probe_m: barycentric.map(|(wa, wb, wc)| {
                (triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc) as f32
            }),
            barycentric,
        })
    }

    fn from_near_triangle(
        triangle: [backend::RoadVec3; 3],
        point_xz: backend::RoadVec2,
    ) -> Option<Self> {
        let xz_distance_m = probe_point_to_triangle_xz_distance_m(triangle, point_xz);
        xz_distance_m.is_finite().then_some(Self {
            triangle,
            contains_xz: false,
            xz_distance_m,
            height_at_probe_m: None,
            barycentric: None,
        })
    }
}

fn probe_triangle_barycentric_weights_xz(
    triangle: [backend::RoadVec3; 3],
    point: backend::RoadVec2,
) -> Option<(f64, f64, f64)> {
    let area = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
        - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
    if area.abs() <= PROBE_EPSILON_M {
        return None;
    }

    let w0 = ((triangle[1].x - point.x) * (triangle[2].z - point.y)
        - (triangle[1].z - point.y) * (triangle[2].x - point.x))
        / area;
    let w1 = ((triangle[2].x - point.x) * (triangle[0].z - point.y)
        - (triangle[2].z - point.y) * (triangle[0].x - point.x))
        / area;
    let w2 = 1.0 - w0 - w1;
    if w0 < -PROBE_EPSILON_M || w1 < -PROBE_EPSILON_M || w2 < -PROBE_EPSILON_M {
        return None;
    }
    Some((w0, w1, w2))
}

fn probe_point_to_triangle_xz_distance_m(
    triangle: [backend::RoadVec3; 3],
    point: backend::RoadVec2,
) -> f64 {
    let a = backend::road_vec3_xz(triangle[0]);
    let b = backend::road_vec3_xz(triangle[1]);
    let c = backend::road_vec3_xz(triangle[2]);
    probe_point_to_segment_xz_distance_m(point, a, b)
        .min(probe_point_to_segment_xz_distance_m(point, b, c))
        .min(probe_point_to_segment_xz_distance_m(point, c, a))
}

fn probe_point_to_segment_xz_distance_m(
    point: backend::RoadVec2,
    start: backend::RoadVec2,
    end: backend::RoadVec2,
) -> f64 {
    let segment = end - start;
    let length_sq = segment.length_squared();
    if length_sq <= f64::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(segment) / length_sq).clamp(0.0, 1.0);
    point.distance(start + segment * t)
}
