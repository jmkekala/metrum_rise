//! Rail-source canonical point collection.

use super::*;
use crate::simulation::network::surface::{
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M,
    keys::{SURFACE_XZ_KEY_SCALE, SurfaceHeightMmKey},
};

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
    let mut paths_by_owner = BTreeMap::<NodeBandOwner, Vec<Vec<NodeOwnershipPointKey>>>::new();
    let source_carriers = rails.source_carriers.clone();
    for points in source_carriers.height_points_by_source.values() {
        all_points.extend(points.iter().copied());
    }
    for contour in &rails.contours {
        let path = contour
            .points_xz
            .iter()
            .copied()
            .map(ownership_key_from_road_point)
            .collect::<Vec<_>>();
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
        if let (NodeGeneratedContourKind::Band { kind }, Some(source_band_index)) =
            (contour.kind, contour.source_band_index)
            && let Some(paths_for_source) = rails.height_carrier_paths_by_source.get(&(
                kind,
                contour.source_mouth_order_index,
                source_band_index,
            ))
        {
            for paths in paths_for_source {
                paths_by_owner
                    .entry(owner)
                    .or_default()
                    .push(height_carrier_loop_points(paths));
            }
        }
        paths_by_owner.entry(owner).or_default().push(path);
    }
    for constraint in &rails.constraints {
        let path = constraint
            .points_xz
            .iter()
            .copied()
            .map(ownership_key_from_road_point)
            .collect::<Vec<_>>();
        for owner in constraint_authority_owners(constraint) {
            points_by_owner
                .entry(owner)
                .or_default()
                .extend(path.iter().copied());
        }
    }
    for (owner, points) in &mut points_by_owner {
        points.sort_unstable();
        points.dedup();
        let _ = owner;
    }
    all_points.sort_unstable();
    all_points.dedup();
    let canonical_points_by_mm_key_by_owner = canonical_points_by_mm_key_by_owner(&points_by_owner);
    NodeRailCanonicalPointSet {
        all_points,
        points_by_owner,
        source_carriers,
        canonical_points_by_mm_key_by_owner,
        paths_by_owner,
    }
}

impl NodeSourceCarrierRegistry {
    pub(crate) fn from_rail_parts(
        contours: &[NodeGeneratedContour],
        constraints: &[NodeRailConstraint],
        height_carrier_paths_by_source: &BTreeMap<
            NodeRailHeightSourceKey,
            Vec<NodeRailHeightCarrierPaths>,
        >,
        height_carrier_points_by_source: &BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
    ) -> Self {
        let mut source_segments_by_owner =
            BTreeMap::<NodeBandOwner, Vec<NodeRailSourceSegmentAuthority>>::new();
        let mut height_points_by_source =
            BTreeMap::<NodeRailHeightSourceKey, Vec<NodeOwnershipPointKey>>::new();
        let mut owners_by_source = BTreeMap::<NodeRailHeightSourceKey, Vec<NodeBandOwner>>::new();
        let numeric_dust_canonicalized_sources = numeric_dust_canonicalized_sources(
            contours,
            height_carrier_paths_by_source,
            height_carrier_points_by_source,
        );
        for (source, points) in height_carrier_points_by_source {
            let points = canonical_source_carrier_points_for_source(
                *source,
                points
                    .iter()
                    .copied()
                    .map(|point| ownership_key_from_road_point(road_vec3_xz(point)))
                    .collect(),
                &numeric_dust_canonicalized_sources,
            );
            height_points_by_source
                .entry(*source)
                .or_default()
                .extend(points);
        }
        for contour in contours {
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
                let source_path = canonical_source_carrier_path_for_source(
                    source,
                    path.clone(),
                    &numeric_dust_canonicalized_sources,
                );
                height_points_by_source
                    .entry(source)
                    .or_default()
                    .extend(source_path.iter().copied());
                if let Some(owner) = contour.owner {
                    owners_by_source.entry(source).or_default().push(owner);
                    insert_closed_generated_surface_authority_segments(
                        &mut source_segments_by_owner,
                        owner,
                        source,
                        &source_path,
                    );
                }
            }
        }
        for constraint in constraints {
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
                    let source_path = canonical_source_carrier_path_for_source(
                        source,
                        path.clone(),
                        &numeric_dust_canonicalized_sources,
                    );
                    height_points_by_source
                        .entry(source)
                        .or_default()
                        .extend(source_path.iter().copied());
                    insert_open_source_authority_segments(
                        &mut source_segments_by_owner,
                        owner,
                        source,
                        &source_path,
                    );
                }
            }
        }
        for owners in owners_by_source.values_mut() {
            owners.sort_unstable();
            owners.dedup();
        }
        for (source, paths_for_source) in height_carrier_paths_by_source {
            let Some(owners) = owners_by_source.get(source) else {
                continue;
            };
            for paths in paths_for_source {
                let path = canonical_source_carrier_path_for_source(
                    *source,
                    height_carrier_loop_points(paths),
                    &numeric_dust_canonicalized_sources,
                );
                for owner in owners {
                    insert_closed_source_authority_segments(
                        &mut source_segments_by_owner,
                        *owner,
                        *source,
                        &path,
                    );
                }
            }
        }
        for (source, points) in &mut height_points_by_source {
            if source_uses_numeric_dust_carrier_canonicalization(
                *source,
                &numeric_dust_canonicalized_sources,
            ) {
                let mut canonical = canonical_source_carrier_points(points.iter().copied());
                canonical.sort_unstable();
                canonical.dedup();
                *points = canonical;
            } else {
                points.sort_unstable();
                points.dedup();
            }
        }
        for segments in source_segments_by_owner.values_mut() {
            segments.sort_unstable();
            segments.dedup();
        }
        Self {
            source_segments_by_owner,
            height_points_by_source,
            numeric_dust_canonicalized_sources,
        }
    }
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

fn canonical_source_carrier_path_for_source(
    source: NodeRailHeightSourceKey,
    points: Vec<NodeOwnershipPointKey>,
    numeric_dust_canonicalized_sources: &BTreeSet<NodeRailHeightSourceKey>,
) -> Vec<NodeOwnershipPointKey> {
    if source_uses_numeric_dust_carrier_canonicalization(source, numeric_dust_canonicalized_sources)
    {
        canonical_source_carrier_path_points(points)
    } else {
        points
    }
}

fn canonical_source_carrier_points_for_source(
    source: NodeRailHeightSourceKey,
    points: Vec<NodeOwnershipPointKey>,
    numeric_dust_canonicalized_sources: &BTreeSet<NodeRailHeightSourceKey>,
) -> Vec<NodeOwnershipPointKey> {
    if source_uses_numeric_dust_carrier_canonicalization(source, numeric_dust_canonicalized_sources)
    {
        canonical_source_carrier_points(points)
    } else {
        points
    }
}

fn source_uses_numeric_dust_carrier_canonicalization(
    source: NodeRailHeightSourceKey,
    numeric_dust_canonicalized_sources: &BTreeSet<NodeRailHeightSourceKey>,
) -> bool {
    numeric_dust_canonicalized_sources.contains(&source)
}

#[derive(Clone, Copy, Debug)]
struct SourceHeightRange {
    min_mm: i64,
    max_mm: i64,
}

impl SourceHeightRange {
    fn from_point(point: RoadVec3) -> Self {
        let height_mm = SurfaceHeightMmKey::from_m_f64(point.y).as_i64();
        Self {
            min_mm: height_mm,
            max_mm: height_mm,
        }
    }

    fn include(&mut self, point: RoadVec3) {
        let height_mm = SurfaceHeightMmKey::from_m_f64(point.y).as_i64();
        self.min_mm = self.min_mm.min(height_mm);
        self.max_mm = self.max_mm.max(height_mm);
    }

    fn has_variation(self) -> bool {
        self.min_mm != self.max_mm
    }
}

fn numeric_dust_canonicalized_sources(
    contours: &[NodeGeneratedContour],
    height_carrier_paths_by_source: &BTreeMap<
        NodeRailHeightSourceKey,
        Vec<NodeRailHeightCarrierPaths>,
    >,
    height_carrier_points_by_source: &BTreeMap<NodeRailHeightSourceKey, Vec<RoadVec3>>,
) -> BTreeSet<NodeRailHeightSourceKey> {
    let mut height_ranges = BTreeMap::<NodeRailHeightSourceKey, SourceHeightRange>::new();
    for (source, points) in height_carrier_points_by_source {
        for point in points.iter().copied() {
            include_source_height_range(&mut height_ranges, *source, point);
        }
    }
    for contour in contours {
        let (NodeGeneratedContourKind::Band { kind }, Some(source_band_index)) =
            (contour.kind, contour.source_band_index)
        else {
            continue;
        };
        let Some(points) = contour.height_points_world.as_deref() else {
            continue;
        };
        let source = (kind, contour.source_mouth_order_index, source_band_index);
        for point in points.iter().copied() {
            include_source_height_range(&mut height_ranges, source, point);
        }
    }
    for (source, paths_for_source) in height_carrier_paths_by_source {
        for paths in paths_for_source {
            for point in paths
                .start_path_world
                .iter()
                .chain(paths.end_path_world.iter())
                .copied()
            {
                include_source_height_range(&mut height_ranges, *source, point);
            }
        }
    }
    height_ranges
        .into_iter()
        .filter_map(|(source, range)| {
            (source_allows_numeric_dust_carrier_canonicalization(source) && range.has_variation())
                .then_some(source)
        })
        .collect()
}

fn source_allows_numeric_dust_carrier_canonicalization(source: NodeRailHeightSourceKey) -> bool {
    source.0 == RoadSurfaceBandKind::Carriageway && source.2 == 2
}

fn include_source_height_range(
    height_ranges: &mut BTreeMap<NodeRailHeightSourceKey, SourceHeightRange>,
    source: NodeRailHeightSourceKey,
    point: RoadVec3,
) {
    height_ranges
        .entry(source)
        .and_modify(|range| range.include(point))
        .or_insert_with(|| SourceHeightRange::from_point(point));
}

fn canonical_source_carrier_path_points(
    points: Vec<NodeOwnershipPointKey>,
) -> Vec<NodeOwnershipPointKey> {
    let mut canonical = Vec::with_capacity(points.len());
    for point in points {
        if canonical
            .last()
            .copied()
            .is_some_and(|last| source_carrier_points_are_numeric_dust_duplicates(last, point))
        {
            continue;
        }
        canonical.push(point);
    }
    if canonical.len() >= 2 {
        let last = canonical[canonical.len() - 1];
        if source_carrier_points_are_numeric_dust_duplicates(canonical[0], last) {
            canonical.pop();
        }
    }
    canonical
}

fn canonical_source_carrier_points(
    points: impl IntoIterator<Item = NodeOwnershipPointKey>,
) -> Vec<NodeOwnershipPointKey> {
    let mut canonical = Vec::new();
    let mut indices_by_mm = BTreeMap::<NodeOwnershipPointKey, Vec<usize>>::new();
    for point in points {
        let point_mm = ownership_mm_key(point);
        if indices_by_mm.get(&point_mm).is_some_and(|indices| {
            indices.iter().copied().any(|index| {
                source_carrier_points_are_numeric_dust_duplicates(canonical[index], point)
            })
        }) {
            continue;
        }
        let index = canonical.len();
        canonical.push(point);
        indices_by_mm.entry(point_mm).or_default().push(index);
    }
    canonical
}

fn source_carrier_points_are_numeric_dust_duplicates(
    first: NodeOwnershipPointKey,
    second: NodeOwnershipPointKey,
) -> bool {
    if ownership_mm_key(first) != ownership_mm_key(second) {
        return false;
    }
    let dx = i128::from(first.0 - second.0);
    let dz = i128::from(first.1 - second.1);
    let dust = i128::from(source_carrier_numeric_dust_key_units());
    dx * dx + dz * dz <= dust * dust
}

fn source_carrier_numeric_dust_key_units() -> i64 {
    (f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * SURFACE_XZ_KEY_SCALE).round() as i64
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
            .push(NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new(segment[0], segment[1]),
            ));
    }
}

fn insert_closed_generated_surface_authority_segments(
    segments_by_owner: &mut BTreeMap<NodeBandOwner, Vec<NodeRailSourceSegmentAuthority>>,
    owner: NodeBandOwner,
    source: NodeRailHeightSourceKey,
    path: &[NodeOwnershipPointKey],
) {
    insert_open_generated_surface_authority_segments(segments_by_owner, owner, source, path);
    let (Some(first), Some(last)) = (path.first().copied(), path.last().copied()) else {
        return;
    };
    if first == last {
        return;
    }
    segments_by_owner.entry(owner).or_default().push(
        NodeRailSourceSegmentAuthority::generated_surface(
            owner,
            source,
            OwnedRegionEdgeKey::new(first, last),
        ),
    );
}

fn insert_open_generated_surface_authority_segments(
    segments_by_owner: &mut BTreeMap<NodeBandOwner, Vec<NodeRailSourceSegmentAuthority>>,
    owner: NodeBandOwner,
    source: NodeRailHeightSourceKey,
    path: &[NodeOwnershipPointKey],
) {
    for segment in path.windows(2) {
        if segment[0] == segment[1] {
            continue;
        }
        segments_by_owner.entry(owner).or_default().push(
            NodeRailSourceSegmentAuthority::generated_surface(
                owner,
                source,
                OwnedRegionEdgeKey::new(segment[0], segment[1]),
            ),
        );
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
        .push(NodeRailSourceSegmentAuthority::new(
            owner,
            source,
            OwnedRegionEdgeKey::new(first, last),
        ));
}
