//! Earthwork boundary segment extraction, loop assembly, and winding orientation.

use super::super::{
    RoadSurfaceSpanOwnedRegion, RoadSurfaceSystem, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M,
};
use super::model::{
    EarthworkBoundaryEdgeKey, EarthworkBoundaryPointKey, IndexedEarthworkBoundarySegment,
};
use super::{
    RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkFaceSource,
    RoadSurfaceEarthworkGeometryError, RoadSurfaceEarthworkSupportPolicy,
};
use crate::simulation::network::types::EdgeClass;
use godot::prelude::Vector2;
use std::collections::{BTreeMap, BTreeSet};

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn span_earthwork_boundary_segment_loops_from_support_regions(
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

    pub(in crate::simulation::network::surface) fn orient_earthwork_boundary_segment_loops_by_nesting(
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use godot::prelude::Vector3;

    fn test_earthwork_source() -> RoadSurfaceEarthworkFaceSource {
        RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
            edge_idx: 0,
            edge_class: EdgeClass::Standard,
            support_policy: RoadSurfaceEarthworkSupportPolicy::StandardFullGroundedSpan,
            owner: super::super::super::RoadSurfaceSpanBandOwner {
                source_band_index: 0,
                kind: super::super::super::RoadSurfaceBandKind::Carriageway,
            },
            role: super::super::super::RoadSurfaceSpanRegionRole::Asphalt,
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
