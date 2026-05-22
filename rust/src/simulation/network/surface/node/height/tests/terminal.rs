//! Terminal cap height-field tests.

use super::*;

#[test]
fn terminal_material_band_height_field_keeps_curb_cap_inner_rail_raised() {
    let inner_start = RoadVec3::new(0.0, 0.12, -1.0);
    let inner_center = RoadVec3::new(0.0, 0.12, 0.0);
    let inner_end = RoadVec3::new(0.0, 0.12, 1.0);
    let outer_start = RoadVec3::new(0.15, 0.12, -1.0);
    let outer_center = RoadVec3::new(0.15, 0.12, 0.0);
    let outer_end = RoadVec3::new(0.15, 0.12, 1.0);
    let cap_band = NodeTerminalCapBand {
        source_band_index: 0,
        band_kind: RoadSurfaceBandKind::CurbOrShoulder,
        provenance: TerminalCapBandProvenance {
            layer_index: 0,
            role: TerminalCapBandRole::EndBand,
            left_source_band_index: 0,
            right_source_band_index: 0,
            source_boundary_start_index: 0,
            source_boundary_end_index: 1,
            inner_offset_m: 0.0,
            outer_offset_m: 0.15,
        },
        inner_path_world: vec![inner_start, inner_center, inner_end],
        outer_path_world: vec![outer_start, outer_center, outer_end],
        contour_world: vec![
            inner_start,
            inner_center,
            inner_end,
            outer_end,
            outer_center,
            outer_start,
        ],
    };
    let patch = NodeBandHeightPatch::from_terminal_cap_band(
        NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::CurbOrShoulder),
        RoadSurfaceBandKind::CurbOrShoulder,
        &cap_band,
    )
    .expect("test terminal cap is a valid height carrier");
    let height = match patch
        .evaluate_surface_height(
            NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::CurbOrShoulder),
            RoadSurfaceBandKind::CurbOrShoulder,
            RoadVec2::new(0.0, 0.0),
        )
        .expect("center vertex should be evaluable")
    {
        NodeHeightPatchEvaluation::Inside(height) => height,
        NodeHeightPatchEvaluation::Outside(error) => {
            panic!("center vertex should be inside terminal material band: {error:?}")
        }
    };

    assert!(
        (height - 0.12).abs() <= 1.0e-6,
        "terminal curb cap inner rail must stay raised across the carriageway split"
    );
}

#[test]
fn terminal_cap_height_field_extends_with_explicit_cap_patches_only() {
    let first_cap = terminal_cap_band_for_height_test(0.0, 0.12, TerminalCapBandRole::EndBand);
    let second_cap = terminal_cap_band_for_height_test(1.0, 0.32, TerminalCapBandRole::RightSide);
    let mut field = NodeBandHeightField::from_terminal_cap_band(0, &first_cap)
        .expect("test terminal cap is a valid height carrier");

    field
        .extend_with_terminal_cap_band(0, &second_cap)
        .expect("same terminal source may carry multiple explicit cap patches");

    let second_height = field
        .evaluate_height(RoadVec2::new(1.0, 0.0))
        .expect("second terminal cap patch should be an explicit carrier");
    assert!((second_height - 0.32).abs() <= 1.0e-6);
    assert!(matches!(
        field.evaluate_height(RoadVec2::new(0.5, 0.0)),
        Err(NodeHeightFieldError::VertexOutsideHeightField { .. })
    ));
}

#[test]
fn oblique_terminal_cap_height_field_covers_canonical_side_edge() {
    let inner_start = RoadVec3::new(66.997093, 0.12, -26.860096);
    let inner_end = RoadVec3::new(67.007172, 0.12, -27.009756);
    let outer_start = RoadVec3::new(67.146747, 0.12, -26.849936);
    let outer_end = RoadVec3::new(67.156826, 0.12, -26.999596);
    let cap_band = NodeTerminalCapBand {
        source_band_index: 6,
        band_kind: RoadSurfaceBandKind::CurbOrShoulder,
        provenance: TerminalCapBandProvenance {
            layer_index: 0,
            role: TerminalCapBandRole::RightSide,
            left_source_band_index: 6,
            right_source_band_index: 6,
            source_boundary_start_index: 0,
            source_boundary_end_index: 1,
            inner_offset_m: 0.0,
            outer_offset_m: 0.15,
        },
        inner_path_world: vec![inner_start, inner_end],
        outer_path_world: vec![outer_start, outer_end],
        contour_world: vec![inner_start, inner_end, outer_end, outer_start],
    };
    let field = NodeBandHeightField::from_terminal_cap_band(0, &cap_band)
        .expect("oblique terminal side cap should build a height field");

    let height = field
        .evaluate_height(RoadVec2::new(66.998175, -26.876167))
        .expect("canonical side-edge point should be inside the terminal cap");

    assert!((height - 0.12).abs() <= 1.0e-6);
}
