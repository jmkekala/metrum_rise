//! Rail-source canonical point collection.

use super::*;

pub(in crate::simulation::network::surface::node::ownership) fn canonical_points_for_rail_set(
    rails: &NodeRailContourSet,
) -> NodeRailCanonicalPointSet {
    let mut all_points = rails
        .constraints
        .iter()
        .flat_map(|constraint| constraint.points_xz.iter().copied())
        .chain(
            rails
                .contours
                .iter()
                .flat_map(|contour| contour.points_xz.iter().copied()),
        )
        .map(ownership_key_from_road_point)
        .collect::<Vec<_>>();
    let mut points_by_owner = BTreeMap::<NodeBandOwner, Vec<NodeOwnershipPointKey>>::new();
    let mut segments_by_owner = BTreeMap::<NodeBandOwner, Vec<OwnedRegionEdgeKey>>::new();
    let mut source_segments_by_owner =
        BTreeMap::<NodeBandOwner, Vec<NodeRailSourceSegmentAuthority>>::new();
    let mut height_points_by_source =
        BTreeMap::<NodeRailHeightSourceKey, Vec<NodeOwnershipPointKey>>::new();
    let mut paths_by_owner = BTreeMap::<NodeBandOwner, Vec<Vec<NodeOwnershipPointKey>>>::new();
    let mut owners_by_source = BTreeMap::<NodeRailHeightSourceKey, Vec<NodeBandOwner>>::new();
    for (source, points) in &rails.height_carrier_points_by_source {
        let points = points
            .iter()
            .copied()
            .map(|point| ownership_key_from_road_point(road_vec3_xz(point)))
            .collect::<Vec<_>>();
        all_points.extend(points.iter().copied());
        height_points_by_source
            .entry(*source)
            .or_default()
            .extend(points);
    }
    for constraint in &rails.constraints {
        let (Some(owner), Some(source_band_index)) =
            (constraint.owner, constraint.source_band_index)
        else {
            continue;
        };
        height_points_by_source
            .entry((
                owner.kind(),
                constraint.source_mouth_order_index,
                source_band_index,
            ))
            .or_default()
            .extend(
                constraint
                    .points_xz
                    .iter()
                    .copied()
                    .map(ownership_key_from_road_point),
            );
    }
    for contour in &rails.contours {
        let path = contour
            .points_xz
            .iter()
            .copied()
            .map(ownership_key_from_road_point)
            .collect::<Vec<_>>();
        if let (NodeGeneratedContourKind::Band { kind }, Some(source_band_index)) =
            (contour.kind, contour.source_band_index)
        {
            let source = (kind, contour.source_mouth_order_index, source_band_index);
            height_points_by_source
                .entry(source)
                .or_default()
                .extend(path.iter().copied());
            if let Some(owner) = contour.owner {
                owners_by_source.entry(source).or_default().push(owner);
                insert_closed_source_authority_segments(
                    &mut source_segments_by_owner,
                    owner,
                    source,
                    &path,
                );
            }
        }
        let Some(owner) = contour.owner else {
            continue;
        };
        points_by_owner
            .entry(owner)
            .or_default()
            .extend(path.iter().copied());
        if contour.height_points_world.is_some() {
            points_by_owner
                .entry(owner)
                .or_default()
                .extend(path.iter().copied());
        }
        insert_closed_source_segments(&mut segments_by_owner, owner, &path);
        paths_by_owner.entry(owner).or_default().push(path);
    }
    for constraint in &rails.constraints {
        let path = constraint
            .points_xz
            .iter()
            .copied()
            .map(ownership_key_from_road_point)
            .collect::<Vec<_>>();
        if let Some(source_band_index) = constraint.source_band_index {
            for owner in constraint_authority_owners(constraint) {
                let source = (
                    owner.kind(),
                    constraint.source_mouth_order_index,
                    source_band_index,
                );
                height_points_by_source
                    .entry(source)
                    .or_default()
                    .extend(path.iter().copied());
                insert_open_source_authority_segments(
                    &mut source_segments_by_owner,
                    owner,
                    source,
                    &path,
                );
            }
        }
        for owner in constraint_authority_owners(constraint) {
            points_by_owner
                .entry(owner)
                .or_default()
                .extend(path.iter().copied());
            insert_open_source_segments(&mut segments_by_owner, owner, &path);
        }
    }
    for owners in owners_by_source.values_mut() {
        owners.sort_unstable();
        owners.dedup();
    }
    for (source, paths) in &rails.height_carrier_paths_by_source {
        let Some([owner]) = owners_by_source.get(source).map(Vec::as_slice) else {
            continue;
        };
        let path = height_carrier_loop_points(paths);
        insert_closed_source_authority_segments(
            &mut source_segments_by_owner,
            *owner,
            *source,
            &path,
        );
    }
    for (owner, points) in &mut points_by_owner {
        points.sort_unstable();
        points.dedup();
        let _ = owner;
    }
    for points in height_points_by_source.values_mut() {
        points.sort_unstable();
        points.dedup();
    }
    for segments in segments_by_owner.values_mut() {
        segments.sort_unstable();
        segments.dedup();
    }
    for segments in source_segments_by_owner.values_mut() {
        segments.sort_unstable();
        segments.dedup();
    }
    all_points.sort_unstable();
    all_points.dedup();
    let canonical_points_by_mm_key_by_owner = canonical_points_by_mm_key_by_owner(&points_by_owner);
    NodeRailCanonicalPointSet {
        all_points,
        points_by_owner,
        segments_by_owner,
        source_segments_by_owner,
        canonical_points_by_mm_key_by_owner,
        height_points_by_source,
        paths_by_owner,
    }
}

pub(in crate::simulation::network::surface::node::ownership) fn insert_open_source_segments(
    segments_by_owner: &mut BTreeMap<NodeBandOwner, Vec<OwnedRegionEdgeKey>>,
    owner: NodeBandOwner,
    path: &[NodeOwnershipPointKey],
) {
    for segment in path.windows(2) {
        if segment[0] == segment[1] {
            continue;
        }
        segments_by_owner
            .entry(owner)
            .or_default()
            .push(OwnedRegionEdgeKey::new(segment[0], segment[1]));
    }
}

fn insert_closed_source_segments(
    segments_by_owner: &mut BTreeMap<NodeBandOwner, Vec<OwnedRegionEdgeKey>>,
    owner: NodeBandOwner,
    path: &[NodeOwnershipPointKey],
) {
    insert_open_source_segments(segments_by_owner, owner, path);
    let (Some(first), Some(last)) = (path.first().copied(), path.last().copied()) else {
        return;
    };
    if first == last {
        return;
    }
    segments_by_owner
        .entry(owner)
        .or_default()
        .push(OwnedRegionEdgeKey::new(first, last));
}

fn height_carrier_loop_points(paths: &NodeRailHeightCarrierPaths) -> Vec<NodeOwnershipPointKey> {
    let mut path = Vec::with_capacity(paths.start_path_world.len() + paths.end_path_world.len());
    path.extend(
        paths
            .start_path_world
            .iter()
            .copied()
            .map(|point| ownership_key_from_road_point(road_vec3_xz(point))),
    );
    path.extend(
        paths
            .end_path_world
            .iter()
            .rev()
            .copied()
            .map(|point| ownership_key_from_road_point(road_vec3_xz(point))),
    );
    path
}

fn insert_open_source_authority_segments(
    segments_by_owner: &mut BTreeMap<NodeBandOwner, Vec<NodeRailSourceSegmentAuthority>>,
    owner: NodeBandOwner,
    source: NodeRailHeightSourceKey,
    path: &[NodeOwnershipPointKey],
) {
    for segment in path.windows(2) {
        if segment[0] == segment[1] {
            continue;
        }
        segments_by_owner
            .entry(owner)
            .or_default()
            .push(NodeRailSourceSegmentAuthority {
                owner,
                source,
                segment: OwnedRegionEdgeKey::new(segment[0], segment[1]),
            });
    }
}

fn insert_closed_source_authority_segments(
    segments_by_owner: &mut BTreeMap<NodeBandOwner, Vec<NodeRailSourceSegmentAuthority>>,
    owner: NodeBandOwner,
    source: NodeRailHeightSourceKey,
    path: &[NodeOwnershipPointKey],
) {
    insert_open_source_authority_segments(segments_by_owner, owner, source, path);
    let (Some(first), Some(last)) = (path.first().copied(), path.last().copied()) else {
        return;
    };
    if first == last {
        return;
    }
    segments_by_owner
        .entry(owner)
        .or_default()
        .push(NodeRailSourceSegmentAuthority {
            owner,
            source,
            segment: OwnedRegionEdgeKey::new(first, last),
        });
}
