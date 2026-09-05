// SPDX-License-Identifier: GPL-2.0-only

//! Exact contributor-local reuse for node ownership compilation.

use super::super::arrangement::{NodeBandOwner, NodeRegionSeamConstraint, NodeSeamSource};
use super::super::backend::{RoadVec3, road_vec3_xz};
use super::super::rails::{
    NodeGeneratedContourClaimPriority, NodeGeneratedContourKind, NodeGeneratedContourPurpose,
    NodeRailConstraint, NodeRailConstraintKind, NodeRailContourSet,
};
use super::rail_authority::NodeRailSourceSegmentMaterialization;
use super::seams::ConstraintOverlapMode;
use super::topology_keys::{
    NodeOwnershipPointKey, ownership_key_from_overlay_point, ownership_key_from_road_point,
};
use super::{
    NodeBooleanOwnedRegion, NodeOverlayShape, NodeOwnedRegionArrangement, RoadSurfaceBandKind,
    RoadSurfaceVisualNodePieceKind,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ExactOverlayPointReuseKey {
    topology: NodeOwnershipPointKey,
    x_bits: u64,
    z_bits: u64,
}

impl ExactOverlayPointReuseKey {
    fn from_point(point: [f64; 2]) -> Self {
        Self {
            topology: ownership_key_from_overlay_point(point),
            x_bits: point[0].to_bits(),
            z_bits: point[1].to_bits(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct CanonicalOwnedShapeKey {
    ordered_contours: Box<[Box<[ExactOverlayPointReuseKey]>]>,
}

impl CanonicalOwnedShapeKey {
    fn from_shape(shape: &NodeOverlayShape) -> Self {
        Self {
            ordered_contours: shape
                .iter()
                .map(|contour| {
                    contour
                        .iter()
                        .copied()
                        .map(ExactOverlayPointReuseKey::from_point)
                        .collect::<Vec<_>>()
                        .into_boxed_slice()
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum OwnedShapeCleanupOperation {
    CleanUnionAndSplit,
    FinalSelfTouchSplit,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct OwnedShapeCleanupKey {
    operation: OwnedShapeCleanupOperation,
    clean_numeric_spikes: bool,
    shape: CanonicalOwnedShapeKey,
}

impl OwnedShapeCleanupKey {
    fn from_shape(shape: &NodeOverlayShape, clean_numeric_spikes: bool) -> Self {
        Self {
            operation: OwnedShapeCleanupOperation::CleanUnionAndSplit,
            clean_numeric_spikes,
            shape: CanonicalOwnedShapeKey::from_shape(shape),
        }
    }

    fn from_final_self_touch(shape: &NodeOverlayShape, clean_numeric_spikes: bool) -> Self {
        Self {
            operation: OwnedShapeCleanupOperation::FinalSelfTouchSplit,
            clean_numeric_spikes,
            shape: CanonicalOwnedShapeKey::from_shape(shape),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct CarrierPoint3Key {
    point_xz: NodeOwnershipPointKey,
    height_bits: u64,
}

impl CarrierPoint3Key {
    fn from_world(point: RoadVec3) -> Self {
        Self {
            point_xz: ownership_key_from_road_point(road_vec3_xz(point)),
            height_bits: point.y.to_bits(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RailConstraintReuseKey {
    constraint_index: usize,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    source_boundary_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    points: Box<[NodeOwnershipPointKey]>,
}

impl RailConstraintReuseKey {
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
                .map(ownership_key_from_road_point)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RegionSeamConstraintReuseKey {
    constraint_index: usize,
    seam_source: NodeSeamSource,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    constrains_shared_height: bool,
    is_material_transition: bool,
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
}

impl RegionSeamConstraintReuseKey {
    fn from_constraint(constraint: &NodeRegionSeamConstraint) -> Self {
        Self {
            constraint_index: constraint.constraint_index,
            seam_source: constraint.seam_source,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
            constrains_shared_height: constraint.constrains_shared_height,
            is_material_transition: constraint.is_material_transition,
            start: ownership_key_from_road_point(constraint.start_xz),
            end: ownership_key_from_road_point(constraint.end_xz),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RegionSeamExtractionReuseKey {
    shape: CanonicalOwnedShapeKey,
    owner: NodeBandOwner,
    grid_bounded_overlap: bool,
    applicable_constraints: Box<[Arc<RailConstraintReuseKey>]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct MaterializedOwnedEdgeSeamReuseKey {
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: u8,
    source_seams: Box<[RegionSeamConstraintReuseKey]>,
    local_constraints: Box<[Arc<RailConstraintReuseKey>]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedCarrierContourReuseKey {
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    claim_priority: NodeGeneratedContourClaimPriority,
    points: Box<[NodeOwnershipPointKey]>,
    height_points: Box<[CarrierPoint3Key]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct CarrierPathReuseKey {
    start: Box<[CarrierPoint3Key]>,
    end: Box<[CarrierPoint3Key]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct CarrierSourceAuthorityReuseKey {
    piece_kind: u8,
    source_height_points: Box<[CarrierPoint3Key]>,
    canonical_height_points: Box<[NodeOwnershipPointKey]>,
    source_segments: Box<[(NodeOwnershipPointKey, NodeOwnershipPointKey, bool)]>,
    numeric_dust_canonicalized: bool,
    paths: Box<[CarrierPathReuseKey]>,
    generated_contours: Box<[GeneratedCarrierContourReuseKey]>,
    source_constraints: Box<[Arc<RailConstraintReuseKey>]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct CarrierSourceAuthorityLookupKey {
    owner: NodeBandOwner,
    kind: RoadSurfaceBandKind,
    source_mouth_order_index: usize,
    source_band_index: usize,
}

impl CarrierSourceAuthorityReuseKey {
    fn for_region(
        region: &NodeBooleanOwnedRegion,
        rails: &NodeRailContourSet,
        constraint_keys: &BTreeMap<usize, Arc<RailConstraintReuseKey>>,
    ) -> Option<Self> {
        let source_band_index = region.source_band_index?;
        let source = (
            region.kind,
            region.source_mouth_order_index,
            source_band_index,
        );
        let source_height_points = rails
            .height_carrier_points_by_source
            .get(&source)
            .into_iter()
            .flatten()
            .copied()
            .map(CarrierPoint3Key::from_world)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let canonical_height_points = rails
            .source_carriers
            .height_points_by_source
            .get(&source)
            .into_iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let source_segments = rails
            .source_carriers
            .source_segments_by_owner
            .get(&region.owner)
            .into_iter()
            .flatten()
            .filter(|segment| segment.source == source)
            .map(|segment| {
                (
                    segment.segment.start,
                    segment.segment.end,
                    matches!(
                        segment.materialization,
                        NodeRailSourceSegmentMaterialization::GeneratedCarrierSurface
                    ),
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let paths = rails
            .height_carrier_paths_by_source
            .get(&source)
            .into_iter()
            .flatten()
            .map(|paths| CarrierPathReuseKey {
                start: paths
                    .start_path_world
                    .iter()
                    .copied()
                    .map(CarrierPoint3Key::from_world)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                end: paths
                    .end_path_world
                    .iter()
                    .copied()
                    .map(CarrierPoint3Key::from_world)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let generated_contours = rails
            .contours
            .iter()
            .filter(|contour| {
                contour.owner == Some(region.owner)
                    && contour.source_mouth_order_index == source.1
                    && contour.source_band_index == Some(source.2)
                    && contour.height_points_world.is_some()
                    && matches!(
                        contour.kind,
                        NodeGeneratedContourKind::Band { kind } if kind == source.0
                    )
            })
            .map(|contour| GeneratedCarrierContourReuseKey {
                kind: contour.kind,
                purpose: contour.purpose,
                claim_priority: contour.claim_priority,
                points: contour
                    .points_xz
                    .iter()
                    .copied()
                    .map(ownership_key_from_road_point)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                height_points: contour
                    .height_points_world
                    .iter()
                    .flatten()
                    .copied()
                    .map(CarrierPoint3Key::from_world)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let source_constraints = rails
            .constraints
            .iter()
            .filter(|constraint| {
                constraint.source_mouth_order_index == source.1
                    && constraint.source_band_index == Some(source.2)
                    && [constraint.owner, constraint.opposite_owner].contains(&Some(region.owner))
            })
            .map(|constraint| {
                constraint_keys
                    .get(&constraint.constraint_index)
                    .cloned()
                    .unwrap_or_else(|| {
                        Arc::new(RailConstraintReuseKey::from_constraint(constraint))
                    })
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Some(Self {
            piece_kind: piece_kind_key(rails.piece_kind),
            source_height_points,
            canonical_height_points,
            source_segments,
            numeric_dust_canonicalized: rails
                .source_carriers
                .numeric_dust_canonicalized_sources
                .contains(&source),
            paths,
            generated_contours,
            source_constraints,
        })
    }
}

fn piece_kind_key(kind: RoadSurfaceVisualNodePieceKind) -> u8 {
    match kind {
        RoadSurfaceVisualNodePieceKind::Terminal => 0,
        RoadSurfaceVisualNodePieceKind::Bend => 1,
        RoadSurfaceVisualNodePieceKind::JunctionN => 2,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FinalBoundaryPointReuseKey {
    kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    claim_priority: NodeGeneratedContourClaimPriority,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    point: NodeOwnershipPointKey,
    carrier_authority: Option<Arc<CarrierSourceAuthorityReuseKey>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct FinalBoundaryAssemblyRegionReuseKey {
    kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    claim_priority: NodeGeneratedContourClaimPriority,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    shape: CanonicalOwnedShapeKey,
}

impl FinalBoundaryAssemblyRegionReuseKey {
    fn from_region(region: &NodeBooleanOwnedRegion) -> Self {
        Self {
            kind: region.kind,
            owner: region.owner,
            claim_priority: region.claim_priority,
            source_mouth_order_index: region.source_mouth_order_index,
            source_band_index: region.source_band_index,
            shape: CanonicalOwnedShapeKey::from_shape(&region.shape),
        }
    }
}

/// Exact post-materialization inputs for one final-boundary assembly.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct FinalBoundaryAssemblyReuseKey {
    node_id: u32,
    piece_kind: u8,
    grid_bounded_overlap: bool,
    regions: Box<[FinalBoundaryAssemblyRegionReuseKey]>,
    constraints: Box<[Arc<RailConstraintReuseKey>]>,
}

#[derive(Clone, Debug)]
struct FinalBoundaryAssemblyReuseValue {
    footprint_shapes: Arc<[NodeOverlayShape]>,
    region_seams: Arc<[Arc<[NodeRegionSeamConstraint]>]>,
    arrangement: Arc<NodeOwnedRegionArrangement>,
    extracted_region_seam_keys: Arc<[RegionSeamExtractionReuseKey]>,
    materialized_edge_seam_keys: Arc<[MaterializedOwnedEdgeSeamReuseKey]>,
}

/// Result of probing the immutable final-boundary assembly cache.
pub(super) enum FinalBoundaryAssemblyLookup {
    /// Exact cached footprint and arrangement output.
    Hit(Vec<NodeOverlayShape>, NodeOwnedRegionArrangement),
    /// Opaque miss key reused when the rebuilt output is stored.
    Miss(FinalBoundaryAssemblyReuseKey),
}

/// Exact contributor outputs reusable by a later ownership generation.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeOwnershipIncrementalCache {
    cleaned_owned_shapes: BTreeMap<OwnedShapeCleanupKey, Arc<[NodeOverlayShape]>>,
    final_boundary_points: BTreeMap<FinalBoundaryPointReuseKey, bool>,
    final_boundary_assemblies:
        BTreeMap<FinalBoundaryAssemblyReuseKey, FinalBoundaryAssemblyReuseValue>,
    extracted_region_seams: BTreeMap<RegionSeamExtractionReuseKey, Arc<[NodeRegionSeamConstraint]>>,
    materialized_edge_seams:
        BTreeMap<MaterializedOwnedEdgeSeamReuseKey, Arc<[NodeRegionSeamConstraint]>>,
}

/// Exact contributor-reuse activity for one ownership build.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct NodeOwnershipReuseStats {
    /// Cleanup requests served by the current or previous exact-key cache.
    pub(crate) cleanup_cache_hits: usize,
    /// Cleanup requests promoted from the immutable previous generation.
    pub(crate) cleanup_previous_hits: usize,
    /// Cleanup requests that executed the context-free cleanup path.
    pub(crate) cleanup_cache_misses: usize,
    /// Final-boundary point decisions served by an exact local contributor match.
    pub(crate) final_boundary_cache_hits: usize,
    /// Final-boundary point decisions promoted from the immutable previous generation.
    pub(crate) final_boundary_previous_hits: usize,
    /// Final-boundary point decisions evaluated against current carrier authority.
    pub(crate) final_boundary_cache_misses: usize,
    /// Complete final-boundary assemblies served by an exact current or previous match.
    pub(crate) final_assembly_cache_hits: usize,
    /// Complete final-boundary assemblies promoted from the immutable previous generation.
    pub(crate) final_assembly_previous_hits: usize,
    /// Complete final-boundary assemblies rebuilt from current contributors.
    pub(crate) final_assembly_cache_misses: usize,
    /// Region seam extractions served by an exact shape and constraint match.
    pub(crate) seam_extraction_cache_hits: usize,
    /// Region seam extractions promoted from the immutable previous generation.
    pub(crate) seam_extraction_previous_hits: usize,
    /// Region seam extractions evaluated against current constraints.
    pub(crate) seam_extraction_cache_misses: usize,
    /// Owned-edge seam materializations served by exact local contributors.
    pub(crate) edge_seam_cache_hits: usize,
    /// Owned-edge seam materializations promoted from the immutable previous generation.
    pub(crate) edge_seam_previous_hits: usize,
    /// Owned-edge seam materializations evaluated against current local contributors.
    pub(crate) edge_seam_cache_misses: usize,
}

/// Mutable ownership reuse state that promotes only encountered previous-generation entries.
pub(super) struct NodeOwnershipBuildReuseContext<'a> {
    previous: Option<&'a NodeOwnershipIncrementalCache>,
    current: NodeOwnershipIncrementalCache,
    stats: NodeOwnershipReuseStats,
    all_constraint_keys: Box<[Arc<RailConstraintReuseKey>]>,
    constraint_keys_by_index: BTreeMap<usize, Arc<RailConstraintReuseKey>>,
    current_carrier_authorities:
        BTreeMap<CarrierSourceAuthorityLookupKey, Arc<CarrierSourceAuthorityReuseKey>>,
    tracking_final_assembly_contributors: bool,
    final_assembly_extracted_region_seam_keys: BTreeSet<RegionSeamExtractionReuseKey>,
    final_assembly_materialized_edge_seam_keys: BTreeSet<MaterializedOwnedEdgeSeamReuseKey>,
}

impl<'a> NodeOwnershipBuildReuseContext<'a> {
    pub(super) fn new(
        previous: Option<&'a NodeOwnershipIncrementalCache>,
        rail_constraints: &[NodeRailConstraint],
    ) -> Self {
        let mut constraint_keys_by_index = BTreeMap::new();
        let mut all_constraint_keys = Vec::with_capacity(rail_constraints.len());
        for constraint in rail_constraints {
            let key = Arc::new(RailConstraintReuseKey::from_constraint(constraint));
            constraint_keys_by_index.insert(constraint.constraint_index, Arc::clone(&key));
            all_constraint_keys.push(key);
        }
        Self {
            previous,
            current: NodeOwnershipIncrementalCache::default(),
            stats: NodeOwnershipReuseStats::default(),
            all_constraint_keys: all_constraint_keys.into_boxed_slice(),
            constraint_keys_by_index,
            current_carrier_authorities: BTreeMap::new(),
            tracking_final_assembly_contributors: false,
            final_assembly_extracted_region_seam_keys: BTreeSet::new(),
            final_assembly_materialized_edge_seam_keys: BTreeSet::new(),
        }
    }

    pub(super) fn finish(self) -> (NodeOwnershipIncrementalCache, NodeOwnershipReuseStats) {
        (self.current, self.stats)
    }

    pub(super) fn stats(&self) -> NodeOwnershipReuseStats {
        self.stats
    }

    /// Replays an exact final-boundary assembly or returns its already-built miss key.
    pub(super) fn cached_final_boundary_assembly(
        &mut self,
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        overlap_mode: ConstraintOverlapMode,
        regions: &mut [NodeBooleanOwnedRegion],
    ) -> FinalBoundaryAssemblyLookup {
        let key = self.final_boundary_assembly_key(node_id, piece_kind, overlap_mode, regions);
        if let Some(value) = self.current.final_boundary_assemblies.get(&key).cloned() {
            self.stats.final_assembly_cache_hits += 1;
            apply_cached_final_boundary_region_seams(regions, &value.region_seams);
            return FinalBoundaryAssemblyLookup::Hit(
                value.footprint_shapes.as_ref().to_vec(),
                value.arrangement.as_ref().clone(),
            );
        }
        if let Some(value) = self
            .previous
            .and_then(|previous| previous.final_boundary_assemblies.get(&key))
            .cloned()
        {
            self.stats.final_assembly_cache_hits += 1;
            self.stats.final_assembly_previous_hits += 1;
            self.current
                .final_boundary_assemblies
                .insert(key, value.clone());
            self.promote_previous_final_boundary_contributors(&value);
            apply_cached_final_boundary_region_seams(regions, &value.region_seams);
            return FinalBoundaryAssemblyLookup::Hit(
                value.footprint_shapes.as_ref().to_vec(),
                value.arrangement.as_ref().clone(),
            );
        }

        self.stats.final_assembly_cache_misses += 1;
        FinalBoundaryAssemblyLookup::Miss(key)
    }

    /// Starts recording only the fine-grained seam contributors used by one assembly rebuild.
    pub(super) fn begin_final_boundary_assembly_build(&mut self) {
        debug_assert!(!self.tracking_final_assembly_contributors);
        self.tracking_final_assembly_contributors = true;
        self.final_assembly_extracted_region_seam_keys.clear();
        self.final_assembly_materialized_edge_seam_keys.clear();
    }

    /// Stores one complete final-boundary result with its exact nested seam contributors.
    pub(super) fn store_final_boundary_assembly(
        &mut self,
        key: FinalBoundaryAssemblyReuseKey,
        regions: &[NodeBooleanOwnedRegion],
        footprint_shapes: &[NodeOverlayShape],
        arrangement: &NodeOwnedRegionArrangement,
    ) {
        debug_assert!(self.tracking_final_assembly_contributors);
        self.tracking_final_assembly_contributors = false;
        let region_seams: Arc<[Arc<[NodeRegionSeamConstraint]>]> = regions
            .iter()
            .map(|region| Arc::from(region.seam_constraints.clone()))
            .collect::<Vec<_>>()
            .into();
        self.current.final_boundary_assemblies.insert(
            key,
            FinalBoundaryAssemblyReuseValue {
                footprint_shapes: Arc::from(footprint_shapes.to_vec()),
                region_seams,
                arrangement: Arc::new(arrangement.clone()),
                extracted_region_seam_keys: Arc::from(
                    std::mem::take(&mut self.final_assembly_extracted_region_seam_keys)
                        .into_iter()
                        .collect::<Vec<_>>(),
                ),
                materialized_edge_seam_keys: Arc::from(
                    std::mem::take(&mut self.final_assembly_materialized_edge_seam_keys)
                        .into_iter()
                        .collect::<Vec<_>>(),
                ),
            },
        );
    }

    fn promote_previous_final_boundary_contributors(
        &mut self,
        assembly: &FinalBoundaryAssemblyReuseValue,
    ) {
        let Some(previous) = self.previous else {
            return;
        };
        for key in assembly.extracted_region_seam_keys.iter() {
            if self.current.extracted_region_seams.contains_key(key) {
                continue;
            }
            if let Some(seams) = previous.extracted_region_seams.get(key) {
                self.current
                    .extracted_region_seams
                    .insert(key.clone(), Arc::clone(seams));
            }
        }
        for key in assembly.materialized_edge_seam_keys.iter() {
            if self.current.materialized_edge_seams.contains_key(key) {
                continue;
            }
            if let Some(seams) = previous.materialized_edge_seams.get(key) {
                self.current
                    .materialized_edge_seams
                    .insert(key.clone(), Arc::clone(seams));
            }
        }
    }

    fn final_boundary_assembly_key(
        &self,
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        overlap_mode: ConstraintOverlapMode,
        regions: &[NodeBooleanOwnedRegion],
    ) -> FinalBoundaryAssemblyReuseKey {
        FinalBoundaryAssemblyReuseKey {
            node_id,
            piece_kind: piece_kind_key(piece_kind),
            grid_bounded_overlap: overlap_mode.allows_grid_bounded_constraint_overlap(),
            regions: regions
                .iter()
                .map(FinalBoundaryAssemblyRegionReuseKey::from_region)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            constraints: self.all_constraint_keys.iter().cloned().collect(),
        }
    }

    pub(super) fn cleaned_owned_shapes<E>(
        &mut self,
        shape: &NodeOverlayShape,
        clean_numeric_spikes: bool,
        build: impl FnOnce() -> Result<Vec<NodeOverlayShape>, E>,
    ) -> Result<Vec<NodeOverlayShape>, E> {
        let key = OwnedShapeCleanupKey::from_shape(shape, clean_numeric_spikes);
        if let Some(shapes) = self.current.cleaned_owned_shapes.get(&key) {
            self.stats.cleanup_cache_hits += 1;
            return Ok(shapes.as_ref().to_vec());
        }
        if let Some(shapes) = self
            .previous
            .and_then(|previous| previous.cleaned_owned_shapes.get(&key))
        {
            self.stats.cleanup_cache_hits += 1;
            self.stats.cleanup_previous_hits += 1;
            self.current
                .cleaned_owned_shapes
                .insert(key, Arc::clone(shapes));
            return Ok(shapes.as_ref().to_vec());
        }

        self.stats.cleanup_cache_misses += 1;
        let shapes = build()?;
        self.current
            .cleaned_owned_shapes
            .insert(key, Arc::from(shapes.clone()));
        Ok(shapes)
    }

    /// Reuses the pure final self-touch split independently from union cleanup.
    pub(super) fn final_self_touch_owned_shapes(
        &mut self,
        shape: &NodeOverlayShape,
        clean_numeric_spikes: bool,
        build: impl FnOnce() -> Vec<NodeOverlayShape>,
    ) -> Vec<NodeOverlayShape> {
        let key = OwnedShapeCleanupKey::from_final_self_touch(shape, clean_numeric_spikes);
        if let Some(shapes) = self.current.cleaned_owned_shapes.get(&key) {
            self.stats.cleanup_cache_hits += 1;
            return shapes.as_ref().to_vec();
        }
        if let Some(shapes) = self
            .previous
            .and_then(|previous| previous.cleaned_owned_shapes.get(&key))
        {
            self.stats.cleanup_cache_hits += 1;
            self.stats.cleanup_previous_hits += 1;
            self.current
                .cleaned_owned_shapes
                .insert(key, Arc::clone(shapes));
            return shapes.as_ref().to_vec();
        }

        self.stats.cleanup_cache_misses += 1;
        let shapes = build();
        self.current
            .cleaned_owned_shapes
            .insert(key, Arc::from(shapes.clone()));
        shapes
    }

    pub(super) fn extracted_region_seams(
        &mut self,
        region: &NodeBooleanOwnedRegion,
        overlap_mode: ConstraintOverlapMode,
        applicable_constraints: &[&NodeRailConstraint],
        build: impl FnOnce() -> Vec<NodeRegionSeamConstraint>,
    ) -> Vec<NodeRegionSeamConstraint> {
        let key = RegionSeamExtractionReuseKey {
            shape: CanonicalOwnedShapeKey::from_shape(&region.shape),
            owner: region.owner,
            grid_bounded_overlap: overlap_mode.allows_grid_bounded_constraint_overlap(),
            applicable_constraints: applicable_constraints
                .iter()
                .map(|constraint| {
                    self.constraint_keys_by_index
                        .get(&constraint.constraint_index)
                        .cloned()
                        .unwrap_or_else(|| {
                            Arc::new(RailConstraintReuseKey::from_constraint(constraint))
                        })
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        };
        if self.tracking_final_assembly_contributors {
            self.final_assembly_extracted_region_seam_keys
                .insert(key.clone());
        }
        if let Some(seams) = self.current.extracted_region_seams.get(&key) {
            self.stats.seam_extraction_cache_hits += 1;
            return seams.as_ref().to_vec();
        }
        if let Some(seams) = self
            .previous
            .and_then(|previous| previous.extracted_region_seams.get(&key))
        {
            self.stats.seam_extraction_cache_hits += 1;
            self.stats.seam_extraction_previous_hits += 1;
            self.current
                .extracted_region_seams
                .insert(key, Arc::clone(seams));
            return seams.as_ref().to_vec();
        }

        self.stats.seam_extraction_cache_misses += 1;
        let seams = build();
        self.current
            .extracted_region_seams
            .insert(key, Arc::from(seams.clone()));
        seams
    }

    pub(super) fn materialized_owned_edge_seams<'constraint>(
        &mut self,
        start: NodeOwnershipPointKey,
        end: NodeOwnershipPointKey,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        source_seams: &[&NodeRegionSeamConstraint],
        local_constraints: impl IntoIterator<Item = &'constraint NodeRailConstraint>,
        build: impl FnOnce() -> Vec<NodeRegionSeamConstraint>,
    ) -> Vec<NodeRegionSeamConstraint> {
        let key = MaterializedOwnedEdgeSeamReuseKey {
            start,
            end,
            owner,
            opposite_owner,
            piece_kind: piece_kind_key(piece_kind),
            source_seams: source_seams
                .iter()
                .map(|constraint| RegionSeamConstraintReuseKey::from_constraint(constraint))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            local_constraints: local_constraints
                .into_iter()
                .map(|constraint| {
                    self.constraint_keys_by_index
                        .get(&constraint.constraint_index)
                        .cloned()
                        .unwrap_or_else(|| {
                            Arc::new(RailConstraintReuseKey::from_constraint(constraint))
                        })
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        };
        if self.tracking_final_assembly_contributors {
            self.final_assembly_materialized_edge_seam_keys
                .insert(key.clone());
        }
        if let Some(seams) = self.current.materialized_edge_seams.get(&key) {
            self.stats.edge_seam_cache_hits += 1;
            return seams.as_ref().to_vec();
        }
        if let Some(seams) = self
            .previous
            .and_then(|previous| previous.materialized_edge_seams.get(&key))
        {
            self.stats.edge_seam_cache_hits += 1;
            self.stats.edge_seam_previous_hits += 1;
            self.current
                .materialized_edge_seams
                .insert(key, Arc::clone(seams));
            return seams.as_ref().to_vec();
        }

        self.stats.edge_seam_cache_misses += 1;
        let seams = build();
        self.current
            .materialized_edge_seams
            .insert(key, Arc::from(seams.clone()));
        seams
    }

    pub(super) fn final_boundary_point_is_supported<E>(
        &mut self,
        region: &NodeBooleanOwnedRegion,
        point: NodeOwnershipPointKey,
        rails: &NodeRailContourSet,
        build: impl FnOnce() -> Result<bool, E>,
    ) -> Result<bool, E> {
        let carrier_authority = region.source_band_index.map(|source_band_index| {
            let lookup = CarrierSourceAuthorityLookupKey {
                owner: region.owner,
                kind: region.kind,
                source_mouth_order_index: region.source_mouth_order_index,
                source_band_index,
            };
            if let Some(authority) = self.current_carrier_authorities.get(&lookup) {
                return Arc::clone(authority);
            }
            let authority = Arc::new(
                CarrierSourceAuthorityReuseKey::for_region(
                    region,
                    rails,
                    &self.constraint_keys_by_index,
                )
                .expect("source-band lookup requires carrier authority"),
            );
            self.current_carrier_authorities
                .insert(lookup, Arc::clone(&authority));
            authority
        });
        let key = FinalBoundaryPointReuseKey {
            kind: region.kind,
            owner: region.owner,
            claim_priority: region.claim_priority,
            source_mouth_order_index: region.source_mouth_order_index,
            source_band_index: region.source_band_index,
            point,
            carrier_authority,
        };
        if let Some(supported) = self.current.final_boundary_points.get(&key) {
            self.stats.final_boundary_cache_hits += 1;
            return Ok(*supported);
        }
        if let Some(supported) = self
            .previous
            .and_then(|previous| previous.final_boundary_points.get(&key))
        {
            self.stats.final_boundary_cache_hits += 1;
            self.stats.final_boundary_previous_hits += 1;
            self.current.final_boundary_points.insert(key, *supported);
            return Ok(*supported);
        }

        self.stats.final_boundary_cache_misses += 1;
        let supported = build()?;
        self.current.final_boundary_points.insert(key, supported);
        Ok(supported)
    }
}

fn apply_cached_final_boundary_region_seams(
    regions: &mut [NodeBooleanOwnedRegion],
    region_seams: &[Arc<[NodeRegionSeamConstraint]>],
) {
    debug_assert_eq!(regions.len(), region_seams.len());
    for (region, seams) in regions.iter_mut().zip(region_seams) {
        region.seam_constraints = seams.as_ref().to_vec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x_offset: f64) -> NodeOverlayShape {
        vec![vec![
            [x_offset, 0.0],
            [x_offset + 1.0, 0.0],
            [x_offset + 1.0, 1.0],
            [x_offset, 1.0],
        ]]
    }

    fn source_region() -> NodeBooleanOwnedRegion {
        NodeBooleanOwnedRegion {
            kind: RoadSurfaceBandKind::Sidewalk,
            owner: NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3),
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            source_mouth_order_index: 1,
            source_band_index: Some(2),
            shape: square(0.0),
            area_m2: 1.0,
            seam_constraints: Vec::new(),
        }
    }

    fn empty_rails() -> NodeRailContourSet {
        NodeRailContourSet {
            node_id: 7,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            contours: Vec::new(),
            corner_trims: Vec::new(),
            side_join_gaps: Vec::new(),
            constraints: Vec::new(),
            height_carrier_paths_by_source: BTreeMap::new(),
            height_carrier_points_by_source: BTreeMap::new(),
            source_carriers: Default::default(),
        }
    }

    #[test]
    fn cleanup_cache_promotes_only_exact_encountered_shapes() {
        let shape = square(0.0);
        let cached_output = vec![square(10.0), square(20.0)];
        let mut first = NodeOwnershipBuildReuseContext::new(None, &[]);
        assert_eq!(
            first
                .cleaned_owned_shapes(&shape, false, || { Ok::<_, ()>(cached_output.clone()) })
                .expect("initial cleanup"),
            cached_output
        );
        let (previous, first_stats) = first.finish();
        assert_eq!(first_stats.cleanup_cache_hits, 0);
        assert_eq!(first_stats.cleanup_cache_misses, 1);

        let mut second = NodeOwnershipBuildReuseContext::new(Some(&previous), &[]);
        assert_eq!(
            second
                .cleaned_owned_shapes(&shape, false, || -> Result<Vec<NodeOverlayShape>, ()> {
                    panic!("exact previous-generation hit must not rebuild")
                },)
                .expect("reused cleanup"),
            cached_output
        );

        // Sub-grid drift can still change raw area and emitted seam coordinates, so it is not an
        // exact contributor match even when the canonical topology key is unchanged.
        let quantized_equal_shape = square(0.000_000_1);
        second
            .cleaned_owned_shapes(&quantized_equal_shape, false, || {
                Ok::<_, ()>(vec![square(25.0)])
            })
            .expect("raw-coordinate-specific cleanup");

        // The cleanup mode is semantic input and must select a different entry.
        second
            .cleaned_owned_shapes(&quantized_equal_shape, true, || {
                Ok::<_, ()>(vec![square(30.0)])
            })
            .expect("mode-specific cleanup");
        let mut reordered_shape = quantized_equal_shape.clone();
        reordered_shape[0].rotate_left(1);
        second
            .cleaned_owned_shapes(&reordered_shape, false, || Ok::<_, ()>(vec![square(40.0)]))
            .expect("order-specific cleanup");
        let (current, second_stats) = second.finish();
        assert_eq!(second_stats.cleanup_cache_hits, 1);
        assert_eq!(second_stats.cleanup_previous_hits, 1);
        assert_eq!(second_stats.cleanup_cache_misses, 3);
        assert_eq!(current.cleaned_owned_shapes.len(), 4);
    }

    #[test]
    fn cleanup_cache_drops_removed_contributors_and_reuses_unchanged_ones() {
        let unchanged = square(0.0);
        let removed = square(10.0);
        let changed = square(20.0);

        let mut first = NodeOwnershipBuildReuseContext::new(None, &[]);
        first
            .cleaned_owned_shapes(&unchanged, false, || Ok::<_, ()>(vec![unchanged.clone()]))
            .expect("seed unchanged contributor");
        first
            .cleaned_owned_shapes(&removed, false, || Ok::<_, ()>(vec![removed.clone()]))
            .expect("seed removable contributor");
        let (previous, _) = first.finish();

        let mut second = NodeOwnershipBuildReuseContext::new(Some(&previous), &[]);
        second
            .cleaned_owned_shapes(&unchanged, false, || -> Result<_, ()> {
                panic!("unchanged contributor must reuse the previous generation")
            })
            .expect("reuse unchanged contributor");
        second
            .cleaned_owned_shapes(&changed, false, || Ok::<_, ()>(vec![changed.clone()]))
            .expect("build changed contributor");
        let (current, stats) = second.finish();

        assert_eq!(stats.cleanup_previous_hits, 1);
        assert_eq!(stats.cleanup_cache_misses, 1);
        assert_eq!(current.cleaned_owned_shapes.len(), 2);
        assert!(
            current
                .cleaned_owned_shapes
                .contains_key(&OwnedShapeCleanupKey::from_shape(&unchanged, false))
        );
        assert!(
            !current
                .cleaned_owned_shapes
                .contains_key(&OwnedShapeCleanupKey::from_shape(&removed, false))
        );
    }

    #[test]
    fn cleanup_cache_keeps_final_self_touch_split_separate_from_union_cleanup() {
        let shape = square(0.0);
        let union_output = vec![square(10.0)];
        let split_output = vec![square(20.0)];
        let mut first = NodeOwnershipBuildReuseContext::new(None, &[]);
        assert_eq!(
            first
                .cleaned_owned_shapes(&shape, false, || Ok::<_, ()>(union_output.clone()))
                .expect("union cleanup"),
            union_output
        );
        assert_eq!(
            first.final_self_touch_owned_shapes(&shape, false, || split_output.clone()),
            split_output
        );
        let (previous, first_stats) = first.finish();
        assert_eq!(first_stats.cleanup_cache_misses, 2);
        assert_eq!(previous.cleaned_owned_shapes.len(), 2);

        let mut second = NodeOwnershipBuildReuseContext::new(Some(&previous), &[]);
        second
            .cleaned_owned_shapes(&shape, false, || -> Result<_, ()> {
                panic!("union cleanup must retain its own previous-generation entry")
            })
            .expect("reused union cleanup");
        second.final_self_touch_owned_shapes(&shape, false, || {
            panic!("final self-touch split must retain its own previous-generation entry")
        });
        let (_, second_stats) = second.finish();
        assert_eq!(second_stats.cleanup_previous_hits, 2);
        assert_eq!(second_stats.cleanup_cache_misses, 0);
    }

    #[test]
    fn final_assembly_hit_preserves_nested_seam_contributors_for_next_generation() {
        let mut region = source_region();
        let owner = region.owner;
        let opposite_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let mut unrelated_region = source_region();
        unrelated_region.owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 99);
        unrelated_region.shape = square(100.0);
        let mut first = NodeOwnershipBuildReuseContext::new(None, &[]);
        first.extracted_region_seams(
            &unrelated_region,
            ConstraintOverlapMode::GridBounded,
            &[],
            Vec::new,
        );
        first.materialized_owned_edge_seams(
            (100, 0),
            (101, 0),
            unrelated_region.owner,
            opposite_owner,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[],
            std::iter::empty(),
            Vec::new,
        );
        let cache_key = match first.cached_final_boundary_assembly(
            7,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            ConstraintOverlapMode::GridBounded,
            std::slice::from_mut(&mut region),
        ) {
            FinalBoundaryAssemblyLookup::Hit(..) => {
                panic!("an empty cache must miss its first final assembly")
            }
            FinalBoundaryAssemblyLookup::Miss(cache_key) => cache_key,
        };
        first.begin_final_boundary_assembly_build();
        first.extracted_region_seams(&region, ConstraintOverlapMode::GridBounded, &[], Vec::new);
        first.materialized_owned_edge_seams(
            (0, 0),
            (1, 0),
            owner,
            opposite_owner,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[],
            std::iter::empty(),
            Vec::new,
        );
        first.store_final_boundary_assembly(
            cache_key,
            std::slice::from_ref(&region),
            &[],
            &NodeOwnedRegionArrangement {
                node_id: 7,
                piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
                region_count: 1,
                edges: Vec::new(),
                diagnostics: Vec::new(),
            },
        );
        let (previous, _) = first.finish();
        assert_eq!(previous.extracted_region_seams.len(), 2);
        assert_eq!(previous.materialized_edge_seams.len(), 2);

        let mut second = NodeOwnershipBuildReuseContext::new(Some(&previous), &[]);
        assert!(matches!(
            second.cached_final_boundary_assembly(
                7,
                RoadSurfaceVisualNodePieceKind::JunctionN,
                ConstraintOverlapMode::GridBounded,
                std::slice::from_mut(&mut region),
            ),
            FinalBoundaryAssemblyLookup::Hit(..)
        ));
        let (current, stats) = second.finish();

        assert_eq!(stats.final_assembly_previous_hits, 1);
        assert_eq!(current.extracted_region_seams.len(), 1);
        assert!(
            current
                .extracted_region_seams
                .keys()
                .all(|key| key.owner == owner)
        );
        assert_eq!(current.materialized_edge_seams.len(), 1);
        assert!(
            current
                .materialized_edge_seams
                .keys()
                .all(|key| key.owner == owner && key.start == (0, 0))
        );
    }

    #[test]
    fn final_boundary_authority_key_is_height_sensitive_and_source_local() {
        let region = source_region();
        let source = (
            region.kind,
            region.source_mouth_order_index,
            region.source_band_index.expect("source band"),
        );
        let mut rails = empty_rails();
        rails
            .height_carrier_points_by_source
            .insert(source, vec![RoadVec3::new(1.0, 2.0, 3.0)]);
        let constraint_keys = BTreeMap::new();
        let baseline =
            CarrierSourceAuthorityReuseKey::for_region(&region, &rails, &constraint_keys)
                .expect("authority");

        let mut changed_height = rails.clone();
        changed_height
            .height_carrier_points_by_source
            .get_mut(&source)
            .expect("source points")[0]
            .y = 2.001;
        assert_ne!(
            CarrierSourceAuthorityReuseKey::for_region(&region, &changed_height, &constraint_keys,)
                .expect("changed authority"),
            baseline
        );

        let mut unrelated_source = rails;
        unrelated_source.height_carrier_points_by_source.insert(
            (RoadSurfaceBandKind::Carriageway, 9, 4),
            vec![RoadVec3::new(5.0, 99.0, 6.0)],
        );
        assert_eq!(
            CarrierSourceAuthorityReuseKey::for_region(
                &region,
                &unrelated_source,
                &constraint_keys,
            )
            .expect("source-local authority"),
            baseline
        );
    }
}
