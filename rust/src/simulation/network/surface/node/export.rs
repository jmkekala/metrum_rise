// SPDX-License-Identifier: GPL-2.0-only

//! Node surface export from canonical arrangement output.

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

mod assembly;
mod footprint_loops;
mod outer_boundary;
mod raised_step_support;
mod terrain_clip_loops;
mod top_regions;

type HeightSplitConflict = (
    arrangement::NodeArrangementKey,
    i64,
    arrangement::NodeBandOwner,
    i64,
    arrangement::NodeBandOwner,
);

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct HeightSplitVertexReuseKey {
    height_mm: i64,
    owners: Box<[arrangement::NodeBandOwner]>,
    grade_authority: height::NodeGradeVertexAuthority,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct HeightSplitCohortReuseKey {
    key: arrangement::NodeArrangementKey,
    vertices: Box<[HeightSplitVertexReuseKey]>,
}

#[derive(Clone, Debug, Default)]
struct NodeHeightSplitIncrementalCache {
    conflicts: BTreeMap<HeightSplitCohortReuseKey, Arc<[HeightSplitConflict]>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct NodeHeightSplitReuseStats {
    previous_hits: usize,
    misses: usize,
}

/// Immutable semantic products reusable by a later node export generation.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeExportIncrementalCache {
    explicit_steps: arrangement::NodeFinalExplicitStepTopologyCache,
    height_splits: NodeHeightSplitIncrementalCache,
    raised_steps: raised_step_support::NodeRaisedStepIncrementalCache,
}

/// Previous-generation semantic reuse observed during one node export.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct NodeExportReuseStats {
    /// Final explicit-step topologies reused from the previous generation.
    pub(crate) explicit_step_previous_hits: usize,
    /// Final explicit-step topologies rebuilt for the current generation.
    pub(crate) explicit_step_misses: usize,
    /// Positive unchanged final-boundary step pairs retained from the previous generation.
    pub(crate) explicit_step_pair_previous_hits: usize,
    /// Final-boundary step pairs evaluated because a contributor changed.
    pub(crate) explicit_step_pair_misses: usize,
    /// Height-split cohorts reused from the previous generation.
    pub(crate) height_split_previous_hits: usize,
    /// Height-split cohorts evaluated from current vertex contributors.
    pub(crate) height_split_misses: usize,
    /// Top-edge contributors served by the current or previous exact cache.
    pub(crate) top_edge_cache_hits: usize,
    /// Top-edge contributors promoted from the previous generation.
    pub(crate) top_edge_previous_hits: usize,
    /// Top-edge contributors extracted from current owned polygons.
    pub(crate) top_edge_cache_misses: usize,
    /// Raised-step span requests served by the current or previous exact cache.
    pub(crate) raised_step_cache_hits: usize,
    /// Raised-step span products promoted from the previous generation.
    pub(crate) raised_step_previous_hits: usize,
    /// Raised-step span products built from current local support.
    pub(crate) raised_step_cache_misses: usize,
}

impl NodeExportReuseStats {
    /// Counts all semantic products promoted from the immutable previous generation.
    pub(crate) fn previous_hits(self) -> usize {
        self.explicit_step_previous_hits
            + self.explicit_step_pair_previous_hits
            + self.height_split_previous_hits
            + self.top_edge_previous_hits
            + self.raised_step_previous_hits
    }

    /// Counts current semantic products that could not be served by an exact previous match.
    #[cfg(test)]
    pub(crate) fn misses(self) -> usize {
        self.explicit_step_misses
            + self.explicit_step_pair_misses
            + self.height_split_misses
            + self.top_edge_cache_misses
            + self.raised_step_cache_misses
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::simulation::network::surface) struct NodeExportProfile {
    pub(in crate::simulation::network::surface) total_ms: f64,
    pub(in crate::simulation::network::surface) explicit_step_topology_ms: f64,
    pub(in crate::simulation::network::surface) height_split_validation_ms: f64,
    pub(in crate::simulation::network::surface) authority_ms: f64,
    pub(in crate::simulation::network::surface) face_export_ms: f64,
    pub(in crate::simulation::network::surface) boundary_sources_ms: f64,
    pub(in crate::simulation::network::surface) raised_step_faces_ms: f64,
    pub(in crate::simulation::network::surface) material_partition_ms: f64,
    pub(in crate::simulation::network::surface) footprint_boundary_ms: f64,
    pub(in crate::simulation::network::surface) earthwork_boundary_ms: f64,
    pub(in crate::simulation::network::surface) outer_boundary_ms: f64,
    pub(in crate::simulation::network::surface) terrain_clip_ms: f64,
    pub(in crate::simulation::network::surface) sorting_ms: f64,
    pub(in crate::simulation::network::surface) arrangement_faces: usize,
    pub(in crate::simulation::network::surface) owned_regions: usize,
    pub(in crate::simulation::network::surface) footprint_loops: usize,
    pub(in crate::simulation::network::surface) earthwork_segments: usize,
    pub(in crate::simulation::network::surface) terrain_clip_loops: usize,
    pub(in crate::simulation::network::surface) raised_step_faces: usize,
}

fn elapsed_profile_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

impl RoadSurfaceSystem {
    #[cfg(test)]
    pub(in crate::simulation::network::surface) fn node_surface_regions_from_arrangement(
        arrangement: &NodeArrangement,
        ownership_footprint_shapes: &super::NodeOverlayShapes,
    ) -> Result<super::NodeSurfaceRegionResult, NodeBoundaryExportError> {
        Self::node_surface_regions_from_arrangement_with_profile(
            arrangement,
            ownership_footprint_shapes,
            false,
        )
        .map(|(regions, _)| regions)
    }

    pub(in crate::simulation::network::surface) fn node_surface_regions_from_arrangement_with_profile(
        arrangement: &NodeArrangement,
        ownership_footprint_shapes: &super::NodeOverlayShapes,
        profile_enabled: bool,
    ) -> Result<(super::NodeSurfaceRegionResult, NodeExportProfile), NodeBoundaryExportError> {
        let explicit_vertical_step_segments = arrangement.explicit_vertical_step_segments();
        Self::node_surface_regions_from_arrangement_with_profile_and_incremental_reuse(
            arrangement,
            ownership_footprint_shapes,
            &explicit_vertical_step_segments,
            profile_enabled,
            None,
        )
        .map(|(regions, profile, _, _)| (regions, profile))
    }

    /// Exports final node regions while promoting exact semantic products from an immutable prior
    /// generation and rebinding every positional source index to the current generation.
    pub(in crate::simulation::network::surface) fn node_surface_regions_from_arrangement_with_profile_and_incremental_reuse(
        arrangement: &NodeArrangement,
        ownership_footprint_shapes: &super::NodeOverlayShapes,
        base_explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
        profile_enabled: bool,
        previous: Option<&NodeExportIncrementalCache>,
    ) -> Result<
        (
            super::NodeSurfaceRegionResult,
            NodeExportProfile,
            NodeExportIncrementalCache,
            NodeExportReuseStats,
        ),
        NodeBoundaryExportError,
    > {
        Self::node_surface_regions_from_arrangement_with_profile_and_incremental_reuse_for_identity(
            arrangement.node_id(),
            arrangement.piece_kind(),
            arrangement,
            ownership_footprint_shapes,
            base_explicit_vertical_step_segments,
            profile_enabled,
            previous,
        )
    }

    pub(in crate::simulation::network::surface::node) fn node_surface_regions_from_arrangement_with_profile_and_incremental_reuse_for_identity(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        arrangement: &NodeArrangement,
        ownership_footprint_shapes: &super::NodeOverlayShapes,
        base_explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
        profile_enabled: bool,
        previous: Option<&NodeExportIncrementalCache>,
    ) -> Result<
        (
            super::NodeSurfaceRegionResult,
            NodeExportProfile,
            NodeExportIncrementalCache,
            NodeExportReuseStats,
        ),
        NodeBoundaryExportError,
    > {
        let total_start = profile_enabled.then(Instant::now);
        let mut profile = NodeExportProfile {
            arrangement_faces: arrangement.faces().len(),
            ..NodeExportProfile::default()
        };
        let explicit_step_topology_start = profile_enabled.then(Instant::now);
        let (explicit_vertical_step_segments, explicit_steps, explicit_step_stats) = arrangement
            .final_explicit_vertical_step_segments_with_reuse(
                base_explicit_vertical_step_segments,
                previous.map(|previous| &previous.explicit_steps),
            );
        profile.explicit_step_topology_ms = elapsed_profile_ms(explicit_step_topology_start);
        let height_split_start = profile_enabled.then(Instant::now);
        let (height_splits, height_split_stats) = reject_unauthorized_arrangement_height_splits(
            arrangement,
            &explicit_vertical_step_segments,
            previous.map(|previous| &previous.height_splits),
        )?;
        profile.height_split_validation_ms = elapsed_profile_ms(height_split_start);
        let authority_start = profile_enabled.then(Instant::now);
        let mut node_grade_authorities = arrangement
            .vertices()
            .iter()
            .map(|vertex| vertex.grade_authority())
            .collect::<Vec<_>>();
        node_grade_authorities.sort();
        node_grade_authorities.dedup();
        let authority_indices = node_grade_authorities
            .iter()
            .enumerate()
            .map(|(index, authority)| (*authority, index))
            .collect::<BTreeMap<_, _>>();
        let top_height_context = NodeExportTopHeightContext::from_arrangement(
            arrangement,
            &explicit_vertical_step_segments,
        );
        profile.authority_ms = elapsed_profile_ms(authority_start);

        let mut owned_region_exports = Vec::new();

        let face_export_start = profile_enabled.then(Instant::now);
        for face in arrangement.faces() {
            let owner = face.owner();
            let Some((polygon, source)) = Self::visual_polygon_from_arrangement_face(
                arrangement,
                face,
                &authority_indices,
                &top_height_context,
            )?
            else {
                continue;
            };
            owned_region_exports.push((
                NodeOwnedRegion {
                    kind: owner.kind(),
                    owner_index: owner.owner_index(),
                    polygon,
                },
                source,
            ));
        }
        profile.face_export_ms = elapsed_profile_ms(face_export_start);
        let (mut owned_regions, mut node_top_surface_sources): (Vec<_>, Vec<_>) =
            owned_region_exports.into_iter().unzip();
        let sorting_start = profile_enabled.then(Instant::now);
        Self::sort_node_owned_regions_with_sources(
            &mut owned_regions,
            &mut node_top_surface_sources,
        )?;
        profile.sorting_ms += elapsed_profile_ms(sorting_start);
        let boundary_sources_start = profile_enabled.then(Instant::now);
        let mut boundary_export_sources = NodeFootprintBoundaryExportSources::from_owned_regions(
            node_id,
            piece_kind,
            &owned_regions,
            &node_top_surface_sources,
            &node_grade_authorities,
            &explicit_vertical_step_segments,
        )?;
        boundary_export_sources
            .extend_arrangement_exposed_boundary_edges(arrangement, &top_height_context)?;
        profile.boundary_sources_ms = elapsed_profile_ms(boundary_sources_start);
        let raised_step_faces_start = profile_enabled.then(Instant::now);
        let (raised_step_faces, raised_steps, raised_step_stats) =
            Self::raised_step_faces_with_owned_top_support(
                &owned_regions,
                &explicit_vertical_step_segments,
                previous.map(|previous| &previous.raised_steps),
            );
        let mut raised_step_faces = raised_step_faces
            .into_iter()
            .map(|face| (face.polygon, face.source))
            .collect::<Vec<_>>();
        profile.raised_step_faces_ms = elapsed_profile_ms(raised_step_faces_start);

        let incremental_cache = NodeExportIncrementalCache {
            explicit_steps,
            height_splits,
            raised_steps,
        };
        let reuse_stats = NodeExportReuseStats {
            explicit_step_previous_hits: explicit_step_stats.previous_hits,
            explicit_step_misses: explicit_step_stats.misses,
            explicit_step_pair_previous_hits: explicit_step_stats.pair_previous_hits,
            explicit_step_pair_misses: explicit_step_stats.pair_misses,
            height_split_previous_hits: height_split_stats.previous_hits,
            height_split_misses: height_split_stats.misses,
            top_edge_cache_hits: raised_step_stats.top_edge_cache_hits,
            top_edge_previous_hits: raised_step_stats.top_edge_previous_hits,
            top_edge_cache_misses: raised_step_stats.top_edge_cache_misses,
            raised_step_cache_hits: raised_step_stats.raised_step_cache_hits,
            raised_step_previous_hits: raised_step_stats.raised_step_previous_hits,
            raised_step_cache_misses: raised_step_stats.raised_step_cache_misses,
        };

        if owned_regions.is_empty() {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }

        let material_partition_start = profile_enabled.then(Instant::now);
        let (mut road_surface_polygons, mut curb_surface_polygons, mut sidewalk_surface_polygons) =
            Self::top_polygons_from_owned_regions_by_material(&owned_regions);
        if road_surface_polygons.is_empty()
            && curb_surface_polygons.is_empty()
            && sidewalk_surface_polygons.is_empty()
        {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }
        let mut final_footprint_shapes = ownership_footprint_shapes.clone();
        Self::sort_overlay_shapes(&mut final_footprint_shapes);
        if final_footprint_shapes.is_empty() {
            return Err(NodeBoundaryExportError::EmptyOuterBoundary);
        }
        profile.material_partition_ms = elapsed_profile_ms(material_partition_start);
        let footprint_boundary_start = profile_enabled.then(Instant::now);
        let footprint_boundary_point_loops =
            Self::footprint_boundary_point_loops_from_footprint_shapes(
                &final_footprint_shapes,
                &owned_regions,
                &mut boundary_export_sources,
            )?;
        profile.footprint_boundary_ms = elapsed_profile_ms(footprint_boundary_start);
        let earthwork_boundary_start = profile_enabled.then(Instant::now);
        let mut earthwork_boundary_segments =
            node_earthwork_boundary_segments_from_footprint_loops(
                node_id,
                piece_kind,
                &footprint_boundary_point_loops,
                &boundary_export_sources,
            )?;
        Self::orient_earthwork_boundary_segment_loops_by_nesting(&mut earthwork_boundary_segments)
            .map_err(|_| NodeBoundaryExportError::DegenerateOuterBoundaryLoop)?;
        profile.earthwork_boundary_ms = elapsed_profile_ms(earthwork_boundary_start);
        let outer_boundary_start = profile_enabled.then(Instant::now);
        let mut outer_boundary_loops =
            Self::outer_boundary_polygons_from_footprint_boundary_point_loops(
                &footprint_boundary_point_loops,
            )?;
        profile.outer_boundary_ms = elapsed_profile_ms(outer_boundary_start);
        let terrain_clip_start = profile_enabled.then(Instant::now);
        let mut terrain_clip_boundary_loops =
            Self::terrain_clip_boundary_loops_from_earthwork_segments(&earthwork_boundary_segments);
        profile.terrain_clip_ms = elapsed_profile_ms(terrain_clip_start);

        let sorting_start = profile_enabled.then(Instant::now);
        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut curb_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Self::sort_terrain_clip_loops(&mut terrain_clip_boundary_loops);
        Self::sort_raised_step_faces(&mut raised_step_faces);
        profile.sorting_ms += elapsed_profile_ms(sorting_start);
        profile.owned_regions = owned_regions.len();
        profile.footprint_loops = footprint_boundary_point_loops.len();
        profile.earthwork_segments = earthwork_boundary_segments.len();
        profile.terrain_clip_loops = terrain_clip_boundary_loops.len();
        profile.raised_step_faces = raised_step_faces.len();
        profile.total_ms = elapsed_profile_ms(total_start);

        Ok((
            super::NodeSurfaceRegionResult {
                outer_boundary_loops,
                earthwork_boundary_segments,
                terrain_clip_boundary_loops,
                road_surface_polygons,
                curb_surface_polygons,
                raised_step_faces,
                sidewalk_surface_polygons,
                explicit_vertical_step_segments,
                node_grade_authorities,
                node_top_surface_sources,
                owned_regions,
                boolean_debug: None,
            },
            profile,
            incremental_cache,
            reuse_stats,
        ))
    }
}

fn reject_unauthorized_arrangement_height_splits(
    arrangement: &NodeArrangement,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
    previous: Option<&NodeHeightSplitIncrementalCache>,
) -> Result<(NodeHeightSplitIncrementalCache, NodeHeightSplitReuseStats), NodeBoundaryExportError> {
    let mut vertices_by_key = BTreeMap::<arrangement::NodeArrangementKey, Vec<_>>::new();
    for vertex in arrangement.vertices() {
        vertices_by_key
            .entry(vertex.key())
            .or_default()
            .push(vertex);
    }
    vertices_by_key.retain(|_, vertices| {
        let Some(first) = vertices.first().copied() else {
            return false;
        };
        let first_height_mm = first.height_mm();
        vertices
            .iter()
            .copied()
            .any(|vertex| vertex.height_mm() != first_height_mm)
    });
    if vertices_by_key.is_empty() {
        return Ok(Default::default());
    }

    let mut current = NodeHeightSplitIncrementalCache::default();
    let mut stats = NodeHeightSplitReuseStats::default();
    let mut conflict_cohorts = Vec::new();

    for (key, vertices) in vertices_by_key {
        let reuse_key = HeightSplitCohortReuseKey::from_vertices(key, &vertices);
        let conflicts = if let Some(conflicts) =
            previous.and_then(|previous| previous.conflicts.get(&reuse_key))
        {
            stats.previous_hits += 1;
            Arc::clone(conflicts)
        } else {
            stats.misses += 1;
            Arc::from(
                candidate_height_split_conflicts_for_cohort(key, &vertices).into_boxed_slice(),
            )
        };
        current.conflicts.insert(reuse_key, Arc::clone(&conflicts));
        if !conflicts.is_empty() {
            conflict_cohorts.push((key, conflicts));
        }
    }
    if conflict_cohorts.is_empty() {
        return Ok((current, stats));
    }

    let candidate_conflict_keys = conflict_cohorts
        .iter()
        .map(|(key, _)| *key)
        .collect::<Vec<_>>();
    let authorization_index = ArrangementHeightSplitAuthorizationIndex::new_for_keys(
        arrangement,
        explicit_vertical_step_segments,
        &candidate_conflict_keys,
    );
    let mut conflicts_by_owner_pair =
        BTreeMap::<(arrangement::NodeBandOwner, arrangement::NodeBandOwner), Vec<_>>::new();
    for (_, conflicts) in conflict_cohorts {
        for conflict in conflicts.iter().copied() {
            if arrangement_height_split_authorized(
                &authorization_index,
                conflict.0,
                conflict.1,
                conflict.2,
                conflict.3,
                conflict.4,
            ) {
                continue;
            }
            let owner_pair = ordered_arrangement_owner_pair(conflict.2, conflict.4);
            conflicts_by_owner_pair
                .entry(owner_pair)
                .or_default()
                .push(conflict);
        }
    }

    for conflicts in conflicts_by_owner_pair.into_values() {
        let mut keys = conflicts
            .iter()
            .map(|(key, _, _, _, _)| *key)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys.dedup();
        if keys.len() < 2 {
            continue;
        }
        let (key, existing_y_mm, existing_owner, incoming_y_mm, incoming_owner) = conflicts[0];
        return Err(
            NodeBoundaryExportError::ConflictingFootprintBoundaryHeight {
                x_key: key.x_key(),
                z_key: key.z_key(),
                existing_y_mm,
                incoming_y_mm,
                existing_owner_kind: existing_owner.kind(),
                existing_owner_index: existing_owner.owner_index(),
                existing_source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: key.x_key(),
                    z_key: key.z_key(),
                    y_mm: existing_y_mm,
                },
                incoming_owner_kind: incoming_owner.kind(),
                incoming_owner_index: incoming_owner.owner_index(),
                incoming_source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: key.x_key(),
                    z_key: key.z_key(),
                    y_mm: incoming_y_mm,
                },
            },
        );
    }

    Ok((current, stats))
}

fn candidate_height_split_conflicts_for_cohort(
    key: arrangement::NodeArrangementKey,
    vertices: &[&arrangement::NodeArrangementVertex],
) -> Vec<HeightSplitConflict> {
    let mut conflicts = Vec::new();
    for left_index in 0..vertices.len() {
        let left = vertices[left_index];
        for right in vertices.iter().copied().skip(left_index + 1) {
            if left.height_mm() == right.height_mm() {
                continue;
            }
            for left_owner in left.owners() {
                for right_owner in right.owners() {
                    if left_owner == right_owner {
                        continue;
                    }
                    if left_owner.kind() == right_owner.kind()
                        && grade_authorities_have_distinct_source_carrier_provenance(
                            left.grade_authority(),
                            right.grade_authority(),
                        )
                    {
                        continue;
                    }
                    if arrangement_vertices_form_source_authorized_side_join_split(
                        left,
                        *left_owner,
                        right,
                        *right_owner,
                    ) {
                        continue;
                    }
                    conflicts.push(normalized_height_split_conflict(
                        key,
                        left.height_mm(),
                        *left_owner,
                        right.height_mm(),
                        *right_owner,
                    ));
                }
            }
        }
    }
    conflicts.sort_unstable();
    conflicts.dedup();
    conflicts
}

fn normalized_height_split_conflict(
    key: arrangement::NodeArrangementKey,
    left_height_mm: i64,
    left_owner: arrangement::NodeBandOwner,
    right_height_mm: i64,
    right_owner: arrangement::NodeBandOwner,
) -> HeightSplitConflict {
    if (left_owner, left_height_mm) <= (right_owner, right_height_mm) {
        (
            key,
            left_height_mm,
            left_owner,
            right_height_mm,
            right_owner,
        )
    } else {
        (
            key,
            right_height_mm,
            right_owner,
            left_height_mm,
            left_owner,
        )
    }
}

impl HeightSplitCohortReuseKey {
    fn from_vertices(
        key: arrangement::NodeArrangementKey,
        vertices: &[&arrangement::NodeArrangementVertex],
    ) -> Self {
        let mut vertices = vertices
            .iter()
            .map(|vertex| {
                let mut owners = vertex.owners().to_vec();
                owners.sort_unstable();
                owners.dedup();
                HeightSplitVertexReuseKey {
                    height_mm: vertex.height_mm(),
                    owners: owners.into_boxed_slice(),
                    grade_authority: vertex.grade_authority(),
                }
            })
            .collect::<Vec<_>>();
        vertices.sort_unstable();
        Self {
            key,
            vertices: vertices.into_boxed_slice(),
        }
    }
}

fn arrangement_vertices_form_source_authorized_side_join_split(
    left: &arrangement::NodeArrangementVertex,
    left_owner: arrangement::NodeBandOwner,
    right: &arrangement::NodeArrangementVertex,
    right_owner: arrangement::NodeBandOwner,
) -> bool {
    let left_authority = left.grade_authority();
    let right_authority = right.grade_authority();
    if left_authority.owner != left_owner || right_authority.owner != right_owner {
        return false;
    }
    arrangement::source_authorities_form_side_join_asphalt_sidewalk_split(
        left_authority,
        right_authority,
    )
}

fn arrangement_height_split_authorized(
    authorization_index: &ArrangementHeightSplitAuthorizationIndex,
    key: arrangement::NodeArrangementKey,
    left_height_mm: i64,
    left_owner: arrangement::NodeBandOwner,
    right_height_mm: i64,
    right_owner: arrangement::NodeBandOwner,
) -> bool {
    let (lower_owner, raised_owner) = if left_height_mm <= right_height_mm {
        (left_owner, right_owner)
    } else {
        (right_owner, left_owner)
    };
    authorization_index.explicit_step_authorizes(key, lower_owner, raised_owner)
        || authorization_index.exposed_final_boundary_authorizes(key, lower_owner, raised_owner)
}

fn grade_authorities_have_distinct_source_carrier_provenance(
    left: height::NodeGradeVertexAuthority,
    right: height::NodeGradeVertexAuthority,
) -> bool {
    match (left.source_provenance, right.source_provenance) {
        (Some(left), Some(right)) => left != right,
        _ => false,
    }
}

struct ArrangementHeightSplitAuthorizationIndex {
    explicit_step_authorizations: BTreeSet<(
        arrangement::NodeArrangementKey,
        arrangement::NodeBandOwner,
        arrangement::NodeBandOwner,
    )>,
    exposed_owners_by_key:
        BTreeMap<arrangement::NodeArrangementKey, BTreeSet<arrangement::NodeBandOwner>>,
}

impl ArrangementHeightSplitAuthorizationIndex {
    fn new_for_keys(
        arrangement: &NodeArrangement,
        explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
        arrangement_keys: &[arrangement::NodeArrangementKey],
    ) -> Self {
        let mut arrangement_keys = arrangement_keys.to_vec();
        arrangement_keys.sort_unstable();
        arrangement_keys.dedup();
        let arrangement_key_index = ArrangementKeyIndex::new(&arrangement_keys);

        let mut explicit_step_authorizations = BTreeSet::new();
        for segment in explicit_vertical_step_segments {
            let (owner, opposite_owner) =
                ordered_arrangement_owner_pair(segment.owner(), segment.opposite_owner());
            let bounds = ArrangementKeyBounds::from_segment(segment.start(), segment.end())
                .expanded(super::boundary::BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS);
            arrangement_key_index.for_each_key_in_bounds(bounds, |key| {
                if arrangement_key_lies_on_step_segment_or_endpoint_dust(
                    key,
                    segment.start(),
                    segment.end(),
                ) {
                    explicit_step_authorizations.insert((key, owner, opposite_owner));
                }
            });
        }

        let exposed_edges = arrangement
            .edges()
            .iter()
            .filter(|edge| edge.exposed_boundary())
            .filter_map(|edge| {
                let start = arrangement.vertices().get(edge.start().index())?;
                let end = arrangement.vertices().get(edge.end().index())?;
                Some((start.key(), end.key(), edge.owner()))
            })
            .collect::<Vec<_>>();
        let mut exposed_owners_by_key = BTreeMap::<_, BTreeSet<arrangement::NodeBandOwner>>::new();
        for (start, end, owner) in exposed_edges {
            let bounds = ArrangementKeyBounds::from_segment(start, end);
            arrangement_key_index.for_each_key_in_bounds(bounds, |key| {
                if arrangement_key_lies_exactly_on_step_segment(key, start, end) {
                    exposed_owners_by_key.entry(key).or_default().insert(owner);
                }
            });
        }

        Self {
            explicit_step_authorizations,
            exposed_owners_by_key,
        }
    }

    fn explicit_step_authorizes(
        &self,
        key: arrangement::NodeArrangementKey,
        lower_owner: arrangement::NodeBandOwner,
        raised_owner: arrangement::NodeBandOwner,
    ) -> bool {
        let (owner, opposite_owner) = ordered_arrangement_owner_pair(lower_owner, raised_owner);
        self.explicit_step_authorizations
            .contains(&(key, owner, opposite_owner))
    }

    fn exposed_final_boundary_authorizes(
        &self,
        key: arrangement::NodeArrangementKey,
        lower_owner: arrangement::NodeBandOwner,
        raised_owner: arrangement::NodeBandOwner,
    ) -> bool {
        let Some(lower_rank) = band_semantics::raised_step_band_rank(lower_owner.kind()) else {
            return false;
        };
        let Some(raised_rank) = band_semantics::raised_step_band_rank(raised_owner.kind()) else {
            return false;
        };
        if lower_rank >= raised_rank
            || !band_semantics::raised_step_kinds_can_contact(
                lower_owner.kind(),
                raised_owner.kind(),
            )
        {
            return false;
        }
        let Some(exposed_owners) = self.exposed_owners_by_key.get(&key) else {
            return false;
        };
        exposed_owners.contains(&raised_owner) && !exposed_owners.contains(&lower_owner)
    }
}

#[derive(Clone, Copy, Debug)]
struct ArrangementKeyBounds {
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

#[derive(Clone, Debug)]
struct ArrangementKeyIndex {
    keys_by_x: Vec<(i64, arrangement::NodeArrangementKey)>,
    keys_by_z: Vec<(i64, arrangement::NodeArrangementKey)>,
}

impl ArrangementKeyIndex {
    fn new(keys: &[arrangement::NodeArrangementKey]) -> Self {
        let mut keys_by_x = keys
            .iter()
            .copied()
            .map(|key| (key.x_key(), key))
            .collect::<Vec<_>>();
        let mut keys_by_z = keys
            .iter()
            .copied()
            .map(|key| (key.z_key(), key))
            .collect::<Vec<_>>();
        keys_by_x.sort_unstable();
        keys_by_z.sort_unstable();
        Self {
            keys_by_x,
            keys_by_z,
        }
    }

    fn for_each_key_in_bounds(
        &self,
        bounds: ArrangementKeyBounds,
        mut visit: impl FnMut(arrangement::NodeArrangementKey),
    ) {
        let x_span = i128::from(bounds.max_x) - i128::from(bounds.min_x);
        let z_span = i128::from(bounds.max_z) - i128::from(bounds.min_z);
        let keys = if x_span <= z_span {
            self.keys_by_x_in_range(bounds.min_x, bounds.max_x)
        } else {
            self.keys_by_z_in_range(bounds.min_z, bounds.max_z)
        };
        for &(_, key) in keys {
            if bounds.contains(key) {
                visit(key);
            }
        }
    }

    fn keys_by_x_in_range(
        &self,
        min_x: i64,
        max_x: i64,
    ) -> &[(i64, arrangement::NodeArrangementKey)] {
        let start = self.keys_by_x.partition_point(|(x, _)| *x < min_x);
        let end = self.keys_by_x.partition_point(|(x, _)| *x <= max_x);
        &self.keys_by_x[start..end]
    }

    fn keys_by_z_in_range(
        &self,
        min_z: i64,
        max_z: i64,
    ) -> &[(i64, arrangement::NodeArrangementKey)] {
        let start = self.keys_by_z.partition_point(|(z, _)| *z < min_z);
        let end = self.keys_by_z.partition_point(|(z, _)| *z <= max_z);
        &self.keys_by_z[start..end]
    }
}

impl ArrangementKeyBounds {
    fn from_segment(
        start: arrangement::NodeArrangementKey,
        end: arrangement::NodeArrangementKey,
    ) -> Self {
        Self {
            min_x: start.x_key().min(end.x_key()),
            min_z: start.z_key().min(end.z_key()),
            max_x: start.x_key().max(end.x_key()),
            max_z: start.z_key().max(end.z_key()),
        }
    }

    fn contains(self, key: arrangement::NodeArrangementKey) -> bool {
        self.min_x <= key.x_key()
            && key.x_key() <= self.max_x
            && self.min_z <= key.z_key()
            && key.z_key() <= self.max_z
    }

    fn expanded(self, amount: i64) -> Self {
        Self {
            min_x: self.min_x - amount,
            min_z: self.min_z - amount,
            max_x: self.max_x + amount,
            max_z: self.max_z + amount,
        }
    }
}

fn ordered_arrangement_owner_pair(
    left: arrangement::NodeBandOwner,
    right: arrangement::NodeBandOwner,
) -> (arrangement::NodeBandOwner, arrangement::NodeBandOwner) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn arrangement_key_lies_exactly_on_step_segment(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> bool {
    let point = super::keys::SurfaceXzKey::from_raw_keys(point.x_key(), point.z_key());
    let start = super::keys::SurfaceXzKey::from_raw_keys(start.x_key(), start.z_key());
    let end = super::keys::SurfaceXzKey::from_raw_keys(end.x_key(), end.z_key());
    super::segments::key_lies_exactly_on_segment(point, start, end)
}

fn arrangement_key_lies_on_step_segment_or_endpoint_dust(
    point: arrangement::NodeArrangementKey,
    start: arrangement::NodeArrangementKey,
    end: arrangement::NodeArrangementKey,
) -> bool {
    arrangement_key_lies_exactly_on_step_segment(point, start, end)
        || arrangement_keys_are_endpoint_dust_neighbors(point, start)
        || arrangement_keys_are_endpoint_dust_neighbors(point, end)
}

fn arrangement_keys_are_endpoint_dust_neighbors(
    a: arrangement::NodeArrangementKey,
    b: arrangement::NodeArrangementKey,
) -> bool {
    let dx = i128::from(a.x_key() - b.x_key());
    let dz = i128::from(a.z_key() - b.z_key());
    let dust = i128::from(super::boundary::BOUNDARY_SOURCE_ENDPOINT_DUST_KEYS);
    dx * dx + dz * dz <= dust * dust
}

#[cfg(test)]
mod incremental_cache_tests {
    use super::*;

    fn height_conflict_arrangement(
        points: &[RoadVec2],
    ) -> (
        NodeArrangement,
        arrangement::NodeBandOwner,
        arrangement::NodeBandOwner,
    ) {
        let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let raised_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let lower_height =
            arrangement::NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
        let raised_height =
            arrangement::NodeBandHeightFieldId::new(1, 1, RoadSurfaceBandKind::CurbOrShoulder);
        let mut arrangement = NodeArrangement::new(401, RoadSurfaceVisualNodePieceKind::JunctionN);
        for point in points {
            arrangement
                .insert_vertex(*point, 0.0, [lower_owner], lower_height, [])
                .expect("lower conflict vertex must be valid");
            arrangement
                .insert_vertex(*point, 0.12, [raised_owner], raised_height, [])
                .expect("raised conflict vertex must be valid");
        }
        (arrangement, lower_owner, raised_owner)
    }

    #[test]
    fn height_split_cache_rebinds_current_authority_and_global_conflicts() {
        let point_a = RoadVec2::new(0.0, 0.0);
        let point_b = RoadVec2::new(1.0, 0.0);
        let (one_cohort, lower_owner, raised_owner) = height_conflict_arrangement(&[point_a]);
        let (one_cache, one_stats) =
            reject_unauthorized_arrangement_height_splits(&one_cohort, &[], None)
                .expect("one unauthorized XZ cohort is below the global conflict gate");
        assert_eq!(one_stats.misses, 1);

        let (two_cohorts, _, _) = height_conflict_arrangement(&[point_a, point_b]);
        assert!(matches!(
            reject_unauthorized_arrangement_height_splits(&two_cohorts, &[], Some(&one_cache),),
            Err(NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. })
        ));

        let authorized_step = arrangement::NodeExplicitVerticalStepSegment::new(
            arrangement::NodeArrangementKey::from_point(point_a),
            arrangement::NodeArrangementKey::from_point(point_b),
            lower_owner,
            raised_owner,
        )
        .expect("test authority segment is non-degenerate");
        let (authorized_cache, authorized_stats) =
            reject_unauthorized_arrangement_height_splits(&two_cohorts, &[authorized_step], None)
                .expect("the current explicit step must authorize both cached cohorts");
        assert_eq!(authorized_stats.misses, 2);
        assert!(matches!(
            reject_unauthorized_arrangement_height_splits(
                &two_cohorts,
                &[],
                Some(&authorized_cache),
            ),
            Err(NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. })
        ));

        let (_, removal_stats) = reject_unauthorized_arrangement_height_splits(
            &one_cohort,
            &[],
            Some(&authorized_cache),
        )
        .expect("a removed second cohort must not leak through cached global aggregation");
        assert_eq!(removal_stats.previous_hits, 1);
        assert_eq!(removal_stats.misses, 0);
    }
}
