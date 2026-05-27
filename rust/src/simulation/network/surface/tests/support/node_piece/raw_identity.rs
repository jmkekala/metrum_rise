//! Exact canonical raw-polygon identity helpers.

use super::*;
use crate::simulation::network::surface::keys::SurfaceXzKey;
use crate::simulation::network::surface::node::NodeTopSurfaceVertexSource;

/// Exact node top-surface and carrier-provenance identity used by matrix tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::simulation::network::surface::tests) struct CanonicalNodeRawPolygonIdentity {
    kind: RoadSurfaceVisualNodePieceKind,
    top_polygons: Vec<CanonicalNodeTopPolygonIdentity>,
    carrier_records: Vec<ownership::NodeCarrierProvenanceRecord>,
}

/// Golden signature for canonical node raw-polygon identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::simulation::network::surface::tests) struct CanonicalNodeRawPolygonGolden {
    /// Expected compiled node-piece kind.
    pub kind: RoadSurfaceVisualNodePieceKind,
    /// Expected number of exported top-surface polygons.
    pub top_polygon_count: usize,
    /// Expected number of carrier-provenance closure records.
    pub carrier_record_count: usize,
    /// Expected number of source-segment provenance records.
    pub source_segment_record_count: usize,
    /// Stable digest of canonical polygon key sets.
    pub polygon_key_set_digest: u64,
    /// Stable digest of top-surface owner / height-field identities.
    pub top_owner_height_field_digest: u64,
    /// Stable digest of carrier owner / source / height-field identities.
    pub carrier_owner_source_height_field_digest: u64,
    /// Stable digest of stable source-carrier segment IDs, when present.
    pub source_segment_id_digest: u64,
    /// Exact stable source-carrier segment IDs for source-segment projection records.
    pub source_segment_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CanonicalNodeTopPolygonIdentity {
    kind: RoadSurfaceBandKind,
    owner_index: usize,
    height_field_id: arrangement::NodeBandHeightFieldId,
    polygon_keys: Vec<(i64, i64)>,
    vertex_keys: Vec<(i64, i64)>,
    vertex_height_mm: Vec<i64>,
    vertex_grade_authorities: Vec<height::NodeGradeVertexAuthority>,
    triangle_grade_authorities: Vec<[height::NodeGradeVertexAuthority; 3]>,
}

pub(in crate::simulation::network::surface::tests) fn canonical_node_raw_polygon_identity(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    node_id: u32,
) -> CanonicalNodeRawPolygonIdentity {
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&node_id)
        .unwrap_or_else(|| panic!("node {node_id} must have a compiled visual node piece"));
    assert_node_top_surface_sources_have_grade_authority(piece);
    let carrier_records = canonical_node_carrier_records(surface, graph, node_id, piece.kind);
    CanonicalNodeRawPolygonIdentity {
        kind: piece.kind,
        top_polygons: canonical_top_polygon_identities(piece),
        carrier_records,
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_canonical_node_raw_polygon_identity_eq(
    left_label: &str,
    left: &CanonicalNodeRawPolygonIdentity,
    right_label: &str,
    right: &CanonicalNodeRawPolygonIdentity,
) {
    assert_eq!(
        left, right,
        "canonical raw polygon identity differs between {left_label} and {right_label}\nleft={left:#?}\nright={right:#?}"
    );
}

pub(in crate::simulation::network::surface::tests) fn assert_canonical_node_raw_polygon_golden(
    label: &str,
    identity: &CanonicalNodeRawPolygonIdentity,
    expected: CanonicalNodeRawPolygonGolden,
) {
    let actual = canonical_node_raw_polygon_golden(identity);
    assert_eq!(
        actual, expected,
        "canonical raw polygon golden mismatch for {label}\nactual={actual:#?}\nexpected={expected:#?}\nexact_canonical_identity={identity:#?}"
    );
}

fn canonical_node_raw_polygon_golden(
    identity: &CanonicalNodeRawPolygonIdentity,
) -> CanonicalNodeRawPolygonGolden {
    CanonicalNodeRawPolygonGolden {
        kind: identity.kind,
        top_polygon_count: identity.top_polygons.len(),
        carrier_record_count: identity.carrier_records.len(),
        source_segment_record_count: identity
            .carrier_records
            .iter()
            .filter(|record| {
                matches!(
                    record.origin,
                    ownership::NodeCarrierProvenanceOrigin::SourceSegment { .. }
                )
            })
            .count(),
        polygon_key_set_digest: polygon_key_set_digest(&identity.top_polygons),
        top_owner_height_field_digest: top_owner_height_field_digest(&identity.top_polygons),
        carrier_owner_source_height_field_digest: carrier_owner_source_height_field_digest(
            &identity.carrier_records,
        ),
        source_segment_id_digest: source_segment_id_digest(&identity.carrier_records),
        source_segment_ids: source_segment_ids(&identity.carrier_records),
    }
}

fn canonical_top_polygon_identities(
    piece: &RoadSurfaceVisualNodePiece,
) -> Vec<CanonicalNodeTopPolygonIdentity> {
    assert_eq!(
        piece.owned_regions.len(),
        piece.node_top_surface_sources.len(),
        "compiled node top regions and source records must stay paired"
    );
    piece
        .owned_regions
        .iter()
        .zip(piece.node_top_surface_sources.iter())
        .map(|(region, source)| {
            assert_eq!(
                region.kind, source.kind,
                "owned region material must match top-source material"
            );
            assert_eq!(
                region.owner_index, source.owner_index,
                "owned region owner must match top-source owner"
            );
            let vertex_keys = source
                .vertex_keys
                .iter()
                .copied()
                .map(arrangement_key_tuple)
                .collect::<Vec<_>>();
            assert_exported_polygon_points_track_canonical_keys(
                &region.polygon.points_world,
                &source.vertex_keys,
            );
            CanonicalNodeTopPolygonIdentity {
                kind: source.kind,
                owner_index: source.owner_index,
                height_field_id: source.height_field_id,
                polygon_keys: vertex_keys.clone(),
                vertex_keys,
                vertex_height_mm: source.vertex_height_mm.clone(),
                vertex_grade_authorities: source
                    .vertex_sources
                    .iter()
                    .copied()
                    .map(|vertex_source| grade_authority_for_source(piece, vertex_source))
                    .collect(),
                triangle_grade_authorities: source
                    .triangle_sources
                    .iter()
                    .map(|triangle| {
                        triangle
                            .map(|vertex_source| grade_authority_for_source(piece, vertex_source))
                    })
                    .collect(),
            }
        })
        .collect()
}

fn canonical_node_carrier_records(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    node_id: u32,
    kind: RoadSurfaceVisualNodePieceKind,
) -> Vec<ownership::NodeCarrierProvenanceRecord> {
    let incidents = surface.sorted_incident_surface_edges(graph, node_id);
    let mouths = surface
        .build_ordered_piece_mouths(&incidents)
        .unwrap_or_else(|| panic!("node {node_id} must produce ordered mouths"));
    let input = RoadSurfaceSystem::build_node_arrangement_input_from_mouths(node_id, kind, &mouths)
        .unwrap_or_else(|error| {
            panic!("node {node_id} must rebuild arrangement input for raw identity: {error:?}")
        });
    let rails =
        RoadSurfaceSystem::build_node_rail_contours_from_input(&input).unwrap_or_else(|error| {
            panic!("node {node_id} must rebuild rail contours for raw identity: {error:?}")
        });
    let ownership = RoadSurfaceSystem::build_node_boolean_ownership_from_rails(&rails)
        .unwrap_or_else(|error| {
            panic!("node {node_id} must rebuild boolean ownership for raw identity: {error:?}")
        });
    ownership.carrier_provenance.records
}

fn grade_authority_for_source(
    piece: &RoadSurfaceVisualNodePiece,
    source: NodeTopSurfaceVertexSource,
) -> height::NodeGradeVertexAuthority {
    piece
        .node_grade_authorities
        .get(source.grade_authority_index)
        .copied()
        .unwrap_or_else(|| {
            panic!(
                "node top source references missing grade authority index {}",
                source.grade_authority_index
            )
        })
}

fn world_point_raw_key(point: RoadVec3) -> (i64, i64) {
    let key = SurfaceXzKey::from_world_xz(point);
    (key.x_key(), key.z_key())
}

fn assert_exported_polygon_points_track_canonical_keys(
    points_world: &[RoadVec3],
    vertex_keys: &[NodeArrangementKey],
) {
    assert_eq!(
        points_world.len(),
        vertex_keys.len(),
        "exported top polygon point count must match canonical arrangement key count"
    );
    for (point_index, (&point, &key)) in points_world.iter().zip(vertex_keys).enumerate() {
        let actual = world_point_raw_key(point);
        let expected = arrangement_key_tuple(key);
        let x_tolerance = display_float_key_tolerance(expected.0);
        let z_tolerance = display_float_key_tolerance(expected.1);
        assert!(
            (actual.0 - expected.0).abs() <= x_tolerance
                && (actual.1 - expected.1).abs() <= z_tolerance,
            "exported top polygon point {point_index} must track canonical arrangement key within f32 display precision: actual={actual:?} expected={expected:?} tolerance=({x_tolerance},{z_tolerance})"
        );
    }
}

fn display_float_key_tolerance(raw_key: i64) -> i64 {
    ((raw_key.unsigned_abs() as f64) * f64::from(f32::EPSILON)).ceil() as i64 + 2
}

fn arrangement_key_tuple(key: NodeArrangementKey) -> (i64, i64) {
    (key.x_key(), key.z_key())
}

fn polygon_key_set_digest(polygons: &[CanonicalNodeTopPolygonIdentity]) -> u64 {
    let mut state = DIGEST_OFFSET_BASIS;
    for polygon in polygons {
        digest_debug(&mut state, &polygon.kind);
        digest_usize(&mut state, polygon.owner_index);
        digest_debug(&mut state, &polygon.height_field_id);
        let mut key_set = polygon.polygon_keys.clone();
        key_set.sort_unstable();
        digest_usize(&mut state, key_set.len());
        for (x_key, z_key) in key_set {
            digest_i64(&mut state, x_key);
            digest_i64(&mut state, z_key);
        }
    }
    state
}

fn top_owner_height_field_digest(polygons: &[CanonicalNodeTopPolygonIdentity]) -> u64 {
    let mut state = DIGEST_OFFSET_BASIS;
    for polygon in polygons {
        digest_debug(&mut state, &polygon.kind);
        digest_usize(&mut state, polygon.owner_index);
        digest_debug(&mut state, &polygon.height_field_id);
        digest_usize(&mut state, polygon.vertex_height_mm.len());
        for height_mm in &polygon.vertex_height_mm {
            digest_i64(&mut state, *height_mm);
        }
        for authority in &polygon.vertex_grade_authorities {
            digest_debug(&mut state, authority);
        }
        for triangle in &polygon.triangle_grade_authorities {
            digest_debug(&mut state, triangle);
        }
    }
    state
}

fn carrier_owner_source_height_field_digest(
    records: &[ownership::NodeCarrierProvenanceRecord],
) -> u64 {
    let mut state = DIGEST_OFFSET_BASIS;
    for record in records {
        digest_debug(&mut state, &record.owner);
        digest_debug(&mut state, &record.source_kind);
        digest_usize(&mut state, record.source_mouth_order_index);
        digest_usize(&mut state, record.source_band_index);
        digest_debug(&mut state, &record.height_field_id);
        digest_debug(&mut state, &record.claim_priority);
        digest_debug(&mut state, &record.point);
        digest_debug(&mut state, &record.origin);
    }
    state
}

fn source_segment_id_digest(records: &[ownership::NodeCarrierProvenanceRecord]) -> u64 {
    let mut state = DIGEST_OFFSET_BASIS;
    for source_segment_id in source_segment_ids_from_records(records) {
        digest_debug(&mut state, &source_segment_id);
    }
    state
}

fn source_segment_ids(records: &[ownership::NodeCarrierProvenanceRecord]) -> Vec<String> {
    source_segment_ids_from_records(records)
        .into_iter()
        .map(|source_segment_id| format!("{source_segment_id:?}"))
        .collect()
}

fn source_segment_ids_from_records(
    records: &[ownership::NodeCarrierProvenanceRecord],
) -> Vec<ownership::NodeSourceCarrierSegmentId> {
    let mut source_segment_ids = Vec::new();
    for record in records {
        let ownership::NodeCarrierProvenanceOrigin::SourceSegment {
            source_segment_id, ..
        } = record.origin
        else {
            continue;
        };
        assert_eq!(
            source_segment_id.owner, record.owner,
            "source-segment provenance ID owner must match record owner"
        );
        assert_eq!(
            source_segment_id.source_kind, record.source_kind,
            "source-segment provenance ID source kind must match record source kind"
        );
        assert_eq!(
            source_segment_id.source_mouth_order_index, record.source_mouth_order_index,
            "source-segment provenance ID mouth index must match record source mouth index"
        );
        assert_eq!(
            source_segment_id.source_band_index, record.source_band_index,
            "source-segment provenance ID band index must match record source band index"
        );
        source_segment_ids.push(source_segment_id);
    }
    source_segment_ids.sort_unstable();
    source_segment_ids
}

const DIGEST_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const DIGEST_PRIME: u64 = 0x0000_0100_0000_01b3;

fn digest_debug(state: &mut u64, value: &impl std::fmt::Debug) {
    for byte in format!("{value:?}").bytes() {
        digest_byte(state, byte);
    }
    digest_byte(state, 0xff);
}

fn digest_usize(state: &mut u64, value: usize) {
    digest_u64(state, value as u64);
}

fn digest_i64(state: &mut u64, value: i64) {
    digest_u64(state, value as u64);
}

fn digest_u64(state: &mut u64, value: u64) {
    for byte in value.to_le_bytes() {
        digest_byte(state, byte);
    }
}

fn digest_byte(state: &mut u64, byte: u8) {
    *state ^= u64::from(byte);
    *state = state.wrapping_mul(DIGEST_PRIME);
}
