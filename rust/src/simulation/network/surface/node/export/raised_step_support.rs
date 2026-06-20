//! Raised-step vertical face support checks against final owned top surfaces.

use super::super::{
    ArrangementBoundaryPointKey, NodeOwnedRegion, RoadSurfaceRaisedStepFace,
    RoadSurfaceVerticalFaceSource,
    arrangement::{NodeBandOwner, NodeExplicitVerticalStepSegment},
    keys, segments,
};
use crate::simulation::network::surface::{
    NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualPolygon, RoadVec3, band_semantics::ordered_raised_step_kinds,
};
use std::collections::{BTreeMap, BTreeSet};

const TOP_SUPPORT_EDGE_TILE_KEYS: i64 = 8_000_000;
const RAISED_STEP_FACE_HEIGHT_DUST_MM: i64 = 1;

impl RoadSurfaceSystem {
    pub(super) fn retain_raised_step_faces_with_owned_top_support(
        raised_step_faces: &mut Vec<RoadSurfaceRaisedStepFace>,
        owned_regions: &[NodeOwnedRegion],
        explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
    ) {
        let top_edges = owned_top_boundary_edges(owned_regions);
        let required_spans =
            final_required_raised_step_spans(explicit_vertical_step_segments, &top_edges);
        let region_centroids = raised_step_region_centroids(owned_regions);
        let owner_centroids = raised_step_owner_centroids(owned_regions);
        complete_raised_step_faces_from_final_spans(
            raised_step_faces,
            &required_spans,
            &region_centroids,
            &owner_centroids,
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeTopSupportVertexKey {
    xz: keys::SurfaceXzKey,
    y_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeTopSupportEdgeKey {
    start: NodeTopSupportVertexKey,
    end: NodeTopSupportVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeTopSupportEdge {
    owner: NodeBandOwner,
    region_index: usize,
    start: NodeTopSupportVertexKey,
    end: NodeTopSupportVertexKey,
}

#[derive(Clone, Copy, Debug)]
struct NodeTopSupportEdgeCandidate {
    edge: NodeTopSupportEdge,
    bounds: SurfaceKeyBounds,
}

#[derive(Clone, Debug, Default)]
struct NodeTopSupportEdgeIndex {
    edges_by_kind: BTreeMap<RoadSurfaceBandKind, Vec<NodeTopSupportEdgeCandidate>>,
    tile_indices_by_kind: BTreeMap<RoadSurfaceBandKind, BTreeMap<SurfaceKeyTile, Vec<usize>>>,
}

#[derive(Clone, Copy, Debug)]
struct SurfaceKeyBounds {
    min_x: i64,
    min_z: i64,
    max_x: i64,
    max_z: i64,
}

#[derive(Clone, Copy, Debug)]
struct FinalRequiredRaisedStepSpan {
    lower_edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
    boundary_keys: RaisedStepBoundaryPointKeys,
    support_key: RaisedStepFaceSupportKey,
    source: RoadSurfaceVerticalFaceSource,
}

#[derive(Clone, Copy, Debug)]
struct RaisedStepBoundaryPointKeys {
    lower_start: ArrangementBoundaryPointKey,
    lower_end: ArrangementBoundaryPointKey,
    raised_start: ArrangementBoundaryPointKey,
    raised_end: ArrangementBoundaryPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RaisedStepFaceSupportKey {
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    lower_edge: NodeTopSupportEdgeKey,
    upper_edge: NodeTopSupportEdgeKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SurfaceKeyTile {
    x: i64,
    z: i64,
}

fn owned_top_boundary_edges(owned_regions: &[NodeOwnedRegion]) -> Vec<NodeTopSupportEdge> {
    let mut edge_counts_by_owner = BTreeMap::<
        NodeBandOwner,
        BTreeMap<
            NodeTopSupportEdgeKey,
            (
                usize,
                NodeTopSupportVertexKey,
                NodeTopSupportVertexKey,
                usize,
            ),
        >,
    >::new();
    for (region_index, region) in owned_regions.iter().enumerate() {
        let owner = NodeBandOwner::new(region.kind, region.owner_index);
        let edge_counts = edge_counts_by_owner.entry(owner).or_default();
        for (edge_key, edge_start, edge_end) in final_polygon_boundary_edges(&region.polygon) {
            edge_counts
                .entry(edge_key)
                .and_modify(|entry| entry.0 += 1)
                .or_insert((1, edge_start, edge_end, region_index));
        }
    }

    let mut top_edges = Vec::new();
    for (owner, edge_counts) in edge_counts_by_owner {
        top_edges.extend(edge_counts.into_iter().filter_map(
            |(_key, (count, start, end, region_index))| {
                (count == 1).then_some(NodeTopSupportEdge {
                    owner,
                    region_index,
                    start,
                    end,
                })
            },
        ));
    }
    top_edges
}

fn final_polygon_boundary_edges(
    polygon: &RoadSurfaceVisualPolygon,
) -> Vec<(
    NodeTopSupportEdgeKey,
    NodeTopSupportVertexKey,
    NodeTopSupportVertexKey,
)> {
    let mut edges = Vec::new();
    if polygon.triangles_world.is_empty() {
        push_loop_edges(&polygon.points_world, &mut edges);
        return edges;
    }
    for triangle in &polygon.triangles_world {
        for edge_index in 0..3 {
            if let Some(edge) = top_support_edge_from_world_points(
                triangle[edge_index],
                triangle[(edge_index + 1) % 3],
            ) {
                edges.push(edge);
            }
        }
    }
    edges
}

fn push_loop_edges(
    points: &[RoadVec3],
    edges: &mut Vec<(
        NodeTopSupportEdgeKey,
        NodeTopSupportVertexKey,
        NodeTopSupportVertexKey,
    )>,
) {
    if points.len() < 2 {
        return;
    }
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        if let Some(edge) = top_support_edge_from_world_points(points[index], points[next]) {
            edges.push(edge);
        }
    }
}

fn top_support_edge_from_world_points(
    start: RoadVec3,
    end: RoadVec3,
) -> Option<(
    NodeTopSupportEdgeKey,
    NodeTopSupportVertexKey,
    NodeTopSupportVertexKey,
)> {
    let start = NodeTopSupportVertexKey::from_world_point(start);
    let end = NodeTopSupportVertexKey::from_world_point(end);
    NodeTopSupportEdgeKey::from_vertices(start, end).map(|key| (key, start, end))
}

fn complete_raised_step_faces_from_final_spans(
    raised_step_faces: &mut Vec<RoadSurfaceRaisedStepFace>,
    required_spans: &[FinalRequiredRaisedStepSpan],
    region_centroids: &[Option<RoadVec3>],
    owner_centroids: &BTreeMap<NodeBandOwner, RoadVec3>,
) {
    // Final owner-wide top boundaries are the rendered authority. Rebuild the face set from these
    // spans so stale arrangement-side quads cannot block corrected final support geometry.
    let mut rebuilt = Vec::new();
    let mut emitted = BTreeSet::new();

    for span in required_spans.iter().copied() {
        if !emitted.insert(span.support_key) {
            continue;
        }
        if let Some(mut face) = raised_step_face_from_span(span) {
            orient_raised_step_face_from_lower_owner(
                &mut face,
                span.lower_edge.region_index,
                region_centroids,
                owner_centroids,
            );
            rebuilt.push(face);
        }
    }
    *raised_step_faces = rebuilt;
}

fn final_required_raised_step_spans(
    explicit_vertical_step_segments: &[NodeExplicitVerticalStepSegment],
    top_edges: &[NodeTopSupportEdge],
) -> Vec<FinalRequiredRaisedStepSpan> {
    let mut spans = Vec::new();
    let mut emitted = BTreeSet::<RaisedStepFaceSupportKey>::new();
    let top_edge_index = NodeTopSupportEdgeIndex::new(top_edges);
    for (step_index, source_segment) in explicit_vertical_step_segments.iter().copied().enumerate()
    {
        let Some((source_lower_owner, source_raised_owner)) =
            vertical_step_lower_and_raised_owners(source_segment)
        else {
            continue;
        };
        let segment_start = keys::SurfaceXzKey::from_raw_keys(
            source_segment.start().x_key(),
            source_segment.start().z_key(),
        );
        let segment_end = keys::SurfaceXzKey::from_raw_keys(
            source_segment.end().x_key(),
            source_segment.end().z_key(),
        );
        let lower_candidates = top_edge_index.support_edge_candidates_on_step_segment(
            source_lower_owner.kind(),
            segment_start,
            segment_end,
        );
        let raised_candidates = top_edge_index.support_edge_candidates_on_step_segment(
            source_raised_owner.kind(),
            segment_start,
            segment_end,
        );
        for (lower_edge, lower_start_t, lower_end_t) in &lower_candidates {
            for (raised_edge, raised_start_t, raised_end_t) in &raised_candidates {
                let lower_owner = lower_edge.owner;
                let raised_owner = raised_edge.owner;
                let Some(source) = vertical_step_source_for_final_support_owners(
                    step_index,
                    source_segment,
                    source_lower_owner,
                    source_raised_owner,
                    lower_owner,
                    raised_owner,
                ) else {
                    continue;
                };
                let start_t = (*lower_start_t).max(*raised_start_t);
                let end_t = (*lower_end_t).min(*raised_end_t);
                if end_t <= start_t {
                    continue;
                }
                let Some(boundary_keys) = raised_step_boundary_points_from_top_support(
                    *lower_edge,
                    *raised_edge,
                    segment_start,
                    segment_end,
                    start_t,
                    end_t,
                ) else {
                    continue;
                };
                let support_key = raised_step_face_support_key_from_boundary_points(
                    lower_owner,
                    raised_owner,
                    boundary_keys,
                );
                let span = FinalRequiredRaisedStepSpan {
                    lower_edge: *lower_edge,
                    segment_start,
                    segment_end,
                    boundary_keys,
                    support_key,
                    source,
                };
                if emitted.insert(support_key) {
                    spans.push(span);
                }
            }
        }
    }
    spans.sort_by_key(|span| span.support_key);
    spans
}

fn vertical_step_source_for_final_support_owners(
    explicit_vertical_step_index: usize,
    segment: NodeExplicitVerticalStepSegment,
    source_lower_owner: NodeBandOwner,
    source_raised_owner: NodeBandOwner,
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
) -> Option<RoadSurfaceVerticalFaceSource> {
    if source_lower_owner == lower_owner && source_raised_owner == raised_owner {
        return Some(RoadSurfaceVerticalFaceSource::CanonicalStep {
            explicit_vertical_step_index,
            segment,
        });
    }
    if source_lower_owner.kind() != lower_owner.kind()
        || source_raised_owner.kind() != raised_owner.kind()
    {
        return None;
    }
    if source_lower_owner != lower_owner && source_raised_owner != raised_owner {
        return None;
    }
    Some(
        RoadSurfaceVerticalFaceSource::CanonicalStepSameMaterialHandoff {
            explicit_vertical_step_index,
            segment,
            lower_owner,
            raised_owner,
        },
    )
}

fn raised_step_face_from_span(
    span: FinalRequiredRaisedStepSpan,
) -> Option<RoadSurfaceRaisedStepFace> {
    raised_step_face_from_top_support(
        span.lower_edge,
        span.segment_start,
        span.segment_end,
        span.boundary_keys,
        span.source,
    )
}

fn raised_step_face_from_top_support(
    lower_edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
    keys: RaisedStepBoundaryPointKeys,
    source: RoadSurfaceVerticalFaceSource,
) -> Option<RoadSurfaceRaisedStepFace> {
    let lower_start = boundary_point_to_world(keys.lower_start);
    let lower_end = boundary_point_to_world(keys.lower_end);
    let raised_start = boundary_point_to_world(keys.raised_start);
    let raised_end = boundary_point_to_world(keys.raised_end);
    let lower_owner_on_right =
        support_edge_owner_lies_right_of_segment(lower_edge, segment_start, segment_end)?;
    let points = if lower_owner_on_right {
        [raised_start, lower_start, lower_end, raised_end]
    } else {
        [raised_end, lower_end, lower_start, raised_start]
    };
    let polygon = RoadSurfaceSystem::make_vertical_quad_polygon(points)?;
    Some(RoadSurfaceRaisedStepFace {
        polygon,
        source,
        lower_edge: (keys.lower_start, keys.lower_end),
    })
}

fn raised_step_boundary_points_from_top_support(
    lower_edge: NodeTopSupportEdge,
    raised_edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
    start_t: keys::SurfaceSegmentParameter,
    end_t: keys::SurfaceSegmentParameter,
) -> Option<RaisedStepBoundaryPointKeys> {
    let lower_start =
        support_edge_point_at_segment_parameter(lower_edge, segment_start, segment_end, start_t)?;
    let lower_end =
        support_edge_point_at_segment_parameter(lower_edge, segment_start, segment_end, end_t)?;
    let raised_start =
        support_edge_point_at_segment_parameter(raised_edge, segment_start, segment_end, start_t)?;
    let raised_end =
        support_edge_point_at_segment_parameter(raised_edge, segment_start, segment_end, end_t)?;
    let raised_start = raised_step_face_height_dust_adjusted_point(lower_start, raised_start)?;
    let raised_end = raised_step_face_height_dust_adjusted_point(lower_end, raised_end)?;
    if lower_start.xz_key() == lower_end.xz_key()
        || raised_start.xz_key() == raised_end.xz_key()
        || (raised_start.y_mm == lower_start.y_mm && raised_end.y_mm == lower_end.y_mm)
    {
        return None;
    }
    Some(RaisedStepBoundaryPointKeys {
        lower_start,
        lower_end,
        raised_start,
        raised_end,
    })
}

fn raised_step_face_height_dust_adjusted_point(
    lower: ArrangementBoundaryPointKey,
    mut raised: ArrangementBoundaryPointKey,
) -> Option<ArrangementBoundaryPointKey> {
    if raised.y_mm >= lower.y_mm {
        return Some(raised);
    }
    if lower.y_mm - raised.y_mm > RAISED_STEP_FACE_HEIGHT_DUST_MM {
        return None;
    }
    raised.y_mm = lower.y_mm;
    Some(raised)
}

fn raised_step_face_support_key_from_boundary_points(
    lower_owner: NodeBandOwner,
    raised_owner: NodeBandOwner,
    keys: RaisedStepBoundaryPointKeys,
) -> RaisedStepFaceSupportKey {
    RaisedStepFaceSupportKey {
        lower_owner,
        raised_owner,
        lower_edge: NodeTopSupportEdgeKey::from_boundary_points((keys.lower_start, keys.lower_end)),
        upper_edge: NodeTopSupportEdgeKey::from_boundary_points((
            keys.raised_start,
            keys.raised_end,
        )),
    }
}

fn raised_step_region_centroids(owned_regions: &[NodeOwnedRegion]) -> Vec<Option<RoadVec3>> {
    owned_regions
        .iter()
        .map(|region| owned_region_centroid_sum(region).map(|(sum, count)| sum / count as f64))
        .collect()
}

fn raised_step_owner_centroids(
    owned_regions: &[NodeOwnedRegion],
) -> BTreeMap<NodeBandOwner, RoadVec3> {
    let mut sums = BTreeMap::<NodeBandOwner, (RoadVec3, usize)>::new();
    for region in owned_regions {
        let owner = NodeBandOwner::new(region.kind, region.owner_index);
        let entry = sums.entry(owner).or_insert((RoadVec3::ZERO, 0));
        if let Some((sum, count)) = owned_region_centroid_sum(region) {
            entry.0 += sum;
            entry.1 += count;
        }
    }
    sums.into_iter()
        .filter_map(|(owner, (sum, count))| (count > 0).then_some((owner, sum / count as f64)))
        .collect()
}

fn owned_region_centroid_sum(region: &NodeOwnedRegion) -> Option<(RoadVec3, usize)> {
    let mut sum = RoadVec3::ZERO;
    let mut count = 0usize;
    if !region.polygon.points_world.is_empty() {
        for point in &region.polygon.points_world {
            sum += RoadVec3::new(point.x, 0.0, point.z);
            count += 1;
        }
    } else {
        for point in region
            .polygon
            .triangles_world
            .iter()
            .flat_map(|triangle| triangle.iter())
        {
            sum += RoadVec3::new(point.x, 0.0, point.z);
            count += 1;
        }
    }
    (count > 0).then_some((sum, count))
}

fn orient_raised_step_face_from_lower_owner(
    face: &mut RoadSurfaceRaisedStepFace,
    lower_region_index: usize,
    region_centroids: &[Option<RoadVec3>],
    owner_centroids: &BTreeMap<NodeBandOwner, RoadVec3>,
) {
    let Some((lower_owner, _)) = face.source.lower_and_raised_owners() else {
        return;
    };
    let Some(lower_centroid) = region_centroids
        .get(lower_region_index)
        .copied()
        .flatten()
        .or_else(|| owner_centroids.get(&lower_owner).copied())
    else {
        return;
    };
    let lower_start = boundary_point_to_world(face.lower_edge.0);
    let lower_end = boundary_point_to_world(face.lower_edge.1);
    let midpoint = RoadVec3::new(
        (lower_start.x + lower_end.x) * 0.5,
        0.0,
        (lower_start.z + lower_end.z) * 0.5,
    );
    let owner_direction = RoadVec3::new(
        lower_centroid.x - midpoint.x,
        0.0,
        lower_centroid.z - midpoint.z,
    );
    if owner_direction.length_squared() <= 1e-8 {
        return;
    }
    let Some(visible_direction) = vertical_face_visible_direction(&face.polygon.points_world)
    else {
        return;
    };
    if visible_direction.dot(owner_direction.normalize()) > 0.0 {
        return;
    }
    let [a, b, c, d] = face.polygon.points_world.as_slice() else {
        return;
    };
    if let Some(flipped) = RoadSurfaceSystem::make_vertical_quad_polygon([*d, *c, *b, *a]) {
        face.polygon = flipped;
    }
}

fn vertical_face_visible_direction(points: &[RoadVec3]) -> Option<RoadVec3> {
    if points.len() < 3 {
        return None;
    }
    for index in 1..points.len().saturating_sub(1) {
        let normal = (points[index] - points[0]).cross(points[index + 1] - points[0]);
        if normal.length_squared() > 1e-8 {
            let visible = -normal.normalize();
            let visible_xz = RoadVec3::new(visible.x, 0.0, visible.z);
            return (visible_xz.length_squared() > 1e-8).then(|| visible_xz.normalize());
        }
    }
    None
}

fn vertical_step_lower_and_raised_owners(
    segment: NodeExplicitVerticalStepSegment,
) -> Option<(NodeBandOwner, NodeBandOwner)> {
    let owner = segment.owner();
    let opposite_owner = segment.opposite_owner();
    let (lower_kind, _) = ordered_raised_step_kinds(owner.kind(), opposite_owner.kind())?;
    Some(if owner.kind() == lower_kind {
        (owner, opposite_owner)
    } else {
        (opposite_owner, owner)
    })
}

fn support_edge_overlap_interval_on_segment(
    edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<(keys::SurfaceSegmentParameter, keys::SurfaceSegmentParameter)> {
    let mut parameters = [keys::SurfaceSegmentParameter::zero(); 4];
    let mut parameter_count = 0;
    for point in [edge.start.xz, edge.end.xz] {
        if let Some(parameter) = endpoint_parameter_on_segment(point, segment_start, segment_end) {
            insert_support_overlap_parameter(
                &mut parameters,
                &mut parameter_count,
                clamped_unit_parameter(parameter),
            );
        }
    }
    for (point, parameter) in [
        (segment_start, keys::SurfaceSegmentParameter::zero()),
        (segment_end, keys::SurfaceSegmentParameter::one()),
    ] {
        if bounded_endpoint_parameter_on_segment(point, edge.start.xz, edge.end.xz).is_some() {
            insert_support_overlap_parameter(&mut parameters, &mut parameter_count, parameter);
        }
    }
    if parameter_count == 0 {
        return None;
    }
    let start = parameters[0];
    let end = parameters[parameter_count - 1];
    (end > start).then_some((start, end))
}

fn insert_support_overlap_parameter(
    parameters: &mut [keys::SurfaceSegmentParameter; 4],
    parameter_count: &mut usize,
    parameter: keys::SurfaceSegmentParameter,
) {
    if parameters[..*parameter_count]
        .iter()
        .any(|existing| *existing == parameter)
    {
        return;
    }
    debug_assert!(*parameter_count < parameters.len());
    if *parameter_count == parameters.len() {
        return;
    }
    let mut insert_index = *parameter_count;
    while insert_index > 0 && parameter < parameters[insert_index - 1] {
        parameters[insert_index] = parameters[insert_index - 1];
        insert_index -= 1;
    }
    parameters[insert_index] = parameter;
    *parameter_count += 1;
}

fn endpoint_parameter_on_segment(
    point: keys::SurfaceXzKey,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<keys::SurfaceSegmentParameter> {
    segments::overlay_segment_parameter(point, segment_start, segment_end)
        .or_else(|| segments::exact_line_parameter(point, segment_start, segment_end))
        .or_else(|| numeric_dust_line_parameter(point, segment_start, segment_end))
        .or_else(|| overlay_grid_line_parameter(point, segment_start, segment_end))
}

fn bounded_endpoint_parameter_on_segment(
    point: keys::SurfaceXzKey,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<keys::SurfaceSegmentParameter> {
    segments::overlay_segment_parameter(point, segment_start, segment_end)
        .or_else(|| {
            bounded_unit_parameter(segments::exact_line_parameter(
                point,
                segment_start,
                segment_end,
            )?)
        })
        .or_else(|| numeric_dust_line_parameter(point, segment_start, segment_end))
        .or_else(|| {
            bounded_unit_parameter(overlay_grid_line_parameter(
                point,
                segment_start,
                segment_end,
            )?)
        })
}

fn overlay_grid_line_parameter(
    point: keys::SurfaceXzKey,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<keys::SurfaceSegmentParameter> {
    if !segments::key_collinear_with_overlay_grid_segment(point, segment_start, segment_end) {
        return None;
    }
    let dx = i128::from(segment_end.x_key() - segment_start.x_key());
    let dz = i128::from(segment_end.z_key() - segment_start.z_key());
    let denominator = dx * dx + dz * dz;
    keys::SurfaceSegmentParameter::new(
        segments::segment_parameter_key(segment_start, segment_end, point),
        denominator,
    )
}

fn numeric_dust_line_parameter(
    point: keys::SurfaceXzKey,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<keys::SurfaceSegmentParameter> {
    if segment_start == segment_end {
        return None;
    }
    let dx = i128::from(segment_end.x_key() - segment_start.x_key());
    let dz = i128::from(segment_end.z_key() - segment_start.z_key());
    let px = i128::from(point.x_key() - segment_start.x_key());
    let pz = i128::from(point.z_key() - segment_start.z_key());
    let denominator = dx * dx + dz * dz;
    if denominator == 0 {
        return None;
    }
    let numerator = px * dx + pz * dz;
    let length_key_units = (denominator as f64).sqrt();
    let dust_key_units = final_step_support_numeric_dust_key_units() as f64;
    let endpoint_padding = dust_key_units * length_key_units;
    let numerator_f64 = numerator as f64;
    if numerator_f64 < -endpoint_padding || numerator_f64 > denominator as f64 + endpoint_padding {
        return None;
    }
    let cross = dx * pz - dz * px;
    if cross.unsigned_abs() as f64 > dust_key_units * length_key_units {
        return None;
    }
    keys::SurfaceSegmentParameter::new(numerator.clamp(0, denominator), denominator)
}

fn final_step_support_numeric_dust_key_units() -> i64 {
    (f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * keys::SURFACE_XZ_KEY_SCALE).round() as i64
}

fn clamped_unit_parameter(
    parameter: keys::SurfaceSegmentParameter,
) -> keys::SurfaceSegmentParameter {
    parameter
        .max(keys::SurfaceSegmentParameter::zero())
        .min(keys::SurfaceSegmentParameter::one())
}

fn bounded_unit_parameter(
    parameter: keys::SurfaceSegmentParameter,
) -> Option<keys::SurfaceSegmentParameter> {
    (parameter >= keys::SurfaceSegmentParameter::zero()
        && parameter <= keys::SurfaceSegmentParameter::one())
    .then_some(parameter)
}

fn support_edge_point_at_segment_parameter(
    edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
    parameter: keys::SurfaceSegmentParameter,
) -> Option<ArrangementBoundaryPointKey> {
    let xz = segments::interpolate_key(segment_start, segment_end, parameter);
    support_edge_point_at_xz(edge, xz)
}

fn support_edge_point_at_xz(
    edge: NodeTopSupportEdge,
    xz: keys::SurfaceXzKey,
) -> Option<ArrangementBoundaryPointKey> {
    let edge_parameter = bounded_endpoint_parameter_on_segment(xz, edge.start.xz, edge.end.xz)?;
    let supported_xz = segments::interpolate_key(edge.start.xz, edge.end.xz, edge_parameter);
    Some(ArrangementBoundaryPointKey {
        x_key: supported_xz.x_key(),
        z_key: supported_xz.z_key(),
        y_mm: segments::interpolate_height_i64(edge.start.y_mm, edge.end.y_mm, edge_parameter),
    })
}

fn support_edge_owner_lies_right_of_segment(
    edge: NodeTopSupportEdge,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<bool> {
    let start_t = support_edge_order_key(edge.start.xz, segment_start, segment_end)?;
    let end_t = support_edge_order_key(edge.end.xz, segment_start, segment_end)?;
    Some(end_t < start_t)
}

fn support_edge_order_key(
    point: keys::SurfaceXzKey,
    segment_start: keys::SurfaceXzKey,
    segment_end: keys::SurfaceXzKey,
) -> Option<i128> {
    if segment_start == segment_end {
        return None;
    }
    Some(segments::segment_parameter_key(
        segment_start,
        segment_end,
        point,
    ))
}

fn boundary_point_to_world(point: ArrangementBoundaryPointKey) -> RoadVec3 {
    let xz = keys::SurfaceXzKey::from_raw_keys(point.x_key, point.z_key).to_road_xz();
    RoadVec3::new(xz.x, point.y_mm as f64 / 1000.0, xz.y)
}

impl NodeTopSupportEdgeKey {
    fn from_vertices(start: NodeTopSupportVertexKey, end: NodeTopSupportVertexKey) -> Option<Self> {
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }

    fn from_boundary_points(
        edge: (ArrangementBoundaryPointKey, ArrangementBoundaryPointKey),
    ) -> Self {
        let start = NodeTopSupportVertexKey::from_boundary_point(edge.0);
        let end = NodeTopSupportVertexKey::from_boundary_point(edge.1);
        if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }
}

impl NodeTopSupportEdgeCandidate {
    fn new(edge: NodeTopSupportEdge) -> Self {
        Self {
            edge,
            bounds: SurfaceKeyBounds::from_segment(edge.start.xz, edge.end.xz),
        }
    }
}

impl NodeTopSupportEdgeIndex {
    fn new(top_edges: &[NodeTopSupportEdge]) -> Self {
        let mut edges_by_kind =
            BTreeMap::<RoadSurfaceBandKind, Vec<NodeTopSupportEdgeCandidate>>::new();
        let mut tile_indices_by_kind =
            BTreeMap::<RoadSurfaceBandKind, BTreeMap<SurfaceKeyTile, Vec<usize>>>::new();
        for edge in top_edges.iter().copied() {
            let kind = edge.owner.kind();
            let candidate = NodeTopSupportEdgeCandidate::new(edge);
            let candidate_index = edges_by_kind.entry(kind).or_default().len();
            for tile in SurfaceKeyTile::tiles_for_bounds(candidate.bounds) {
                tile_indices_by_kind
                    .entry(kind)
                    .or_default()
                    .entry(tile)
                    .or_default()
                    .push(candidate_index);
            }
            edges_by_kind.entry(kind).or_default().push(candidate);
        }
        Self {
            edges_by_kind,
            tile_indices_by_kind,
        }
    }

    fn support_edge_candidates_on_step_segment(
        &self,
        owner_kind: RoadSurfaceBandKind,
        segment_start: keys::SurfaceXzKey,
        segment_end: keys::SurfaceXzKey,
    ) -> Vec<(
        NodeTopSupportEdge,
        keys::SurfaceSegmentParameter,
        keys::SurfaceSegmentParameter,
    )> {
        let Some(edges) = self.edges_by_kind.get(&owner_kind) else {
            return Vec::new();
        };
        let Some(tile_indices) = self.tile_indices_by_kind.get(&owner_kind) else {
            return Vec::new();
        };
        let segment_bounds = SurfaceKeyBounds::from_segment(segment_start, segment_end);
        let mut candidate_indices = BTreeSet::new();
        for tile in SurfaceKeyTile::tiles_for_bounds(segment_bounds) {
            if let Some(indices) = tile_indices.get(&tile) {
                candidate_indices.extend(indices.iter().copied());
            }
        }
        candidate_indices
            .into_iter()
            .filter_map(|candidate_index| edges.get(candidate_index))
            .filter(|candidate| candidate.bounds.overlaps(segment_bounds))
            .filter_map(|candidate| {
                let edge = candidate.edge;
                let (start_t, end_t) =
                    support_edge_overlap_interval_on_segment(edge, segment_start, segment_end)?;
                Some((edge, start_t, end_t))
            })
            .collect()
    }
}

impl SurfaceKeyTile {
    fn tiles_for_bounds(bounds: SurfaceKeyBounds) -> Vec<Self> {
        let min_tile_x = bounds.min_x.div_euclid(TOP_SUPPORT_EDGE_TILE_KEYS);
        let max_tile_x = bounds.max_x.div_euclid(TOP_SUPPORT_EDGE_TILE_KEYS);
        let min_tile_z = bounds.min_z.div_euclid(TOP_SUPPORT_EDGE_TILE_KEYS);
        let max_tile_z = bounds.max_z.div_euclid(TOP_SUPPORT_EDGE_TILE_KEYS);
        let mut tiles = Vec::new();
        for x in min_tile_x..=max_tile_x {
            for z in min_tile_z..=max_tile_z {
                tiles.push(Self { x, z });
            }
        }
        tiles
    }
}

impl SurfaceKeyBounds {
    fn from_segment(start: keys::SurfaceXzKey, end: keys::SurfaceXzKey) -> Self {
        Self {
            min_x: start.x_key().min(end.x_key()),
            min_z: start.z_key().min(end.z_key()),
            max_x: start.x_key().max(end.x_key()),
            max_z: start.z_key().max(end.z_key()),
        }
    }

    fn overlaps(self, other: Self) -> bool {
        self.min_x <= other.max_x
            && other.min_x <= self.max_x
            && self.min_z <= other.max_z
            && other.min_z <= self.max_z
    }
}

impl NodeTopSupportVertexKey {
    fn from_world_point(point: RoadVec3) -> Self {
        Self {
            xz: keys::SurfaceXzKey::from_world_xz(point),
            y_mm: keys::SurfaceHeightMmKey::from_m_f64(point.y).as_i64(),
        }
    }

    fn from_boundary_point(point: ArrangementBoundaryPointKey) -> Self {
        Self {
            xz: keys::SurfaceXzKey::from_raw_keys(point.x_key, point.z_key),
            y_mm: point.y_mm,
        }
    }
}
