//! Owned-region ring canonicalization entry points.

use super::*;

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
        false,
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
        allow_source_carrier_key_adoption_for_piece_kind(piece_kind),
    )
}

fn canonicalize_final_owned_region_boundary_edges_with_options(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_canonical_points: &NodeRailCanonicalPointSet,
    allow_source_carrier_key_adoption: bool,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_owned_region_rings_with_rail_point_set_with_options(
        regions,
        rail_canonical_points,
        allow_source_carrier_key_adoption,
    )?;
    node_owned_region_rings_to_global_points(regions, footprint_shapes);
    canonicalize_owned_region_rings_with_rail_point_set_with_options(
        regions,
        rail_canonical_points,
        allow_source_carrier_key_adoption,
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
    canonicalize_owned_region_rings_with_rail_point_set_with_options(regions, rail_points, false)
}

pub(in crate::simulation::network::surface::node::ownership) fn canonicalize_owned_region_rings_with_rail_point_set_for_piece_kind(
    regions: &mut [NodeBooleanOwnedRegion],
    rail_points: &NodeRailCanonicalPointSet,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Result<(), NodeBooleanOwnershipError> {
    canonicalize_owned_region_rings_with_rail_point_set_with_options(
        regions,
        rail_points,
        allow_source_carrier_key_adoption_for_piece_kind(piece_kind),
    )
}

fn canonicalize_owned_region_rings_with_rail_point_set_with_options(
    regions: &mut [NodeBooleanOwnedRegion],
    rail_points: &NodeRailCanonicalPointSet,
    allow_source_carrier_key_adoption: bool,
) -> Result<(), NodeBooleanOwnershipError> {
    if rail_points.all_points.is_empty() {
        return Ok(());
    }

    for region in regions {
        canonicalize_owned_region_ring_with_rail_point_set(
            region,
            rail_points,
            allow_source_carrier_key_adoption,
        )?;
    }
    Ok(())
}

fn allow_source_carrier_key_adoption_for_piece_kind(
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> bool {
    matches!(piece_kind, RoadSurfaceVisualNodePieceKind::Terminal)
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
        canonicalize_owned_region_ring_with_rail_point_set(region, rail_points, false)?;
    }
    Ok(())
}

fn canonicalize_owned_region_ring_with_rail_point_set(
    region: &mut NodeBooleanOwnedRegion,
    rail_points: &NodeRailCanonicalPointSet,
    allow_source_carrier_key_adoption: bool,
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
    let mut preserved_points = source_height_points.cloned().unwrap_or_default();
    preserved_points.sort_unstable();
    preserved_points.dedup();
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
) -> Result<Option<NodeOwnershipPointKey>, NodeBooleanOwnershipError> {
    if preserved_source_points.binary_search(&point).is_ok() {
        return Ok(Some(point));
    }
    match rail_points.canonicalized_point_for_owner(owner, point) {
        Ok(canonical) => Ok(Some(canonical)),
        Err(NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn canonicalize_owned_region_contour_to_owner_source_points(
    contour: &mut NodeOverlayContour,
    owner: NodeBandOwner,
    source_points: &[NodeOwnershipPointKey],
    has_source_carrier: bool,
    uses_generated_join_or_cap: bool,
    allow_source_carrier_key_adoption: bool,
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    for point in contour.iter_mut() {
        let key = ownership_key_from_overlay_point(*point);
        if source_points.binary_search(&key).is_ok() {
            continue;
        }
        if has_source_carrier {
            if (uses_generated_join_or_cap || allow_source_carrier_key_adoption)
                && let Some(canonical) =
                    region_noding_point_for_owner_source(owner, source_points, key, rail_points)?
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
