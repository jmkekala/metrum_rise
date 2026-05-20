//! Footprint boundary height resolution and conflict rejection.

use super::super::band_semantics::{raised_step_band_rank, raised_step_kinds_can_contact};
use super::sources::node_footprint_boundary_vertex_source_for_edge_point;
use super::*;

impl NodeFootprintBoundaryExportSources {
    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn height_mm_at_key(
        &mut self,
        key: arrangement::NodeArrangementKey,
    ) -> Result<Option<i64>, NodeBoundaryExportError> {
        let Some(candidate) = self.height_candidate_at_boundary_vertex(key)? else {
            return Ok(None);
        };
        self.insert_boundary_vertex_source(key, candidate.height_mm, candidate.source);
        Ok(Some(candidate.height_mm))
    }

    pub(in crate::simulation::network::surface) fn reject_boundary_vertex_height_conflict(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Result<(), NodeBoundaryExportError> {
        self.height_at_boundary_vertex(key).map(|_| ())
    }

    #[cfg(test)]
    pub(super) fn height_candidate_at_boundary_vertex(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Result<Option<NodeFootprintBoundaryHeightCandidate>, NodeBoundaryExportError> {
        let exact_candidates = self
            .direct_height_candidates_at_key(key)
            .into_iter()
            .chain(self.boundary_edge_height_candidates_at_key(key))
            .collect::<Vec<_>>();
        if !exact_candidates.is_empty() {
            return self.unique_height_candidate_at_key(key, exact_candidates);
        }
        Ok(None)
    }

    fn height_at_boundary_vertex(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Result<Option<i64>, NodeBoundaryExportError> {
        let exact_candidates = self
            .direct_height_candidates_at_key(key)
            .into_iter()
            .chain(self.boundary_edge_height_candidates_at_key(key))
            .collect::<Vec<_>>();
        if exact_candidates.is_empty() {
            return Ok(None);
        }

        let mut heights = exact_candidates
            .iter()
            .map(|candidate| candidate.height_mm)
            .collect::<Vec<_>>();
        heights.sort_unstable();
        heights.dedup();
        if heights.len() == 1 {
            return Ok(Some(heights[0]));
        }

        if let Some(candidate) = raised_step_footprint_height_candidate(
            key,
            &exact_candidates,
            &heights,
            &self.explicit_vertical_step_segments,
            &self.source_edges,
        ) {
            return Ok(Some(candidate.height_mm));
        }

        let existing = exact_candidates
            .iter()
            .find(|candidate| candidate.height_mm == heights[0])
            .expect("conflicting boundary height includes existing candidate");
        let incoming = exact_candidates
            .iter()
            .find(|candidate| candidate.height_mm == heights[1])
            .expect("conflicting boundary height includes incoming candidate");
        Err(
            NodeBoundaryExportError::ConflictingFootprintBoundaryHeight {
                x_key: key.x_key(),
                z_key: key.z_key(),
                existing_y_mm: heights[0],
                incoming_y_mm: heights[1],
                existing_owner_kind: existing.source.owner_kind,
                existing_owner_index: existing.source.owner_index,
                existing_source: existing.source.source,
                incoming_owner_kind: incoming.source.owner_kind,
                incoming_owner_index: incoming.source.owner_index,
                incoming_source: incoming.source.source,
            },
        )
    }

    #[cfg(test)]
    pub(super) fn height_candidate_at_point(
        &self,
        key: arrangement::NodeArrangementKey,
        height_mm: i64,
    ) -> Result<Option<NodeFootprintBoundaryHeightCandidate>, NodeBoundaryExportError> {
        let exact_candidates = self
            .height_candidates_at_key(key)
            .into_iter()
            .collect::<Vec<NodeFootprintBoundaryHeightCandidate>>();
        if !exact_candidates.is_empty() {
            return Ok(self
                .unique_height_candidate_at_key(key, exact_candidates)?
                .filter(|candidate| candidate.height_mm == height_mm));
        }

        Ok(None)
    }

    #[cfg(test)]
    fn unique_height_candidate_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
        candidates: Vec<NodeFootprintBoundaryHeightCandidate>,
    ) -> Result<Option<NodeFootprintBoundaryHeightCandidate>, NodeBoundaryExportError> {
        if candidates.is_empty() {
            return Ok(None);
        }

        let mut heights = candidates
            .iter()
            .map(|candidate| candidate.height_mm)
            .collect::<Vec<_>>();
        heights.sort_unstable();
        heights.dedup();
        if heights.len() != 1 {
            if let Some(candidate) = raised_step_footprint_height_candidate(
                key,
                &candidates,
                &heights,
                &self.explicit_vertical_step_segments,
                &self.source_edges,
            ) {
                return Ok(Some(candidate));
            }
            let existing = candidates
                .iter()
                .find(|candidate| candidate.height_mm == heights[0])
                .expect("conflicting boundary height includes existing candidate");
            let incoming = candidates
                .iter()
                .find(|candidate| candidate.height_mm == heights[1])
                .expect("conflicting boundary height includes incoming candidate");
            return Err(
                NodeBoundaryExportError::ConflictingFootprintBoundaryHeight {
                    x_key: key.x_key(),
                    z_key: key.z_key(),
                    existing_y_mm: heights[0],
                    incoming_y_mm: heights[1],
                    existing_owner_kind: existing.source.owner_kind,
                    existing_owner_index: existing.source.owner_index,
                    existing_source: existing.source.source,
                    incoming_owner_kind: incoming.source.owner_kind,
                    incoming_owner_index: incoming.source.owner_index,
                    incoming_source: incoming.source.source,
                },
            );
        }

        let mut source = None;
        let point_key = ArrangementBoundaryPointKey {
            x_key: key.x_key(),
            z_key: key.z_key(),
            y_mm: heights[0],
        };
        for candidate in candidates {
            merge_node_footprint_boundary_point_source(point_key, &mut source, candidate.source)?;
        }
        Ok(source.map(|source| NodeFootprintBoundaryHeightCandidate {
            height_mm: heights[0],
            source,
        }))
    }

    #[cfg(test)]
    pub(super) fn insert_boundary_vertex_source(
        &mut self,
        key: arrangement::NodeArrangementKey,
        height_mm: i64,
        candidate: NodeFootprintBoundaryDirectVertex,
    ) {
        let point_key = ArrangementBoundaryPointKey {
            x_key: key.x_key(),
            z_key: key.z_key(),
            y_mm: height_mm,
        };
        self.direct_vertex_sources
            .entry(point_key)
            .and_modify(|current| {
                debug_assert!(
                    node_footprint_direct_vertices_share_source_identity(candidate, *current),
                    "test helper must not hide direct boundary source ambiguity"
                );
            })
            .or_insert(candidate);
    }

    #[cfg(test)]
    fn height_candidates_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Vec<NodeFootprintBoundaryHeightCandidate> {
        self.direct_height_candidates_at_key(key)
            .into_iter()
            .chain(self.boundary_edge_height_candidates_at_key(key))
            .collect()
    }

    fn boundary_edge_height_candidates_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> impl Iterator<Item = NodeFootprintBoundaryHeightCandidate> + '_ {
        self.source_edges.iter().filter_map(move |source_edge| {
            if !arrangement_key_lies_exactly_on_segment(
                key,
                source_edge.start_key,
                source_edge.end_key,
            ) {
                return None;
            }
            let parameter = arrangement_key_segment_parameter_xz(
                key,
                source_edge.start_key,
                source_edge.end_key,
            )?;
            let height_mm = interpolated_segment_height_mm(
                source_edge.start_point_key,
                source_edge.end_point_key,
                parameter,
            );
            Some(NodeFootprintBoundaryHeightCandidate {
                height_mm,
                source: NodeFootprintBoundaryDirectVertex {
                    source: node_footprint_boundary_vertex_source_for_edge_point(
                        source_edge,
                        ArrangementBoundaryPointKey {
                            x_key: key.x_key(),
                            z_key: key.z_key(),
                            y_mm: height_mm,
                        },
                    )?,
                    owner_kind: source_edge.owner_kind,
                    owner_index: source_edge.owner_index,
                },
            })
        })
    }

    fn direct_height_candidates_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Vec<NodeFootprintBoundaryHeightCandidate> {
        let mut candidates = Vec::new();
        for point_key in self
            .direct_vertex_source_candidates
            .keys()
            .chain(self.direct_vertex_sources.keys())
            .copied()
        {
            if point_key.x_key != key.x_key() || point_key.z_key != key.z_key() {
                continue;
            }
            for source in self.direct_vertex_candidates_at_point(point_key) {
                let candidate = NodeFootprintBoundaryHeightCandidate {
                    height_mm: point_key.y_mm,
                    source,
                };
                if !candidates.iter().any(|existing| {
                    node_footprint_height_candidates_share_source_identity(*existing, candidate)
                }) {
                    candidates.push(candidate);
                }
            }
        }
        candidates
    }
}

fn raised_step_footprint_height_candidate(
    key: arrangement::NodeArrangementKey,
    candidates: &[NodeFootprintBoundaryHeightCandidate],
    heights: &[i64],
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> Option<NodeFootprintBoundaryHeightCandidate> {
    // Raised-step corners can put a lower material edge and its raised neighbor at one footprint
    // key. Accept that only when a materialized vertical-step segment or exact terminal source-edge
    // endpoints prove the ordered owner pair at that canonical key; unrelated cross-material
    // conflicts still reject.
    let [_, _] = heights else {
        return None;
    };

    let mut raised_candidates = Vec::new();
    let mut checked_pairs = 0usize;
    for (left_index, left) in candidates.iter().copied().enumerate() {
        for right in candidates.iter().copied().skip(left_index + 1) {
            if left.height_mm == right.height_mm {
                continue;
            }
            checked_pairs += 1;
            let Some((lower, raised)) = ordered_raised_step_footprint_candidates(left, right)
            else {
                return None;
            };
            let explicit_step_authorized = explicit_vertical_step_authorizes_footprint_height_pair(
                key,
                lower.source,
                raised.source,
                explicit_vertical_step_segments,
            );
            let terminal_source_endpoint_authorized =
                terminal_source_edge_endpoints_authorize_footprint_height_pair(
                    key,
                    lower,
                    raised,
                    source_edges,
                );
            if !explicit_step_authorized && !terminal_source_endpoint_authorized {
                continue;
            }
            if !raised_candidates
                .iter()
                .any(|candidate: &NodeFootprintBoundaryHeightCandidate| *candidate == raised)
            {
                raised_candidates.push(raised);
            }
        }
    }
    if checked_pairs == 0 || raised_candidates.is_empty() {
        return None;
    }
    let mut source = None;
    for candidate in raised_candidates {
        let point_key = ArrangementBoundaryPointKey {
            x_key: key.x_key(),
            z_key: key.z_key(),
            y_mm: candidate.height_mm,
        };
        if merge_node_footprint_boundary_point_source(point_key, &mut source, candidate.source)
            .is_err()
        {
            return None;
        }
    }
    source.map(|source| NodeFootprintBoundaryHeightCandidate {
        height_mm: heights[1],
        source,
    })
}

fn node_footprint_height_candidates_share_source_identity(
    a: NodeFootprintBoundaryHeightCandidate,
    b: NodeFootprintBoundaryHeightCandidate,
) -> bool {
    a.height_mm == b.height_mm
        && node_footprint_direct_vertices_share_source_identity(a.source, b.source)
}

fn ordered_raised_step_footprint_candidates(
    left: NodeFootprintBoundaryHeightCandidate,
    right: NodeFootprintBoundaryHeightCandidate,
) -> Option<(
    NodeFootprintBoundaryHeightCandidate,
    NodeFootprintBoundaryHeightCandidate,
)> {
    if !raised_step_kinds_can_contact(left.source.owner_kind, right.source.owner_kind) {
        return None;
    }
    let left_rank = raised_step_band_rank(left.source.owner_kind)?;
    let right_rank = raised_step_band_rank(right.source.owner_kind)?;
    match left_rank.cmp(&right_rank) {
        std::cmp::Ordering::Less => Some((left, right)),
        std::cmp::Ordering::Greater => Some((right, left)),
        std::cmp::Ordering::Equal => None,
    }
}

fn explicit_vertical_step_authorizes_footprint_height_pair(
    key: arrangement::NodeArrangementKey,
    lower: NodeFootprintBoundaryDirectVertex,
    raised: NodeFootprintBoundaryDirectVertex,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> bool {
    let lower_owner = arrangement::NodeBandOwner::new(lower.owner_kind, lower.owner_index);
    let raised_owner = arrangement::NodeBandOwner::new(raised.owner_kind, raised.owner_index);
    explicit_vertical_step_segments.iter().any(|segment| {
        arrangement_key_lies_exactly_on_segment(key, segment.start(), segment.end())
            && vertical_step_segment_authorizes_owner_pair(*segment, lower_owner, raised_owner)
    })
}

fn vertical_step_segment_authorizes_owner_pair(
    segment: arrangement::NodeExplicitVerticalStepSegment,
    lower_owner: arrangement::NodeBandOwner,
    raised_owner: arrangement::NodeBandOwner,
) -> bool {
    ((segment.owner() == lower_owner && segment.opposite_owner() == raised_owner)
        || (segment.owner() == raised_owner && segment.opposite_owner() == lower_owner))
        && raised_step_kinds_can_contact(lower_owner.kind(), raised_owner.kind())
        && raised_step_band_rank(lower_owner.kind())
            .zip(raised_step_band_rank(raised_owner.kind()))
            .is_some_and(|(lower_rank, raised_rank)| lower_rank < raised_rank)
}

fn terminal_source_edge_endpoints_authorize_footprint_height_pair(
    key: arrangement::NodeArrangementKey,
    lower: NodeFootprintBoundaryHeightCandidate,
    raised: NodeFootprintBoundaryHeightCandidate,
    source_edges: &[NodeEarthworkBoundarySourceEdge],
) -> bool {
    if !raised_step_kinds_can_contact(lower.source.owner_kind, raised.source.owner_kind) {
        return false;
    }
    let Some(lower_rank) = raised_step_band_rank(lower.source.owner_kind) else {
        return false;
    };
    let Some(raised_rank) = raised_step_band_rank(raised.source.owner_kind) else {
        return false;
    };
    if lower_rank >= raised_rank {
        return false;
    }
    source_edges.iter().any(|lower_edge| {
        terminal_source_edge_endpoint_proves_candidate_at_key(lower_edge, key, lower)
            && source_edges.iter().any(|raised_edge| {
                terminal_source_edge_endpoint_proves_candidate_at_key(raised_edge, key, raised)
            })
    })
}

fn terminal_source_edge_endpoint_proves_candidate_at_key(
    source_edge: &NodeEarthworkBoundarySourceEdge,
    key: arrangement::NodeArrangementKey,
    candidate: NodeFootprintBoundaryHeightCandidate,
) -> bool {
    if source_edge.kind != RoadSurfaceVisualNodePieceKind::Terminal
        || source_edge.owner_kind != candidate.source.owner_kind
        || source_edge.owner_index != candidate.source.owner_index
    {
        return false;
    }
    terminal_source_edge_endpoint_matches_candidate(
        source_edge.start_key,
        source_edge.start_point_key.y_mm,
        source_edge.start_source,
        key,
        candidate,
    ) || terminal_source_edge_endpoint_matches_candidate(
        source_edge.end_key,
        source_edge.end_point_key.y_mm,
        source_edge.end_source,
        key,
        candidate,
    )
}

fn terminal_source_edge_endpoint_matches_candidate(
    endpoint_key: arrangement::NodeArrangementKey,
    endpoint_height_mm: i64,
    endpoint_source: NodeFootprintBoundaryDirectSource,
    key: arrangement::NodeArrangementKey,
    candidate: NodeFootprintBoundaryHeightCandidate,
) -> bool {
    if endpoint_height_mm != candidate.height_mm {
        return false;
    }
    endpoint_key == key
        && candidate.source.source == NodeFootprintBoundaryVertexSource::Direct(endpoint_source)
}
