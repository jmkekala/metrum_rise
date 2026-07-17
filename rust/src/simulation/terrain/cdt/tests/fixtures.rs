//! Shared road-loop, DEM, provenance, and mesh assertion fixtures.

use super::*;

fn simplified_test_road_loop(points: Vec<TerrainCdtVertex>) -> Vec<TerrainCdtVertex> {
    simplified_road_loop(points).expect("test road loop must not contain conflicting X/Z heights")
}

pub(super) fn sourced_road_loop(
    stable_piece_id: u64,
    local_loop_index: u32,
    vertices: Vec<TerrainCdtVertex>,
    source: TerrainCdtRoadBoundarySource,
) -> TerrainCdtRoadLoop {
    let source_edges = vertices
        .iter()
        .copied()
        .enumerate()
        .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
            start,
            end: vertices[(index + 1) % vertices.len()],
            source,
        })
        .collect();
    TerrainCdtRoadLoop::new_with_source_edges(
        stable_piece_id,
        local_loop_index,
        vertices,
        source_edges,
    )
}

pub(super) fn sourced_l_road_loop_with_notch_sources(
    notch_horizontal_source: TerrainCdtRoadBoundarySource,
    notch_vertical_source: TerrainCdtRoadBoundarySource,
) -> TerrainCdtRoadLoop {
    let fallback_source = test_node_boundary_source(88, TerrainCdtRoadBandKind::Sidewalk, 3);
    let vertices = vec![
        TerrainCdtVertex::new(2.0, 0.0, 2.0),
        TerrainCdtVertex::new(8.0, 0.0, 2.0),
        TerrainCdtVertex::new(8.0, 4.0, 4.0),
        TerrainCdtVertex::new(4.0, 2.0, 4.0),
        TerrainCdtVertex::new(4.0, 0.0, 8.0),
        TerrainCdtVertex::new(2.0, 0.0, 8.0),
    ];
    let sources = [
        fallback_source,
        fallback_source,
        notch_horizontal_source,
        notch_vertical_source,
        fallback_source,
        fallback_source,
    ];
    let source_edges = vertices
        .iter()
        .copied()
        .enumerate()
        .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
            start,
            end: vertices[(index + 1) % vertices.len()],
            source: sources[index],
        })
        .collect();
    TerrainCdtRoadLoop::new_with_source_edges(88, 0, vertices, source_edges)
}

pub(super) fn test_node_boundary_source(
    node_id: u32,
    owner_kind: TerrainCdtRoadBandKind,
    owner_index: u32,
) -> TerrainCdtRoadBoundarySource {
    TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
        node_id,
        node_kind: TerrainCdtNodePieceKind::JunctionN,
        owner_kind,
        owner_index,
        boundary_source: None,
    }
}

pub(super) fn test_node_boundary_source_with_direct_provenance(
    node_id: u32,
    owner_kind: TerrainCdtRoadBandKind,
    owner_index: u32,
    start_grade_authority_index: u64,
    end_grade_authority_index: u64,
) -> TerrainCdtRoadBoundarySource {
    TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
        node_id,
        node_kind: TerrainCdtNodePieceKind::JunctionN,
        owner_kind,
        owner_index,
        boundary_source: Some(TerrainCdtNodeFootprintBoundarySegmentSource {
            start: TerrainCdtNodeFootprintBoundaryVertexSource::Direct(
                TerrainCdtNodeFootprintBoundaryDirectSource {
                    top_surface_source_index: 7,
                    grade_authority_index: start_grade_authority_index,
                },
            ),
            end: TerrainCdtNodeFootprintBoundaryVertexSource::Direct(
                TerrainCdtNodeFootprintBoundaryDirectSource {
                    top_surface_source_index: 7,
                    grade_authority_index: end_grade_authority_index,
                },
            ),
        }),
    }
}

pub(super) fn test_span_boundary_source(
    edge_idx: u64,
    band_kind: TerrainCdtRoadBandKind,
    source_band_index: u32,
) -> TerrainCdtRoadBoundarySource {
    test_span_boundary_source_range(edge_idx, band_kind, source_band_index, 3, 4, 12.0, 16.0)
}

pub(super) fn test_span_boundary_source_range(
    edge_idx: u64,
    band_kind: TerrainCdtRoadBandKind,
    source_band_index: u32,
    start_section_index: u32,
    end_section_index: u32,
    start_s_m: f32,
    end_s_m: f32,
) -> TerrainCdtRoadBoundarySource {
    TerrainCdtRoadBoundarySource::SpanSupportBoundary {
        edge_idx,
        edge_class: TerrainCdtEdgeClass::Standard,
        support_policy: TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan,
        source_band_index,
        band_kind,
        role: match band_kind {
            TerrainCdtRoadBandKind::Carriageway => TerrainCdtSpanRegionRole::Asphalt,
            TerrainCdtRoadBandKind::CurbOrShoulder => TerrainCdtSpanRegionRole::CurbOrShoulder,
            _ => TerrainCdtSpanRegionRole::NonRoad,
        },
        start_section_index,
        end_section_index,
        start_s_m,
        end_s_m,
    }
}

pub(super) fn test_structural_span_boundary_source(
    edge_idx: u64,
    band_kind: TerrainCdtRoadBandKind,
    source_band_index: u32,
) -> TerrainCdtRoadBoundarySource {
    test_nonstandard_span_boundary_source(
        edge_idx,
        TerrainCdtEdgeClass::Bridge,
        TerrainCdtEarthworkSupportPolicy::BridgeEndpointAbutments,
        band_kind,
        source_band_index,
    )
}

pub(super) fn test_tunnel_span_boundary_source(
    edge_idx: u64,
    band_kind: TerrainCdtRoadBandKind,
    source_band_index: u32,
) -> TerrainCdtRoadBoundarySource {
    test_nonstandard_span_boundary_source(
        edge_idx,
        TerrainCdtEdgeClass::Tunnel,
        TerrainCdtEarthworkSupportPolicy::TunnelVisiblePortals,
        band_kind,
        source_band_index,
    )
}

fn test_nonstandard_span_boundary_source(
    edge_idx: u64,
    edge_class: TerrainCdtEdgeClass,
    support_policy: TerrainCdtEarthworkSupportPolicy,
    band_kind: TerrainCdtRoadBandKind,
    source_band_index: u32,
) -> TerrainCdtRoadBoundarySource {
    TerrainCdtRoadBoundarySource::SpanSupportBoundary {
        edge_idx,
        edge_class,
        support_policy,
        source_band_index,
        band_kind,
        role: match band_kind {
            TerrainCdtRoadBandKind::Carriageway => TerrainCdtSpanRegionRole::Asphalt,
            TerrainCdtRoadBandKind::CurbOrShoulder => TerrainCdtSpanRegionRole::CurbOrShoulder,
            _ => TerrainCdtSpanRegionRole::NonRoad,
        },
        start_section_index: 3,
        end_section_index: 4,
        start_s_m: 12.0,
        end_s_m: 16.0,
    }
}

pub(super) fn diagonal_road_loop() -> Vec<TerrainCdtVertex> {
    road_loop_from_centerline(
        TerrainCdtVertex::new(8.0, 0.0, 12.0),
        TerrainCdtVertex::new(32.0, 0.0, 28.0),
        6.0,
    )
}

pub(super) fn road_loop_from_centerline(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    width: f64,
) -> Vec<TerrainCdtVertex> {
    let dx = end.x - start.x;
    let dz = end.z - start.z;
    let length = (dx * dx + dz * dz).sqrt();
    let normal_x = -dz / length;
    let normal_z = dx / length;
    let half_width = width * 0.5;
    let mut road = vec![
        TerrainCdtVertex::new(
            start.x + normal_x * half_width,
            0.0,
            start.z + normal_z * half_width,
        ),
        TerrainCdtVertex::new(
            end.x + normal_x * half_width,
            0.0,
            end.z + normal_z * half_width,
        ),
        TerrainCdtVertex::new(
            end.x - normal_x * half_width,
            0.0,
            end.z - normal_z * half_width,
        ),
        TerrainCdtVertex::new(
            start.x - normal_x * half_width,
            0.0,
            start.z - normal_z * half_width,
        ),
    ];
    if signed_area(&road) < 0.0 {
        road.reverse();
    }
    road
}

pub(super) fn road_loop_from_centerline_with_heights(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    width: f64,
) -> Vec<TerrainCdtVertex> {
    let dx = end.x - start.x;
    let dz = end.z - start.z;
    let length = (dx * dx + dz * dz).sqrt();
    let normal_x = -dz / length;
    let normal_z = dx / length;
    let half_width = width * 0.5;
    let mut road = vec![
        TerrainCdtVertex::new(
            start.x + normal_x * half_width,
            start.height_m,
            start.z + normal_z * half_width,
        ),
        TerrainCdtVertex::new(
            end.x + normal_x * half_width,
            end.height_m,
            end.z + normal_z * half_width,
        ),
        TerrainCdtVertex::new(
            end.x - normal_x * half_width,
            end.height_m,
            end.z - normal_z * half_width,
        ),
        TerrainCdtVertex::new(
            start.x - normal_x * half_width,
            start.height_m,
            start.z - normal_z * half_width,
        ),
    ];
    if signed_area(&road) < 0.0 {
        road.reverse();
    }
    road
}

pub(super) fn piece_test_patch() -> TerrainCdtPatch {
    TerrainCdtPatch::new(0.0, 0.0, 60.0, 60.0, [0.0; 4])
}

pub(super) fn test_vertex(x: f64, z: f64) -> TerrainCdtVertex {
    TerrainCdtVertex::new(x, 0.0, z)
}

pub(super) fn build_piece_patch(
    patch: TerrainCdtPatch,
    stable_piece_id: u64,
    road: Vec<TerrainCdtVertex>,
) -> TerrainCdtMesh {
    build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![TerrainCdtRoadLoop::new(stable_piece_id, 0, road)],
        piece_source_samples(),
    ))
    .expect("Spade should triangulate a piece-owned road footprint")
}

pub(super) fn piece_source_samples() -> Vec<TerrainCdtVertex> {
    vec![
        test_vertex(6.0, 6.0),
        test_vertex(6.0, 20.0),
        test_vertex(6.0, 40.0),
        test_vertex(6.0, 54.0),
        test_vertex(20.0, 6.0),
        test_vertex(20.0, 54.0),
        test_vertex(40.0, 6.0),
        test_vertex(40.0, 54.0),
        test_vertex(54.0, 6.0),
        test_vertex(54.0, 20.0),
        test_vertex(54.0, 40.0),
        test_vertex(54.0, 54.0),
    ]
}

pub(super) fn assert_road_touched_dem_tie_in_case(
    case_name: &str,
    road: Vec<TerrainCdtVertex>,
    source_samples: Vec<TerrainCdtVertex>,
    expected_widened_source_samples: usize,
    expect_retaining_wall: bool,
) {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]);
    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![TerrainCdtRoadLoop::new(17, 0, road.clone())],
        source_samples.clone(),
    ))
    .unwrap_or_else(|_| panic!("{case_name}: terrain CDT should build"));

    let mut reversed_samples = source_samples;
    reversed_samples.reverse();
    let reordered_mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![TerrainCdtRoadLoop::new(17, 0, road.clone())],
        reversed_samples,
    ))
    .unwrap_or_else(|_| panic!("{case_name}: reordered terrain CDT should build"));

    assert_eq!(
        mesh.stats, reordered_mesh.stats,
        "{case_name}: source sample order must not change CDT diagnostics"
    );
    assert_eq!(
        canonical_triangle_set(&mesh.triangles),
        canonical_triangle_set(&reordered_mesh.triangles),
        "{case_name}: ordinary terrain triangles must be deterministic"
    );
    assert_eq!(
        canonical_triangle_set(&mesh.retaining_wall_triangles),
        canonical_triangle_set(&reordered_mesh.retaining_wall_triangles),
        "{case_name}: retaining wall triangles must be deterministic"
    );
    assert_eq!(
        mesh.stats.invalid_constraint_edges, 0,
        "{case_name}: DEM tie-in must not invalidate exact road seam constraints"
    );
    assert_eq!(
        mesh.stats.preserved_road_constraint_edges, mesh.stats.road_constraint_edges,
        "{case_name}: every road seam constraint must survive Spade insertion"
    );
    assert_eq!(
        mesh.stats.accepted_faces,
        mesh.triangles.len() + mesh.retaining_wall_triangles.len(),
        "{case_name}: accepted faces must be fully classified"
    );
    assert_eq!(
        mesh.stats.tie_in_widened_source_samples, expected_widened_source_samples,
        "{case_name}: widened DEM source sample count changed"
    );
    if expected_widened_source_samples == 0 {
        assert!(
            mesh.tie_in_widened_samples.is_empty(),
            "{case_name}: unexpected widened tie-in diagnostics"
        );
    } else {
        assert_eq!(
            mesh.tie_in_widened_samples.len(),
            expected_widened_source_samples.min(MAX_TIE_IN_SAMPLE_DIAGNOSTICS),
            "{case_name}: widened tie-in diagnostics should be capped deterministically"
        );
        assert!(
            mesh.tie_in_widened_samples
                .iter()
                .all(|sample| sample.required_distance_m > sample.distance_m),
            "{case_name}: widened samples must prove the ordinary tie-in would exceed budget"
        );
    }

    if expect_retaining_wall {
        assert!(
            mesh.stats.retaining_wall_faces > 0,
            "{case_name}: expected explicit retaining-wall faces"
        );
        assert_eq!(
            mesh.stats.retaining_wall_faces,
            mesh.retaining_wall_triangles.len(),
            "{case_name}: retaining-wall face count must match emitted wall topology"
        );
        assert!(
            mesh.stats.retaining_wall_max_slope_ratio > MAX_TERRAIN_TIE_IN_SLOPE_RATIO,
            "{case_name}: retaining walls must be driven by the documented slope budget"
        );
        assert!(
            mesh.retaining_wall_face_samples
                .iter()
                .all(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall),
            "{case_name}: retaining diagnostics must not be ordinary terrain samples"
        );
    } else {
        assert_eq!(
            mesh.stats.retaining_wall_faces, 0,
            "{case_name}: ordinary DEM tie-ins must not emit retaining-wall faces"
        );
        assert!(
            mesh.retaining_wall_triangles.is_empty(),
            "{case_name}: ordinary DEM tie-ins must not emit retaining-wall topology"
        );
        assert!(
            mesh.stats.road_seam_max_slope_ratio <= MAX_TERRAIN_TIE_IN_SLOPE_RATIO + 0.0001,
            "{case_name}: ordinary road seam faces exceeded the slope budget: {:?}",
            mesh.stats
        );
    }

    let road = ensure_ccw(simplified_test_road_loop(road));
    for triangle in mesh
        .triangles
        .iter()
        .chain(mesh.retaining_wall_triangles.iter())
    {
        let center = centroid([
            mesh.vertices[triangle[0]],
            mesh.vertices[triangle[1]],
            mesh.vertices[triangle[2]],
        ]);
        assert!(
            !point_in_polygon(center, &road),
            "{case_name}: emitted terrain tie-in leaked into the road-owned footprint"
        );
    }
}

pub(super) fn assert_sourced_road_touched_mesh_contract(
    case_name: &str,
    mesh: &TerrainCdtMesh,
    patch: TerrainCdtPatch,
    road_loops: &[Vec<TerrainCdtVertex>],
    expected_source: TerrainCdtRoadBoundarySource,
) {
    assert!(
        !mesh.emitted_faces.is_empty(),
        "{case_name}: terrain CDT should emit accepted terrain topology"
    );
    assert_eq!(
        mesh.stats.invalid_constraint_edges, 0,
        "{case_name}: authored DEM must not create invalid road constraints"
    );
    assert_eq!(
        mesh.stats.preserved_road_constraint_edges, mesh.stats.road_constraint_edges,
        "{case_name}: road seam constraints must survive triangulation"
    );
    assert_eq!(
        mesh.stats.accepted_faces,
        mesh.triangles.len() + mesh.retaining_wall_triangles.len(),
        "{case_name}: every accepted face must be projected into one emitted bucket"
    );
    assert_eq!(
        mesh.emitted_faces.len(),
        mesh.stats.accepted_faces,
        "{case_name}: first-class emitted faces must cover every accepted face"
    );
    assert_eq!(
        mesh.terrain_triangle_sources.len(),
        mesh.triangles.len(),
        "{case_name}: terrain triangle source sidecars must match terrain triangles"
    );
    assert_eq!(
        mesh.retaining_wall_triangle_sources.len(),
        mesh.retaining_wall_triangles.len(),
        "{case_name}: retaining-wall source sidecars must match retaining-wall triangles"
    );
    assert!(
        mesh.stats.road_seam_faces > 0,
        "{case_name}: road-touched terrain should report sourced road-seam faces"
    );
    assert!(
        mesh.road_seam_face_samples
            .iter()
            .all(|sample| sample.sources.contains(&expected_source)),
        "{case_name}: road-seam diagnostics must name the source owner"
    );
    assert!(
        mesh.retaining_wall_face_samples
            .iter()
            .all(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall
                && sample.sources.contains(&expected_source)),
        "{case_name}: retaining-wall diagnostics must name the source owner"
    );
    assert!(
        mesh.retaining_wall_triangle_sources
            .iter()
            .all(|sources| sources.contains(&expected_source)),
        "{case_name}: emitted retaining-wall faces must carry structured source provenance"
    );
    assert!(
        mesh.emitted_faces.iter().all(|face| {
            if face.kind == TerrainCdtTieInKind::RetainingWall {
                face.sources.contains(&expected_source)
            } else {
                true
            }
        }),
        "{case_name}: first-class retaining-wall faces must not be anonymous"
    );

    for source in mesh
        .emitted_faces
        .iter()
        .flat_map(|face| face.sources.iter().copied())
        .chain(
            mesh.road_seam_face_samples
                .iter()
                .flat_map(|sample| sample.sources.iter().copied()),
        )
        .chain(
            mesh.retaining_wall_face_samples
                .iter()
                .flat_map(|sample| sample.sources.iter().copied()),
        )
    {
        assert_source_exports_structured_provenance(case_name, source);
    }

    let clipped_roads = road_loops
        .iter()
        .filter_map(|road| {
            let clipped = ensure_ccw(simplified_test_road_loop(clip_loop_to_patch(
                road.clone(),
                patch,
            )));
            (clipped.len() >= 3).then_some(clipped)
        })
        .collect::<Vec<_>>();
    for triangle in mesh
        .triangles
        .iter()
        .chain(mesh.retaining_wall_triangles.iter())
    {
        let center = centroid([
            mesh.vertices[triangle[0]],
            mesh.vertices[triangle[1]],
            mesh.vertices[triangle[2]],
        ]);
        assert!(
            clipped_roads
                .iter()
                .all(|road| !point_in_polygon(center, road)),
            "{case_name}: emitted terrain tie-in leaked into a road-owned footprint"
        );
    }
}

pub(super) fn assert_source_exports_structured_provenance(
    case_name: &str,
    source: TerrainCdtRoadBoundarySource,
) {
    assert!(
        !source.debug_label().is_empty(),
        "{case_name}: source should retain a human debug label"
    );
    match source {
        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
            support_policy,
            start_section_index,
            end_section_index,
            start_s_m,
            end_s_m,
            ..
        } => {
            assert_eq!(
                source.source_kind_code(),
                0,
                "{case_name}: span support source kind code changed"
            );
            assert!(source.primary_id_code() >= 0);
            assert!(source.edge_class_code() >= 0);
            assert!(source.owner_kind_code() >= 0);
            assert!(source.owner_index_code() >= 0);
            assert!(source.role_code() >= 0);
            assert!(source.support_policy_code() >= 0);
            assert_eq!(
                support_policy,
                TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan
            );
            assert!(end_section_index >= start_section_index);
            assert!(end_s_m >= start_s_m);
            assert_eq!(
                source.section_range_codes(),
                [
                    i32::try_from(start_section_index).unwrap(),
                    i32::try_from(end_section_index).unwrap()
                ]
            );
            assert_eq!(source.s_range_values(), [start_s_m, end_s_m]);
        }
        TerrainCdtRoadBoundarySource::NodeFootprintBoundary { owner_index, .. } => {
            assert_eq!(
                source.source_kind_code(),
                1,
                "{case_name}: node footprint source kind code changed"
            );
            assert!(source.primary_id_code() >= 0);
            assert!(source.node_kind_code() >= 0);
            assert!(source.owner_kind_code() >= 0);
            assert_eq!(
                source.owner_index_code(),
                i32::try_from(owner_index).unwrap()
            );
            assert_eq!(source.support_policy_code(), -1);
            assert_eq!(source.role_code(), -1);
            assert_eq!(source.section_range_codes(), [-1, -1]);
            assert_eq!(source.s_range_values(), [-1.0, -1.0]);
        }
        TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff {
            owner_index_a,
            owner_index_b,
            boundary_source,
            ..
        } => {
            assert_eq!(
                source.source_kind_code(),
                2,
                "{case_name}: node same-material handoff source kind code changed"
            );
            assert!(source.primary_id_code() >= 0);
            assert!(source.node_kind_code() >= 0);
            assert!(source.owner_kind_code() >= 0);
            assert_eq!(
                source.owner_index_code(),
                i32::try_from(owner_index_a).unwrap()
            );
            assert!(owner_index_b >= owner_index_a);
            assert!(boundary_source.is_some());
            assert_eq!(source.support_policy_code(), -1);
            assert_eq!(source.role_code(), -1);
            assert_eq!(source.section_range_codes(), [-1, -1]);
            assert_eq!(source.s_range_values(), [-1.0, -1.0]);
        }
        TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
            building_idx,
            local_loop_index,
            ..
        } => {
            assert_eq!(
                source.source_kind_code(),
                3,
                "{case_name}: building-site source kind code changed"
            );
            assert_eq!(
                source.primary_id_code(),
                i32::try_from(building_idx).unwrap_or(i32::MAX)
            );
            assert_eq!(
                source.owner_index_code(),
                i32::try_from(local_loop_index).unwrap_or(i32::MAX)
            );
            assert_eq!(source.node_kind_code(), -1);
            assert_eq!(source.edge_class_code(), -1);
            assert_eq!(source.owner_kind_code(), -1);
            assert_eq!(source.support_policy_code(), -1);
            assert_eq!(source.role_code(), -1);
            assert_eq!(source.section_range_codes(), [-1, -1]);
            assert_eq!(source.s_range_values(), [-1.0, -1.0]);
        }
        TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. } => {
            panic!("{case_name}: source-preserving validation must not use synthetic sources")
        }
    }
}

pub(super) fn authored_dem_samples(
    patch: TerrainCdtPatch,
    step_m: f64,
    height_at: fn(f64, f64) -> f32,
) -> Vec<TerrainCdtVertex> {
    let mut samples = Vec::new();
    let mut z = patch.min_z;
    while z <= patch.max_z + CDT_EPSILON_M {
        let mut x = patch.min_x;
        while x <= patch.max_x + CDT_EPSILON_M {
            samples.push(TerrainCdtVertex::new(x, height_at(x, z), z));
            x += step_m;
        }
        z += step_m;
    }
    samples
}

pub(super) fn authored_cross_slope_height(x: f64, z: f64) -> f32 {
    (x * 0.16 + z * 0.02 - 3.6) as f32
}

pub(super) fn authored_along_slope_height(x: f64, _z: f64) -> f32 {
    ((x - 20.0) * 0.22) as f32
}

pub(super) fn authored_ridge_valley_height(x: f64, z: f64) -> f32 {
    let ridge_dx = x - 20.0;
    let valley_dz = z - 25.0;
    let ridge = 3.5 * (-(ridge_dx * ridge_dx) / (2.0 * 5.0 * 5.0)).exp();
    let valley = -2.2 * (-(valley_dz * valley_dz) / (2.0 * 7.0 * 7.0)).exp();
    (ridge + valley) as f32
}

pub(super) fn square_road_loop(min: f64, max: f64, height_m: f32) -> Vec<TerrainCdtVertex> {
    vec![
        TerrainCdtVertex::new(min, height_m, min),
        TerrainCdtVertex::new(max, height_m, min),
        TerrainCdtVertex::new(max, height_m, max),
        TerrainCdtVertex::new(min, height_m, max),
    ]
}

pub(super) fn canonical_square_road_loop(min: f64, max: f64) -> CanonicalTerrainCdtRoadLoop {
    let vertices = square_road_loop(min, max, 0.0);
    let bounds = terrain_cdt_loop_bounds(&vertices);
    CanonicalTerrainCdtRoadLoop {
        footprint_group_id: 7,
        is_hole: false,
        edge_sources: vec![None; vertices.len()],
        min_x: bounds.min_x,
        min_z: bounds.min_z,
        max_x: bounds.max_x,
        max_z: bounds.max_z,
        vertices,
    }
}

pub(super) fn build_crossing_patch(
    patch: TerrainCdtPatch,
    road: Vec<TerrainCdtVertex>,
) -> TerrainCdtMesh {
    build_road_touched_terrain_patch(TerrainCdtInput::new(
        patch,
        vec![TerrainCdtRoadLoop::new(7, 0, road)],
        vec![
            TerrainCdtVertex::new(5.0, 0.0, 5.0),
            TerrainCdtVertex::new(5.0, 0.0, 35.0),
            TerrainCdtVertex::new(20.0, 0.0, 5.0),
            TerrainCdtVertex::new(20.0, 0.0, 35.0),
            TerrainCdtVertex::new(35.0, 0.0, 5.0),
            TerrainCdtVertex::new(35.0, 0.0, 35.0),
        ],
    ))
    .expect("Spade should triangulate a clipped road footprint")
}

pub(super) fn assert_valid_clipped_mesh(
    mesh: &TerrainCdtMesh,
    patch: TerrainCdtPatch,
    original_road: &[TerrainCdtVertex],
) {
    let clipped_road = ensure_ccw(simplified_test_road_loop(clip_loop_to_patch(
        original_road.to_vec(),
        patch,
    )));
    assert!(clipped_road.len() >= 3);
    assert!(!mesh.triangles.is_empty());
    assert!(mesh.stats.rejected_road_faces > 0);
    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert_eq!(
        mesh.stats.preserved_road_constraint_edges,
        mesh.stats.road_constraint_edges
    );
    for vertex in &mesh.vertices {
        assert!(patch_contains(*vertex, patch));
    }
    for triangle in &mesh.triangles {
        let center = centroid([
            mesh.vertices[triangle[0]],
            mesh.vertices[triangle[1]],
            mesh.vertices[triangle[2]],
        ]);
        assert!(
            !point_in_polygon(center, &clipped_road),
            "accepted terrain triangle leaked into the clipped road footprint"
        );
    }
}

pub(super) fn assert_valid_piece_footprint_mesh(
    mesh: &TerrainCdtMesh,
    patch: TerrainCdtPatch,
    road: &[TerrainCdtVertex],
) {
    let road = ensure_ccw(simplified_test_road_loop(road.to_vec()));
    assert!(road.len() >= 3);
    assert!(!mesh.triangles.is_empty());
    assert_eq!(mesh.stats.road_constraint_edges, road.len());
    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
    assert!(mesh.stats.rejected_road_faces > 0);
    assert_eq!(
        mesh.stats.preserved_road_constraint_edges,
        mesh.stats.road_constraint_edges
    );
    for vertex in &mesh.vertices {
        assert!(patch_contains(*vertex, patch));
    }
    for triangle in &mesh.triangles {
        let center = centroid([
            mesh.vertices[triangle[0]],
            mesh.vertices[triangle[1]],
            mesh.vertices[triangle[2]],
        ]);
        assert!(
            !point_in_polygon(center, &road),
            "accepted terrain triangle leaked into a piece-owned road footprint"
        );
    }
}

pub(super) fn canonical_triangle_set(triangles: &[[usize; 3]]) -> Vec<[usize; 3]> {
    let mut canonical = triangles
        .iter()
        .map(|triangle| {
            let mut sorted = *triangle;
            sorted.sort_unstable();
            sorted
        })
        .collect::<Vec<_>>();
    canonical.sort_unstable();
    canonical
}
