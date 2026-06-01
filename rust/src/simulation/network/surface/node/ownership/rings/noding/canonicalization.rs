//! Owned-region ring canonicalization entry points.

use super::*;
use crate::simulation::network::surface::{
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, keys::SURFACE_XZ_KEY_SCALE,
};
use std::collections::BTreeMap;

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_owned_region_rings(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    for region in regions.iter_mut() {
        for contour in &mut region.shape {
            *contour = noded_owned_region_contour(contour, &global_points);
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
        rail_canonical_points,
        SourceCarrierKeyPolicy::none(),
    )
}

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_final_owned_region_boundary_edges_for_piece_kind(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_canonical_points: &NodeRailCanonicalPointSet,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_final_owned_region_boundary_edges_with_options(
        regions,
        footprint_shapes,
        rail_canonical_points,
        SourceCarrierKeyPolicy::for_piece_kind(piece_kind),
    )
}

fn canonicalize_final_owned_region_boundary_edges_with_options(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_canonical_points: &NodeRailCanonicalPointSet,
    source_carrier_key_policy: SourceCarrierKeyPolicy,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_owned_region_rings_with_rail_point_set_with_options(
        regions,
        rail_canonical_points,
        source_carrier_key_policy,
    )?;
    node_owned_region_rings_to_global_points(regions, footprint_shapes);
    canonicalize_owned_region_rings_with_rail_point_set_with_options(
        regions,
        rail_canonical_points,
        source_carrier_key_policy,
    )?;
    Ok(())
}

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_final_join_or_cap_owned_region_boundary_edges(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_canonical_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_join_or_cap_owned_region_rings_with_rail_point_set(
        regions,
        rail_canonical_points,
    )?;
    node_join_or_cap_owned_region_rings_to_global_points(regions, footprint_shapes);
    canonicalize_join_or_cap_owned_region_rings_with_rail_point_set(
        regions,
        rail_canonical_points,
    )?;
    Ok(())
}

fn node_owned_region_rings_to_global_points(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    for region in regions {
        for contour in &mut region.shape {
            *contour = noded_owned_region_contour(contour, &global_points);
        }
    }
}

fn node_join_or_cap_owned_region_rings_to_global_points(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
) {
    let global_points = owned_region_global_points(regions, footprint_shapes);
    for region in regions {
        if region.claim_priority != NodeGeneratedContourClaimPriority::JoinOrCap {
            continue;
        }
        for contour in &mut region.shape {
            *contour = noded_owned_region_contour(contour, &global_points);
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
        rail_points,
        SourceCarrierKeyPolicy::none(),
    )
}

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_owned_region_rings_with_rail_point_set_for_piece_kind(
    regions: &mut [NodeBooleanOwnedRegion],
    rail_points: &NodeRailCanonicalPointSet,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_owned_region_rings_with_rail_point_set_with_options(
        regions,
        rail_points,
        SourceCarrierKeyPolicy::for_piece_kind(piece_kind),
    )
}

fn canonicalize_owned_region_rings_with_rail_point_set_with_options(
    regions: &mut [NodeBooleanOwnedRegion],
    rail_points: &NodeRailCanonicalPointSet,
    source_carrier_key_policy: SourceCarrierKeyPolicy,
) -> Result<(), NodeBooleanOwnershipError> {
    if rail_points.all_points.is_empty() {
        return Ok(());
    }

    for region in regions {
        canonicalize_owned_region_ring_with_rail_point_set(
            region,
            rail_points,
            source_carrier_key_policy,
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
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
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    if rail_points.all_points.is_empty() {
        return Ok(());
    }

    for region in regions {
        if region.claim_priority != NodeGeneratedContourClaimPriority::JoinOrCap {
            continue;
        }
        canonicalize_owned_region_ring_with_rail_point_set(
            region,
            rail_points,
            SourceCarrierKeyPolicy::none(),
        )?;
    }
    Ok(())
}

fn canonicalize_owned_region_ring_with_rail_point_set(
    region: &mut NodeBooleanOwnedRegion,
    rail_points: &NodeRailCanonicalPointSet,
    source_carrier_key_policy: SourceCarrierKeyPolicy,
) -> Result<(), NodeBooleanOwnershipError> {
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
    let owner_paths = if region.claim_priority == NodeGeneratedContourClaimPriority::JoinOrCap {
        rail_points
            .paths_by_owner
            .get(&region.owner)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    } else {
        &[]
    };

    for contour in &mut region.shape {
        canonicalize_owned_region_contour_to_owner_source_points(
            contour,
            region.owner,
            &preserved_points,
            has_source_carrier,
            uses_generated_join_or_cap,
            allow_source_carrier_key_adoption,
            canonicalize_source_height_numeric_dust,
            rail_points,
        )?;
        *contour = noded_owned_region_contour_with_rail_paths(
            contour,
            &source_points,
            owner_paths,
            region.claim_priority == NodeGeneratedContourClaimPriority::JoinOrCap,
        );
    }
    Ok(())
}

fn region_noding_point_for_owner_source(
    owner: NodeBandOwner,
    preserved_source_points: &[NodeOwnershipPointKey],
    point: NodeOwnershipPointKey,
    rail_points: &NodeRailCanonicalPointSet,
    canonicalize_source_height_numeric_dust: bool,
) -> Result<Option<NodeOwnershipPointKey>, NodeBooleanOwnershipError> {
    if preserved_source_points.binary_search(&point).is_ok() {
        return Ok(Some(point));
    }
    if canonicalize_source_height_numeric_dust
        && let Some(point) =
            unique_preserved_source_numeric_dust_point(preserved_source_points, point)
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
    preserved_source_points: &[NodeOwnershipPointKey],
    point: NodeOwnershipPointKey,
) -> Option<NodeOwnershipPointKey> {
    let point_mm = ownership_mm_key(point);
    let mut candidates = preserved_source_points
        .iter()
        .copied()
        .filter(|candidate| ownership_mm_key(*candidate) == point_mm)
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
