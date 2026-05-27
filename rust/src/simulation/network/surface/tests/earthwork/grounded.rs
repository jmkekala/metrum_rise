//! Grounded standard-road support tests.

use super::*;

#[test]
fn grounded_standard_roadbed_is_laterally_flat_and_footprint_stays_below_carriageway() {
    let mut terrain = TerrainSystem::with_chunking(129, 97, 1.0, 8, 0.0);
    for z in 0..97 {
        for x in 0..129 {
            terrain.set_height(x, z, x as f32 * 0.03);
        }
    }

    let mut graph = RegionGraph::new();
    let grounded_height = terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE;
    let start = graph.add_node(
        Vector3::new(0.0, grounded_height, -24.0),
        NodeType::Junction,
    );
    let end = graph.add_node(Vector3::new(0.0, grounded_height, 24.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![
            Vector3::new(0.0, grounded_height, -24.0),
            Vector3::new(0.0, grounded_height, 24.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.rebuild_all_earthworks(&graph, &mut terrain);

    let section = surface
        .compiled_sections()
        .get(&edge_idx)
        .unwrap()
        .iter()
        .min_by(|a, b| a.center_xz.y.abs().total_cmp(&b.center_xz.y.abs()))
        .unwrap();
    let half_carriageway = graph.edge(edge_idx).width.max(crate::config::LANE_WIDTH) * 0.5;
    let left_height = section_height_at_lateral_offset(section, -half_carriageway).unwrap();
    let right_height = section_height_at_lateral_offset(section, half_carriageway).unwrap();
    let lateral_grade_rate =
        (right_height - left_height) / (half_carriageway * 2.0).max(super::SAMPLE_EPSILON_M);

    assert!(
        lateral_grade_rate.abs() <= 0.001,
        "expected grounded-road carriageway to stay laterally flat: actual_rate={lateral_grade_rate:.4}"
    );
    for sidewalk in section
        .bands
        .iter()
        .filter(|band| band.kind == RoadSurfaceBandKind::Sidewalk)
    {
        assert!(
            (sidewalk.height_start_m - section.center_height_m - CURB_STEP_HEIGHT_M).abs() <= 0.001
        );
        assert!(
            (sidewalk.height_end_m - section.center_height_m - CURB_STEP_HEIGHT_M).abs() <= 0.001
        );
    }

    let mut sampled_profile = Vec::new();
    for lateral_offset in [-half_carriageway * 0.8, 0.0, half_carriageway * 0.8] {
        let road_height = section_height_at_lateral_offset(section, lateral_offset).unwrap();
        let sample_x = section.center_xz.x + section.lateral_xz.x * f64::from(lateral_offset);
        let sample_z = section.center_xz.y + section.lateral_xz.y * f64::from(lateral_offset);
        let source_height = terrain.sample_height_world(sample_x as f32, sample_z as f32)
            * crate::config::HEIGHT_SCALE;
        let visual_height = terrain.sample_visual_height_world(sample_x as f32, sample_z as f32)
            * crate::config::HEIGHT_SCALE;
        let visible_surface_height = surface
            .sample_visible_surface_height(&graph, &terrain, sample_x as f32, sample_z as f32)
            .expect("standard road footprint should be owned by the road surface");
        sampled_profile.push((lateral_offset, road_height, visible_surface_height));
        assert!(
            (visual_height - source_height).abs() <= 0.05,
            "ordinary standard roads must not stamp visual terrain on a steep hillside: lateral_offset={lateral_offset:.2} visual_height={visual_height:.3} source_height={source_height:.3}"
        );
        assert!(
            (road_height - visible_surface_height).abs() <= 0.08,
            "expected grounded-road visible surface to follow the solved road surface: lateral_offset={lateral_offset:.2} visible_surface_height={visible_surface_height:.3} road_height={road_height:.3}"
        );
    }

    let left = sampled_profile.first().unwrap();
    let right = sampled_profile.last().unwrap();
    let road_profile_delta = right.1 - left.1;
    let support_profile_delta = right.2 - left.2;
    assert!(
        (support_profile_delta - road_profile_delta).abs() <= 0.05,
        "expected visible road footprint to follow the solved flat roadbed profile: road_profile_delta={road_profile_delta:.3} support_profile_delta={support_profile_delta:.3}"
    );
}
