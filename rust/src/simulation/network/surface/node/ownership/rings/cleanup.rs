//! Owned-region ring cleanup and self-touch splitting.

use super::super::super::arrangement::NodeBandOwner;
use super::super::super::keys::SurfaceXzKey;
use super::super::super::rails::{NodeGeneratedContourClaimPriority, NodeRailConstraint};
use super::super::super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayShape, NodeOverlayShapes,
    RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};
use super::super::domains::overlay_union;
use super::super::rail_authority::NodeRailCanonicalPointSet;
use super::super::seams::{ConstraintOverlapMode, owned_shape_is_discardable_numeric_dust};
use super::super::topology_keys::{NodeOwnershipPointKey, ownership_key_from_overlay_point};
use super::super::{NodeBooleanOwnedRegion, NodeBooleanOwnershipError};
use super::noding::{
    canonicalize_final_join_or_cap_owned_region_boundary_edges,
    canonicalize_final_owned_region_boundary_edges_for_piece_kind,
    canonicalize_owned_region_rings_with_rail_point_set_for_piece_kind,
    dedup_consecutive_overlay_points,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
struct OwnedRegionRebuildSource {
    kind: RoadSurfaceBandKind,
    owner: NodeBandOwner,
    claim_priority: NodeGeneratedContourClaimPriority,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
}

impl OwnedRegionRebuildSource {
    fn from_region(region: &NodeBooleanOwnedRegion) -> Self {
        Self {
            kind: region.kind,
            owner: region.owner,
            claim_priority: region.claim_priority,
            source_mouth_order_index: region.source_mouth_order_index,
            source_band_index: region.source_band_index,
        }
    }
}

pub(in crate::simulation::network::surface::node::ownership) fn clean_canonical_owned_region_shapes(
    regions: &mut Vec<NodeBooleanOwnedRegion>,
    footprint_shapes: &NodeOverlayShapes,
    rail_constraints: &[NodeRailConstraint],
    rail_canonical_points: &NodeRailCanonicalPointSet,
    overlap_mode: ConstraintOverlapMode,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> Result<(), NodeBooleanOwnershipError> {
    let mut cleaned_regions = Vec::with_capacity(regions.len());
    for region in regions.drain(..) {
        let source = OwnedRegionRebuildSource::from_region(&region);
        let shapes = cleaned_or_unioned_owned_region_shapes(
            region.shape,
            overlap_mode.cleans_overlay_numeric_spikes(),
            "owned_region_ring_clean",
        )?;
        for shape in shapes.into_iter().flat_map(|shape| {
            split_self_touching_owned_shape(shape, overlap_mode.cleans_overlay_numeric_spikes())
        }) {
            if let Some(region) = rebuilt_owned_region_for_shape(source, shape, rail_constraints) {
                cleaned_regions.push(region);
            }
        }
    }
    canonicalize_owned_region_rings_with_rail_point_set_for_piece_kind(
        &mut cleaned_regions,
        rail_canonical_points,
        piece_kind,
    )?;
    clean_owned_region_shapes_once(&mut cleaned_regions, rail_constraints, overlap_mode)?;
    canonicalize_owned_region_rings_with_rail_point_set_for_piece_kind(
        &mut cleaned_regions,
        rail_canonical_points,
        piece_kind,
    )?;
    canonicalize_final_owned_region_boundary_edges_for_piece_kind(
        &mut cleaned_regions,
        footprint_shapes,
        rail_canonical_points,
        piece_kind,
    )?;
    clean_owned_region_shapes_once(&mut cleaned_regions, rail_constraints, overlap_mode)?;
    canonicalize_final_owned_region_boundary_edges_for_piece_kind(
        &mut cleaned_regions,
        footprint_shapes,
        rail_canonical_points,
        piece_kind,
    )?;
    split_final_canonical_owned_region_self_touches(
        &mut cleaned_regions,
        rail_constraints,
        ConstraintOverlapMode::ExactCanonical,
    );
    canonicalize_owned_region_rings_with_rail_point_set_for_piece_kind(
        &mut cleaned_regions,
        rail_canonical_points,
        piece_kind,
    )?;
    clean_owned_region_shapes_once(&mut cleaned_regions, rail_constraints, overlap_mode)?;
    canonicalize_final_join_or_cap_owned_region_boundary_edges(
        &mut cleaned_regions,
        footprint_shapes,
        rail_canonical_points,
    )?;
    *regions = cleaned_regions;
    Ok(())
}

fn split_final_canonical_owned_region_self_touches(
    regions: &mut Vec<NodeBooleanOwnedRegion>,
    rail_constraints: &[NodeRailConstraint],
    overlap_mode: ConstraintOverlapMode,
) {
    let mut split_regions = Vec::with_capacity(regions.len());
    for region in regions.drain(..) {
        let source = OwnedRegionRebuildSource::from_region(&region);
        for shape in split_self_touching_owned_shape(
            region.shape,
            overlap_mode.cleans_overlay_numeric_spikes(),
        ) {
            if let Some(region) = rebuilt_owned_region_for_shape(source, shape, rail_constraints) {
                split_regions.push(region);
            }
        }
    }
    *regions = split_regions;
}

fn clean_owned_region_shapes_once(
    regions: &mut Vec<NodeBooleanOwnedRegion>,
    rail_constraints: &[NodeRailConstraint],
    overlap_mode: ConstraintOverlapMode,
) -> Result<(), NodeBooleanOwnershipError> {
    let mut cleaned_regions = Vec::with_capacity(regions.len());
    for region in regions.drain(..) {
        let source = OwnedRegionRebuildSource::from_region(&region);
        let shapes = cleaned_or_unioned_owned_region_shapes(
            region.shape,
            overlap_mode.cleans_overlay_numeric_spikes(),
            "owned_region_constraint_noded_clean",
        )?;
        for shape in shapes.into_iter().flat_map(|shape| {
            split_self_touching_owned_shape(shape, overlap_mode.cleans_overlay_numeric_spikes())
        }) {
            if let Some(region) = rebuilt_owned_region_for_shape(source, shape, rail_constraints) {
                cleaned_regions.push(region);
            }
        }
    }
    *regions = cleaned_regions;
    Ok(())
}

fn cleaned_or_unioned_owned_region_shapes(
    shape: NodeOverlayShape,
    clean_numeric_spikes: bool,
    union_stage: &'static str,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let cleaned = cleaned_owned_shape(shape.clone(), clean_numeric_spikes);
    if cleaned.is_empty() {
        return Ok(Vec::new());
    }
    if owned_shape_is_single_simple_contour(&cleaned) {
        return Ok(vec![cleaned]);
    }
    let mut shapes = overlay_union(&shape, union_stage)?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

fn rebuilt_owned_region_for_shape(
    source: OwnedRegionRebuildSource,
    shape: NodeOverlayShape,
    rail_constraints: &[NodeRailConstraint],
) -> Option<NodeBooleanOwnedRegion> {
    let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&shape);
    if owned_shape_is_discardable_numeric_dust(&shape, area_m2, source.owner, rail_constraints) {
        return None;
    }
    Some(NodeBooleanOwnedRegion {
        kind: source.kind,
        owner: source.owner,
        claim_priority: source.claim_priority,
        source_mouth_order_index: source.source_mouth_order_index,
        source_band_index: source.source_band_index,
        shape,
        area_m2,
        seam_constraints: Vec::new(),
    })
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

fn owned_shape_is_single_simple_contour(shape: &NodeOverlayShape) -> bool {
    let [contour] = shape.as_slice() else {
        return false;
    };
    if contour.len() < 3 {
        return false;
    }
    let mut seen = BTreeSet::new();
    for point in contour {
        if !seen.insert(ownership_key_from_overlay_point(*point)) {
            return false;
        }
    }
    !owned_contour_has_strict_self_intersection(contour)
}

fn owned_contour_has_strict_self_intersection(contour: &NodeOverlayContour) -> bool {
    for first in 0..contour.len() {
        let first_next = (first + 1) % contour.len();
        let first_start = ownership_key_from_overlay_point(contour[first]);
        let first_end = ownership_key_from_overlay_point(contour[first_next]);
        for second in first + 1..contour.len() {
            let second_next = (second + 1) % contour.len();
            if first == second || first_next == second || second_next == first {
                continue;
            }
            let second_start = ownership_key_from_overlay_point(contour[second]);
            let second_end = ownership_key_from_overlay_point(contour[second_next]);
            if ownership_segments_strictly_intersect(
                first_start,
                first_end,
                second_start,
                second_end,
            ) {
                return true;
            }
        }
    }
    false
}

fn ownership_segments_strictly_intersect(
    a: NodeOwnershipPointKey,
    b: NodeOwnershipPointKey,
    c: NodeOwnershipPointKey,
    d: NodeOwnershipPointKey,
) -> bool {
    if a == b || c == d || a == c || a == d || b == c || b == d {
        return false;
    }
    let ab_c = ownership_orientation(a, b, c);
    let ab_d = ownership_orientation(a, b, d);
    let cd_a = ownership_orientation(c, d, a);
    let cd_b = ownership_orientation(c, d, b);
    ab_c != 0
        && ab_d != 0
        && cd_a != 0
        && cd_b != 0
        && ab_c.signum() != ab_d.signum()
        && cd_a.signum() != cd_b.signum()
}

fn ownership_orientation(
    a: NodeOwnershipPointKey,
    b: NodeOwnershipPointKey,
    c: NodeOwnershipPointKey,
) -> i128 {
    let ab_x = i128::from(b.0 - a.0);
    let ab_z = i128::from(b.1 - a.1);
    let ac_x = i128::from(c.0 - a.0);
    let ac_z = i128::from(c.1 - a.1);
    ab_x * ac_z - ab_z * ac_x
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
