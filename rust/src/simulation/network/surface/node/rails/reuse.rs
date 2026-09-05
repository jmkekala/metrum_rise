//! Exact node-local plan-topology reuse for node-surface recompilation.

use super::super::input::NodeArrangementInput;
use super::super::keys::SurfaceHeightMmKey;
use super::super::ownership::{NodeRailHeightSourceKey, NodeSourceCarrierRegistry};
use super::super::{
    IncidentEdgeSide, RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind,
    backend::{RoadVec3, road_vec3_xz},
    joins::NodeInputSideJoinGapRole,
};
use super::contacts::synchronize_shared_height_contact_vertices;
use super::contacts::{
    NodeContactNodingPairCache, NodeRetainedContactCache, NodeSameMaterialContactPairCache,
    NodeSourceAuthorizedContactCache,
};
use super::contours::height_for_key_on_generated_edge;
use super::geometry::{road_point_from_key, road_point_key};
use super::topology::NodeRailPointKey;
use super::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeGeneratedContourPurpose, NodeRailBuildProfile, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailContourSet, NodeRailGenerationError, NodeRailHeightCarrierPaths,
};
use crate::simulation::network::surface::node::arrangement::NodeBandOwner;

#[derive(Clone, Debug)]
pub(crate) struct NodeRailTopologyCache {
    base_topology: Option<NodeRailTopologyKey>,
    final_topology: Option<NodeRailTopologyKey>,
    incremental: NodeRailIncrementalCache,
    rails: Option<NodeRailContourSet>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct NodeRailIncrementalCache {
    pub(super) same_material_contact_pairs: NodeSameMaterialContactPairCache,
    pub(super) source_authorized_contacts: NodeSourceAuthorizedContactCache,
    pub(super) contact_noding_pairs: NodeContactNodingPairCache,
    pub(super) retained_contacts: NodeRetainedContactCache,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NodeRailReuseStatus {
    pub(crate) rail_topology_reused: bool,
    pub(crate) ownership_reuse_safe: bool,
    pub(crate) arrangement_reuse_safe: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NodeRailTopologyKey {
    piece_kind: RoadSurfaceVisualNodePieceKind,
    ordered_mouth_sides: Vec<IncidentEdgeSide>,
    contours: Vec<NodeRailContourTopology>,
    constraints: Vec<NodeRailConstraintTopology>,
    side_join_gaps: Vec<NodeRailSideJoinTopology>,
    corner_trims: Vec<NodeRailCornerTrimTopology>,
    height_carrier_paths: Vec<(
        NodeRailHeightSourceKey,
        Vec<NodeRailHeightCarrierPathTopology>,
    )>,
    height_carrier_points: Vec<(NodeRailHeightSourceKey, Vec<NodeRailPointKey>)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NodeRailContourTopology {
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    points: Vec<NodeRailPointKey>,
    has_height_carrier: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NodeRailConstraintTopology {
    constraint_index: usize,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    source_boundary_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    points: Vec<NodeRailPointKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NodeRailSideJoinTopology {
    from_mouth_order_index: usize,
    to_mouth_order_index: usize,
    from_side: IncidentEdgeSide,
    to_side: IncidentEdgeSide,
    angle_rad_bits: u64,
    role: NodeInputSideJoinGapRole,
    emitted_band_kinds: Vec<RoadSurfaceBandKind>,
    suppressed_band_kinds: Vec<RoadSurfaceBandKind>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NodeRailCornerTrimTopology {
    source_mouth_order_index: usize,
    source_band_index: usize,
    source_band_kind: RoadSurfaceBandKind,
    source_owner: NodeBandOwner,
    points: Vec<NodeRailPointKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NodeRailHeightCarrierPathTopology {
    start: Vec<NodeRailPointKey>,
    end: Vec<NodeRailPointKey>,
}

impl NodeRailContourSet {
    pub(crate) fn from_input_with_profile_and_topology_reuse(
        input: &NodeArrangementInput,
        profile_enabled: bool,
        previous: Option<&NodeRailTopologyCache>,
    ) -> Result<
        (
            Self,
            NodeRailBuildProfile,
            NodeRailReuseStatus,
            NodeRailTopologyCache,
        ),
        NodeRailGenerationError,
    > {
        let (base, source_constraint_count, mut profile) =
            Self::base_from_input_with_profile(input, profile_enabled)?;
        let base_topology = NodeRailTopologyKey::from_rails(input, &base);

        if crate::debug::category_enabled("road")
            && let Some(previous_topology) = previous.and_then(|cache| cache.base_topology.as_ref())
            && previous_topology != &base_topology
        {
            crate::debug_log!(
                "road",
                "node_rail_topology_cache_miss node={} mismatch={}",
                input.node_id,
                previous_topology.first_mismatch(&base_topology)
            );
        }

        if let Some(previous) = previous
            && previous.base_topology.as_ref() == Some(&base_topology)
            && let Some(previous_rails) = previous.rails.as_ref()
            && let Some(rails) = project_cached_topology_onto_fresh_base(&base, previous_rails)
        {
            let final_topology = NodeRailTopologyKey::from_rails(input, &rails);
            if previous.final_topology.as_ref() == Some(&final_topology)
                && rails.source_carriers == previous_rails.source_carriers
            {
                let exact_height_translation_mm =
                    exact_uniform_height_translation_mm(previous_rails, &rails);
                let arrangement_reuse_safe = exact_height_values_match(previous_rails, &rails);
                profile.contours = rails.contours.len();
                profile.constraints = rails.constraints.len();
                profile.validation_constraints = rails.constraints.len();
                profile.height_carrier_sources = rails.height_carrier_points_by_source.len();
                profile.height_carrier_points = rails
                    .height_carrier_points_by_source
                    .values()
                    .map(Vec::len)
                    .sum();
                let cache = NodeRailTopologyCache {
                    base_topology: Some(base_topology),
                    final_topology: Some(final_topology),
                    incremental: previous.incremental.clone(),
                    rails: Some(rails.clone()),
                };
                return Ok((
                    rails,
                    profile,
                    NodeRailReuseStatus {
                        rail_topology_reused: true,
                        ownership_reuse_safe: exact_height_translation_mm.is_some(),
                        arrangement_reuse_safe,
                    },
                    cache,
                ));
            }
        }

        if crate::debug::category_enabled("road")
            && let Some(previous) = previous
            && previous.base_topology.as_ref() == Some(&base_topology)
        {
            let rejection = match previous.rails.as_ref() {
                None => "missing_cached_rails",
                Some(previous_rails) => {
                    match project_cached_topology_onto_fresh_base(&base, previous_rails) {
                        None => "height_projection",
                        Some(rails) => {
                            let final_topology = NodeRailTopologyKey::from_rails(input, &rails);
                            if previous.final_topology.as_ref() != Some(&final_topology) {
                                "final_topology"
                            } else if rails.source_carriers != previous_rails.source_carriers {
                                "source_carriers"
                            } else {
                                "unknown"
                            }
                        }
                    }
                }
            };
            crate::debug_log!(
                "road",
                "node_rail_topology_replay_rejected node={} reason={}",
                input.node_id,
                rejection
            );
        }

        let (rails, profile, incremental) = Self::finish_base_with_profile(
            base,
            source_constraint_count,
            profile,
            profile_enabled,
            previous.map(|previous| &previous.incremental),
        )?;
        let cache = NodeRailTopologyCache {
            base_topology: Some(base_topology),
            final_topology: Some(NodeRailTopologyKey::from_rails(input, &rails)),
            incremental,
            rails: Some(rails.clone()),
        };
        Ok((rails, profile, NodeRailReuseStatus::default(), cache))
    }
}

impl NodeRailTopologyCache {
    /// Drops whole-topology replay data while retaining contributor caches for a later node edit.
    pub(crate) fn into_incremental_only(mut self) -> Self {
        self.base_topology = None;
        self.final_topology = None;
        self.rails = None;
        self
    }
}

impl NodeRailTopologyKey {
    fn first_mismatch(&self, other: &Self) -> &'static str {
        if self.piece_kind != other.piece_kind {
            "piece_kind"
        } else if self.ordered_mouth_sides != other.ordered_mouth_sides {
            "ordered_mouth_sides"
        } else if self.contours != other.contours {
            "contours"
        } else if self.constraints != other.constraints {
            "constraints"
        } else if self.side_join_gaps != other.side_join_gaps {
            "side_join_gaps"
        } else if self.corner_trims != other.corner_trims {
            "corner_trims"
        } else if self.height_carrier_paths != other.height_carrier_paths {
            "height_carrier_paths"
        } else if self.height_carrier_points != other.height_carrier_points {
            "height_carrier_points"
        } else {
            "unknown"
        }
    }

    fn from_rails(input: &NodeArrangementInput, rails: &NodeRailContourSet) -> Self {
        Self {
            piece_kind: rails.piece_kind,
            ordered_mouth_sides: input.mouths.iter().map(|mouth| mouth.side).collect(),
            contours: rails
                .contours
                .iter()
                .map(NodeRailContourTopology::from_contour)
                .collect(),
            constraints: rails
                .constraints
                .iter()
                .map(NodeRailConstraintTopology::from_constraint)
                .collect(),
            side_join_gaps: rails
                .side_join_gaps
                .iter()
                .map(|gap| NodeRailSideJoinTopology {
                    from_mouth_order_index: gap.from_mouth_order_index,
                    to_mouth_order_index: gap.to_mouth_order_index,
                    from_side: gap.from_side,
                    to_side: gap.to_side,
                    angle_rad_bits: gap.angle_rad.to_bits(),
                    role: gap.role,
                    emitted_band_kinds: gap.emitted_band_kinds.clone(),
                    suppressed_band_kinds: gap.suppressed_band_kinds.clone(),
                })
                .collect(),
            corner_trims: rails
                .corner_trims
                .iter()
                .map(|trim| NodeRailCornerTrimTopology {
                    source_mouth_order_index: trim.source_mouth_order_index,
                    source_band_index: trim.source_band_index,
                    source_band_kind: trim.source_band_kind,
                    source_owner: trim.source_owner,
                    points: trim.points_xz.iter().copied().map(road_point_key).collect(),
                })
                .collect(),
            height_carrier_paths: rails
                .height_carrier_paths_by_source
                .iter()
                .map(|(source, paths)| {
                    (
                        *source,
                        paths
                            .iter()
                            .map(NodeRailHeightCarrierPathTopology::from_paths)
                            .collect(),
                    )
                })
                .collect(),
            height_carrier_points: rails
                .height_carrier_points_by_source
                .iter()
                .map(|(source, points)| {
                    (
                        *source,
                        points
                            .iter()
                            .copied()
                            .map(road_vec3_xz)
                            .map(road_point_key)
                            .collect(),
                    )
                })
                .collect(),
        }
    }
}

impl NodeRailContourTopology {
    fn from_contour(contour: &NodeGeneratedContour) -> Self {
        Self {
            kind: contour.kind,
            purpose: contour.purpose,
            source_mouth_order_index: contour.source_mouth_order_index,
            source_band_index: contour.source_band_index,
            owner: contour.owner,
            claim_priority: contour.claim_priority,
            points: contour
                .points_xz
                .iter()
                .copied()
                .map(road_point_key)
                .collect(),
            has_height_carrier: contour.height_points_world.is_some(),
        }
    }
}

impl NodeRailConstraintTopology {
    fn from_constraint(constraint: &NodeRailConstraint) -> Self {
        Self {
            constraint_index: constraint.constraint_index,
            kind: constraint.kind,
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            source_boundary_index: constraint.source_boundary_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
            points: constraint
                .points_xz
                .iter()
                .copied()
                .map(road_point_key)
                .collect(),
        }
    }
}

impl NodeRailHeightCarrierPathTopology {
    fn from_paths(paths: &NodeRailHeightCarrierPaths) -> Self {
        Self {
            start: paths
                .start_path_world
                .iter()
                .copied()
                .map(road_vec3_xz)
                .map(road_point_key)
                .collect(),
            end: paths
                .end_path_world
                .iter()
                .copied()
                .map(road_vec3_xz)
                .map(road_point_key)
                .collect(),
        }
    }
}

fn exact_uniform_height_translation_mm(
    previous: &NodeRailContourSet,
    current: &NodeRailContourSet,
) -> Option<i64> {
    let mut translation_mm = None;
    if previous.contours.len() != current.contours.len() {
        return None;
    }
    for (previous_contour, current_contour) in previous.contours.iter().zip(&current.contours) {
        match (
            previous_contour.height_points_world.as_deref(),
            current_contour.height_points_world.as_deref(),
        ) {
            (None, None) => {}
            (Some(previous_points), Some(current_points)) => {
                if !include_uniform_height_translation(
                    previous_points,
                    current_points,
                    &mut translation_mm,
                ) {
                    return None;
                }
            }
            _ => return None,
        }
    }

    if previous.height_carrier_paths_by_source.len() != current.height_carrier_paths_by_source.len()
    {
        return None;
    }
    for ((previous_source, previous_paths), (current_source, current_paths)) in previous
        .height_carrier_paths_by_source
        .iter()
        .zip(&current.height_carrier_paths_by_source)
    {
        if previous_source != current_source || previous_paths.len() != current_paths.len() {
            return None;
        }
        for (previous_path, current_path) in previous_paths.iter().zip(current_paths) {
            if !include_uniform_height_translation(
                &previous_path.start_path_world,
                &current_path.start_path_world,
                &mut translation_mm,
            ) || !include_uniform_height_translation(
                &previous_path.end_path_world,
                &current_path.end_path_world,
                &mut translation_mm,
            ) {
                return None;
            }
        }
    }

    if previous.height_carrier_points_by_source.len()
        != current.height_carrier_points_by_source.len()
    {
        return None;
    }
    for ((previous_source, previous_points), (current_source, current_points)) in previous
        .height_carrier_points_by_source
        .iter()
        .zip(&current.height_carrier_points_by_source)
    {
        if previous_source != current_source
            || !include_uniform_height_translation(
                previous_points,
                current_points,
                &mut translation_mm,
            )
        {
            return None;
        }
    }
    Some(translation_mm.unwrap_or(0))
}

fn exact_height_values_match(previous: &NodeRailContourSet, current: &NodeRailContourSet) -> bool {
    previous.contours.len() == current.contours.len()
        && previous
            .contours
            .iter()
            .zip(&current.contours)
            .all(|(previous, current)| {
                exact_optional_point_heights_match(
                    previous.height_points_world.as_deref(),
                    current.height_points_world.as_deref(),
                )
            })
        && previous.height_carrier_paths_by_source.len()
            == current.height_carrier_paths_by_source.len()
        && previous
            .height_carrier_paths_by_source
            .iter()
            .zip(&current.height_carrier_paths_by_source)
            .all(
                |((previous_source, previous_paths), (current_source, current_paths))| {
                    previous_source == current_source
                        && previous_paths.len() == current_paths.len()
                        && previous_paths.iter().zip(current_paths).all(
                            |(previous_path, current_path)| {
                                exact_point_heights_match(
                                    &previous_path.start_path_world,
                                    &current_path.start_path_world,
                                ) && exact_point_heights_match(
                                    &previous_path.end_path_world,
                                    &current_path.end_path_world,
                                )
                            },
                        )
                },
            )
        && previous.height_carrier_points_by_source.len()
            == current.height_carrier_points_by_source.len()
        && previous
            .height_carrier_points_by_source
            .iter()
            .zip(&current.height_carrier_points_by_source)
            .all(
                |((previous_source, previous_points), (current_source, current_points))| {
                    previous_source == current_source
                        && exact_point_heights_match(previous_points, current_points)
                },
            )
}

fn exact_optional_point_heights_match(
    previous: Option<&[RoadVec3]>,
    current: Option<&[RoadVec3]>,
) -> bool {
    match (previous, current) {
        (None, None) => true,
        (Some(previous), Some(current)) => exact_point_heights_match(previous, current),
        _ => false,
    }
}

fn exact_point_heights_match(previous: &[RoadVec3], current: &[RoadVec3]) -> bool {
    previous.len() == current.len()
        && previous
            .iter()
            .zip(current)
            .all(|(previous, current)| previous.y.to_bits() == current.y.to_bits())
}

fn include_uniform_height_translation(
    previous: &[RoadVec3],
    current: &[RoadVec3],
    translation_mm: &mut Option<i64>,
) -> bool {
    if previous.len() != current.len() {
        return false;
    }
    for (previous, current) in previous.iter().zip(current) {
        if road_point_key(road_vec3_xz(*previous)) != road_point_key(road_vec3_xz(*current)) {
            return false;
        }
        let previous_mm = SurfaceHeightMmKey::from_m_f64(previous.y).as_i64();
        let current_mm = SurfaceHeightMmKey::from_m_f64(current.y).as_i64();
        let Some(delta_mm) = current_mm.checked_sub(previous_mm) else {
            return false;
        };
        match *translation_mm {
            Some(expected_mm) if expected_mm != delta_mm => return false,
            Some(_) => {}
            None => *translation_mm = Some(delta_mm),
        }
    }
    true
}

fn project_cached_topology_onto_fresh_base(
    base: &NodeRailContourSet,
    cached: &NodeRailContourSet,
) -> Option<NodeRailContourSet> {
    if base.contours.len() != cached.contours.len() {
        return None;
    }
    let mut projected = cached.clone();
    for ((projected_contour, cached_contour), base_contour) in projected
        .contours
        .iter_mut()
        .zip(&cached.contours)
        .zip(&base.contours)
    {
        projected_contour.height_points_world =
            project_contour_heights(base_contour, cached_contour)?;
    }
    projected.height_carrier_paths_by_source = base.height_carrier_paths_by_source.clone();
    projected.height_carrier_points_by_source = base.height_carrier_points_by_source.clone();
    projected.corner_trims = base.corner_trims.clone();
    projected.side_join_gaps = base.side_join_gaps.clone();
    synchronize_shared_height_contact_vertices(&mut projected.contours, &projected.constraints);
    projected.source_carriers = NodeSourceCarrierRegistry::from_rail_parts(
        &projected.contours,
        &projected.constraints,
        &projected.height_carrier_paths_by_source,
        &projected.height_carrier_points_by_source,
    );
    Some(projected)
}

fn project_contour_heights(
    base: &NodeGeneratedContour,
    cached: &NodeGeneratedContour,
) -> Option<Option<Vec<RoadVec3>>> {
    let Some(base_heights) = base.height_points_world.as_ref() else {
        return cached.height_points_world.is_none().then_some(None);
    };
    if cached.height_points_world.is_none()
        || base_heights.len() != base.points_xz.len()
        || base.points_xz.len() < 2
    {
        return None;
    }
    let base_keys = base
        .points_xz
        .iter()
        .copied()
        .map(road_point_key)
        .collect::<Vec<_>>();
    let mut projected = Vec::with_capacity(cached.points_xz.len());
    for target in cached.points_xz.iter().copied().map(road_point_key) {
        let mut height_mm = None;
        for index in 0..base_keys.len() {
            let next = (index + 1) % base_keys.len();
            let Some(height) = height_for_key_on_generated_edge(
                target,
                base_keys[index],
                base_keys[next],
                base_heights[index].y,
                base_heights[next].y,
            ) else {
                continue;
            };
            let candidate = SurfaceHeightMmKey::from_m_f64(height).as_i64();
            if height_mm.is_some_and(|existing| existing != candidate) {
                return None;
            }
            height_mm = Some(candidate);
        }
        let height_mm = height_mm?;
        let point = road_point_from_key(target);
        projected.push(RoadVec3::new(point.x, height_mm as f64 / 1000.0, point.y));
    }
    Some(Some(projected))
}
