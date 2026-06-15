//! Terrain CDT export and provenance helpers for road-surface tests.

use super::*;

pub(in crate::simulation::network::surface::tests) fn terrain_cdt_input_for_bounds(
    terrain: &TerrainSystem,
    road_loops: Vec<TerrainCdtRoadLoop>,
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
    sample_step_m: f32,
) -> TerrainCdtInput {
    let patch = TerrainCdtPatch::new(
        f64::from(min_x),
        f64::from(min_z),
        f64::from(max_x),
        f64::from(max_z),
        [
            terrain_height_m(terrain, min_x, min_z),
            terrain_height_m(terrain, min_x, max_z),
            terrain_height_m(terrain, max_x, max_z),
            terrain_height_m(terrain, max_x, min_z),
        ],
    );
    let mut source_samples = Vec::new();
    let step = sample_step_m.max(1.0);
    let mut z = min_z;
    while z <= max_z + SAMPLE_EPSILON_M {
        let mut x = min_x;
        while x <= max_x + SAMPLE_EPSILON_M {
            source_samples.push(TerrainCdtVertex::new(
                f64::from(x),
                terrain_height_m(terrain, x, z),
                f64::from(z),
            ));
            x += step;
        }
        z += step;
    }
    TerrainCdtInput::new(patch, road_loops, source_samples)
}

pub(in crate::simulation::network::surface::tests) fn terrain_height_m(
    terrain: &TerrainSystem,
    x: f32,
    z: f32,
) -> f32 {
    terrain.sample_visual_height_world(x, z) * crate::config::HEIGHT_SCALE
}

pub(in crate::simulation::network::surface::tests) fn assert_surface_terrain_cdt_contract(
    case_name: &str,
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    terrain: &TerrainSystem,
    bounds: (f32, f32, f32, f32),
    expect_retaining_wall: bool,
) {
    let (min_x, min_z, max_x, max_z) = bounds;
    let (road_loops, source_count) = surface
        .terrain_cdt_road_loops_for_world_bounds(graph, min_x, min_z, max_x, max_z)
        .unwrap_or_else(|err| panic!("{case_name}: terrain clip export failed: {err:?}"));
    assert!(
        !road_loops.is_empty(),
        "{case_name}: expected production terrain CDT road loops"
    );
    assert!(
        source_count
            >= road_loops
                .iter()
                .filter(|road_loop| !road_loop.is_hole)
                .count(),
        "{case_name}: source loop count should name the raw owned footprint contributors"
    );
    for edge_source in road_loops
        .iter()
        .flat_map(|road_loop| road_loop.source_edges.iter())
    {
        assert_surface_cdt_boundary_source(case_name, edge_source.source);
    }

    let mesh = build_road_touched_terrain_patch(terrain_cdt_input_for_bounds(
        terrain,
        road_loops.clone(),
        min_x,
        min_z,
        max_x,
        max_z,
        8.0,
    ))
    .unwrap_or_else(|err| {
        panic!("{case_name}: production terrain CDT input should build: {err:?}")
    });

    assert_eq!(
        mesh.stats.invalid_constraint_edges, 0,
        "{case_name}: production CDT input must not contain invalid road constraints"
    );
    assert_eq!(
        mesh.stats.preserved_road_constraint_edges, mesh.stats.road_constraint_edges,
        "{case_name}: accepted terrain faces must preserve every road seam constraint"
    );
    assert_eq!(
        mesh.stats.accepted_faces,
        mesh.triangles.len() + mesh.retaining_wall_triangles.len(),
        "{case_name}: accepted faces must project into terrain or retaining-wall buckets"
    );
    assert_eq!(
        mesh.emitted_faces.len(),
        mesh.stats.accepted_faces,
        "{case_name}: first-class emitted face provenance must cover accepted faces"
    );
    assert_eq!(
        mesh.terrain_triangle_sources.len(),
        mesh.triangles.len(),
        "{case_name}: terrain face source sidecars must match terrain triangles"
    );
    assert_eq!(
        mesh.retaining_wall_triangle_sources.len(),
        mesh.retaining_wall_triangles.len(),
        "{case_name}: retaining-wall face source sidecars must match wall triangles"
    );
    assert!(
        mesh.stats.road_seam_faces > 0,
        "{case_name}: road-touched CDT should expose road-seam diagnostics"
    );
    assert!(
        mesh.road_seam_face_samples
            .iter()
            .all(|sample| !sample.sources.is_empty()),
        "{case_name}: road-seam diagnostics must name source owners"
    );
    assert!(
        mesh.retaining_wall_face_samples
            .iter()
            .all(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall
                && !sample.sources.is_empty()),
        "{case_name}: retaining-wall diagnostics must name source owners"
    );
    assert!(
        mesh.retaining_wall_triangle_sources
            .iter()
            .all(|sources| !sources.is_empty()),
        "{case_name}: retaining-wall emitted faces must not be anonymous"
    );
    assert_eq!(
        mesh.stats.blocking_degenerate_seam_edges, 0,
        "{case_name}: production CDT input must not pass unresolved sub-budget seam fragments to Spade"
    );
    assert_eq!(
        mesh.stats.omitted_near_seam_source_samples, mesh.stats.tie_in_widened_source_samples,
        "{case_name}: omitted near-seam terrain samples must stay visible as tie-in diagnostics"
    );
    assert!(
        mesh.emitted_faces.iter().all(|face| {
            face.kind != TerrainCdtTieInKind::RetainingWall || !face.sources.is_empty()
        }),
        "{case_name}: first-class retaining-wall emitted faces must carry source provenance"
    );
    if expect_retaining_wall {
        assert!(
            mesh.stats.retaining_wall_faces > 0,
            "{case_name}: elevated or extreme authored terrain should expose wall tie-ins"
        );
    } else {
        assert_eq!(
            mesh.stats.retaining_wall_faces, 0,
            "{case_name}: ordinary grounded road terrain tie-ins must not emit retaining-wall faces"
        );
        assert!(
            mesh.retaining_wall_triangles.is_empty(),
            "{case_name}: ordinary grounded road terrain tie-ins must not emit retaining-wall topology"
        );
    }
    assert_cdt_mesh_stays_outside_road_loops(case_name, &mesh, &road_loops);
    assert_cdt_mesh_sources_are_structured(case_name, &mesh);
}

pub(in crate::simulation::network::surface::tests) fn assert_surface_cdt_boundary_source(
    case_name: &str,
    source: TerrainCdtRoadBoundarySource,
) {
    assert!(
        !source.debug_label().is_empty(),
        "{case_name}: source label should be available for human debug"
    );
    match source {
        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
            start_section_index,
            end_section_index,
            start_s_m,
            end_s_m,
            ..
        } => {
            assert_eq!(source.source_kind_code(), 0);
            assert!(source.primary_id_code() >= 0);
            assert!(source.edge_class_code() >= 0);
            assert!(source.owner_kind_code() >= 0);
            assert!(source.owner_index_code() >= 0);
            assert!(source.support_policy_code() >= 0);
            assert!(source.role_code() >= 0);
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
        TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
            owner_index,
            boundary_source,
            ..
        } => {
            assert_eq!(source.source_kind_code(), 1);
            assert!(source.primary_id_code() >= 0);
            assert!(source.node_kind_code() >= 0);
            assert!(source.owner_kind_code() >= 0);
            assert!(
                boundary_source.is_some(),
                "{case_name}: production node CDT source must preserve endpoint boundary provenance"
            );
            assert_eq!(
                source.owner_index_code(),
                i32::try_from(owner_index).unwrap()
            );
            assert_eq!(source.edge_class_code(), -1);
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
            assert_eq!(source.source_kind_code(), 2);
            assert!(source.primary_id_code() >= 0);
            assert!(source.node_kind_code() >= 0);
            assert!(source.owner_kind_code() >= 0);
            assert!(
                boundary_source.is_some(),
                "{case_name}: production same-material node CDT handoff must preserve endpoint boundary provenance"
            );
            assert_eq!(
                source.owner_index_code(),
                i32::try_from(owner_index_a).unwrap()
            );
            assert!(owner_index_b >= owner_index_a);
            assert_eq!(source.edge_class_code(), -1);
            assert_eq!(source.support_policy_code(), -1);
            assert_eq!(source.role_code(), -1);
            assert_eq!(source.section_range_codes(), [-1, -1]);
            assert_eq!(source.s_range_values(), [-1.0, -1.0]);
        }
        TerrainCdtRoadBoundarySource::BuildingSiteBoundary { .. } => {
            panic!(
                "{case_name}: road-surface terrain CDT export must not use building-site sources"
            )
        }
        TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. } => {
            panic!("{case_name}: production terrain CDT export must not use synthetic sources")
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_cdt_mesh_sources_are_structured(
    case_name: &str,
    mesh: &TerrainCdtMesh,
) {
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
        .chain(
            mesh.tie_in_widened_samples
                .iter()
                .map(|sample| sample.seam_source),
        )
    {
        assert_surface_cdt_boundary_source(case_name, source);
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_cdt_mesh_stays_outside_road_loops(
    case_name: &str,
    mesh: &TerrainCdtMesh,
    road_loops: &[TerrainCdtRoadLoop],
) {
    for (triangle_index, triangle) in mesh
        .triangles
        .iter()
        .chain(mesh.retaining_wall_triangles.iter())
        .enumerate()
    {
        let center = {
            let a = mesh.vertices[triangle[0]];
            let b = mesh.vertices[triangle[1]];
            let c = mesh.vertices[triangle[2]];
            Vector2::new(
                ((a.x + b.x + c.x) / 3.0) as f32,
                ((a.z + b.z + c.z) / 3.0) as f32,
            )
        };
        if let Some((loop_index, road_loop)) = road_loops
            .iter()
            .enumerate()
            .filter(|(_, road_loop)| !road_loop.is_hole)
            .find(|(_, road_loop)| {
                road_loop_contains_road_owned_point_xz(road_loops, road_loop, center)
            })
        {
            panic!(
                "{case_name}: accepted terrain triangle centroid leaked inside road-owned footprint; triangle_index={triangle_index} center=({:.3},{:.3}) loop_index={loop_index} footprint_group_id={}",
                center.x, center.y, road_loop.footprint_group_id
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn road_loop_contains_road_owned_point_xz(
    road_loops: &[TerrainCdtRoadLoop],
    outer_loop: &TerrainCdtRoadLoop,
    point: Vector2,
) -> bool {
    if !terrain_cdt_loop_strictly_contains_point_xz(outer_loop, point) {
        return false;
    }
    !road_loops.iter().any(|candidate| {
        candidate.is_hole
            && candidate.footprint_group_id == outer_loop.footprint_group_id
            && terrain_cdt_loop_strictly_contains_point_xz(candidate, point)
    })
}

pub(in crate::simulation::network::surface::tests) fn terrain_cdt_loop_strictly_contains_point_xz(
    road_loop: &TerrainCdtRoadLoop,
    point: Vector2,
) -> bool {
    if road_loop.vertices.len() < 3 {
        return false;
    }
    let mut inside = false;
    for index in 0..road_loop.vertices.len() {
        let start = road_loop.vertices[index];
        let end = road_loop.vertices[(index + 1) % road_loop.vertices.len()];
        if (start.z as f32 > point.y) != (end.z as f32 > point.y) {
            let edge_x_at_point_z = ((end.x - start.x) as f32) * (point.y - start.z as f32)
                / ((end.z - start.z) as f32)
                + start.x as f32;
            if point.x < edge_x_at_point_z {
                inside = !inside;
            }
        }
    }
    inside
}
