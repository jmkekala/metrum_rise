// SPDX-License-Identifier: GPL-2.0-only

//! Owned-region ring canonicalization entry points.

use super::*;
use crate::simulation::network::surface::RoadSurfaceBandKind;
use crate::simulation::network::surface::node::NodeHashMap;
use crate::simulation::network::surface::{
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, keys::SURFACE_XZ_KEY_SCALE,
};
use std::collections::{BTreeMap, btree_map::Entry};

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_owned_region_rings(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    let global_point_index = NodeOwnershipPointIndex::new(&global_points);
    for region in regions.iter_mut() {
        for contour in &mut region.shape {
            *contour = noded_owned_region_contour_with_point_index(contour, &global_point_index);
        }
    }
}

#[cfg(test)]
pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_final_owned_region_boundary_edges(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_canonical_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_final_owned_region_boundary_edges_with_options(
        regions,
        footprint_shapes,
        &mut NodeRingCanonicalization::new(rail_canonical_points),
        SourceCarrierKeyPolicy::none(),
    )
}

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_final_owned_region_boundary_edges_for_piece_kind(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    prepared: &mut NodeRingCanonicalization<'_>,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_final_owned_region_boundary_edges_with_options(
        regions,
        footprint_shapes,
        prepared,
        SourceCarrierKeyPolicy::for_piece_kind(piece_kind),
    )
}

fn canonicalize_final_owned_region_boundary_edges_with_options(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    prepared: &mut NodeRingCanonicalization<'_>,
    source_carrier_key_policy: SourceCarrierKeyPolicy,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_owned_region_rings_with_rail_point_set_with_options(
        regions,
        prepared,
        source_carrier_key_policy,
    )?;
    node_owned_region_rings_to_global_points(regions, footprint_shapes);
    canonicalize_owned_region_rings_with_rail_point_set_with_options(
        regions,
        prepared,
        source_carrier_key_policy,
    )?;
    Ok(())
}

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_final_join_or_cap_owned_region_boundary_edges(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    prepared: &mut NodeRingCanonicalization<'_>,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_join_or_cap_owned_region_rings_with_rail_point_set(regions, prepared)?;
    node_join_or_cap_owned_region_rings_to_global_points(regions, footprint_shapes);
    canonicalize_join_or_cap_owned_region_rings_with_rail_point_set(regions, prepared)?;
    Ok(())
}

fn node_owned_region_rings_to_global_points(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    let global_point_index = NodeOwnershipPointIndex::new(&global_points);
    for region in regions {
        for contour in &mut region.shape {
            *contour = noded_owned_region_contour_with_point_index(contour, &global_point_index);
        }
    }
}

fn node_join_or_cap_owned_region_rings_to_global_points(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    let global_point_index = NodeOwnershipPointIndex::new(&global_points);
    for region in regions {
        if region.claim_priority != NodeGeneratedContourClaimPriority::JoinOrCap {
            continue;
        }
        for contour in &mut region.shape {
            *contour = noded_owned_region_contour_with_point_index(contour, &global_point_index);
        }
    }
}

#[cfg(test)]
pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_owned_region_rings_with_rail_point_set(
    regions: &mut [NodeBooleanOwnedRegion],
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_owned_region_rings_with_rail_point_set_with_options(
        regions,
        &mut NodeRingCanonicalization::new(rail_points),
        SourceCarrierKeyPolicy::none(),
    )
}

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_owned_region_rings_with_rail_point_set_for_piece_kind(
    regions: &mut [NodeBooleanOwnedRegion],
    prepared: &mut NodeRingCanonicalization<'_>,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_owned_region_rings_with_rail_point_set_with_options(
        regions,
        prepared,
        SourceCarrierKeyPolicy::for_piece_kind(piece_kind),
    )
}

fn canonicalize_owned_region_rings_with_rail_point_set_with_options(
    regions: &mut [NodeBooleanOwnedRegion],
    prepared: &mut NodeRingCanonicalization<'_>,
    source_carrier_key_policy: SourceCarrierKeyPolicy,
) -> Result<(), NodeBooleanOwnershipError> {
    if prepared.rail_points.all_points.is_empty() {
        return Ok(());
    }

    for region in regions {
        prepared.canonicalize(region, source_carrier_key_policy)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
struct SourceCarrierKeyPolicy {
    allow_key_adoption: bool,
    canonicalize_source_height_numeric_dust: bool,
}

impl SourceCarrierKeyPolicy {
    fn none() -> Self {
        Self {
            allow_key_adoption: false,
            canonicalize_source_height_numeric_dust: false,
        }
    }

    fn for_piece_kind(piece_kind: RoadSurfaceVisualNodePieceKind) -> Self {
        match piece_kind {
            RoadSurfaceVisualNodePieceKind::Terminal => Self {
                allow_key_adoption: true,
                canonicalize_source_height_numeric_dust: false,
            },
            RoadSurfaceVisualNodePieceKind::JunctionN => Self {
                allow_key_adoption: true,
                canonicalize_source_height_numeric_dust: true,
            },
            RoadSurfaceVisualNodePieceKind::Bend => Self::none(),
        }
    }
}

fn canonicalize_join_or_cap_owned_region_rings_with_rail_point_set(
    regions: &mut [NodeBooleanOwnedRegion],
    prepared: &mut NodeRingCanonicalization<'_>,
) -> Result<(), NodeBooleanOwnershipError> {
    if prepared.rail_points.all_points.is_empty() {
        return Ok(());
    }

    for region in regions {
        if region.claim_priority != NodeGeneratedContourClaimPriority::JoinOrCap {
            continue;
        }
        prepared.canonicalize(region, SourceCarrierKeyPolicy::none())?;
    }
    Ok(())
}

/// Source lookup data shared only across cleanup passes using the same immutable rail set.
pub(in crate::simulation::network::surface::node::ownership) struct NodeRingCanonicalization<'a> {
    rail_points: &'a NodeRailCanonicalPointSet,
    sources: BTreeMap<RegionSourceKey, PreparedRegionSource<'a>>,
}

#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct RegionSourceKey {
    owner: NodeBandOwner,
    kind: RoadSurfaceBandKind,
    mouth: usize,
    band: Option<usize>,
    join_or_cap: bool,
    policy: SourceCarrierKeyPolicy,
}

struct PreparedRegionSource<'a> {
    preserved_points: Vec<NodeOwnershipPointKey>,
    preserved_points_by_mm: Option<NodeHashMap<NodeOwnershipPointKey, Vec<NodeOwnershipPointKey>>>,
    has_source_carrier: bool,
    uses_generated_join_or_cap: bool,
    allow_source_carrier_key_adoption: bool,
    canonicalize_source_height_numeric_dust: bool,
    source_point_index: NodeOwnershipPointIndex,
    prepared_owner_paths: PreparedRailPaths<'a>,
}

impl<'a> NodeRingCanonicalization<'a> {
    /// Borrows one rail set so preparation cannot outlive or observe changes to its authority.
    pub(in crate::simulation::network::surface::node::ownership) fn new(
        rail_points: &'a NodeRailCanonicalPointSet,
    ) -> Self {
        Self {
            rail_points,
            sources: BTreeMap::new(),
        }
    }

    fn canonicalize(
        &mut self,
        region: &mut NodeBooleanOwnedRegion,
        policy: SourceCarrierKeyPolicy,
    ) -> Result<(), NodeBooleanOwnershipError> {
        let key = RegionSourceKey {
            owner: region.owner,
            kind: region.kind,
            mouth: region.source_mouth_order_index,
            band: region.source_band_index,
            join_or_cap: region.claim_priority == NodeGeneratedContourClaimPriority::JoinOrCap,
            policy,
        };
        let source = match self.sources.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                entry.insert(PreparedRegionSource::new(region, self.rail_points, policy)?)
            }
        };
        for contour in &mut region.shape {
            canonicalize_owned_region_contour_to_owner_source_points(
                contour,
                region.owner,
                &source.preserved_points,
                source.has_source_carrier,
                source.uses_generated_join_or_cap,
                source.allow_source_carrier_key_adoption,
                source.canonicalize_source_height_numeric_dust,
                source.preserved_points_by_mm.as_ref(),
                self.rail_points,
            )?;
            *contour = noded_owned_region_contour_with_rail_paths_and_point_index(
                contour,
                &source.source_point_index,
                &source.prepared_owner_paths,
                region.claim_priority == NodeGeneratedContourClaimPriority::JoinOrCap,
            );
        }
        Ok(())
    }
}

impl<'a> PreparedRegionSource<'a> {
    fn new(
        region: &NodeBooleanOwnedRegion,
        rail_points: &'a NodeRailCanonicalPointSet,
        source_carrier_key_policy: SourceCarrierKeyPolicy,
    ) -> Result<Self, NodeBooleanOwnershipError> {
        let owner_points = rail_points
            .points_by_owner
            .get(&region.owner)
            .map(Vec::as_slice)
            .unwrap_or(&rail_points.all_points);
        let source_height_points = region.source_band_index.and_then(|source_band_index| {
            rail_points.source_carriers.height_points((
                region.kind,
                region.source_mouth_order_index,
                source_band_index,
            ))
        });
        let has_source_carrier = region.source_band_index.is_some_and(|source_band_index| {
            rail_points.source_carriers.has_source_carrier(
                region.owner,
                (
                    region.kind,
                    region.source_mouth_order_index,
                    source_band_index,
                ),
            )
        });
        let source_key = region.source_band_index.map(|source_band_index| {
            (
                region.kind,
                region.source_mouth_order_index,
                source_band_index,
            )
        });
        let source_uses_numeric_dust_carrier_canonicalization = source_key.is_some_and(|source| {
            rail_points
                .source_carriers
                .uses_numeric_dust_carrier_canonicalization(source)
        });
        let canonicalize_source_height_numeric_dust = source_carrier_key_policy
            .canonicalize_source_height_numeric_dust
            && source_uses_numeric_dust_carrier_canonicalization;
        let allow_source_carrier_key_adoption = source_carrier_key_policy.allow_key_adoption
            && (!source_carrier_key_policy.canonicalize_source_height_numeric_dust
                || source_uses_numeric_dust_carrier_canonicalization);
        let mut preserved_points = source_height_points.cloned().unwrap_or_default();
        if canonicalize_source_height_numeric_dust {
            preserved_points = canonical_source_height_numeric_dust_points(preserved_points);
        } else {
            preserved_points.sort_unstable();
            preserved_points.dedup();
        }
        let preserved_points_by_mm = canonicalize_source_height_numeric_dust.then(|| {
            let mut points_by_mm =
                NodeHashMap::<NodeOwnershipPointKey, Vec<NodeOwnershipPointKey>>::default();
            for &point in &preserved_points {
                points_by_mm
                    .entry(ownership_mm_key(point))
                    .or_default()
                    .push(point);
            }
            points_by_mm
        });
        let authority_points = if let Some(source_height_points) = source_height_points {
            source_height_points.as_slice()
        } else if has_source_carrier {
            &[]
        } else {
            owner_points
        };
        let mut source_points = preserved_points.clone();
        for point in authority_points.iter().copied() {
            if let Some(point) = region_noding_point_for_owner_source(
                region.owner,
                &preserved_points,
                preserved_points_by_mm.as_ref(),
                point,
                rail_points,
                canonicalize_source_height_numeric_dust,
            )? {
                source_points.push(point);
            }
        }
        let uses_generated_join_or_cap =
            region.claim_priority == NodeGeneratedContourClaimPriority::JoinOrCap;
        if !has_source_carrier || uses_generated_join_or_cap {
            for point in rail_points.all_points.iter().copied() {
                if let Some(point) = region_noding_point_for_owner_source(
                    region.owner,
                    &preserved_points,
                    preserved_points_by_mm.as_ref(),
                    point,
                    rail_points,
                    canonicalize_source_height_numeric_dust,
                )? {
                    source_points.push(point);
                }
            }
        }
        source_points.sort_unstable();
        source_points.dedup();
        let source_point_index = NodeOwnershipPointIndex::new(&source_points);
        let owner_paths = if region.claim_priority == NodeGeneratedContourClaimPriority::JoinOrCap {
            rail_points
                .paths_by_owner
                .get(&region.owner)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        } else {
            &[]
        };
        let prepared_owner_paths = PreparedRailPaths::new(owner_paths);
        Ok(Self {
            preserved_points,
            preserved_points_by_mm,
            has_source_carrier,
            uses_generated_join_or_cap,
            allow_source_carrier_key_adoption,
            canonicalize_source_height_numeric_dust,
            source_point_index,
            prepared_owner_paths,
        })
    }
}

fn region_noding_point_for_owner_source(
    owner: NodeBandOwner,
    preserved_source_points: &[NodeOwnershipPointKey],
    preserved_source_points_by_mm: Option<
        &NodeHashMap<NodeOwnershipPointKey, Vec<NodeOwnershipPointKey>>,
    >,
    point: NodeOwnershipPointKey,
    rail_points: &NodeRailCanonicalPointSet,
    canonicalize_source_height_numeric_dust: bool,
) -> Result<Option<NodeOwnershipPointKey>, NodeBooleanOwnershipError> {
    if preserved_source_points.binary_search(&point).is_ok() {
        return Ok(Some(point));
    }
    if canonicalize_source_height_numeric_dust
        && let Some(points_by_mm) = preserved_source_points_by_mm
        && let Some(point) = unique_preserved_source_numeric_dust_point(points_by_mm, point)
    {
        return Ok(Some(point));
    }
    match rail_points.canonicalized_point_for_owner(owner, point) {
        Ok(canonical) => Ok(Some(canonical)),
        Err(NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn canonical_source_height_numeric_dust_points(
    mut points: Vec<NodeOwnershipPointKey>,
) -> Vec<NodeOwnershipPointKey> {
    points.sort_unstable();
    points.dedup();
    let mut canonical = Vec::with_capacity(points.len());
    let mut indices_by_mm = BTreeMap::<NodeOwnershipPointKey, Vec<usize>>::new();
    for point in points {
        let point_mm = ownership_mm_key(point);
        if indices_by_mm.get(&point_mm).is_some_and(|indices| {
            indices
                .iter()
                .copied()
                .any(|index| source_points_are_numeric_dust_duplicates(canonical[index], point))
        }) {
            continue;
        }
        let index = canonical.len();
        canonical.push(point);
        indices_by_mm.entry(point_mm).or_default().push(index);
    }
    canonical
}

fn unique_preserved_source_numeric_dust_point(
    preserved_source_points_by_mm: &NodeHashMap<NodeOwnershipPointKey, Vec<NodeOwnershipPointKey>>,
    point: NodeOwnershipPointKey,
) -> Option<NodeOwnershipPointKey> {
    let point_mm = ownership_mm_key(point);
    let mut candidates = preserved_source_points_by_mm
        .get(&point_mm)?
        .iter()
        .copied()
        .filter(|candidate| source_points_are_numeric_dust_duplicates(*candidate, point));
    let first = candidates.next()?;
    candidates.next().is_none().then_some(first)
}

fn source_points_are_numeric_dust_duplicates(
    first: NodeOwnershipPointKey,
    second: NodeOwnershipPointKey,
) -> bool {
    let dx = i128::from(first.0 - second.0);
    let dz = i128::from(first.1 - second.1);
    let dust = i128::from(source_numeric_dust_key_units());
    dx * dx + dz * dz <= dust * dust
}

fn source_numeric_dust_key_units() -> i64 {
    (f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * SURFACE_XZ_KEY_SCALE).round() as i64
}

fn canonicalize_owned_region_contour_to_owner_source_points(
    contour: &mut NodeOverlayContour,
    owner: NodeBandOwner,
    source_points: &[NodeOwnershipPointKey],
    has_source_carrier: bool,
    uses_generated_join_or_cap: bool,
    allow_source_carrier_key_adoption: bool,
    canonicalize_source_height_numeric_dust: bool,
    preserved_source_points_by_mm: Option<
        &NodeHashMap<NodeOwnershipPointKey, Vec<NodeOwnershipPointKey>>,
    >,
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    for point in contour.iter_mut() {
        let key = ownership_key_from_overlay_point(*point);
        if source_points.binary_search(&key).is_ok() {
            continue;
        }
        if has_source_carrier {
            if (uses_generated_join_or_cap || allow_source_carrier_key_adoption)
                && let Some(canonical) = region_noding_point_for_owner_source(
                    owner,
                    source_points,
                    preserved_source_points_by_mm,
                    key,
                    rail_points,
                    canonicalize_source_height_numeric_dust,
                )?
                && canonical != key
            {
                *point = overlay_point_from_key(canonical);
            }
            continue;
        }
        let canonical = match rail_points.canonicalized_point_for_owner(owner, key) {
            Ok(canonical) => canonical,
            Err(error) => return Err(error),
        };
        if canonical == key {
            continue;
        }
        *point = overlay_point_from_key(canonical);
    }
    dedup_consecutive_overlay_points(contour);
    if contour.len() >= 2
        && ownership_key_from_overlay_point(contour[0])
            == ownership_key_from_overlay_point(*contour.last().expect("contour has last"))
    {
        contour.pop();
    }
    Ok(())
}

pub(in crate::simulation::network::surface::node::ownership) fn owned_region_global_points(
    regions: &[NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) -> Vec<NodeOwnershipPointKey> {
    let mut global_points = regions
        .iter()
        .flat_map(|region| region.shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .map(ownership_key_from_overlay_point)
        .chain(
            footprint_shapes
                .iter()
                .flat_map(|shape| shape.iter())
                .flat_map(|contour| contour.iter().copied())
                .map(ownership_key_from_overlay_point),
        )
        .collect::<Vec<_>>();
    global_points.sort_unstable();
    global_points.dedup();
    global_points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::node::ownership::rail_authority::{
        NodeSourceCarrierRegistry, canonical_points_by_mm_key_by_owner,
    };

    #[test]
    fn shared_preparation_preserves_source_and_policy_boundaries() {
        let kind = RoadSurfaceBandKind::Carriageway;
        let owner = NodeBandOwner::new(kind, 0);
        let other_owner = NodeBandOwner::new(kind, 1);
        let points_by_owner = BTreeMap::from([
            (owner, vec![(1_000_000, 0)]),
            (other_owner, vec![(1_000_100, 0)]),
        ]);
        let rails = NodeRailCanonicalPointSet {
            all_points: vec![(1_000_000, 0), (1_000_100, 0)],
            canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(
                &points_by_owner,
            ),
            points_by_owner,
            paths_by_owner: BTreeMap::new(),
            source_carriers: NodeSourceCarrierRegistry {
                height_points_by_source: BTreeMap::from([
                    ((kind, 0, 0), vec![(1_000_001, 0), (1_000_002, 0)]),
                    ((kind, 0, 1), vec![(1_000_010, 0)]),
                    ((kind, 1, 0), vec![(1_000_020, 0)]),
                    ((RoadSurfaceBandKind::Sidewalk, 0, 0), vec![(1_000_030, 0)]),
                ]),
                numeric_dust_canonicalized_sources: [(kind, 0, 0)].into(),
                ..Default::default()
            },
        };
        let none = SourceCarrierKeyPolicy::none();
        let terminal =
            SourceCarrierKeyPolicy::for_piece_kind(RoadSurfaceVisualNodePieceKind::Terminal);
        let junction =
            SourceCarrierKeyPolicy::for_piece_kind(RoadSurfaceVisualNodePieceKind::JunctionN);
        let cases = [
            (
                owner,
                kind,
                0,
                Some(0),
                false,
                terminal,
                1_000_001,
                1_000_001,
            ),
            (
                owner,
                kind,
                0,
                Some(1),
                false,
                terminal,
                1_000_001,
                1_000_000,
            ),
            (
                owner,
                kind,
                1,
                Some(0),
                false,
                terminal,
                1_000_001,
                1_000_000,
            ),
            (
                owner,
                RoadSurfaceBandKind::Sidewalk,
                0,
                Some(0),
                false,
                terminal,
                1_000_001,
                1_000_000,
            ),
            (
                other_owner,
                kind,
                0,
                Some(0),
                false,
                terminal,
                1_000_003,
                1_000_100,
            ),
            (owner, kind, 0, None, false, terminal, 1_000_001, 1_000_000),
            (owner, kind, 0, Some(0), false, none, 1_000_003, 1_000_003),
            (owner, kind, 0, Some(0), true, none, 1_000_003, 1_000_000),
            (
                owner,
                kind,
                0,
                Some(0),
                false,
                terminal,
                1_000_003,
                1_000_000,
            ),
            (
                owner,
                kind,
                0,
                Some(0),
                false,
                junction,
                1_000_002,
                1_000_001,
            ),
            (
                owner,
                kind,
                0,
                Some(0),
                false,
                terminal,
                1_000_002,
                1_000_002,
            ),
        ];
        let mut prepared = NodeRingCanonicalization::new(&rails);
        // Revisit each authority with new contour coordinates. Only source preparation is reusable.
        for (index, &(owner, kind, mouth, band, cap, policy, input, expected)) in
            cases.iter().chain(cases.iter().rev()).enumerate()
        {
            let tail = [0.0, 1.0 + index as f64];
            let mut region = NodeBooleanOwnedRegion {
                owner,
                kind,
                claim_priority: if cap {
                    NodeGeneratedContourClaimPriority::JoinOrCap
                } else {
                    NodeGeneratedContourClaimPriority::MouthBand
                },
                source_mouth_order_index: mouth,
                source_band_index: band,
                shape: vec![vec![overlay_point_from_key((input, 0)), [0.0, 0.0], tail]],
                area_m2: 0.5,
                seam_constraints: Vec::new(),
            };
            let mut fresh = region.clone();
            NodeRingCanonicalization::new(&rails)
                .canonicalize(&mut fresh, policy)
                .unwrap();
            prepared.canonicalize(&mut region, policy).unwrap();
            assert_eq!(region, fresh);
            assert_eq!(
                ownership_key_from_overlay_point(region.shape[0][0]),
                (expected, 0)
            );
            assert!(region.shape[0].contains(&tail));
        }
    }
}
