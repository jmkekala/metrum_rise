//! Boundary height candidate lookup and conflict reporting.

use super::super::super::{
    ownership::NodeCarrierProvenanceOrigin, rails::NodeGeneratedContourClaimPriority,
};
#[cfg(test)]
use super::raised_steps::raised_step_footprint_height_candidate;
use super::raised_steps::raised_step_footprint_height_mm;
use super::source_conflicts::{
    node_footprint_height_candidates_share_source_identity,
    reject_same_owner_same_height_source_conflicts,
};
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

    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn boundary_height_mm_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Result<Option<i64>, NodeBoundaryExportError> {
        self.boundary_height_mm_at_key_with_context(key, false)
    }

    #[cfg(test)]
    fn boundary_height_mm_at_key_with_context(
        &self,
        key: arrangement::NodeArrangementKey,
        allow_final_context_raised_step: bool,
    ) -> Result<Option<i64>, NodeBoundaryExportError> {
        let final_edge_candidates = self
            .boundary_edge_height_candidates_at_key(key)
            .filter(|candidate| candidate.final_footprint_boundary)
            .map(|candidate| candidate.height)
            .chain(self.final_boundary_vertex_height_candidates_at_key(key))
            .collect::<Vec<_>>();
        if let Some(height_mm) = self.boundary_height_mm_from_final_candidates(
            key,
            final_edge_candidates,
            allow_final_context_raised_step,
        )? {
            return Ok(Some(height_mm));
        }
        let exact_candidates = self
            .direct_height_candidates_at_key(key)
            .into_iter()
            .chain(
                self.boundary_edge_height_candidates_at_key(key)
                    .map(|candidate| candidate.height),
            )
            .collect::<Vec<_>>();
        self.boundary_height_mm_from_candidates(key, exact_candidates)
    }

    pub(in crate::simulation::network::surface) fn boundary_height_mm_at_contour_key(
        &self,
        key: arrangement::NodeArrangementKey,
        previous_key: arrangement::NodeArrangementKey,
        next_key: arrangement::NodeArrangementKey,
    ) -> Result<Option<i64>, NodeBoundaryExportError> {
        let direct_final_context_candidates = self.prefer_strongest_direct_final_boundary_context(
            key,
            previous_key,
            next_key,
            self.direct_height_candidates_at_key(key)
                .into_iter()
                .filter(|candidate| {
                    self.direct_candidate_has_final_boundary_context(
                        candidate,
                        key,
                        previous_key,
                        next_key,
                    )
                })
                .collect::<Vec<_>>(),
        );
        if let Some(height_mm) = self.boundary_height_mm_from_final_candidates(
            key,
            direct_final_context_candidates,
            true,
        )? {
            return Ok(Some(height_mm));
        }
        let final_context_candidates = self
            .final_boundary_context_height_candidates_at_key(key, previous_key, next_key)
            .into_iter()
            .chain(
                self.boundary_edge_height_candidates_at_key(key)
                    .filter(|candidate| candidate.final_footprint_boundary)
                    .filter(|candidate| {
                        self.final_source_edge_supports_boundary_context(
                            candidate,
                            previous_key,
                            next_key,
                        )
                    })
                    .map(|candidate| candidate.height),
            )
            .collect::<Vec<_>>();
        let final_context_candidates = self.prefer_strongest_final_boundary_context(
            key,
            previous_key,
            next_key,
            final_context_candidates,
        );
        if let Some(height_mm) =
            self.boundary_height_mm_from_final_candidates(key, final_context_candidates, true)?
        {
            return Ok(Some(height_mm));
        }
        let exact_candidates = self
            .direct_height_candidates_at_key(key)
            .into_iter()
            .chain(
                self.boundary_edge_height_candidates_at_key(key)
                    .filter(|candidate| candidate.final_footprint_boundary)
                    .map(|candidate| candidate.height),
            )
            .collect::<Vec<_>>();
        self.boundary_height_mm_from_candidates(key, exact_candidates)
    }

    fn boundary_height_mm_from_final_candidates(
        &self,
        key: arrangement::NodeArrangementKey,
        final_candidates: Vec<NodeFootprintBoundaryHeightCandidate>,
        allow_final_context_raised_step: bool,
    ) -> Result<Option<i64>, NodeBoundaryExportError> {
        if final_candidates.is_empty() {
            return Ok(None);
        }
        let mut heights = final_candidates
            .iter()
            .map(|candidate| candidate.height_mm)
            .collect::<Vec<_>>();
        heights.sort_unstable();
        heights.dedup();
        if let Some(height_mm) = raised_step_footprint_height_mm(
            key,
            &final_candidates,
            &heights,
            &self.explicit_vertical_step_segments,
            &self.source_edges,
        ) {
            return Ok(Some(height_mm));
        }
        if let Some(candidate) = self.source_intersection_final_endpoint_rank_gap_candidate(
            key,
            &final_candidates,
            &heights,
        ) {
            return Ok(Some(candidate.height_mm));
        }
        if allow_final_context_raised_step
            && let Some(height_mm) =
                final_context_raised_step_footprint_height_mm(&final_candidates, &heights)
        {
            return Ok(Some(height_mm));
        }
        self.boundary_height_mm_from_candidates(key, final_candidates)
    }

    fn boundary_height_mm_from_candidates(
        &self,
        key: arrangement::NodeArrangementKey,
        exact_candidates: Vec<NodeFootprintBoundaryHeightCandidate>,
    ) -> Result<Option<i64>, NodeBoundaryExportError> {
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
            reject_same_owner_same_height_source_conflicts(key, &exact_candidates)?;
            return Ok(Some(heights[0]));
        }

        if let Some(height_mm) = raised_step_footprint_height_mm(
            key,
            &exact_candidates,
            &heights,
            &self.explicit_vertical_step_segments,
            &self.source_edges,
        ) {
            return Ok(Some(height_mm));
        }
        if let Some(candidate) = self.source_intersection_final_endpoint_rank_gap_candidate(
            key,
            &exact_candidates,
            &heights,
        ) {
            return Ok(Some(candidate.height_mm));
        }
        if let Some(candidate) =
            self.canonical_distinct_source_provenance_candidate(&exact_candidates, &heights)
        {
            return Ok(Some(candidate.height_mm));
        }
        if let Some(candidate) =
            self.canonical_endpoint_dust_outer_boundary_candidate(key, &exact_candidates, &heights)
        {
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
    pub(super) fn height_candidate_at_boundary_vertex(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Result<Option<NodeFootprintBoundaryHeightCandidate>, NodeBoundaryExportError> {
        let exact_candidates = self
            .direct_height_candidates_at_key(key)
            .into_iter()
            .chain(
                self.boundary_edge_height_candidates_at_key(key)
                    .map(|candidate| candidate.height),
            )
            .collect::<Vec<_>>();
        if !exact_candidates.is_empty() {
            return self.unique_height_candidate_at_key(key, exact_candidates);
        }
        Ok(None)
    }

    #[cfg(test)]
    pub(in crate::simulation::network::surface::node::boundary) fn height_candidate_at_point(
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
            if let Some(candidate) = self.source_intersection_final_endpoint_rank_gap_candidate(
                key,
                &candidates,
                &heights,
            ) {
                return Ok(Some(candidate));
            }
            if let Some(candidate) =
                self.canonical_distinct_source_provenance_candidate(&candidates, &heights)
            {
                return Ok(Some(candidate));
            }
            if let Some(candidate) =
                self.canonical_endpoint_dust_outer_boundary_candidate(key, &candidates, &heights)
            {
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
                if !node_footprint_direct_vertices_share_source_identity(candidate, *current) {
                    debug_assert!(
                        node_footprint_direct_vertices_share_boundary_point_authority(
                            point_key, candidate, *current
                        ),
                        "test helper must not hide direct boundary source ambiguity"
                    );
                    *current = candidate;
                }
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
            .chain(
                self.boundary_edge_height_candidates_at_key(key)
                    .map(|candidate| candidate.height),
            )
            .collect()
    }

    fn boundary_edge_height_candidates_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> impl Iterator<Item = NodeFootprintBoundaryEdgeHeightCandidate> + '_ {
        self.source_edges.iter().filter_map(move |source_edge| {
            let key_point = ArrangementBoundaryPointKey {
                x_key: key.x_key(),
                z_key: key.z_key(),
                y_mm: 0,
            };
            let parameter = boundary_segment_parameter_xz_on_segment(
                key_point,
                source_edge.start_point_key,
                source_edge.end_point_key,
            )?;
            let height_mm = interpolated_segment_height_mm(
                source_edge.start_point_key,
                source_edge.end_point_key,
                parameter,
            );
            let point_key = ArrangementBoundaryPointKey {
                x_key: key.x_key(),
                z_key: key.z_key(),
                y_mm: height_mm,
            };
            let source =
                node_footprint_boundary_vertex_source_for_edge_point(source_edge, point_key)?;
            Some(NodeFootprintBoundaryEdgeHeightCandidate {
                height: NodeFootprintBoundaryHeightCandidate {
                    height_mm,
                    source: NodeFootprintBoundaryDirectVertex {
                        source,
                        owner_kind: source_edge.owner_kind,
                        owner_index: source_edge.owner_index,
                    },
                },
                final_footprint_boundary: source_edge.final_footprint_boundary,
            })
        })
    }

    fn final_boundary_context_height_candidates_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
        _previous_key: arrangement::NodeArrangementKey,
        _next_key: arrangement::NodeArrangementKey,
    ) -> Vec<NodeFootprintBoundaryHeightCandidate> {
        let mut candidates = Vec::new();
        candidates.extend(self.final_boundary_vertex_height_candidates_at_key(key));
        candidates.extend(self.final_endpoint_dust_height_candidates_at_key(key));
        self.push_final_height_edge_candidates_at_key(&mut candidates, key, true);
        candidates
    }

    fn push_final_height_edge_candidates_at_key(
        &self,
        candidates: &mut Vec<NodeFootprintBoundaryHeightCandidate>,
        key: arrangement::NodeArrangementKey,
        require_exact_support: bool,
    ) {
        for source_edge in &self.final_height_edges {
            let supports_key = if require_exact_support {
                final_height_edge_supports_key_exactly(source_edge, key)
            } else {
                final_height_edge_supports_key(source_edge, key)
            };
            if !supports_key {
                continue;
            }
            let height_mm = final_height_edge_height_mm_at_key(source_edge, key);
            let point_key = ArrangementBoundaryPointKey {
                x_key: key.x_key(),
                z_key: key.z_key(),
                y_mm: height_mm,
            };
            let candidate = NodeFootprintBoundaryHeightCandidate {
                height_mm,
                source: NodeFootprintBoundaryDirectVertex {
                    source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                        x_key: point_key.x_key,
                        z_key: point_key.z_key,
                        y_mm: point_key.y_mm,
                    },
                    owner_kind: source_edge.owner_kind,
                    owner_index: source_edge.owner_index,
                },
            };
            if !candidates.iter().any(|existing| {
                node_footprint_height_candidates_share_source_identity(*existing, candidate)
            }) {
                candidates.push(candidate);
            }
        }
    }

    fn final_source_edge_supports_boundary_context(
        &self,
        candidate: &NodeFootprintBoundaryEdgeHeightCandidate,
        previous_key: arrangement::NodeArrangementKey,
        next_key: arrangement::NodeArrangementKey,
    ) -> bool {
        self.source_edges.iter().any(|source_edge| {
            source_edge.final_footprint_boundary
                && source_edge.owner_kind == candidate.height.source.owner_kind
                && source_edge.owner_index == candidate.height.source.owner_index
                && (final_source_edge_supports_key(source_edge, previous_key)
                    || final_source_edge_supports_key(source_edge, next_key))
        })
    }

    fn direct_candidate_has_final_boundary_context(
        &self,
        candidate: &NodeFootprintBoundaryHeightCandidate,
        key: arrangement::NodeArrangementKey,
        previous_key: arrangement::NodeArrangementKey,
        next_key: arrangement::NodeArrangementKey,
    ) -> bool {
        if !matches!(
            candidate.source.source,
            NodeFootprintBoundaryVertexSource::Direct(_)
        ) {
            return false;
        }
        self.final_height_edges.iter().any(|source_edge| {
            source_edge.owner_kind == candidate.source.owner_kind
                && source_edge.owner_index == candidate.source.owner_index
                && final_height_edge_supports_key_exactly(source_edge, key)
                && (final_height_edge_supports_key(source_edge, previous_key)
                    || final_height_edge_supports_key(source_edge, next_key))
        })
    }

    fn prefer_strongest_direct_final_boundary_context(
        &self,
        key: arrangement::NodeArrangementKey,
        previous_key: arrangement::NodeArrangementKey,
        next_key: arrangement::NodeArrangementKey,
        candidates: Vec<NodeFootprintBoundaryHeightCandidate>,
    ) -> Vec<NodeFootprintBoundaryHeightCandidate> {
        if candidates.len() < 2 {
            return candidates;
        }
        let scored = candidates
            .into_iter()
            .map(|candidate| {
                (
                    self.direct_candidate_final_boundary_context_score(
                        candidate,
                        key,
                        previous_key,
                        next_key,
                    ),
                    candidate,
                )
            })
            .collect::<Vec<_>>();
        let Some(max_score) = scored.iter().map(|(score, _)| *score).max() else {
            return Vec::new();
        };
        if max_score == 0
            || scored
                .iter()
                .filter(|(score, _)| *score == max_score)
                .count()
                != 1
        {
            return scored.into_iter().map(|(_, candidate)| candidate).collect();
        }
        scored
            .into_iter()
            .filter_map(|(score, candidate)| (score == max_score).then_some(candidate))
            .collect()
    }

    fn prefer_strongest_final_boundary_context(
        &self,
        key: arrangement::NodeArrangementKey,
        previous_key: arrangement::NodeArrangementKey,
        next_key: arrangement::NodeArrangementKey,
        candidates: Vec<NodeFootprintBoundaryHeightCandidate>,
    ) -> Vec<NodeFootprintBoundaryHeightCandidate> {
        if candidates.len() < 2 {
            return candidates;
        }
        let scored = candidates
            .into_iter()
            .map(|candidate| {
                (
                    self.final_boundary_context_rank(candidate, key, previous_key, next_key),
                    candidate,
                )
            })
            .collect::<Vec<_>>();
        let Some(max_rank) = scored.iter().map(|(rank, _)| *rank).max() else {
            return Vec::new();
        };
        if max_rank == (0, 0, 0) || scored.iter().filter(|(rank, _)| *rank == max_rank).count() != 1
        {
            return scored.into_iter().map(|(_, candidate)| candidate).collect();
        }
        scored
            .into_iter()
            .filter_map(|(rank, candidate)| (rank == max_rank).then_some(candidate))
            .collect()
    }

    fn final_boundary_context_rank(
        &self,
        candidate: NodeFootprintBoundaryHeightCandidate,
        key: arrangement::NodeArrangementKey,
        previous_key: arrangement::NodeArrangementKey,
        next_key: arrangement::NodeArrangementKey,
    ) -> (u8, u8, u8) {
        let supports_previous = self.final_height_edges.iter().any(|source_edge| {
            source_edge.owner_kind == candidate.source.owner_kind
                && source_edge.owner_index == candidate.source.owner_index
                && final_height_edge_supports_key_exactly(source_edge, key)
                && final_height_edge_supports_key(source_edge, previous_key)
        });
        let supports_next = self.final_height_edges.iter().any(|source_edge| {
            source_edge.owner_kind == candidate.source.owner_kind
                && source_edge.owner_index == candidate.source.owner_index
                && final_height_edge_supports_key_exactly(source_edge, key)
                && final_height_edge_supports_key(source_edge, next_key)
        });
        let adjacent_context = u8::from(supports_previous) + u8::from(supports_next);
        let exact_boundary_corner_degree = self
            .final_height_edges
            .iter()
            .filter(|source_edge| {
                source_edge.owner_kind == candidate.source.owner_kind
                    && source_edge.owner_index == candidate.source.owner_index
                    && final_height_edge_supports_key_exactly(source_edge, key)
            })
            .count()
            .min(u8::MAX as usize) as u8;
        let endpoint_dust_support_count = self
            .final_height_edges
            .iter()
            .filter(|source_edge| {
                source_edge.owner_kind == candidate.source.owner_kind
                    && source_edge.owner_index == candidate.source.owner_index
                    && final_height_edge_endpoint_dust_supports_candidate(
                        source_edge,
                        key,
                        candidate.height_mm,
                    )
            })
            .count()
            .min(u8::MAX as usize) as u8;
        (
            adjacent_context,
            exact_boundary_corner_degree,
            endpoint_dust_support_count,
        )
    }

    fn direct_candidate_final_boundary_context_score(
        &self,
        candidate: NodeFootprintBoundaryHeightCandidate,
        key: arrangement::NodeArrangementKey,
        previous_key: arrangement::NodeArrangementKey,
        next_key: arrangement::NodeArrangementKey,
    ) -> u8 {
        if !matches!(
            candidate.source.source,
            NodeFootprintBoundaryVertexSource::Direct(_)
        ) {
            return 0;
        }
        let supports_previous = self.final_height_edges.iter().any(|source_edge| {
            source_edge.owner_kind == candidate.source.owner_kind
                && source_edge.owner_index == candidate.source.owner_index
                && final_height_edge_supports_key_exactly(source_edge, key)
                && final_height_edge_supports_key(source_edge, previous_key)
        });
        let supports_next = self.final_height_edges.iter().any(|source_edge| {
            source_edge.owner_kind == candidate.source.owner_kind
                && source_edge.owner_index == candidate.source.owner_index
                && final_height_edge_supports_key_exactly(source_edge, key)
                && final_height_edge_supports_key(source_edge, next_key)
        });
        u8::from(supports_previous) + u8::from(supports_next)
    }

    fn canonical_distinct_source_provenance_candidate(
        &self,
        candidates: &[NodeFootprintBoundaryHeightCandidate],
        heights: &[i64],
    ) -> Option<NodeFootprintBoundaryHeightCandidate> {
        if heights.len() < 2 {
            return None;
        }
        let owner_kind = candidates.first()?.source.owner_kind;
        if candidates
            .iter()
            .any(|candidate| candidate.source.owner_kind != owner_kind)
        {
            return None;
        }
        let mut owner_indices = candidates
            .iter()
            .map(|candidate| candidate.source.owner_index)
            .collect::<Vec<_>>();
        owner_indices.sort_unstable();
        owner_indices.dedup();
        if owner_indices.len() < 2 {
            return None;
        }
        let mut candidates_by_provenance = BTreeMap::<
            height::NodeHeightCarrierProvenanceKey,
            NodeFootprintBoundaryHeightCandidate,
        >::new();
        for candidate in candidates.iter().copied() {
            let provenance = self.boundary_vertex_source_provenance(candidate.source.source)?;
            if let Some(existing) = candidates_by_provenance.insert(provenance, candidate)
                && existing.height_mm != candidate.height_mm
            {
                return None;
            }
        }
        if candidates_by_provenance.len() < 2 {
            return None;
        }
        candidates_by_provenance.into_values().next()
    }

    fn source_intersection_final_endpoint_rank_gap_candidate(
        &self,
        key: arrangement::NodeArrangementKey,
        candidates: &[NodeFootprintBoundaryHeightCandidate],
        heights: &[i64],
    ) -> Option<NodeFootprintBoundaryHeightCandidate> {
        let [lower_height_mm, raised_height_mm] = heights else {
            return None;
        };
        let mut ranked_candidates = Vec::new();
        for candidate in candidates.iter().copied() {
            if !self.candidate_has_exact_final_boundary_endpoint_support(candidate, key) {
                return None;
            }
            let provenance = self.boundary_vertex_source_provenance(candidate.source.source)?;
            if provenance.owner.kind() != candidate.source.owner_kind
                || provenance.owner.owner_index() != candidate.source.owner_index
                || provenance.source_kind != candidate.source.owner_kind
                || provenance.claim_priority != NodeGeneratedContourClaimPriority::MouthBand
                || !matches!(
                    provenance.origin,
                    NodeCarrierProvenanceOrigin::SourceIntersection { peer_count } if peer_count > 0
                )
            {
                return None;
            }
            ranked_candidates.push((
                raised_step_band_rank(candidate.source.owner_kind)?,
                candidate,
            ));
        }
        if ranked_candidates.len() < 2 {
            return None;
        }
        let min_rank = ranked_candidates.iter().map(|(rank, _)| *rank).min()?;
        let max_rank = ranked_candidates.iter().map(|(rank, _)| *rank).max()?;
        if max_rank <= min_rank + 1 {
            return None;
        }
        if ranked_candidates.iter().any(|(rank, candidate)| {
            (candidate.height_mm == *lower_height_mm && *rank != min_rank)
                || (candidate.height_mm == *raised_height_mm && *rank != max_rank)
                || (candidate.height_mm != *lower_height_mm
                    && candidate.height_mm != *raised_height_mm)
        }) {
            return None;
        }
        let mut raised_candidates = ranked_candidates
            .into_iter()
            .filter_map(|(_, candidate)| {
                (candidate.height_mm == *raised_height_mm).then_some(candidate)
            })
            .collect::<Vec<_>>();
        raised_candidates.sort_by(|a, b| a.source.source.cmp(&b.source.source));
        raised_candidates
            .dedup_by(|a, b| node_footprint_height_candidates_share_source_identity(*a, *b));
        let [raised_candidate] = raised_candidates.as_slice() else {
            return None;
        };
        Some(*raised_candidate)
    }

    fn boundary_vertex_source_provenance(
        &self,
        source: NodeFootprintBoundaryVertexSource,
    ) -> Option<height::NodeHeightCarrierProvenanceKey> {
        match source {
            NodeFootprintBoundaryVertexSource::Direct(source) => self
                .grade_authority_source_provenance
                .get(source.grade_authority_index)
                .copied()
                .flatten(),
            NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start,
                owning_segment_end,
                ..
            } => {
                let start = self
                    .grade_authority_source_provenance
                    .get(owning_segment_start.grade_authority_index)
                    .copied()
                    .flatten()?;
                let end = self
                    .grade_authority_source_provenance
                    .get(owning_segment_end.grade_authority_index)
                    .copied()
                    .flatten()?;
                (start == end).then_some(start)
            }
            NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. } => None,
        }
    }

    fn canonical_endpoint_dust_outer_boundary_candidate(
        &self,
        key: arrangement::NodeArrangementKey,
        candidates: &[NodeFootprintBoundaryHeightCandidate],
        heights: &[i64],
    ) -> Option<NodeFootprintBoundaryHeightCandidate> {
        if heights.len() < 2
            || candidates
                .iter()
                .any(|candidate| self.candidate_has_exact_final_boundary_support(*candidate, key))
        {
            return None;
        }
        let mut ranked_candidates = Vec::new();
        for candidate in candidates.iter().copied() {
            if !self.candidate_has_endpoint_dust_final_boundary_support(candidate, key) {
                return None;
            }
            ranked_candidates.push((
                raised_step_band_rank(candidate.source.owner_kind)?,
                candidate,
            ));
        }
        let max_rank = ranked_candidates.iter().map(|(rank, _)| *rank).max()?;
        if ranked_candidates
            .iter()
            .filter(|(rank, _)| *rank == max_rank)
            .count()
            != 1
        {
            return None;
        }
        ranked_candidates
            .into_iter()
            .find_map(|(rank, candidate)| (rank == max_rank).then_some(candidate))
    }

    fn candidate_has_exact_final_boundary_support(
        &self,
        candidate: NodeFootprintBoundaryHeightCandidate,
        key: arrangement::NodeArrangementKey,
    ) -> bool {
        self.final_height_edges.iter().any(|source_edge| {
            source_edge.owner_kind == candidate.source.owner_kind
                && source_edge.owner_index == candidate.source.owner_index
                && final_height_edge_supports_key_exactly(source_edge, key)
        })
    }

    fn candidate_has_exact_final_boundary_endpoint_support(
        &self,
        candidate: NodeFootprintBoundaryHeightCandidate,
        key: arrangement::NodeArrangementKey,
    ) -> bool {
        self.final_height_edges.iter().any(|source_edge| {
            source_edge.owner_kind == candidate.source.owner_kind
                && source_edge.owner_index == candidate.source.owner_index
                && ((source_edge.start_point_key.xz_key() == key
                    && source_edge.start_point_key.y_mm == candidate.height_mm)
                    || (source_edge.end_point_key.xz_key() == key
                        && source_edge.end_point_key.y_mm == candidate.height_mm))
        })
    }

    fn candidate_has_endpoint_dust_final_boundary_support(
        &self,
        candidate: NodeFootprintBoundaryHeightCandidate,
        key: arrangement::NodeArrangementKey,
    ) -> bool {
        self.final_height_edges.iter().any(|source_edge| {
            source_edge.owner_kind == candidate.source.owner_kind
                && source_edge.owner_index == candidate.source.owner_index
                && final_height_edge_endpoint_dust_supports_candidate(
                    source_edge,
                    key,
                    candidate.height_mm,
                )
        })
    }

    fn final_boundary_vertex_height_candidates_at_key(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Vec<NodeFootprintBoundaryHeightCandidate> {
        let mut candidates = Vec::new();
        for (point_key, sources) in &self.final_vertex_sources {
            if point_key.x_key != key.x_key() || point_key.z_key != key.z_key() {
                continue;
            }
            for source in sources {
                let candidate = NodeFootprintBoundaryHeightCandidate {
                    height_mm: point_key.y_mm,
                    source: *source,
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

fn final_height_edge_supports_key(
    source_edge: &NodeFinalFootprintBoundaryHeightEdge,
    key: arrangement::NodeArrangementKey,
) -> bool {
    let point_key = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 0,
    };
    final_boundary_segment_parameter_xz_on_segment(
        point_key,
        source_edge.start_point_key,
        source_edge.end_point_key,
    )
    .is_some()
}

fn final_height_edge_supports_key_exactly(
    source_edge: &NodeFinalFootprintBoundaryHeightEdge,
    key: arrangement::NodeArrangementKey,
) -> bool {
    let point_key = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 0,
    };
    arrangement_key_lies_exactly_on_segment(
        point_key.xz_key(),
        source_edge.start_point_key.xz_key(),
        source_edge.end_point_key.xz_key(),
    )
}

fn final_height_edge_height_mm_at_key(
    source_edge: &NodeFinalFootprintBoundaryHeightEdge,
    key: arrangement::NodeArrangementKey,
) -> i64 {
    let point_key = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 0,
    };
    let parameter = final_boundary_segment_parameter_xz_on_segment(
        point_key,
        source_edge.start_point_key,
        source_edge.end_point_key,
    )
    .expect("final height edge supports queried key");
    interpolated_segment_height_mm(
        source_edge.start_point_key,
        source_edge.end_point_key,
        parameter,
    )
}

fn final_height_edge_endpoint_dust_supports_candidate(
    source_edge: &NodeFinalFootprintBoundaryHeightEdge,
    key: arrangement::NodeArrangementKey,
    height_mm: i64,
) -> bool {
    let point = arrangement_key(key);
    let start = arrangement_key(source_edge.start_point_key.xz_key());
    let end = arrangement_key(source_edge.end_point_key.xz_key());
    let near_start = key_distance_squared(point, start)
        <= i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS)
            * i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS);
    let near_end = key_distance_squared(point, end)
        <= i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS)
            * i128::from(BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS);
    (near_start && source_edge.start_point_key.y_mm == height_mm)
        || (near_end && source_edge.end_point_key.y_mm == height_mm)
}

fn final_source_edge_supports_key(
    source_edge: &NodeEarthworkBoundarySourceEdge,
    key: arrangement::NodeArrangementKey,
) -> bool {
    let point_key = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 0,
    };
    final_boundary_segment_parameter_xz_on_segment(
        point_key,
        source_edge.start_point_key,
        source_edge.end_point_key,
    )
    .is_some()
}

fn final_context_raised_step_footprint_height_mm(
    candidates: &[NodeFootprintBoundaryHeightCandidate],
    heights: &[i64],
) -> Option<i64> {
    let [lower_height_mm, raised_height_mm] = heights else {
        return None;
    };
    let lower_candidates = candidates
        .iter()
        .filter(|candidate| candidate.height_mm == *lower_height_mm)
        .collect::<Vec<_>>();
    let raised_candidates = candidates
        .iter()
        .filter(|candidate| candidate.height_mm == *raised_height_mm)
        .collect::<Vec<_>>();
    if lower_candidates.is_empty() || raised_candidates.is_empty() {
        return None;
    }

    for lower in &lower_candidates {
        let lower_rank = raised_step_band_rank(lower.source.owner_kind)?;
        for raised in &raised_candidates {
            if !raised_step_kinds_can_contact(lower.source.owner_kind, raised.source.owner_kind) {
                return None;
            }
            let raised_rank = raised_step_band_rank(raised.source.owner_kind)?;
            if lower_rank >= raised_rank {
                return None;
            }
        }
    }
    Some(*raised_height_mm)
}
