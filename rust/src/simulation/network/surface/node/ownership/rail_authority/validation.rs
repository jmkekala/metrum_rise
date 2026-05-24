//! Rail-source authority validation helpers.

use super::*;

pub(in crate::simulation::network::surface::node::ownership) fn canonical_points_by_mm_key_by_owner(
    points_by_owner: &BTreeMap<NodeBandOwner, Vec<NodeOwnershipPointKey>>,
) -> BTreeMap<NodeBandOwner, BTreeMap<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>> {
    let mut by_owner = BTreeMap::new();
    for (owner, points) in points_by_owner {
        let mut by_mm_key =
            BTreeMap::<NodeOwnershipPointKey, BTreeSet<NodeOwnershipPointKey>>::new();
        for point in points {
            by_mm_key
                .entry(ownership_mm_key(*point))
                .or_default()
                .insert(*point);
        }
        by_owner.insert(*owner, by_mm_key);
    }
    by_owner
}

pub(in crate::simulation::network::surface::node::ownership) fn validate_owned_region_vertices_against_source_authority(
    regions: &[NodeBooleanOwnedRegion],
    rail_points: &NodeRailCanonicalPointSet,
) -> Result<(), NodeBooleanOwnershipError> {
    for region in regions {
        let source_height_points = region
            .source_band_index
            .and_then(|source_band_index| {
                rail_points.height_points_by_source.get(&(
                    region.kind,
                    region.source_mouth_order_index,
                    source_band_index,
                ))
            })
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let source_key = region.source_band_index.map(|source_band_index| {
            (
                region.kind,
                region.source_mouth_order_index,
                source_band_index,
            )
        });
        for contour in &region.shape {
            for point in contour
                .iter()
                .copied()
                .map(ownership_key_from_overlay_point)
            {
                if source_height_points.binary_search(&point).is_ok() {
                    continue;
                }
                if rail_points
                    .source_authorizes_same_mm_duplicate_cluster(point, source_height_points)
                {
                    continue;
                }
                if rail_points
                    .source_canonicalized_point_for_owner(
                        region.owner,
                        point,
                        source_key,
                        source_height_points,
                    )?
                    .is_some()
                {
                    continue;
                }
                if rail_points.owner_source_authorizes_point(region.owner, point)? {
                    continue;
                }
                let Some(canonical) =
                    rail_points.canonical_conflict_for_owner(region.owner, point)?
                else {
                    continue;
                };
                return Err(NodeBooleanOwnershipError::NonCanonicalOwnedRegionVertex {
                    owner: region.owner,
                    point_x_key: point.0,
                    point_z_key: point.1,
                    canonical_x_key: canonical.0,
                    canonical_z_key: canonical.1,
                });
            }
        }
    }
    Ok(())
}

pub(in crate::simulation::network::surface::node::ownership) fn constraint_authority_owners(
    constraint: &NodeRailConstraint,
) -> Vec<NodeBandOwner> {
    let mut owners = [constraint.owner, constraint.opposite_owner]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners.dedup();
    owners
}
