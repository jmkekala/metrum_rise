//! Canonical crossing validation tests.

use super::*;

#[test]
fn reports_crossing_constraints() {
    let mut solution = solved_triangulation();
    let region = &mut solution.regions[0];
    region.boundary_constraints = vec![[0, 2], [1, 3], [0, 1], [2, 3]];

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("crossing constraints must fail validation");

    assert!(error.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.backend == NodeGeometryBackend::CanonicalKeys
            && matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    reason: NodeInvalidConstraintReason::Crossing,
                    ..
                }
            )
    }));
    assert!(
        !error.report.has_blocking_diagnostics(),
        "crossing constraints remain diagnostic-only when CDT output and coverage are valid: {}",
        error.report.debug_dump()
    );
}

#[test]
fn canonical_key_crossing_rejects_logged_microscopic_connector_false_positive() {
    let microscopic_connector = key_edge([-63.632900, -27.195601], [-63.632896, -27.195602]);
    let boundary = key_edge([-64.056534, -30.669868], [-58.100647, -31.396107]);

    assert!(
        !canonical_key_segments_strictly_intersect(microscopic_connector, boundary),
        "logged terminal sample is not a true canonical interior/interior crossing"
    );
}

#[test]
fn canonical_key_crossing_reports_only_true_interior_intersections() {
    assert!(canonical_key_segments_strictly_intersect(
        key_edge([0.0, 0.0], [2.0, 2.0]),
        key_edge([0.0, 2.0], [2.0, 0.0])
    ));
    assert!(!canonical_key_segments_strictly_intersect(
        key_edge([0.0, 0.0], [1.0, 1.0]),
        key_edge([1.0, 1.0], [2.0, 0.0])
    ));
    assert!(!canonical_key_segments_strictly_intersect(
        key_edge([0.0, 0.0], [2.0, 0.0]),
        key_edge([2.0, 0.0], [3.0, 0.0])
    ));
    assert!(canonical_key_segments_strictly_intersect(
        key_edge([0.0, 0.0], [3.0, 0.0]),
        key_edge([1.0, 0.0], [2.0, 0.0])
    ));
}
