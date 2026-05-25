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
            let polygon_keys = region
                .polygon
                .points_world
                .iter()
                .copied()
                .map(world_point_raw_key)
                .collect::<Vec<_>>();
            let vertex_keys = source
                .vertex_keys
                .iter()
                .copied()
                .map(arrangement_key_tuple)
                .collect::<Vec<_>>();
            assert_eq!(
                polygon_keys, vertex_keys,
                "exported top polygon points must match canonical arrangement keys"
            );
            CanonicalNodeTopPolygonIdentity {
                kind: source.kind,
                owner_index: source.owner_index,
                height_field_id: source.height_field_id,
                polygon_keys,
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

fn world_point_raw_key(point: Vector3) -> (i64, i64) {
    let key = SurfaceXzKey::from_road_xz(backend::godot_vec3_xz_to_road(point));
    (key.x_key(), key.z_key())
}

fn arrangement_key_tuple(key: NodeArrangementKey) -> (i64, i64) {
    (key.x_key(), key.z_key())
}
