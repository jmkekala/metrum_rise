//! Canonical ring noding and cleanup helpers for node boolean ownership.

use super::super::arrangement::NodeBandOwner;
use super::super::keys::SurfaceXzKey;
use super::super::rails::{
    NodeGeneratedContourClaimPriority, NodeGeneratedContourKind, NodeRailConstraint,
    NodeRailContourSet,
};
use super::super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayShape, NodeOverlayShapes,
    RoadSurfaceBandKind, RoadSurfaceSystem,
};
use super::seams::{owned_shape_is_discardable_numeric_dust, seam_constraints_for_shape};
use super::{
    NodeBooleanOwnedRegion, NodeBooleanOwnershipError, NodeOwnershipPointKey,
    NodeRailCanonicalPointSet, OwnedRegionEdgeKey, overlay_point_from_key, overlay_union,
    ownership_key_from_overlay_point, ownership_key_from_road_point, ownership_mm_key,
    point_key_lies_exactly_on_segment, point_key_lies_on_segment, segment_parameter_key,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn canonicalize_owned_region_rings(
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

pub(super) fn clean_canonical_owned_region_shapes(
    regions: &mut Vec<NodeBooleanOwnedRegion>,
    footprint_shapes: &NodeOverlayShapes,
    rail_constraints: &[NodeRailConstraint],
    rail_canonical_points: &NodeRailCanonicalPointSet,
    allow_grid_bounded_constraint_overlap: bool,
) -> Result<(), NodeBooleanOwnershipError> {
    let mut cleaned_regions = Vec::with_capacity(regions.len());
    for region in regions.drain(..) {
        let mut shapes = overlay_union(&region.shape, "owned_region_ring_clean")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
        for shape in shapes.into_iter().flat_map(|shape| {
            split_self_touching_owned_shape(shape, allow_grid_bounded_constraint_overlap)
        }) {
            let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&shape);
            if owned_shape_is_discardable_numeric_dust(
                &shape,
                area_m2,
                region.owner,
                rail_constraints,
            ) {
                continue;
            }
            cleaned_regions.push(NodeBooleanOwnedRegion {
                kind: region.kind,
                owner: region.owner,
                claim_priority: region.claim_priority,
                source_mouth_order_index: region.source_mouth_order_index,
                source_band_index: region.source_band_index,
                shape: shape.clone(),
                area_m2,
                seam_constraints: seam_constraints_for_shape(
                    &shape,
                    region.owner,
                    rail_constraints,
                    allow_grid_bounded_constraint_overlap,
                ),
            });
        }
    }
    canonicalize_owned_region_rings_with_rail_point_set(
        &mut cleaned_regions,
        rail_canonical_points,
    );
    clean_owned_region_shapes_once(
        &mut cleaned_regions,
        rail_constraints,
        allow_grid_bounded_constraint_overlap,
    )?;
    canonicalize_owned_region_rings_with_rail_point_set(
        &mut cleaned_regions,
        rail_canonical_points,
    );
    canonicalize_final_owned_region_boundary_edges(
        &mut cleaned_regions,
        footprint_shapes,
        rail_canonical_points,
    );
    clean_owned_region_shapes_once(
        &mut cleaned_regions,
        rail_constraints,
        allow_grid_bounded_constraint_overlap,
    )?;
    canonicalize_final_owned_region_boundary_edges(
        &mut cleaned_regions,
        footprint_shapes,
        rail_canonical_points,
    );
    split_final_canonical_owned_region_self_touches(&mut cleaned_regions, rail_constraints, false);
    canonicalize_owned_region_rings_with_rail_point_set(
        &mut cleaned_regions,
        rail_canonical_points,
    );
    for region in &mut cleaned_regions {
        region.seam_constraints = seam_constraints_for_shape(
            &region.shape,
            region.owner,
            rail_constraints,
            allow_grid_bounded_constraint_overlap,
        );
    }
    validate_owned_region_vertices_against_source_authority(
        &cleaned_regions,
        rail_canonical_points,
    )?;
    *regions = cleaned_regions;
    Ok(())
}

pub(super) fn canonicalize_final_owned_region_boundary_edges(
    regions: &mut [NodeBooleanOwnedRegion],
    footprint_shapes: &NodeOverlayShapes,
    rail_canonical_points: &NodeRailCanonicalPointSet,
) {
    canonicalize_owned_region_rings_with_rail_point_set(regions, rail_canonical_points);
    node_owned_region_rings_to_global_points(regions, footprint_shapes);
    canonicalize_owned_region_rings_with_rail_point_set(regions, rail_canonical_points);
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

fn split_final_canonical_owned_region_self_touches(
    regions: &mut Vec<NodeBooleanOwnedRegion>,
    rail_constraints: &[NodeRailConstraint],
    allow_grid_bounded_constraint_overlap: bool,
) {
    let mut split_regions = Vec::with_capacity(regions.len());
    for region in regions.drain(..) {
        let NodeBooleanOwnedRegion {
            kind,
            owner,
            claim_priority,
            source_mouth_order_index,
            source_band_index,
            shape,
            ..
        } = region;
        for shape in split_self_touching_owned_shape(shape, allow_grid_bounded_constraint_overlap) {
            let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&shape);
            if owned_shape_is_discardable_numeric_dust(&shape, area_m2, owner, rail_constraints) {
                continue;
            }
            split_regions.push(NodeBooleanOwnedRegion {
                kind,
                owner,
                claim_priority,
                source_mouth_order_index,
                source_band_index,
                shape,
                area_m2,
                seam_constraints: Vec::new(),
            });
        }
    }
    *regions = split_regions;
}

fn clean_owned_region_shapes_once(
    regions: &mut Vec<NodeBooleanOwnedRegion>,
    rail_constraints: &[NodeRailConstraint],
    allow_grid_bounded_constraint_overlap: bool,
) -> Result<(), NodeBooleanOwnershipError> {
    let mut cleaned_regions = Vec::with_capacity(regions.len());
    for region in regions.drain(..) {
        let mut shapes = overlay_union(&region.shape, "owned_region_constraint_noded_clean")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
        for shape in shapes.into_iter().flat_map(|shape| {
            split_self_touching_owned_shape(shape, allow_grid_bounded_constraint_overlap)
        }) {
            let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&shape);
            if owned_shape_is_discardable_numeric_dust(
                &shape,
                area_m2,
                region.owner,
                rail_constraints,
            ) {
                continue;
            }
            cleaned_regions.push(NodeBooleanOwnedRegion {
                kind: region.kind,
                owner: region.owner,
                claim_priority: region.claim_priority,
                source_mouth_order_index: region.source_mouth_order_index,
                source_band_index: region.source_band_index,
                shape: shape.clone(),
                area_m2,
                seam_constraints: seam_constraints_for_shape(
                    &shape,
                    region.owner,
                    rail_constraints,
                    allow_grid_bounded_constraint_overlap,
                ),
            });
        }
    }
    *regions = cleaned_regions;
    Ok(())
}

pub(super) fn canonical_points_for_rail_set(
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
    let mut height_points_by_source =
        BTreeMap::<(RoadSurfaceBandKind, usize, usize), Vec<NodeOwnershipPointKey>>::new();
    let mut paths_by_owner = BTreeMap::<NodeBandOwner, Vec<Vec<NodeOwnershipPointKey>>>::new();
    for (source, points) in &rails.height_carrier_points_by_source {
        let points = points
            .iter()
            .copied()
            .map(ownership_key_from_road_point)
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
            height_points_by_source
                .entry((kind, contour.source_mouth_order_index, source_band_index))
                .or_default()
                .extend(path.iter().copied());
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
        for owner in constraint_authority_owners(constraint) {
            points_by_owner
                .entry(owner)
                .or_default()
                .extend(path.iter().copied());
            insert_open_source_segments(&mut segments_by_owner, owner, &path);
        }
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
    all_points.sort_unstable();
    all_points.dedup();
    let canonical_points_by_mm_key_by_owner = canonical_points_by_mm_key_by_owner(&points_by_owner);
    NodeRailCanonicalPointSet {
        all_points,
        points_by_owner,
        segments_by_owner,
        canonical_points_by_mm_key_by_owner,
        height_points_by_source,
        paths_by_owner,
    }
}

pub(super) fn canonicalize_owned_region_rings_with_rail_point_set(
    regions: &mut [NodeBooleanOwnedRegion],
    rail_points: &NodeRailCanonicalPointSet,
) {
    if rail_points.all_points.is_empty() {
        return;
    }

    for region in regions {
        let owner_points = rail_points
            .points_by_owner
            .get(&region.owner)
            .map(Vec::as_slice)
            .unwrap_or(&rail_points.all_points);
        let source_height_points = region.source_band_index.and_then(|source_band_index| {
            rail_points.height_points_by_source.get(&(
                region.kind,
                region.source_mouth_order_index,
                source_band_index,
            ))
        });
        let mut preserved_points = source_height_points.cloned().unwrap_or_default();
        preserved_points.sort_unstable();
        preserved_points.dedup();
        let authority_points = source_height_points
            .map(Vec::as_slice)
            .unwrap_or(owner_points);
        let mut source_points = preserved_points.clone();
        source_points.extend(authority_points.iter().copied().map(|point| {
            rail_points
                .canonical_point_for_owner(region.owner, point)
                .unwrap_or(point)
        }));
        source_points.extend(rail_points.all_points.iter().copied().map(|point| {
            rail_points
                .canonical_point_for_owner(region.owner, point)
                .unwrap_or(point)
        }));
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
                rail_points,
            );
            *contour =
                noded_owned_region_contour_with_rail_paths(contour, &source_points, owner_paths);
        }
    }
}

fn canonicalize_owned_region_contour_to_owner_source_points(
    contour: &mut NodeOverlayContour,
    owner: NodeBandOwner,
    source_points: &[NodeOwnershipPointKey],
    rail_points: &NodeRailCanonicalPointSet,
) {
    for point in contour.iter_mut() {
        let key = ownership_key_from_overlay_point(*point);
        if source_points.binary_search(&key).is_ok() {
            continue;
        }
        let Some(canonical) = rail_points.canonical_point_for_owner(owner, key) else {
            continue;
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
}

pub(super) fn insert_open_source_segments(
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

pub(super) fn canonical_points_by_mm_key_by_owner(
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

pub(super) fn validate_owned_region_vertices_against_source_authority(
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
        for contour in &region.shape {
            for point in contour
                .iter()
                .copied()
                .map(ownership_key_from_overlay_point)
            {
                if source_height_points.binary_search(&point).is_ok() {
                    continue;
                }
                if rail_points.owner_source_authorizes_point(region.owner, point) {
                    continue;
                }
                let Some(canonical) =
                    rail_points.conflicting_canonical_point_for_owner(region.owner, point)
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

impl NodeRailCanonicalPointSet {
    fn owner_source_authorizes_point(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> bool {
        if self
            .canonical_point_for_owner(owner, point)
            .is_some_and(|canonical| canonical != point)
        {
            return false;
        }
        self.points_by_owner
            .get(&owner)
            .is_some_and(|points| points.binary_search(&point).is_ok())
            || self.segments_by_owner.get(&owner).is_some_and(|segments| {
                segments
                    .iter()
                    .any(|segment| point_key_lies_on_segment(point, segment.start, segment.end))
            })
    }

    fn conflicting_canonical_point_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Option<NodeOwnershipPointKey> {
        self.canonical_point_for_owner(owner, point)
            .filter(|canonical| *canonical != point)
    }

    fn canonical_point_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Option<NodeOwnershipPointKey> {
        let candidates = self.canonical_candidates_for_owner(owner, point)?;
        candidates.iter().copied().next()
    }

    fn canonical_candidates_for_owner(
        &self,
        owner: NodeBandOwner,
        point: NodeOwnershipPointKey,
    ) -> Option<&BTreeSet<NodeOwnershipPointKey>> {
        self.canonical_points_by_mm_key_by_owner
            .get(&owner)?
            .get(&ownership_mm_key(point))
    }
}

pub(super) fn constraint_authority_owners(constraint: &NodeRailConstraint) -> Vec<NodeBandOwner> {
    let mut owners = [constraint.owner, constraint.opposite_owner]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners.dedup();
    owners
}

fn split_self_touching_owned_shape(
    shape: NodeOverlayShape,
    clean_numeric_spikes: bool,
) -> Vec<NodeOverlayShape> {
    let shape = cleaned_owned_shape(shape, clean_numeric_spikes);
    if shape.is_empty() {
        return Vec::new();
    }
    if shape.len() != 1 {
        return vec![shape];
    }
    let mut pending = vec![shape[0].clone()];
    let mut split_contours = Vec::new();
    while let Some(contour) = pending.pop() {
        let Some((first, second)) = first_repeated_owned_contour_point_pair(&contour) else {
            split_contours.push(contour);
            continue;
        };

        let first_cycle = contour[first..second].to_vec();
        let mut second_cycle = Vec::with_capacity(contour.len() - (second - first));
        second_cycle.extend_from_slice(&contour[second..]);
        second_cycle.extend_from_slice(&contour[..first]);

        for cycle in [first_cycle, second_cycle] {
            if let Some(cycle) = cleaned_self_touch_split_contour(cycle, clean_numeric_spikes) {
                pending.push(cycle);
            }
        }
    }

    if split_contours.is_empty() {
        Vec::new()
    } else {
        split_contours
            .into_iter()
            .map(|contour| vec![contour])
            .collect()
    }
}

fn cleaned_owned_shape(shape: NodeOverlayShape, clean_numeric_spikes: bool) -> NodeOverlayShape {
    shape
        .into_iter()
        .filter_map(|contour| cleaned_owned_contour(contour, clean_numeric_spikes))
        .collect()
}

fn first_repeated_owned_contour_point_pair(contour: &NodeOverlayContour) -> Option<(usize, usize)> {
    for first in 0..contour.len() {
        for second in first + 2..contour.len() {
            if first == 0 && second + 1 == contour.len() {
                continue;
            }
            if ownership_key_from_overlay_point(contour[first])
                == ownership_key_from_overlay_point(contour[second])
            {
                return Some((first, second));
            }
        }
    }
    None
}

fn cleaned_self_touch_split_contour(
    contour: NodeOverlayContour,
    clean_numeric_spikes: bool,
) -> Option<NodeOverlayContour> {
    cleaned_owned_contour(contour, clean_numeric_spikes)
}

fn cleaned_owned_contour(
    mut contour: NodeOverlayContour,
    clean_numeric_spikes: bool,
) -> Option<NodeOverlayContour> {
    dedup_consecutive_overlay_points(&mut contour);
    if clean_numeric_spikes {
        remove_numeric_spike_vertices(&mut contour);
    }
    if contour.len() >= 2
        && ownership_key_from_overlay_point(contour[0])
            == ownership_key_from_overlay_point(
                *contour.last().expect("split contour has last point"),
            )
    {
        contour.pop();
    }
    if contour.len() < 3 {
        return None;
    }
    if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
        contour.reverse();
    }
    let shape = vec![contour.clone()];
    let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&shape);
    (area_m2 > RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(&shape)).then_some(contour)
}

fn remove_numeric_spike_vertices(contour: &mut NodeOverlayContour) {
    loop {
        if contour.len() < 4 {
            return;
        }
        let mut removed = false;
        for index in 0..contour.len() {
            let previous = if index == 0 {
                contour.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == contour.len() {
                0
            } else {
                index + 1
            };
            let previous_key = ownership_key_from_overlay_point(contour[previous]);
            let current_key = ownership_key_from_overlay_point(contour[index]);
            let next_key = ownership_key_from_overlay_point(contour[next]);
            if previous_key == next_key
                || ownership_triangle_area_m2(previous_key, current_key, next_key)
                    <= f64::from(NODE_OVERLAY_MIN_AREA_M2)
            {
                contour.remove(index);
                removed = true;
                break;
            }
        }
        if !removed {
            return;
        }
    }
}

fn ownership_triangle_area_m2(
    a: NodeOwnershipPointKey,
    b: NodeOwnershipPointKey,
    c: NodeOwnershipPointKey,
) -> f64 {
    SurfaceXzKey::raw_tuple_triangle_area_m2_abs(a, b, c)
}

pub(super) fn owned_region_global_points(
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

fn noded_owned_region_contour(
    contour: &NodeOverlayContour,
    global_points: &[NodeOwnershipPointKey],
) -> NodeOverlayContour {
    noded_owned_region_contour_with_edge_points(contour, |start, end| {
        noded_owned_region_edge_points(start, end, global_points)
    })
}

fn noded_owned_region_contour_with_rail_paths(
    contour: &NodeOverlayContour,
    global_points: &[NodeOwnershipPointKey],
    rail_paths: &[Vec<NodeOwnershipPointKey>],
) -> NodeOverlayContour {
    noded_owned_region_contour_with_edge_points(contour, |start, end| {
        noded_owned_region_edge_points_with_rail_paths(start, end, global_points, rail_paths)
    })
}

fn noded_owned_region_contour_with_edge_points(
    contour: &NodeOverlayContour,
    mut edge_points: impl FnMut(
        NodeOwnershipPointKey,
        NodeOwnershipPointKey,
    ) -> Vec<NodeOwnershipPointKey>,
) -> NodeOverlayContour {
    if contour.len() < 2 {
        return contour.clone();
    }

    let mut noded = Vec::with_capacity(contour.len());
    for edge_index in 0..contour.len() {
        let start = ownership_key_from_overlay_point(contour[edge_index]);
        let end = ownership_key_from_overlay_point(contour[(edge_index + 1) % contour.len()]);
        if start == end {
            continue;
        }
        let points = edge_points(start, end);
        let limit = points.len().saturating_sub(1);
        noded.extend(points.into_iter().take(limit).map(overlay_point_from_key));
    }
    dedup_consecutive_overlay_points(&mut noded);
    if noded.len() >= 2
        && ownership_key_from_overlay_point(noded[0])
            == ownership_key_from_overlay_point(
                *noded.last().expect("noded contour has last point"),
            )
    {
        noded.pop();
    }
    noded
}

fn noded_owned_region_edge_points_with_rail_paths(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    global_points: &[NodeOwnershipPointKey],
    rail_paths: &[Vec<NodeOwnershipPointKey>],
) -> Vec<NodeOwnershipPointKey> {
    rail_path_points_between(start, end, rail_paths)
        .unwrap_or_else(|| noded_owned_region_edge_points(start, end, global_points))
}

fn dedup_consecutive_overlay_points(points: &mut NodeOverlayContour) {
    points.dedup_by(|a, b| {
        ownership_key_from_overlay_point(*a) == ownership_key_from_overlay_point(*b)
    });
}

pub(super) fn noded_owned_region_edge_points(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    global_points: &[NodeOwnershipPointKey],
) -> Vec<NodeOwnershipPointKey> {
    let mut split_points = global_points
        .iter()
        .copied()
        .filter(|point| *point != start && *point != end)
        .filter(|point| point_key_lies_exactly_on_segment(*point, start, end))
        .collect::<Vec<_>>();
    split_points.sort_by_key(|point| segment_parameter_key(start, end, *point));
    split_points.dedup();

    let mut points = Vec::with_capacity(split_points.len() + 2);
    points.push(start);
    points.extend(split_points);
    points.push(end);
    points
}

fn rail_path_points_between(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    rail_paths: &[Vec<NodeOwnershipPointKey>],
) -> Option<Vec<NodeOwnershipPointKey>> {
    if start == end {
        return None;
    }
    let mut best = None;
    for points in rail_paths {
        for start_index in points
            .iter()
            .enumerate()
            .filter_map(|(index, point)| (*point == start).then_some(index))
        {
            for end_index in start_index + 1..points.len() {
                if points[end_index] != end {
                    continue;
                }
                let mut candidate = points[start_index..=end_index].to_vec();
                dedup_consecutive_ownership_keys(&mut candidate);
                if candidate.len() == 3
                    && best
                        .as_ref()
                        .is_none_or(|best: &Vec<NodeOwnershipPointKey>| {
                            candidate.len() > best.len()
                        })
                {
                    best = Some(candidate);
                }
            }
        }
        for end_index in points
            .iter()
            .enumerate()
            .filter_map(|(index, point)| (*point == end).then_some(index))
        {
            for start_index in end_index + 1..points.len() {
                if points[start_index] != start {
                    continue;
                }
                let mut candidate = points[end_index..=start_index].to_vec();
                candidate.reverse();
                dedup_consecutive_ownership_keys(&mut candidate);
                if candidate.len() == 3
                    && best
                        .as_ref()
                        .is_none_or(|best: &Vec<NodeOwnershipPointKey>| {
                            candidate.len() > best.len()
                        })
                {
                    best = Some(candidate);
                }
            }
        }
    }
    best
}

fn dedup_consecutive_ownership_keys(points: &mut Vec<NodeOwnershipPointKey>) {
    points.dedup();
}
