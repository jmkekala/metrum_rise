//! Road-owned earthwork generation, terrain stamping, and structural visibility rules.

use super::{
    ChunkCacheKind, NodeOverlayContour, NodeOverlayShapes, RoadSurfaceSection,
    RoadSurfaceSpanOwnedRegion, RoadSurfaceSystem, RoadSurfaceVisualNodePiece,
    RoadSurfaceVisualPolygon, RoadSurfaceVisualSpanPiece, SAMPLE_EPSILON_M, SurfaceChunkKey,
    backend,
};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet};

mod model;
mod stamping;

use model::{EarthworkBoundaryEdgeKey, EarthworkBoundaryPointKey, IndexedEarthworkBoundarySegment};

pub(crate) use model::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceKind,
    RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkGeometryError,
    RoadSurfaceEarthworkRenderFace, RoadSurfaceEarthworkSupportPolicy,
};

// Vertical roadbed offset applied when terrain earthworks need pavement clearance.
pub(super) const EARTHWORK_PAVEMENT_DEPTH_M: f32 = 0.04;

// Lateral terrain probing envelope and sampling cadence for slopes.
const EARTHWORK_MIN_MARGIN_M: f32 = 4.0;
pub(super) const EARTHWORK_MAX_MARGIN_M: f32 = 18.0;
const EARTHWORK_MARGIN_SAMPLE_STEP_M: f32 = 1.0;

// Earthwork slope rates and retaining-wall classification threshold.
const EARTHWORK_CUT_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_FILL_SLOPE_RATE: f32 = 0.5;
const EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD: f32 = 1.25;

// Structural end caps that constrain bridge abutments and tunnel portal stamps.
const BRIDGE_ABUTMENT_LENGTH_M: f32 = 12.0;
const TUNNEL_PORTAL_STAMP_DEPTH_M: f32 = 1.0;

impl RoadSurfaceSystem {
    /// Rebuilds terrain earthworks only for the currently dirty road-surface chunks.
    pub fn rebuild_dirty_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
    ) -> Vec<SurfaceChunkKey> {
        let had_dirty_work = !self.compiled_once
            || !self.dirty_edges.is_empty()
            || !self.dirty_nodes.is_empty()
            || !self.dirty_surface_chunks.is_empty()
            || !self.dirty_terrain_chunks.is_empty();
        self.compile_dirty(graph, terrain);

        let chunks = if had_dirty_work {
            self.last_rebuilt_terrain_chunks.clone()
        } else {
            self.collect_all_chunks(ChunkCacheKind::Earthwork)
        };
        self.apply_earthwork_chunks(graph, terrain, &chunks);
        chunks
    }

    /// Rebuilds terrain earthworks for the whole world from the current compiled roadbed cache.
    pub fn rebuild_all_earthworks(
        &mut self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
    ) -> Vec<SurfaceChunkKey> {
        terrain.reset_visuals_from_source();
        self.compile_dirty(graph, terrain);
        let chunks = self.collect_all_chunks(ChunkCacheKind::Earthwork);
        self.apply_earthwork_chunks(graph, terrain, &chunks);
        chunks
    }
    fn apply_earthwork_chunks(
        &self,
        graph: &RegionGraph,
        terrain: &mut TerrainSystem,
        chunks: &[SurfaceChunkKey],
    ) {
        for &chunk in chunks {
            let (chunk_min, chunk_max) = self.chunk_bounds(chunk);
            terrain.reset_visual_region_from_source_world(
                chunk_min.x,
                chunk_min.z,
                chunk_max.x,
                chunk_max.z,
            );

            let Some(entry) = self.earthwork_chunk_cache.get(&chunk) else {
                continue;
            };

            for &edge_idx in &entry.edge_indices {
                let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                    continue;
                };
                self.stamp_visual_span_piece_earthworks_for_chunk(piece, chunk, terrain);
            }

            for &node_id in &entry.node_ids {
                if node_id as usize >= graph.node_count() {
                    continue;
                }
                let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                    continue;
                };
                self.stamp_visual_node_piece_earthworks_for_chunk(
                    graph, node_id, piece, chunk, terrain,
                );
            }
        }
    }
    pub(super) fn span_earthwork_boundary_segment_loops_from_support_regions(
        regions: &[RoadSurfaceSpanOwnedRegion],
        edge_class: EdgeClass,
    ) -> Result<Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>, RoadSurfaceEarthworkGeometryError>
    {
        let support_policy = RoadSurfaceEarthworkSupportPolicy::from_edge_class(edge_class);
        let mut candidate_segments = Vec::new();
        for region in regions {
            let source = RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                edge_idx: region.edge_idx,
                edge_class,
                support_policy,
                owner: region.owner,
                role: region.role,
                start_section_index: region.start_section_index,
                end_section_index: region.end_section_index,
                start_s_m: region.start_s_m,
                end_s_m: region.end_s_m,
            };
            Self::push_region_polygon_boundary_segments(
                &region.polygon,
                source,
                &mut candidate_segments,
            );
        }
        Self::owned_region_boundary_segment_loops(candidate_segments)
    }

    fn push_region_polygon_boundary_segments(
        polygon: &RoadSurfaceVisualPolygon,
        source: RoadSurfaceEarthworkFaceSource,
        segments: &mut Vec<RoadSurfaceEarthworkBoundarySegment>,
    ) {
        let points = &polygon.points_world;
        if points.len() < 3 {
            return;
        }
        for index in 0..points.len() {
            let inner_start = points[index];
            let inner_end = points[(index + 1) % points.len()];
            let span_xz = Vector2::new(inner_end.x - inner_start.x, inner_end.z - inner_start.z);
            if span_xz.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
                continue;
            }
            segments.push(RoadSurfaceEarthworkBoundarySegment {
                inner_start,
                inner_end,
                source,
            });
        }
    }

    fn owned_region_boundary_segment_loops(
        candidate_segments: Vec<RoadSurfaceEarthworkBoundarySegment>,
    ) -> Result<Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>, RoadSurfaceEarthworkGeometryError>
    {
        let mut edge_counts = BTreeMap::<EarthworkBoundaryEdgeKey, usize>::new();
        for segment in &candidate_segments {
            let start_key = EarthworkBoundaryPointKey::from_point(segment.inner_start);
            let end_key = EarthworkBoundaryPointKey::from_point(segment.inner_end);
            let Some(edge_key) = EarthworkBoundaryEdgeKey::normalized(start_key, end_key) else {
                continue;
            };
            *edge_counts.entry(edge_key).or_insert(0) += 1;
        }

        let mut boundary_segments = Vec::new();
        for (segment_index, segment) in candidate_segments.into_iter().enumerate() {
            let start_key = EarthworkBoundaryPointKey::from_point(segment.inner_start);
            let end_key = EarthworkBoundaryPointKey::from_point(segment.inner_end);
            let Some(edge_key) = EarthworkBoundaryEdgeKey::normalized(start_key, end_key) else {
                continue;
            };
            if edge_counts.get(&edge_key).copied() != Some(1) {
                continue;
            }
            boundary_segments.push(IndexedEarthworkBoundarySegment {
                segment_index,
                segment,
                start_key,
                end_key,
            });
        }
        Self::assemble_earthwork_boundary_segment_loops(boundary_segments)
    }

    fn assemble_earthwork_boundary_segment_loops(
        mut boundary_segments: Vec<IndexedEarthworkBoundarySegment>,
    ) -> Result<Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>, RoadSurfaceEarthworkGeometryError>
    {
        boundary_segments.sort_by(|a, b| {
            a.start_key
                .cmp(&b.start_key)
                .then(a.end_key.cmp(&b.end_key))
                .then(a.segment_index.cmp(&b.segment_index))
        });

        let mut adjacency = BTreeMap::<EarthworkBoundaryPointKey, Vec<usize>>::new();
        for (index, segment) in boundary_segments.iter().enumerate() {
            adjacency.entry(segment.start_key).or_default().push(index);
            adjacency.entry(segment.end_key).or_default().push(index);
        }
        for (point_key, indices) in adjacency.iter_mut() {
            indices.sort_by(|a, b| {
                let segment_a = &boundary_segments[*a];
                let segment_b = &boundary_segments[*b];
                let other_a = if segment_a.start_key == *point_key {
                    segment_a.end_key
                } else {
                    segment_a.start_key
                };
                let other_b = if segment_b.start_key == *point_key {
                    segment_b.end_key
                } else {
                    segment_b.start_key
                };
                other_a
                    .cmp(&other_b)
                    .then(segment_a.segment_index.cmp(&segment_b.segment_index))
            });
        }

        let mut used = BTreeSet::<usize>::new();
        let mut loops = Vec::new();
        for start_index in 0..boundary_segments.len() {
            if used.contains(&start_index) {
                continue;
            }
            let start_key = boundary_segments[start_index].start_key;
            let mut current_key = boundary_segments[start_index].end_key;
            let mut loop_segments = vec![boundary_segments[start_index].segment];
            used.insert(start_index);

            while current_key != start_key {
                let Some(next_indices) = adjacency.get(&current_key) else {
                    break;
                };
                let Some(next_index) = next_indices
                    .iter()
                    .copied()
                    .find(|candidate| !used.contains(candidate))
                else {
                    break;
                };
                let indexed = boundary_segments[next_index];
                let (segment, next_key) = if indexed.start_key == current_key {
                    (indexed.segment, indexed.end_key)
                } else {
                    (
                        RoadSurfaceEarthworkBoundarySegment {
                            inner_start: indexed.segment.inner_end,
                            inner_end: indexed.segment.inner_start,
                            source: indexed.segment.source,
                        },
                        indexed.start_key,
                    )
                };
                loop_segments.push(segment);
                used.insert(next_index);
                current_key = next_key;
            }

            if current_key != start_key {
                return Err(RoadSurfaceEarthworkGeometryError::OpenBoundaryChain {
                    segment_count: loop_segments.len(),
                });
            }
            if loop_segments.len() < 3 {
                return Err(RoadSurfaceEarthworkGeometryError::DegenerateBoundaryLoop {
                    point_count: loop_segments.len(),
                });
            }
            let point_loop = loop_segments
                .iter()
                .map(|segment| segment.inner_start)
                .collect::<Vec<_>>();
            if Self::signed_polygon_area_xz(&point_loop).abs() <= SAMPLE_EPSILON_M {
                return Err(RoadSurfaceEarthworkGeometryError::DegenerateBoundaryLoop {
                    point_count: point_loop.len(),
                });
            }
            loops.push(loop_segments);
        }

        Self::orient_earthwork_boundary_segment_loops_by_nesting(&mut loops)?;
        Ok(loops)
    }

    pub(super) fn orient_earthwork_boundary_segment_loops_by_nesting(
        loops: &mut [Vec<RoadSurfaceEarthworkBoundarySegment>],
    ) -> Result<(), RoadSurfaceEarthworkGeometryError> {
        let point_loops = loops
            .iter()
            .map(|segments| {
                segments
                    .iter()
                    .map(|segment| segment.inner_start)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for points in &point_loops {
            if points.is_empty() {
                return Err(RoadSurfaceEarthworkGeometryError::DegenerateBoundaryLoop {
                    point_count: points.len(),
                });
            }
        }
        let samples = point_loops
            .iter()
            .map(|points| {
                points.iter().fold(Vector2::ZERO, |sum, point| {
                    sum + Vector2::new(point.x, point.z)
                }) / points.len() as f32
            })
            .collect::<Vec<_>>();
        let should_be_ccw = point_loops
            .iter()
            .enumerate()
            .map(|(loop_index, _)| {
                let depth = point_loops
                    .iter()
                    .enumerate()
                    .filter(|(candidate_index, candidate)| {
                        *candidate_index != loop_index
                            && RoadSurfaceSystem::polygon_contains_point_xz(
                                candidate,
                                samples[loop_index],
                            )
                    })
                    .count();
                depth % 2 == 0
            })
            .collect::<Vec<_>>();
        for (segments, should_be_ccw) in loops.iter_mut().zip(should_be_ccw) {
            let points = segments
                .iter()
                .map(|segment| segment.inner_start)
                .collect::<Vec<_>>();
            let is_ccw = Self::signed_polygon_area_xz(&points) > 0.0;
            if is_ccw != should_be_ccw {
                Self::reverse_earthwork_boundary_segment_loop(segments);
            }
        }
        Ok(())
    }

    fn reverse_earthwork_boundary_segment_loop(
        segments: &mut Vec<RoadSurfaceEarthworkBoundarySegment>,
    ) {
        segments.reverse();
        for segment in segments {
            std::mem::swap(&mut segment.inner_start, &mut segment.inner_end);
        }
    }

    pub(super) fn build_closed_earthwork_geometry_from_boundary_segments(
        &self,
        boundary_segment_loops: &[Vec<RoadSurfaceEarthworkBoundarySegment>],
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> Result<
        (
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceEarthworkRenderFace>,
        ),
        RoadSurfaceEarthworkGeometryError,
    > {
        let mut earthwork_surface_polygons = Vec::new();
        let mut earthwork_outer_boundary_loops = Vec::new();
        let mut render_earthwork_faces = Vec::new();

        for boundary_segments in boundary_segment_loops {
            let Some((outer_loop, side_polygons, render_faces)) = self
                .build_closed_earthwork_loop_geometry(
                    boundary_segments,
                    terrain,
                    top_surface_shapes,
                )?
            else {
                continue;
            };
            if let Some(outer_loop) = outer_loop {
                earthwork_outer_boundary_loops.push(outer_loop);
            }
            earthwork_surface_polygons.extend(side_polygons);
            render_earthwork_faces.extend(render_faces);
        }

        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);
        Ok((
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        ))
    }

    fn build_closed_earthwork_loop_geometry(
        &self,
        boundary_segments: &[RoadSurfaceEarthworkBoundarySegment],
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> Result<
        Option<(
            Option<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceVisualPolygon>,
            Vec<RoadSurfaceEarthworkRenderFace>,
        )>,
        RoadSurfaceEarthworkGeometryError,
    > {
        if boundary_segments.len() < 3 {
            return Err(RoadSurfaceEarthworkGeometryError::DegenerateBoundaryLoop {
                point_count: boundary_segments.len(),
            });
        }
        let boundary_points = boundary_segments
            .iter()
            .map(|segment| segment.inner_start)
            .collect::<Vec<_>>();

        let mut vertex_outer_points = Vec::with_capacity(boundary_points.len());
        for (index, point) in boundary_points.iter().enumerate() {
            let Some(outward) = Self::closed_loop_vertex_outward_xz(&boundary_points, index) else {
                vertex_outer_points.clear();
                break;
            };
            let Some(outer_point) = self.earthwork_transition_point(*point, outward, terrain)
            else {
                vertex_outer_points.clear();
                break;
            };
            vertex_outer_points.push(outer_point);
        }
        // A cusp has no unique vertex miter. Keep edge-based faces, but do not invent a loop point.
        let outer_loop = if vertex_outer_points.len() == boundary_points.len() {
            Self::make_visual_polygon(vertex_outer_points)
        } else {
            None
        };
        let mut side_polygons = Vec::new();
        let mut render_faces = Vec::new();
        let winding_ccw = Self::signed_polygon_area_xz(&boundary_points) > 0.0;
        for segment in boundary_segments {
            let current = segment.inner_start;
            let next = segment.inner_end;
            let (outer_current, outer_next) = self.earthwork_edge_transition_points(
                current,
                next,
                winding_ccw,
                terrain,
                top_surface_shapes,
            )?;
            // Handoff and internal seam edges can be part of a closed footprint loop. They are not
            // terrain tie-ins, so a skirt whose plan area enters solved top ownership is rejected.
            if let Some(top_surface_shapes) = top_surface_shapes
                && Self::earthwork_candidate_intrudes_top(
                    [current, next, outer_next, outer_current],
                    top_surface_shapes,
                )
            {
                continue;
            }
            let Some(polygon) =
                Self::make_visual_polygon(vec![current, next, outer_next, outer_current])
            else {
                continue;
            };
            let face_kind =
                Self::classify_earthwork_face_kind(current, next, outer_next, outer_current);
            render_faces.push(RoadSurfaceEarthworkRenderFace {
                kind: face_kind,
                source: segment.source,
                inner_start: current,
                inner_end: next,
                polygon: polygon.clone(),
            });
            side_polygons.push(polygon);
        }

        if outer_loop.is_none() && side_polygons.is_empty() {
            return Ok(None);
        }
        Ok(Some((outer_loop, side_polygons, render_faces)))
    }

    fn earthwork_edge_transition_points(
        &self,
        current: Vector3,
        next: Vector3,
        winding_ccw: bool,
        terrain: &TerrainSystem,
        top_surface_shapes: Option<&NodeOverlayShapes>,
    ) -> Result<(Vector3, Vector3), RoadSurfaceEarthworkGeometryError> {
        let edge = Vector2::new(next.x - current.x, next.z - current.z);
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

    fn earthwork_candidate_intrudes_top(
        points: [Vector3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> bool {
        let Some((overlap_area_m2, budget_m2)) =
            Self::earthwork_candidate_top_overlap_metrics_m2(points, top_surface_shapes)
        else {
            return true;
        };
        overlap_area_m2 > budget_m2
    }

    fn earthwork_candidate_top_overlap_area_m2(
        points: [Vector3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> Option<f32> {
        Self::earthwork_candidate_top_overlap_metrics_m2(points, top_surface_shapes)
            .map(|(overlap_area_m2, _)| overlap_area_m2)
    }

    fn earthwork_candidate_top_overlap_metrics_m2(
        mut points: [Vector3; 4],
        top_surface_shapes: &NodeOverlayShapes,
    ) -> Option<(f32, f32)> {
        if Self::signed_polygon_area_xz(&points) < 0.0 {
            points.reverse();
        }
        let candidate_shapes =
            Self::overlay_union_contours(&[Self::earthwork_overlay_contour_from_points(&points)])?;
        let overlap = Self::overlay_binary_shapes(
            &candidate_shapes,
            top_surface_shapes,
            OverlayRule::Intersect,
        )?;
        let overlap_area_m2 = overlap.iter().map(Self::overlay_shape_area_m2).sum();
        let budget_m2 = Self::overlay_numeric_area_budget_for_shapes(&candidate_shapes).max(
            Self::overlay_numeric_area_budget_for_shapes(top_surface_shapes),
        );
        Some((overlap_area_m2, budget_m2))
    }

    fn earthwork_overlay_contour_from_points(points: &[Vector3]) -> NodeOverlayContour {
        let mut contour = Vec::with_capacity(points.len());
        for point in points {
            let point = backend::road_vec2_to_overlay_point(backend::godot_vec3_xz_to_road(*point));
            if contour.last().is_none_or(|last| *last != point) {
                contour.push(point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        contour
    }

    pub(super) fn top_surface_overlay_shapes<'a>(
        polygons: impl IntoIterator<Item = &'a RoadSurfaceVisualPolygon>,
    ) -> Option<NodeOverlayShapes> {
        let mut contours = Vec::new();
        for polygon in polygons {
            if polygon.points_world.len() >= 3 {
                contours.push(Self::earthwork_overlay_contour_from_points(
                    &polygon.points_world,
                ));
            }
        }
        Self::overlay_union_contours(&contours)
    }

    fn closed_loop_vertex_outward_xz(boundary_points: &[Vector3], index: usize) -> Option<Vector2> {
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

    fn edge_outward_normal_xz(edge_xz: Vector2, winding_ccw: bool) -> Option<Vector2> {
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

    pub(super) fn classify_earthwork_face_kind(
        inner_start: Vector3,
        inner_end: Vector3,
        outer_end: Vector3,
        outer_start: Vector3,
    ) -> RoadSurfaceEarthworkFaceKind {
        let setback_a =
            Vector2::new(outer_start.x - inner_start.x, outer_start.z - inner_start.z).length();
        let setback_b = Vector2::new(outer_end.x - inner_end.x, outer_end.z - inner_end.z).length();
        let avg_setback = (setback_a + setback_b) * 0.5;
        if avg_setback <= SAMPLE_EPSILON_M {
            return RoadSurfaceEarthworkFaceKind::RetainingWall;
        }

        let max_height_delta = (outer_start.y - inner_start.y)
            .abs()
            .max((outer_end.y - inner_end.y).abs());
        let slope_ratio = max_height_delta / avg_setback.max(SAMPLE_EPSILON_M);
        if slope_ratio >= EARTHWORK_RETAINING_WALL_SLOPE_THRESHOLD {
            RoadSurfaceEarthworkFaceKind::RetainingWall
        } else {
            RoadSurfaceEarthworkFaceKind::Slope
        }
    }
    pub(super) fn sort_earthwork_render_faces(faces: &mut [RoadSurfaceEarthworkRenderFace]) {
        faces.sort_by(|a, b| {
            let kind_order = match (a.kind, b.kind) {
                (
                    RoadSurfaceEarthworkFaceKind::Slope,
                    RoadSurfaceEarthworkFaceKind::RetainingWall,
                ) => std::cmp::Ordering::Less,
                (
                    RoadSurfaceEarthworkFaceKind::RetainingWall,
                    RoadSurfaceEarthworkFaceKind::Slope,
                ) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            };
            if kind_order != std::cmp::Ordering::Equal {
                return kind_order;
            }
            a.source
                .source_ordering(b.source)
                .then(
                    a.inner_start
                        .x
                        .total_cmp(&b.inner_start.x)
                        .then(a.inner_start.z.total_cmp(&b.inner_start.z))
                        .then(a.inner_start.y.total_cmp(&b.inner_start.y)),
                )
                .then(
                    a.polygon
                        .points_world
                        .len()
                        .cmp(&b.polygon.points_world.len()),
                )
                .then_with(|| {
                    match a
                        .polygon
                        .points_world
                        .iter()
                        .zip(&b.polygon.points_world)
                        .find_map(|(point_a, point_b)| {
                            let ordering = point_a
                                .x
                                .total_cmp(&point_b.x)
                                .then(point_a.z.total_cmp(&point_b.z))
                                .then(point_a.y.total_cmp(&point_b.y));
                            (ordering != std::cmp::Ordering::Equal).then_some(ordering)
                        }) {
                        Some(ordering) => ordering,
                        None => std::cmp::Ordering::Equal,
                    }
                })
        });
    }

    pub(super) fn earthwork_transition_point(
        &self,
        road_point: Vector3,
        outward_xz: Vector2,
        terrain: &TerrainSystem,
    ) -> Option<Vector3> {
        if outward_xz.length_squared() <= SAMPLE_EPSILON_M * SAMPLE_EPSILON_M {
            return None;
        }
        let outward_xz = outward_xz.normalized();
        let distance_m = self.earthwork_transition_distance_m(road_point, outward_xz, terrain);
        let outer_xz = Vector2::new(road_point.x, road_point.z) + outward_xz * distance_m;
        let outer_height_m =
            terrain.sample_height_world(outer_xz.x, outer_xz.y) * config::HEIGHT_SCALE;
        Some(Vector3::new(outer_xz.x, outer_height_m, outer_xz.y))
    }

    fn earthwork_transition_distance_m(
        &self,
        road_point: Vector3,
        outward_xz: Vector2,
        terrain: &TerrainSystem,
    ) -> f32 {
        let source_height_at_edge =
            terrain.sample_height_world(road_point.x, road_point.z) * config::HEIGHT_SCALE;
        let cut_side = source_height_at_edge > road_point.y;
        let slope_rate = if cut_side {
            EARTHWORK_CUT_SLOPE_RATE
        } else {
            EARTHWORK_FILL_SLOPE_RATE
        };

        let mut distance_m = EARTHWORK_MIN_MARGIN_M;
        while distance_m < EARTHWORK_MAX_MARGIN_M {
            let sample_x = road_point.x + outward_xz.x * distance_m;
            let sample_z = road_point.z + outward_xz.y * distance_m;
            let source_height =
                terrain.sample_height_world(sample_x, sample_z) * config::HEIGHT_SCALE;
            let transition_height = if cut_side {
                road_point.y + slope_rate * distance_m
            } else {
                road_point.y - slope_rate * distance_m
            };
            let rejoins_source = if cut_side {
                transition_height >= source_height
            } else {
                transition_height <= source_height
            };
            if rejoins_source {
                return distance_m;
            }
            distance_m += EARTHWORK_MARGIN_SAMPLE_STEP_M;
        }

        EARTHWORK_MAX_MARGIN_M
    }

    fn stamp_visual_span_piece_earthworks_for_chunk(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        if !self.span_piece_uses_visible_earthwork(piece) {
            return;
        }

        let height_offset_m = self.span_piece_integrated_surface_offset_m(piece);
        self.stamp_span_top_surface_support_for_chunk(
            &piece.span_earthwork_support_regions,
            chunk,
            terrain,
            height_offset_m,
        );
    }

    fn stamp_visual_node_piece_earthworks_for_chunk(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        piece: &RoadSurfaceVisualNodePiece,
        chunk: SurfaceChunkKey,
        terrain: &mut TerrainSystem,
    ) {
        if !self.node_piece_uses_earthworks(graph, node_id, terrain) {
            return;
        }
        if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
            return;
        }

        let height_offset_m = self.node_piece_integrated_surface_offset_m(graph, node_id, terrain);
        self.stamp_piece_top_surface_clearance_for_chunk(
            &piece.road_surface_polygons,
            &piece.curb_surface_polygons,
            &piece.sidewalk_surface_polygons,
            chunk,
            terrain,
            height_offset_m,
        );
    }

    pub(super) fn node_piece_uses_earthworks(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        terrain: &TerrainSystem,
    ) -> bool {
        if node_id as usize >= graph.node_adjacency_count() {
            return false;
        }

        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted || !Self::is_surface_edge(edge) {
                continue;
            }
            if edge.class != EdgeClass::Tunnel || edge.primary_type == TransitType::Foot {
                return true;
            }

            let at_start = graph.get_valid_node(edge.start_node) == node_id;
            if self.tunnel_throat_is_visible(edge_idx, at_start, terrain) {
                return true;
            }
        }

        false
    }

    pub(super) fn earthwork_section_ranges_for_edge(
        &self,
        edge: &Edge,
        sections: &[RoadSurfaceSection],
        terrain: &TerrainSystem,
    ) -> Vec<(usize, usize)> {
        let Some((start_index, end_index)) = self.corridor_index_range_for_edge(edge, sections)
        else {
            return Vec::new();
        };

        match edge.class {
            EdgeClass::Standard => vec![(start_index, end_index)],
            EdgeClass::Bridge => self.endpoint_limited_section_ranges(
                sections,
                start_index,
                end_index,
                BRIDGE_ABUTMENT_LENGTH_M,
            ),
            EdgeClass::Tunnel => {
                self.tunnel_visible_section_ranges(sections, start_index, end_index, terrain)
            }
        }
    }

    fn corridor_index_range_for_edge(
        &self,
        edge: &Edge,
        sections: &[RoadSurfaceSection],
    ) -> Option<(usize, usize)> {
        if sections.len() < 2 {
            return None;
        }

        // Bridge abutments and tunnel portals are structural endpoint regions; trimming them by
        // the ordinary road-width handoff can either erase portals or collapse short spans into
        // one full-length stamp.
        if edge.class != EdgeClass::Standard {
            return Some((0, sections.len().saturating_sub(1)));
        }

        let total_length = sections.last()?.s_m.max(0.0);
        let start_handoff = Self::visual_start_handoff_m(edge, total_length);
        let end_handoff = Self::visual_end_handoff_s_m(edge, total_length);
        Self::section_index_range_for_s_bounds(sections, start_handoff, end_handoff)
    }

    fn endpoint_limited_section_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        start_index: usize,
        end_index: usize,
        endpoint_length_m: f32,
    ) -> Vec<(usize, usize)> {
        if end_index <= start_index {
            return Vec::new();
        }

        let start_s = sections[start_index].s_m;
        let end_s = sections[end_index].s_m;
        if end_s - start_s <= endpoint_length_m * 2.0 {
            return vec![(start_index, end_index)];
        }

        let mut ranges = Vec::new();
        if let Some(start_end) = sections[start_index..=end_index]
            .iter()
            .rposition(|section| section.s_m <= start_s + endpoint_length_m + SAMPLE_EPSILON_M)
            .map(|offset| start_index + offset)
        {
            if start_end > start_index {
                ranges.push((start_index, start_end));
            }
        }

        if let Some(end_start) = sections[start_index..=end_index]
            .iter()
            .position(|section| section.s_m >= end_s - endpoint_length_m - SAMPLE_EPSILON_M)
            .map(|offset| start_index + offset)
        {
            if end_index > end_start {
                ranges.push((end_start, end_index));
            }
        }

        ranges.sort_unstable();
        ranges.dedup();
        ranges
    }

    pub(super) fn tunnel_visible_section_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        start_index: usize,
        end_index: usize,
        terrain: &TerrainSystem,
    ) -> Vec<(usize, usize)> {
        if end_index <= start_index {
            return Vec::new();
        }

        let mut ranges = Vec::new();

        if self.section_is_tunnel_surface_visible(&sections[start_index], terrain) {
            let mut visible_end = start_index;
            while visible_end < end_index
                && self.section_is_tunnel_surface_visible(&sections[visible_end + 1], terrain)
            {
                visible_end += 1;
            }
            let transition_end = (visible_end + 1).min(end_index);
            if transition_end > start_index {
                ranges.push((start_index, transition_end));
            }
        }

        if self.section_is_tunnel_surface_visible(&sections[end_index], terrain) {
            let mut visible_start = end_index;
            while visible_start > start_index
                && self.section_is_tunnel_surface_visible(&sections[visible_start - 1], terrain)
            {
                visible_start -= 1;
            }
            let transition_start = visible_start.saturating_sub(1).max(start_index);
            if end_index > transition_start {
                if let Some(last) = ranges.last_mut() {
                    if transition_start <= last.1 {
                        last.1 = end_index;
                    } else {
                        ranges.push((transition_start, end_index));
                    }
                } else {
                    ranges.push((transition_start, end_index));
                }
            }
        }

        ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_earthwork_source() -> RoadSurfaceEarthworkFaceSource {
        RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
            edge_idx: 0,
            edge_class: EdgeClass::Standard,
            support_policy: RoadSurfaceEarthworkSupportPolicy::StandardFullGroundedSpan,
            owner: super::super::RoadSurfaceSpanBandOwner {
                source_band_index: 0,
                kind: super::super::RoadSurfaceBandKind::Carriageway,
            },
            role: super::super::RoadSurfaceSpanRegionRole::Asphalt,
            start_section_index: 0,
            end_section_index: 1,
            start_s_m: 0.0,
            end_s_m: 1.0,
        }
    }

    fn boundary_segment(
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
    ) -> RoadSurfaceEarthworkBoundarySegment {
        RoadSurfaceEarthworkBoundarySegment {
            inner_start: Vector3::new(start_x, 0.0, start_z),
            inner_end: Vector3::new(end_x, 0.0, end_z),
            source: test_earthwork_source(),
        }
    }

    #[test]
    fn earthwork_boundary_loop_assembly_rejects_open_chain() {
        let result = RoadSurfaceSystem::owned_region_boundary_segment_loops(vec![
            boundary_segment(0.0, 0.0, 1.0, 0.0),
            boundary_segment(1.0, 0.0, 1.0, 1.0),
        ]);

        assert!(matches!(
            result,
            Err(RoadSurfaceEarthworkGeometryError::OpenBoundaryChain { segment_count: 2 })
        ));
    }

    #[test]
    fn earthwork_vertex_outward_rejects_degenerate_spur() {
        let points = vec![
            Vector3::new(-1.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(-1.0, 0.0, 0.0),
        ];

        assert!(RoadSurfaceSystem::closed_loop_vertex_outward_xz(&points, 1).is_none());
    }

    #[test]
    fn earthwork_edge_outward_accepts_short_nonzero_edges() {
        let outward = RoadSurfaceSystem::edge_outward_normal_xz(
            Vector2::new(SAMPLE_EPSILON_M * 10.0, 0.0),
            true,
        );

        assert_eq!(outward, Some(Vector2::new(0.0, -1.0)));
    }

    #[test]
    fn earthwork_loop_orientation_preserves_prevalidated_skinny_loops() {
        let mut loops = vec![vec![
            boundary_segment(0.0, 0.0, 0.01, 0.0),
            boundary_segment(0.01, 0.0, 0.0, 0.01),
            boundary_segment(0.0, 0.01, 0.0, 0.0),
        ]];

        assert!(
            RoadSurfaceSystem::orient_earthwork_boundary_segment_loops_by_nesting(&mut loops)
                .is_ok()
        );
    }
}
