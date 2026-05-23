//! Diagnostic assertion helpers.

use super::*;

pub(in crate::simulation::network::surface::tests) fn assert_junction_rejected_with_canonical_height_diagnostic(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    node_id: u32,
    label: &str,
) {
    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&node_id),
        "{label} unexpectedly compiled after same-XZ height disagreement"
    );
    let report = canonical_junction_pipeline_report(surface, graph, node_id);
    let accepted_canonical_rejection = report.contains("shared_source_height_conflict")
        || report.contains("source_height_field_conflict")
        || report.contains("vertex_outside_height_field")
        || report.contains("missing_owned_region_carrier_support")
        || report.contains("\"height_conflict\"")
        || report.contains("missing_raised_step_vertical_face")
        || report.contains("MissingRaisedStepVerticalFace")
        || report.contains("AmbiguousEarthworkBoundarySegmentSource");
    assert!(
        accepted_canonical_rejection,
        "{label} must reject with a canonical height/provenance diagnostic: {report}"
    );
}

pub(in crate::simulation::network::surface::tests) fn assert_debug_dump_mouth_seams_are_clean(
    dump: &str,
) {
    let json_start = dump
        .find('{')
        .expect("road geometry dump should contain a JSON object");
    let json_end = dump
        .rfind('}')
        .expect("road geometry dump should contain a JSON object");
    let json: serde_json::Value = serde_json::from_str(&dump[json_start..=json_end])
        .expect("road geometry dump JSON should parse");
    let nodes = json["nodes"]
        .as_array()
        .expect("road geometry dump should include nodes");
    let mut checked = 0usize;
    for node in nodes {
        let node_id = node["node_id"].as_u64().unwrap_or_default();
        let mouth_seams = node["mouth_seams"]
            .as_array()
            .expect("node debug dump should include mouth seams");
        for seam in mouth_seams {
            checked += 1;
            let problem_count = seam["problem_count"]
                .as_u64()
                .expect("mouth seam debug should include a problem count");
            assert_eq!(
                problem_count, 0,
                "mouth seam debug must be clean; node_id={node_id} seam={seam}"
            );
        }
    }
    assert!(
        checked > 0,
        "road geometry dump should include mouth seam checks"
    );
}
