//! Node surface export from canonical arrangement output.

use super::*;
use std::collections::BTreeMap;
use std::time::Instant;

mod assembly;
mod footprint_loops;
mod outer_boundary;
mod raised_step_support;
mod terrain_clip_loops;
mod top_regions;

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::simulation::network::surface) struct NodeExportProfile {
    pub(in crate::simulation::network::surface) total_ms: f64,
    pub(in crate::simulation::network::surface) height_split_ms: f64,
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
        let total_start = profile_enabled.then(Instant::now);
        let mut profile = NodeExportProfile {
            arrangement_faces: arrangement.faces().len(),
            ..NodeExportProfile::default()
        };
        let height_split_start = profile_enabled.then(Instant::now);
        let explicit_vertical_step_segments = arrangement.explicit_vertical_step_segments();
        reject_unauthorized_arrangement_height_splits(
            arrangement,
            &explicit_vertical_step_segments,
        )?;
        profile.height_split_ms = elapsed_profile_ms(height_split_start);
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
        profile.authority_ms = elapsed_profile_ms(authority_start);

        let mut owned_region_exports = Vec::new();

        let face_export_start = profile_enabled.then(Instant::now);
        for face in arrangement.faces() {
            let owner = face.owner();
            let Some((polygon, source)) =
                Self::visual_polygon_from_arrangement_face(arrangement, face, &authority_indices)?
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
            arrangement.node_id(),
            arrangement.piece_kind(),
            &owned_regions,
            &node_top_surface_sources,
            &node_grade_authorities,
            &explicit_vertical_step_segments,
        )?;
        boundary_export_sources.extend_arrangement_exposed_boundary_edges(arrangement)?;
        profile.boundary_sources_ms = elapsed_profile_ms(boundary_sources_start);
        let raised_step_faces_start = profile_enabled.then(Instant::now);
        let mut raised_step_faces = Self::raised_step_face_polygons_from_arrangement(
            arrangement,
            &explicit_vertical_step_segments,
        );
        Self::retain_raised_step_faces_with_owned_top_support(
            &mut raised_step_faces,
            &owned_regions,
            &explicit_vertical_step_segments,
        );
        let mut raised_step_faces = raised_step_faces
            .into_iter()
            .map(|face| (face.polygon, face.source))
            .collect::<Vec<_>>();
        profile.raised_step_faces_ms = elapsed_profile_ms(raised_step_faces_start);

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
                &mut boundary_export_sources,
            )?;
        profile.footprint_boundary_ms = elapsed_profile_ms(footprint_boundary_start);
        let earthwork_boundary_start = profile_enabled.then(Instant::now);
        let mut earthwork_boundary_segments =
            node_earthwork_boundary_segments_from_footprint_loops(
                arrangement.node_id(),
                arrangement.piece_kind(),
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
        ))
    }
}

fn reject_unauthorized_arrangement_height_splits(
    arrangement: &NodeArrangement,
    explicit_vertical_step_segments: &[arrangement::NodeExplicitVerticalStepSegment],
) -> Result<(), NodeBoundaryExportError> {
    let mut vertices_by_key = BTreeMap::<arrangement::NodeArrangementKey, Vec<_>>::new();
    for vertex in arrangement.vertices() {
        vertices_by_key
            .entry(vertex.key())
            .or_default()
            .push(vertex);
    }

    let mut candidate_conflicts_by_owner_pair =
        BTreeMap::<(arrangement::NodeBandOwner, arrangement::NodeBandOwner), Vec<_>>::new();
    let mut candidate_conflict_keys = BTreeSet::new();
    for (key, vertices) in vertices_by_key {
        for left_index in 0..vertices.len() {
            for right in vertices.iter().copied().skip(left_index + 1) {
                let left = vertices[left_index];
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
                        let owner_pair = if left_owner <= right_owner {
                            (*left_owner, *right_owner)
                        } else {
                            (*right_owner, *left_owner)
                        };
                        candidate_conflict_keys.insert(key);
                        candidate_conflicts_by_owner_pair
                            .entry(owner_pair)
                            .or_default()
                            .push((
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
    }
    if candidate_conflicts_by_owner_pair.is_empty() {
        return Ok(());
    }

    let candidate_conflict_keys = candidate_conflict_keys.into_iter().collect::<Vec<_>>();
    let authorization_index = ArrangementHeightSplitAuthorizationIndex::new_for_keys(
        arrangement,
        explicit_vertical_step_segments,
        &candidate_conflict_keys,
    );
    let mut conflicts_by_owner_pair =
        BTreeMap::<(arrangement::NodeBandOwner, arrangement::NodeBandOwner), Vec<_>>::new();
    for (owner_pair, conflicts) in candidate_conflicts_by_owner_pair {
        for conflict in conflicts {
            let (key, left_height_mm, left_owner, right_height_mm, right_owner) = conflict;
            if arrangement_height_split_authorized(
                &authorization_index,
                key,
                left_height_mm,
                left_owner,
                right_height_mm,
                right_owner,
            ) {
                continue;
            }
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
    Ok(())
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
            let bounds = ArrangementKeyBounds::from_segment(segment.start(), segment.end());
            arrangement_key_index.for_each_key_in_bounds(bounds, |key| {
                if arrangement_key_lies_exactly_on_step_segment(key, segment.start(), segment.end())
                {
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
