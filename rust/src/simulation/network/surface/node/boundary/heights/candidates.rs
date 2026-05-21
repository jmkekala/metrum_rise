//! Boundary height candidate lookup and conflict reporting.

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

    pub(in crate::simulation::network::surface) fn boundary_height_mm_at_key(
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

    pub(in crate::simulation::network::surface) fn reject_boundary_vertex_height_conflict(
        &self,
        key: arrangement::NodeArrangementKey,
    ) -> Result<(), NodeBoundaryExportError> {
        self.boundary_height_mm_at_key(key).map(|_| ())
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
