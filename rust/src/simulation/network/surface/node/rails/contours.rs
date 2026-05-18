//! Generic generated contour, constraint, and path helpers for node rails.

use super::super::arrangement::NodeBandOwner;
use super::super::backend::{
    RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points, road_points_to_polyline,
    road_vec3_xz as xz,
};
use super::super::input::{NodeInputBoundaryRailRole, NodeInputMouth, NodeInputProfileRail};
use super::super::keys::SurfaceHeightMmKey;
use super::super::segments::raw_tuple_key_lies_on_segment as generated_point_key_lies_on_segment;
use super::super::{NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind};
use super::constraints::{GeneratedRaisedStepOwnerPair, boundary_constraint_kind};
use super::geometry::road_point_key;
use super::topology::NodeRailPointKey;
use super::{
    NodeGeneratedContour, NodeGeneratedContourClaimPriority, NodeGeneratedContourKind,
    NodeGeneratedContourPurpose, NodeRailConstraint, NodeRailConstraintKind,
    NodeRailGenerationError, RAIL_CONTOUR_POINT_EQUAL_EPS_M,
};
use cavalier_contours::polyline::{PlineCreation, PlineSource, PlineSourceMut};
use std::collections::BTreeMap;

pub(super) fn push_path_strip_contours(
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    validate_paired_path_band_height_carrier(
        kind,
        mouth_order_index,
        band_index,
        start_path_world,
        end_path_world,
    )?;
    let mut first_error = None;
    let mut pushed = false;
    for points_world in path_strip_contours_world(start_path_world, end_path_world) {
        let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
        match push_generated_contour_with_purpose(
            kind,
            purpose,
            mouth_order_index,
            band_index,
            owner,
            claim_priority,
            constraint_kind,
            points,
            Some(points_world),
            contours,
            constraints,
        ) {
            Ok(()) => pushed = true,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        };
    }
    if pushed {
        Ok(())
    } else {
        Err(
            first_error.unwrap_or(NodeRailGenerationError::DegenerateContour {
                kind,
                mouth_order_index,
                band_index,
                area_m2: 0.0,
                vertex_count: 0,
            }),
        )
    }
}

pub(super) fn push_generated_contour(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    points: Vec<RoadVec2>,
    height_points_world: Option<Vec<RoadVec3>>,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let purpose = default_generated_contour_purpose(kind);
    push_generated_contour_with_purpose(
        kind,
        purpose,
        mouth_order_index,
        band_index,
        owner,
        claim_priority,
        constraint_kind,
        points,
        height_points_world,
        contours,
        constraints,
    )
}

pub(super) fn push_generated_contour_with_purpose(
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    points: Vec<RoadVec2>,
    height_points_world: Option<Vec<RoadVec3>>,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let contour = cleaned_closed_contour(kind, mouth_order_index, band_index, points)?;
    let points_xz = polyline_to_road_points(&contour);
    let height_points_world = match height_points_world.as_deref() {
        Some(points_world) => Some(
            align_height_points_to_contour(&points_xz, points_world).ok_or(
                NodeRailGenerationError::InvalidHeightCarrier {
                    kind,
                    mouth_order_index,
                    band_index,
                    reason: "height_points_do_not_match_contour",
                },
            )?,
        ),
        None => None,
    };
    contours.push(NodeGeneratedContour {
        kind,
        purpose,
        source_mouth_order_index: mouth_order_index,
        source_band_index: band_index,
        owner,
        claim_priority,
        points_xz: points_xz.clone(),
        height_points_world,
        backend_polyline: contour,
    });
    push_constraint(
        constraints,
        constraint_kind,
        mouth_order_index,
        band_index,
        None,
        owner,
        None,
        points_xz,
    )
}

pub(super) fn default_generated_contour_purpose(
    kind: NodeGeneratedContourKind,
) -> NodeGeneratedContourPurpose {
    match kind {
        NodeGeneratedContourKind::FullRoadbed => NodeGeneratedContourPurpose::FullRoadbedCorridor,
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        } => NodeGeneratedContourPurpose::CarriagewayCorridor,
        NodeGeneratedContourKind::Band { .. } => NodeGeneratedContourPurpose::NonRoadBand,
    }
}

pub(super) fn push_path_band_contour(
    kind: NodeGeneratedContourKind,
    purpose: NodeGeneratedContourPurpose,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    validate_paired_path_band_height_carrier(
        kind,
        mouth_order_index,
        band_index,
        start_path_world,
        end_path_world,
    )?;
    let mut points_world = Vec::with_capacity(start_path_world.len() + end_path_world.len());
    append_world_path_points(&mut points_world, start_path_world.iter());
    append_world_path_points(&mut points_world, end_path_world.iter().rev());
    remove_closing_world_path_duplicate(&mut points_world);
    let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
    push_generated_contour_with_purpose(
        kind,
        purpose,
        mouth_order_index,
        band_index,
        owner,
        claim_priority,
        constraint_kind,
        points,
        Some(points_world),
        contours,
        constraints,
    )
}

fn path_strip_contours_world(
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Vec<Vec<RoadVec3>> {
    let point_count = start_path_world.len();
    if point_count < 2 {
        return Vec::new();
    }
    let mut strips = Vec::with_capacity(point_count - 1);
    for index in 0..point_count - 1 {
        let mut points = Vec::with_capacity(4);
        push_world_path_point(&mut points, start_path_world[index]);
        push_world_path_point(&mut points, end_path_world[index]);
        push_world_path_point(&mut points, end_path_world[index + 1]);
        push_world_path_point(&mut points, start_path_world[index + 1]);
        remove_closing_world_path_duplicate(&mut points);
        strips.push(points);
    }
    strips
}

fn validate_paired_path_band_height_carrier(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Result<(), NodeRailGenerationError> {
    if start_path_world.len() != end_path_world.len() {
        return Err(NodeRailGenerationError::InvalidHeightCarrier {
            kind,
            mouth_order_index,
            band_index,
            reason: "mismatched_path_height_carrier_lengths",
        });
    }
    if start_path_world.len() < 2 {
        return Err(NodeRailGenerationError::InvalidHeightCarrier {
            kind,
            mouth_order_index,
            band_index,
            reason: "too_few_path_height_carrier_points",
        });
    }
    Ok(())
}

pub(super) fn subdivided_world_chord(
    start: RoadVec3,
    end: RoadVec3,
    point_count: usize,
) -> Vec<RoadVec3> {
    if point_count < 2 {
        return vec![start, end];
    }
    (0..point_count)
        .map(|index| {
            let t = index as f64 / (point_count - 1) as f64;
            start + (end - start) * t
        })
        .collect()
}

pub(super) fn push_generated_band_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: usize,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    start: RoadVec2,
    end: RoadVec2,
) -> Result<(), NodeRailGenerationError> {
    if road_point_key(start) == road_point_key(end) {
        return Ok(());
    }
    push_owned_rail_constraint(
        constraints,
        kind,
        source_mouth_order_index,
        Some(source_band_index),
        None,
        Some(owner),
        opposite_owner,
        vec![start, end],
    )
}

pub(super) fn push_generated_band_path_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: usize,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    points: Vec<RoadVec2>,
) -> Result<(), NodeRailGenerationError> {
    let Some(points) = clean_generated_constraint_path(points) else {
        return Ok(());
    };
    push_owned_rail_constraint(
        constraints,
        kind,
        source_mouth_order_index,
        Some(source_band_index),
        None,
        Some(owner),
        opposite_owner,
        points,
    )
}

pub(super) fn clean_generated_constraint_path(points: Vec<RoadVec2>) -> Option<Vec<RoadVec2>> {
    let mut cleaned = Vec::with_capacity(points.len());
    for point in points {
        push_road_path_point(&mut cleaned, point);
    }
    if cleaned
        .windows(2)
        .any(|segment| road_point_key(segment[0]) != road_point_key(segment[1]))
    {
        let raw = road_points_to_polyline(cleaned, false);
        let rail = RoadPolyline::create_from_remove_repeat(&raw, RAIL_CONTOUR_POINT_EQUAL_EPS_M);
        (rail.vertex_count() >= 2 && rail.path_length() > RAIL_CONTOUR_POINT_EQUAL_EPS_M)
            .then(|| polyline_to_road_points(&rail))
    } else {
        None
    }
}

pub(super) fn open_world_path_xz(
    path_world: &[RoadVec3],
    mouth_world: RoadVec3,
    endpoint_world: RoadVec3,
) -> Vec<RoadVec2> {
    let mut points = Vec::new();
    push_road_path_point(&mut points, xz(mouth_world));
    append_world_path_xz(&mut points, path_world.iter());
    push_road_path_point(&mut points, xz(endpoint_world));
    points
}

pub(super) fn append_world_path_xz<'a>(
    points: &mut Vec<RoadVec2>,
    path_world: impl IntoIterator<Item = &'a RoadVec3>,
) {
    for point in path_world {
        push_road_path_point(points, xz(*point));
    }
}

pub(super) fn append_world_path_points<'a>(
    points: &mut Vec<RoadVec3>,
    path_world: impl IntoIterator<Item = &'a RoadVec3>,
) {
    for point in path_world {
        push_world_path_point(points, *point);
    }
}

pub(super) fn push_road_path_point(points: &mut Vec<RoadVec2>, point: RoadVec2) {
    if points
        .last()
        .is_none_or(|last| road_point_key(*last) != road_point_key(point))
    {
        points.push(point);
    }
}

pub(super) fn push_world_path_point(points: &mut Vec<RoadVec3>, point: RoadVec3) {
    if points.last().is_none_or(|last| {
        road_point_key(xz(*last)) != road_point_key(xz(point))
            || SurfaceHeightMmKey::from_m_f64(last.y) != SurfaceHeightMmKey::from_m_f64(point.y)
    }) {
        points.push(point);
    }
}

pub(super) fn remove_closing_road_path_duplicate(points: &mut Vec<RoadVec2>) {
    if points.len() > 1
        && road_point_key(points[0]) == road_point_key(*points.last().expect("len checked"))
    {
        points.pop();
    }
}

pub(super) fn remove_closing_world_path_duplicate(points: &mut Vec<RoadVec3>) {
    if points.len() > 1
        && road_point_key(xz(points[0])) == road_point_key(xz(*points.last().expect("len checked")))
        && SurfaceHeightMmKey::from_m_f64(points[0].y)
            == SurfaceHeightMmKey::from_m_f64(points.last().expect("len checked").y)
    {
        points.pop();
    }
}

pub(super) fn align_height_points_to_contour(
    contour_points_xz: &[RoadVec2],
    source_points_world: &[RoadVec3],
) -> Option<Vec<RoadVec3>> {
    let mut height_by_key = BTreeMap::<NodeRailPointKey, f64>::new();
    for point in source_points_world {
        let key = road_point_key(xz(*point));
        if let Some(existing_height_m) = height_by_key.get(&key)
            && (*existing_height_m - point.y).abs() > f64::EPSILON
        {
            return None;
        }
        height_by_key.insert(key, point.y);
    }
    contour_points_xz
        .iter()
        .copied()
        .map(|point_xz| {
            height_by_key
                .get(&road_point_key(point_xz))
                .copied()
                .map(|height_m| RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect()
}

pub(super) fn align_height_points_to_source_contours(
    contour_points_xz: &[RoadVec2],
    source_contours_world: &[&[RoadVec3]],
) -> Option<Vec<RoadVec3>> {
    contour_points_xz
        .iter()
        .copied()
        .map(|point_xz| {
            height_on_source_contours(point_xz, source_contours_world)
                .map(|height_m| RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect()
}

fn height_on_source_contours(
    point_xz: RoadVec2,
    source_contours_world: &[&[RoadVec3]],
) -> Option<f64> {
    let key = road_point_key(point_xz);
    let mut height_m: Option<f64> = None;
    for source_contour_world in source_contours_world {
        if let Some(candidate_height_m) = height_on_source_contour_edge(key, source_contour_world) {
            if let Some(existing_height_m) = height_m
                && (existing_height_m - candidate_height_m).abs() > f64::EPSILON
            {
                return None;
            }
            height_m = Some(candidate_height_m);
        }
    }
    height_m
}

fn height_on_source_contour_edge(
    key: NodeRailPointKey,
    source_points_world: &[RoadVec3],
) -> Option<f64> {
    if source_points_world.is_empty() {
        return None;
    }
    for point in source_points_world {
        if road_point_key(xz(*point)) == key {
            return Some(point.y);
        }
    }
    for index in 0..source_points_world.len() {
        let next = (index + 1) % source_points_world.len();
        let start = road_point_key(xz(source_points_world[index]));
        let end = road_point_key(xz(source_points_world[next]));
        if start == end || !generated_point_key_lies_on_segment(key, start, end) {
            continue;
        }
        if let Some(height_m) = height_for_key_on_generated_edge(
            key,
            start,
            end,
            source_points_world[index].y,
            source_points_world[next].y,
        ) {
            return Some(height_m);
        }
    }
    None
}

pub(super) fn push_boundary_constraint(
    mouth: &NodeInputMouth,
    boundary_index: usize,
    role: NodeInputBoundaryRailRole,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let rail = &mouth.boundary_rails[boundary_index];
    push_owned_rail_constraint(
        constraints,
        boundary_constraint_kind(role),
        mouth.order_index,
        None,
        Some(boundary_index),
        owner,
        opposite_owner,
        open_world_path_xz(&rail.path_world, rail.mouth_world, rail.endpoint_world),
    )
}

pub(super) fn push_span_handoff_constraint(
    mouth: &NodeInputMouth,
    profile_rail: &NodeInputProfileRail,
    owner: NodeBandOwner,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    push_constraint(
        constraints,
        NodeRailConstraintKind::SpanHandoff {
            kind: profile_rail.band_kind,
        },
        mouth.order_index,
        Some(profile_rail.band_index),
        None,
        Some(owner),
        None,
        vec![xz(profile_rail.start_world), xz(profile_rail.end_world)],
    )
}

fn push_owned_rail_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    source_boundary_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    points: Vec<RoadVec2>,
) -> Result<(), NodeRailGenerationError> {
    if kind == NodeRailConstraintKind::RaisedStepContact {
        let (Some(owner), Some(opposite_owner)) = (owner, opposite_owner) else {
            return Ok(());
        };
        let Some(pair) = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner) else {
            return Ok(());
        };
        return push_constraint(
            constraints,
            kind,
            source_mouth_order_index,
            source_band_index,
            source_boundary_index,
            Some(pair.owner),
            Some(pair.opposite_owner),
            points,
        );
    }
    push_constraint(
        constraints,
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index,
        owner,
        opposite_owner,
        points,
    )
}

pub(super) fn push_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    source_boundary_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    points: Vec<RoadVec2>,
) -> Result<(), NodeRailGenerationError> {
    let polyline = cleaned_open_rail(
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index,
        points,
    )?;
    let constraint_index = constraints.len();
    constraints.push(NodeRailConstraint {
        constraint_index,
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index,
        owner,
        opposite_owner,
        points_xz: polyline_to_road_points(&polyline),
    });
    Ok(())
}

pub(super) fn height_for_key_on_generated_edge(
    point: NodeRailPointKey,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    start_height_m: f64,
    end_height_m: f64,
) -> Option<f64> {
    if start == end || !generated_point_key_lies_on_segment(point, start, end) {
        return None;
    }
    let dx = end.0 - start.0;
    let dz = end.1 - start.1;
    let denominator = if dx.abs() >= dz.abs() { dx } else { dz };
    if denominator == 0 {
        return None;
    }
    let numerator = if dx.abs() >= dz.abs() {
        point.0 - start.0
    } else {
        point.1 - start.1
    };
    let t = numerator as f64 / denominator as f64;
    Some(start_height_m + (end_height_m - start_height_m) * t)
}

pub(super) fn cleaned_closed_contour(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    points: Vec<RoadVec2>,
) -> Result<RoadPolyline, NodeRailGenerationError> {
    let raw = road_points_to_polyline(points, true);
    let mut contour = RoadPolyline::create_from_remove_repeat(&raw, RAIL_CONTOUR_POINT_EQUAL_EPS_M);
    if contour.area() < 0.0 {
        contour.invert_direction_mut();
    }
    let area_m2 = contour.area().abs();
    if contour.vertex_count() < 3 || area_m2 <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
        return Err(NodeRailGenerationError::DegenerateContour {
            kind,
            mouth_order_index,
            band_index,
            area_m2,
            vertex_count: contour.vertex_count(),
        });
    }
    Ok(contour)
}

fn cleaned_open_rail(
    kind: NodeRailConstraintKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    boundary_index: Option<usize>,
    points: Vec<RoadVec2>,
) -> Result<RoadPolyline, NodeRailGenerationError> {
    let raw = road_points_to_polyline(points, false);
    let rail = RoadPolyline::create_from_remove_repeat(&raw, RAIL_CONTOUR_POINT_EQUAL_EPS_M);
    let path_length_m = rail.path_length();
    if rail.vertex_count() < 2 || path_length_m <= RAIL_CONTOUR_POINT_EQUAL_EPS_M {
        return Err(NodeRailGenerationError::DegenerateConstraint {
            kind,
            mouth_order_index,
            band_index,
            boundary_index,
            path_length_m,
            vertex_count: rail.vertex_count(),
        });
    }
    Ok(rail)
}
