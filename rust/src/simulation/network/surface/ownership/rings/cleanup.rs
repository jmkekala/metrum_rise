//! Owned-region ring cleanup and self-touch splitting.

use super::super::super::keys::SurfaceXzKey;
use super::super::super::rails::NodeRailConstraint;
use super::super::super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayShape, NodeOverlayShapes,
    RoadSurfaceSystem,
};
use super::super::domains::overlay_union;
use super::super::rail_authority::{
    NodeRailCanonicalPointSet, validate_owned_region_vertices_against_source_authority,
};
use super::super::seams::{owned_shape_is_discardable_numeric_dust, seam_constraints_for_shape};
use super::super::topology_keys::{NodeOwnershipPointKey, ownership_key_from_overlay_point};
use super::super::{NodeBooleanOwnedRegion, NodeBooleanOwnershipError};
use super::noding::{
    canonicalize_final_owned_region_boundary_edges,
    canonicalize_owned_region_rings_with_rail_point_set, dedup_consecutive_overlay_points,
};

pub(in crate::simulation::network::surface::ownership) fn clean_canonical_owned_region_shapes(
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
