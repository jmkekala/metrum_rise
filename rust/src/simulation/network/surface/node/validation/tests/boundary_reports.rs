//! Boundary validation diagnostic tests.

use super::*;

#[test]
fn reports_open_boundaries_with_stage_and_backend() {
    let mut solution = solved_triangulation();
    let expected_owner = solution.regions[0].owner;
    let expected_height_field_id = solution.regions[0].height_field_id;
    solution.regions[0].boundary_constraints.pop();

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("missing explicit boundary constraint must fail validation");

    assert!(error.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.stage == NodeGeometryStage::Validation
            && diagnostic.backend == NodeGeometryBackend::CanonicalKeys
            && matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::OpenBoundary {
                    region_index: 0,
                    owner,
                    owner_index,
                    height_field_id,
                    x_key: Some(_),
                    z_key: Some(_),
                    x_mm: Some(_),
                    z_mm: Some(_),
                    degree: 1,
                    ..
                } if owner == expected_owner.kind()
                    && owner_index == expected_owner.owner_index()
                    && height_field_id == expected_height_field_id
            )
    }));
    let dump = error.report.debug_dump();
    assert!(dump.contains("\"stage\":\"validation\""));
    assert!(dump.contains("\"backend\":\"canonical_keys\""));
    assert!(dump.contains("\"kind\":\"open_boundary\""));
    let parsed: serde_json::Value =
        serde_json::from_str(&dump).expect("diagnostic dump must be valid JSON");
    let diagnostic = parsed["diagnostics"]
        .as_array()
        .expect("diagnostics must be an array")
        .iter()
        .find(|diagnostic| diagnostic["kind"] == "open_boundary")
        .expect("open boundary diagnostic must be present");
    let expected_owner_name = format!("{:?}", expected_owner.kind());
    let expected_height_field = format!("{expected_height_field_id:?}");
    assert_eq!(
        diagnostic["owner"]["kind"].as_str(),
        Some(expected_owner_name.as_str())
    );
    assert_eq!(
        diagnostic["height_field_id"]["debug"].as_str(),
        Some(expected_height_field.as_str())
    );
    assert!(diagnostic["x_key"].is_number());
    assert!(diagnostic["z_key"].is_number());
}

#[test]
fn accepts_exposed_triangle_edges_split_by_topology_dust_boundary_vertex() {
    let kind = RoadSurfaceBandKind::Carriageway;
    let height_field_id = NodeBandHeightFieldId::new(0, 0, kind);
    let region = manual_region_with_constraints_and_triangles(
        kind,
        0,
        height_field_id,
        vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(0.5, 0.0, 0.0002),
            RoadVec3::new(1.0, 0.0, 0.0),
            RoadVec3::new(0.0, 0.0, 1.0),
        ],
        vec![[0, 2], [2, 3], [0, 3]],
        vec![
            NodeTriangulatedTriangle {
                vertices: [0, 1, 3],
            },
            NodeTriangulatedTriangle {
                vertices: [1, 2, 3],
            },
        ],
        0.5,
    );
    let solution = NodeTriangulationSolution {
        node_id: 121,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![region],
        explicit_vertical_step_segments: Vec::new(),
    };

    NodeValidationReport::from_triangulation_solution(&solution)
        .expect("topology-dust split boundary vertex should validate against explicit boundary");
}

#[test]
fn reports_duplicate_exposed_edge_even_for_tiny_canonical_sliver() {
    let start = RoadVec3::new(0.0, 0.0, 0.0);
    let end = RoadVec3::new(0.005, 0.0, 0.0);
    let tiny_edge = key_edge(
        [f64::from(start.x), f64::from(start.z)],
        [f64::from(end.x), f64::from(end.z)],
    );
    let regions = [
        (
            RoadSurfaceBandKind::Carriageway,
            RoadVec3::new(0.0, 0.0, 1.0),
        ),
        (RoadSurfaceBandKind::Footpath, RoadVec3::new(0.0, 0.0, -1.0)),
        (RoadSurfaceBandKind::Parking, RoadVec3::new(0.005, 0.0, 1.0)),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (kind, apex))| {
        manual_region_with_constraints_and_triangles(
            kind,
            index,
            NodeBandHeightFieldId::new(0, index, kind),
            vec![start, end, apex],
            vec![[0, 1], [1, 2], [0, 2]],
            vec![NodeTriangulatedTriangle {
                vertices: [0, 1, 2],
            }],
            0.0025,
        )
    })
    .collect();
    let solution = NodeTriangulationSolution {
        node_id: 119,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions,
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("tiny duplicate exposed topology must be reported, not silently suppressed");

    assert!(error.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.stage == NodeGeometryStage::Validation
            && diagnostic.backend == NodeGeometryBackend::Parry2d
            && matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::DuplicateExposedEdge {
                    region_index: None,
                    ref regions,
                    start_x_key,
                    start_z_key,
                    end_x_key,
                    end_z_key,
                    start_x_mm,
                    start_z_mm,
                    end_x_mm,
                    end_z_mm,
                    count: 3,
                } if regions.len() == 3
                    && start_x_key == tiny_edge.start.x_key
                    && start_z_key == tiny_edge.start.z_key
                    && end_x_key == tiny_edge.end.x_key
                    && end_z_key == tiny_edge.end.z_key
                    && start_x_mm == tiny_edge.start.x_mm()
                    && start_z_mm == tiny_edge.start.z_mm()
                    && end_x_mm == tiny_edge.end.x_mm()
                    && end_z_mm == tiny_edge.end.z_mm()
            )
    }));
    let parsed: serde_json::Value = serde_json::from_str(&error.report.debug_dump())
        .expect("diagnostic dump must be valid JSON");
    let diagnostic = parsed["diagnostics"]
        .as_array()
        .expect("diagnostics must be an array")
        .iter()
        .find(|diagnostic| diagnostic["kind"] == "duplicate_exposed_edge")
        .expect("duplicate exposed edge diagnostic must be present");
    assert_eq!(diagnostic["regions"].as_array().unwrap().len(), 3);
    assert_eq!(diagnostic["regions"][0]["owner"]["kind"], "Carriageway");
    let expected_height_field = format!(
        "{:?}",
        NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway)
    );
    assert_eq!(
        diagnostic["regions"][0]["height_field_id"]["debug"].as_str(),
        Some(expected_height_field.as_str())
    );
    assert_eq!(diagnostic["start_x_key"], tiny_edge.start.x_key);
    assert_eq!(diagnostic["end_x_key"], tiny_edge.end.x_key);
}

#[test]
fn reports_non_explicit_boundary_vertices_with_owner_and_canonical_keys() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 4);
    let height_field_id = NodeBandHeightFieldId::new(2, 6, RoadSurfaceBandKind::Carriageway);
    let center = RoadVec3::new(0.5, 0.0, 0.5);
    let center_key = key_point(0.5, 0.5);
    let region = manual_region_with_constraints_and_triangles(
        RoadSurfaceBandKind::Carriageway,
        owner.owner_index(),
        height_field_id,
        vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 0.0),
            RoadVec3::new(1.0, 0.0, 1.0),
            RoadVec3::new(0.0, 0.0, 1.0),
            center,
        ],
        vec![[0, 1], [1, 2], [2, 3], [0, 3]],
        vec![NodeTriangulatedTriangle {
            vertices: [0, 1, 4],
        }],
        0.25,
    );
    let solution = NodeTriangulationSolution {
        node_id: 120,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![region],
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("triangle edge outside explicit boundary constraints must fail validation");

    assert!(error.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.stage == NodeGeometryStage::Validation
            && diagnostic.backend == NodeGeometryBackend::Parry2d
            && matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::NonExplicitBoundaryVertex {
                    region_index: 0,
                    owner: RoadSurfaceBandKind::Carriageway,
                    owner_index: 4,
                    height_field_id: id,
                    x_key,
                    z_key,
                    x_mm,
                    z_mm,
                    min_boundary_distance_mm,
                } if id == height_field_id
                    && x_key == center_key.x_key
                    && z_key == center_key.z_key
                    && x_mm == center_key.x_mm()
                    && z_mm == center_key.z_mm()
                    && min_boundary_distance_mm > 0
            )
    }));
    let parsed: serde_json::Value = serde_json::from_str(&error.report.debug_dump())
        .expect("diagnostic dump must be valid JSON");
    let diagnostic = parsed["diagnostics"]
        .as_array()
        .expect("diagnostics must be an array")
        .iter()
        .find(|diagnostic| diagnostic["kind"] == "non_explicit_boundary_vertex")
        .expect("non-explicit boundary diagnostic must be present");
    assert_eq!(diagnostic["owner"]["kind"], "Carriageway");
    assert_eq!(diagnostic["owner"]["owner_index"], 4);
    let expected_height_field = format!("{height_field_id:?}");
    assert_eq!(
        diagnostic["height_field_id"]["debug"].as_str(),
        Some(expected_height_field.as_str())
    );
    assert_eq!(diagnostic["x_key"], center_key.x_key);
    assert_eq!(diagnostic["z_key"], center_key.z_key);
}
