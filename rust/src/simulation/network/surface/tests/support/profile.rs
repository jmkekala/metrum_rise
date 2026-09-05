//! Span/profile helper routines for road-surface tests.

use super::*;
use crate::simulation::network::surface::backend::RoadVec2;

pub(in crate::simulation::network::surface::tests) fn span_profile_test_section(
    edge_idx: usize,
    s_m: f32,
    bands: Vec<RoadSurfaceBand>,
) -> RoadSurfaceSection {
    RoadSurfaceSection {
        edge_idx,
        s_m,
        center_xz: RoadVec2::new(f64::from(s_m), 0.0),
        center_height_m: 0.0,
        tangent_xz: RoadVec2::new(1.0, 0.0),
        lateral_xz: RoadVec2::new(0.0, 1.0),
        bands,
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_rejects_invalid_span_profile(
    sections_for_edge: impl FnOnce(usize) -> Vec<RoadSurfaceSection>,
    reason: &str,
) {
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(40.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)],
        5.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR,
    ));

    let mut surface = RoadSurfaceSystem::new(64.0);
    surface
        .compiled_sections
        .insert(edge_idx, std::sync::Arc::new(sections_for_edge(edge_idx)));

    assert!(
        surface
            .compile_visual_span_piece(&graph, &flat_terrain(64, 64), edge_idx)
            .is_none(),
        "span region resolution must reject {reason} instead of emitting partial top-surface or terrain-clip output"
    );
}

pub(in crate::simulation::network::surface::tests) fn section_height_at_lateral_offset(
    section: &RoadSurfaceSection,
    lateral_offset_m: f32,
) -> Option<f32> {
    let mut best_height_m: Option<f32> = None;
    for band in &section.bands {
        let start = band.lateral_start_m.min(band.lateral_end_m);
        let end = band.lateral_start_m.max(band.lateral_end_m);
        if lateral_offset_m < start - 0.001 || lateral_offset_m > end + 0.001 {
            continue;
        }

        let span = band.lateral_end_m - band.lateral_start_m;
        let t = if span.abs() <= 0.001 {
            0.0
        } else {
            ((lateral_offset_m - band.lateral_start_m) / span).clamp(0.0, 1.0)
        };
        let height_m = band.height_start_m + (band.height_end_m - band.height_start_m) * t;
        best_height_m = Some(best_height_m.map_or(height_m, |best| best.max(height_m)));
    }

    best_height_m
}

pub(in crate::simulation::network::surface::tests) fn assert_junction_mouth_section_profile_matches_endpoint_plane(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    edge_idx: usize,
    at_start: bool,
) {
    let sections = surface
        .compiled_sections()
        .get(&edge_idx)
        .unwrap_or_else(|| panic!("edge {edge_idx} must have compiled sections"));
    let section = if at_start {
        sections
            .iter()
            .min_by(|a, b| a.s_m.total_cmp(&b.s_m))
            .unwrap()
    } else {
        sections
            .iter()
            .max_by(|a, b| a.s_m.total_cmp(&b.s_m))
            .unwrap()
    };
    let edge = graph.edge(edge_idx);
    let node_id = graph.get_valid_node(if at_start {
        edge.start_node
    } else {
        edge.end_node
    });
    let plane = graph
        .junction_endpoint_profile_plane(node_id)
        .expect("JunctionN endpoint must expose a solved profile plane");
    let tolerance_m = 0.005;
    for band in &section.bands {
        let height_offset_m = match band.kind {
            RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk => {
                CURB_STEP_HEIGHT_M
            }
            _ => 0.0,
        };
        for (lateral_m, height_m) in [
            (band.lateral_start_m, band.height_start_m),
            (band.lateral_end_m, band.height_end_m),
        ] {
            let expected_height_m = plane.height_at_xz(
                (section.center_xz.x + section.lateral_xz.x * f64::from(lateral_m)) as f32,
                (section.center_xz.y + section.lateral_xz.y * f64::from(lateral_m)) as f32,
            ) + height_offset_m;
            assert!(
                (height_m - expected_height_m).abs() <= tolerance_m,
                "JunctionN mouth band height must match the endpoint profile plane: edge={edge_idx} at_start={at_start} s_m={:.3} kind={:?} lateral={lateral_m:.3} height={height_m:.3} expected={expected_height_m:.3} delta={:.3}",
                section.s_m,
                band.kind,
                height_m - expected_height_m
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn outer_surface_lateral_bounds(
    section: &RoadSurfaceSection,
) -> Option<(f32, f32)> {
    Some((
        section.bands.first()?.lateral_start_m,
        section.bands.last()?.lateral_end_m,
    ))
}
