// SPDX-License-Identifier: GPL-2.0-only

//! Compact span terrain-clip loops: points plus edges keyed into them and the span's sections.

use super::super::{
    RoadSurfaceBandKind, RoadSurfaceEarthworkFaceSource, RoadSurfaceEarthworkSupportPolicy,
    RoadSurfaceSection, RoadSurfaceSystem, RoadSurfaceTerrainClipLoop,
    RoadSurfaceTerrainClipSourceEdge, backend::RoadVec3,
};
use super::{RoadSurfaceSpanBandOwner, RoadSurfaceSpanRegionRole, RoadSurfaceVisualSpanPiece};
use crate::simulation::network::types::EdgeClass;

/// One span terrain-clip loop. Its edges are span support or handoff boundaries, so each is
/// stored as endpoint indices and a band and section pair (16 B instead of 176 B); the edge
/// kind and the rest of the source are rebuilt from the piece and its sections on read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpanTerrainClipLoop {
    points_world: Box<[RoadVec3]>,
    edges: SpanTerrainClipEdges,
}

#[derive(Clone, Debug, PartialEq)]
enum SpanTerrainClipEdges {
    // Every edge rebuilds bit for bit from its key. Endpoints index the loop points, then
    // `extra_points`: curb-step corners of handoff edges that the loop polygon merged.
    Keyed {
        edges: Box<[SpanTerrainClipEdge]>,
        extra_points: Box<[RoadVec3]>,
    },
    // Fallback when some edge does not; sources carry the piece's edge id.
    Explicit(Box<[RoadSurfaceTerrainClipSourceEdge]>),
}

// A support boundary spans sections `start_section` and `start_section + 1`; a handoff has
// equal sections. `start` and `end` index the loop points followed by the extra points.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SpanTerrainClipEdge {
    start_section: u32,
    end_section: u32,
    start: u16,
    end: u16,
    band: u16,
    kind: RoadSurfaceBandKind,
}

impl SpanTerrainClipLoop {
    // Keeps the keyed form only when it rebuilds every edge exactly. O(P log P + E log P).
    pub(super) fn new(
        clip_loop: RoadSurfaceTerrainClipLoop,
        edge_idx: usize,
        edge_class: EdgeClass,
        sections: &[RoadSurfaceSection],
    ) -> Self {
        let points_world = clip_loop.points_world.into_boxed_slice();
        let edges = match Self::keyed_edges(
            &points_world,
            &clip_loop.source_edges,
            edge_idx,
            edge_class,
            sections,
        ) {
            Some((edges, extra_points)) => SpanTerrainClipEdges::Keyed {
                edges,
                extra_points,
            },
            None => SpanTerrainClipEdges::Explicit(clip_loop.source_edges.into_boxed_slice()),
        };
        Self {
            points_world,
            edges,
        }
    }

    fn keyed_edges(
        points_world: &[RoadVec3],
        source_edges: &[RoadSurfaceTerrainClipSourceEdge],
        edge_idx: usize,
        edge_class: EdgeClass,
        sections: &[RoadSurfaceSection],
    ) -> Option<(Box<[SpanTerrainClipEdge]>, Box<[RoadVec3]>)> {
        let point_bits = |point: RoadVec3| [point.x, point.y, point.z].map(f64::to_bits);
        let mut indexed_points = points_world
            .iter()
            .enumerate()
            .map(|(index, &point)| Some((point_bits(point), u16::try_from(index).ok()?)))
            .collect::<Option<Vec<_>>>()?;
        indexed_points.sort_unstable();
        let mut extra_points = Vec::<RoadVec3>::new();
        // Endpoints index a bit-identical point; equal bits give equal points at any index.
        // Misses are few per loop, so the extra list is searched linearly.
        let mut index_of = |point: RoadVec3| {
            let bits = point_bits(point);
            let index = indexed_points.partition_point(|entry| entry.0 < bits);
            if let Some(entry) = indexed_points.get(index).filter(|entry| entry.0 == bits) {
                return Some(entry.1);
            }
            let extra = match extra_points
                .iter()
                .position(|&extra| point_bits(extra) == bits)
            {
                Some(extra) => extra,
                None => {
                    extra_points.push(point);
                    extra_points.len() - 1
                }
            };
            u16::try_from(points_world.len() + extra).ok()
        };
        let edges = source_edges
            .iter()
            .map(|source_edge| {
                let RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
                    owner,
                    start_section_index,
                    end_section_index,
                    ..
                } = source_edge.source
                else {
                    return None;
                };
                let edge = SpanTerrainClipEdge {
                    start_section: u32::try_from(start_section_index).ok()?,
                    end_section: u32::try_from(end_section_index).ok()?,
                    start: index_of(source_edge.start)?,
                    end: index_of(source_edge.end)?,
                    band: u16::try_from(owner.source_band_index).ok()?,
                    kind: owner.kind,
                };
                Some(edge)
            })
            .collect::<Option<Box<[_]>>>()?;
        edges
            .iter()
            .zip(source_edges)
            .all(|(edge, source_edge)| {
                edge.source_edge(points_world, &extra_points, edge_idx, edge_class, sections)
                    .is_some_and(|rebuilt| rebuilt == *source_edge)
            })
            .then(|| (edges, extra_points.into_boxed_slice()))
    }

    // Rebuilds the loop for a span with this edge id, class and sections.
    fn boundary_loop(
        &self,
        edge_idx: usize,
        edge_class: EdgeClass,
        sections: &[RoadSurfaceSection],
    ) -> RoadSurfaceTerrainClipLoop {
        let source_edges = match &self.edges {
            SpanTerrainClipEdges::Keyed {
                edges,
                extra_points,
            } => edges
                .iter()
                .map(|edge| {
                    edge.source_edge(
                        &self.points_world,
                        extra_points,
                        edge_idx,
                        edge_class,
                        sections,
                    )
                    .expect("keyed span clip edges index the sections they compiled from")
                })
                .collect(),
            SpanTerrainClipEdges::Explicit(edges) => edges.to_vec(),
        };
        RoadSurfaceTerrainClipLoop {
            points_world: self.points_world.to_vec(),
            source_edges,
        }
    }

    /// Whether every edge is stored in the keyed form.
    #[cfg(test)]
    pub(crate) fn is_keyed(&self) -> bool {
        matches!(self.edges, SpanTerrainClipEdges::Keyed { .. })
    }

    /// Loop points in order.
    pub(crate) fn points(&self) -> &[RoadVec3] {
        &self.points_world
    }

    // Explicit sources carry the edge id; keyed edges take it from the piece on read.
    pub(super) fn set_span_identity(&mut self, edge_idx: usize) {
        if let SpanTerrainClipEdges::Explicit(edges) = &mut self.edges {
            for edge in edges.iter_mut() {
                edge.source = edge.source.with_span_identity(edge_idx);
            }
        }
    }
}

impl SpanTerrainClipEdge {
    fn source_edge(
        self,
        points_world: &[RoadVec3],
        extra_points: &[RoadVec3],
        edge_idx: usize,
        edge_class: EdgeClass,
        sections: &[RoadSurfaceSection],
    ) -> Option<RoadSurfaceTerrainClipSourceEdge> {
        // Region bands keep one kind across both sections, so the role is the band pair's.
        let source = RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
            edge_idx,
            edge_class,
            support_policy: RoadSurfaceEarthworkSupportPolicy::from_edge_class(edge_class),
            owner: RoadSurfaceSpanBandOwner {
                source_band_index: usize::from(self.band),
                kind: self.kind,
            },
            role: RoadSurfaceSpanRegionRole::from_band_pair(self.kind, self.kind),
            start_section_index: self.start_section as usize,
            end_section_index: self.end_section as usize,
            start_s_m: sections.get(self.start_section as usize)?.s_m,
            end_s_m: sections.get(self.end_section as usize)?.s_m,
        };
        let point = |index: u16| {
            let index = usize::from(index);
            points_world
                .get(index)
                .or_else(|| extra_points.get(index - points_world.len()))
                .copied()
        };
        Some(RoadSurfaceTerrainClipSourceEdge {
            start: point(self.start)?,
            end: point(self.end)?,
            kind: RoadSurfaceSystem::span_terrain_clip_edge_kind_for_source(source),
            source,
        })
    }
}

impl RoadSurfaceVisualSpanPiece {
    /// Rebuilds one of this piece's terrain-clip loops. O(points + edges); allocates the loop.
    pub(crate) fn terrain_clip_boundary_loop(
        &self,
        clip_loop: &SpanTerrainClipLoop,
    ) -> RoadSurfaceTerrainClipLoop {
        clip_loop.boundary_loop(self.edge_idx, self.edge_class, &self.sections)
    }

    /// Every terrain-clip loop of the span, rebuilt in compiled order.
    pub(crate) fn terrain_clip_boundary_loops(&self) -> Vec<RoadSurfaceTerrainClipLoop> {
        self.terrain_clip_loops
            .iter()
            .map(|clip_loop| self.terrain_clip_boundary_loop(clip_loop))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{RoadSurfaceTerrainClipEdgeKind, backend::RoadVec2};
    use super::*;

    fn section(s_m: f32) -> RoadSurfaceSection {
        RoadSurfaceSection {
            edge_idx: 7,
            s_m,
            center_xz: RoadVec2::new(f64::from(s_m), 0.0),
            center_height_m: 0.0,
            tangent_xz: RoadVec2::new(1.0, 0.0),
            lateral_xz: RoadVec2::new(0.0, 1.0),
            bands: Vec::new(),
        }
    }

    fn support_edge(
        start: RoadVec3,
        end: RoadVec3,
        kind: RoadSurfaceBandKind,
        sections: [usize; 2],
        edge_class: EdgeClass,
        all_sections: &[RoadSurfaceSection],
    ) -> RoadSurfaceTerrainClipSourceEdge {
        let edge = SpanTerrainClipEdge {
            start_section: sections[0] as u32,
            end_section: sections[1] as u32,
            start: 0,
            end: 0,
            band: 2,
            kind,
        };
        RoadSurfaceTerrainClipSourceEdge {
            start,
            end,
            ..edge
                .source_edge(
                    &[RoadVec3::new(0.0, 0.0, 0.0)],
                    &[],
                    7,
                    edge_class,
                    all_sections,
                )
                .unwrap()
        }
    }

    #[test]
    fn span_clip_loops_rebuild_every_storage_form_bit_for_bit() {
        let sections = [section(0.0), section(4.5), section(9.25)];
        let points = vec![
            RoadVec3::new(0.0, 1.0, -3.0),
            RoadVec3::new(4.5, 1.25, -3.0),
            RoadVec3::new(9.25, 1.5, 3.0),
            RoadVec3::new(0.0, 1.0, 3.0),
        ];
        let sidewalk = RoadSurfaceBandKind::Sidewalk;
        let shoulder = RoadSurfaceBandKind::CurbOrShoulder;
        let keyed_edges = vec![
            support_edge(
                points[0],
                points[1],
                sidewalk,
                [0, 1],
                EdgeClass::Bridge,
                &sections,
            ),
            support_edge(
                points[1],
                points[2],
                shoulder,
                [1, 2],
                EdgeClass::Bridge,
                &sections,
            ),
            support_edge(
                points[2],
                points[3],
                sidewalk,
                [2, 2],
                EdgeClass::Bridge,
                &sections,
            ),
            support_edge(
                points[3],
                points[0],
                sidewalk,
                [0, 0],
                EdgeClass::Bridge,
                &sections,
            ),
        ];
        assert_eq!(
            keyed_edges[2].kind,
            RoadSurfaceTerrainClipEdgeKind::SpanHandoff
        );
        assert_eq!(
            keyed_edges[0].kind,
            RoadSurfaceTerrainClipEdgeKind::SidewalkOuter
        );
        let keyed = RoadSurfaceTerrainClipLoop {
            points_world: points.clone(),
            source_edges: keyed_edges.clone(),
        };

        // An endpoint that is not a loop point bit for bit (here only its sign differs) is
        // kept as an extra point, as curb-step corners of handoff edges are.
        let mut signed_edges = keyed_edges.clone();
        signed_edges[0].start.y = -0.0;
        let mut signed_points = points.clone();
        signed_points[0].y = 0.0;
        // A source the key cannot express (a foreign edge id) stays explicit.
        let mut foreign_edges = keyed_edges.clone();
        foreign_edges[1].source = foreign_edges[1].source.with_span_identity(8);
        let cases = [
            (keyed, true),
            (
                RoadSurfaceTerrainClipLoop {
                    points_world: signed_points,
                    source_edges: signed_edges,
                },
                true,
            ),
            (
                RoadSurfaceTerrainClipLoop {
                    points_world: points,
                    source_edges: foreign_edges,
                },
                false,
            ),
        ];
        for (index, (clip_loop, expect_keyed)) in cases.into_iter().enumerate() {
            let stored =
                SpanTerrainClipLoop::new(clip_loop.clone(), 7, EdgeClass::Bridge, &sections);
            assert_eq!(stored.is_keyed(), expect_keyed, "case {index}");
            let rebuilt = stored.boundary_loop(7, EdgeClass::Bridge, &sections);
            assert_eq!(
                format!("{rebuilt:?}"),
                format!("{clip_loop:?}"),
                "case {index}"
            );

            // Remapping the span id rebuilds the same loop with every source remapped.
            let mut remapped = stored.clone();
            remapped.set_span_identity(11);
            let mut expected = clip_loop;
            for edge in &mut expected.source_edges {
                edge.source = edge.source.with_span_identity(11);
            }
            assert_eq!(
                format!(
                    "{:?}",
                    remapped.boundary_loop(11, EdgeClass::Bridge, &sections)
                ),
                format!("{expected:?}"),
                "case {index} remapped"
            );
        }
    }
}
